//! Ende-zu-Ende-Verschlüsselung mit S/MIME (#56), mit OpenSSL (`openssl`, vendored).
//!
//! - `zertifikat`: Zertifikate einlesen, prüfen, beschreiben; private Schlüssel schützen
//! - `speicher`: Tabellen in `schluessel.db` (gemeinsam mit OpenPGP)
//! - `lesen`: verschlüsselte und signierte Mails erkennen, entschlüsseln und prüfen
//! - `schreiben`: Mails verschlüsseln und signieren
//! - `der`: kleiner DER-Leser für Angaben, die OpenSSL nicht herausgibt
//!
//! Vertraut wird dem Zertifikatsspeicher des Betriebssystems und Wurzeln, die der Nutzer
//! ausdrücklich übernommen hat. Es gibt keine Verzeichnisdienste (LDAP), keine OCSP- oder
//! CRL-Abfragen und keine anderen Online-Abfragen. Passphrasen werden nie gespeichert, sondern
//! nach dem Entsperren höchstens bis zum Programmende im Arbeitsspeicher gehalten (`Zeroizing`).

pub mod der;
pub mod lesen;
pub mod schreiben;
pub mod speicher;
pub mod zertifikat;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use openssl::x509::{X509PurposeId, X509};
use serde::Serialize;
use zeroize::Zeroizing;

use crate::fehler::Fehlermeldung;
use crate::pgp::lesen::{Signatur, Verarbeitet};
use crate::pgp::speicher::{SchluesselDb, SchluesselSpeicher};
use crate::pgp::VersandPruefung;
use speicher::{Art, EigenerEintrag, SmimeSpeicher};
use zertifikat::{fehler, ZertifikatInfo, Zweck};

/// Ein eigenes Zertifikat in der Übersicht.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EigenesZertifikat {
    pub konto: String,
    #[serde(flatten)]
    pub info: ZertifikatInfo,
    /// Ist die Passphrase in dieser Sitzung bereits eingegeben?
    pub entsperrt: bool,
}

/// Alle Zertifikate für die Oberfläche (ohne geheime Teile).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct Uebersicht {
    pub eigene: Vec<EigenesZertifikat>,
    pub fremde: Vec<ZertifikatInfo>,
    /// Vom Nutzer übernommene Wurzeln.
    pub wurzeln: Vec<ZertifikatInfo>,
}

/// Verwaltet S/MIME-Zertifikate und entschlüsselt bzw. prüft Mails.
pub struct SmimeDienst {
    db: Arc<SchluesselDb>,
    /// Wurzeln des Betriebssystems verwenden (in Tests aus).
    systemwurzeln: bool,
    system: OnceLock<Vec<X509>>,
    /// Fingerabdruck (Hex) → Passphrase, nur im Arbeitsspeicher.
    passphrasen: Mutex<HashMap<String, Zeroizing<String>>>,
    /// Gelesene, aber noch nicht bestätigte Wurzel.
    ausstehende_wurzel: Mutex<Option<X509>>,
}

impl SmimeDienst {
    pub fn neu(db: Arc<SchluesselDb>) -> Self {
        Self::mit_systemwurzeln(db, true)
    }

    /// `systemwurzeln = false`: nur vom Nutzer übernommene Wurzeln (für Tests).
    pub fn mit_systemwurzeln(db: Arc<SchluesselDb>, systemwurzeln: bool) -> Self {
        Self {
            db,
            systemwurzeln,
            system: OnceLock::new(),
            passphrasen: Mutex::new(HashMap::new()),
            ausstehende_wurzel: Mutex::new(None),
        }
    }

    fn speicher(&self) -> Result<Arc<SchluesselSpeicher>, Fehlermeldung> {
        self.db.oeffnen()
    }

    fn passphrase(&self, fingerabdruck: &str) -> Option<Zeroizing<String>> {
        self.passphrasen
            .lock()
            .ok()
            .and_then(|p| p.get(fingerabdruck).cloned())
    }

    /// Lesbare eigene Einträge mit Zertifikat.
    fn eigene(&self) -> Result<Vec<(EigenerEintrag, X509)>, Fehlermeldung> {
        Ok(self
            .speicher()?
            .smime_eigene()?
            .into_iter()
            .filter_map(|e| {
                let z = X509::from_der(&e.zertifikat).ok()?;
                Some((e, z))
            })
            .collect())
    }

