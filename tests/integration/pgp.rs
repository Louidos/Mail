//! Integrationstests OpenPGP (#44): Interop mit GnuPG, Schlüssel bleiben nach „Lokalen Speicher
//! leeren“ und Neustart erhalten, verschlüsselte Mails liegen nie entschlüsselt im Speicher.
//!
//! Die Testdaten in `tests/integration/daten/pgp/` wurden eigens mit GnuPG 2.4 erzeugt
//! (siehe LIESMICH.md dort).

mod gemeinsam;

use std::sync::Arc;

use gemeinsam::*;
use sichermail_lib::anzeige;
use sichermail_lib::imap::cache::MailCache;
use sichermail_lib::imap::dienst::ImapDienst;
use sichermail_lib::konto::schluesselbund::{Schluesselbund, SpeicherSchluesselbund};
use sichermail_lib::pgp::lesen::{Signatur, Verschluesselung};
use sichermail_lib::pgp::PgpDienst;
use sichermail_lib::speicher::SqlCache;

const ANNA_PRIVAT: &str = include_str!("daten/pgp/anna-privat.asc");
const ANNA_OEFFENTLICH: &str = include_str!("daten/pgp/anna-oeffentlich.asc");
const BEA_OEFFENTLICH: &str = include_str!("daten/pgp/bea-oeffentlich.asc");
const VERSCHLUESSELT_SIGNIERT: &str = include_str!("daten/pgp/verschluesselt-signiert.asc");
const INLINE: &str = include_str!("daten/pgp/inline.asc");
const KLARTEXT_SIGNIERT: &str = include_str!("daten/pgp/klartext-signiert.asc");
const SIGNIERT_TEIL: &str = include_str!("daten/pgp/signiert-teil.txt");
const SIGNIERT_SIGNATUR: &str = include_str!("daten/pgp/teil1.sig");
const PASSPHRASE: &str = "test-passphrase";

fn kopf(an: &str) -> String {
    format!(
        "From: Bea Test <bea@example.org>\r\nTo: {an}\r\nSubject: Geheim\r\nMessage-ID: <pgp-test@example.org>\r\nMIME-Version: 1.0\r\n"
    )
}

fn pgp_mime(an: &str, armored: &str) -> String {
    format!(
        "{}Content-Type: multipart/encrypted; protocol=\"application/pgp-encrypted\"; boundary=\"aussen\"\r\n\r\n\
--aussen\r\nContent-Type: application/pgp-encrypted\r\n\r\nVersion: 1\r\n\r\n\
--aussen\r\nContent-Type: application/octet-stream\r\n\r\n{}\r\n--aussen--\r\n",
        kopf(an),
        armored.replace('\n', "\r\n").replace("\r\r\n", "\r\n")
    )
}

/// Dienst mit Annas Schlüssel (entsperrt) und Beas öffentlichem Schlüssel.
fn dienst_fuer_anna(ordner: &std::path::Path, bund: Arc<dyn Schluesselbund>) -> PgpDienst {
    let dienst = PgpDienst::neu(ordner, bund);
    assert!(
        dienst
            .eigenen_importieren("k1", "anna@example.org", ANNA_PRIVAT.as_bytes(), "falsch")
            .is_err(),
        "falsche Passphrase muss abgelehnt werden"
    );
    assert!(
        dienst
            .eigenen_importieren("k1", "bea@example.org", ANNA_PRIVAT.as_bytes(), PASSPHRASE)
            .is_err(),
        "Schlüssel muss zur Adresse des Kontos passen"
    );
    let info = dienst
        .eigenen_importieren("k1", "anna@example.org", ANNA_PRIVAT.as_bytes(), PASSPHRASE)
        .unwrap();
    assert_eq!(info.benutzer_ids, vec!["Anna Test <anna@example.org>"]);
    dienst
        .oeffentliche_importieren(BEA_OEFFENTLICH.as_bytes())
        .unwrap();
    dienst.entsperren(&info.fingerabdruck, PASSPHRASE).unwrap();
    dienst
}

