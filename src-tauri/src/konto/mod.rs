//! Konten mit frei wählbarem IMAP- und SMTP-Server.
//!
//! Kontodaten (ohne Passwort) liegen in `konten.json` im App-Datenordner,
//! das Passwort ausschliesslich im Schlüsselbund des Betriebssystems.

pub mod autoconfig;
mod datei;
mod pruefung;
pub mod schluesselbund;

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use zeroize::Zeroizing;

use crate::fehler::Fehlermeldung;
pub use pruefung::{verbindung_testen, Pruefergebnis};
use schluesselbund::Schluesselbund;

/// Art der Verschlüsselung. Unverschlüsselte Verbindungen gibt es nicht.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verschluesselung {
    /// TLS ab dem ersten Byte (IMAP 993, SMTP 465).
    Tls,
    /// Verbindung startet unverschlüsselt und wird sofort per STARTTLS verschlüsselt
    /// (IMAP 143, SMTP 587). Bietet der Server kein STARTTLS an, wird abgebrochen.
    Starttls,
}

/// Einstellungen für einen Server (IMAP oder SMTP).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerEinstellung {
    pub host: String,
    pub port: u16,
    pub verschluesselung: Verschluesselung,
    /// Optional: bewusst hinterlegtes Serverzertifikat (PEM) für selbstsignierte Server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zertifikat: Option<String>,
}

/// Ein eingerichtetes Konto, wie es gespeichert wird (ohne Passwort).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Konto {
    pub id: String,
    /// Anzeigename, z. B. „Anna Muster“.
    pub name: String,
    pub email: String,
    pub benutzername: String,
    pub imap: ServerEinstellung,
    pub smtp: ServerEinstellung,
    /// Gesendete Mails per IMAP im Ordner „Gesendet“ ablegen. Ausschalten, wenn der
    /// Server das selbst tut (sonst liegen sie doppelt dort).
    #[serde(default = "standard_an")]
    pub gesendet_ablegen: bool,
}

fn standard_an() -> bool {
    true
}

/// Eingabe aus dem Einrichtungsdialog.
#[derive(Clone, Deserialize)]
pub struct KontoEingabe {
    /// Leer beim Anlegen, gesetzt beim Bearbeiten.
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    pub email: String,
    pub benutzername: String,
    /// Beim Bearbeiten optional: leer heisst „bisheriges Passwort behalten“.
    #[serde(default)]
    pub passwort: Option<Zeroizing<String>>,
    pub imap: ServerEinstellung,
    pub smtp: ServerEinstellung,
    #[serde(default = "standard_an")]
    pub gesendet_ablegen: bool,
}

// Eigene Debug-Ausgabe, damit das Passwort nie in Logs oder Fehlermeldungen landet.
impl fmt::Debug for KontoEingabe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KontoEingabe")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("email", &self.email)
            .field("benutzername", &self.benutzername)
            .field("passwort", &self.passwort.as_ref().map(|_| "***"))
            .field("imap", &self.imap)
            .field("smtp", &self.smtp)
            .field("gesendet_ablegen", &self.gesendet_ablegen)
            .finish()
    }
}

impl KontoEingabe {
    /// Prüft die Eingabe auf offensichtliche Fehler.
    pub fn pruefen(&self) -> Result<(), Fehlermeldung> {
        let fehler = |text: &str| Err(Fehlermeldung::neu("eingabe", text));
        if self.name.trim().is_empty() {
            return fehler("Bitte einen Namen angeben.");
        }
        let email = self.email.trim();
        match email.split_once('@') {
            Some((lokal, domain))
                if !lokal.is_empty()
                    && domain.contains('.')
                    && !domain.contains('@')
                    && !email.contains(char::is_whitespace) => {}
            _ => return fehler("Bitte eine gültige E-Mail-Adresse angeben."),
        }
        if self.benutzername.trim().is_empty() {
            return fehler("Bitte einen Benutzernamen angeben.");
        }
        for (art, server) in [("IMAP", &self.imap), ("SMTP", &self.smtp)] {
            let host = server.host.trim();
            if host.is_empty()
                || host.contains(char::is_whitespace)
                || host.contains('/')
                || host.contains(':')
            {
                return Err(Fehlermeldung::neu(
                    "eingabe",
                    format!("Bitte einen gültigen {art}-Server angeben (nur der Name, z. B. mail.example.org)."),
                ));
            }
            if server.port == 0 {
                return Err(Fehlermeldung::neu(
                    "eingabe",
                    format!("Bitte einen gültigen {art}-Port angeben."),
                ));
            }
        }
        Ok(())
    }

