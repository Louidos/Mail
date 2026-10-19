//! S/MIME-Mails erkennen, entschlüsseln und Signaturen prüfen (#56, RFC 8551).
//!
//! Erkannt werden `application/pkcs7-mime` (enveloped-data, authEnveloped-data und opakes
//! signed-data, auch `x-pkcs7-*`) und `multipart/signed` mit `application/pkcs7-signature`.
//! Es gelten dieselben Regeln wie bei OpenPGP (`crate::pgp::lesen`):
//!
//! - Entschlüsselt wird nur, wenn die **ganze** Mail verschlüsselt ist (EFAIL). Verschlüsselte
//!   Teile zwischen unverschlüsselten werden nur als „teilweise verschlüsselt“ gekennzeichnet.
//! - Höchstens 64 MB Klartext; der Klartext wird nie gespeichert und läuft durch dieselbe
//!   sichere Anzeige wie jede andere Mail.
//! - Nur AES-CBC und AES-GCM werden entschlüsselt; RC2, DES und 3DES werden abgelehnt. Bei CBC
//!   ohne gültige innere Signatur gibt es den Hinweis „nicht gegen Veränderung geschützt“.
//! - Eine Signatur ist nur gültig, wenn die Kette zu einer vertrauenswürdigen Wurzel führt
//!   (Zertifikatsspeicher des Systems oder vom Nutzer übernommen), das Zertifikat zum Zeitpunkt
//!   der Signatur gültig war, der Zweck passt (E-Mail-Schutz) und eine Adresse im Zertifikat zum
//!   Absender passt. MD5 wird abgelehnt, SHA-1 als „schwach“ gekennzeichnet. Sperrlisten (CRL)
//!   und OCSP werden nicht abgefragt – SicherMail verbindet sich dafür mit niemandem.

use std::collections::HashSet;

use foreign_types::ForeignType;
use mail_parser::{Message as MimeNachricht, MessageParser, MimeHeaders};
use openssl::bn::BigNum;
use openssl::cms::{CMSOptions, CmsContentInfo};
use openssl::stack::Stack;
use openssl::x509::store::X509StoreBuilder;
use openssl::x509::verify::X509VerifyParam;
use openssl::x509::{X509PurposeId, X509Ref, X509StoreContext, X509};
use zeroize::Zeroizing;

use super::der::{self, Element};
use super::zertifikat::{self, Zweck};
use crate::pgp::lesen::{
    absender_von, crlf, kind_inhalt, mime_typ, mit_aeusserem_kopf, signierter_teil, PgpStatus,
    Signatur, Verarbeitet, Verfahren, Verschluesselung, TEILWEISE_VERSCHLUESSELT,
};

/// Höchstens so viele Bytes werden entschlüsselt bzw. aus einer Signatur ausgepackt.
const MAX_KLARTEXT: usize = 64 * 1024 * 1024;

/// Hinweis bei jeder geprüften Signatur.
pub const SPERRSTATUS_HINWEIS: &str = "Sperrstatus nicht online geprüft";
/// Hinweis bei AES-CBC ohne gültige innere Signatur.
pub const CBC_HINWEIS: &str = "Verschlüsselt, aber nicht gegen Veränderung geschützt (AES-CBC ohne gültige Signatur): Ein Angreifer könnte Teile des Inhalts unbemerkt verändert haben.";

mod oid {
    pub const SIGNIERT: &str = "1.2.840.113549.1.7.2";
    pub const UMSCHLAG: &str = "1.2.840.113549.1.7.3";
    pub const AUTH_UMSCHLAG: &str = "1.2.840.113549.1.9.16.1.23";
    pub const AES_CBC: [&str; 3] = [
        "2.16.840.1.101.3.4.1.2",
        "2.16.840.1.101.3.4.1.22",
        "2.16.840.1.101.3.4.1.42",
    ];
    pub const AES_GCM: [&str; 3] = [
        "2.16.840.1.101.3.4.1.6",
        "2.16.840.1.101.3.4.1.26",
        "2.16.840.1.101.3.4.1.46",
    ];
    pub const RC2: &str = "1.2.840.113549.3.2";
    pub const DES: &str = "1.3.14.3.2.7";
    pub const DES3: &str = "1.2.840.113549.3.7";
    pub const MD5: [&str; 3] = [
        "1.2.840.113549.2.5",
        "1.2.840.113549.1.1.4",
        "1.2.840.113549.2.2",
    ];
    pub const SHA1: [&str; 4] = [
        "1.3.14.3.2.26",
        "1.2.840.113549.1.1.5",
        "1.2.840.10045.4.1",
        "1.2.840.10040.4.3",
    ];
    pub const SIGNATURZEIT: &str = "1.2.840.113549.1.9.5";
}

