//! Mails mit OpenPGP verschlüsseln und signieren (#44, PGP/MIME nach RFC 3156).

use pgp::composed::{
    ArmorOptions, DetachedSignature, MessageBuilder, SignedPublicKey, SignedPublicSubKey,
    SignedSecretKey,
};
use pgp::crypto::hash::HashAlgorithm;
use pgp::crypto::sym::SymmetricKeyAlgorithm;
use pgp::packet::{PublicKey, SecretKey, SecretSubkey};
use pgp::types::{KeyDetails, Password, SigningKey};
use rand::rngs::OsRng;
use zeroize::Zeroizing;

use crate::fehler::Fehlermeldung;

/// Hash für Signaturen und der passende `micalg`-Wert für `multipart/signed`.
const HASH: HashAlgorithm = HashAlgorithm::Sha256;
pub const MICALG: &str = "pgp-sha256";

fn fehler(meldung: &str) -> Fehlermeldung {
    Fehlermeldung::neu("pgp", meldung)
}

/// Eigener Schlüssel mit Passphrase zum Signieren.
pub struct Signierer {
    pub schluessel: SignedSecretKey,
    pub passphrase: Zeroizing<String>,
}

/// Teil des privaten Schlüssels, der signieren darf.
enum Signierteil<'a> {
    Haupt(&'a SecretKey),
    Unter(&'a SecretSubkey),
}

impl Signierer {
    fn teil(&self) -> Signierteil<'_> {
        // Bevorzugt ein Unterschlüssel mit Signierrecht, sonst der Hauptschlüssel (wie bei
        // GnuPG-Schlüsseln mit [SC]).
        let unter = self.schluessel.secret_subkeys.iter().find(|u| {
            u.signatures.iter().any(|s| s.key_flags().sign()) && u.key.algorithm().can_sign()
        });
        match unter {
            Some(u) => Signierteil::Unter(&u.key),
            None => Signierteil::Haupt(&self.schluessel.primary_key),
        }
    }

    fn passwort(&self) -> Password {
        Password::from(self.passphrase.as_str())
    }
}

impl Signierteil<'_> {
    fn als_dyn(&self) -> &dyn SigningKey {
        match self {
            Signierteil::Haupt(k) => *k,
            Signierteil::Unter(k) => *k,
        }
    }
}

/// Teilschlüssel, an die verschlüsselt wird: Unterschlüssel mit Verschlüsselungsrecht; hat ein
/// (älterer) Schlüssel keinen, der Hauptschlüssel, falls er verschlüsseln kann.
enum Verschluesselungsteil<'a> {
    Haupt(&'a PublicKey),
    Unter(&'a SignedPublicSubKey),
}

fn verschluesselungsteile(schluessel: &SignedPublicKey) -> Vec<Verschluesselungsteil<'_>> {
    let mit_recht: Vec<Verschluesselungsteil> = schluessel
        .public_subkeys
        .iter()
        .filter(|u| {
            u.key.algorithm().can_encrypt()
                && u.signatures.iter().any(|s| {
                    let f = s.key_flags();
                    f.encrypt_comms() || f.encrypt_storage()
                })
        })
        .map(Verschluesselungsteil::Unter)
        .collect();
    if !mit_recht.is_empty() {
        return mit_recht;
    }
    let unterschluessel: Vec<Verschluesselungsteil> = schluessel
        .public_subkeys
        .iter()
        .filter(|u| u.key.algorithm().can_encrypt())
        .map(Verschluesselungsteil::Unter)
        .collect();
    if !unterschluessel.is_empty() {
        return unterschluessel;
    }
    if schluessel.primary_key.algorithm().can_encrypt() {
        return vec![Verschluesselungsteil::Haupt(&schluessel.primary_key)];
    }
    Vec::new()
}

/// Kann an diesen Schlüssel verschlüsselt werden?
pub fn kann_verschluesseln(schluessel: &SignedPublicKey) -> bool {
    !verschluesselungsteile(schluessel).is_empty()
}