    fn zertifikate(&self, art: Art) -> Result<Vec<X509>, Fehlermeldung> {
        Ok(self
            .speicher()?
            .smime_zertifikate(art)?
            .iter()
            .filter_map(|d| X509::from_der(d).ok())
            .collect())
    }

    pub fn uebersicht(&self) -> Result<Uebersicht, Fehlermeldung> {
        let eigene = self
            .eigene()?
            .into_iter()
            .map(|(e, z)| EigenesZertifikat {
                konto: e.konto,
                info: zertifikat::info(&z),
                entsperrt: self.passphrase(&e.fingerabdruck).is_some(),
            })
            .collect();
        let fremde = self
            .zertifikate(Art::Fremde)?
            .iter()
            .map(|z| zertifikat::info(z))
            .collect();
        let wurzeln = self
            .zertifikate(Art::Wurzeln)?
            .iter()
            .map(|z| zertifikat::info(z))
            .collect();
        Ok(Uebersicht {
            eigene,
            fremde,
            wurzeln,
        })
    }

    /// Importiert ein eigenes Zertifikat aus PKCS#12. Das Passwort der Datei schützt danach
    /// auch den gespeicherten privaten Schlüssel.
    pub fn eigenes_importieren(
        &self,
        konto: &str,
        email: &str,
        daten: &[u8],
        passwort: &str,
    ) -> Result<ZertifikatInfo, Fehlermeldung> {
        if passwort.is_empty() {
            return Err(fehler(
                "Bitte das Passwort der Datei eingeben. SicherMail speichert private Schlüssel nur mit Passphrase.",
            ));
        }
        let paket = zertifikat::pkcs12_lesen(daten, passwort, email)?;
        let fingerabdruck = zertifikat::fingerabdruck_hex(&paket.zertifikat);
        let eintrag = EigenerEintrag {
            konto: konto.to_string(),
            fingerabdruck: fingerabdruck.clone(),
            zertifikat: paket
                .zertifikat
                .to_der()
                .map_err(|_| fehler("Interner Fehler."))?,
            kette: zertifikat::als_pem(&paket.kette),
            geheim: Zeroizing::new(zertifikat::schluessel_schuetzen(
                &paket.schluessel,
                passwort,
            )?),
        };
        self.speicher()?.smime_eigenen_speichern(&eintrag)?;
        // Wer das Passwort gerade eingegeben hat, muss es nicht gleich noch einmal eingeben.
        if let Ok(mut p) = self.passphrasen.lock() {
            p.insert(fingerabdruck, Zeroizing::new(passwort.to_string()));
        }
        Ok(zertifikat::info(&paket.zertifikat))
    }

    /// Speichert Zertifikate anderer (nur persönliche Zertifikate mit Mailadresse).
    fn fremde_speichern(&self, zertifikate: &[X509]) -> Result<Vec<ZertifikatInfo>, Fehlermeldung> {
        let speicher = self.speicher()?;
        let mut infos = Vec::new();
        for z in zertifikate {
            if zertifikat::ist_stelle(z) || zertifikat::adressen(z).is_empty() {
                continue;
            }
            let der = z.to_der().map_err(|_| fehler("Interner Fehler."))?;
            speicher.smime_zertifikat_speichern(
                Art::Fremde,
                &zertifikat::fingerabdruck_hex(z),
                &der,
            )?;
            infos.push(zertifikat::info(z));
        }
        if infos.is_empty() {
            return Err(fehler(
                "Die Datei enthält kein persönliches Zertifikat mit Mailadresse (Zertifikate von Zertifizierungsstellen werden hier nicht übernommen).",
            ));
        }
        Ok(infos)
    }

    /// Importiert Zertifikate anderer aus einer Datei (`.cer`, `.crt`, `.pem`, `.p7c`).
    pub fn fremde_importieren(&self, daten: &[u8]) -> Result<Vec<ZertifikatInfo>, Fehlermeldung> {
        self.fremde_speichern(&zertifikat::lesen(daten)?)
    }