/// Ein eigenes Zertifikat mit dem geschützten privaten Schlüssel.
pub struct EigenerSchluessel {
    pub zertifikat: X509,
    /// Mit der Passphrase geschützt (`zertifikat::schluessel_schuetzen`).
    pub geheim: Zeroizing<Vec<u8>>,
    /// Passphrase, falls in dieser Sitzung entsperrt.
    pub passphrase: Option<Zeroizing<String>>,
}

/// Alles, was zum Entschlüsseln und Prüfen gebraucht wird.
#[derive(Default)]
pub struct Ring {
    pub eigene: Vec<EigenerSchluessel>,
    /// Vertrauenswürdige Wurzeln (System und vom Nutzer übernommen).
    pub wurzeln: Vec<X509>,
    /// Bekannte Zertifikate (eigene samt Kette, fremde): helfen, Signierer und Kette zu finden,
    /// sind aber nicht automatisch vertrauenswürdig.
    pub bekannte: Vec<X509>,
    /// Fingerabdrücke (Hex) gespeicherter eigener und fremder Zertifikate.
    pub gespeichert: HashSet<String>,
    /// Prüfzeitpunkt (Unix); `None` = jetzt. Nur für Tests.
    pub jetzt: Option<i64>,
}

impl Ring {
    fn jetzt(&self) -> i64 {
        self.jetzt.unwrap_or_else(zertifikat::jetzt)
    }
}

/// Kann die Mail S/MIME enthalten? Schnelle Vorprüfung, damit für gewöhnliche Mails der
/// Schlüsselspeicher gar nicht erst geöffnet wird.
pub fn moeglich(roh: &[u8]) -> bool {
    let enthaelt = |merkmal: &[u8]| {
        roh.windows(merkmal.len())
            .any(|fenster| fenster.eq_ignore_ascii_case(merkmal))
    };
    enthaelt(b"pkcs7-mime") || enthaelt(b"pkcs7-signature")
}

/// Fehler beim Entschlüsseln.
#[derive(Debug, PartialEq, Eq)]
enum Fehler {
    KeinSchluessel,
    /// Fingerabdruck (zur Anzeige) des eigenen Zertifikats, dessen Passphrase fehlt.
    PassphraseNoetig(String),
    /// Unsicheres Verfahren (Name).
    Unsicher(&'static str),
    ZuGross,
    Kaputt,
}

impl Fehler {
    fn in_status(self, status: &mut PgpStatus) {
        status.verschluesselung = Verschluesselung::NichtEntschluesselt;
        status.meldung = Some(match self {
            Fehler::KeinSchluessel => "Die Mail kann nicht entschlüsselt werden: Es gibt kein passendes eigenes S/MIME-Zertifikat mit privatem Schlüssel.".into(),
            Fehler::PassphraseNoetig(fingerabdruck) => {
                status.passphrase_fuer = Some(fingerabdruck);
                "Zum Entschlüsseln wird die Passphrase deines S/MIME-Zertifikats benötigt.".into()
            }
            Fehler::Unsicher(verfahren) => format!(
                "Die Mail wird nicht entschlüsselt: Sie verwendet das unsichere Verfahren {verfahren}. Bitte den Absender, ein aktuelles Programm (AES) zu verwenden."
            ),
            Fehler::ZuGross => "Die Mail ist zu gross, um sie zu entschlüsseln (höchstens 64 MB).".into(),
            Fehler::Kaputt => "Die Mail kann nicht entschlüsselt werden: Sie ist beschädigt, manipuliert oder verwendet ein nicht unterstütztes Verfahren.".into(),
        });
    }
}

/// Wie der Inhalt verschlüsselt war.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Schutz {
    /// AES-GCM: auch gegen Veränderung geschützt.
    Gcm,
    /// AES-CBC: nur vertraulich.
    Cbc,
}

/// Ergebnis einer Signaturprüfung.
#[derive(Debug, Default)]
struct Pruefung {
    signatur: Signatur,
    signierer: Option<String>,
    schluessel: Option<String>,
    meldung: Option<String>,
    hinweise: Vec<String>,
    /// DER des Signierers, wenn gültig.
    zertifikat: Option<Vec<u8>>,
    uebernehmbar: bool,
}

impl Pruefung {
    fn ungueltig(meldung: Option<String>) -> Self {
        Self {
            signatur: Signatur::Ungueltig,
            meldung,
            ..Self::default()
        }
    }

    fn in_ergebnis(self, ergebnis: &mut Verarbeitet) {
        let status = &mut ergebnis.status;
        status.signatur = self.signatur;
        status.signierer = self.signierer;
        status.signierer_schluessel = self.schluessel;
        if let Some(meldung) = self.meldung {
            status.meldung = Some(match status.meldung.take() {
                Some(alt) => format!("{alt} {meldung}"),
                None => meldung,
            });
        }
        for hinweis in self.hinweise {
            if !status.hinweise.contains(&hinweis) {
                status.hinweise.push(hinweis);
            }
        }
        status.zertifikat_uebernehmbar = self.uebernehmbar;
        ergebnis.zertifikat = self.zertifikat;
    }
}