/// Verschlüsselt `daten` (eine MIME-Entität) an alle `empfaenger` und signiert optional
/// (Signatur innerhalb der Verschlüsselung). Ergebnis: ASCII-armored OpenPGP-Nachricht.
///
/// Mit `anonym` werden die Schlüssel-IDs der Empfänger nicht in die Nachricht geschrieben.
pub fn verschluesseln(
    daten: &[u8],
    empfaenger: &[&SignedPublicKey],
    signierer: Option<&Signierer>,
    anonym: bool,
) -> Result<String, Fehlermeldung> {
    if empfaenger.is_empty() {
        return Err(fehler("Es gibt keinen Empfänger zum Verschlüsseln."));
    }
    let teil = signierer.map(Signierer::teil);
    // SEIPD v1 (mit MDC) versteht jede verbreitete OpenPGP-Software; AES-256.
    let mut builder = MessageBuilder::from_bytes("", daten.to_vec())
        .seipd_v1(OsRng, SymmetricKeyAlgorithm::AES256);
    for schluessel in empfaenger {
        let teile = verschluesselungsteile(schluessel);
        if teile.is_empty() {
            return Err(fehler(
                "Ein Empfängerschlüssel kann nicht zum Verschlüsseln verwendet werden.",
            ));
        }
        for teil in teile {
            let ergebnis = match (teil, anonym) {
                (Verschluesselungsteil::Unter(u), false) => builder.encrypt_to_key(OsRng, u),
                (Verschluesselungsteil::Unter(u), true) => {
                    builder.encrypt_to_key_anonymous(OsRng, u)
                }
                (Verschluesselungsteil::Haupt(h), false) => builder.encrypt_to_key(OsRng, h),
                (Verschluesselungsteil::Haupt(h), true) => {
                    builder.encrypt_to_key_anonymous(OsRng, h)
                }
            };
            ergebnis.map_err(|_| fehler("Die Mail konnte nicht verschlüsselt werden."))?;
        }
    }
    if let (Some(s), Some(teil)) = (signierer, &teil) {
        builder.sign(teil.als_dyn(), s.passwort(), HASH);
    }
    builder
        .to_armored_string(OsRng, ArmorOptions::default())
        .map_err(|_| fehler("Die Mail konnte nicht verschlüsselt werden. Stimmt die Passphrase?"))
}

