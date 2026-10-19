//! Ende-zu-Ende-Verschlüsselung mit OpenPGP (#44), mit der reinen Rust-Bibliothek rPGP (`pgp`).
//!
//! - `schluessel`: Schlüssel erzeugen, einlesen, prüfen, beschreiben
//! - `speicher`: verschlüsselter Schlüsselspeicher (`schluessel.db`, SQLCipher)
//! - `lesen`: verschlüsselte und signierte Mails erkennen, entschlüsseln und prüfen
//! - `schreiben`: Mails verschlüsseln und signieren
//!
//! Passphrasen werden nie gespeichert, sondern nach dem Entsperren höchstens bis zum
//! Programmende im Arbeitsspeicher gehalten (`Zeroizing`).

pub mod lesen;
pub mod schluessel;
pub mod schreiben;
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

/// Ergebnis der Prüfung vor dem Senden (für die Oberfläche).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct VersandPruefung {
    /// Empfänger ohne (verwendbaren) öffentlichen Schlüssel.
    pub fehlende_empfaenger: Vec<String>,
    /// Das Konto hat keinen eigenen Schlüssel.
    pub eigener_fehlt: bool,
    /// Fingerabdruck (Anzeige) des eigenen Schlüssels, dessen Passphrase zum Signieren fehlt.
    pub passphrase_fuer: Option<String>,
}

/// Schlüssel für den Versand einer Mail.
pub struct VersandSchluessel {
    /// Öffentlicher Schlüssel je Empfängeradresse (kleingeschrieben); nur beim Verschlüsseln.
    pub empfaenger: Option<HashMap<String, SignedPublicKey>>,
    /// Eigener öffentlicher Schlüssel (es wird immer auch an ihn verschlüsselt).
    pub eigener: SignedPublicKey,
    /// Zum Signieren (eigener Schlüssel mit Passphrase).
    pub signierer: Option<schreiben::Signierer>,
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

    /// Neuester verwendbarer öffentlicher Schlüssel für eine Adresse (importiert oder eigener).
    fn schluessel_fuer<'a>(
        alle: &'a [SignedPublicKey],
        adresse: &str,
    ) -> Option<&'a SignedPublicKey> {
        alle.iter()
            .filter(|s| {
                schluessel::passt_zu_adresse(s, adresse) && schreiben::kann_verschluesseln(s)
            })
            .max_by_key(|s| {
                use pgp::types::KeyDetails;
                s.primary_key.created_at().as_secs()
            })
    }

    /// Prüft, ob mit diesen Einstellungen gesendet werden kann (ohne etwas zu verändern).
    pub fn versand_pruefen(
        &self,
        konto: &str,
        adressen: &[String],
        verschluesseln: bool,
        signieren: bool,
    ) -> Result<VersandPruefung, Fehlermeldung> {
        let mut pruefung = VersandPruefung::default();
        if !verschluesseln && !signieren {
            return Ok(pruefung);
        }
        let eigene = self.eigene()?;
        match eigene.iter().find(|(k, _)| k == konto) {
            None => pruefung.eigener_fehlt = true,
            Some((_, geheim)) => {
                if signieren && !self.entsperrt(geheim) {
                    pruefung.passphrase_fuer = Some(schluessel::fingerabdruck_anzeige(
                        &schluessel::fingerabdruck_hex(&geheim.primary_key),
                    ));
                }
            }
        }
        if verschluesseln {
            let alle: Vec<SignedPublicKey> = eigene
                .iter()
                .map(|(_, s)| s.to_public_key())
                .chain(self.fremde()?)
                .collect();
            for adresse in adressen {
                let adresse = adresse.trim().to_lowercase();
                if Self::schluessel_fuer(&alle, &adresse).is_none()
                    && !pruefung.fehlende_empfaenger.contains(&adresse)
                {
                    pruefung.fehlende_empfaenger.push(adresse);
                }
            }
        }
        Ok(pruefung)
    }

    /// Stellt die Schlüssel für den Versand zusammen. Fehlt etwas, wird nicht gesendet:
    /// SicherMail sendet nie unbemerkt unverschlüsselt.
    pub fn versand_vorbereiten(
        &self,
        konto: &str,
        adressen: &[String],
        verschluesseln: bool,
        signieren: bool,
    ) -> Result<VersandSchluessel, Fehlermeldung> {
        let pruefung = self.versand_pruefen(konto, adressen, verschluesseln, signieren)?;
        if pruefung.eigener_fehlt {
            return Err(Fehlermeldung::neu(
                "pgp_kein_eigener",
                "Für dieses Konto gibt es keinen eigenen OpenPGP-Schlüssel. Erzeuge oder importiere ihn unter „Schlüssel (OpenPGP)“.",
            ));
        }
        if !pruefung.fehlende_empfaenger.is_empty() {
            return Err(Fehlermeldung::neu(
                "pgp_schluessel_fehlt",
                format!(
                    "Für diese Empfänger fehlt ein öffentlicher Schlüssel: {}. SicherMail sendet nie unbemerkt unverschlüsselt – importiere die Schlüssel oder schalte „Verschlüsseln“ aus.",
                    pruefung.fehlende_empfaenger.join(", ")
                ),
            ));
        }
        if pruefung.passphrase_fuer.is_some() {
            return Err(Fehlermeldung::neu(
                "pgp_passphrase",
                "Zum Signieren wird die Passphrase deines Schlüssels benötigt.",
            ));
        }
        let eigene = self.eigene()?;
        let (_, geheim) = eigene
            .into_iter()
            .find(|(k, _)| k == konto)
            .ok_or_else(|| fehler("Kein eigener Schlüssel."))?;
        let eigener = geheim.to_public_key();
        let empfaenger = if verschluesseln {
            let alle: Vec<SignedPublicKey> = std::iter::once(eigener.clone())
                .chain(self.fremde()?)
                .collect();
            let mut zuordnung = HashMap::new();
            for adresse in adressen {
                let adresse = adresse.trim().to_lowercase();
                let schluessel = Self::schluessel_fuer(&alle, &adresse)
                    .ok_or_else(|| fehler("Ein Empfängerschlüssel fehlt."))?;
                zuordnung.insert(adresse, schluessel.clone());
            }
            Some(zuordnung)
        } else {
            None
        };
        let signierer = if signieren {
            let passphrase = if schluessel::geschuetzt(&geheim) {
                self.passphrase(&schluessel::fingerabdruck_hex(&geheim.primary_key))
                    .ok_or_else(|| fehler("Die Passphrase fehlt."))?
            } else {
                Zeroizing::new(String::new())
            };
            Some(schreiben::Signierer {
                schluessel: geheim,
                passphrase,
            })
        } else {
            None
        };
        Ok(VersandSchluessel {
            empfaenger,
            eigener,
            signierer,
        })
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
