//! Gemeinsame Hilfen für die Integrationstests gegen den lokalen Test-Mailserver.
//!
//! Der Server wird mit `scripts/test-mailserver.sh start` gestartet. Die Tests sind
//! mit `#[ignore]` markiert und laufen mit `cargo test -- --include-ignored`.
#![allow(dead_code)]

use std::path::PathBuf;

use sichermail_lib::konto::{KontoEingabe, ServerEinstellung, Verschluesselung};
use zeroize::Zeroizing;

pub const HOST: &str = "localhost";
pub const IMAP_TLS: u16 = 3993;
pub const IMAP_KLARTEXT: u16 = 3143;
pub const SMTP_TLS: u16 = 3465;
pub const SMTP_KLARTEXT: u16 = 3025;
pub const BENUTZER: &str = "test";
pub const PASSWORT: &str = "testpasswort";
pub const ADRESSE: &str = "test@localhost";

/// Zertifikat des Test-Mailservers (vom Startskript erzeugt).
pub fn zertifikat() -> String {
    let pfad = std::env::var("SICHERMAIL_TESTSERVER_ZERTIFIKAT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.testserver/cert.pem")
        });
    std::fs::read_to_string(&pfad).unwrap_or_else(|_| {
        panic!(
            "Zertifikat {} fehlt – zuerst scripts/test-mailserver.sh start ausführen",
            pfad.display()
        )
    })
}

pub fn server(
    port: u16,
    verschluesselung: Verschluesselung,
    mit_zertifikat: bool,
) -> ServerEinstellung {
    ServerEinstellung {
        host: HOST.into(),
        port,
        verschluesselung,
        zertifikat: mit_zertifikat.then(zertifikat),
    }
}

/// Eingabe für ein Konto beim Test-Mailserver mit hinterlegtem Zertifikat.
pub fn eingabe(benutzer: &str, passwort: &str) -> KontoEingabe {
    KontoEingabe {
        id: None,
        name: "Test Nutzer".into(),
        email: format!("{benutzer}@localhost.test"),
        benutzername: benutzer.into(),
        passwort: Some(Zeroizing::new(passwort.into())),
        imap: server(IMAP_TLS, Verschluesselung::Tls, true),
        smtp: server(SMTP_TLS, Verschluesselung::Tls, true),
    }
}

/// Neue, eindeutige Adresse beim Test-Mailserver. GreenMail legt das Postfach beim
/// ersten Empfang an; Benutzername und Passwort sind dann die Adresse selbst.
pub fn neue_adresse() -> String {
    format!(
        "t{}@localhost",
        &uuid::Uuid::new_v4().simple().to_string()[..12]
    )
}

/// Stellt Mails über SMTP (TLS, hinterlegtes Zertifikat, Anmeldung als „test“) zu.
pub async fn zustellen(an: &str, betreffe: &[&str]) {
    use lettre::message::Message;
    let mut verbindung = sichermail_lib::smtp::anmelden(
        &server(SMTP_TLS, Verschluesselung::Tls, true),
        BENUTZER,
        PASSWORT,
    )
    .await
    .expect("Anmeldung beim Test-SMTP-Server");
    for betreff in betreffe {
        let mail = Message::builder()
            .from("Test Absender <test@localhost>".parse().unwrap())
            .to(an.parse().unwrap())
            .subject(*betreff)
            .body(format!("Inhalt von {betreff}"))
            .unwrap();
        verbindung
            .send(mail.envelope(), &mail.formatted())
            .await
            .expect("Mail zustellen");
    }
    let _ = verbindung.quit().await;
}

/// Eigene IMAP-Sitzung direkt zum Test-Mailserver (für Prüfungen „von aussen“).
pub async fn direkte_sitzung(adresse: &str) -> sichermail_lib::imap::ImapSitzung {
    sichermail_lib::imap::anmelden(
        &server(IMAP_TLS, Verschluesselung::Tls, true),
        adresse,
        adresse,
    )
    .await
    .expect("direkte IMAP-Anmeldung")
}

/// Kontoverwaltung mit einem Konto für `adresse` (Postfach muss schon existieren).
pub async fn konto_mit_imap_port(
    adresse: &str,
    imap_port: u16,
) -> (
    tempfile::TempDir,
    std::sync::Arc<sichermail_lib::konto::KontoVerwaltung>,
    sichermail_lib::konto::Konto,
) {
    let ordner = tempfile::tempdir().unwrap();
    let verwaltung = std::sync::Arc::new(sichermail_lib::konto::KontoVerwaltung::neu(
        ordner.path(),
        std::sync::Arc::new(
            sichermail_lib::konto::schluesselbund::SpeicherSchluesselbund::default(),
        ),
    ));
    let mut e = eingabe(adresse, adresse);
    e.imap.port = imap_port;
    let konto = verwaltung.speichern(&e).await.expect("Konto anlegen");
    (ordner, verwaltung, konto)
}

/// TCP-Weiterleitung, die sich hart unterbrechen lässt (simuliert Verbindungsabbrüche).
pub struct Weiterleitung {
    pub port: u16,
    offen: std::sync::Arc<std::sync::atomic::AtomicBool>,
    trennen: tokio::sync::watch::Sender<u64>,
}

impl Weiterleitung {
    pub async fn starten(ziel_port: u16) -> Self {
        use std::sync::atomic::Ordering;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let offen = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let (trennen, _) = tokio::sync::watch::channel(0u64);
        let offen_kopie = offen.clone();
        let trennen_kopie = trennen.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut eingehend, _)) = listener.accept().await else {
                    return;
                };
                if !offen_kopie.load(Ordering::SeqCst) {
                    drop(eingehend);
                    continue;
                }
                let mut signal = trennen_kopie.subscribe();
                tokio::spawn(async move {
                    let Ok(mut ausgehend) =
                        tokio::net::TcpStream::connect(("127.0.0.1", ziel_port)).await
                    else {
                        return;
                    };
                    tokio::select! {
                        _ = tokio::io::copy_bidirectional(&mut eingehend, &mut ausgehend) => {}
                        _ = signal.changed() => {}
                    }
                });
            }
        });
        Self {
            port,
            offen,
            trennen,
        }
    }

    /// Trennt alle Verbindungen und nimmt keine neuen mehr an.
    pub fn unterbrechen(&self) {
        self.offen.store(false, std::sync::atomic::Ordering::SeqCst);
        self.trennen.send_modify(|n| *n += 1);
    }

    /// Nimmt wieder Verbindungen an.
    pub fn wiederherstellen(&self) {
        self.offen.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}
