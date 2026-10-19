//! Integrationstest: Eine präparierte Mail mit Skripten und Tracking über den Test-Mailserver
//! empfangen und aufbereiten – ohne Skripte und ohne jeden Netzwerkzugriff.

mod gemeinsam;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use gemeinsam::*;
use sichermail_lib::anzeige;
use sichermail_lib::imap::cache::SpeicherCache;
use sichermail_lib::imap::dienst::ImapDienst;

/// Zählt alle eingehenden Verbindungen auf einem lokalen Port („Tracking-Server“).
async fn lauscher() -> (u16, Arc<AtomicUsize>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let zaehler = Arc::new(AtomicUsize::new(0));
    let z = zaehler.clone();
    tokio::spawn(async move {
        while let Ok((verbindung, _)) = listener.accept().await {
            z.fetch_add(1, Ordering::SeqCst);
            drop(verbindung);
        }
    });
    (port, zaehler)
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
async fn praeparierte_mail_ohne_skripte_und_ohne_netzwerkzugriff() {
    let (port, verbindungen) = lauscher().await;
    let ziel = format!("127.0.0.1:{port}");
    let adresse = neue_adresse();
    let roh = format!(
        "From: Angreifer <boese@example.org>\r\nTo: {adresse}\r\nSubject: Rechnung\r\nMIME-Version: 1.0\r\n\
Content-Type: text/html; charset=utf-8\r\n\r\n\
<html><head><script src=\"http://{ziel}/a.js\"></script><link rel=stylesheet href=\"http://{ziel}/a.css\">\
<style>@import url(http://{ziel}/b.css);</style></head>\
<body onload=\"fetch('http://{ziel}/onload')\">\
<p>Ihre Rechnung</p>\
<script>new Image().src='http://{ziel}/skript'</script>\
<img src=\"http://{ziel}/pixel.gif\" width=1 height=1>\
<img src=\"https://{ziel}/pixel2.gif\">\
<img src=\"x\" onerror=\"this.src='http://{ziel}/onerror'\">\
<div style=\"background:url(http://{ziel}/bg.png)\">Hintergrund</div>\
<iframe src=\"http://{ziel}/frame\"></iframe>\
<a href=\"http://{ziel}/phish\">https://meine-bank.ch</a>\
</body></html>\r\n"
    );
    roh_zustellen(&adresse, &roh).await;

    let (_ordner, konten, konto) = konto_mit_imap_port(&adresse, IMAP_TLS).await;
    let dienst = ImapDienst::neu(konten, Arc::new(SpeicherCache::default()));
    let seite = dienst.mails_seite(&konto.id, "INBOX", 0, 10).await.unwrap();
    let inhalt = dienst
        .inhalt(&konto.id, "INBOX", seite.mails[0].uid)
        .await
        .unwrap();

    let mail = anzeige::aufbereiten(&inhalt, None).unwrap();
    let html = mail.html.unwrap().to_ascii_lowercase();
    for verboten in [
        "<script", "onload", "onerror", "<iframe", "<link", "<style", "url(", "/pixel", "/skript",
    ] {
        assert!(
            !html.contains(verboten),
            "„{verboten}“ steht noch drin: {html}"
        );
    }
    assert_eq!(
        mail.extern_blockiert, 2,
        "beide Tracking-Bilder werden gezählt"
    );
    // Der Link bleibt sichtbar (die Oberfläche warnt wegen des abweichenden Textes).
    assert!(html.contains(&format!("href=\"http://{ziel}/phish\"")));

    // Kurz warten: Es darf keine einzige Verbindung zum „Tracking-Server“ gegeben haben.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(verbindungen.load(Ordering::SeqCst), 0);
}

/// Absenderprüfung (#45): Gezählt wird nur der oberste `Authentication-Results`-Header mit der
/// Kennung des eigenen Servers (Standard: IMAP-Hostname, hier `localhost`).
#[tokio::test]
#[ignore = "braucht den Test-Mailserver (scripts/test-mailserver.sh start)"]
async fn absenderpruefung_vertraut_nur_dem_eigenen_server() {
    use sichermail_lib::anzeige::absender::Stufe;

    let adresse = neue_adresse();
    let mails = [
        // Eigener Server bestätigt.
        ("Authentication-Results: localhost; dkim=pass header.d=example.org; dmarc=pass header.from=example.org\r\n", "news@example.org"),
        // Eigener Server meldet DMARC-Fehler; der eingeschleuste Header darunter ändert nichts.
        ("Authentication-Results: localhost; dmarc=fail (p=reject) header.from=bank.example\r\nAuthentication-Results: localhost; dmarc=pass header.from=bank.example\r\n", "konto@bank.example"),
        // Nur ein eingeschleuster Header mit fremder Kennung.
        ("Authentication-Results: mx.angreifer.example; dkim=pass header.d=bank.example; dmarc=pass header.from=bank.example\r\n", "konto@bank.example"),
    ];
    for (i, (kopf, von)) in mails.iter().enumerate() {
        let roh = format!(
            "{kopf}From: Absender <{von}>\r\nTo: {adresse}\r\nSubject: Mail {i}\r\nMessage-ID: <pruefung-{i}@example.org>\r\n\r\nHallo\r\n"
        );
        roh_zustellen(&adresse, &roh).await;
    }

    let (_ordner, konten, konto) = konto_mit_imap_port(&adresse, IMAP_TLS).await;
    assert_eq!(konto.absender_kennung(), "localhost");
    let dienst = ImapDienst::neu(konten, Arc::new(SpeicherCache::default()));
    let seite = dienst.mails_seite(&konto.id, "INBOX", 0, 10).await.unwrap();
    assert_eq!(seite.mails.len(), 3);
    let mut stufen = Vec::new();
    for kopf in &seite.mails {
        let inhalt = dienst.inhalt(&konto.id, "INBOX", kopf.uid).await.unwrap();
        let pruefung = anzeige::absender_pruefen(&inhalt, konto.absender_kennung()).unwrap();
        stufen.push((kopf.betreff.clone(), pruefung.stufe));
    }
    stufen.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        stufen,
        vec![
            ("Mail 0".to_string(), Stufe::Bestaetigt),
            ("Mail 1".to_string(), Stufe::Warnung),
            ("Mail 2".to_string(), Stufe::NichtGeprueft),
        ]
    );
}