#[test]
fn gnupg_mails_werden_gelesen() {
    let ordner = tempfile::tempdir().unwrap();
    let dienst = dienst_fuer_anna(ordner.path(), Arc::new(SpeicherSchluesselbund::default()));

    // PGP/MIME, signiert und verschlüsselt (GnuPG-AEAD/OCB)
    let mail = pgp_mime("anna@example.org", VERSCHLUESSELT_SIGNIERT);
    let ergebnis = dienst.verarbeiten(mail.as_bytes());
    assert_eq!(
        ergebnis.status.verschluesselung,
        Verschluesselung::Entschluesselt
    );
    assert_eq!(ergebnis.status.signatur, Signatur::Gueltig);
    assert_eq!(
        ergebnis.status.signierer.as_deref(),
        Some("Bea Test <bea@example.org>")
    );
    let anzeige = anzeige::aufbereiten(&ergebnis.roh.unwrap(), None).unwrap();
    assert!(anzeige.text.contains("das ist geheim. Grüsse, Bea"));

    // Inline-PGP
    let mail = format!(
        "{}Content-Type: text/plain\r\n\r\n{INLINE}",
        kopf("anna@example.org")
    );
    let ergebnis = dienst.verarbeiten(mail.as_bytes());
    assert_eq!(
        ergebnis.status.verschluesselung,
        Verschluesselung::Entschluesselt
    );
    assert_eq!(ergebnis.status.signatur, Signatur::Gueltig);
    assert!(anzeige::aufbereiten(&ergebnis.roh.unwrap(), None)
        .unwrap()
        .text
        .contains("inline verschluesselt"));

    // Klartext-Signatur
    let mail = format!(
        "{}Content-Type: text/plain\r\n\r\n{KLARTEXT_SIGNIERT}",
        kopf("anna@example.org")
    );
    assert_eq!(
        dienst.verarbeiten(mail.as_bytes()).status.signatur,
        Signatur::Gueltig
    );

    // PGP/MIME signiert, auch nach Veränderung erkannt
    let signiert = |teil: &str| {
        format!(
            "{}Content-Type: multipart/signed; micalg=pgp-sha512; protocol=\"application/pgp-signature\"; boundary=\"s\"\r\n\r\n\
--s\r\n{}\r\n--s\r\nContent-Type: application/pgp-signature\r\n\r\n{SIGNIERT_SIGNATUR}\r\n--s--\r\n",
            kopf("anna@example.org"),
            // Signiert ist der Teil samt abschliessendem Zeilenende (wie von GnuPG erzeugt).
            teil
        )
    };
    let ergebnis = dienst.verarbeiten(signiert(SIGNIERT_TEIL).as_bytes());
    assert_eq!(ergebnis.status.signatur, Signatur::Gueltig);
    assert!(anzeige::aufbereiten(&ergebnis.roh.unwrap(), None)
        .unwrap()
        .text
        .contains("dieser Text ist signiert. Grüsse, Bea"));
    let ergebnis =
        dienst.verarbeiten(signiert(&SIGNIERT_TEIL.replace("Hallo", "Hello")).as_bytes());
    assert_eq!(ergebnis.status.signatur, Signatur::Ungueltig);
}

#[test]
fn ohne_entsperren_wird_die_passphrase_verlangt() {
    let ordner = tempfile::tempdir().unwrap();
    let bund: Arc<dyn Schluesselbund> = Arc::new(SpeicherSchluesselbund::default());
    drop(dienst_fuer_anna(ordner.path(), bund.clone()));
    // Neustart: Passphrase ist vergessen.
    let dienst = PgpDienst::neu(ordner.path(), bund);
    let ergebnis =
        dienst.verarbeiten(pgp_mime("anna@example.org", VERSCHLUESSELT_SIGNIERT).as_bytes());
    assert_eq!(
        ergebnis.status.verschluesselung,
        Verschluesselung::NichtEntschluesselt
    );
    assert!(ergebnis.status.passphrase_fuer.is_some());
    assert!(ergebnis.roh.is_none());
}

