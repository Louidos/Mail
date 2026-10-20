//! Verschlüsselter lokaler Zwischenspeicher (SQLite mit SQLCipher).
//!
//! - Die Datenbank `mails.db` im App-Datenordner ist vollständig verschlüsselt (AES-256).
//! - Der Schlüssel (32 zufällige Bytes) wird beim ersten Start erzeugt und nur im
//!   Schlüsselbund des Betriebssystems abgelegt.
//! - Ohne Schlüssel ist die Datei nicht lesbar. Fehlt der Schlüssel (z. B. Schlüsselbund
//!   zurückgesetzt), wird die Datei verworfen und neu angelegt – die Mails liegen ja auf dem Server.
//! - Der Suchindex (#49, FTS5) liegt in derselben verschlüsselten Datei (siehe [`suche`]).

pub mod suche;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use ring::rand::{SecureRandom, SystemRandom};
use rusqlite::types::Value;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use zeroize::Zeroizing;

use crate::fehler::Fehlermeldung;
use crate::imap::cache::{MailCache, MailKopf, OrdnerStand};
use crate::konto::schluesselbund::Schluesselbund;
use suche::{MailEintrag, Suche};

/// Name des Schlüsselbund-Eintrags für den Datenbankschlüssel.
pub const SCHLUESSEL_EINTRAG: &str = "speicher:schluessel";

/// Dateiname der Datenbank im App-Datenordner.
pub const DATEINAME: &str = "mails.db";

/// Version 2 (#49): Suchindex `mails_suche` und Spalte `mails.suche_id`.
const SCHEMA_VERSION: i32 = 2;

/// Suchindex (#49): FTS5 in derselben verschlüsselten Datenbank. Die Zeilennummer einer Mail im
/// Index steht in `mails.suche_id`; wird die Mail gelöscht, entfernt der Trigger den Eintrag.
/// Gross-/Kleinschreibung und Akzente (`ü` = `u`) spielen beim Suchen keine Rolle.
const SUCHE_SCHEMA: &str = "
    CREATE VIRTUAL TABLE IF NOT EXISTS mails_suche USING fts5(
        absender, betreff, text,
        tokenize = 'unicode61 remove_diacritics 2'
    );
    CREATE INDEX IF NOT EXISTS mails_nach_suche_id ON mails (suche_id);
    CREATE TRIGGER IF NOT EXISTS mails_suche_entfernen AFTER DELETE ON mails
    WHEN old.suche_id IS NOT NULL
    BEGIN
        DELETE FROM mails_suche WHERE rowid = old.suche_id;
    END;";

/// Fehlerart: Die Datei passt nicht zum Schlüssel (oder ist keine Datenbank).
pub const UNLESBAR: &str = "speicher_unlesbar";

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
    schluessel_holen_fuer(schluesselbund, SCHLUESSEL_EINTRAG)
}

/// Wie [`schluessel_holen`], aber für einen beliebigen Schlüsselbund-Eintrag.
pub fn schluessel_holen_fuer(
    schluesselbund: &dyn Schluesselbund,
    eintrag: &str,
) -> Result<(Zeroizing<[u8; 32]>, bool), Fehlermeldung> {
    if let Some(gespeichert) = schluesselbund.lesen(eintrag)? {
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
    schluesselbund.setzen(eintrag, &text)?;
    Ok((schluessel, true))
}

/// Öffnet eine mit SQLCipher verschlüsselte Datenbank (Rohschlüssel), prüft den Schlüssel und
/// schaltet `secure_delete` ein. Passt der Schlüssel nicht, ist die Fehlerart [`UNLESBAR`].
pub fn verschluesselt_oeffnen(
    pfad: &Path,
    schluessel: &[u8; 32],
) -> Result<Connection, Fehlermeldung> {
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
             PRAGMA foreign_keys = ON;",
        )
        .map_err(fehler)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(pfad, std::fs::Permissions::from_mode(0o600));
    }
    Ok(verbindung)
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
        let mut verbindung = verschluesselt_oeffnen(pfad, schluessel)?;
        verbindung
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS ordner_stand (
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
        suchindex_einrichten(&mut verbindung).map_err(fehler)?;
        verbindung
            .pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(fehler)?;
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

    /// Wie [`SqlCache::mit`], aber in einer Transaktion: Mails und Suchindex ändern sich gemeinsam.
    fn in_transaktion<T>(
        &self,
        f: impl FnOnce(&Connection) -> rusqlite::Result<T>,
    ) -> Result<T, Fehlermeldung> {
        let mut verbindung = self
            .verbindung
            .lock()
            .map_err(|_| Fehlermeldung::neu("intern", "Interner Fehler im lokalen Speicher."))?;
        let transaktion = verbindung.transaction().map_err(fehler)?;
        let ergebnis = f(&transaktion).map_err(fehler)?;
        transaktion.commit().map_err(fehler)?;
        Ok(ergebnis)
    }
}