/// Erkennt S/MIME in einer Mail, entschlüsselt und prüft.
pub fn verarbeiten(roh: &[u8], ring: &Ring) -> Verarbeitet {
    let mut ergebnis = Verarbeitet::default();
    let Some(nachricht) = MessageParser::default().parse(roh) else {
        return ergebnis;
    };
    let absender = absender_von(&nachricht);
    match mime_typ(&nachricht, 0).as_str() {
        "application/pkcs7-mime" | "application/x-pkcs7-mime" => {
            let daten = nachricht
                .parts
                .first()
                .map(|t| t.contents().to_vec())
                .unwrap_or_default();
            opak_verarbeiten(
                roh,
                &nachricht,
                &daten,
                ring,
                absender.as_deref(),
                &mut ergebnis,
            );
        }
        "multipart/signed" => {
            if let Some((anzeige, pruefung)) = signiert_pruefen(roh, ring, absender.as_deref()) {
                ergebnis.roh = Some(anzeige);
                pruefung.in_ergebnis(&mut ergebnis);
            }
        }
        _ => teile_pruefen(roh, &nachricht, ring, absender.as_deref(), &mut ergebnis),
    }
    if ergebnis.status != PgpStatus::default() {
        ergebnis.status.verfahren = Verfahren::Smime;
    }
    ergebnis
}

/// `application/pkcs7-mime` als ganze Mail: entschlüsseln bzw. opake Signatur prüfen.
fn opak_verarbeiten(
    roh: &[u8],
    nachricht: &MimeNachricht,
    daten: &[u8],
    ring: &Ring,
    absender: Option<&str>,
    ergebnis: &mut Verarbeitet,
) {
    let Some((mut cms, der)) = cms_lesen(daten) else {
        // Weder entschlüsselbar noch prüfbar: so kennzeichnen, wie es der Typ verspricht.
        if smime_typ(nachricht, 0).is_some_and(|t| t.contains("signed")) {
            Pruefung::ungueltig(None).in_ergebnis(ergebnis);
        } else {
            Fehler::Kaputt.in_status(&mut ergebnis.status);
        }
        return;
    };
    match inhaltstyp(&der).as_deref() {
        Some(oid::UMSCHLAG) | Some(oid::AUTH_UMSCHLAG) => {
            match entschluesseln(&cms, &der, ring) {
                Ok((klartext, schutz)) => {
                    ergebnis.status.verschluesselung = Verschluesselung::Entschluesselt;
                    let anzeige = mit_aeusserem_kopf(roh, nachricht, &klartext);
                    // Innen signiert (erst signieren, dann verschlüsseln, RFC 8551)?
                    if let Some((innen, pruefung)) = innen_signiert(&anzeige, ring, absender) {
                        ergebnis.roh = Some(innen);
                        pruefung.in_ergebnis(ergebnis);
                    } else {
                        ergebnis.roh = Some(anzeige);
                    }
                    if schutz == Schutz::Cbc && ergebnis.status.signatur != Signatur::Gueltig {
                        ergebnis.status.hinweise.push(CBC_HINWEIS.into());
                    }
                }
                Err(fehler) => fehler.in_status(&mut ergebnis.status),
            }
        }
        Some(oid::SIGNIERT) => {
            let (inhalt, pruefung) = signatur_pruefen(&mut cms, &der, None, ring, absender);
            if let Some(inhalt) = inhalt {
                ergebnis.roh = Some(mit_aeusserem_kopf(roh, nachricht, &inhalt));
            }
            pruefung.in_ergebnis(ergebnis);
        }
        _ => Fehler::Kaputt.in_status(&mut ergebnis.status),
    }
}

/// Prüft, ob der entschlüsselte Inhalt selbst signiert ist, und gibt dann die Anzeige
/// (äussere Kopfzeilen + signierter Inhalt) mit dem Prüfergebnis zurück.
fn innen_signiert(
    anzeige: &[u8],
    ring: &Ring,
    absender: Option<&str>,
) -> Option<(Zeroizing<Vec<u8>>, Pruefung)> {
    let nachricht = MessageParser::default().parse(anzeige)?;
    match mime_typ(&nachricht, 0).as_str() {
        "multipart/signed" => signiert_pruefen(anzeige, ring, absender),
        "application/pkcs7-mime" | "application/x-pkcs7-mime" => {
            let daten = nachricht.parts.first()?.contents().to_vec();
            let (mut cms, der) = cms_lesen(&daten)?;
            if inhaltstyp(&der).as_deref() != Some(oid::SIGNIERT) {
                return None;
            }
            let (inhalt, pruefung) = signatur_pruefen(&mut cms, &der, None, ring, absender);
            let inhalt = inhalt.unwrap_or_default();
            Some((mit_aeusserem_kopf(anzeige, &nachricht, &inhalt), pruefung))
        }
        _ => None,
    }
}

