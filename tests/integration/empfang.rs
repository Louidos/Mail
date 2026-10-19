//! Integrationstests: Mails über IMAP empfangen (gegen GreenMail).

mod gemeinsam;

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use async_imap::types::Fetch;
use futures_util::TryStreamExt;
use gemeinsam::*;
use sichermail_lib::imap::beobachter::{Beobachter, Ereignis, Melder, Wartezeit};
use sichermail_lib::imap::cache::{MailCache, SpeicherCache};
use sichermail_lib::imap::dienst::ImapDienst;
use sichermail_lib::imap::ordner::Sonderordner;
use sichermail_lib::konto::Verschluesselung;
use tokio::sync::mpsc;
use tokio::time::timeout;

const OHNE_SERVER: &str = "braucht den Test-Mailserver (scripts/test-mailserver.sh start)";

fn melder() -> (Melder, mpsc::UnboundedReceiver<Ereignis>) {
    let (tx, rx) = mpsc::unbounded_channel();
    (Arc::new(move |e| drop(tx.send(e))), rx)
}

/// Wartet auf ein Ereignis, das `passt`.
async fn warten_auf(
    rx: &mut mpsc::UnboundedReceiver<Ereignis>,
    sekunden: u64,
    passt: impl Fn(&Ereignis) -> bool,
) -> Ereignis {
    timeout(Duration::from_secs(sekunden), async {
        loop {
            let e = rx.recv().await.expect("Ereigniskanal geschlossen");
            if passt(&e) {
                return e;
            }
        }
    })
    .await
    .expect("Ereignis kam nicht rechtzeitig")
}

/// UIDs aller Mails im Posteingang, direkt vom Server.
async fn uids_auf_server(adresse: &str) -> Vec<u32> {
    let mut s = direkte_sitzung(adresse).await;
    s.select("INBOX").await.unwrap();
    let mut uids: Vec<u32> = s.uid_search("ALL").await.unwrap().into_iter().collect();
    uids.sort_unstable();
    let _ = s.logout().await;
    uids
}

#[tokio::test]
#[ignore = "braucht den Test-Mailserver (scripts/test-mailserver.sh start)"]
async fn ordner_und_mails_seitenweise() {
    let _ = OHNE_SERVER;
    let adresse = neue_adresse();
    zustellen(&adresse, &["Eins", "Zwei", "Drei", "Vier", "Fünf"]).await;
    let (_ordner, konten, konto) = konto_mit_imap_port(&adresse, IMAP_TLS).await;
    let cache = Arc::new(SpeicherCache::default());
    let dienst = ImapDienst::neu(konten, cache.clone()).mit_erster_seite(2);

    let ordner = dienst.ordner(&konto.id).await.unwrap();
    let posteingang = ordner.first().unwrap();
    assert_eq!(posteingang.name, "INBOX");
    assert_eq!(posteingang.sonder, Some(Sonderordner::Posteingang));
    assert_eq!(posteingang.anzeigename, "Posteingang");

    // Erste Seite: die neuesten zwei, neueste zuerst.
    let seite = dienst.mails_seite(&konto.id, "INBOX", 0, 2).await.unwrap();
    assert_eq!(seite.auf_server, Some(5));
    assert!(seite.offline.is_none());
    let betreffe: Vec<_> = seite.mails.iter().map(|m| m.betreff.as_str()).collect();
    assert_eq!(betreffe, vec!["Fünf", "Vier"]);
    assert!(seite.mails.iter().all(|m| !m.gelesen));
    assert_eq!(seite.mails[0].absender, "Test Absender");
    assert_eq!(seite.mails[0].absender_adresse, "test@localhost");

    // Weitere Seiten werden vom Server nachgeladen.
    let seite2 = dienst.mails_seite(&konto.id, "INBOX", 2, 2).await.unwrap();
    let betreffe: Vec<_> = seite2.mails.iter().map(|m| m.betreff.as_str()).collect();
    assert_eq!(betreffe, vec!["Drei", "Zwei"]);
    let seite3 = dienst.mails_seite(&konto.id, "INBOX", 4, 2).await.unwrap();
    assert_eq!(seite3.mails.len(), 1);
    assert!(!seite3.weitere);

    // Erneuter Abgleich erzeugt keine Duplikate.
    dienst.abgleichen(&konto.id, "INBOX").await.unwrap();
    dienst.abgleichen(&konto.id, "INBOX").await.unwrap();
    assert_eq!(
        cache.uids(&konto.id, "INBOX").unwrap(),
        uids_auf_server(&adresse).await
    );

    // Inhalt laden markiert nicht automatisch als gelesen.
    let uid = seite.mails[0].uid;
    let inhalt = dienst.inhalt(&konto.id, "INBOX", uid).await.unwrap();
    assert!(String::from_utf8_lossy(&inhalt).contains("Inhalt von F"));
    dienst.abgleichen(&konto.id, "INBOX").await.unwrap();
    assert!(
        !cache
            .kopf(&konto.id, "INBOX", uid)
            .unwrap()
            .unwrap()
            .gelesen
    );
}

