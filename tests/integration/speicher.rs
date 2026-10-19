//! Integrationstests: verschlüsselter lokaler Speicher, Lesen ohne Netzverbindung.

mod gemeinsam;

use std::sync::Arc;
use std::time::Duration;

use gemeinsam::*;
use sichermail_lib::anzeige;
use sichermail_lib::imap::beobachter::{Beobachter, Melder};
use sichermail_lib::imap::cache::MailCache;
use sichermail_lib::imap::dienst::ImapDienst;
use sichermail_lib::konto::schluesselbund::SpeicherSchluesselbund;
use sichermail_lib::speicher::{SqlCache, DATEINAME};

#[tokio::test]
#[ignore = "braucht den Test-Mailserver (scripts/test-mailserver.sh start)"]
async fn mails_sind_ohne_netzverbindung_lesbar() {
    let adresse = neue_adresse();
    zustellen(&adresse, &["Offline eins", "Offline zwei", "Offline drei"]).await;
    let weiterleitung = Weiterleitung::starten(IMAP_TLS).await;
    let (ordner, konten, konto) = konto_mit_imap_port(&adresse, weiterleitung.port).await;
    let datenbank = tempfile::tempdir().unwrap();
    let bund = SpeicherSchluesselbund::default();

    {
        let cache = Arc::new(SqlCache::mit_schluesselbund(datenbank.path(), &bund).unwrap());
        let dienst = ImapDienst::neu(konten.clone(), cache);
        let seite = dienst.mails_seite(&konto.id, "INBOX", 0, 10).await.unwrap();
        assert_eq!(seite.mails.len(), 3);
        assert_eq!(
            dienst
                .inhalte_vorladen(&konto.id, "INBOX", 50)
                .await
                .unwrap(),
            3
        );
        // Programm wird beendet.
    }

    // Netz weg, Programm neu gestartet.
    weiterleitung.unterbrechen();
    let cache = Arc::new(SqlCache::mit_schluesselbund(datenbank.path(), &bund).unwrap());
    let dienst = ImapDienst::neu(konten.clone(), cache.clone());
    let seite = dienst.mails_seite(&konto.id, "INBOX", 0, 10).await.unwrap();
    assert!(
        seite.offline.is_some(),
        "ohne Netz muss der Offline-Hinweis kommen"
    );
    let betreffe: Vec<_> = seite.mails.iter().map(|m| m.betreff.as_str()).collect();
    assert_eq!(
        betreffe,
        vec!["Offline drei", "Offline zwei", "Offline eins"]
    );
    let inhalt = dienst
        .inhalt(&konto.id, "INBOX", seite.mails[0].uid)
        .await
        .unwrap();
    let mail = anzeige::aufbereiten(&inhalt, None).unwrap();
    assert_eq!(mail.betreff, "Offline drei");
    assert!(mail.text.contains("Inhalt von Offline drei"));

    // Der Inhalt steht nicht im Klartext in der Datei.
    let roh = std::fs::read(datenbank.path().join(DATEINAME)).unwrap();
    assert!(!roh.windows(12).any(|f| f == b"Offline drei"));

    // „Lokalen Speicher leeren“ entfernt alles.
    cache.leeren().unwrap();
    let seite = dienst.mails_seite(&konto.id, "INBOX", 0, 10).await.unwrap();
    assert!(seite.mails.is_empty());
    assert!(dienst.inhalt(&konto.id, "INBOX", 1).await.is_err());
    drop(ordner);
}

#[tokio::test]
#[ignore = "braucht den Test-Mailserver (scripts/test-mailserver.sh start)"]
async fn neue_mails_werden_fuer_offline_vorgeladen() {
    let adresse = neue_adresse();
    zustellen(&adresse, &["Erste"]).await;
    let (_ordner, konten, konto) = konto_mit_imap_port(&adresse, IMAP_TLS).await;
    let datenbank = tempfile::tempdir().unwrap();
    let bund = SpeicherSchluesselbund::default();
    let cache = Arc::new(SqlCache::mit_schluesselbund(datenbank.path(), &bund).unwrap());
    let dienst = Arc::new(ImapDienst::neu(konten, cache.clone()));
    dienst.vorladen_setzen(true);

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let melden: Melder = Arc::new(move |e| drop(tx.send(e)));
    let beobachter = Beobachter::default();
    beobachter.starten(&konto.id, dienst.clone(), melden);
    zustellen(&adresse, &["Zweite"]).await;

    // Warten, bis beide Mails samt Inhalt lokal liegen.
    let ok = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let uids = cache.uids(&konto.id, "INBOX").unwrap();
            if uids.len() == 2
                && uids
                    .iter()
                    .all(|uid| cache.hat_inhalt(&konto.id, "INBOX", *uid).unwrap())
            {
                return;
            }
            let _ = tokio::time::timeout(Duration::from_millis(200), rx.recv()).await;
        }
    })
    .await;
    assert!(ok.is_ok(), "Inhalte wurden nicht vorgeladen");
    beobachter.stoppen(&konto.id);
}
