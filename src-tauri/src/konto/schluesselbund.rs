//! Zugriff auf den Schlüsselbund des Betriebssystems.
//!
//! macOS: Schlüsselbund, Windows: Anmeldeinformationsverwaltung,
//! Linux: Secret Service (z. B. GNOME Keyring, KWallet).

use std::collections::HashMap;
use std::sync::Mutex;

use zeroize::Zeroizing;

use crate::fehler::Fehlermeldung;

/// Dienstname, unter dem SicherMail seine Einträge ablegt.
pub const DIENST: &str = "SicherMail";

/// Name des Eintrags für das Passwort eines Kontos.
pub fn konto_schluessel(konto_id: &str) -> String {
    format!("konto:{konto_id}")
}

/// Ablage für Geheimnisse.
pub trait Schluesselbund: Send + Sync {
    fn setzen(&self, schluessel: &str, wert: &str) -> Result<(), Fehlermeldung>;
    fn lesen(&self, schluessel: &str) -> Result<Option<Zeroizing<String>>, Fehlermeldung>;
    /// Löscht einen Eintrag. Fehlt er bereits, ist das kein Fehler.
    fn loeschen(&self, schluessel: &str) -> Result<(), Fehlermeldung>;
}

fn fehler(_: keyring::Error) -> Fehlermeldung {
    Fehlermeldung::neu(
        "schluesselbund",
        "Der Schlüsselbund des Betriebssystems ist nicht erreichbar. Ist er entsperrt?",
    )
}

/// Schlüsselbund des Betriebssystems.
pub struct SystemSchluesselbund;

impl Schluesselbund for SystemSchluesselbund {
    fn setzen(&self, schluessel: &str, wert: &str) -> Result<(), Fehlermeldung> {
        keyring::Entry::new(DIENST, schluessel)
            .and_then(|eintrag| eintrag.set_password(wert))
            .map_err(fehler)
    }

    fn lesen(&self, schluessel: &str) -> Result<Option<Zeroizing<String>>, Fehlermeldung> {
        let eintrag = keyring::Entry::new(DIENST, schluessel).map_err(fehler)?;
        match eintrag.get_password() {
            Ok(wert) => Ok(Some(Zeroizing::new(wert))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(fehler(e)),
        }
    }

    fn loeschen(&self, schluessel: &str) -> Result<(), Fehlermeldung> {
        let eintrag = keyring::Entry::new(DIENST, schluessel).map_err(fehler)?;
        match eintrag.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(fehler(e)),
        }
    }
}

/// Schlüsselbund im Arbeitsspeicher – nur für Tests.
#[derive(Default)]
pub struct SpeicherSchluesselbund {
    eintraege: Mutex<HashMap<String, Zeroizing<String>>>,
}

impl SpeicherSchluesselbund {
    pub fn anzahl(&self) -> usize {
        self.eintraege.lock().map(|e| e.len()).unwrap_or(0)
    }
}

impl Schluesselbund for SpeicherSchluesselbund {
    fn setzen(&self, schluessel: &str, wert: &str) -> Result<(), Fehlermeldung> {
        if let Ok(mut eintraege) = self.eintraege.lock() {
            eintraege.insert(schluessel.to_string(), Zeroizing::new(wert.to_string()));
        }
        Ok(())
    }

    fn lesen(&self, schluessel: &str) -> Result<Option<Zeroizing<String>>, Fehlermeldung> {
        Ok(self
            .eintraege
            .lock()
            .ok()
            .and_then(|e| e.get(schluessel).cloned()))
    }

    fn loeschen(&self, schluessel: &str) -> Result<(), Fehlermeldung> {
        if let Ok(mut eintraege) = self.eintraege.lock() {
            eintraege.remove(schluessel);
        }
        Ok(())
    }
}
