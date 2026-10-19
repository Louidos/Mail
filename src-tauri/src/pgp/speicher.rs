//! Verschlüsselter Speicher für OpenPGP-Schlüssel (`schluessel.db`, SQLCipher).
//!
//! - Dieselbe Datei hält auch die S/MIME-Zertifikate (#56, Tabellen `smime_*`, siehe
//!   `crate::smime::speicher`); die Tabellen werden beim Öffnen ergänzt, bestehende bleiben
//!   unverändert, alte Dateien öffnen weiter.
//! - Eigene Datei, unabhängig von „Mails lokal speichern“ und „Lokalen Speicher leeren“.
//! - Der Datenbankschlüssel (32 Zufallsbytes) liegt nur im Schlüsselbund
//!   (Eintrag [`PGP_SCHLUESSEL_EINTRAG`]).
//! - Private Schlüssel liegen hier zusätzlich mit der Passphrase des Nutzers geschützt (S2K).
//! - Fehlt der Datenbankschlüssel, wird die Datei nie gelöscht, sondern beiseitegelegt
//!   (`schluessel.db.unlesbar-<Zeit>`): Private Schlüssel gibt es oft nur hier.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rusqlite::{params, Connection};
use zeroize::Zeroizing;

use crate::fehler::Fehlermeldung;
use crate::konto::schluesselbund::{Schluesselbund, PGP_SCHLUESSEL_EINTRAG};
use crate::speicher::{schluessel_holen_fuer, verschluesselt_oeffnen, UNLESBAR};

/// Dateiname im App-Datenordner.
pub const DATEINAME: &str = "schluessel.db";

/// Ein gespeicherter eigener (privater) Schlüssel.
pub struct EigenerEintrag {
    pub konto: String,
    pub fingerabdruck: String,
    /// ASCII-armored, mit der Passphrase des Nutzers geschützt.
    pub geheim: Zeroizing<Vec<u8>>,
}

/// Ein gespeicherter öffentlicher Schlüssel eines anderen.
pub struct FremderEintrag {
    pub fingerabdruck: String,
    pub oeffentlich: Vec<u8>,
}

fn fehler(_: rusqlite::Error) -> Fehlermeldung {
    Fehlermeldung::neu(
        "pgp_speicher",
        "Der Schlüsselspeicher konnte nicht gelesen oder geschrieben werden.",
    )
}

fn jetzt() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub struct SchluesselSpeicher {
    verbindung: Mutex<Connection>,
}

impl SchluesselSpeicher {
    /// Öffnet `schluessel.db` im Ordner mit dem Schlüssel aus dem Schlüsselbund.
    pub fn mit_schluesselbund(
        ordner: &Path,
        schluesselbund: &dyn Schluesselbund,
    ) -> Result<Self, Fehlermeldung> {
        let pfad = ordner.join(DATEINAME);
        let (schluessel, neu) = schluessel_holen_fuer(schluesselbund, PGP_SCHLUESSEL_EINTRAG)?;
        if neu && pfad.exists() {
            beiseitelegen(&pfad);
        }
        match Self::oeffnen(&pfad, &schluessel) {
            Err(e) if e.art == UNLESBAR => {
                beiseitelegen(&pfad);
                Self::oeffnen(&pfad, &schluessel)
            }
            ergebnis => ergebnis,
        }
    }

    /// Öffnet (oder erstellt) die Datenbank mit diesem Schlüssel.
    pub fn oeffnen(pfad: &Path, schluessel: &[u8; 32]) -> Result<Self, Fehlermeldung> {
        let verbindung = verschluesselt_oeffnen(pfad, schluessel)?;
        verbindung
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS eigene (
                     fingerabdruck TEXT PRIMARY KEY,
                     konto TEXT NOT NULL,
                     geheim BLOB NOT NULL,
                     gespeichert INTEGER NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS fremde (
                     fingerabdruck TEXT PRIMARY KEY,
                     oeffentlich BLOB NOT NULL,
                     gespeichert INTEGER NOT NULL
                 );",
            )
            .map_err(fehler)?;
        crate::smime::speicher::tabellen_anlegen(&verbindung).map_err(fehler)?;
        Ok(Self {
            verbindung: Mutex::new(verbindung),
        })
    }