    fn in_konto(&self, id: String) -> Konto {
        let saeubern = |s: &ServerEinstellung| ServerEinstellung {
            host: s.host.trim().to_string(),
            port: s.port,
            verschluesselung: s.verschluesselung,
            zertifikat: s
                .zertifikat
                .as_ref()
                .map(|z| z.trim().to_string())
                .filter(|z| !z.is_empty()),
        };
        Konto {
            id,
            name: self.name.trim().to_string(),
            email: self.email.trim().to_string(),
            benutzername: self.benutzername.trim().to_string(),
            imap: saeubern(&self.imap),
            smtp: saeubern(&self.smtp),
            gesendet_ablegen: self.gesendet_ablegen,
        }
    }
}

/// Verwaltet die Konten: Konfigurationsdatei und Schlüsselbund.
pub struct KontoVerwaltung {
    datei: PathBuf,
    schluesselbund: Arc<dyn Schluesselbund>,
    sperre: Mutex<()>,
}

impl KontoVerwaltung {
    /// `ordner` ist der App-Datenordner.
    pub fn neu(ordner: &Path, schluesselbund: Arc<dyn Schluesselbund>) -> Self {
        Self {
            datei: ordner.join("konten.json"),
            schluesselbund,
            sperre: Mutex::new(()),
        }
    }

    /// Alle eingerichteten Konten (ohne Passwörter).
    pub async fn alle(&self) -> Result<Vec<Konto>, Fehlermeldung> {
        let _sperre = self.sperre.lock().await;
        datei::laden(&self.datei)
    }

    /// Ein Konto anhand seiner ID.
    pub async fn finden(&self, id: &str) -> Result<Konto, Fehlermeldung> {
        self.alle()
            .await?
            .into_iter()
            .find(|k| k.id == id)
            .ok_or_else(|| Fehlermeldung::neu("unbekannt", "Dieses Konto gibt es nicht mehr."))
    }

    /// Passwort eines Kontos aus dem Schlüsselbund.
    pub fn passwort(&self, id: &str) -> Result<Zeroizing<String>, Fehlermeldung> {
        self.schluesselbund
            .lesen(&schluesselbund::konto_schluessel(id))?
            .ok_or_else(|| {
                Fehlermeldung::neu(
                    "kein_passwort",
                    "Für dieses Konto ist kein Passwort im Schlüsselbund gespeichert. Bitte Konto bearbeiten und Passwort neu eingeben.",
                )
            })
    }

    /// Testet die Verbindung mit den eingegebenen Daten, ohne etwas zu speichern.
    ///
    /// Ist beim Bearbeiten kein neues Passwort angegeben, wird das gespeicherte verwendet
    /// (nur solange Server und Benutzername unverändert sind).
    pub async fn testen(&self, eingabe: &KontoEingabe) -> Result<Pruefergebnis, Fehlermeldung> {
        eingabe.pruefen()?;
        let passwort = self.passwort_fuer(eingabe).await?;
        let konto = eingabe.in_konto(eingabe.id.clone().unwrap_or_default());
        Ok(verbindung_testen(&konto, &passwort).await)
    }