/// Abgetrennte Signatur über `daten` (kanonische MIME-Entität mit CRLF), ASCII-armored.
pub fn signieren(daten: &[u8], signierer: &Signierer) -> Result<String, Fehlermeldung> {
    let falsch = || fehler("Die Mail konnte nicht signiert werden. Stimmt die Passphrase?");
    let teil = signierer.teil();
    let signatur = match teil {
        Signierteil::Haupt(k) => {
            DetachedSignature::sign_binary_data(OsRng, k, &signierer.passwort(), HASH, daten)
        }
        Signierteil::Unter(k) => {
            DetachedSignature::sign_binary_data(OsRng, k, &signierer.passwort(), HASH, daten)
        }
    }
    .map_err(|_| falsch())?;
    signatur
        .to_armored_string(ArmorOptions::default())
        .map_err(|_| falsch())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pgp::lesen::tests::{kopf, pgp_mime, schluessel, signiert_mime, PASSPHRASE};
    use crate::pgp::lesen::{verarbeiten, Schluesselring, Signatur, Verschluesselung};

    fn signierer(s: &SignedSecretKey) -> Signierer {
        Signierer {
            schluessel: s.clone(),
            passphrase: Zeroizing::new(PASSPHRASE.into()),
        }
    }

    fn ring_von(eigen: &SignedSecretKey, bekannt: &[&SignedSecretKey]) -> Schluesselring {
        Schluesselring {
            eigene: vec![(eigen.clone(), Some(Zeroizing::new(PASSPHRASE.into())))],
            oeffentliche: std::iter::once(eigen)
                .chain(bekannt.iter().copied())
                .map(|k| k.to_public_key())
                .collect(),
        }
    }

    const INNEN: &str = "Content-Type: text/plain; charset=utf-8\r\n\r\nGeheimer Rundlauf.\r\n";

    #[test]
    fn rundlauf_verschluesseln_signieren_lesen() {
        let s = schluessel();
        let armored = verschluesseln(
            INNEN.as_bytes(),
            &[&s.bea.to_public_key(), &s.anna.to_public_key()],
            Some(&signierer(&s.anna)),
            false,
        )
        .unwrap();
        let mail = pgp_mime("anna@example.org", &armored);
        // Bea kann lesen und die Signatur von Anna prüfen.
        let ergebnis = verarbeiten(mail.as_bytes(), &ring_von(&s.bea, &[&s.anna]));
        assert_eq!(
            ergebnis.status.verschluesselung,
            Verschluesselung::Entschluesselt
        );
        assert_eq!(ergebnis.status.signatur, Signatur::Gueltig);
        // Anna (eigene Kopie in „Gesendet“) ebenfalls.
        let ergebnis = verarbeiten(mail.as_bytes(), &ring_von(&s.anna, &[]));
        assert_eq!(
            ergebnis.status.verschluesselung,
            Verschluesselung::Entschluesselt
        );
        // Carl nicht.
        let ergebnis = verarbeiten(mail.as_bytes(), &ring_von(&s.carl, &[&s.anna]));
        assert_eq!(
            ergebnis.status.verschluesselung,
            Verschluesselung::NichtEntschluesselt
        );
    }

    #[test]
    fn anonyme_empfaenger_verraten_keine_schluessel_id() {
        let s = schluessel();
        let bea = s.bea.to_public_key();
        let armored = verschluesseln(INNEN.as_bytes(), &[&bea], None, true).unwrap();
        let (nachricht, _) =
            pgp::composed::Message::from_armor(std::io::BufReader::new(armored.as_bytes()))
                .unwrap();
        let pgp::composed::Message::Encrypted { esk, .. } = &nachricht else {
            panic!("nicht verschlüsselt");
        };
        for e in esk {
            if let pgp::composed::Esk::PublicKeyEncryptedSessionKey(p) = e {
                assert!(p.id().unwrap().is_wildcard());
            }
        }
        // Bea kann trotzdem lesen.
        let mail = pgp_mime("anna@example.org", &armored);
        let ergebnis = verarbeiten(mail.as_bytes(), &ring_von(&s.bea, &[]));
        assert_eq!(
            ergebnis.status.verschluesselung,
            Verschluesselung::Entschluesselt
        );
    }

    #[test]
    fn abgetrennte_signatur_rundlauf_und_falsche_passphrase() {
        let s = schluessel();
        let teil = "Content-Type: text/plain; charset=utf-8\r\n\r\nSigniert.";
        let signatur = signieren(teil.as_bytes(), &signierer(&s.anna)).unwrap();
        let mail = signiert_mime("anna@example.org", teil, &signatur);
        let ergebnis = verarbeiten(mail.as_bytes(), &ring_von(&s.bea, &[&s.anna]));
        assert_eq!(ergebnis.status.signatur, Signatur::Gueltig);

        let falsch = Signierer {
            schluessel: s.anna.clone(),
            passphrase: Zeroizing::new("falsch-falsch".into()),
        };
        assert!(signieren(teil.as_bytes(), &falsch).is_err());
        assert!(verschluesseln(
            INNEN.as_bytes(),
            &[&s.bea.to_public_key()],
            Some(&falsch),
            false
        )
        .is_err());
        let _ = kopf("x@example.org");
    }

    #[test]
    fn schluessel_ohne_verschluesselungsteil_wird_abgelehnt() {
        let s = schluessel();
        let mut nur_signieren = s.bea.to_public_key();
        nur_signieren.public_subkeys.clear();
        assert!(!kann_verschluesseln(&nur_signieren));
        assert!(verschluesseln(INNEN.as_bytes(), &[&nur_signieren], None, false).is_err());
        assert!(kann_verschluesseln(&s.bea.to_public_key()));
    }
}
