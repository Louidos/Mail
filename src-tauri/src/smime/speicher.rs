//! S/MIME-Zertifikate in `schluessel.db` (gemeinsam mit OpenPGP, siehe `crate::pgp::speicher`).
//!
//! - `smime_eigene`: eigene Zertifikate mit Kette und dem privaten Schlüssel, zusätzlich mit der
//!   Passphrase geschützt (siehe `zertifikat::schluessel_schuetzen`).
//! - `smime_fremde`: Zertifikate anderer (zum Verschlüsseln an sie).
//! - `smime_wurzeln`: vom Nutzer ausdrücklich übernommene Wurzelzertifikate.
//!
//! Die Tabellen werden beim Öffnen ergänzt; ältere Dateien ohne sie öffnen weiter.

use rusqlite::{params, Connection};
use zeroize::Zeroizing;

use crate::fehler::Fehlermeldung;
use crate::pgp::speicher::SchluesselSpeicher;

/// Legt die S/MIME-Tabellen an, falls sie fehlen (Migration älterer `schluessel.db`).
pub fn tabellen_anlegen(verbindung: &Connection) -> rusqlite::Result<()> {
    verbindung.execute_batch(
        "CREATE TABLE IF NOT EXISTS smime_eigene (
             fingerabdruck TEXT PRIMARY KEY,
             konto TEXT NOT NULL,
             zertifikat BLOB NOT NULL,
             kette BLOB NOT NULL,
             geheim BLOB NOT NULL,
             gespeichert INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS smime_fremde (
             fingerabdruck TEXT PRIMARY KEY,
             zertifikat BLOB NOT NULL,
             gespeichert INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS smime_wurzeln (
             fingerabdruck TEXT PRIMARY KEY,
             zertifikat BLOB NOT NULL,
             gespeichert INTEGER NOT NULL
         );",
    )
}

/// Ein eigenes Zertifikat.
pub struct EigenerEintrag {
    pub konto: String,
    pub fingerabdruck: String,
    /// DER
    pub zertifikat: Vec<u8>,
    /// Zwischenzertifikate als PEM (kann leer sein).
    pub kette: Vec<u8>,
    /// Mit der Passphrase geschützter privater Schlüssel.
    pub geheim: Zeroizing<Vec<u8>>,
}

/// Tabelle für Zertifikate ohne privaten Schlüssel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Art {
    Fremde,
    Wurzeln,
}

impl Art {
    fn tabelle(self) -> &'static str {
        match self {
            Art::Fremde => "smime_fremde",
            Art::Wurzeln => "smime_wurzeln",
        }
    }
}

fn jetzt() -> i64 {
    super::zertifikat::jetzt()
}

/// Zugriff auf die S/MIME-Tabellen.
pub trait SmimeSpeicher {
    fn smime_eigene(&self) -> Result<Vec<EigenerEintrag>, Fehlermeldung>;
    fn smime_eigenen_speichern(&self, eintrag: &EigenerEintrag) -> Result<(), Fehlermeldung>;
    /// DER-Zertifikate einer Tabelle (älteste zuerst).
    fn smime_zertifikate(&self, art: Art) -> Result<Vec<Vec<u8>>, Fehlermeldung>;
    fn smime_zertifikat_speichern(
        &self,
        art: Art,
        fingerabdruck: &str,
        zertifikat: &[u8],
    ) -> Result<(), Fehlermeldung>;
    /// Löscht ein Zertifikat aus allen S/MIME-Tabellen. Gibt zurück, ob es eines gab.
    fn smime_loeschen(&self, fingerabdruck: &str) -> Result<bool, Fehlermeldung>;
}

impl SmimeSpeicher for SchluesselSpeicher {
    fn smime_eigene(&self) -> Result<Vec<EigenerEintrag>, Fehlermeldung> {
        self.mit(|v| {
            let mut abfrage = v.prepare(
                "SELECT konto, fingerabdruck, zertifikat, kette, geheim FROM smime_eigene ORDER BY gespeichert",
            )?;
            let zeilen = abfrage.query_map([], |z| {
                Ok(EigenerEintrag {
                    konto: z.get(0)?,
                    fingerabdruck: z.get(1)?,
                    zertifikat: z.get(2)?,
                    kette: z.get(3)?,
                    geheim: Zeroizing::new(z.get(4)?),
                })
            })?;
            zeilen.collect()
        })
    }

    fn smime_eigenen_speichern(&self, e: &EigenerEintrag) -> Result<(), Fehlermeldung> {
        self.mit(|v| {
            v.execute(
                "INSERT INTO smime_eigene (fingerabdruck, konto, zertifikat, kette, geheim, gespeichert)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(fingerabdruck) DO UPDATE SET konto = ?2, zertifikat = ?3, kette = ?4, geheim = ?5",
                params![
                    e.fingerabdruck,
                    e.konto,
                    e.zertifikat,
                    e.kette,
                    &e.geheim[..],
                    jetzt()
                ],
            )
            .map(|_| ())
        })
    }

