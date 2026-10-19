//! Integrationstests: Mails über SMTP senden (gegen GreenMail).

mod gemeinsam;

use std::sync::Arc;

use async_imap::types::{Fetch, Flag};
use futures_util::TryStreamExt;
use gemeinsam::*;
use mail_parser::{MessageParser, MimeHeaders};
use sichermail_lib::imap::cache::SpeicherCache;
use sichermail_lib::imap::dienst::ImapDienst;
use sichermail_lib::smtp::dienst::SendeDienst;
use sichermail_lib::smtp::entwurf::Entwurf;

/// Neues Postfach anlegen (GreenMail legt es beim ersten Empfang an).
async fn postfach() -> String {
    let adresse = neue_adresse();
    zustellen(&adresse, &["Willkommen"]).await;
    adresse
}

/// Alle Mails (roh) eines Ordners, direkt vom Server.
async fn mails_im_ordner(adresse: &str, ordner: &str) -> Vec<(Vec<u8>, Vec<String>)> {
    let mut s = direkte_sitzung(adresse).await;
    s.examine(ordner).await.unwrap();
    let antworten: Vec<Fetch> = s
        .fetch("1:*", "(FLAGS BODY.PEEK[])")
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    let _ = s.logout().await;
    antworten
        .iter()
        .map(|f| {
            (
                f.body().unwrap_or_default().to_vec(),
                f.flags().map(|fl| format!("{fl:?}")).collect(),
            )
        })
        .collect()
}