    /// Übernimmt das Zertifikat des Absenders einer gültig signierten Mail.
    pub fn aus_mail_uebernehmen(&self, roh: &[u8]) -> Result<ZertifikatInfo, Fehlermeldung> {
        let ergebnis = self.verarbeiten(roh);
        let der = match (ergebnis.status.signatur, ergebnis.zertifikat) {
            (Signatur::Gueltig, Some(der)) if !ergebnis.status.signatur_teilweise => der,
            _ => {
                return Err(fehler(
                    "Das Zertifikat wird nur aus einer gültig signierten Mail übernommen.",
                ))
            }
        };
        let z = X509::from_der(&der).map_err(|_| fehler("Interner Fehler."))?;
        Ok(self.fremde_speichern(&[z])?.remove(0))
    }

    /// Liest ein Wurzelzertifikat zur Bestätigung (noch nicht übernommen).
    pub fn wurzel_lesen(&self, daten: &[u8]) -> Result<ZertifikatInfo, Fehlermeldung> {
        let alle = zertifikat::lesen(daten)?;
        let [wurzel] = <[X509; 1]>::try_from(alle)
            .map_err(|_| fehler("Die Datei muss genau ein Wurzelzertifikat enthalten."))?;
        let selbst_signiert = wurzel
            .public_key()
            .and_then(|k| wurzel.verify(&k))
            .unwrap_or(false);
        if !zertifikat::ist_stelle(&wurzel) || !selbst_signiert {
            return Err(fehler(
                "Das ist kein Wurzelzertifikat (selbst signierte Zertifizierungsstelle).",
            ));
        }
        let info = zertifikat::info(&wurzel);
        *self
            .ausstehende_wurzel
            .lock()
            .map_err(|_| fehler("Interner Fehler."))? = Some(wurzel);
        Ok(info)
    }

    /// Übernimmt die zuvor gelesene Wurzel, wenn der bestätigte Fingerabdruck stimmt.
    pub fn wurzel_bestaetigen(&self, fingerabdruck: &str) -> Result<ZertifikatInfo, Fehlermeldung> {
        let wurzel = self
            .ausstehende_wurzel
            .lock()
            .map_err(|_| fehler("Interner Fehler."))?
            .take()
            .ok_or_else(|| fehler("Es wartet kein Wurzelzertifikat auf Bestätigung."))?;
        let hex = zertifikat::fingerabdruck_hex(&wurzel);
        if zertifikat::fingerabdruck_normal(fingerabdruck) != hex {
            return Err(fehler("Der Fingerabdruck stimmt nicht überein."));
        }
        let der = wurzel.to_der().map_err(|_| fehler("Interner Fehler."))?;
        self.speicher()?
            .smime_zertifikat_speichern(Art::Wurzeln, &hex, &der)?;
        Ok(zertifikat::info(&wurzel))
    }

    /// Löscht ein Zertifikat (eigenes, fremdes oder übernommene Wurzel) endgültig.
    pub fn loeschen(&self, fingerabdruck: &str) -> Result<(), Fehlermeldung> {
        let gesucht = zertifikat::fingerabdruck_normal(fingerabdruck);
        if !self.speicher()?.smime_loeschen(&gesucht)? {
            return Err(fehler("Dieses Zertifikat gibt es nicht."));
        }
        if let Ok(mut p) = self.passphrasen.lock() {
            p.remove(&gesucht);
        }
        Ok(())
    }

    /// Prüft die Passphrase eines eigenen Zertifikats und merkt sie sich bis zum Programmende.
    pub fn entsperren(&self, fingerabdruck: &str, passphrase: &str) -> Result<(), Fehlermeldung> {
        let gesucht = zertifikat::fingerabdruck_normal(fingerabdruck);
        let (eintrag, _) = self
            .eigene()?
            .into_iter()
            .find(|(e, _)| e.fingerabdruck == gesucht)
            .ok_or_else(|| fehler("Dieses Zertifikat gibt es nicht."))?;
        if zertifikat::schluessel_oeffnen(&eintrag.geheim, passphrase).is_none() {
            return Err(fehler("Die Passphrase ist falsch."));
        }
        self.passphrasen
            .lock()
            .map_err(|_| fehler("Interner Fehler."))?
            .insert(gesucht, Zeroizing::new(passphrase.to_string()));
        Ok(())
    }

