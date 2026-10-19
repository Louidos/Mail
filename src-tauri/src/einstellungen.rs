//! Allgemeine Einstellungen (`einstellungen.json` im App-Datenordner).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::fehler::Fehlermeldung;

/// Gespeicherte Einstellungen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Einstellungen {
    /// Mails verschlüsselt lokal speichern (für das Lesen ohne Internet). Standard: an.
    #[serde(default = "standard_an")]
    pub lokal_speichern: bool,
    /// Absender (kleingeschrieben), bei denen externe Inhalte immer geladen werden.
    #[serde(default)]
    pub externe_inhalte_erlaubt: Vec<String>,
}

fn standard_an() -> bool {
    true
}

impl Default for Einstellungen {
    fn default() -> Self {
        Self {
            lokal_speichern: true,
            externe_inhalte_erlaubt: Vec::new(),
        }
    }
}

pub struct EinstellungsSpeicher {
    pfad: PathBuf,
    sperre: Mutex<()>,
}

impl EinstellungsSpeicher {
    pub fn neu(ordner: &Path) -> Self {
        Self {
            pfad: ordner.join("einstellungen.json"),
            sperre: Mutex::new(()),
        }
    }

    fn laden_ohne_sperre(&self) -> Einstellungen {
        std::fs::read(&self.pfad)
            .ok()
            .and_then(|daten| serde_json::from_slice(&daten).ok())
            .unwrap_or_default()
    }

    pub async fn laden(&self) -> Einstellungen {
        let _sperre = self.sperre.lock().await;
        self.laden_ohne_sperre()
    }

    /// Ändert die Einstellungen und speichert sie.
    pub async fn aendern(
        &self,
        aenderung: impl FnOnce(&mut Einstellungen),
    ) -> Result<Einstellungen, Fehlermeldung> {
        let _sperre = self.sperre.lock().await;
        let mut einstellungen = self.laden_ohne_sperre();
        aenderung(&mut einstellungen);
        let inhalt = serde_json::to_vec_pretty(&einstellungen)
            .map_err(|_| Fehlermeldung::neu("datei", "Interner Fehler beim Speichern."))?;
        crate::dateien::atomar_schreiben(&self.pfad, &inhalt).map_err(|_| {
            Fehlermeldung::neu(
                "datei",
                "Die Einstellungen konnten nicht gespeichert werden.",
            )
        })?;
        Ok(einstellungen)
    }

    /// Sind externe Inhalte für diesen Absender immer erlaubt?
    pub async fn extern_erlaubt(&self, absender: &str) -> bool {
        let absender = absender.trim().to_lowercase();
        !absender.is_empty()
            && self
                .laden()
                .await
                .externe_inhalte_erlaubt
                .contains(&absender)
    }

    /// Nimmt einen Absender aus der Liste „externe Inhalte immer erlauben“.
    pub async fn extern_entfernen(&self, absender: &str) -> Result<(), Fehlermeldung> {
        let absender = absender.trim().to_lowercase();
        self.aendern(|e| e.externe_inhalte_erlaubt.retain(|a| *a != absender))
            .await?;
        Ok(())
    }

    /// Erlaubt externe Inhalte für einen Absender dauerhaft.
    pub async fn extern_erlauben(&self, absender: &str) -> Result<(), Fehlermeldung> {
        let absender = absender.trim().to_lowercase();
        if absender.is_empty() || !absender.contains('@') {
            return Err(Fehlermeldung::neu("eingabe", "Ungültige Absenderadresse."));
        }
        self.aendern(|e| {
            if !e.externe_inhalte_erlaubt.contains(&absender) {
                e.externe_inhalte_erlaubt.push(absender);
                e.externe_inhalte_erlaubt.sort();
            }
        })
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn lokal_speichern_ist_standardmaessig_an() {
        let ordner = tempfile::tempdir().unwrap();
        let speicher = EinstellungsSpeicher::neu(ordner.path());
        assert!(speicher.laden().await.lokal_speichern);
        // Ältere Datei ohne das Feld
        std::fs::write(
            ordner.path().join("einstellungen.json"),
            r#"{"externe_inhalte_erlaubt":[]}"#,
        )
        .unwrap();
        assert!(speicher.laden().await.lokal_speichern);
        speicher
            .aendern(|e| e.lokal_speichern = false)
            .await
            .unwrap();
        assert!(
            !EinstellungsSpeicher::neu(ordner.path())
                .laden()
                .await
                .lokal_speichern
        );
    }

    #[tokio::test]
    async fn absender_erlauben() {
        let ordner = tempfile::tempdir().unwrap();
        let speicher = EinstellungsSpeicher::neu(ordner.path());
        assert!(!speicher.extern_erlaubt("Anna@Example.org").await);
        speicher.extern_erlauben("Anna@Example.org").await.unwrap();
        speicher.extern_erlauben("anna@example.org").await.unwrap();
        assert!(speicher.extern_erlaubt("anna@example.org ").await);
        assert_eq!(speicher.laden().await.externe_inhalte_erlaubt.len(), 1);
        assert!(speicher.extern_erlauben("keine-adresse").await.is_err());
        // Neu laden aus der Datei
        let neu = EinstellungsSpeicher::neu(ordner.path());
        assert!(neu.extern_erlaubt("ANNA@example.org").await);
    }
}
