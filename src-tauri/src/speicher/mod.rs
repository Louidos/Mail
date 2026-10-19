//! Verschlüsselter lokaler Zwischenspeicher (SQLite mit SQLCipher).
//!
//! - Die Datenbank `mails.db` im App-Datenordner ist vollständig verschlüsselt (AES-256).
//! - Der Schlüssel (32 zufällige Bytes) wird beim ersten Start erzeugt und nur im
//!   Schlüsselbund des Betriebssystems abgelegt.
//! - Ohne Schlüssel ist die Datei nicht lesbar. Fehlt der Schlüssel (z. B. Schlüsselbund
//!   zurückgesetzt), wird die Datei verworfen und neu angelegt – die Mails liegen ja auf dem Server.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use ring::rand::{SecureRandom, SystemRandom};
use rusqlite::{params, Connection, OptionalExtension};
use zeroize::Zeroizing;

use crate::fehler::Fehlermeldung;
use crate::imap::cache::{MailCache, MailKopf, OrdnerStand};
use crate::konto::schluesselbund::Schluesselbund;

/// Name des Schlüsselbund-Eintrags für den Datenbankschlüssel.
pub const SCHLUESSEL_EINTRAG: &str = "speicher:schluessel";

/// Dateiname der Datenbank im App-Datenordner.
pub const DATEINAME: &str = "mails.db";

const SCHEMA_VERSION: i32 = 1;

/// Fehlerart: Die Datei passt nicht zum Schlüssel (oder ist keine Datenbank).
const UNLESBAR: &str = "speicher_unlesbar";

/// So lange wird gewartet, wenn die Datei gerade von einem anderen Zugriff gesperrt ist.
const SPERRE_WARTEN: std::time::Duration = std::time::Duration::from_secs(5);

fn fehler(_: rusqlite::Error) -> Fehlermeldung {
    Fehlermeldung::neu(
        "speicher",
        "Der lokale Speicher konnte nicht gelesen oder geschrieben werden.",
    )
}

/// Holt den Datenbankschlüssel aus dem Schlüsselbund oder erzeugt einen neuen.
///
/// Gibt zusätzlich zurück, ob der Schlüssel neu erzeugt wurde.
pub fn schluessel_holen(
    schluesselbund: &dyn Schluesselbund,
) -> Result<(Zeroizing<[u8; 32]>, bool), Fehlermeldung> {
    if let Some(gespeichert) = schluesselbund.lesen(SCHLUESSEL_EINTRAG)? {
        let mut schluessel = Zeroizing::new([0u8; 32]);
        if hex::decode_to_slice(gespeichert.as_bytes(), schluessel.as_mut()).is_ok() {
            return Ok((schluessel, false));
        }
    }
    let mut schluessel = Zeroizing::new([0u8; 32]);
    SystemRandom::new()
        .fill(schluessel.as_mut())
        .map_err(|_| Fehlermeldung::neu("speicher", "Es konnte kein Schlüssel erzeugt werden."))?;
    let text = Zeroizing::new(hex::encode(schluessel.as_ref()));
    schluesselbund.setzen(SCHLUESSEL_EINTRAG, &text)?;
    Ok((schluessel, true))
}

/// Mails im verschlüsselten Speicher.
pub struct SqlCache {
    verbindung: Mutex<Connection>,
    pfad: PathBuf,
}

impl SqlCache {
    /// Öffnet (oder erstellt) die verschlüsselte Datenbank.
    ///
    /// Passt der Schlüssel nicht zur vorhandenen Datei, wird die Datei verworfen und neu angelegt.
    /// Bei anderen Fehlern (z. B. Datei gerade gesperrt) bleibt sie erhalten – sie enthält auch
    /// Entwürfe, die es nur lokal gibt.
    pub fn oeffnen(pfad: &Path, schluessel: &[u8; 32]) -> Result<Self, Fehlermeldung> {
        match Self::oeffnen_versuch(pfad, schluessel) {
            Ok(cache) => Ok(cache),
            Err(fehler) if fehler.art == UNLESBAR && pfad.exists() => {
                Self::dateien_entfernen(pfad);
                Self::oeffnen_versuch(pfad, schluessel)
            }
            Err(e) => Err(e),
        }
    }

