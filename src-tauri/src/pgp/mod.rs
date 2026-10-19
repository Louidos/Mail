//! Ende-zu-Ende-Verschlüsselung mit OpenPGP (#44), mit der reinen Rust-Bibliothek rPGP (`pgp`).
//!
//! - `schluessel`: Schlüssel erzeugen, einlesen, prüfen, beschreiben
//! - `speicher`: verschlüsselter Schlüsselspeicher (`schluessel.db`, SQLCipher)
//! - `lesen`: verschlüsselte und signierte Mails erkennen, entschlüsseln und prüfen
//!
//! Passphrasen werden nie gespeichert, sondern nach dem Entsperren höchstens bis zum
//! Programmende im Arbeitsspeicher gehalten (`Zeroizing`).

pub mod lesen;
pub mod schluessel;
pub mod speicher;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use pgp::composed::{SignedPublicKey, SignedSecretKey};
use serde::Serialize;
use zeroize::Zeroizing;

use crate::fehler::Fehlermeldung;
use crate::konto::schluesselbund::Schluesselbund;
use lesen::{Schluesselring, Verarbeitet};
use schluessel::SchluesselInfo;
use speicher::SchluesselSpeicher;

/// Ein eigener Schlüssel in der Übersicht.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EigenerSchluessel {
    pub konto: String,
    #[serde(flatten)]
    pub info: SchluesselInfo,
    /// Mit Passphrase geschützt?
    pub geschuetzt: bool,
    /// Ist die Passphrase in dieser Sitzung bereits eingegeben (oder nicht nötig)?
    pub entsperrt: bool,
}

/// Alle Schlüssel für die Oberfläche (ohne geheime Teile).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct Uebersicht {
    pub eigene: Vec<EigenerSchluessel>,
    pub fremde: Vec<SchluesselInfo>,
}

fn fehler(meldung: &str) -> Fehlermeldung {
    Fehlermeldung::neu("pgp", meldung)
}

/// Verwaltet Schlüssel und entschlüsselt bzw. prüft Mails.
pub struct PgpDienst {
    ordner: PathBuf,
    schluesselbund: Arc<dyn Schluesselbund>,
    speicher: Mutex<Option<Arc<SchluesselSpeicher>>>,
    /// Fingerabdruck (Hex) → Passphrase, nur im Arbeitsspeicher.
    passphrasen: Mutex<HashMap<String, Zeroizing<String>>>,
}

impl PgpDienst {
    pub fn neu(ordner: &Path, schluesselbund: Arc<dyn Schluesselbund>) -> Self {
        Self {
            ordner: ordner.to_path_buf(),
            schluesselbund,
            speicher: Mutex::new(None),
            passphrasen: Mutex::new(HashMap::new()),
        }
    }

    /// Öffnet den Schlüsselspeicher beim ersten Zugriff (z. B. erst, wenn der Schlüsselbund
    /// entsperrt ist).
    fn speicher(&self) -> Result<Arc<SchluesselSpeicher>, Fehlermeldung> {
        let mut speicher = self
            .speicher
            .lock()
            .map_err(|_| fehler("Interner Fehler im Schlüsselspeicher."))?;
        if let Some(offen) = speicher.as_ref() {
            return Ok(offen.clone());
        }
        let offen = Arc::new(SchluesselSpeicher::mit_schluesselbund(
            &self.ordner,
            self.schluesselbund.as_ref(),
        )?);
        *speicher = Some(offen.clone());
        Ok(offen)
    }

    fn passphrase(&self, fingerabdruck: &str) -> Option<Zeroizing<String>> {
        self.passphrasen
            .lock()
            .ok()
            .and_then(|p| p.get(fingerabdruck).cloned())
    }

    /// Eigene Schlüssel mit Konto (verworfen werden nicht lesbare Einträge).
    fn eigene(&self) -> Result<Vec<(String, SignedSecretKey)>, Fehlermeldung> {
        Ok(self
            .speicher()?
            .eigene()?
            .into_iter()
            .filter_map(|e| Some((e.konto, schluessel::geheim_lesen(&e.geheim).ok()?)))
            .collect())
    }

    fn fremde(&self) -> Result<Vec<SignedPublicKey>, Fehlermeldung> {
        Ok(self
            .speicher()?
            .fremde()?
            .into_iter()
            .filter_map(|e| schluessel::oeffentlich_lesen(&e.oeffentlich).ok())
            .flatten()
            .collect())
    }

    /// Ist der Schlüssel ohne weitere Eingabe nutzbar?
    fn entsperrt(&self, schluessel: &SignedSecretKey) -> bool {
        !schluessel::geschuetzt(schluessel)
            || self
                .passphrase(&schluessel::fingerabdruck_hex(&schluessel.primary_key))
                .is_some()
    }

