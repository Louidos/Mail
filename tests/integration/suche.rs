//! Integrationstests: Suche in Mails (#49) – lokal und auf dem Server (gegen GreenMail).

mod gemeinsam;

use std::sync::Arc;

use gemeinsam::*;
use sichermail_lib::imap::cache::MailCache;
use sichermail_lib::imap::dienst::ImapDienst;
use sichermail_lib::konto::Verschluesselung;
use sichermail_lib::speicher::suche::{SuchFeld, SuchOrt, Suche};
use sichermail_lib::speicher::{SqlCache, DATEINAME};

/// Stellt Mails mit eigenem Betreff und Text zu.
async fn zustellen_mit_text(an: &str, mails: &[(&str, &str)]) {
    use lettre::message::Message;
    let mut verbindung = sichermail_lib::smtp::anmelden(
        &server(SMTP_TLS, Verschluesselung::Tls, true),
        BENUTZER,
        PASSWORT,
    )
    .await
    .expect("Anmeldung beim Test-SMTP-Server");
    for (betreff, text) in mails {
        let mail = Message::builder()
            .from("Jürg Müller <test@localhost>".parse().unwrap())
            .to(an.parse().unwrap())
            .subject(*betreff)
            .body(text.to_string())
            .unwrap();
        verbindung
            .send(mail.envelope(), &mail.formatted())
            .await
            .expect("Mail zustellen");
    }
    let _ = verbindung.quit().await;
}

fn betreffe(seite: &sichermail_lib::speicher::suche::SuchSeite) -> Vec<String> {
    let mut liste: Vec<String> = seite.mails.iter().map(|m| m.kopf.betreff.clone()).collect();
    liste.sort();
    liste
}

