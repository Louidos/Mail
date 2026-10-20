//! Integrationstests: gemeinsamer Posteingang für mehrere Konten (#50, gegen GreenMail).

mod gemeinsam;

use std::sync::Arc;
use std::time::Duration;

use gemeinsam::*;
use sichermail_lib::imap::cache::SpeicherCache;
use sichermail_lib::imap::dienst::ImapDienst;
use sichermail_lib::smtp::dienst::SendeDienst;
use sichermail_lib::smtp::entwurf::Entwurf;

/// Anzahl Mails in einem Ordner, direkt vom Server.
async fn anzahl_im_ordner(adresse: &str, ordner: &str) -> u32 {
    let mut s = direkte_sitzung(adresse).await;
    let anzahl = s.examine(ordner).await.map(|p| p.exists).unwrap_or(0);
    let _ = s.logout().await;
    anzahl
}

#[tokio::test]
#[ignore = "braucht den Test-Mailserver (scripts/test-mailserver.sh start)"]
async fn zwei_konten_in_einer_liste_und_aktionen_treffen_das_richtige_konto() {
    let a = neue_adresse();
    let b = neue_adresse();
    // Abwechselnd zustellen, damit die Reihenfolge nach Datum die Konten mischt (Date in Sekunden).
    zustellen(&a, &["A alt"]).await;
    tokio::time::sleep(Duration::from_millis(1100)).await;
    zustellen(&b, &["B mittel"]).await;
    tokio::time::sleep(Duration::from_millis(1100)).await;
    zustellen(&a, &["A neu"]).await;

    // Konto A (Standardkonto, zuerst angelegt) über Weiterleitungen, die sich trennen lassen.
    let imap_a = Weiterleitung::starten(IMAP_TLS).await;
    let smtp_a = Weiterleitung::starten(SMTP_TLS).await;
    let (_ordner, konten, konto_a) = konto_mit_ports(&a, imap_a.port, smtp_a.port).await;
    let konto_b = konten.speichern(&eingabe(&b, &b)).await.unwrap();
    assert_eq!(konten.alle().await.unwrap()[0].id, konto_a.id);
    let ids = vec![konto_a.id.clone(), konto_b.id.clone()];

    let cache = Arc::new(SpeicherCache::default());
    let imap = Arc::new(ImapDienst::neu(konten.clone(), cache.clone()));
    // So gleichen sonst die Beobachter (IDLE) die Posteingänge ab.
    imap.abgleichen(&konto_a.id, "INBOX").await.unwrap();
    imap.abgleichen(&konto_b.id, "INBOX").await.unwrap();

    let seite = imap.alle_posteingaenge(&ids, 0, 50, false).await.unwrap();
    let liste: Vec<(&str, &str)> = seite
        .mails
        .iter()
        .map(|m| (m.konto.as_str(), m.kopf.betreff.as_str()))
        .collect();
    assert_eq!(
        liste,
        vec![
            (konto_a.id.as_str(), "A neu"),
            (konto_b.id.as_str(), "B mittel"),
            (konto_a.id.as_str(), "A alt"),
        ]
    );
    assert!(seite.mails.iter().all(|m| m.ordner == "INBOX"));
    assert_eq!(seite.ungelesen, 3);

    // Keine neuen Verbindungen: Der gemeinsame Posteingang liest nur lokal – auch ohne Netz zu A.
    imap.trennen(&konto_a.id).await;
    imap_a.unterbrechen();
    let ohne_netz = imap.alle_posteingaenge(&ids, 0, 50, false).await.unwrap();
    assert_eq!(ohne_netz.mails.len(), 3);
    assert!(ohne_netz.hinweis.is_none());
    // „Ältere Mails laden“ meldet das fehlende Konto, die anderen bleiben da.
    let nachgeladen = imap.alle_posteingaenge(&ids, 0, 50, true).await.unwrap();
    assert_eq!(nachgeladen.mails.len(), 3);
    assert!(nachgeladen.hinweis.is_some());
    imap_a.wiederherstellen();

    // Löschen in der gemeinsamen Ansicht ändert nur die Mail im richtigen Konto.
    let b_mail = seite.mails[1].clone();
    imap.loeschen(&b_mail.konto, &b_mail.ordner, b_mail.kopf.uid)
        .await
        .unwrap();
    assert_eq!(anzahl_im_ordner(&b, "INBOX").await, 0);
    assert_eq!(anzahl_im_ordner(&a, "INBOX").await, 2);

    // Verschieben ebenso.
    let mut direkt = direkte_sitzung(&a).await;
    direkt.create("Archiv").await.unwrap();
    let _ = direkt.logout().await;
    imap.ordner(&konto_a.id).await.unwrap();
    let a_mail = seite.mails[0].clone();
    imap.verschieben(&a_mail.konto, &a_mail.ordner, a_mail.kopf.uid, "Archiv")
        .await
        .unwrap();
    assert_eq!(anzahl_im_ordner(&a, "INBOX").await, 1);
    assert_eq!(anzahl_im_ordner(&a, "Archiv").await, 1);
    assert_eq!(anzahl_im_ordner(&b, "INBOX").await, 0);
    let seite = imap.alle_posteingaenge(&ids, 0, 50, false).await.unwrap();
    assert_eq!(seite.mails.len(), 1);
    assert_eq!(seite.mails[0].kopf.betreff, "A alt");

    // Antwort auf die Mail von Konto B: geht mit Adresse und SMTP-Server von B, obwohl A das
    // Standardkonto ist. Damit ein Versand über A auffiele, ist der SMTP-Server von A nicht erreichbar.
    smtp_a.unterbrechen();
    let empfaenger = neue_adresse();
    zustellen(&empfaenger, &["Postfach anlegen"]).await;
    let senden = SendeDienst::neu(konten.clone(), imap.clone());
    let antwort = Entwurf {
        konto: b_mail.konto.clone(),
        an: empfaenger.clone(),
        betreff: "Re: B mittel".into(),
        text: "Antwort von B".into(),
        ..Default::default()
    };
    senden.senden(&antwort).await.unwrap();
    let mut s = direkte_sitzung(&empfaenger).await;
    s.examine("INBOX").await.unwrap();
    let mails: Vec<async_imap::types::Fetch> = futures_util::TryStreamExt::try_collect(
        s.fetch("1:*", "BODY.PEEK[HEADER.FIELDS (FROM SUBJECT)]")
            .await
            .unwrap(),
    )
    .await
    .unwrap();
    let _ = s.logout().await;
    let koepfe: Vec<String> = mails
        .iter()
        .map(|f| String::from_utf8_lossy(f.header().unwrap_or_default()).into_owned())
        .collect();
    let von_b = koepfe
        .iter()
        .find(|k| k.contains("Re: B mittel"))
        .expect("Antwort angekommen");
    assert!(von_b.contains(&konto_b.email), "{von_b}");
    assert!(!von_b.contains(&konto_a.email), "{von_b}");
    // Die Kopie liegt im „Gesendet“ von B, nicht von A.
    assert!(anzahl_im_ordner(&b, "Sent").await >= 1);
    assert_eq!(anzahl_im_ordner(&a, "Sent").await, 0);

    // Gegenprobe: Über Konto A ginge es nicht (dessen SMTP-Server ist getrennt).
    let ueber_a = Entwurf {
        konto: konto_a.id.clone(),
        ..antwort
    };
    assert!(senden.senden(&ueber_a).await.is_err());
}