#[test]
fn schluessel_ueberleben_speicher_leeren_und_neustart() {
    let ordner = tempfile::tempdir().unwrap();
    let bund = Arc::new(SpeicherSchluesselbund::default());
    let dienst = dienst_fuer_anna(ordner.path(), bund.clone());
    let vorher = dienst.uebersicht().unwrap();
    drop(dienst);

    // „Lokalen Speicher leeren“ und „Mails lokal speichern“ ausschalten (Datei und Schlüssel weg).
    let cache = SqlCache::mit_schluesselbund(ordner.path(), bund.as_ref()).unwrap();
    cache.leeren().unwrap();
    cache.loeschen();
    bund.loeschen(sichermail_lib::speicher::SCHLUESSEL_EINTRAG)
        .unwrap();

    // Neustart
    let dienst = PgpDienst::neu(ordner.path(), bund.clone());
    let nachher = dienst.uebersicht().unwrap();
    assert_eq!(nachher.eigene.len(), 1);
    assert_eq!(nachher.eigene[0].info, vorher.eigene[0].info);
    assert_eq!(nachher.fremde, vorher.fremde);

    // Kein privater Schlüssel im Klartext irgendwo im Datenordner.
    let geheim_kern: String = ANNA_PRIVAT
        .lines()
        .filter(|z| z.len() > 40)
        .nth(2)
        .unwrap()
        .to_string();
    for eintrag in std::fs::read_dir(ordner.path()).unwrap() {
        let inhalt = std::fs::read(eintrag.unwrap().path()).unwrap();
        let text = String::from_utf8_lossy(&inhalt);
        assert!(!text.contains("PRIVATE KEY"));
        assert!(!text.contains(&geheim_kern));
    }

    // Öffentlicher Schlüssel lässt sich exportieren und wieder einlesen.
    let (_, text) = dienst
        .exportieren(&nachher.eigene[0].info.fingerabdruck)
        .unwrap();
    assert!(text.contains("PUBLIC KEY"));
    let oeffentlich_gpg = PgpDienst::neu(tempfile::tempdir().unwrap().path(), bund)
        .oeffentliche_importieren(ANNA_OEFFENTLICH.as_bytes())
        .unwrap();
    assert_eq!(
        oeffentlich_gpg[0].fingerabdruck,
        nachher.eigene[0].info.fingerabdruck
    );
}

async fn roh_zustellen(an: &str, roh: &str) {
    let mut verbindung = sichermail_lib::smtp::anmelden(
        &server(SMTP_TLS, sichermail_lib::konto::Verschluesselung::Tls, true),
        BENUTZER,
        PASSWORT,
    )
    .await
    .unwrap();
    let umschlag = lettre::address::Envelope::new(
        Some("test@localhost".parse().unwrap()),
        vec![an.parse().unwrap()],
    )
    .unwrap();
    verbindung.send(&umschlag, roh.as_bytes()).await.unwrap();
    let _ = verbindung.quit().await;
}

#[tokio::test]
#[ignore = "braucht den Test-Mailserver (scripts/test-mailserver.sh start)"]
async fn verschluesselte_mail_liegt_nur_verschluesselt_im_lokalen_speicher() {
    let adresse = neue_adresse();
    roh_zustellen(&adresse, &pgp_mime(&adresse, VERSCHLUESSELT_SIGNIERT)).await;

    let ordner = tempfile::tempdir().unwrap();
    let bund = Arc::new(SpeicherSchluesselbund::default());
    let pgp = dienst_fuer_anna(ordner.path(), bund.clone());
    let cache = Arc::new(SqlCache::mit_schluesselbund(ordner.path(), bund.as_ref()).unwrap());
    let (_konto_ordner, konten, konto) = konto_mit_imap_port(&adresse, IMAP_TLS).await;
    let dienst = ImapDienst::neu(konten, cache.clone());
    let seite = dienst.mails_seite(&konto.id, "INBOX", 0, 10).await.unwrap();
    let uid = seite.mails[0].uid;
    let roh = dienst.inhalt(&konto.id, "INBOX", uid).await.unwrap();

    let ergebnis = pgp.verarbeiten(&roh);
    assert_eq!(
        ergebnis.status.verschluesselung,
        Verschluesselung::Entschluesselt
    );
    assert!(anzeige::aufbereiten(&ergebnis.roh.unwrap(), None)
        .unwrap()
        .text
        .contains("das ist geheim"));

    // Im lokalen Speicher liegt nur die verschlüsselte Fassung.
    let gespeichert = cache.inhalt(&konto.id, "INBOX", uid).unwrap().unwrap();
    let text = String::from_utf8_lossy(&gespeichert);
    assert!(text.contains("-----BEGIN PGP MESSAGE-----"));
    assert!(!text.contains("das ist geheim"));
}