#[tokio::test]
#[ignore = "braucht den Test-Mailserver (scripts/test-mailserver.sh start)"]
async fn neue_mail_erscheint_ohne_klick() {
    let adresse = neue_adresse();
    zustellen(&adresse, &["Vorher"]).await;
    let (_ordner, konten, konto) = konto_mit_imap_port(&adresse, IMAP_TLS).await;
    let cache = Arc::new(SpeicherCache::default());
    let dienst = Arc::new(ImapDienst::neu(konten, cache.clone()));
    let (melden, mut rx) = melder();
    let beobachter = Beobachter::default();
    beobachter.starten(&konto.id, dienst.clone(), melden);

    let verbunden = warten_auf(&mut rx, 20, |e| matches!(e, Ereignis::Verbunden { .. })).await;
    assert_eq!(
        verbunden,
        Ereignis::Verbunden {
            konto: konto.id.clone(),
            idle: true
        }
    );
    // Der erste Abgleich meldet die vorhandene Mail.
    warten_auf(&mut rx, 20, |e| {
        matches!(e, Ereignis::MailsGeaendert { .. })
    })
    .await;
    assert_eq!(cache.uids(&konto.id, "INBOX").unwrap().len(), 1);

    let gesendet = tokio::time::Instant::now();
    zustellen(&adresse, &["Ganz neu"]).await;
    let ereignis = warten_auf(&mut rx, 10, |e| {
        matches!(e, Ereignis::MailsGeaendert { .. })
    })
    .await;
    assert_eq!(
        ereignis,
        Ereignis::MailsGeaendert {
            konto: konto.id.clone(),
            ordner: "INBOX".into()
        }
    );
    assert!(gesendet.elapsed() < Duration::from_secs(10));
    let seite = cache.seite(&konto.id, "INBOX", 0, 10).unwrap();
    assert_eq!(seite.len(), 2);
    assert_eq!(seite[0].betreff, "Ganz neu");
    beobachter.stoppen(&konto.id);
}

