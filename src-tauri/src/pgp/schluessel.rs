//! OpenPGP-Schlüssel erzeugen, einlesen, prüfen und beschreiben.

use pgp::composed::{
    ArmorOptions, Deserializable, EncryptionCaps, KeyType, SecretKeyParamsBuilder, SignedPublicKey,
    SignedSecretKey, SubkeyParamsBuilder,
};
use pgp::crypto::ecc_curve::ECCCurve;
use pgp::crypto::hash::HashAlgorithm;
use pgp::crypto::sym::SymmetricKeyAlgorithm;
use pgp::types::{CompressionAlgorithm, KeyDetails, Password};
use serde::Serialize;
use smallvec::smallvec;
use zeroize::Zeroizing;

use crate::fehler::Fehlermeldung;

/// Mindestlänge einer Passphrase für neu erzeugte Schlüssel.
pub const MIN_PASSPHRASE: usize = 8;

/// Grösste eingelesene Schlüsseldatei (auch aus Anhängen).
pub const MAX_SCHLUESSELDATEI: usize = 1024 * 1024;

/// Beschreibung eines Schlüssels für die Oberfläche (ohne geheime Teile).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SchluesselInfo {
    /// Fingerabdruck in Vierergruppen, z. B. `ABCD 1234 …`.
    pub fingerabdruck: String,
    /// Alle User-IDs, z. B. `Anna Muster <anna@example.org>`.
    pub benutzer_ids: Vec<String>,
    /// Mailadressen aus den User-IDs (kleingeschrieben).
    pub adressen: Vec<String>,
    /// Unix-Zeit der Erzeugung.
    pub erstellt: i64,
    /// Gibt es einen Teilschlüssel zum Verschlüsseln?
    pub kann_verschluesseln: bool,
}

fn fehler(meldung: &str) -> Fehlermeldung {
    Fehlermeldung::neu("pgp", meldung)
}

/// Fingerabdruck als Hex-Text ohne Leerzeichen (Schlüssel für Speicher und Befehle).
pub fn fingerabdruck_hex(schluessel: &impl KeyDetails) -> String {
    hex::encode_upper(schluessel.fingerprint().as_bytes())
}