    /// Öffnet die Datenbank mit dem Schlüssel aus dem Schlüsselbund.
    pub fn mit_schluesselbund(
        ordner: &Path,
        schluesselbund: &dyn Schluesselbund,
    ) -> Result<Self, Fehlermeldung> {
        let pfad = ordner.join(DATEINAME);
        let (schluessel, neu) = schluessel_holen(schluesselbund)?;
        if neu && pfad.exists() {
            // Alte Datei mit unbekanntem Schlüssel: nicht lesbar, also verwerfen.
            Self::dateien_entfernen(&pfad);
        }
        Self::oeffnen(&pfad, &schluessel)
    }

    fn oeffnen_versuch(pfad: &Path, schluessel: &[u8; 32]) -> Result<Self, Fehlermeldung> {
        if let Some(ordner) = pfad.parent() {
            std::fs::create_dir_all(ordner).map_err(|_| {
                Fehlermeldung::neu("speicher", "Der Datenordner konnte nicht angelegt werden.")
            })?;
        }
        let verbindung = Connection::open(pfad).map_err(fehler)?;
        verbindung.busy_timeout(SPERRE_WARTEN).map_err(fehler)?;
        // Keine SQLCipher-Meldungen auf der Konsole.
        let _ = verbindung.execute_batch("PRAGMA cipher_log_level = NONE;");
        // Rohschlüssel (kein Passwort): x'<64 Hex-Zeichen>'
        let pragma = Zeroizing::new(format!("PRAGMA key = \"x'{}'\";", hex::encode(schluessel)));
        verbindung.execute_batch(&pragma).map_err(fehler)?;
        // Prüft, ob der Schlüssel passt (sonst: „file is not a database“).
        verbindung
            .query_row("SELECT count(*) FROM sqlite_master", [], |z| {
                z.get::<_, i64>(0)
            })
            .map_err(|e| match e.sqlite_error_code() {
                Some(rusqlite::ErrorCode::NotADatabase) => {
                    Fehlermeldung::neu(UNLESBAR, "Der lokale Speicher passt nicht zum Schlüssel.")
                }
                _ => fehler(e),
            })?;
        verbindung
            .execute_batch(
                "PRAGMA secure_delete = ON;
                 PRAGMA journal_mode = DELETE;
                 PRAGMA foreign_keys = ON;
                 CREATE TABLE IF NOT EXISTS ordner_stand (
                     konto TEXT NOT NULL,
                     ordner TEXT NOT NULL,
                     uidvalidity INTEGER NOT NULL,
                     uidnext INTEGER NOT NULL,
                     PRIMARY KEY (konto, ordner)
                 );
                 CREATE TABLE IF NOT EXISTS entwuerfe (
                     id TEXT PRIMARY KEY,
                     konto TEXT NOT NULL,
                     daten TEXT NOT NULL,
                     geaendert INTEGER NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS mails (
                     konto TEXT NOT NULL,
                     ordner TEXT NOT NULL,
                     uid INTEGER NOT NULL,
                     betreff TEXT NOT NULL,
                     absender TEXT NOT NULL,
                     absender_adresse TEXT NOT NULL,
                     datum INTEGER,
                     gelesen INTEGER NOT NULL,
                     markiert INTEGER NOT NULL,
                     groesse INTEGER NOT NULL,
                     inhalt BLOB,
                     PRIMARY KEY (konto, ordner, uid)
                 );",
            )
            .map_err(fehler)?;
        verbindung
            .pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(fehler)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(pfad, std::fs::Permissions::from_mode(0o600));
        }
        Ok(Self {
            verbindung: Mutex::new(verbindung),
            pfad: pfad.to_path_buf(),
        })
    }

    fn dateien_entfernen(pfad: &Path) {
        let _ = std::fs::remove_file(pfad);
        for endung in ["-journal", "-wal", "-shm"] {
            let mut name = pfad.as_os_str().to_owned();
            name.push(endung);
            let _ = std::fs::remove_file(PathBuf::from(name));
        }
    }

    /// Schliesst die Datenbank und löscht die Datei.
    pub fn loeschen(self) {
        let pfad = self.pfad.clone();
        drop(self);
        Self::dateien_entfernen(&pfad);
    }

    fn mit<T>(
        &self,
        f: impl FnOnce(&Connection) -> rusqlite::Result<T>,
    ) -> Result<T, Fehlermeldung> {
        let verbindung = self
            .verbindung
            .lock()
            .map_err(|_| Fehlermeldung::neu("intern", "Interner Fehler im lokalen Speicher."))?;
        f(&verbindung).map_err(fehler)
    }
}