#[tokio::test]
#[ignore = "braucht den Test-Mailserver (scripts/test-mailserver.sh start)"]
async fn nach_verbindungsabbruch_alle_mails_da_keine_doppelt() {
    let adresse = neue_adresse();
    zustellen(&adresse, &["A", "B", "C"]).await;
    let weiterleitung = Weiterleitung::starten(IMAP_TLS).await;
    let (_ordner, konten, konto) = konto_mit_imap_port(&adresse, weiterleitung.port).await;
    let cache = Arc::new(SpeicherCache::default());
    let dienst = Arc::new(ImapDienst::neu(konten, cache.clone()));
    let (melden, mut rx) = melder();
    let beobachter = Beobachter::default();
    beobachter.starten_mit(
        &konto.id,
        dienst.clone(),
        melden,
        Wartezeit::neu(Duration::from_millis(200), Duration::from_secs(2)),
    );
    warten_auf(&mut rx, 20, |e| {
        matches!(e, Ereignis::MailsGeaendert { .. })
    })
    .await;
    assert_eq!(cache.uids(&konto.id, "INBOX").unwrap().len(), 3);

    // Verbindung hart unterbrechen.
    weiterleitung.unterbrechen();
    warten_auf(&mut rx, 20, |e| matches!(e, Ereignis::Getrennt { .. })).await;

    // Während der Unterbrechung: zwei neue Mails, eine Mail wird von aussen gelöscht.
    zustellen(&adresse, &["D", "E"]).await;
    {
        let mut s = direkte_sitzung(&adresse).await;
        s.select("INBOX").await.unwrap();
        let erste = *uids_auf_server(&adresse).await.first().unwrap();
        let _: Vec<Fetch> = s
            .uid_store(erste.to_string(), "+FLAGS.SILENT (\\Deleted)")
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        let _: Vec<u32> = s
            .uid_expunge(erste.to_string())
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        let _ = s.logout().await;
    }

    // Weitere Abbrüche während des Wiederverbindens schaden nicht.
    weiterleitung.wiederherstellen();
    warten_auf(&mut rx, 30, |e| matches!(e, Ereignis::Verbunden { .. })).await;
    warten_auf(&mut rx, 30, |e| {
        matches!(e, Ereignis::MailsGeaendert { .. })
    })
    .await;

    let lokal = cache.uids(&konto.id, "INBOX").unwrap();
    let server = uids_auf_server(&adresse).await;
    assert_eq!(server.len(), 4);
    assert_eq!(
        lokal, server,
        "lokal und Server müssen genau übereinstimmen"
    );
    let eindeutig: HashSet<u32> = lokal.iter().copied().collect();
    assert_eq!(eindeutig.len(), lokal.len(), "keine Mail doppelt");
    let betreffe: HashSet<String> = cache
        .seite(&konto.id, "INBOX", 0, 10)
        .unwrap()
        .into_iter()
        .map(|m| m.betreff)
        .collect();
    assert_eq!(
        betreffe,
        ["B", "C", "D", "E"].into_iter().map(String::from).collect()
    );

    // Auch der Befehlskanal verbindet nach dem Abbruch selbständig neu.
    let seite = dienst.mails_seite(&konto.id, "INBOX", 0, 10).await.unwrap();
    assert!(seite.offline.is_none());
    assert_eq!(seite.mails.len(), 4);
    beobachter.stoppen(&konto.id);
}

#[tokio::test]
#[ignore = "braucht den Test-Mailserver (scripts/test-mailserver.sh start)"]
async fn offline_zeigt_zwischengespeicherte_mails() {
    let adresse = neue_adresse();
    zustellen(&adresse, &["Offline lesbar"]).await;
    let weiterleitung = Weiterleitung::starten(IMAP_TLS).await;
    let (_ordner, konten, konto) = konto_mit_imap_port(&adresse, weiterleitung.port).await;
    let dienst = ImapDienst::neu(konten, Arc::new(SpeicherCache::default()));
    let seite = dienst.mails_seite(&konto.id, "INBOX", 0, 10).await.unwrap();
    assert_eq!(seite.mails.len(), 1);

    weiterleitung.unterbrechen();
    let seite = dienst.mails_seite(&konto.id, "INBOX", 0, 10).await.unwrap();
    assert!(seite.offline.is_some());
    assert_eq!(seite.mails.len(), 1);
    assert_eq!(seite.mails[0].betreff, "Offline lesbar");
}

#[tokio::test]
#[ignore = "braucht den Test-Mailserver (scripts/test-mailserver.sh start)"]
async fn gelesen_verschieben_loeschen_werden_uebertragen() {
    let adresse = neue_adresse();
    zustellen(&adresse, &["Lesen", "Verschieben", "Löschen"]).await;
    {
        let mut s = direkte_sitzung(&adresse).await;
        s.create("Archiv").await.unwrap();
        let _ = s.logout().await;
    }
    let (_ordner, konten, konto) = konto_mit_imap_port(&adresse, IMAP_TLS).await;
    let cache = Arc::new(SpeicherCache::default());
    let dienst = ImapDienst::neu(konten, cache.clone());
    let seite = dienst.mails_seite(&konto.id, "INBOX", 0, 10).await.unwrap();
    let uid_von = |betreff: &str| {
        seite
            .mails
            .iter()
            .find(|m| m.betreff == betreff)
            .unwrap()
            .uid
    };

    // Gelesen
    dienst
        .gelesen_setzen(&konto.id, "INBOX", uid_von("Lesen"), true)
        .await
        .unwrap();
    let mut s = direkte_sitzung(&adresse).await;
    s.select("INBOX").await.unwrap();
    let gelesen: Vec<u32> = s.uid_search("SEEN").await.unwrap().into_iter().collect();
    assert_eq!(gelesen, vec![uid_von("Lesen")]);
    dienst
        .gelesen_setzen(&konto.id, "INBOX", uid_von("Lesen"), false)
        .await
        .unwrap();
    s.noop().await.unwrap();
    assert!(s.uid_search("SEEN").await.unwrap().is_empty());

    // Verschieben
    dienst
        .verschieben(&konto.id, "INBOX", uid_von("Verschieben"), "Archiv")
        .await
        .unwrap();
    let archiv = s.examine("Archiv").await.unwrap();
    assert_eq!(archiv.exists, 1);

    // Löschen (ohne Papierkorb: endgültig, nur diese eine Mail)
    dienst
        .loeschen(&konto.id, "INBOX", uid_von("Löschen"))
        .await
        .unwrap();
    let _ = s.logout().await;
    let server = uids_auf_server(&adresse).await;
    assert_eq!(server, vec![uid_von("Lesen")]);
    assert_eq!(cache.uids(&konto.id, "INBOX").unwrap(), server);
}