#[tokio::test]
#[ignore = "braucht den Test-Mailserver (scripts/test-mailserver.sh start)"]
async fn mail_mit_anhang_kommt_an_und_liegt_in_gesendet() {
    let absender = postfach().await;
    let empfaenger = postfach().await;
    let bcc = postfach().await;
    let (_ordner, konten, konto) = konto_mit_ports(&absender, IMAP_TLS, SMTP_TLS).await;
    let imap = Arc::new(ImapDienst::neu(
        konten.clone(),
        Arc::new(SpeicherCache::default()),
    ));
    let dienst = SendeDienst::neu(konten, imap);

    let dateien = tempfile::tempdir().unwrap();
    let pfad = dateien.path().join("Bericht.pdf");
    std::fs::write(&pfad, b"%PDF-1.4 Testinhalt").unwrap();
    let anhang = dienst.register.datei_eintragen(pfad).unwrap();

    let ergebnis = dienst
        .senden(&Entwurf {
            konto: konto.id.clone(),
            an: format!("Empfänger <{empfaenger}>"),
            bcc: bcc.clone(),
            betreff: "Bericht mit Anhang".into(),
            text: "Hallo,\n\nanbei der Bericht.\n".into(),
            anhaenge: vec![anhang],
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(ergebnis.abgelegt, "{ergebnis:?}");
    assert!(ergebnis.hinweis.is_none());

    // Beim Empfänger angekommen, mit Anhang, ohne Bcc im Kopf.
    let posteingang = mails_im_ordner(&empfaenger, "INBOX").await;
    assert_eq!(posteingang.len(), 2);
    let roh = &posteingang[1].0;
    let mail = MessageParser::default().parse(roh).unwrap();
    assert_eq!(mail.subject(), Some("Bericht mit Anhang"));
    assert_eq!(mail.attachment_count(), 1);
    let teil = mail.attachment(0).unwrap();
    assert_eq!(teil.attachment_name(), Some("Bericht.pdf"));
    assert_eq!(teil.contents(), b"%PDF-1.4 Testinhalt");
    assert!(!String::from_utf8_lossy(roh).contains(&bcc));

    // Bcc-Empfänger hat sie auch bekommen.
    assert_eq!(mails_im_ordner(&bcc, "INBOX").await.len(), 2);

    // Im Ordner „Gesendet“ des Absenders, als gelesen markiert, mit Bcc für die eigenen Unterlagen.
    let gesendet = mails_im_ordner(&absender, "Sent").await;
    assert_eq!(gesendet.len(), 1);
    assert!(gesendet[0]
        .1
        .iter()
        .any(|f| f == &format!("{:?}", Flag::Seen)));
    let kopie = String::from_utf8_lossy(&gesendet[0].0);
    assert!(kopie.contains("Bericht mit Anhang"));
    assert!(kopie.contains(&bcc));
}

#[tokio::test]
#[ignore = "braucht den Test-Mailserver (scripts/test-mailserver.sh start)"]
async fn antwort_setzt_in_reply_to_und_references() {
    let absender = postfach().await;
    let empfaenger = postfach().await;
    let (_ordner, konten, konto) = konto_mit_ports(&absender, IMAP_TLS, SMTP_TLS).await;
    let imap = Arc::new(ImapDienst::neu(
        konten.clone(),
        Arc::new(SpeicherCache::default()),
    ));
    let dienst = SendeDienst::neu(konten, imap);

    dienst
        .senden(&Entwurf {
            konto: konto.id.clone(),
            an: empfaenger.clone(),
            betreff: "Re: Frage".into(),
            text: "Antwort\n\n> Frage\n".into(),
            antwort_auf: Some("frage-2@example.org".into()),
            referenzen: vec!["start-1@example.org".into(), "frage-2@example.org".into()],
            ..Default::default()
        })
        .await
        .unwrap();

    let posteingang = mails_im_ordner(&empfaenger, "INBOX").await;
    let roh = &posteingang.last().unwrap().0;
    let text = String::from_utf8_lossy(roh);
    assert!(
        text.contains("In-Reply-To: <frage-2@example.org>"),
        "{text}"
    );
    assert!(
        text.contains("References: <start-1@example.org> <frage-2@example.org>"),
        "{text}"
    );
    let mail = MessageParser::default().parse(roh).unwrap();
    assert_eq!(mail.in_reply_to().as_text(), Some("frage-2@example.org"));
}

#[tokio::test]
#[ignore = "braucht den Test-Mailserver (scripts/test-mailserver.sh start)"]
async fn bei_unerreichbarem_server_bleibt_der_text_als_entwurf() {
    let absender = postfach().await;
    let empfaenger = postfach().await;
    let weiterleitung = Weiterleitung::starten(SMTP_TLS).await;
    let (_ordner, konten, konto) = konto_mit_ports(&absender, IMAP_TLS, weiterleitung.port).await;
    let imap = Arc::new(ImapDienst::neu(
        konten.clone(),
        Arc::new(SpeicherCache::default()),
    ));
    let dienst = SendeDienst::neu(konten, imap);

    let dateien = tempfile::tempdir().unwrap();
    let pfad = dateien.path().join("notizen.txt");
    std::fs::write(&pfad, b"Wichtige Notizen").unwrap();
    let anhang = dienst.register.datei_eintragen(pfad).unwrap();
    let entwurf = Entwurf {
        konto: konto.id.clone(),
        an: empfaenger.clone(),
        betreff: "Langer Text".into(),
        text: "Ein langer, mühsam geschriebener Text, der nicht verloren gehen darf.".into(),
        anhaenge: vec![anhang],
        ..Default::default()
    };

    // Server nicht erreichbar: Fehler, Text als Entwurf gesichert.
    weiterleitung.unterbrechen();
    let fehler = dienst.senden(&entwurf).await.unwrap_err();
    assert!(
        fehler.meldung.contains("als Entwurf gespeichert"),
        "{fehler:?}"
    );
    let entwuerfe = dienst.entwuerfe(&konto.id).unwrap();
    assert_eq!(entwuerfe.len(), 1);
    assert_eq!(entwuerfe[0].text, entwurf.text);
    assert_eq!(entwuerfe[0].anhaenge.len(), 1);
    assert_eq!(entwuerfe[0].anhaenge[0].name, "notizen.txt");

    // Server wieder da: Entwurf senden, danach ist er weg.
    weiterleitung.wiederherstellen();
    dienst.senden(&entwuerfe[0]).await.unwrap();
    assert!(dienst.entwuerfe(&konto.id).unwrap().is_empty());
    let posteingang = mails_im_ordner(&empfaenger, "INBOX").await;
    let mail = MessageParser::default()
        .parse(&posteingang.last().unwrap().0)
        .unwrap()
        .into_owned();
    assert_eq!(mail.subject(), Some("Langer Text"));
    assert_eq!(mail.attachment(0).unwrap().contents(), b"Wichtige Notizen");
}