    /// Alles zum Entschlüsseln und Prüfen.
    pub fn ring(&self) -> Result<lesen::Ring, Fehlermeldung> {
        let mut ring = lesen::Ring::default();
        for (eintrag, z) in self.eigene()? {
            ring.gespeichert.insert(eintrag.fingerabdruck.clone());
            ring.bekannte.push(z.clone());
            ring.bekannte
                .extend(zertifikat::kette_lesen(&eintrag.kette));
            let passphrase = self.passphrase(&eintrag.fingerabdruck);
            ring.eigene.push(lesen::EigenerSchluessel {
                zertifikat: z,
                geheim: eintrag.geheim,
                passphrase,
            });
        }
        for z in self.zertifikate(Art::Fremde)? {
            ring.gespeichert.insert(zertifikat::fingerabdruck_hex(&z));
            ring.bekannte.push(z);
        }
        ring.wurzeln = self.zertifikate(Art::Wurzeln)?;
        if self.systemwurzeln {
            ring.wurzeln.extend(
                self.system
                    .get_or_init(zertifikat::systemwurzeln)
                    .iter()
                    .cloned(),
            );
        }
        Ok(ring)
    }

    /// Entschlüsselt und prüft eine Mail. Ist der Schlüsselspeicher nicht verfügbar, wird die
    /// Mail trotzdem erkannt (Signaturen nur gegen die Systemwurzeln) und der Grund gemeldet.
    pub fn verarbeiten(&self, roh: &[u8]) -> Verarbeitet {
        if !lesen::moeglich(roh) {
            return Verarbeitet::default();
        }
        match self.ring() {
            Ok(ring) => lesen::verarbeiten(roh, &ring),
            Err(e) => {
                let mut ring = lesen::Ring::default();
                if self.systemwurzeln {
                    ring.wurzeln = self.system.get_or_init(zertifikat::systemwurzeln).clone();
                }
                let mut ergebnis = lesen::verarbeiten(roh, &ring);
                if ergebnis.status.verschluesselt() {
                    ergebnis.status.meldung = Some(format!(
                        "Der Schlüsselspeicher ist nicht verfügbar: {}",
                        e.meldung
                    ));
                }
                ergebnis
            }
        }
    }

    /// Eigenes Zertifikat des Kontos, das jetzt gültig ist und für alle `zwecke` taugt (das
    /// neueste).
    fn eigenes_fuer(
        &self,
        konto: &str,
        zwecke: &[Zweck],
    ) -> Result<Option<(EigenerEintrag, X509)>, Fehlermeldung> {
        let jetzt = zertifikat::jetzt();
        Ok(self
            .eigene()?
            .into_iter()
            .filter(|(e, z)| {
                let (ab, bis) = zertifikat::gueltigkeit(z);
                e.konto == konto
                    && ab <= jetzt
                    && jetzt <= bis
                    && zwecke
                        .iter()
                        .all(|zweck| zertifikat::zweck_passt(z, *zweck))
            })
            .max_by_key(|(_, z)| zertifikat::gueltigkeit(z).0))
    }

