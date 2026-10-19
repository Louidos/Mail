//! Register der vom Nutzer gewählten Anhänge.
//!
//! Das Frontend bekommt nie Dateipfade, sondern nur Kennungen (Token). Dateien gelangen
//! ausschliesslich über den Dialog „Datei anhängen“ (im Backend) oder als Anhang einer
//! weitergeleiteten Mail in das Register. So kann auch eingeschleuster Code in der
//! Oberfläche keine beliebigen Dateien des Rechners verschicken.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::fehler::Fehlermeldung;

/// Höchstgrösse aller Anhänge einer Mail zusammen.
pub const MAX_GESAMT: u64 = 25 * 1024 * 1024;

/// Woher ein Anhang stammt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "art", rename_all = "snake_case")]
pub enum Quelle {
    /// Vom Nutzer im Dialog gewählte Datei.
    Datei { pfad: PathBuf },
    /// Anhang einer vorhandenen Mail (beim Weiterleiten).
    Mail {
        konto: String,
        ordner: String,
        uid: u32,
        index: usize,
    },
}

/// Eingetragener Anhang.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Eintrag {
    pub name: String,
    pub typ: String,
    pub groesse: u64,
    pub quelle: Quelle,
}

/// Was die Oberfläche über einen Anhang erfährt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnhangAuswahl {
    pub token: String,
    pub name: String,
    pub typ: String,
    pub groesse: u64,
}

#[derive(Default)]
pub struct AnhangRegister {
    eintraege: Mutex<HashMap<String, Eintrag>>,
}

impl AnhangRegister {
    /// Trägt einen Anhang ein und gibt die Auswahl (mit neuem Token) zurück.
    pub fn eintragen(&self, eintrag: Eintrag) -> AnhangAuswahl {
        let token = uuid::Uuid::new_v4().to_string();
        let auswahl = AnhangAuswahl {
            token: token.clone(),
            name: eintrag.name.clone(),
            typ: eintrag.typ.clone(),
            groesse: eintrag.groesse,
        };
        if let Ok(mut eintraege) = self.eintraege.lock() {
            eintraege.insert(token, eintrag);
        }
        auswahl
    }

    /// Trägt eine vom Nutzer gewählte Datei ein.
    pub fn datei_eintragen(&self, pfad: PathBuf) -> Result<AnhangAuswahl, Fehlermeldung> {
        let metadaten = std::fs::metadata(&pfad)
            .map_err(|_| Fehlermeldung::neu("anhang", "Die Datei konnte nicht gelesen werden."))?;
        if !metadaten.is_file() {
            return Err(Fehlermeldung::neu("anhang", "Bitte eine Datei auswählen."));
        }
        let name = pfad
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "anhang".into());
        Ok(self.eintragen(Eintrag {
            name: crate::dateien::dateiname_saeubern(&name, "anhang"),
            typ: typ_aus_name(&name).to_string(),
            groesse: metadaten.len(),
            quelle: Quelle::Datei { pfad },
        }))
    }

    pub fn holen(&self, token: &str) -> Result<Eintrag, Fehlermeldung> {
        self.eintraege
            .lock()
            .ok()
            .and_then(|e| e.get(token).cloned())
            .ok_or_else(|| {
                Fehlermeldung::neu(
                    "anhang",
                    "Ein Anhang ist nicht mehr verfügbar. Bitte erneut hinzufügen.",
                )
            })
    }
}

/// Inhaltstyp anhand der Dateiendung (für den MIME-Kopf).
pub fn typ_aus_name(name: &str) -> &'static str {
    let endung = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match endung.as_str() {
        "pdf" => "application/pdf",
        "txt" => "text/plain",
        "csv" => "text/csv",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "zip" => "application/zip",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "odt" => "application/vnd.oasis.opendocument.text",
        "ods" => "application/vnd.oasis.opendocument.spreadsheet",
        "ics" => "text/calendar",
        "eml" => "message/rfc822",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nur_eingetragene_anhaenge_sind_verfuegbar() {
        let register = AnhangRegister::default();
        let ordner = tempfile::tempdir().unwrap();
        let pfad = ordner.path().join("Bericht.pdf");
        std::fs::write(&pfad, b"%PDF").unwrap();
        let auswahl = register.datei_eintragen(pfad.clone()).unwrap();
        assert_eq!(auswahl.name, "Bericht.pdf");
        assert_eq!(auswahl.typ, "application/pdf");
        assert_eq!(auswahl.groesse, 4);
        assert_eq!(
            register.holen(&auswahl.token).unwrap().quelle,
            Quelle::Datei { pfad }
        );
        assert!(register.holen("erfunden").is_err());
        assert!(register
            .datei_eintragen(ordner.path().to_path_buf())
            .is_err());
    }
}
