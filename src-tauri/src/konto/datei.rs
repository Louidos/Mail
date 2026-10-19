//! Konfigurationsdatei `konten.json` (ohne Passwörter).

use std::fs;
use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::Konto;
use crate::fehler::Fehlermeldung;

const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct KontenDatei {
    version: u32,
    konten: Vec<Konto>,
}

pub(super) fn laden(pfad: &Path) -> Result<Vec<Konto>, Fehlermeldung> {
    let inhalt = match fs::read_to_string(pfad) {
        Ok(inhalt) => inhalt,
        Err(fehler) if fehler.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => {
            return Err(Fehlermeldung::neu(
                "datei",
                "Die Kontoeinstellungen konnten nicht gelesen werden.",
            ))
        }
    };
    let datei: KontenDatei = serde_json::from_str(&inhalt)
        .map_err(|_| Fehlermeldung::neu("datei", "Die Kontoeinstellungen sind beschädigt."))?;
    if datei.version > VERSION {
        return Err(Fehlermeldung::neu(
            "datei",
            "Die Kontoeinstellungen stammen von einer neueren SicherMail-Version.",
        ));
    }
    Ok(datei.konten)
}

/// Schreibt die Datei atomar (erst in eine temporäre Datei, dann umbenennen),
/// unter Unix nur für den eigenen Benutzer lesbar.
pub(super) fn speichern(pfad: &Path, konten: &[Konto]) -> Result<(), Fehlermeldung> {
    let fehler = |_| {
        Fehlermeldung::neu(
            "datei",
            "Die Kontoeinstellungen konnten nicht gespeichert werden.",
        )
    };
    if let Some(ordner) = pfad.parent() {
        fs::create_dir_all(ordner).map_err(fehler)?;
    }
    let inhalt = serde_json::to_vec_pretty(&KontenDatei {
        version: VERSION,
        konten: konten.to_vec(),
    })
    .map_err(|_| Fehlermeldung::neu("datei", "Interner Fehler beim Speichern."))?;

    let temp = pfad.with_extension("json.neu");
    {
        let mut optionen = fs::OpenOptions::new();
        optionen.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            optionen.mode(0o600);
        }
        let mut datei = optionen.open(&temp).map_err(fehler)?;
        datei.write_all(&inhalt).map_err(fehler)?;
        datei.sync_all().map_err(fehler)?;
    }
    fs::rename(&temp, pfad).map_err(fehler)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::konto::{ServerEinstellung, Verschluesselung};

    fn konto(id: &str) -> Konto {
        let server = ServerEinstellung {
            host: "mail.example.org".into(),
            port: 993,
            verschluesselung: Verschluesselung::Tls,
            zertifikat: None,
        };
        Konto {
            id: id.into(),
            name: "Test".into(),
            email: "test@example.org".into(),
            benutzername: "test".into(),
            imap: server.clone(),
            smtp: server,
        }
    }

    #[test]
    fn fehlende_datei_heisst_keine_konten() {
        let ordner = tempfile::tempdir().unwrap();
        assert!(laden(&ordner.path().join("konten.json"))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn speichern_und_laden() {
        let ordner = tempfile::tempdir().unwrap();
        let pfad = ordner.path().join("unter").join("konten.json");
        speichern(&pfad, &[konto("a"), konto("b")]).unwrap();
        let geladen = laden(&pfad).unwrap();
        assert_eq!(geladen, vec![konto("a"), konto("b")]);
        assert!(!pfad.with_extension("json.neu").exists());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let rechte = fs::metadata(&pfad).unwrap().permissions().mode() & 0o777;
            assert_eq!(rechte, 0o600);
        }
    }

    #[test]
    fn beschaedigte_datei_gibt_fehler() {
        let ordner = tempfile::tempdir().unwrap();
        let pfad = ordner.path().join("konten.json");
        fs::write(&pfad, "{kaputt").unwrap();
        assert!(laden(&pfad).is_err());
    }
}
