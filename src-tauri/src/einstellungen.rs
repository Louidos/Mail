//! Allgemeine Einstellungen (`einstellungen.json` im App-Datenordner).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize};
use tokio::sync::Mutex;

use crate::anzeige::absender::Stufe;
use crate::anzeige::extern_inhalte::ProxyAdresse;
use crate::anzeige::AufbereiteteMail;
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
    /// Externe Bilder über einen eigenen Proxy laden (#46). Standard: aus – dann nie über einen
    /// Proxy, auch nicht über `HTTPS_PROXY` usw.
    #[serde(default)]
    pub proxy_externe_bilder: bool,
    /// Proxy-Adresse (`http://host:port` oder `https://host:port`), nur wirksam mit
    /// `proxy_externe_bilder`. Wird beim Setzen geprüft.
    #[serde(default)]
    pub proxy_adresse: String,
    /// Hell, dunkel oder wie das System (#47). Standard: wie das System.
    #[serde(default, deserialize_with = "darstellung_nachsichtig")]
    pub darstellung: Darstellung,
}

fn standard_an() -> bool {
    true
}

/// Setzt `data-darstellung`, sobald es das `<html>`-Element gibt (das Skript läuft, bevor die
/// Seite gelesen wird).
macro_rules! start_skript {
    ($wert:literal) => {
        concat!(
            "(function(){var s=function(){document.documentElement.dataset.darstellung=\"",
            $wert,
            "\";};if(document.documentElement){s();return;}",
            "new MutationObserver(function(m,o){if(document.documentElement){s();o.disconnect();}})",
            ".observe(document,{childList:true});})();"
        )
    };
}

/// Darstellung der Oberfläche (#47).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Darstellung {
    /// Folgt der Systemeinstellung.
    #[default]
    Automatisch,
    Hell,
    Dunkel,
}

impl Darstellung {
    /// Fenster-Thema für Tauri: `None` heisst „wie das System“.
    pub fn fenster_thema(self) -> Option<tauri::Theme> {
        match self {
            Darstellung::Automatisch => None,
            Darstellung::Hell => Some(tauri::Theme::Light),
            Darstellung::Dunkel => Some(tauri::Theme::Dark),
        }
    }

    /// Skript, das die Wahl vor dem ersten Zeichnen am `<html>`-Element setzt (siehe
    /// `src/einstellungen/darstellung.ts`). Bei „Automatisch“ genügt das CSS.
    pub fn start_skript(self) -> Option<&'static str> {
        match self {
            Darstellung::Automatisch => None,
            Darstellung::Hell => Some(start_skript!("hell")),
            Darstellung::Dunkel => Some(start_skript!("dunkel")),
        }
    }
}

/// Unbekannte Werte (z. B. von Hand verändert) ergeben „Automatisch“, statt die ganze
/// Einstellungsdatei unlesbar zu machen.
fn darstellung_nachsichtig<'de, D: Deserializer<'de>>(d: D) -> Result<Darstellung, D::Error> {
    let wert = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value(wert).unwrap_or_default())
}