fn kopf_aus_zeile(zeile: &rusqlite::Row) -> rusqlite::Result<MailKopf> {
    Ok(MailKopf {
        uid: zeile.get(0)?,
        betreff: zeile.get(1)?,
        absender: zeile.get(2)?,
        absender_adresse: zeile.get(3)?,
        datum: zeile.get(4)?,
        gelesen: zeile.get(5)?,
        markiert: zeile.get(6)?,
        groesse: zeile.get(7)?,
    })
}

const KOPF_SPALTEN: &str =
    "uid, betreff, absender, absender_adresse, datum, gelesen, markiert, groesse";

impl MailCache for SqlCache {
    fn stand(&self, konto: &str, ordner: &str) -> Result<Option<OrdnerStand>, Fehlermeldung> {
        self.mit(|v| {
            v.query_row(
                "SELECT uidvalidity FROM ordner_stand WHERE konto = ?1 AND ordner = ?2",
                params![konto, ordner],
                |z| {
                    Ok(OrdnerStand {
                        uidvalidity: z.get(0)?,
                    })
                },
            )
            .optional()
        })
    }

    fn stand_setzen(
        &self,
        konto: &str,
        ordner: &str,
        stand: OrdnerStand,
    ) -> Result<(), Fehlermeldung> {
        self.mit(|v| {
            let vorher: Option<u32> = v
                .query_row(
                    "SELECT uidvalidity FROM ordner_stand WHERE konto = ?1 AND ordner = ?2",
                    params![konto, ordner],
                    |z| z.get(0),
                )
                .optional()?;
            if vorher != Some(stand.uidvalidity) {
                v.execute(
                    "DELETE FROM mails WHERE konto = ?1 AND ordner = ?2",
                    params![konto, ordner],
                )?;
            }
            // Die Spalte `uidnext` wird nicht mehr gelesen; sie bleibt, damit das Datenformat
            // unverändert ist, und wird bei neuen Zeilen nur mit 0 gefüllt.
            v.execute(
                "INSERT INTO ordner_stand (konto, ordner, uidvalidity, uidnext) VALUES (?1, ?2, ?3, 0)
                 ON CONFLICT (konto, ordner) DO UPDATE SET uidvalidity = ?3",
                params![konto, ordner, stand.uidvalidity],
            )?;
            Ok(())
        })
    }

    fn koepfe_speichern(
        &self,
        konto: &str,
        ordner: &str,
        koepfe: &[MailKopf],
    ) -> Result<(), Fehlermeldung> {
        let mut verbindung = self
            .verbindung
            .lock()
            .map_err(|_| Fehlermeldung::neu("intern", "Interner Fehler im lokalen Speicher."))?;
        let transaktion = verbindung.transaction().map_err(fehler)?;
        {
            let mut anweisung = transaktion
                .prepare_cached(
                    "INSERT INTO mails (konto, ordner, uid, betreff, absender, absender_adresse, datum, gelesen, markiert, groesse)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                     ON CONFLICT (konto, ordner, uid) DO UPDATE SET
                       betreff = ?4, absender = ?5, absender_adresse = ?6, datum = ?7,
                       gelesen = ?8, markiert = ?9, groesse = ?10",
                )
                .map_err(fehler)?;
            for k in koepfe {
                anweisung
                    .execute(params![
                        konto,
                        ordner,
                        k.uid,
                        k.betreff,
                        k.absender,
                        k.absender_adresse,
                        k.datum,
                        k.gelesen,
                        k.markiert,
                        k.groesse
                    ])
                    .map_err(fehler)?;
            }
        }
        transaktion.commit().map_err(fehler)
    }

    fn uids(&self, konto: &str, ordner: &str) -> Result<Vec<u32>, Fehlermeldung> {
        self.mit(|v| {
            let mut anweisung = v.prepare_cached(
                "SELECT uid FROM mails WHERE konto = ?1 AND ordner = ?2 ORDER BY uid",
            )?;
            let uids = anweisung
                .query_map(params![konto, ordner], |z| z.get(0))?
                .collect::<rusqlite::Result<Vec<u32>>>()?;
            Ok(uids)
        })
    }

    fn entfernen(&self, konto: &str, ordner: &str, uids: &[u32]) -> Result<(), Fehlermeldung> {
        self.mit(|v| {
            let mut anweisung = v.prepare_cached(
                "DELETE FROM mails WHERE konto = ?1 AND ordner = ?2 AND uid = ?3",
            )?;
            for uid in uids {
                anweisung.execute(params![konto, ordner, uid])?;
            }
            Ok(())
        })
    }