/// Liest CMS (DER oder BER) und gibt es samt DER-Fassung zurück.
fn cms_lesen(daten: &[u8]) -> Option<(CmsContentInfo, Vec<u8>)> {
    if daten.len() > MAX_KLARTEXT + 1024 * 1024 {
        return None;
    }
    let cms = CmsContentInfo::from_der(daten).ok()?;
    let der = cms.to_der().ok()?;
    Some((cms, der))
}

/// `smime-type` aus dem Content-Type eines Teils, kleingeschrieben.
fn smime_typ(nachricht: &MimeNachricht, teil: usize) -> Option<String> {
    nachricht
        .parts
        .get(teil)?
        .content_type()?
        .attribute("smime-type")
        .map(|t| t.to_ascii_lowercase())
}

/// Inhalt (`[0] EXPLICIT`) eines ContentInfo und dessen Typ.
fn content_info(der: &[u8]) -> Option<(String, Element<'_>)> {
    let wurzel = der::ganz(der, der::SEQUENCE)?;
    let kinder = der::kinder(wurzel.inhalt)?;
    let typ = kinder.first().filter(|e| e.tag == der::OID)?;
    let inhalt = kinder.get(1).filter(|e| e.tag == der::kontext(0))?;
    let innen = der::ganz(inhalt.inhalt, der::SEQUENCE)?;
    Some((der::oid_text(typ.inhalt)?, innen))
}

fn inhaltstyp(der: &[u8]) -> Option<String> {
    content_info(der).map(|(typ, _)| typ)
}

/// Verfahren der Inhaltsverschlüsselung prüfen.
fn verschluesselung_pruefen(algorithmus: &str, auth: bool) -> Result<Schutz, Fehler> {
    if oid::AES_GCM.contains(&algorithmus) && auth {
        return Ok(Schutz::Gcm);
    }
    if oid::AES_CBC.contains(&algorithmus) && !auth {
        return Ok(Schutz::Cbc);
    }
    Err(match algorithmus {
        oid::RC2 => Fehler::Unsicher("RC2"),
        oid::DES => Fehler::Unsicher("DES"),
        oid::DES3 => Fehler::Unsicher("Triple-DES"),
        _ => Fehler::Kaputt,
    })
}

/// Empfänger einer verschlüsselten Mail.
#[derive(Debug, Default)]
struct Empfaenger<'a> {
    /// (Aussteller als DER-Inhalt des Namens, Seriennummer)
    aussteller_seriennummer: Vec<(&'a [u8], &'a [u8])>,
    schluessel_ids: Vec<&'a [u8]>,
    /// Schlüsselvereinbarung (EC): passende Zertifikate werden durchprobiert.
    vereinbarung: bool,
}

/// Liest Verfahren und Empfänger aus EnvelopedData bzw. AuthEnvelopedData.
fn umschlag_lesen(der: &[u8]) -> Result<(Schutz, Empfaenger<'_>), Fehler> {
    let (typ, umschlag) = content_info(der).ok_or(Fehler::Kaputt)?;
    let auth = typ == oid::AUTH_UMSCHLAG;
    let kinder = der::kinder(umschlag.inhalt).ok_or(Fehler::Kaputt)?;
    // version, [0] originatorInfo (optional), recipientInfos, (auth)encryptedContentInfo, …
    let mut rest = kinder.iter().skip(1).filter(|e| e.tag != der::kontext(0));
    let empfaenger_menge = rest
        .next()
        .filter(|e| e.tag == der::SET)
        .ok_or(Fehler::Kaputt)?;
    let inhalt = rest
        .next()
        .filter(|e| e.tag == der::SEQUENCE)
        .ok_or(Fehler::Kaputt)?;
    let inhalt_kinder = der::kinder(inhalt.inhalt).ok_or(Fehler::Kaputt)?;
    let algorithmus = inhalt_kinder
        .get(1)
        .and_then(der::algorithmus)
        .ok_or(Fehler::Kaputt)?;
    let schutz = verschluesselung_pruefen(&algorithmus, auth)?;

    let mut empfaenger = Empfaenger::default();
    for info in der::kinder(empfaenger_menge.inhalt).ok_or(Fehler::Kaputt)? {
        match info.tag {
            // KeyTransRecipientInfo: version, rid, …
            der::SEQUENCE => {
                let teile = der::kinder(info.inhalt).ok_or(Fehler::Kaputt)?;
                match teile.get(1) {
                    Some(rid) if rid.tag == der::SEQUENCE => {
                        let is = der::kinder(rid.inhalt).ok_or(Fehler::Kaputt)?;
                        if let (Some(name), Some(nummer)) = (is.first(), is.get(1)) {
                            empfaenger
                                .aussteller_seriennummer
                                .push((name.inhalt, nummer.inhalt));
                        }
                    }
                    Some(rid) if rid.tag == der::kontext_einfach(0) => {
                        empfaenger.schluessel_ids.push(rid.inhalt);
                    }
                    _ => {}
                }
            }
            t if t == der::kontext(1) => empfaenger.vereinbarung = true,
            _ => {}
        }
    }
    Ok((schutz, empfaenger))
}

