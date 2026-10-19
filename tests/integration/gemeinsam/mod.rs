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
