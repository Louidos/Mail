//! Tests für STARTTLS mit einem kleinen Test-Server im Test selbst.
//!
//! GreenMail bietet kein STARTTLS an; hier wird deshalb ein minimaler IMAP- bzw.
//! SMTP-Server nachgebildet, der STARTTLS kann. Geprüft wird:
//! - der erfolgreiche Weg (STARTTLS, danach Anmeldung über TLS),
//! - dass vor TLS eingeschleuste Daten nicht in die verschlüsselte Sitzung gelangen.

use std::sync::Arc;

use base64::Engine as _;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use sichermail_lib::fehler::VerbindungsFehler;
use sichermail_lib::konto::{ServerEinstellung, Verschluesselung};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;

struct TestServer {
    port: u16,
    zertifikat_pem: String,
    listener: TcpListener,
    acceptor: TlsAcceptor,
}

async fn test_server() -> TestServer {
    let schluessel = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    let zertifikat: CertificateDer<'static> = schluessel.cert.der().clone();
    let privat = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
        schluessel.signing_key.serialize_der(),
    ));
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(vec![zertifikat], privat)
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    TestServer {
        port: listener.local_addr().unwrap().port(),
        zertifikat_pem: schluessel.cert.pem(),
        listener,
        acceptor: TlsAcceptor::from(Arc::new(config)),
    }
}

fn einstellung(server: &TestServer) -> ServerEinstellung {
    ServerEinstellung {
        host: "localhost".into(),
        port: server.port,
        verschluesselung: Verschluesselung::Starttls,
        zertifikat: Some(server.zertifikat_pem.clone()),
    }
}

/// Liest eine Zeile vom Klartext-Teil.
async fn zeile(leser: &mut BufReader<TcpStream>) -> String {
    let mut zeile = String::new();
    leser.read_line(&mut zeile).await.unwrap();
    zeile
}

/// Minimaler IMAP-Server mit STARTTLS. `einschleusen` sendet nach „a2 OK“ zusätzliche
/// Klartext-Bytes, wie es ein Angreifer im Netz tun könnte.
async fn imap_server(server: TestServer, einschleusen: bool) {
    let (tcp, _) = server.listener.accept().await.unwrap();
    let mut leser = BufReader::new(tcp);
    leser
        .get_mut()
        .write_all(b"* OK Test-IMAP bereit\r\n")
        .await
        .unwrap();
    assert!(zeile(&mut leser).await.starts_with("a1 CAPABILITY"));
    leser
        .get_mut()
        .write_all(b"* CAPABILITY IMAP4rev1 STARTTLS LOGINDISABLED\r\na1 OK fertig\r\n")
        .await
        .unwrap();
    assert!(zeile(&mut leser).await.starts_with("a2 STARTTLS"));
    if einschleusen {
        leser
            .get_mut()
            .write_all(
                b"a2 OK TLS starten\r\n* OK [CAPABILITY IMAP4rev1 AUTH=PLAIN] eingeschleust\r\n",
            )
            .await
            .unwrap();
        return;
    }
    leser
        .get_mut()
        .write_all(b"a2 OK TLS starten\r\n")
        .await
        .unwrap();

    let tls = server.acceptor.accept(leser.into_inner()).await.unwrap();
    let mut tls = BufReader::new(tls);
    loop {
        let mut zeile = String::new();
        if tls.read_line(&mut zeile).await.unwrap_or(0) == 0 {
            return;
        }
        let tag = zeile.split_whitespace().next().unwrap_or("*").to_string();
        let befehl = zeile
            .split_whitespace()
            .nth(1)
            .unwrap_or("")
            .to_ascii_uppercase();
        let antwort = match befehl.as_str() {
            "LOGIN" if zeile.contains("\"benutzer\" \"passwort\"") => {
                format!("{tag} OK LOGIN fertig\r\n")
            }
            "LOGIN" => format!("{tag} NO falsche Anmeldung\r\n"),
            "LOGOUT" => format!("* BYE tschüss\r\n{tag} OK fertig\r\n"),
            _ => format!("{tag} BAD unbekannt\r\n"),
        };
        tls.get_mut().write_all(antwort.as_bytes()).await.unwrap();
    }
}

