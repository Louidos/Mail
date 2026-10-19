//! S/MIME-Zertifikate einlesen, prüfen und beschreiben; eigene private Schlüssel schützen.
//!
//! - Eigene Zertifikate kommen aus PKCS#12 (`.p12`/`.pfx`). Angenommen wird nur, wenn eine
//!   Adresse im Zertifikat zur Kontoadresse passt, der Schlüssel zum Zertifikat gehört, der
//!   Schlüsselzweck für Mails passt und der Schlüssel stark genug ist (RSA ab 2048 Bit).
//! - Der private Schlüssel wird mit der Passphrase geschützt abgelegt (scrypt + AES-256-GCM)
//!   und nur bei Bedarf entschlüsselt.

use foreign_types::ForeignTypeRef;
use openssl::hash::MessageDigest;
use openssl::nid::Nid;
use openssl::pkcs12::Pkcs12;
use openssl::pkcs7::Pkcs7;
use openssl::pkey::{Id, PKey, Private};
use openssl::stack::Stack;
use openssl::symm::{decrypt_aead, encrypt_aead, Cipher};
use openssl::x509::{X509Ref, X509};
use serde::Serialize;
use zeroize::Zeroizing;

use crate::fehler::Fehlermeldung;

/// Grösste Zertifikatsdatei, die eingelesen wird.
pub const MAX_ZERTIFIKATSDATEI: usize = 1024 * 1024;

/// Mindestgrösse für RSA-Schlüssel.
pub const MIN_RSA_BITS: u32 = 2048;

pub(super) fn fehler(meldung: &str) -> Fehlermeldung {
    Fehlermeldung::neu("smime", meldung)
}

/// Ein Zertifikat für die Oberfläche (ohne geheime Teile).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ZertifikatInfo {
    /// SHA-256-Fingerabdruck, zur Anzeige in Vierergruppen.
    pub fingerabdruck: String,
    /// Name des Inhabers (CN), sonst die erste Adresse.
    pub name: String,
    pub adressen: Vec<String>,
    /// Aussteller (CN bzw. Organisation).
    pub aussteller: String,
    /// Unix-Zeit in Sekunden.
    pub gueltig_ab: i64,
    pub gueltig_bis: i64,
    pub abgelaufen: bool,
}

/// Fingerabdruck (SHA-256, Grossbuchstaben ohne Trenner).
pub fn fingerabdruck_hex(zertifikat: &X509Ref) -> String {
    zertifikat
        .digest(MessageDigest::sha256())
        .map(hex::encode_upper)
        .unwrap_or_default()
}