/// Fingerabdruck in Vierergruppen zur Anzeige.
pub fn fingerabdruck_anzeige(hex: &str) -> String {
    hex.as_bytes()
        .chunks(4)
        .map(|teil| String::from_utf8_lossy(teil).into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Vereinheitlicht einen eingegebenen Fingerabdruck (ohne Leerzeichen, Grossbuchstaben).
pub fn fingerabdruck_normal(eingabe: &str) -> String {
    eingabe
        .chars()
        .filter(|z| !z.is_whitespace())
        .collect::<String>()
        .to_ascii_uppercase()
}

/// Mailadresse aus einer User-ID wie `Anna <anna@example.org>` oder `anna@example.org`.
pub fn adresse_aus_user_id(user_id: &str) -> Option<String> {
    let kandidat = match (user_id.rfind('<'), user_id.rfind('>')) {
        (Some(start), Some(ende)) if start < ende => &user_id[start + 1..ende],
        _ => user_id,
    };
    let kandidat = kandidat.trim();
    let (lokal, domain) = kandidat.rsplit_once('@')?;
    (!lokal.is_empty() && !domain.is_empty() && !kandidat.contains(char::is_whitespace))
        .then(|| kandidat.to_lowercase())
}

/// Beschreibt einen öffentlichen Schlüssel.
pub fn info(schluessel: &SignedPublicKey) -> SchluesselInfo {
    let benutzer_ids: Vec<String> = schluessel
        .details
        .users
        .iter()
        .map(|u| String::from_utf8_lossy(u.id.id()).into_owned())
        .collect();
    let mut adressen: Vec<String> = benutzer_ids
        .iter()
        .filter_map(|u| adresse_aus_user_id(u))
        .collect();
    adressen.dedup();
    SchluesselInfo {
        fingerabdruck: fingerabdruck_anzeige(&fingerabdruck_hex(&schluessel.primary_key)),
        benutzer_ids,
        adressen,
        erstellt: i64::from(schluessel.primary_key.created_at().as_secs()),
        kann_verschluesseln: schluessel
            .public_subkeys
            .iter()
            .any(|s| s.key.algorithm().can_encrypt()),
    }
}

/// Passt eine Mailadresse zu einer User-ID des Schlüssels?
pub fn passt_zu_adresse(schluessel: &SignedPublicKey, adresse: &str) -> bool {
    let adresse = adresse.trim().to_lowercase();
    !adresse.is_empty() && info(schluessel).adressen.contains(&adresse)
}

/// Erzeugt einen neuen Schlüssel (Ed25519 zum Signieren, Cv25519 zum Verschlüsseln, v4 wie
/// bei GnuPG), geschützt mit `passphrase`.
pub fn erzeugen(
    name: &str,
    email: &str,
    passphrase: &str,
) -> Result<SignedSecretKey, Fehlermeldung> {
    if passphrase.chars().count() < MIN_PASSPHRASE {
        return Err(fehler(
            "Bitte eine Passphrase mit mindestens 8 Zeichen wählen.",
        ));
    }
    let email = email.trim();
    let name = name.trim().replace(['<', '>', '\r', '\n'], " ");
    let user_id = if name.is_empty() {
        format!("<{email}>")
    } else {
        format!("{name} <{email}>")
    };
    let unterschluessel = SubkeyParamsBuilder::default()
        .key_type(KeyType::ECDH(ECCCurve::Curve25519Legacy))
        .can_encrypt(EncryptionCaps::All)
        .passphrase(Some(passphrase.to_string()))
        .build()
        .map_err(|_| fehler("Der Schlüssel konnte nicht erzeugt werden."))?;
    let mut parameter = SecretKeyParamsBuilder::default();
    parameter
        .key_type(KeyType::Ed25519Legacy)
        .can_certify(true)
        .can_sign(true)
        .primary_user_id(user_id)
        .passphrase(Some(passphrase.to_string()))
        .preferred_symmetric_algorithms(smallvec![
            SymmetricKeyAlgorithm::AES256,
            SymmetricKeyAlgorithm::AES128,
        ])
        .preferred_hash_algorithms(smallvec![HashAlgorithm::Sha512, HashAlgorithm::Sha256])
        .preferred_compression_algorithms(smallvec![CompressionAlgorithm::Uncompressed])
        .subkeys(vec![unterschluessel]);
    let schluessel = parameter
        .build()
        .map_err(|_| fehler("Der Schlüssel konnte nicht erzeugt werden."))?
        .generate(rand::rngs::OsRng)
        .map_err(|_| fehler("Der Schlüssel konnte nicht erzeugt werden."))?;
    Ok(schluessel)
}

/// Liest einen privaten Schlüssel (ASCII-armored oder binär) und prüft die Eigensignaturen.
pub fn geheim_lesen(daten: &[u8]) -> Result<SignedSecretKey, Fehlermeldung> {
    if daten.len() > MAX_SCHLUESSELDATEI {
        return Err(fehler("Die Schlüsseldatei ist zu gross."));
    }
    let (schluessel, _) = SignedSecretKey::from_reader_single(daten)
        .map_err(|_| fehler("Die Datei enthält keinen gültigen privaten OpenPGP-Schlüssel."))?;
    schluessel
        .verify_bindings()
        .map_err(|_| fehler("Die Eigensignaturen des Schlüssels sind ungültig."))?;
    Ok(schluessel)
}

/// Liest einen oder mehrere öffentliche Schlüssel und prüft deren Eigensignaturen.
pub fn oeffentlich_lesen(daten: &[u8]) -> Result<Vec<SignedPublicKey>, Fehlermeldung> {
    if daten.len() > MAX_SCHLUESSELDATEI {
        return Err(fehler("Die Schlüsseldatei ist zu gross."));
    }
    if SignedSecretKey::from_reader_single(daten).is_ok() {
        return Err(fehler(
            "Das ist ein privater Schlüssel. Eigene Schlüssel bitte unter „Eigenen Schlüssel importieren“ einlesen und privaten Schlüssel niemals weitergeben.",
        ));
    }
    let keine = || fehler("Die Datei enthält keinen gültigen öffentlichen OpenPGP-Schlüssel.");
    let (schluessel, _) = SignedPublicKey::from_reader_many(daten).map_err(|_| keine())?;
    let mut ergebnis = Vec::new();
    for schluessel in schluessel {
        let schluessel = schluessel.map_err(|_| keine())?;
        schluessel
            .verify_bindings()
            .map_err(|_| fehler("Die Eigensignaturen des Schlüssels sind ungültig."))?;
        if schluessel.details.users.is_empty() {
            return Err(fehler(
                "Der Schlüssel hat keine gültige User-ID (Name und Mailadresse).",
            ));
        }
        ergebnis.push(schluessel);
    }
    if ergebnis.is_empty() {
        return Err(keine());
    }
    Ok(ergebnis)
}

/// Ist der private Schlüssel mit einer Passphrase geschützt?
pub fn geschuetzt(schluessel: &SignedSecretKey) -> bool {
    schluessel.primary_key.secret_params().is_encrypted()
        || schluessel
            .secret_subkeys
            .iter()
            .any(|s| s.key.secret_params().is_encrypted())
}

/// Prüft die Passphrase (alle Teilschlüssel müssen sich damit entsperren lassen).
pub fn passphrase_pruefen(schluessel: &SignedSecretKey, passphrase: &str) -> bool {
    let passwort = Password::from(passphrase);
    let haupt = matches!(
        schluessel.primary_key.unlock(&passwort, |_, _| Ok(())),
        Ok(Ok(()))
    );
    haupt
        && schluessel
            .secret_subkeys
            .iter()
            .all(|s| matches!(s.key.unlock(&passwort, |_, _| Ok(())), Ok(Ok(()))))
}

/// Privater Schlüssel als ASCII-armored Text (bleibt mit der Passphrase geschützt).
pub fn geheim_armored(schluessel: &SignedSecretKey) -> Result<Zeroizing<Vec<u8>>, Fehlermeldung> {
    schluessel
        .to_armored_bytes(ArmorOptions::default())
        .map(Zeroizing::new)
        .map_err(|_| fehler("Der Schlüssel konnte nicht gespeichert werden."))
}

/// Öffentlicher Schlüssel als ASCII-armored Text.
pub fn oeffentlich_armored(schluessel: &SignedPublicKey) -> Result<String, Fehlermeldung> {
    schluessel
        .to_armored_string(ArmorOptions::default())
        .map_err(|_| fehler("Der Schlüssel konnte nicht exportiert werden."))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Eigens für Tests erzeugter Schlüssel (einmal pro Testlauf, nie echte Daten).
    pub(crate) fn test_schluessel(email: &str) -> SignedSecretKey {
        erzeugen("Test Person", email, "test-passphrase").unwrap()
    }

    #[test]
    fn erzeugen_ist_geschuetzt_und_hat_user_id() {
        let schluessel = test_schluessel("anna@example.org");
        assert!(geschuetzt(&schluessel));
        assert!(passphrase_pruefen(&schluessel, "test-passphrase"));
        assert!(!passphrase_pruefen(&schluessel, "falsch-falsch"));
        let info = info(&schluessel.to_public_key());
        assert_eq!(info.benutzer_ids, vec!["Test Person <anna@example.org>"]);
        assert_eq!(info.adressen, vec!["anna@example.org"]);
        assert!(info.kann_verschluesseln);
        assert_eq!(info.fingerabdruck.len(), 40 + 9);
        assert!(erzeugen("x", "a@example.org", "kurz").is_err());
    }

    #[test]
    fn armored_hin_und_zurueck() {
        let schluessel = test_schluessel("bea@example.org");
        let geheim = geheim_armored(&schluessel).unwrap();
        assert!(geheim.starts_with(b"-----BEGIN PGP PRIVATE KEY BLOCK-----"));
        let wieder = geheim_lesen(&geheim).unwrap();
        assert_eq!(
            fingerabdruck_hex(&wieder.primary_key),
            fingerabdruck_hex(&schluessel.primary_key)
        );
        assert!(passphrase_pruefen(&wieder, "test-passphrase"));

        let oeffentlich = oeffentlich_armored(&schluessel.to_public_key()).unwrap();
        assert!(!oeffentlich.contains("PRIVATE"));
        let gelesen = oeffentlich_lesen(oeffentlich.as_bytes()).unwrap();
        assert_eq!(gelesen.len(), 1);
        assert!(passt_zu_adresse(&gelesen[0], "BEA@example.org"));
        assert!(!passt_zu_adresse(&gelesen[0], "anna@example.org"));
        // Ein privater Schlüssel wird nicht als öffentlicher eingelesen.
        assert!(oeffentlich_lesen(&geheim).is_err());
        assert!(oeffentlich_lesen(b"kein Schluessel").is_err());
        assert!(geheim_lesen(oeffentlich.as_bytes()).is_err());
    }

    #[test]
    fn adressen_aus_user_ids() {
        assert_eq!(
            adresse_aus_user_id("Anna Muster <Anna@Example.org>").as_deref(),
            Some("anna@example.org")
        );
        assert_eq!(
            adresse_aus_user_id("anna@example.org").as_deref(),
            Some("anna@example.org")
        );
        assert_eq!(adresse_aus_user_id("Anna Muster"), None);
        assert_eq!(fingerabdruck_normal(" abcd 12ef "), "ABCD12EF");
        assert_eq!(fingerabdruck_anzeige("ABCD1234EF"), "ABCD 1234 EF");
    }
}