    /// Zertifikat, an das für `adresse` verschlüsselt werden darf: jetzt gültig, passende
    /// Adresse, Zweck „Verschlüsseln“, starker Schlüssel und vertrauenswürdige Kette (das neueste).
    fn zum_verschluesseln<'a>(ring: &'a lesen::Ring, adresse: &str) -> Option<&'a X509> {
        let jetzt = zertifikat::jetzt();
        ring.bekannte
            .iter()
            .filter(|z| {
                !zertifikat::ist_stelle(z)
                    && zertifikat::passt_zu_adresse(z, adresse)
                    && zertifikat::zweck_passt(z, Zweck::Verschluesseln)
                    && zertifikat::schluessel_stark_genug(z)
                    && schreiben::verfahren_unterstuetzt(z)
                    && lesen::kette_pruefen(
                        z,
                        ring.bekannte.clone(),
                        &ring.wurzeln,
                        jetzt,
                        X509PurposeId::SMIME_ENCRYPT,
                    ) == lesen::Kette::Gueltig
            })
            .max_by_key(|z| zertifikat::gueltigkeit(z).0)
    }

    /// Prüft, ob mit S/MIME gesendet werden kann (ohne etwas zu verändern).
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
        let zwecke = Self::zwecke(verschluesseln, signieren);
        match self.eigenes_fuer(konto, &zwecke)? {
            None => pruefung.eigener_fehlt = true,
            Some((eintrag, _)) => {
                if signieren && self.passphrase(&eintrag.fingerabdruck).is_none() {
                    pruefung.passphrase_fuer =
                        Some(zertifikat::fingerabdruck_anzeige(&eintrag.fingerabdruck));
                }
            }
        }
        if verschluesseln {
            let ring = self.ring()?;
            for adresse in adressen {
                let adresse = adresse.trim().to_lowercase();
                if Self::zum_verschluesseln(&ring, &adresse).is_none()
                    && !pruefung.fehlende_empfaenger.contains(&adresse)
                {
                    pruefung.fehlende_empfaenger.push(adresse);
                }
            }
        }
        Ok(pruefung)
    }

    fn zwecke(verschluesseln: bool, signieren: bool) -> Vec<Zweck> {
        let mut zwecke = Vec::new();
        if verschluesseln {
            zwecke.push(Zweck::Verschluesseln);
        }
        if signieren {
            zwecke.push(Zweck::Signieren);
        }
        zwecke
    }

    /// Stellt die Zertifikate für den Versand zusammen. Fehlt etwas, wird nicht gesendet:
    /// SicherMail sendet nie unbemerkt unverschlüsselt.
    pub fn versand_vorbereiten(
        &self,
        konto: &str,
        adressen: &[String],
        verschluesseln: bool,
        signieren: bool,
    ) -> Result<schreiben::Versand, Fehlermeldung> {
        let pruefung = self.versand_pruefen(konto, adressen, verschluesseln, signieren)?;
        if pruefung.eigener_fehlt {
            return Err(Fehlermeldung::neu(
                "smime_kein_eigenes",
                "Für dieses Konto gibt es kein gültiges eigenes S/MIME-Zertifikat. Importiere es unter „Schlüssel und Zertifikate“.",
            ));
        }
        if !pruefung.fehlende_empfaenger.is_empty() {
            return Err(Fehlermeldung::neu(
                "smime_zertifikat_fehlt",
                format!(
                    "Für diese Empfänger fehlt ein gültiges S/MIME-Zertifikat: {}. SicherMail sendet nie unbemerkt unverschlüsselt – importiere die Zertifikate oder schalte „Verschlüsseln“ aus.",
                    pruefung.fehlende_empfaenger.join(", ")
                ),
            ));
        }
        if pruefung.passphrase_fuer.is_some() {
            return Err(Fehlermeldung::neu(
                "smime_passphrase",
                "Zum Signieren wird die Passphrase deines S/MIME-Zertifikats benötigt.",
            ));
        }
        let (eintrag, eigenes) = self
            .eigenes_fuer(konto, &Self::zwecke(verschluesseln, signieren))?
            .ok_or_else(|| fehler("Kein eigenes Zertifikat."))?;
        let empfaenger = if verschluesseln {
            let ring = self.ring()?;
            let mut zuordnung = std::collections::HashMap::new();
            for adresse in adressen {
                let adresse = adresse.trim().to_lowercase();
                let z = Self::zum_verschluesseln(&ring, &adresse)
                    .ok_or_else(|| fehler("Ein Empfängerzertifikat fehlt."))?;
                zuordnung.insert(adresse, z.clone());
            }
            Some(zuordnung)
        } else {
            None
        };
        let signierer = if signieren {
            let passphrase = self
                .passphrase(&eintrag.fingerabdruck)
                .ok_or_else(|| fehler("Die Passphrase fehlt."))?;
            let schluessel = zertifikat::schluessel_oeffnen(&eintrag.geheim, &passphrase)
                .ok_or_else(|| fehler("Die Passphrase ist falsch."))?;
            Some(schreiben::Signierer {
                zertifikat: eigenes.clone(),
                schluessel,
                kette: zertifikat::kette_lesen(&eintrag.kette),
            })
        } else {
            None
        };
        Ok(schreiben::Versand {
            empfaenger,
            eigenes,
            signierer,
        })
    }
}