/// Richtet den Suchindex ein (additiv): Ältere Datenbanken bekommen die Spalte `suche_id` und die
/// Tabelle `mails_suche`; alle Mails ohne Eintrag – beim ersten Start nach dem Update also alle
/// gespeicherten – werden in den Index übernommen. Bestehende Daten bleiben unverändert.
fn suchindex_einrichten(verbindung: &mut Connection) -> rusqlite::Result<()> {
    let hat_spalte = verbindung
        .prepare("SELECT suche_id FROM mails LIMIT 0")
        .is_ok();
    let transaktion = verbindung.transaction()?;
    if !hat_spalte {
        transaktion.execute_batch("ALTER TABLE mails ADD COLUMN suche_id INTEGER;")?;
    }
    transaktion.execute_batch(SUCHE_SCHEMA)?;
    let offen = {
        let mut anweisung =
            transaktion.prepare("SELECT konto, ordner, uid FROM mails WHERE suche_id IS NULL")?;
        let zeilen = anweisung
            .query_map([], |z| {
                Ok((z.get::<_, String>(0)?, z.get::<_, String>(1)?, z.get(2)?))
            })?
            .collect::<rusqlite::Result<Vec<(String, String, u32)>>>()?;
        zeilen
    };
    for (konto, ordner, uid) in offen {
        index_nachtragen(&transaktion, &konto, &ordner, uid)?;
    }
    transaktion.commit()
}

/// Trägt eine gespeicherte Mail (Kopfdaten und – falls vorhanden – Inhalt) in den Index ein.
fn index_nachtragen(v: &Connection, konto: &str, ordner: &str, uid: u32) -> rusqlite::Result<()> {
    let zeile = v
        .query_row(
            &format!(
                "SELECT {KOPF_SPALTEN}, inhalt FROM mails WHERE konto = ?1 AND ordner = ?2 AND uid = ?3"
            ),
            params![konto, ordner, uid],
            |z| Ok((kopf_aus_zeile(z)?, z.get::<_, Option<Vec<u8>>>(8)?)),
        )
        .optional()?;
    let Some((kopf, inhalt)) = zeile else {
        return Ok(());
    };
    let text = inhalt
        .as_deref()
        .and_then(suche::suchtext)
        .unwrap_or_default();
    index_eintragen(v, konto, ordner, &kopf, &text)
}

/// Neuer Eintrag im Suchindex; die Zeilennummer kommt in `mails.suche_id`.
fn index_eintragen(
    v: &Connection,
    konto: &str,
    ordner: &str,
    kopf: &MailKopf,
    text: &str,
) -> rusqlite::Result<()> {
    v.execute(
        "INSERT INTO mails_suche (absender, betreff, text) VALUES (?1, ?2, ?3)",
        params![suche::absender_text(kopf), kopf.betreff, text],
    )?;
    let id = v.last_insert_rowid();
    v.execute(
        "UPDATE mails SET suche_id = ?4 WHERE konto = ?1 AND ordner = ?2 AND uid = ?3",
        params![konto, ordner, kopf.uid, id],
    )?;
    Ok(())
}

fn suche_id(v: &Connection, konto: &str, ordner: &str, uid: u32) -> rusqlite::Result<Option<i64>> {
    Ok(v.query_row(
        "SELECT suche_id FROM mails WHERE konto = ?1 AND ordner = ?2 AND uid = ?3",
        params![konto, ordner, uid],
        |z| z.get::<_, Option<i64>>(0),
    )
    .optional()?
    .flatten())
}

fn kopf_aus_zeile(zeile: &rusqlite::Row) -> rusqlite::Result<MailKopf> {
    kopf_ab(zeile, 0)
}