    /// Führt `f` mit der Datenbankverbindung aus (auch für die S/MIME-Tabellen).
    pub(crate) fn mit<T>(
        &self,
        f: impl FnOnce(&Connection) -> rusqlite::Result<T>,
    ) -> Result<T, Fehlermeldung> {
        let verbindung = self
            .verbindung
            .lock()
            .map_err(|_| Fehlermeldung::neu("intern", "Interner Fehler im Schlüsselspeicher."))?;
        f(&verbindung).map_err(fehler)
    }

    pub fn eigene(&self) -> Result<Vec<EigenerEintrag>, Fehlermeldung> {
        self.mit(|v| {
            let mut abfrage =
                v.prepare("SELECT konto, fingerabdruck, geheim FROM eigene ORDER BY gespeichert")?;
            let zeilen = abfrage.query_map([], |z| {
                Ok(EigenerEintrag {
                    konto: z.get(0)?,
                    fingerabdruck: z.get(1)?,
                    geheim: Zeroizing::new(z.get(2)?),
                })
            })?;
            zeilen.collect()
        })
    }

    pub fn eigenen_speichern(
        &self,
        konto: &str,
        fingerabdruck: &str,
        geheim: &[u8],
    ) -> Result<(), Fehlermeldung> {
        self.mit(|v| {
            v.execute(
                "INSERT INTO eigene (fingerabdruck, konto, geheim, gespeichert) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(fingerabdruck) DO UPDATE SET konto = ?2, geheim = ?3",
                params![fingerabdruck, konto, geheim, jetzt()],
            )
            .map(|_| ())
        })
    }

    pub fn fremde(&self) -> Result<Vec<FremderEintrag>, Fehlermeldung> {
        self.mit(|v| {
            let mut abfrage =
                v.prepare("SELECT fingerabdruck, oeffentlich FROM fremde ORDER BY gespeichert")?;
            let zeilen = abfrage.query_map([], |z| {
                Ok(FremderEintrag {
                    fingerabdruck: z.get(0)?,
                    oeffentlich: z.get(1)?,
                })
            })?;
            zeilen.collect()
        })
    }

    pub fn fremden_speichern(
        &self,
        fingerabdruck: &str,
        oeffentlich: &[u8],
    ) -> Result<(), Fehlermeldung> {
        self.mit(|v| {
            v.execute(
                "INSERT INTO fremde (fingerabdruck, oeffentlich, gespeichert) VALUES (?1, ?2, ?3)
                 ON CONFLICT(fingerabdruck) DO UPDATE SET oeffentlich = ?2",
                params![fingerabdruck, oeffentlich, jetzt()],
            )
            .map(|_| ())
        })
    }

    /// Löscht einen Schlüssel (eigenen oder fremden). Gibt zurück, ob es ihn gab.
    pub fn loeschen(&self, fingerabdruck: &str) -> Result<bool, Fehlermeldung> {
        self.mit(|v| {
            let eigene = v.execute(
                "DELETE FROM eigene WHERE fingerabdruck = ?1",
                params![fingerabdruck],
            )?;
            let fremde = v.execute(
                "DELETE FROM fremde WHERE fingerabdruck = ?1",
                params![fingerabdruck],
            )?;
            Ok(eigene + fremde > 0)
        })
    }
}

/// Öffnet `schluessel.db` beim ersten Zugriff (z. B. erst, wenn der Schlüsselbund entsperrt
/// ist). Gemeinsam für OpenPGP und S/MIME, damit es nur eine Verbindung zur Datei gibt.
pub struct SchluesselDb {
    ordner: PathBuf,
    schluesselbund: Arc<dyn Schluesselbund>,
    offen: Mutex<Option<Arc<SchluesselSpeicher>>>,
}