    fn smime_zertifikate(&self, art: Art) -> Result<Vec<Vec<u8>>, Fehlermeldung> {
        self.mit(|v| {
            let mut abfrage = v.prepare(&format!(
                "SELECT zertifikat FROM {} ORDER BY gespeichert",
                art.tabelle()
            ))?;
            let zeilen = abfrage.query_map([], |z| z.get(0))?;
            zeilen.collect()
        })
    }

    fn smime_zertifikat_speichern(
        &self,
        art: Art,
        fingerabdruck: &str,
        zertifikat: &[u8],
    ) -> Result<(), Fehlermeldung> {
        self.mit(|v| {
            v.execute(
                &format!(
                    "INSERT INTO {} (fingerabdruck, zertifikat, gespeichert) VALUES (?1, ?2, ?3)
                     ON CONFLICT(fingerabdruck) DO UPDATE SET zertifikat = ?2",
                    art.tabelle()
                ),
                params![fingerabdruck, zertifikat, jetzt()],
            )
            .map(|_| ())
        })
    }

    fn smime_loeschen(&self, fingerabdruck: &str) -> Result<bool, Fehlermeldung> {
        self.mit(|v| {
            let mut anzahl = 0;
            for tabelle in ["smime_eigene", "smime_fremde", "smime_wurzeln"] {
                anzahl += v.execute(
                    &format!("DELETE FROM {tabelle} WHERE fingerabdruck = ?1"),
                    params![fingerabdruck],
                )?;
            }
            Ok(anzahl > 0)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::konto::schluesselbund::SpeicherSchluesselbund;
    use crate::pgp::speicher::DATEINAME;

    #[test]
    fn alte_schluessel_db_bekommt_die_tabellen_dazu() {
        let ordner = tempfile::tempdir().unwrap();
        let bund = SpeicherSchluesselbund::default();
        // Datei einer älteren Version: nur die OpenPGP-Tabellen.
        let (schluessel, _) = crate::speicher::schluessel_holen_fuer(
            &bund,
            crate::konto::schluesselbund::PGP_SCHLUESSEL_EINTRAG,
        )
        .unwrap();
        {
            let alt = crate::speicher::verschluesselt_oeffnen(
                &ordner.path().join(DATEINAME),
                &schluessel,
            )
            .unwrap();
            alt.execute_batch(
                "CREATE TABLE eigene (fingerabdruck TEXT PRIMARY KEY, konto TEXT NOT NULL, geheim BLOB NOT NULL, gespeichert INTEGER NOT NULL);
                 CREATE TABLE fremde (fingerabdruck TEXT PRIMARY KEY, oeffentlich BLOB NOT NULL, gespeichert INTEGER NOT NULL);
                 INSERT INTO eigene VALUES ('AAAA', 'k1', x'01', 1);",
            )
            .unwrap();
        }
        let speicher = SchluesselSpeicher::mit_schluesselbund(ordner.path(), &bund).unwrap();
        // OpenPGP-Schlüssel bleibt, S/MIME-Tabellen sind neu und leer.
        assert_eq!(speicher.eigene().unwrap().len(), 1);
        assert!(speicher.smime_eigene().unwrap().is_empty());

        speicher
            .smime_eigenen_speichern(&EigenerEintrag {
                konto: "k1".into(),
                fingerabdruck: "CCCC".into(),
                zertifikat: b"ZERT".to_vec(),
                kette: Vec::new(),
                geheim: Zeroizing::new(b"GEHEIM-SMIME-789".to_vec()),
            })
            .unwrap();
        speicher
            .smime_zertifikat_speichern(Art::Wurzeln, "DDDD", b"WURZEL")
            .unwrap();
        assert_eq!(
            speicher.smime_zertifikate(Art::Wurzeln).unwrap(),
            vec![b"WURZEL".to_vec()]
        );
        assert!(speicher.smime_zertifikate(Art::Fremde).unwrap().is_empty());
        drop(speicher);
        let roh = std::fs::read(ordner.path().join(DATEINAME)).unwrap();
        assert!(!roh.windows(12).any(|w| w == b"GEHEIM-SMIME"));

        let speicher = SchluesselSpeicher::mit_schluesselbund(ordner.path(), &bund).unwrap();
        assert_eq!(speicher.smime_eigene().unwrap()[0].konto, "k1");
        assert!(speicher.smime_loeschen("DDDD").unwrap());
        assert!(!speicher.smime_loeschen("DDDD").unwrap());
        // Der OpenPGP-Schlüssel ist davon nicht betroffen.
        assert_eq!(speicher.eigene().unwrap().len(), 1);
    }
}