#[tokio::test]
#[ignore = "braucht den Test-Mailserver (scripts/test-mailserver.sh start)"]
async fn loeschen_verschiebt_in_den_papierkorb() {
    let adresse = neue_adresse();
    zustellen(&adresse, &["Weg damit"]).await;
    {
        let mut s = direkte_sitzung(&adresse).await;
        s.create("Trash").await.unwrap();
        let _ = s.logout().await;
    }
    let (_ordner, konten, konto) = konto_mit_imap_port(&adresse, IMAP_TLS).await;
    let dienst = ImapDienst::neu(konten, Arc::new(SpeicherCache::default()));
    let seite = dienst.mails_seite(&konto.id, "INBOX", 0, 10).await.unwrap();
    dienst
        .loeschen(&konto.id, "INBOX", seite.mails[0].uid)
        .await
        .unwrap();
    assert!(uids_auf_server(&adresse).await.is_empty());
    let mut s = direkte_sitzung(&adresse).await;
    assert_eq!(s.examine("Trash").await.unwrap().exists, 1);
    let _ = s.logout().await;
}

#[tokio::test]
#[ignore = "braucht den Test-Mailserver (scripts/test-mailserver.sh start)"]
async fn abgleich_loescht_nie_auf_dem_server() {
    let adresse = neue_adresse();
    zustellen(&adresse, &["Bleibt", "Bleibt auch"]).await;
    // Ein anderes Programm markiert eine Mail als gelöscht, ohne sie zu entfernen.
    {
        let mut s = direkte_sitzung(&adresse).await;
        s.select("INBOX").await.unwrap();
        let _: Vec<Fetch> = s
            .store("1", "+FLAGS.SILENT (\\Deleted)")
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        let _ = s.logout().await;
    }
    let (_ordner, konten, konto) = konto_mit_imap_port(&adresse, IMAP_TLS).await;
    let dienst = ImapDienst::neu(konten, Arc::new(SpeicherCache::default()));
    for _ in 0..3 {
        dienst.mails_seite(&konto.id, "INBOX", 0, 10).await.unwrap();
        dienst.ordner(&konto.id).await.unwrap();
    }
    // Die als gelöscht markierte Mail wird ausgeblendet, aber nicht vom Server entfernt.
    let seite = dienst.mails_seite(&konto.id, "INBOX", 0, 10).await.unwrap();
    assert_eq!(seite.mails.len(), 1);
    assert_eq!(uids_auf_server(&adresse).await.len(), 2);
}

#[tokio::test]
#[ignore = "braucht den Test-Mailserver (scripts/test-mailserver.sh start)"]
async fn imap_ohne_gueltiges_zertifikat_oder_starttls_wird_abgelehnt() {
    let fehler = sichermail_lib::imap::anmelden(
        &server(IMAP_TLS, Verschluesselung::Tls, false),
        BENUTZER,
        PASSWORT,
    )
    .await
    .err()
    .unwrap();
    assert_eq!(fehler.art(), "zertifikat_ungueltig");

    let fehler = sichermail_lib::imap::anmelden(
        &server(IMAP_KLARTEXT, Verschluesselung::Starttls, true),
        BENUTZER,
        PASSWORT,
    )
    .await
    .err()
    .unwrap();
    assert_eq!(fehler.art(), "kein_starttls");
}