/// Minimaler SMTP-Server mit STARTTLS.
async fn smtp_server(server: TestServer, mit_starttls: bool) {
    let (tcp, _) = server.listener.accept().await.unwrap();
    let mut leser = BufReader::new(tcp);
    leser
        .get_mut()
        .write_all(b"220 test ESMTP\r\n")
        .await
        .unwrap();
    assert!(zeile(&mut leser).await.starts_with("EHLO"));
    if !mit_starttls {
        leser
            .get_mut()
            .write_all(b"250-test\r\n250 AUTH PLAIN LOGIN\r\n")
            .await
            .unwrap();
        // Der Client muss jetzt abbrechen und darf nichts mehr senden.
        let mut rest = String::new();
        let gelesen = leser.read_line(&mut rest).await.unwrap_or(0);
        assert_eq!(gelesen, 0, "Client hat ohne TLS weitergemacht: {rest}");
        return;
    }
    leser
        .get_mut()
        .write_all(b"250-test\r\n250-STARTTLS\r\n250 8BITMIME\r\n")
        .await
        .unwrap();
    assert!(zeile(&mut leser).await.starts_with("STARTTLS"));
    leser.get_mut().write_all(b"220 los\r\n").await.unwrap();

    let tls = server.acceptor.accept(leser.into_inner()).await.unwrap();
    let mut tls = BufReader::new(tls);
    let erwartet = format!(
        "AUTH PLAIN {}",
        base64::engine::general_purpose::STANDARD.encode("\0benutzer\0passwort")
    );
    loop {
        let mut zeile = String::new();
        if tls.read_line(&mut zeile).await.unwrap_or(0) == 0 {
            return;
        }
        let antwort = if zeile.starts_with("EHLO") {
            "250-test\r\n250 AUTH PLAIN LOGIN\r\n".to_string()
        } else if zeile.trim_end() == erwartet {
            "235 angemeldet\r\n".to_string()
        } else if zeile.starts_with("AUTH") {
            "535 falsche Anmeldung\r\n".to_string()
        } else if zeile.starts_with("QUIT") {
            "221 tschüss\r\n".to_string()
        } else {
            "500 unbekannt\r\n".to_string()
        };
        tls.get_mut().write_all(antwort.as_bytes()).await.unwrap();
    }
}

#[tokio::test]
async fn imap_starttls_mit_anmeldung() {
    let server = test_server().await;
    let einstellung = einstellung(&server);
    let task = tokio::spawn(imap_server(server, false));
    let mut sitzung = sichermail_lib::imap::anmelden(&einstellung, "benutzer", "passwort")
        .await
        .unwrap();
    sitzung.logout().await.unwrap();
    drop(sitzung);
    task.await.unwrap();
}

#[tokio::test]
async fn imap_starttls_eingeschleuste_daten_brechen_ab() {
    let server = test_server().await;
    let einstellung = einstellung(&server);
    let task = tokio::spawn(imap_server(server, true));
    let fehler = sichermail_lib::imap::anmelden(&einstellung, "benutzer", "passwort")
        .await
        .unwrap_err();
    assert!(
        matches!(fehler, VerbindungsFehler::Protokoll(_)),
        "{fehler:?}"
    );
    task.await.unwrap();
}

#[tokio::test]
async fn smtp_starttls_mit_anmeldung() {
    let server = test_server().await;
    let einstellung = einstellung(&server);
    let task = tokio::spawn(smtp_server(server, true));
    let mut verbindung = sichermail_lib::smtp::anmelden(&einstellung, "benutzer", "passwort")
        .await
        .unwrap();
    verbindung.quit().await.unwrap();
    drop(verbindung);
    task.await.unwrap();
}

#[tokio::test]
async fn smtp_falsches_passwort_ueber_starttls() {
    let server = test_server().await;
    let einstellung = einstellung(&server);
    tokio::spawn(smtp_server(server, true));
    let Err(fehler) = sichermail_lib::smtp::anmelden(&einstellung, "benutzer", "falsch").await
    else {
        panic!("Anmeldung mit falschem Passwort darf nicht gelingen");
    };
    assert_eq!(fehler, VerbindungsFehler::AnmeldungFalsch);
}

#[tokio::test]
async fn smtp_ohne_starttls_bricht_ab() {
    let server = test_server().await;
    let einstellung = einstellung(&server);
    let task = tokio::spawn(smtp_server(server, false));
    let Err(fehler) = sichermail_lib::smtp::anmelden(&einstellung, "benutzer", "passwort").await
    else {
        panic!("Verbindung ohne STARTTLS darf nicht gelingen");
    };
    assert_eq!(fehler, VerbindungsFehler::KeinStartTls);
    task.await.unwrap();
}