/// Vierergruppen zur Anzeige.
pub fn fingerabdruck_anzeige(hex: &str) -> String {
    hex.as_bytes()
        .chunks(4)
        .map(|g| String::from_utf8_lossy(g).into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Normalisiert einen eingegebenen Fingerabdruck (Leerzeichen, Doppelpunkte, Kleinbuchstaben).
pub fn fingerabdruck_normal(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_ascii_hexdigit())
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// Jetzt als Unix-Zeit.
pub fn jetzt() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn unix(zeit: &openssl::asn1::Asn1TimeRef) -> i64 {
    openssl::asn1::Asn1Time::from_unix(0)
        .ok()
        .and_then(|null| null.diff(zeit).ok())
        .map(|d| d.days as i64 * 86_400 + d.secs as i64)
        .unwrap_or(0)
}

/// Gültigkeitszeitraum (Unix-Zeit).
pub fn gueltigkeit(zertifikat: &X509Ref) -> (i64, i64) {
    (unix(zertifikat.not_before()), unix(zertifikat.not_after()))
}

fn eintrag(name: &openssl::x509::X509NameRef, nid: Nid) -> Option<String> {
    name.entries_by_nid(nid)
        .next()
        .and_then(|e| e.data().to_string().ok())
}

/// Alle Mailadressen im Zertifikat (SAN `rfc822Name`, ersatzweise Subject `E=`), kleingeschrieben.
pub fn adressen(zertifikat: &X509Ref) -> Vec<String> {
    let mut alle: Vec<String> = zertifikat
        .subject_alt_names()
        .map(|namen| {
            namen
                .iter()
                .filter_map(|n| n.email().map(|e| e.trim().to_lowercase()))
                .collect()
        })
        .unwrap_or_default();
    if alle.is_empty() {
        if let Some(e) = eintrag(zertifikat.subject_name(), Nid::PKCS9_EMAILADDRESS) {
            alle.push(e.trim().to_lowercase());
        }
    }
    alle.dedup();
    alle
}

/// Passt eine Adresse im Zertifikat zu `adresse`?
pub fn passt_zu_adresse(zertifikat: &X509Ref, adresse: &str) -> bool {
    let adresse = adresse.trim().to_lowercase();
    !adresse.is_empty() && adressen(zertifikat).contains(&adresse)
}

/// Beschreibung für die Oberfläche.
pub fn info(zertifikat: &X509Ref) -> ZertifikatInfo {
    let adressen = adressen(zertifikat);
    let name = eintrag(zertifikat.subject_name(), Nid::COMMONNAME)
        .or_else(|| adressen.first().cloned())
        .unwrap_or_else(|| "(ohne Namen)".into());
    let aussteller = eintrag(zertifikat.issuer_name(), Nid::COMMONNAME)
        .or_else(|| eintrag(zertifikat.issuer_name(), Nid::ORGANIZATIONNAME))
        .unwrap_or_else(|| "(unbekannt)".into());
    let (gueltig_ab, gueltig_bis) = gueltigkeit(zertifikat);
    ZertifikatInfo {
        fingerabdruck: fingerabdruck_anzeige(&fingerabdruck_hex(zertifikat)),
        name,
        adressen,
        aussteller,
        gueltig_ab,
        gueltig_bis,
        abgelaufen: gueltig_bis < jetzt(),
    }
}

/// Zweck eines Zertifikats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zweck {
    Signieren,
    Verschluesseln,
}

// Bits aus OpenSSL (x509v3.h): KU_* und XKU_*.
const KU_DIGITAL_SIGNATURE: u32 = 0x0080;
const KU_NON_REPUDIATION: u32 = 0x0040;
const KU_KEY_ENCIPHERMENT: u32 = 0x0020;
const KU_KEY_AGREEMENT: u32 = 0x0008;
const XKU_SMIME: u32 = 0x0004;
const XKU_ANYEKU: u32 = 0x0100;

/// Passt der Schlüsselzweck (KeyUsage, ExtendedKeyUsage `emailProtection`)? Fehlt eine
/// Erweiterung, ist sie keine Einschränkung (RFC 5280).
pub fn zweck_passt(zertifikat: &X509Ref, zweck: Zweck) -> bool {
    // SAFETY: Beide Funktionen lesen nur die (zwischengespeicherten) Erweiterungen eines gültigen
    // Zertifikats; `as_ptr` bleibt für die Dauer des Aufrufs gültig.
    let (ku, xku) = unsafe {
        (
            openssl_sys::X509_get_key_usage(zertifikat.as_ptr()),
            openssl_sys::X509_get_extended_key_usage(zertifikat.as_ptr()),
        )
    };
    let eku_passt = xku == u32::MAX || xku & (XKU_SMIME | XKU_ANYEKU) != 0;
    let ku_passt = ku == u32::MAX
        || match zweck {
            Zweck::Signieren => ku & (KU_DIGITAL_SIGNATURE | KU_NON_REPUDIATION) != 0,
            Zweck::Verschluesseln => ku & (KU_KEY_ENCIPHERMENT | KU_KEY_AGREEMENT) != 0,
        };
    eku_passt && ku_passt
}

/// Ist der öffentliche Schlüssel stark genug? RSA ab 2048 Bit, EC ab 256 Bit.
pub fn schluessel_stark_genug(zertifikat: &X509Ref) -> bool {
    let Ok(schluessel) = zertifikat.public_key() else {
        return false;
    };
    match schluessel.id() {
        Id::RSA => schluessel.bits() >= MIN_RSA_BITS,
        Id::EC => schluessel.bits() >= 256,
        _ => false,
    }
}

/// Ist das Zertifikat eine Zertifizierungsstelle (basicConstraints CA:TRUE)?
pub fn ist_stelle(zertifikat: &X509Ref) -> bool {
    // SAFETY: wie bei `zweck_passt`; liest nur Erweiterungen.
    unsafe { X509_check_ca(zertifikat.as_ptr()) != 0 }
}

// Nicht in `openssl-sys` enthalten, aber Teil der mitgelieferten OpenSSL.
extern "C" {
    fn X509_check_ca(zertifikat: *mut openssl_sys::X509) -> std::ffi::c_int;
}

/// Liest Zertifikate aus PEM (auch mehrere), DER oder PKCS#7 (`.p7c`, „certs-only“).
pub fn lesen(daten: &[u8]) -> Result<Vec<X509>, Fehlermeldung> {
    if daten.len() > MAX_ZERTIFIKATSDATEI {
        return Err(fehler("Die Zertifikatsdatei ist zu gross."));
    }
    let text = String::from_utf8_lossy(daten);
    let mut alle = Vec::new();
    if text.contains("-----BEGIN CERTIFICATE-----") {
        alle = X509::stack_from_pem(daten).unwrap_or_default();
    } else if text.contains("-----BEGIN PKCS7-----") {
        if let Ok(p7) = Pkcs7::from_pem(daten) {
            alle = pkcs7_zertifikate(&p7);
        }
    } else if let Ok(einzeln) = X509::from_der(daten) {
        alle.push(einzeln);
    } else if let Ok(p7) = Pkcs7::from_der(daten) {
        alle = pkcs7_zertifikate(&p7);
    }
    if alle.is_empty() {
        return Err(fehler(
            "Die Datei enthält kein lesbares Zertifikat (erwartet: .cer, .crt, .pem oder .p7c).",
        ));
    }
    Ok(alle)
}

fn pkcs7_zertifikate(p7: &Pkcs7) -> Vec<X509> {
    p7.signed()
        .and_then(|s| s.certificates())
        .map(|z| z.iter().map(|c| c.to_owned()).collect())
        .unwrap_or_default()
}

/// Ein eigenes Zertifikat aus einer PKCS#12-Datei.
pub struct EigenesPaket {
    pub zertifikat: X509,
    pub schluessel: PKey<Private>,
    /// Mitgelieferte Zwischenzertifikate (nicht automatisch vertrauenswürdig).
    pub kette: Vec<X509>,
}

/// Liest eine PKCS#12-Datei und prüft sie für das Konto mit `adresse`.
pub fn pkcs12_lesen(
    daten: &[u8],
    passwort: &str,
    adresse: &str,
) -> Result<EigenesPaket, Fehlermeldung> {
    if daten.len() > MAX_ZERTIFIKATSDATEI {
        return Err(fehler("Die Datei ist zu gross."));
    }
    let teile = Pkcs12::from_der(daten)
        .and_then(|p| p.parse2(passwort))
        .map_err(|_| {
            fehler(
                "Die Datei konnte nicht geöffnet werden: Das Passwort ist falsch oder die Datei ist keine gültige .p12/.pfx-Datei.",
            )
        })?;
    let (Some(zertifikat), Some(schluessel)) = (teile.cert, teile.pkey) else {
        return Err(fehler(
            "Die Datei enthält kein Zertifikat mit privatem Schlüssel.",
        ));
    };
    let kette: Vec<X509> = teile
        .ca
        .map(|s| s.into_iter().collect())
        .unwrap_or_default();
    let passt = zertifikat
        .public_key()
        .map(|o| o.public_eq(&schluessel))
        .unwrap_or(false);
    if !passt {
        return Err(fehler(
            "Der private Schlüssel in der Datei gehört nicht zum Zertifikat.",
        ));
    }
    if !passt_zu_adresse(&zertifikat, adresse) {
        let gefunden = adressen(&zertifikat);
        return Err(fehler(&if gefunden.is_empty() {
            format!("Das Zertifikat passt nicht zu diesem Konto: Es enthält keine Mailadresse, gesucht war {adresse}.")
        } else {
            format!(
                "Das Zertifikat passt nicht zu diesem Konto: Es gilt für {}, nicht für {adresse}.",
                gefunden.join(", ")
            )
        }));
    }
    if !(zweck_passt(&zertifikat, Zweck::Signieren)
        || zweck_passt(&zertifikat, Zweck::Verschluesseln))
    {
        return Err(fehler(
            "Das Zertifikat ist nicht für Mails bestimmt (Schlüsselverwendung bzw. „E-Mail-Schutz“ fehlt).",
        ));
    }
    if ist_stelle(&zertifikat) {
        return Err(fehler(
            "Das ist das Zertifikat einer Zertifizierungsstelle, nicht dein persönliches Zertifikat.",
        ));
    }
    if !schluessel_stark_genug(&zertifikat) {
        return Err(fehler(
            "Der Schlüssel des Zertifikats ist zu schwach (RSA unter 2048 Bit) oder wird nicht unterstützt.",
        ));
    }
    let (ab, bis) = gueltigkeit(&zertifikat);
    let jetzt = jetzt();
    if bis < jetzt {
        return Err(fehler("Das Zertifikat ist abgelaufen."));
    }
    if ab > jetzt {
        return Err(fehler("Das Zertifikat ist noch nicht gültig."));
    }
    Ok(EigenesPaket {
        zertifikat,
        schluessel,
        kette,
    })
}

/// Kennung des Formats für geschützte private Schlüssel.
const FORMAT: &[u8; 4] = b"SMK1";
const SALZ: usize = 16;
const NONCE: usize = 12;
const TAG: usize = 16;
/// scrypt-Parameter: N = 2^15, r = 8, p = 1 (rund 32 MB Speicher).
const SCRYPT_N: u64 = 1 << 15;
const SCRYPT_R: u64 = 8;
const SCRYPT_P: u64 = 1;

fn schluessel_ableiten(
    passphrase: &str,
    salz: &[u8],
) -> Result<Zeroizing<[u8; 32]>, Fehlermeldung> {
    let mut schluessel = Zeroizing::new([0u8; 32]);
    openssl::pkcs5::scrypt(
        passphrase.as_bytes(),
        salz,
        SCRYPT_N,
        SCRYPT_R,
        SCRYPT_P,
        64 * 1024 * 1024,
        &mut schluessel[..],
    )
    .map_err(|_| fehler("Interner Fehler bei der Schlüsselableitung."))?;
    Ok(schluessel)
}

/// Schützt einen privaten Schlüssel mit der Passphrase (scrypt + AES-256-GCM).
pub fn schluessel_schuetzen(
    schluessel: &PKey<Private>,
    passphrase: &str,
) -> Result<Vec<u8>, Fehlermeldung> {
    if passphrase.is_empty() {
        return Err(fehler(
            "SicherMail speichert private Schlüssel nur mit Passphrase. Die Datei braucht ein Passwort.",
        ));
    }
    let klartext = Zeroizing::new(
        schluessel
            .private_key_to_pkcs8()
            .map_err(|_| fehler("Der private Schlüssel kann nicht gelesen werden."))?,
    );
    let mut salz = [0u8; SALZ];
    let mut nonce = [0u8; NONCE];
    openssl::rand::rand_bytes(&mut salz).map_err(|_| fehler("Kein Zufall verfügbar."))?;
    openssl::rand::rand_bytes(&mut nonce).map_err(|_| fehler("Kein Zufall verfügbar."))?;
    let abgeleitet = schluessel_ableiten(passphrase, &salz)?;
    let mut tag = [0u8; TAG];
    let geheim = encrypt_aead(
        Cipher::aes_256_gcm(),
        &abgeleitet[..],
        Some(&nonce),
        FORMAT,
        &klartext,
        &mut tag,
    )
    .map_err(|_| fehler("Der private Schlüssel konnte nicht geschützt werden."))?;
    let mut aus = Vec::with_capacity(4 + SALZ + NONCE + TAG + geheim.len());
    aus.extend_from_slice(FORMAT);
    aus.extend_from_slice(&salz);
    aus.extend_from_slice(&nonce);
    aus.extend_from_slice(&tag);
    aus.extend_from_slice(&geheim);
    Ok(aus)
}

/// Entschlüsselt einen geschützten privaten Schlüssel. `None` bei falscher Passphrase.
pub fn schluessel_oeffnen(geschuetzt: &[u8], passphrase: &str) -> Option<PKey<Private>> {
    let rest = geschuetzt.strip_prefix(FORMAT)?;
    if rest.len() < SALZ + NONCE + TAG {
        return None;
    }
    let (salz, rest) = rest.split_at(SALZ);
    let (nonce, rest) = rest.split_at(NONCE);
    let (tag, geheim) = rest.split_at(TAG);
    let abgeleitet = schluessel_ableiten(passphrase, salz).ok()?;
    let klartext = Zeroizing::new(
        decrypt_aead(
            Cipher::aes_256_gcm(),
            &abgeleitet[..],
            Some(nonce),
            FORMAT,
            geheim,
            tag,
        )
        .ok()?,
    );
    PKey::private_key_from_pkcs8(&klartext).ok()
}

/// Zertifikate als PEM hintereinander (für die Kette).
pub fn als_pem(zertifikate: &[X509]) -> Vec<u8> {
    zertifikate
        .iter()
        .filter_map(|z| z.to_pem().ok())
        .flatten()
        .collect()
}

/// Liest eine gespeicherte Kette (PEM, auch leer).
pub fn kette_lesen(pem: &[u8]) -> Vec<X509> {
    if pem.is_empty() {
        return Vec::new();
    }
    X509::stack_from_pem(pem).unwrap_or_default()
}

/// Stapel für OpenSSL.
pub fn stapel(zertifikate: impl IntoIterator<Item = X509>) -> Result<Stack<X509>, Fehlermeldung> {
    let mut stapel = Stack::new().map_err(|_| fehler("Interner Fehler."))?;
    for z in zertifikate {
        stapel.push(z).map_err(|_| fehler("Interner Fehler."))?;
    }
    Ok(stapel)
}

/// Wurzelzertifikate des Betriebssystems (DER). Fehler einzelner Einträge werden übergangen.
pub fn systemwurzeln() -> Vec<X509> {
    let ergebnis = rustls_native_certs::load_native_certs();
    ergebnis
        .certs
        .iter()
        .filter_map(|z| X509::from_der(z.as_ref()).ok())
        .collect()
}