/// Ist dieses Zertifikat einer der Empfänger?
fn ist_empfaenger(zertifikat: &X509Ref, empfaenger: &Empfaenger) -> bool {
    let aussteller = zertifikat
        .issuer_name()
        .to_der()
        .ok()
        .and_then(|d| der::lesen(&d).map(|(e, _)| e.inhalt.to_vec()));
    let nummer = zertifikat.serial_number().to_bn().ok();
    let passt_is = empfaenger.aussteller_seriennummer.iter().any(|(name, n)| {
        aussteller.as_deref() == Some(*name)
            && match (&nummer, BigNum::from_slice(n)) {
                (Some(a), Ok(b)) => *a == b,
                _ => false,
            }
    });
    let passt_ski = zertifikat
        .subject_key_id()
        .is_some_and(|id| empfaenger.schluessel_ids.contains(&id.as_slice()));
    let ec = zertifikat
        .public_key()
        .is_ok_and(|k| k.id() == openssl::pkey::Id::EC);
    passt_is || passt_ski || (empfaenger.vereinbarung && ec)
}

/// Entschlüsselt EnvelopedData (AES-CBC) bzw. AuthEnvelopedData (AES-GCM).
fn entschluesseln(
    cms: &CmsContentInfo,
    der: &[u8],
    ring: &Ring,
) -> Result<(Zeroizing<Vec<u8>>, Schutz), Fehler> {
    if der.len() > MAX_KLARTEXT + 1024 * 1024 {
        return Err(Fehler::ZuGross);
    }
    let (schutz, empfaenger) = umschlag_lesen(der)?;
    let passende: Vec<&EigenerSchluessel> = ring
        .eigene
        .iter()
        .filter(|e| ist_empfaenger(&e.zertifikat, &empfaenger))
        .collect();
    if passende.is_empty() {
        return Err(Fehler::KeinSchluessel);
    }
    let mut versucht = false;
    for eigener in &passende {
        let Some(passphrase) = &eigener.passphrase else {
            continue;
        };
        let Some(schluessel) = zertifikat::schluessel_oeffnen(&eigener.geheim, passphrase) else {
            continue;
        };
        versucht = true;
        if let Ok(klartext) = cms.decrypt(&schluessel, &eigener.zertifikat) {
            let klartext = Zeroizing::new(klartext);
            if klartext.len() > MAX_KLARTEXT {
                return Err(Fehler::ZuGross);
            }
            return Ok((klartext, schutz));
        }
    }
    if versucht {
        return Err(Fehler::Kaputt);
    }
    Err(Fehler::PassphraseNoetig(zertifikat::fingerabdruck_anzeige(
        &zertifikat::fingerabdruck_hex(&passende[0].zertifikat),
    )))
}

/// Prüft eine `multipart/signed`-Entität mit `application/pkcs7-signature`. Gibt die Mail zur
/// Anzeige (äussere Kopfzeilen + signierter Teil) und das Prüfergebnis zurück.
fn signiert_pruefen(
    entitaet: &[u8],
    ring: &Ring,
    absender: Option<&str>,
) -> Option<(Zeroizing<Vec<u8>>, Pruefung)> {
    let kanonisch = crlf(entitaet);
    let nachricht = MessageParser::default().parse(&kanonisch)?;
    if mime_typ(&nachricht, 0) != "multipart/signed" {
        return None;
    }
    let typ = nachricht.parts.first()?.content_type()?;
    let protokoll = typ.attribute("protocol")?.to_ascii_lowercase();
    if protokoll != "application/pkcs7-signature" && protokoll != "application/x-pkcs7-signature" {
        return None;
    }
    let grenze = typ.attribute("boundary")?.to_string();
    let signiert = signierter_teil(&kanonisch, &grenze)?;
    let anzeige = mit_aeusserem_kopf(&kanonisch, &nachricht, signiert);
    let pruefung = match kind_inhalt(&nachricht, 0, 1).and_then(|d| cms_lesen(&d)) {
        Some((mut cms, der)) if inhaltstyp(&der).as_deref() == Some(oid::SIGNIERT) => {
            signatur_pruefen(&mut cms, &der, Some(signiert), ring, absender).1
        }
        _ => Pruefung::ungueltig(None),
    };
    Some((anzeige, pruefung))
}

/// Verfahren und Signaturzeit aus SignedData.
#[derive(Debug, Default, PartialEq, Eq)]
struct SignaturAngaben {
    md5: bool,
    sha1: bool,
    zeit: Option<i64>,
}