#[tokio::test]
#[ignore = "braucht den Test-Mailserver (scripts/test-mailserver.sh start)"]
async fn server_suche_findet_auch_nicht_gespeicherte_mails() {
    let adresse = neue_adresse();
    zustellen_mit_text(
        &adresse,
        &[
            ("Rechnung März", "Bitte bis Ende Monat bezahlen."),
            ("Grüsse aus Zürich", "Das Velo steht am Bahnhof."),
            ("Ferien", "Wir sind ab Montag weg."),
        ],
    )
    .await;
    let (_ordner, konten, konto) = konto_mit_imap_port(&adresse, IMAP_TLS).await;
    let datenordner = tempfile::tempdir().unwrap();
    let cache = Arc::new(SqlCache::oeffnen(&datenordner.path().join(DATEINAME), &[4; 32]).unwrap());
    // Lokal liegt nur die neueste Mail.
    let dienst = ImapDienst::neu(konten, cache.clone()).mit_erster_seite(1);
    dienst.abgleichen(&konto.id, "INBOX").await.unwrap();
    assert_eq!(cache.uids(&konto.id, "INBOX").unwrap().len(), 1);

    let lokal = |begriff: &str| Suche {
        begriff: begriff.into(),
        feld: SuchFeld::Alle,
        orte: vec![SuchOrt {
            konto: konto.id.clone(),
            ordner: None,
        }],
    };
    assert!(dienst
        .suchen(&lokal("Rechnung"), 0, 200)
        .unwrap()
        .mails
        .is_empty());
    assert_eq!(
        dienst.suchen(&lokal("ferien"), 0, 200).unwrap().mails.len(),
        1
    );

    // Auf dem Server: auch Mails, die noch nicht lokal liegen.
    let seite = dienst
        .server_suchen(&konto.id, Some("INBOX"), "Rechnung", SuchFeld::Betreff)
        .await
        .unwrap();
    assert_eq!(betreffe(&seite), vec!["Rechnung März"]);
    assert_eq!(seite.mails[0].ordner, "INBOX");
    assert_eq!(seite.mails[0].konto, konto.id);
    assert!(!seite.weitere);
    // Nicht in den Zwischenspeicher übernommen …
    let uid = seite.mails[0].kopf.uid;
    assert!(cache.kopf(&konto.id, "INBOX", uid).unwrap().is_none());
    // … aber lesbar.
    let inhalt = dienst.inhalt(&konto.id, "INBOX", uid).await.unwrap();
    assert!(String::from_utf8_lossy(&inhalt).contains("bezahlen"));

    // Umlaute gehen als Literal mit CHARSET UTF-8.
    let seite = dienst
        .server_suchen(&konto.id, Some("INBOX"), "Zürich", SuchFeld::Betreff)
        .await
        .unwrap();
    assert_eq!(betreffe(&seite), vec!["Grüsse aus Zürich"]);
    let seite = dienst
        .server_suchen(
            &konto.id,
            Some("INBOX"),
            "test@localhost",
            SuchFeld::Absender,
        )
        .await
        .unwrap();
    assert_eq!(seite.mails.len(), 3);

    // Inhalt und „Alle“.
    let seite = dienst
        .server_suchen(&konto.id, Some("INBOX"), "Velo Bahnhof", SuchFeld::Inhalt)
        .await
        .unwrap();
    assert_eq!(betreffe(&seite), vec!["Grüsse aus Zürich"]);
    let seite = dienst
        .server_suchen(&konto.id, Some("INBOX"), "montag", SuchFeld::Alle)
        .await
        .unwrap();
    assert_eq!(betreffe(&seite), vec!["Ferien"]);
    assert!(dienst
        .server_suchen(&konto.id, Some("INBOX"), "Velo", SuchFeld::Betreff)
        .await
        .unwrap()
        .mails
        .is_empty());

    // Sonderzeichen: keine Fehler, keine fremden Befehle; die Verbindung bleibt brauchbar.
    for begriff in [
        "\"",
        "*",
        "\\",
        "a\"b\\c",
        "{5}",
        "x\r\nA1 LOGOUT",
        "Ä\"Ö\\",
        "😀",
    ] {
        let ergebnis = dienst
            .server_suchen(&konto.id, Some("INBOX"), begriff, SuchFeld::Alle)
            .await;
        assert!(ergebnis.is_ok(), "{begriff:?}: {ergebnis:?}");
        assert!(ergebnis.unwrap().mails.is_empty(), "{begriff:?}");
    }
    let mut direkt = direkte_sitzung(&adresse).await;
    let postfach = direkt.select("INBOX").await.unwrap();
    assert_eq!(postfach.exists, 3, "nichts wurde gelöscht oder verändert");
    let _ = direkt.logout().await;

    // Alle Ordner des Kontos.
    direkt = direkte_sitzung(&adresse).await;
    direkt.create("Archiv").await.unwrap();
    let _ = direkt.logout().await;
    dienst.ordner(&konto.id).await.unwrap();
    dienst
        .verschieben(&konto.id, "INBOX", uid, "Archiv")
        .await
        .unwrap();
    let seite = dienst
        .server_suchen(&konto.id, None, "Rechnung", SuchFeld::Alle)
        .await
        .unwrap();
    assert_eq!(seite.mails.len(), 1);
    assert_eq!(seite.mails[0].ordner, "Archiv");
    assert!(dienst
        .server_suchen(&konto.id, Some("INBOX"), "Rechnung", SuchFeld::Alle)
        .await
        .unwrap()
        .mails
        .is_empty());

    // Nach dem Laden aller Mails findet auch die lokale Suche alles.
    dienst.mails_seite(&konto.id, "INBOX", 0, 50).await.unwrap();
    dienst
        .mails_seite(&konto.id, "Archiv", 0, 50)
        .await
        .unwrap();
    let seite = dienst.suchen(&lokal("Rechnung"), 0, 200).unwrap();
    assert_eq!(seite.mails.len(), 1);
    assert_eq!(seite.mails[0].ordner, "Archiv");
    assert_eq!(
        dienst.suchen(&lokal("zurich"), 0, 200).unwrap().mails.len(),
        1
    );
}