impl SchluesselDb {
    pub fn neu(ordner: &Path, schluesselbund: Arc<dyn Schluesselbund>) -> Self {
        Self {
            ordner: ordner.to_path_buf(),
            schluesselbund,
            offen: Mutex::new(None),
        }
    }

    pub fn oeffnen(&self) -> Result<Arc<SchluesselSpeicher>, Fehlermeldung> {
        let mut offen = self
            .offen
            .lock()
            .map_err(|_| Fehlermeldung::neu("intern", "Interner Fehler im Schlüsselspeicher."))?;
        if let Some(speicher) = offen.as_ref() {
            return Ok(speicher.clone());
        }
        let speicher = Arc::new(SchluesselSpeicher::mit_schluesselbund(
            &self.ordner,
            self.schluesselbund.as_ref(),
        )?);
        *offen = Some(speicher.clone());
        Ok(speicher)
    }
}

/// Legt eine nicht lesbare Schlüsseldatei beiseite, statt sie zu löschen.
fn beiseitelegen(pfad: &Path) {
    let mut ziel = pfad.as_os_str().to_owned();
    ziel.push(format!(".unlesbar-{}", jetzt()));
    let _ = std::fs::rename(pfad, PathBuf::from(ziel));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::konto::schluesselbund::SpeicherSchluesselbund;

    #[test]
    fn speichern_laden_loeschen_und_kein_klartext() {
        let ordner = tempfile::tempdir().unwrap();
        let bund = SpeicherSchluesselbund::default();
        let speicher = SchluesselSpeicher::mit_schluesselbund(ordner.path(), &bund).unwrap();
        speicher
            .eigenen_speichern("k1", "AAAA", b"GEHEIMER-SCHLUESSEL-123")
            .unwrap();
        speicher
            .fremden_speichern("BBBB", b"OEFFENTLICH-456")
            .unwrap();
        assert_eq!(speicher.eigene().unwrap()[0].konto, "k1");
        assert_eq!(
            speicher.fremde().unwrap()[0].oeffentlich,
            b"OEFFENTLICH-456"
        );
        drop(speicher);

        // Datei enthält keinen Klartext.
        let roh = std::fs::read(ordner.path().join(DATEINAME)).unwrap();
        assert!(!roh.windows(9).any(|w| w == b"GEHEIMER-"));
        assert!(!roh.windows(11).any(|w| w == b"OEFFENTLICH"));

        // Nach „Neustart“ noch da.
        let speicher = SchluesselSpeicher::mit_schluesselbund(ordner.path(), &bund).unwrap();
        assert_eq!(speicher.eigene().unwrap().len(), 1);
        assert!(speicher.loeschen("AAAA").unwrap());
        assert!(!speicher.loeschen("AAAA").unwrap());
        assert!(speicher.eigene().unwrap().is_empty());
        assert_eq!(speicher.fremde().unwrap().len(), 1);
    }

    #[test]
    fn ohne_datenbankschluessel_wird_nichts_geloescht() {
        let ordner = tempfile::tempdir().unwrap();
        let bund = SpeicherSchluesselbund::default();
        let speicher = SchluesselSpeicher::mit_schluesselbund(ordner.path(), &bund).unwrap();
        speicher.eigenen_speichern("k1", "AAAA", b"x").unwrap();
        drop(speicher);

        // Schlüsselbund verloren: neue, leere Datei; die alte liegt daneben.
        let neuer_bund = SpeicherSchluesselbund::default();
        let speicher = SchluesselSpeicher::mit_schluesselbund(ordner.path(), &neuer_bund).unwrap();
        assert!(speicher.eigene().unwrap().is_empty());
        let beiseite = std::fs::read_dir(ordner.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".unlesbar-"))
            .count();
        assert_eq!(beiseite, 1);
    }
}