fn signatur_angaben(der: &[u8]) -> Option<SignaturAngaben> {
    let (_, signiert) = content_info(der)?;
    let kinder = der::kinder(signiert.inhalt)?;
    let mut oids = Vec::new();
    // digestAlgorithms
    for a in der::kinder(kinder.get(1).filter(|e| e.tag == der::SET)?.inhalt)? {
        oids.extend(der::algorithmus(&a));
    }
    let mut angaben = SignaturAngaben::default();
    // signerInfos: die letzte SET
    let infos = kinder.iter().rev().find(|e| e.tag == der::SET)?;
    for info in der::kinder(infos.inhalt)? {
        for teil in der::kinder(info.inhalt)? {
            match teil.tag {
                der::SEQUENCE => oids.extend(der::algorithmus(&teil)),
                // signedAttrs [0] IMPLICIT SET OF Attribute
                t if t == der::kontext(0) => {
                    for attribut in der::kinder(teil.inhalt)? {
                        let a = der::kinder(attribut.inhalt)?;
                        let ist_zeit = a
                            .first()
                            .and_then(|o| der::oid_text(o.inhalt))
                            .is_some_and(|o| o == oid::SIGNATURZEIT);
                        if ist_zeit {
                            let werte = der::kinder(a.get(1)?.inhalt)?;
                            angaben.zeit = angaben.zeit.or(werte.first().and_then(der::zeit));
                        }
                    }
                }
                _ => {}
            }
        }
    }
    angaben.md5 = oids.iter().any(|o| oid::MD5.contains(&o.as_str()));
    angaben.sha1 = oids.iter().any(|o| oid::SHA1.contains(&o.as_str()));
    Some(angaben)
}

// Nicht in `openssl-sys` enthalten, aber Teil der mitgelieferten OpenSSL.
extern "C" {
    fn CMS_get0_signers(cms: *mut openssl_sys::CMS_ContentInfo) -> *mut openssl_sys::stack_st_X509;
    fn CMS_get1_certs(cms: *mut openssl_sys::CMS_ContentInfo) -> *mut openssl_sys::stack_st_X509;
}

/// Zertifikate in der Nachricht.
fn mitgelieferte(cms: &CmsContentInfo) -> Vec<X509> {
    // SAFETY: CMS_get1_certs gibt einen neuen Stapel mit eigenen Referenzen zurück (oder NULL);
    // `Stack::from_ptr` übernimmt beides und gibt es wieder frei.
    unsafe {
        let stapel = CMS_get1_certs(cms.as_ptr());
        if stapel.is_null() {
            return Vec::new();
        }
        Stack::<X509>::from_ptr(stapel).into_iter().collect()
    }
}

/// Zertifikate der Signierer (nach erfolgreichem `CMS_verify`).
fn signierer(cms: &CmsContentInfo) -> Vec<X509> {
    // SAFETY: CMS_get0_signers gibt einen neuen Stapel mit *geliehenen* Zertifikaten zurück. Die
    // Zertifikate werden mit `to_owned` (Referenz erhöhen) übernommen; danach wird nur der Stapel
    // selbst freigegeben, nicht die Zertifikate.
    unsafe {
        let stapel = CMS_get0_signers(cms.as_ptr());
        if stapel.is_null() {
            return Vec::new();
        }
        let geliehen = std::mem::ManuallyDrop::new(Stack::<X509>::from_ptr(stapel));
        let alle = geliehen.iter().map(|z| z.to_owned()).collect();
        openssl_sys::OPENSSL_sk_free(stapel as *mut openssl_sys::OPENSSL_STACK);
        alle
    }
}

/// Fehlercodes von `X509_verify_cert` (x509_vfy.h).
mod pruefcode {
    pub const NOCH_NICHT_GUELTIG: i32 = 9;
    pub const ABGELAUFEN: i32 = 10;
    pub const FALSCHER_ZWECK: i32 = 26;
}