    pub fn uebersicht(&self) -> Result<Uebersicht, Fehlermeldung> {
        let eigene = self
            .eigene()?
            .into_iter()
            .map(|(konto, s)| EigenerSchluessel {
                konto,
                info: schluessel::info(&s.to_public_key()),
                geschuetzt: schluessel::geschuetzt(&s),
                entsperrt: self.entsperrt(&s),
            })
            .collect();
        let fremde = self.fremde()?.iter().map(schluessel::info).collect();
        Ok(Uebersicht { eigene, fremde })
    }

    fn konto_frei(&self, konto: &str) -> Result<(), Fehlermeldung> {
        if self.eigene()?.iter().any(|(k, _)| k == konto) {
            return Err(fehler(
                "Für dieses Konto gibt es schon einen eigenen Schlüssel. Lösche ihn zuerst, wenn du ihn ersetzen willst.",
            ));
        }
        Ok(())
    }

    fn eigenen_ablegen(
        &self,
        konto: &str,
        geheim: &SignedSecretKey,
    ) -> Result<SchluesselInfo, Fehlermeldung> {
        let fingerabdruck = schluessel::fingerabdruck_hex(&geheim.primary_key);
        self.speicher()?.eigenen_speichern(
            konto,
            &fingerabdruck,
            &schluessel::geheim_armored(geheim)?,
        )?;
        Ok(schluessel::info(&geheim.to_public_key()))
    }

    /// Erzeugt einen neuen eigenen Schlüssel für ein Konto.
    pub fn erzeugen(
        &self,
        konto: &str,
        name: &str,
        email: &str,
        passphrase: &str,
    ) -> Result<SchluesselInfo, Fehlermeldung> {
        self.konto_frei(konto)?;
        let geheim = schluessel::erzeugen(name, email, passphrase)?;
        self.eigenen_ablegen(konto, &geheim)
    }

    /// Importiert einen eigenen privaten Schlüssel. Die Passphrase muss stimmen, und eine
    /// User-ID muss zur Adresse des Kontos passen.
    pub fn eigenen_importieren(
        &self,
        konto: &str,
        email: &str,
        daten: &[u8],
        passphrase: &str,
    ) -> Result<SchluesselInfo, Fehlermeldung> {
        self.konto_frei(konto)?;
        let geheim = schluessel::geheim_lesen(daten)?;
        if !schluessel::passt_zu_adresse(&geheim.to_public_key(), email) {
            return Err(fehler(&format!(
                "Der Schlüssel passt nicht zu diesem Konto: Keine seiner User-IDs enthält {email}."
            )));
        }
        if !schluessel::passphrase_pruefen(&geheim, passphrase) {
            return Err(fehler("Die Passphrase ist falsch."));
        }
        self.eigenen_ablegen(konto, &geheim)
    }

    /// Importiert öffentliche Schlüssel anderer (aus Datei oder Anhang).
    pub fn oeffentliche_importieren(
        &self,
        daten: &[u8],
    ) -> Result<Vec<SchluesselInfo>, Fehlermeldung> {
        let speicher = self.speicher()?;
        let mut infos = Vec::new();
        for oeffentlich in schluessel::oeffentlich_lesen(daten)? {
            speicher.fremden_speichern(
                &schluessel::fingerabdruck_hex(&oeffentlich.primary_key),
                schluessel::oeffentlich_armored(&oeffentlich)?.as_bytes(),
            )?;
            infos.push(schluessel::info(&oeffentlich));
        }
        Ok(infos)
    }

    /// Öffentlicher Schlüssel (eigener oder fremder) als ASCII-armored Text mit Dateinamen.
    pub fn exportieren(&self, fingerabdruck: &str) -> Result<(String, String), Fehlermeldung> {
        let gesucht = schluessel::fingerabdruck_normal(fingerabdruck);
        let oeffentlich = self
            .eigene()?
            .into_iter()
            .map(|(_, s)| s.to_public_key())
            .chain(self.fremde()?)
            .find(|s| schluessel::fingerabdruck_hex(&s.primary_key) == gesucht)
            .ok_or_else(|| fehler("Diesen Schlüssel gibt es nicht."))?;
        let info = schluessel::info(&oeffentlich);
        let name = info
            .adressen
            .first()
            .map(|a| format!("{a}.asc"))
            .unwrap_or_else(|| format!("{}.asc", &gesucht[gesucht.len().saturating_sub(16)..]));
        Ok((name, schluessel::oeffentlich_armored(&oeffentlich)?))
    }

    /// Löscht einen Schlüssel endgültig.
    pub fn loeschen(&self, fingerabdruck: &str) -> Result<(), Fehlermeldung> {
        let gesucht = schluessel::fingerabdruck_normal(fingerabdruck);
        if !self.speicher()?.loeschen(&gesucht)? {
            return Err(fehler("Diesen Schlüssel gibt es nicht."));
        }
        if let Ok(mut p) = self.passphrasen.lock() {
            p.remove(&gesucht);
        }
        Ok(())
    }