    fn markierungen_setzen(
        &self,
        konto: &str,
        ordner: &str,
        uid: u32,
        gelesen: bool,
        markiert: bool,
    ) -> Result<(), Fehlermeldung> {
        self.mit(|v| {
            v.execute(
                "UPDATE mails SET gelesen = ?4, markiert = ?5 WHERE konto = ?1 AND ordner = ?2 AND uid = ?3",
                params![konto, ordner, uid, gelesen, markiert],
            )?;
            Ok(())
        })
    }

    fn seite(
        &self,
        konto: &str,
        ordner: &str,
        start: usize,
        anzahl: usize,
    ) -> Result<Vec<MailKopf>, Fehlermeldung> {
        self.mit(|v| {
            let mut anweisung = v.prepare_cached(&format!(
                "SELECT {KOPF_SPALTEN} FROM mails WHERE konto = ?1 AND ordner = ?2
                 ORDER BY uid DESC LIMIT ?3 OFFSET ?4"
            ))?;
            let koepfe = anweisung
                .query_map(
                    params![konto, ordner, anzahl as i64, start as i64],
                    kopf_aus_zeile,
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(koepfe)
        })
    }

    fn kopf(&self, konto: &str, ordner: &str, uid: u32) -> Result<Option<MailKopf>, Fehlermeldung> {
        self.mit(|v| {
            v.query_row(
                &format!(
                    "SELECT {KOPF_SPALTEN} FROM mails WHERE konto = ?1 AND ordner = ?2 AND uid = ?3"
                ),
                params![konto, ordner, uid],
                kopf_aus_zeile,
            )
            .optional()
        })
    }

    fn inhalt(
        &self,
        konto: &str,
        ordner: &str,
        uid: u32,
    ) -> Result<Option<Vec<u8>>, Fehlermeldung> {
        self.mit(|v| {
            Ok(v.query_row(
                "SELECT inhalt FROM mails WHERE konto = ?1 AND ordner = ?2 AND uid = ?3",
                params![konto, ordner, uid],
                |z| z.get::<_, Option<Vec<u8>>>(0),
            )
            .optional()?
            .flatten())
        })
    }

    fn hat_inhalt(&self, konto: &str, ordner: &str, uid: u32) -> Result<bool, Fehlermeldung> {
        self.mit(|v| {
            Ok(v.query_row(
                "SELECT inhalt IS NOT NULL FROM mails WHERE konto = ?1 AND ordner = ?2 AND uid = ?3",
                params![konto, ordner, uid],
                |z| z.get::<_, bool>(0),
            )
            .optional()?
            .unwrap_or(false))
        })
    }

    fn inhalt_speichern(
        &self,
        konto: &str,
        ordner: &str,
        uid: u32,
        inhalt: &[u8],
    ) -> Result<(), Fehlermeldung> {
        self.mit(|v| {
            v.execute(
                "UPDATE mails SET inhalt = ?4 WHERE konto = ?1 AND ordner = ?2 AND uid = ?3",
                params![konto, ordner, uid, inhalt],
            )?;
            Ok(())
        })
    }

    fn konto_entfernen(&self, konto: &str) -> Result<(), Fehlermeldung> {
        self.mit(|v| {
            v.execute("DELETE FROM mails WHERE konto = ?1", params![konto])?;
            v.execute("DELETE FROM ordner_stand WHERE konto = ?1", params![konto])?;
            v.execute("DELETE FROM entwuerfe WHERE konto = ?1", params![konto])?;
            Ok(())
        })
    }

    fn leeren(&self) -> Result<(), Fehlermeldung> {
        self.mit(|v| {
            v.execute_batch("DELETE FROM mails; DELETE FROM ordner_stand; VACUUM;")?;
            Ok(())
        })
    }

    fn entwurf_speichern(&self, id: &str, konto: &str, daten: &str) -> Result<(), Fehlermeldung> {
        self.mit(|v| {
            // Laufende Nummer statt Uhrzeit: eindeutige Reihenfolge auch bei schnellen Änderungen.
            v.execute(
                "INSERT INTO entwuerfe (id, konto, daten, geaendert)
                 VALUES (?1, ?2, ?3, (SELECT COALESCE(MAX(geaendert), 0) + 1 FROM entwuerfe))
                 ON CONFLICT (id) DO UPDATE SET konto = ?2, daten = ?3,
                   geaendert = (SELECT COALESCE(MAX(geaendert), 0) + 1 FROM entwuerfe)",
                params![id, konto, daten],
            )?;
            Ok(())
        })
    }

    fn entwuerfe(&self, konto: &str) -> Result<Vec<(String, String)>, Fehlermeldung> {
        self.mit(|v| {
            let mut anweisung = v.prepare_cached(
                "SELECT id, daten FROM entwuerfe WHERE konto = ?1 ORDER BY geaendert DESC",
            )?;
            let liste = anweisung
                .query_map(params![konto], |z| Ok((z.get(0)?, z.get(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(liste)
        })
    }

    fn entwurf_loeschen(&self, id: &str) -> Result<(), Fehlermeldung> {
        self.mit(|v| {
            v.execute("DELETE FROM entwuerfe WHERE id = ?1", params![id])?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::imap::cache::tests::{kopf, verhalten_pruefen};
    use crate::konto::schluesselbund::SpeicherSchluesselbund;

    fn schluessel(wert: u8) -> [u8; 32] {
        [wert; 32]
    }

    #[test]
    fn verhaelt_sich_wie_jeder_zwischenspeicher() {
        let ordner = tempfile::tempdir().unwrap();
        let cache = SqlCache::oeffnen(&ordner.path().join(DATEINAME), &schluessel(7)).unwrap();
        verhalten_pruefen(&cache);
    }

    #[test]
    fn bestehende_datenbank_mit_uidnext_bleibt_lesbar() {
        let ordner = tempfile::tempdir().unwrap();
        let pfad = ordner.path().join(DATEINAME);
        {
            // So hat eine frühere Version den Stand geschrieben (mit echtem UIDNEXT).
            let cache = SqlCache::oeffnen(&pfad, &schluessel(3)).unwrap();
            cache
                .mit(|v| {
                    v.execute(
                        "INSERT INTO ordner_stand (konto, ordner, uidvalidity, uidnext)
                         VALUES ('k', 'INBOX', 7, 42)",
                        [],
                    )
                })
                .unwrap();
            cache.koepfe_speichern("k", "INBOX", &[kopf(1)]).unwrap();
        }
        let cache = SqlCache::oeffnen(&pfad, &schluessel(3)).unwrap();
        assert_eq!(
            cache.stand("k", "INBOX").unwrap(),
            Some(OrdnerStand { uidvalidity: 7 })
        );
        cache
            .stand_setzen("k", "INBOX", OrdnerStand { uidvalidity: 7 })
            .unwrap();
        assert_eq!(cache.uids("k", "INBOX").unwrap(), vec![1]);
        let uidnext: u32 = cache
            .mit(|v| v.query_row("SELECT uidnext FROM ordner_stand", [], |z| z.get(0)))
            .unwrap();
        assert_eq!(uidnext, 42, "vorhandener Wert bleibt unangetastet");
    }

    #[test]
    fn datei_ist_ohne_schluessel_nicht_lesbar() {
        let ordner = tempfile::tempdir().unwrap();
        let pfad = ordner.path().join(DATEINAME);
        {
            let cache = SqlCache::oeffnen(&pfad, &schluessel(1)).unwrap();
            let mut k = kopf(1);
            k.betreff = "Streng vertraulicher Betreff".into();
            cache.koepfe_speichern("konto", "INBOX", &[k]).unwrap();
            cache
                .inhalt_speichern("konto", "INBOX", 1, b"Geheimer Inhalt der Mail")
                .unwrap();
        }

        // Kein Klartext und kein SQLite-Kopf in der Datei.
        let roh = std::fs::read(&pfad).unwrap();
        assert!(!roh.starts_with(b"SQLite format 3\0"));
        for klartext in [
            &b"Streng vertraulich"[..],
            b"Geheimer Inhalt",
            b"CREATE TABLE",
        ] {
            assert!(!roh.windows(klartext.len()).any(|f| f == klartext));
        }

        // Öffnen wie mit normalem SQLite (ohne Schlüssel) schlägt fehl.
        let ohne = Connection::open(&pfad).unwrap();
        let fehler = ohne
            .query_row("SELECT count(*) FROM sqlite_master", [], |z| {
                z.get::<_, i64>(0)
            })
            .unwrap_err();
        assert!(fehler.to_string().contains("not a database"), "{fehler}");

        // Mit falschem Schlüssel ebenfalls.
        let falsch = Connection::open(&pfad).unwrap();
        falsch
            .execute_batch(&format!(
                "PRAGMA key = \"x'{}'\";",
                hex::encode(schluessel(2))
            ))
            .unwrap();
        assert!(falsch
            .query_row("SELECT count(*) FROM mails", [], |z| z.get::<_, i64>(0))
            .is_err());

        // Mit dem richtigen Schlüssel ist alles da.
        let cache = SqlCache::oeffnen(&pfad, &schluessel(1)).unwrap();
        assert_eq!(cache.uids("konto", "INBOX").unwrap(), vec![1]);
        assert_eq!(
            cache.inhalt("konto", "INBOX", 1).unwrap().unwrap(),
            b"Geheimer Inhalt der Mail"
        );
    }

    #[test]
    fn falscher_schluessel_verwirft_die_datei() {
        let ordner = tempfile::tempdir().unwrap();
        let pfad = ordner.path().join(DATEINAME);
        {
            let cache = SqlCache::oeffnen(&pfad, &schluessel(1)).unwrap();
            cache.koepfe_speichern("k", "INBOX", &[kopf(1)]).unwrap();
        }
        let cache = SqlCache::oeffnen(&pfad, &schluessel(9)).unwrap();
        assert!(cache.uids("k", "INBOX").unwrap().is_empty());
    }

    #[test]
    fn gesperrte_datei_wird_nicht_verworfen() {
        let ordner = tempfile::tempdir().unwrap();
        let pfad = ordner.path().join(DATEINAME);
        let erste = SqlCache::oeffnen(&pfad, &schluessel(4)).unwrap();
        erste.koepfe_speichern("k", "INBOX", &[kopf(1)]).unwrap();
        // Eine andere Instanz hält die Datei gesperrt.
        erste
            .verbindung
            .lock()
            .unwrap()
            .execute_batch("BEGIN EXCLUSIVE;")
            .unwrap();
        let fehler = SqlCache::oeffnen(&pfad, &schluessel(4)).err().unwrap();
        assert_ne!(fehler.art, UNLESBAR);
        erste
            .verbindung
            .lock()
            .unwrap()
            .execute_batch("COMMIT;")
            .unwrap();
        drop(erste);
        // Die Daten (z. B. Entwürfe) sind noch da.
        let wieder = SqlCache::oeffnen(&pfad, &schluessel(4)).unwrap();
        assert_eq!(wieder.uids("k", "INBOX").unwrap(), vec![1]);
    }

    #[test]
    fn leeren_entfernt_alles() {
        let ordner = tempfile::tempdir().unwrap();
        let pfad = ordner.path().join(DATEINAME);
        let cache = SqlCache::oeffnen(&pfad, &schluessel(3)).unwrap();
        cache
            .koepfe_speichern("k", "INBOX", &[kopf(1), kopf(2)])
            .unwrap();
        cache.inhalt_speichern("k", "INBOX", 1, b"Inhalt").unwrap();
        cache.leeren().unwrap();
        assert!(cache.uids("k", "INBOX").unwrap().is_empty());
        assert!(cache.inhalt("k", "INBOX", 1).unwrap().is_none());
        assert!(cache.stand("k", "INBOX").unwrap().is_none());
        cache.loeschen();
        assert!(!pfad.exists());
    }

    #[test]
    fn schluessel_liegt_nur_im_schluesselbund() {
        let ordner = tempfile::tempdir().unwrap();
        let bund = SpeicherSchluesselbund::default();
        let (erster, neu) = schluessel_holen(&bund).unwrap();
        assert!(neu);
        let (zweiter, neu) = schluessel_holen(&bund).unwrap();
        assert!(!neu);
        assert_eq!(*erster, *zweiter);
        assert_ne!(*erster, [0u8; 32]);

        let cache = SqlCache::mit_schluesselbund(ordner.path(), &bund).unwrap();
        cache.koepfe_speichern("k", "INBOX", &[kopf(1)]).unwrap();
        drop(cache);
        // Der Schlüssel steht in keiner Datei des Datenordners.
        let schluessel_hex = hex::encode(*erster);
        for eintrag in std::fs::read_dir(ordner.path()).unwrap() {
            let inhalt = std::fs::read(eintrag.unwrap().path()).unwrap();
            assert!(!inhalt
                .windows(schluessel_hex.len())
                .any(|f| f == schluessel_hex.as_bytes()));
            assert!(!inhalt.windows(32).any(|f| f == erster.as_ref()));
        }

        // Schlüssel verloren: Datei wird verworfen und neu angelegt.
        let leer = SpeicherSchluesselbund::default();
        let cache = SqlCache::mit_schluesselbund(ordner.path(), &leer).unwrap();
        assert!(cache.uids("k", "INBOX").unwrap().is_empty());
    }
}