    /// Legt ein Konto an oder speichert Änderungen.
    ///
    /// Vorher wird die Verbindung zu IMAP und SMTP geprüft. Schlägt sie fehl,
    /// wird nichts gespeichert.
    pub async fn speichern(&self, eingabe: &KontoEingabe) -> Result<Konto, Fehlermeldung> {
        eingabe.pruefen()?;
        let passwort = self.passwort_fuer(eingabe).await?;
        let id = eingabe
            .id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let konto = eingabe.in_konto(id.clone());

        verbindung_testen(&konto, &passwort).await.als_ergebnis()?;

        let _sperre = self.sperre.lock().await;
        let mut konten = datei::laden(&self.datei)?;
        let schluessel = schluesselbund::konto_schluessel(&id);
        let altes_passwort = self.schluesselbund.lesen(&schluessel)?;

        match konten.iter_mut().find(|k| k.id == id) {
            Some(bestehend) => *bestehend = konto.clone(),
            None if eingabe.id.is_some() => {
                return Err(Fehlermeldung::neu(
                    "unbekannt",
                    "Dieses Konto gibt es nicht mehr.",
                ))
            }
            None => konten.push(konto.clone()),
        }

        // Erst das Passwort, dann die Datei: Schlägt das Schreiben fehl,
        // wird der alte Zustand im Schlüsselbund wiederhergestellt.
        self.schluesselbund.setzen(&schluessel, &passwort)?;
        if let Err(fehler) = datei::speichern(&self.datei, &konten) {
            let _ = match altes_passwort {
                Some(alt) => self.schluesselbund.setzen(&schluessel, &alt),
                None => self.schluesselbund.loeschen(&schluessel),
            };
            return Err(fehler);
        }
        Ok(konto)
    }

    /// Löscht ein Konto samt Schlüsselbund-Eintrag.
    pub async fn loeschen(&self, id: &str) -> Result<(), Fehlermeldung> {
        let _sperre = self.sperre.lock().await;
        let mut konten = datei::laden(&self.datei)?;
        let vorher = konten.len();
        konten.retain(|k| k.id != id);
        if konten.len() == vorher {
            return Err(Fehlermeldung::neu(
                "unbekannt",
                "Dieses Konto gibt es nicht mehr.",
            ));
        }
        // Erst den Schlüsselbund-Eintrag, dann die Datei: Ist der Schlüsselbund gesperrt,
        // bleibt das Konto bestehen und das Löschen kann wiederholt werden.
        self.schluesselbund
            .loeschen(&schluesselbund::konto_schluessel(id))?;
        datei::speichern(&self.datei, &konten)?;
        Ok(())
    }