impl Default for Einstellungen {
    fn default() -> Self {
        Self {
            lokal_speichern: true,
            externe_inhalte_erlaubt: Vec::new(),
            proxy_externe_bilder: false,
            proxy_adresse: String::new(),
            darstellung: Darstellung::Automatisch,
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

    /// Proxy für externe Bilder, falls eingeschaltet und gültig. `Err`, wenn er eingeschaltet,
    /// aber die Adresse ungültig ist (dann werden keine Bilder geladen).
    pub async fn bilder_proxy(&self) -> Result<Option<ProxyAdresse>, Fehlermeldung> {
        let einstellungen = self.laden().await;
        if !einstellungen.proxy_externe_bilder {
            return Ok(None);
        }
        ProxyAdresse::pruefen(&einstellungen.proxy_adresse).map(Some)
    }

    /// Schaltet den Proxy für externe Bilder ein oder aus. Beim Einschalten muss die Adresse
    /// gültig sein, sonst wird nichts gespeichert.
    pub async fn proxy_setzen(&self, an: bool, adresse: &str) -> Result<(), Fehlermeldung> {
        let adresse = adresse.trim();
        if an {
            ProxyAdresse::pruefen(adresse)?;
        }
        self.aendern(|e| {
            e.proxy_externe_bilder = an;
            e.proxy_adresse = adresse.to_string();
        })
        .await?;
        Ok(())
    }

    /// Gilt die dauerhafte Freigabe externer Inhalte für den Absender dieser Mail? Nie, wenn der
    /// eigene Server den Absender als wahrscheinlich gefälscht erkannt hat (#45).
    pub async fn extern_erlaubt_fuer(&self, mail: &AufbereiteteMail) -> bool {
        if mail.absender_pruefung.stufe == Stufe::Warnung {
            return false;
        }
        let absender = mail.von.first().map(|a| a.adresse.as_str()).unwrap_or("");
        self.extern_erlaubt(absender).await
    }

    /// Speichert die Darstellung (#47).
    pub async fn darstellung_setzen(&self, darstellung: Darstellung) -> Result<(), Fehlermeldung> {
        self.aendern(|e| e.darstellung = darstellung).await?;
        Ok(())
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
    async fn proxy_ist_standardmaessig_aus_und_alte_dateien_laden() {
        let ordner = tempfile::tempdir().unwrap();
        // Datei einer älteren Version ohne die Proxy-Felder
        std::fs::write(
            ordner.path().join("einstellungen.json"),
            r#"{"lokal_speichern":false,"externe_inhalte_erlaubt":["anna@example.org"]}"#,
        )
        .unwrap();
        let speicher = EinstellungsSpeicher::neu(ordner.path());
        let geladen = speicher.laden().await;
        assert!(!geladen.lokal_speichern);
        assert_eq!(geladen.externe_inhalte_erlaubt, vec!["anna@example.org"]);
        assert!(!geladen.proxy_externe_bilder);
        assert!(geladen.proxy_adresse.is_empty());
        assert_eq!(speicher.bilder_proxy().await.unwrap(), None);
    }

    #[tokio::test]
    async fn darstellung_ist_standardmaessig_automatisch_und_alte_dateien_laden() {
        let ordner = tempfile::tempdir().unwrap();
        let speicher = EinstellungsSpeicher::neu(ordner.path());
        assert_eq!(speicher.laden().await.darstellung, Darstellung::Automatisch);
        // Datei von Version 0.3.0 ohne das Feld
        std::fs::write(
            ordner.path().join("einstellungen.json"),
            r#"{"lokal_speichern":false,"externe_inhalte_erlaubt":["anna@example.org"],"proxy_externe_bilder":false,"proxy_adresse":""}"#,
        )
        .unwrap();
        let geladen = speicher.laden().await;
        assert_eq!(geladen.darstellung, Darstellung::Automatisch);
        assert!(!geladen.lokal_speichern);
        assert_eq!(geladen.externe_inhalte_erlaubt, vec!["anna@example.org"]);

        speicher
            .darstellung_setzen(Darstellung::Dunkel)
            .await
            .unwrap();
        let inhalt = std::fs::read_to_string(ordner.path().join("einstellungen.json")).unwrap();
        assert!(inhalt.contains(r#""darstellung": "dunkel""#), "{inhalt}");
        let neu = EinstellungsSpeicher::neu(ordner.path()).laden().await;
        assert_eq!(neu.darstellung, Darstellung::Dunkel);
        assert!(!neu.lokal_speichern);

        // Unbekannter Wert: nur die Darstellung fällt auf „Automatisch“ zurück, der Rest bleibt.
        std::fs::write(
            ordner.path().join("einstellungen.json"),
            r#"{"lokal_speichern":false,"externe_inhalte_erlaubt":["anna@example.org"],"darstellung":"lila"}"#,
        )
        .unwrap();
        let verdorben = speicher.laden().await;
        assert_eq!(verdorben.darstellung, Darstellung::Automatisch);
        assert_eq!(verdorben.externe_inhalte_erlaubt, vec!["anna@example.org"]);
    }

    #[test]
    fn darstellung_fuer_fenster_und_startskript() {
        assert_eq!(Darstellung::Automatisch.fenster_thema(), None);
        assert_eq!(
            Darstellung::Dunkel.fenster_thema(),
            Some(tauri::Theme::Dark)
        );
        assert_eq!(Darstellung::Hell.fenster_thema(), Some(tauri::Theme::Light));
        assert_eq!(Darstellung::Automatisch.start_skript(), None);
        let skript = Darstellung::Dunkel.start_skript().unwrap();
        assert!(
            skript.contains(r#"dataset.darstellung="dunkel""#),
            "{skript}"
        );
        assert!(Darstellung::Hell
            .start_skript()
            .unwrap()
            .contains(r#"dataset.darstellung="hell""#));
    }

    #[tokio::test]
    async fn proxy_nur_mit_gueltiger_adresse() {
        let ordner = tempfile::tempdir().unwrap();
        let speicher = EinstellungsSpeicher::neu(ordner.path());
        assert!(speicher
            .proxy_setzen(true, "http://anna:geheim@proxy.example.org:3128")
            .await
            .is_err());
        assert!(!speicher.laden().await.proxy_externe_bilder);

        speicher
            .proxy_setzen(true, " http://proxy.example.org:3128 ")
            .await
            .unwrap();
        let neu = EinstellungsSpeicher::neu(ordner.path());
        assert_eq!(
            neu.bilder_proxy().await.unwrap().unwrap().als_text(),
            "http://proxy.example.org:3128/"
        );

        // Ausschalten behält die Adresse, nutzt sie aber nicht mehr.
        speicher
            .proxy_setzen(false, "http://proxy.example.org:3128")
            .await
            .unwrap();
        assert_eq!(speicher.bilder_proxy().await.unwrap(), None);
        assert_eq!(
            speicher.laden().await.proxy_adresse,
            "http://proxy.example.org:3128"
        );

        // Von Hand verdorbene Datei: eingeschaltet, aber ungültig → keine Bilder.
        speicher
            .aendern(|e| {
                e.proxy_externe_bilder = true;
                e.proxy_adresse = "socks5://x:1".into();
            })
            .await
            .unwrap();
        assert!(speicher.bilder_proxy().await.is_err());
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

    #[tokio::test]
    async fn freigabe_gilt_nicht_bei_gefaelschtem_absender() {
        let ordner = tempfile::tempdir().unwrap();
        let speicher = EinstellungsSpeicher::neu(ordner.path());
        speicher
            .extern_erlauben("konto@bank.example")
            .await
            .unwrap();
        let roh = "Authentication-Results: mail.example.ch; dmarc=fail header.from=bank.example\r\n\
From: Bank <konto@bank.example>\r\nSubject: x\r\nContent-Type: text/html\r\n\r\n<img src=\"https://t.example/p.gif\">\r\n";
        let mut mail = crate::anzeige::aufbereiten(roh.as_bytes(), None).unwrap();
        mail.absender_pruefung =
            crate::anzeige::absender_pruefen(roh.as_bytes(), "mail.example.ch").unwrap();
        assert_eq!(mail.absender_pruefung.stufe, Stufe::Warnung);
        assert!(!speicher.extern_erlaubt_fuer(&mail).await);

        // Andere Kennung: Header wird ignoriert, Freigabe gilt wie bisher.
        mail.absender_pruefung =
            crate::anzeige::absender_pruefen(roh.as_bytes(), "mx.anders.example").unwrap();
        assert_eq!(mail.absender_pruefung.stufe, Stufe::NichtGeprueft);
        assert!(speicher.extern_erlaubt_fuer(&mail).await);
    }
}