/// Liest die Spalten aus [`KOPF_SPALTEN`] ab Spalte `ab`.
fn kopf_ab(zeile: &rusqlite::Row, ab: usize) -> rusqlite::Result<MailKopf> {
    Ok(MailKopf {
        uid: zeile.get(ab)?,
        betreff: zeile.get(ab + 1)?,
        absender: zeile.get(ab + 2)?,
        absender_adresse: zeile.get(ab + 3)?,
        datum: zeile.get(ab + 4)?,
        gelesen: zeile.get(ab + 5)?,
        markiert: zeile.get(ab + 6)?,
        groesse: zeile.get(ab + 7)?,
    })
}

const KOPF_SPALTEN: &str =
    "uid, betreff, absender, absender_adresse, datum, gelesen, markiert, groesse";

/// Wie [`KOPF_SPALTEN`], mit Tabellenname (der Suchindex hat gleichnamige Spalten).
const KOPF_SPALTEN_M: &str =
    "m.uid, m.betreff, m.absender, m.absender_adresse, m.datum, m.gelesen, m.markiert, m.groesse";

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
            let mut vorher_lesen = transaktion
                .prepare_cached(
                    "SELECT suche_id, betreff, absender, absender_adresse FROM mails
                     WHERE konto = ?1 AND ordner = ?2 AND uid = ?3",
                )
                .map_err(fehler)?;
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
                let vorher = vorher_lesen
                    .query_row(params![konto, ordner, k.uid], |z| {
                        Ok((
                            z.get::<_, Option<i64>>(0)?,
                            z.get::<_, String>(1)?,
                            z.get::<_, String>(2)?,
                            z.get::<_, String>(3)?,
                        ))
                    })
                    .optional()
                    .map_err(fehler)?;
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
                // Suchindex: neue Mails eintragen, geänderte Kopfdaten übernehmen (Inhalt bleibt).
                match vorher {
                    None => index_eintragen(&transaktion, konto, ordner, k, ""),
                    Some((None, ..)) => index_nachtragen(&transaktion, konto, ordner, k.uid),
                    Some((Some(id), betreff, absender, adresse)) => {
                        if betreff == k.betreff
                            && absender == k.absender
                            && adresse == k.absender_adresse
                        {
                            Ok(())
                        } else {
                            transaktion
                                .execute(
                                    "UPDATE mails_suche SET absender = ?1, betreff = ?2 WHERE rowid = ?3",
                                    params![suche::absender_text(k), k.betreff, id],
                                )
                                .map(|_| ())
                        }
                    }
                }
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
        // Ausserhalb der Sperre: Das Lesen der Mail braucht etwas Zeit. Bei Ende-zu-Ende
        // verschlüsselten Mails bleibt der Inhalt im Index leer.
        let text = suche::suchtext(inhalt).unwrap_or_default();
        self.in_transaktion(|v| {
            v.execute(
                "UPDATE mails SET inhalt = ?4 WHERE konto = ?1 AND ordner = ?2 AND uid = ?3",
                params![konto, ordner, uid, inhalt],
            )?;
            match suche_id(v, konto, ordner, uid)? {
                Some(id) => {
                    v.execute(
                        "UPDATE mails_suche SET text = ?1 WHERE rowid = ?2",
                        params![text, id],
                    )?;
                }
                None => index_nachtragen(v, konto, ordner, uid)?,
            }
            Ok(())
        })
    }

    fn konto_entfernen(&self, konto: &str) -> Result<(), Fehlermeldung> {
        self.mit(|v| {
            v.execute("DELETE FROM mails WHERE konto = ?1", params![konto])?;
            v.execute("DELETE FROM ordner_stand WHERE konto = ?1", params![konto])?;
            v.execute("DELETE FROM entwuerfe WHERE konto = ?1", params![konto])?;
            // Gelöschte Einträge sofort aus allen Teilen des Index entfernen (sonst erst später).
            v.execute(
                "INSERT INTO mails_suche (mails_suche) VALUES ('optimize')",
                [],
            )?;
            Ok(())
        })
    }

    fn leeren(&self) -> Result<(), Fehlermeldung> {
        self.mit(|v| {
            // Der Index wird ganz neu angelegt, damit nichts von ihm zurückbleibt.
            v.execute_batch(&format!(
                "DROP TRIGGER IF EXISTS mails_suche_entfernen;
                 DROP TABLE IF EXISTS mails_suche;
                 DELETE FROM mails; DELETE FROM ordner_stand;
                 {SUCHE_SCHEMA}
                 VACUUM;"
            ))?;
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

    fn suchen(
        &self,
        suche: &Suche,
        start: usize,
        anzahl: usize,
    ) -> Result<Vec<MailEintrag>, Fehlermeldung> {
        // Die Eingabe des Nutzers geht nur als sicher gebauter Ausdruck und als Parameter ein.
        let Some(ausdruck) = suche::fts_ausdruck(&suche.begriff, suche.feld) else {
            return Ok(Vec::new());
        };
        if suche.orte.is_empty() || anzahl == 0 {
            return Ok(Vec::new());
        }
        let mut werte = vec![Value::Text(ausdruck)];
        let mut bereich = Vec::new();
        for ort in &suche.orte {
            werte.push(Value::Text(ort.konto.clone()));
            let nr = werte.len();
            match &ort.ordner {
                Some(ordner) => {
                    werte.push(Value::Text(ordner.clone()));
                    bereich.push(format!("(m.konto = ?{nr} AND m.ordner = ?{})", nr + 1));
                }
                None => bereich.push(format!("m.konto = ?{nr}")),
            }
        }
        werte.push(Value::Integer(i64::try_from(anzahl).unwrap_or(i64::MAX)));
        werte.push(Value::Integer(i64::try_from(start).unwrap_or(i64::MAX)));
        let (grenze, versatz) = (werte.len() - 1, werte.len());
        let sql = format!(
            "SELECT m.konto, m.ordner, {KOPF_SPALTEN_M}
             FROM mails_suche JOIN mails m ON m.suche_id = mails_suche.rowid
             WHERE mails_suche MATCH ?1 AND ({})
             ORDER BY m.datum IS NULL, m.datum DESC, m.uid DESC
             LIMIT ?{grenze} OFFSET ?{versatz}",
            bereich.join(" OR ")
        );
        self.mit(|v| {
            let mut anweisung = v.prepare(&sql)?;
            let treffer = anweisung
                .query_map(params_from_iter(werte), |z| {
                    Ok(MailEintrag {
                        konto: z.get(0)?,
                        ordner: z.get(1)?,
                        kopf: kopf_ab(z, 2)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(treffer)
        })
    }

    fn ueber_konten(
        &self,
        konten: &[String],
        ordner: &str,
        start: usize,
        anzahl: usize,
    ) -> Result<Vec<MailEintrag>, Fehlermeldung> {
        if konten.is_empty() || anzahl == 0 {
            return Ok(Vec::new());
        }
        let platzhalter = vec!["?"; konten.len()].join(", ");
        let mut werte: Vec<Value> = konten.iter().cloned().map(Value::Text).collect();
        werte.push(Value::Text(ordner.to_string()));
        werte.push(Value::Integer(i64::try_from(anzahl).unwrap_or(i64::MAX)));
        werte.push(Value::Integer(i64::try_from(start).unwrap_or(i64::MAX)));
        let sql = format!(
            "SELECT m.konto, m.ordner, {KOPF_SPALTEN_M} FROM mails m
             WHERE m.konto IN ({platzhalter}) AND m.ordner = ?
             ORDER BY m.datum IS NULL, m.datum DESC, m.uid DESC
             LIMIT ? OFFSET ?"
        );
        self.mit(|v| {
            let mut anweisung = v.prepare(&sql)?;
            let mails = anweisung
                .query_map(params_from_iter(werte), |z| {
                    Ok(MailEintrag {
                        konto: z.get(0)?,
                        ordner: z.get(1)?,
                        kopf: kopf_ab(z, 2)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(mails)
        })
    }

    fn ungelesen(&self, konten: &[String], ordner: &str) -> Result<usize, Fehlermeldung> {
        if konten.is_empty() {
            return Ok(0);
        }
        let platzhalter = vec!["?"; konten.len()].join(", ");
        let mut werte: Vec<Value> = konten.iter().cloned().map(Value::Text).collect();
        werte.push(Value::Text(ordner.to_string()));
        self.mit(|v| {
            let anzahl: i64 = v.query_row(
                &format!(
                    "SELECT count(*) FROM mails
                     WHERE konto IN ({platzhalter}) AND ordner = ? AND gelesen = 0"
                ),
                params_from_iter(werte),
                |z| z.get(0),
            )?;
            Ok(usize::try_from(anzahl).unwrap_or(0))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::imap::cache::tests::{kopf, suche_pruefen, ueber_konten_pruefen, verhalten_pruefen};
    use crate::konto::schluesselbund::SpeicherSchluesselbund;
    use crate::speicher::suche::{SuchFeld, SuchOrt};

    fn suche(begriff: &str, feld: SuchFeld) -> Suche {
        Suche {
            begriff: begriff.into(),
            feld,
            orte: vec![SuchOrt {
                konto: "k".into(),
                ordner: None,
            }],
        }
    }

    fn gefunden(cache: &SqlCache, begriff: &str, feld: SuchFeld) -> Vec<u32> {
        cache
            .suchen(&suche(begriff, feld), 0, 200)
            .unwrap()
            .iter()
            .map(|t| t.kopf.uid)
            .collect()
    }

    #[test]
    fn sqlcipher_enthaelt_fts5() {
        let ordner = tempfile::tempdir().unwrap();
        let cache = SqlCache::oeffnen(&ordner.path().join(DATEINAME), &schluessel(5)).unwrap();
        let optionen: Vec<String> = cache
            .mit(|v| {
                let mut a = v.prepare("PRAGMA compile_options")?;
                let liste = a
                    .query_map([], |z| z.get(0))?
                    .collect::<rusqlite::Result<Vec<String>>>()?;
                Ok(liste)
            })
            .unwrap();
        assert!(optionen.iter().any(|o| o == "ENABLE_FTS5"), "{optionen:?}");
        assert!(
            optionen.iter().any(|o| o.starts_with("HAS_CODEC")),
            "SQLCipher: {optionen:?}"
        );
    }

    #[test]
    fn gemeinsamer_posteingang_im_verschluesselten_speicher() {
        let ordner = tempfile::tempdir().unwrap();
        let cache = SqlCache::oeffnen(&ordner.path().join(DATEINAME), &schluessel(10)).unwrap();
        ueber_konten_pruefen(&cache);
    }

    #[test]
    fn suche_im_verschluesselten_speicher() {
        let ordner = tempfile::tempdir().unwrap();
        let cache = SqlCache::oeffnen(&ordner.path().join(DATEINAME), &schluessel(6)).unwrap();
        suche_pruefen(&cache);
    }

    /// So hat Version 0.4.0 (Schema 1) die Datenbank angelegt.
    const SCHEMA_1: &str = "
        CREATE TABLE ordner_stand (konto TEXT NOT NULL, ordner TEXT NOT NULL,
            uidvalidity INTEGER NOT NULL, uidnext INTEGER NOT NULL, PRIMARY KEY (konto, ordner));
        CREATE TABLE entwuerfe (id TEXT PRIMARY KEY, konto TEXT NOT NULL, daten TEXT NOT NULL,
            geaendert INTEGER NOT NULL);
        CREATE TABLE mails (konto TEXT NOT NULL, ordner TEXT NOT NULL, uid INTEGER NOT NULL,
            betreff TEXT NOT NULL, absender TEXT NOT NULL, absender_adresse TEXT NOT NULL,
            datum INTEGER, gelesen INTEGER NOT NULL, markiert INTEGER NOT NULL,
            groesse INTEGER NOT NULL, inhalt BLOB, PRIMARY KEY (konto, ordner, uid));
        PRAGMA user_version = 1;";

    #[test]
    fn migration_baut_den_index_fuer_alte_mails_auf() {
        let ordner = tempfile::tempdir().unwrap();
        let pfad = ordner.path().join(DATEINAME);
        {
            let v = verschluesselt_oeffnen(&pfad, &schluessel(8)).unwrap();
            v.execute_batch(SCHEMA_1).unwrap();
            v.execute_batch(
                "INSERT INTO ordner_stand VALUES ('k', 'INBOX', 7, 0);
                 INSERT INTO entwuerfe VALUES ('e1', 'k', '{}', 1);
                 INSERT INTO mails VALUES ('k', 'INBOX', 1, 'Offerte Velo', 'Jürg Müller',
                     'juerg@example.org', 1700000001, 1, 0, 10, NULL);
                 INSERT INTO mails VALUES ('k', 'Archiv', 2, 'Alt', 'Anna', 'anna@example.org',
                     1700000002, 0, 1, 10, NULL);",
            )
            .unwrap();
            v.execute(
                "UPDATE mails SET inhalt = ?1 WHERE uid = 2",
                [&b"From: anna@example.org\r\nContent-Type: text/plain\r\n\r\nTreffpunkt Bahnhof\r\n"[..]],
            )
            .unwrap();
        }
        let cache = SqlCache::oeffnen(&pfad, &schluessel(8)).unwrap();
        // Mails von vor dem Update werden gefunden …
        assert_eq!(gefunden(&cache, "offerte", SuchFeld::Betreff), vec![1]);
        assert_eq!(gefunden(&cache, "muller", SuchFeld::Absender), vec![1]);
        assert_eq!(gefunden(&cache, "bahnhof", SuchFeld::Inhalt), vec![2]);
        // … und alle Daten sind unverändert da.
        let version: i32 = cache
            .mit(|v| v.query_row("PRAGMA user_version", [], |z| z.get(0)))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        assert_eq!(
            cache.stand("k", "INBOX").unwrap(),
            Some(OrdnerStand { uidvalidity: 7 })
        );
        assert_eq!(cache.entwuerfe("k").unwrap().len(), 1);
        let k1 = cache.kopf("k", "INBOX", 1).unwrap().unwrap();
        assert!(k1.gelesen && !k1.markiert && k1.betreff == "Offerte Velo");
        assert!(cache.inhalt("k", "Archiv", 2).unwrap().is_some());
        drop(cache);

        // Erneutes Öffnen baut nichts doppelt auf.
        let cache = SqlCache::oeffnen(&pfad, &schluessel(8)).unwrap();
        let eintraege: i64 = cache
            .mit(|v| v.query_row("SELECT count(*) FROM mails_suche", [], |z| z.get(0)))
            .unwrap();
        assert_eq!(eintraege, 2);
        assert_eq!(gefunden(&cache, "offerte", SuchFeld::Alle), vec![1]);
    }

    #[test]
    fn verschluesselte_mails_nur_mit_kopfdaten_im_index() {
        let ordner = tempfile::tempdir().unwrap();
        let cache = SqlCache::oeffnen(&ordner.path().join(DATEINAME), &schluessel(2)).unwrap();
        let mut k = kopf(1);
        k.betreff = "Vertrag".into();
        cache.koepfe_speichern("k", "INBOX", &[k]).unwrap();
        // Ein unverschlüsselter Teil neben einem S/MIME-verschlüsselten: kein Inhalt im Index.
        cache
            .inhalt_speichern(
                "k",
                "INBOX",
                1,
                b"From: a@example.org\r\nSubject: Vertrag\r\nContent-Type: multipart/mixed; boundary=\"m\"\r\n\r\n--m\r\nContent-Type: text/plain\r\n\r\nKlartextwort\r\n--m\r\nContent-Type: application/pkcs7-mime; smime-type=enveloped-data\r\n\r\nMIAG\r\n--m--\r\n",
            )
            .unwrap();
        assert_eq!(gefunden(&cache, "vertrag", SuchFeld::Betreff), vec![1]);
        assert!(gefunden(&cache, "klartextwort", SuchFeld::Alle).is_empty());
        let text: String = cache
            .mit(|v| v.query_row("SELECT text FROM mails_suche", [], |z| z.get(0)))
            .unwrap();
        assert_eq!(text, "");
    }

    #[test]
    fn suchindex_ist_ohne_schluessel_nicht_lesbar() {
        let ordner = tempfile::tempdir().unwrap();
        let pfad = ordner.path().join(DATEINAME);
        {
            let cache = SqlCache::oeffnen(&pfad, &schluessel(1)).unwrap();
            let mut k = kopf(1);
            k.betreff = "Quartalsbilanz".into();
            cache.koepfe_speichern("k", "INBOX", &[k]).unwrap();
            cache
                .inhalt_speichern(
                    "k",
                    "INBOX",
                    1,
                    b"From: a@example.org\r\nContent-Type: text/plain\r\n\r\nSuchwortImInhalt\r\n",
                )
                .unwrap();
            assert_eq!(
                gefunden(&cache, "suchwortiminhalt", SuchFeld::Inhalt),
                vec![1]
            );
        }
        // Weder Begriffe noch Index-Strukturen im Klartext in einer Datei des Ordners.
        for eintrag in std::fs::read_dir(ordner.path()).unwrap() {
            let roh = std::fs::read(eintrag.unwrap().path()).unwrap();
            for klartext in [
                &b"Quartalsbilanz"[..],
                b"quartalsbilanz",
                b"SuchwortImInhalt",
                b"suchwortiminhalt",
                b"mails_suche",
                b"fts5",
            ] {
                assert!(!roh.windows(klartext.len()).any(|f| f == klartext));
            }
        }
    }

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