    /// Prüft die Passphrase eines eigenen Schlüssels und merkt sie sich bis zum Programmende.
    pub fn entsperren(&self, fingerabdruck: &str, passphrase: &str) -> Result<(), Fehlermeldung> {
        let gesucht = schluessel::fingerabdruck_normal(fingerabdruck);
        let (_, geheim) = self
            .eigene()?
            .into_iter()
            .find(|(_, s)| schluessel::fingerabdruck_hex(&s.primary_key) == gesucht)
            .ok_or_else(|| fehler("Diesen Schlüssel gibt es nicht."))?;
        if !schluessel::passphrase_pruefen(&geheim, passphrase) {
            return Err(fehler("Die Passphrase ist falsch."));
        }
        self.passphrasen
            .lock()
            .map_err(|_| fehler("Interner Fehler."))?
            .insert(gesucht, Zeroizing::new(passphrase.to_string()));
        Ok(())
    }

    /// Alle Schlüssel zum Entschlüsseln und Prüfen.
    pub fn ring(&self) -> Result<Schluesselring, Fehlermeldung> {
        let mut ring = Schluesselring::default();
        for (_, geheim) in self.eigene()? {
            let passphrase = if schluessel::geschuetzt(&geheim) {
                self.passphrase(&schluessel::fingerabdruck_hex(&geheim.primary_key))
            } else {
                Some(Zeroizing::new(String::new()))
            };
            ring.oeffentliche.push(geheim.to_public_key());
            ring.eigene.push((geheim, passphrase));
        }
        ring.oeffentliche.extend(self.fremde()?);
        Ok(ring)
    }

    /// Entschlüsselt und prüft eine Mail. Ist der Schlüsselspeicher nicht verfügbar, wird
    /// die Mail trotzdem erkannt und der Grund gemeldet.
    pub fn verarbeiten(&self, roh: &[u8]) -> Verarbeitet {
        // Gewöhnliche Mails: Schlüsselspeicher (und Schlüsselbund) gar nicht erst öffnen.
        if !lesen::moeglich(roh) {
            return Verarbeitet::default();
        }
        match self.ring() {
            Ok(ring) => lesen::verarbeiten(roh, &ring),
            Err(e) => {
                let mut ergebnis = lesen::verarbeiten(roh, &Schluesselring::default());
                if ergebnis.status.verschluesselt()
                    || ergebnis.status.signatur != lesen::Signatur::Keine
                {
                    ergebnis.status.meldung = Some(format!(
                        "Der Schlüsselspeicher ist nicht verfügbar: {}",
                        e.meldung
                    ));
                }
                ergebnis
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::konto::schluesselbund::SpeicherSchluesselbund;

    #[test]
    fn verwalten_entsperren_und_neustart() {
        let ordner = tempfile::tempdir().unwrap();
        let bund: Arc<dyn Schluesselbund> = Arc::new(SpeicherSchluesselbund::default());
        let dienst = PgpDienst::neu(ordner.path(), bund.clone());
        assert_eq!(dienst.uebersicht().unwrap(), Uebersicht::default());

        let info = dienst
            .erzeugen("k1", "Anna", "anna@example.org", "test-passphrase")
            .unwrap();
        assert!(dienst
            .erzeugen("k1", "Anna", "anna@example.org", "test-passphrase")
            .is_err());
        let uebersicht = dienst.uebersicht().unwrap();
        assert_eq!(uebersicht.eigene.len(), 1);
        assert!(uebersicht.eigene[0].geschuetzt);
        assert!(!uebersicht.eigene[0].entsperrt);

        assert!(dienst
            .entsperren(&info.fingerabdruck, "falsch-falsch")
            .is_err());
        dienst
            .entsperren(&info.fingerabdruck, "test-passphrase")
            .unwrap();
        assert!(dienst.uebersicht().unwrap().eigene[0].entsperrt);

        let (name, text) = dienst.exportieren(&info.fingerabdruck).unwrap();
        assert_eq!(name, "anna@example.org.asc");
        assert!(text.starts_with("-----BEGIN PGP PUBLIC KEY BLOCK-----"));

        // Neustart: Schlüssel noch da, Passphrase vergessen.
        drop(dienst);
        let dienst = PgpDienst::neu(ordner.path(), bund);
        let uebersicht = dienst.uebersicht().unwrap();
        assert_eq!(uebersicht.eigene[0].info, info);
        assert!(!uebersicht.eigene[0].entsperrt);

        dienst.loeschen(&info.fingerabdruck).unwrap();
        assert!(dienst.uebersicht().unwrap().eigene.is_empty());
        assert!(dienst.loeschen(&info.fingerabdruck).is_err());
    }
}