/// Prüft eine CMS-Signatur. Gibt den signierten Inhalt (bei opaker Signatur) und das Ergebnis
/// zurück.
fn signatur_pruefen(
    cms: &mut CmsContentInfo,
    der: &[u8],
    abgetrennt: Option<&[u8]>,
    ring: &Ring,
    absender: Option<&str>,
) -> (Option<Zeroizing<Vec<u8>>>, Pruefung) {
    let Some(angaben) = signatur_angaben(der) else {
        return (None, Pruefung::ungueltig(None));
    };
    if angaben.md5 {
        return (
            None,
            Pruefung::ungueltig(Some(
                "Die Signatur verwendet das unsichere Verfahren MD5 und wird nicht anerkannt."
                    .into(),
            )),
        );
    }
    let mut hinweise = Vec::new();
    if angaben.sha1 {
        hinweise.push("Schwache Signatur (SHA-1)".to_string());
    }

    // 1. Unverändert? (ohne Kettenprüfung; die folgt unten mit eigenen Regeln)
    let Ok(bekannte) = zertifikat::stapel(ring.bekannte.iter().cloned()) else {
        return (None, Pruefung::ungueltig(None));
    };
    let mut inhalt = Vec::new();
    let geprueft = cms.verify(
        Some(&bekannte),
        None,
        abgetrennt,
        Some(&mut inhalt),
        CMSOptions::NO_SIGNER_CERT_VERIFY | CMSOptions::BINARY,
    );
    let inhalt = Zeroizing::new(inhalt);
    if inhalt.len() > MAX_KLARTEXT {
        return (None, Pruefung::ungueltig(None));
    }
    if let Err(fehler) = geprueft {
        let ohne_zertifikat = fehler
            .errors()
            .iter()
            .any(|e| e.reason() == Some("signer certificate not found"));
        if ohne_zertifikat {
            let pruefung = Pruefung {
                signatur: Signatur::Unbekannt,
                meldung: Some(
                    "Das Zertifikat des Absenders liegt der Mail nicht bei und ist nicht gespeichert."
                        .into(),
                ),
                hinweise,
                ..Pruefung::default()
            };
            return (None, pruefung);
        }
        return (None, Pruefung::ungueltig(None));
    }
    let ausgabe = (abgetrennt.is_none()).then(|| inhalt.clone());

    // 2. Wer hat signiert?
    let Some(signierer) = signierer(cms).into_iter().next() else {
        return (ausgabe, Pruefung::ungueltig(None));
    };
    let info = zertifikat::info(&signierer);
    let mut pruefung = Pruefung {
        signierer: Some(match info.adressen.first() {
            Some(a) if *a != info.name => format!("{} <{a}>", info.name),
            _ => info.name.clone(),
        }),
        schluessel: Some(info.fingerabdruck.clone()),
        hinweise,
        ..Pruefung::default()
    };
    if !zertifikat::schluessel_stark_genug(&signierer) {
        pruefung.signatur = Signatur::Ungueltig;
        pruefung.meldung = Some(
            "Die Signatur wird nicht anerkannt: Der Schlüssel des Absenders ist zu schwach (RSA unter 2048 Bit)."
                .into(),
        );
        return (ausgabe, pruefung);
    }

    // 3. Vertrauen: Kette zu einer Wurzel, gültig zum Zeitpunkt der Signatur, Zweck E-Mail.
    let jetzt = ring.jetzt();
    let zeit = angaben.zeit.filter(|z| *z <= jetzt).unwrap_or(jetzt);
    let mut kette = mitgelieferte(cms);
    kette.extend(ring.bekannte.iter().cloned());
    pruefung.signatur = match kette_pruefen(&signierer, kette, &ring.wurzeln, zeit) {
        Kette::Gueltig => {
            if !zertifikat::zweck_passt(&signierer, Zweck::Signieren) {
                pruefung.meldung =
                    Some("Das Zertifikat ist nicht zum Signieren von Mails bestimmt.".into());
                Signatur::Ungueltig
            } else if absender.is_some_and(|a| zertifikat::passt_zu_adresse(&signierer, a)) {
                Signatur::Gueltig
            } else {
                Signatur::AndereAdresse
            }
        }
        Kette::Abgelaufen => Signatur::Abgelaufen,
        Kette::FalscherZweck => {
            pruefung.meldung =
                Some("Das Zertifikat ist nicht zum Signieren von Mails bestimmt.".into());
            Signatur::Ungueltig
        }
        Kette::Unbekannt => Signatur::Unbekannt,
    };
    if matches!(
        pruefung.signatur,
        Signatur::Gueltig | Signatur::AndereAdresse | Signatur::Abgelaufen
    ) {
        pruefung.hinweise.push(SPERRSTATUS_HINWEIS.into());
    }
    if pruefung.signatur == Signatur::Gueltig {
        let (_, bis) = zertifikat::gueltigkeit(&signierer);
        if bis < jetzt {
            pruefung
                .hinweise
                .push("Das Zertifikat ist inzwischen abgelaufen.".into());
        }
        let fingerabdruck = zertifikat::fingerabdruck_hex(&signierer);
        pruefung.uebernehmbar = !ring.gespeichert.contains(&fingerabdruck);
        pruefung.zertifikat = signierer.to_der().ok();
    }
    (ausgabe, pruefung)
}

/// Ergebnis der Kettenprüfung.
#[derive(Debug, PartialEq, Eq)]
enum Kette {
    Gueltig,
    Abgelaufen,
    FalscherZweck,
    Unbekannt,
}

