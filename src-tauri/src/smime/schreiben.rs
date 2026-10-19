//! Mails mit S/MIME verschlüsseln und signieren (#56, RFC 8551).
//!
//! - Verschlüsselt wird als EnvelopedData mit AES-256-CBC (breite Verträglichkeit) und
//!   Schlüsseltransport mit RSA bzw. Schlüsselvereinbarung mit EC (beides von OpenSSL).
//! - Signiert wird abgetrennt (`multipart/signed`, SHA-256) mit eigenem Zertifikat und den
//!   Zwischenzertifikaten. Beim Verschlüsseln wird zuerst signiert, dann verschlüsselt.

use std::collections::HashMap;

use openssl::cms::{CMSOptions, CmsContentInfo};
use openssl::pkey::{Id, PKey, Private};
use openssl::symm::Cipher;
use openssl::x509::X509;

use super::zertifikat::{self, fehler};
use crate::fehler::Fehlermeldung;

/// `micalg` für `multipart/signed` (die Signatur verwendet SHA-256).
pub const MICALG: &str = "sha-256";

/// Eigenes Zertifikat mit entsperrtem privatem Schlüssel zum Signieren.
pub struct Signierer {
    pub zertifikat: X509,
    pub schluessel: PKey<Private>,
    /// Zwischenzertifikate, die der Signatur beigelegt werden.
    pub kette: Vec<X509>,
}

/// Zertifikate für den Versand einer Mail.
pub struct Versand {
    /// Zertifikat je Empfängeradresse (kleingeschrieben); nur beim Verschlüsseln.
    pub empfaenger: Option<HashMap<String, X509>>,
    /// Eigenes Zertifikat (es wird immer auch daran verschlüsselt).
    pub eigenes: X509,
    /// Zum Signieren.
    pub signierer: Option<Signierer>,
}

/// Kann OpenSSL an dieses Zertifikat verschlüsseln? RSA (Schlüsseltransport) und EC
/// (Schlüsselvereinbarung nach RFC 5753); anderes wird abgelehnt.
pub fn verfahren_unterstuetzt(zertifikat: &X509) -> bool {
    zertifikat
        .public_key()
        .is_ok_and(|k| matches!(k.id(), Id::RSA | Id::EC))
}

/// Erzeugt eine abgetrennte Signatur (DER, `application/pkcs7-signature`) über `daten`.
pub fn signieren(daten: &[u8], signierer: &Signierer) -> Result<Vec<u8>, Fehlermeldung> {
    let kette = zertifikat::stapel(signierer.kette.iter().cloned())?;
    CmsContentInfo::sign(
        Some(&signierer.zertifikat),
        Some(&signierer.schluessel),
        Some(&kette),
        Some(daten),
        CMSOptions::DETACHED | CMSOptions::BINARY,
    )
    .and_then(|cms| cms.to_der())
    .map_err(|_| fehler("Die Mail konnte nicht signiert werden."))
}

/// Verschlüsselt eine MIME-Entität an die Zertifikate (DER, `application/pkcs7-mime`).
pub fn verschluesseln(daten: &[u8], empfaenger: &[&X509]) -> Result<Vec<u8>, Fehlermeldung> {
    if let Some(z) = empfaenger.iter().find(|z| !verfahren_unterstuetzt(z)) {
        return Err(Fehlermeldung::neu(
            "smime_zertifikat_fehlt",
            format!(
                "An das Zertifikat von {} kann nicht verschlüsselt werden: Sein Schlüsseltyp wird nicht unterstützt (nur RSA und EC).",
                zertifikat::info(z).name
            ),
        ));
    }
    let mut eindeutig: Vec<X509> = Vec::new();
    for z in empfaenger {
        // Jedes Zertifikat nur einmal (z. B. bei einer Mail an sich selbst).
        if !eindeutig
            .iter()
            .any(|e| zertifikat::fingerabdruck_hex(e) == zertifikat::fingerabdruck_hex(z))
        {
            eindeutig.push((*z).clone());
        }
    }
    let stapel = zertifikat::stapel(eindeutig)?;
    CmsContentInfo::encrypt(&stapel, daten, Cipher::aes_256_cbc(), CMSOptions::BINARY)
        .and_then(|cms| cms.to_der())
        .map_err(|_| fehler("Die Mail konnte nicht verschlüsselt werden."))
}