    /// Passwort für Test und Speichern: das neu eingegebene oder – beim Bearbeiten – das
    /// gespeicherte. Das gespeicherte wird nur verwendet, wenn Benutzername und Server
    /// unverändert sind; sonst könnte es an einen anderen (oder vertippten) Server gehen.
    async fn passwort_fuer(
        &self,
        eingabe: &KontoEingabe,
    ) -> Result<Zeroizing<String>, Fehlermeldung> {
        match (&eingabe.passwort, &eingabe.id) {
            (Some(passwort), _) if !passwort.is_empty() => Ok(passwort.clone()),
            (_, Some(id)) => {
                let bisher = self.finden(id).await?;
                let neu = eingabe.in_konto(id.clone());
                if bisher.benutzername != neu.benutzername
                    || bisher.imap != neu.imap
                    || bisher.smtp != neu.smtp
                {
                    return Err(Fehlermeldung::neu(
                        "eingabe",
                        "Server oder Benutzername wurden geändert. Bitte das Passwort erneut eingeben.",
                    ));
                }
                self.passwort(id)
            }
            _ => Err(Fehlermeldung::neu("eingabe", "Bitte das Passwort angeben.")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn beispiel_eingabe() -> KontoEingabe {
        KontoEingabe {
            id: None,
            name: "Anna Muster".into(),
            email: "anna@example.org".into(),
            benutzername: "anna".into(),
            passwort: Some(Zeroizing::new("geheim-123".into())),
            imap: ServerEinstellung {
                host: "imap.example.org".into(),
                port: 993,
                verschluesselung: Verschluesselung::Tls,
                zertifikat: None,
            },
            smtp: ServerEinstellung {
                host: "smtp.example.org".into(),
                port: 587,
                verschluesselung: Verschluesselung::Starttls,
                zertifikat: None,
            },
            gesendet_ablegen: true,
        }
    }

    #[test]
    fn debug_ausgabe_enthaelt_kein_passwort() {
        let ausgabe = format!("{:?}", beispiel_eingabe());
        assert!(!ausgabe.contains("geheim-123"));
        assert!(ausgabe.contains("***"));
    }

    #[test]
    fn gueltige_eingabe_wird_akzeptiert() {
        assert!(beispiel_eingabe().pruefen().is_ok());
    }

    #[test]
    fn ungueltige_eingaben_werden_abgelehnt() {
        let mut e = beispiel_eingabe();
        e.email = "keine-adresse".into();
        assert!(e.pruefen().is_err());

        let mut e = beispiel_eingabe();
        e.email = "a@b@example.org".into();
        assert!(e.pruefen().is_err());

        let mut e = beispiel_eingabe();
        e.imap.host = "imap.example.org:993".into();
        assert!(e.pruefen().is_err());

        let mut e = beispiel_eingabe();
        e.smtp.port = 0;
        assert!(e.pruefen().is_err());

        let mut e = beispiel_eingabe();
        e.name = "  ".into();
        assert!(e.pruefen().is_err());
    }

    #[test]
    fn klartext_verschluesselung_gibt_es_nicht() {
        let json = r#"{"host":"x","port":143,"verschluesselung":"keine"}"#;
        assert!(serde_json::from_str::<ServerEinstellung>(json).is_err());
    }

    #[tokio::test]
    async fn gespeichertes_passwort_geht_nicht_an_geaenderten_server() {
        let ordner = tempfile::tempdir().unwrap();
        let schluesselbund = Arc::new(schluesselbund::SpeicherSchluesselbund::default());
        let verwaltung = KontoVerwaltung::neu(ordner.path(), schluesselbund.clone());
        let konto = beispiel_eingabe().in_konto("k1".into());
        datei::speichern(&ordner.path().join("konten.json"), &[konto]).unwrap();
        schluesselbund
            .setzen(&schluesselbund::konto_schluessel("k1"), "geheim-123")
            .unwrap();

        let geaendert = |aendern: fn(&mut KontoEingabe)| {
            let mut e = beispiel_eingabe();
            e.id = Some("k1".into());
            e.passwort = None;
            aendern(&mut e);
            e
        };
        for eingabe in [
            geaendert(|e| e.imap.host = "imap.angreifer.example".into()),
            geaendert(|e| e.smtp.host = "smtp.angreifer.example".into()),
            geaendert(|e| e.imap.port = 1993),
            geaendert(|e| e.benutzername = "jemand".into()),
        ] {
            let fehler = verwaltung.testen(&eingabe).await.unwrap_err();
            assert_eq!(fehler.art, "eingabe");
            let fehler = verwaltung.speichern(&eingabe).await.unwrap_err();
            assert_eq!(fehler.art, "eingabe");
        }
    }

    #[tokio::test]
    async fn loeschen_entfernt_konto_und_passwort() {
        let ordner = tempfile::tempdir().unwrap();
        let schluesselbund = Arc::new(schluesselbund::SpeicherSchluesselbund::default());
        let verwaltung = KontoVerwaltung::neu(ordner.path(), schluesselbund.clone());
        let konto = beispiel_eingabe().in_konto("k1".into());
        datei::speichern(&ordner.path().join("konten.json"), &[konto]).unwrap();
        schluesselbund
            .setzen(&schluesselbund::konto_schluessel("k1"), "geheim-123")
            .unwrap();

        verwaltung.loeschen("k1").await.unwrap();
        assert!(verwaltung.alle().await.unwrap().is_empty());
        assert_eq!(schluesselbund.anzahl(), 0);
    }
}
