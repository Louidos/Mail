//! „Verbindung testen“: Anmeldung bei IMAP und SMTP prüfen.

use serde::Serialize;

use super::Konto;
use crate::fehler::{Fehlermeldung, VerbindungsFehler};

/// Ergebnis des Verbindungstests für beide Server.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Pruefergebnis {
    /// `None` heisst: IMAP in Ordnung.
    pub imap: Option<Fehlermeldung>,
    /// `None` heisst: SMTP in Ordnung.
    pub smtp: Option<Fehlermeldung>,
}

impl Pruefergebnis {
    pub fn erfolgreich(&self) -> bool {
        self.imap.is_none() && self.smtp.is_none()
    }

    /// Wandelt das Ergebnis in einen Fehler um, falls ein Server nicht funktioniert.
    pub fn als_ergebnis(&self) -> Result<(), Fehlermeldung> {
        match (&self.imap, &self.smtp) {
            (None, None) => Ok(()),
            (Some(imap), None) => Err(Fehlermeldung::neu(
                &imap.art,
                format!("IMAP (Empfang): {}", imap.meldung),
            )),
            (None, Some(smtp)) => Err(Fehlermeldung::neu(
                &smtp.art,
                format!("SMTP (Versand): {}", smtp.meldung),
            )),
            (Some(imap), Some(smtp)) => Err(Fehlermeldung::neu(
                &imap.art,
                format!(
                    "IMAP (Empfang): {}\nSMTP (Versand): {}",
                    imap.meldung, smtp.meldung
                ),
            )),
        }
    }
}

/// Meldet sich bei IMAP und SMTP an und gleich wieder ab. Speichert nichts.
pub async fn verbindung_testen(konto: &Konto, passwort: &str) -> Pruefergebnis {
    let (imap, smtp) = tokio::join!(imap_testen(konto, passwort), smtp_testen(konto, passwort));
    Pruefergebnis {
        imap: imap.err().map(Fehlermeldung::from),
        smtp: smtp.err().map(Fehlermeldung::from),
    }
}

async fn imap_testen(konto: &Konto, passwort: &str) -> Result<(), VerbindungsFehler> {
    let mut sitzung = crate::imap::anmelden(&konto.imap, &konto.benutzername, passwort).await?;
    let _ = sitzung.logout().await;
    Ok(())
}

async fn smtp_testen(konto: &Konto, passwort: &str) -> Result<(), VerbindungsFehler> {
    let mut verbindung = crate::smtp::anmelden(&konto.smtp, &konto.benutzername, passwort).await?;
    let _ = verbindung.quit().await;
    Ok(())
}