/// Prüft die Kette von `zertifikat` zu einer der `wurzeln` zum Zeitpunkt `zeit`.
fn kette_pruefen(zertifikat: &X509Ref, zwischen: Vec<X509>, wurzeln: &[X509], zeit: i64) -> Kette {
    let pruefen = || -> Result<i32, openssl::error::ErrorStack> {
        let mut bauer = X509StoreBuilder::new()?;
        for wurzel in wurzeln {
            // Doppelte Wurzeln sind kein Fehler.
            let _ = bauer.add_cert(wurzel.clone());
        }
        let mut parameter = X509VerifyParam::new()?;
        // `time_t` ist je nach System verschieden breit.
        #[allow(clippy::useless_conversion)]
        parameter.set_time(zeit.try_into().unwrap_or_default());
        parameter.set_purpose(X509PurposeId::SMIME_SIGN)?;
        bauer.set_param(&parameter)?;
        let speicher = bauer.build();
        let zwischen =
            zertifikat::stapel(zwischen).map_err(|_| openssl::error::ErrorStack::get())?;
        let mut kontext = X509StoreContext::new()?;
        kontext.init(&speicher, zertifikat, &zwischen, |k| {
            let ok = k.verify_cert()?;
            Ok(if ok { 0 } else { k.error().as_raw() })
        })
    };
    match pruefen() {
        Ok(0) => Kette::Gueltig,
        Ok(pruefcode::ABGELAUFEN | pruefcode::NOCH_NICHT_GUELTIG) => Kette::Abgelaufen,
        Ok(pruefcode::FALSCHER_ZWECK) => Kette::FalscherZweck,
        _ => Kette::Unbekannt,
    }
}

/// Durchsucht verschachtelte Teile: Verschlüsselte Teile werden nur gekennzeichnet, signierte
/// Teile geprüft und als „teilweise signiert“ gekennzeichnet.
fn teile_pruefen(
    roh: &[u8],
    nachricht: &MimeNachricht,
    ring: &Ring,
    absender: Option<&str>,
    ergebnis: &mut Verarbeitet,
) {
    for (index, teil) in nachricht.parts.iter().enumerate().skip(1) {
        let typ = mime_typ(nachricht, index);
        let entitaet = roh.get(teil.offset_header as usize..teil.offset_end as usize);
        match typ.as_str() {
            "application/pkcs7-mime" | "application/x-pkcs7-mime" => {
                let daten = teil.contents();
                let art = cms_lesen(daten).and_then(|(_, der)| inhaltstyp(&der));
                match art.as_deref() {
                    Some(oid::UMSCHLAG) | Some(oid::AUTH_UMSCHLAG) => {
                        ergebnis.status.verschluesselung = Verschluesselung::Teilweise;
                        ergebnis.status.meldung = Some(TEILWEISE_VERSCHLUESSELT.into());
                    }
                    Some(oid::SIGNIERT) if ergebnis.status.signatur == Signatur::Keine => {
                        if let Some((mut cms, der)) = cms_lesen(daten) {
                            let (_, pruefung) =
                                signatur_pruefen(&mut cms, &der, None, ring, absender);
                            teilweise_signiert(pruefung, ergebnis);
                        }
                    }
                    _ => {}
                }
            }
            "multipart/signed" if ergebnis.status.signatur == Signatur::Keine => {
                if let Some((_, pruefung)) =
                    entitaet.and_then(|e| signiert_pruefen(e, ring, absender))
                {
                    teilweise_signiert(pruefung, ergebnis);
                }
            }
            _ => {}
        }
    }
}

fn teilweise_signiert(mut pruefung: Pruefung, ergebnis: &mut Verarbeitet) {
    // Nur ein Teil ist signiert: Das Zertifikat wird nicht zum Übernehmen angeboten.
    pruefung.uebernehmbar = false;
    pruefung.zertifikat = None;
    pruefung.in_ergebnis(ergebnis);
    ergebnis.status.signatur_teilweise = true;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nur_aes_wird_entschluesselt() {
        assert_eq!(
            verschluesselung_pruefen("2.16.840.1.101.3.4.1.42", false),
            Ok(Schutz::Cbc)
        );
        assert_eq!(
            verschluesselung_pruefen("2.16.840.1.101.3.4.1.46", true),
            Ok(Schutz::Gcm)
        );
        // GCM gehört in AuthEnvelopedData, CBC nicht.
        assert_eq!(
            verschluesselung_pruefen("2.16.840.1.101.3.4.1.46", false),
            Err(Fehler::Kaputt)
        );
        assert_eq!(
            verschluesselung_pruefen(oid::RC2, false),
            Err(Fehler::Unsicher("RC2"))
        );
        assert_eq!(
            verschluesselung_pruefen(oid::DES, false),
            Err(Fehler::Unsicher("DES"))
        );
        assert_eq!(
            verschluesselung_pruefen(oid::DES3, false),
            Err(Fehler::Unsicher("Triple-DES"))
        );
    }

    #[test]
    fn erkennt_smime_schnell() {
        assert!(moeglich(
            b"Content-Type: application/pkcs7-mime; smime-type=enveloped-data"
        ));
        assert!(moeglich(b"Content-Type: Application/X-PKCS7-Signature"));
        assert!(!moeglich(
            b"Content-Type: multipart/signed; protocol=\"application/pgp-signature\""
        ));
    }

    #[test]
    fn gewoehnliche_mail_bleibt_unveraendert() {
        let roh = b"From: a@example.org\r\nSubject: x\r\nContent-Type: text/plain\r\n\r\nHallo\r\n";
        let ergebnis = verarbeiten(roh, &Ring::default());
        assert!(ergebnis.roh.is_none());
        assert_eq!(ergebnis.status, PgpStatus::default());
    }
}
