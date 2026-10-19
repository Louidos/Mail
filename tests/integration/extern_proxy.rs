//! Integrationstest: Externe Bilder und Proxys (#24, #46).
//!
//! - Ohne Einstellung nie über einen im System eingestellten Proxy (`HTTPS_PROXY` …).
//! - Mit Einstellung nur über die eingetragene Adresse; Ziele im eigenen Netz werden auch dann
//!   abgelehnt, bevor sie den Proxy erreichen.
//!
//! Eigene Testdatei und damit eigener Prozess, weil hier Umgebungsvariablen gesetzt werden,
//! die sonst andere Tests beeinflussen würden.

use std::sync::{Arc, Mutex};

use sichermail_lib::anzeige::extern_inhalte::{self, ProxyAdresse};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Lokaler „Proxy“: merkt sich die erste Zeile jeder Anfrage und antwortet mit 502.
async fn proxy() -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let anfragen = Arc::new(Mutex::new(Vec::new()));
    let a = anfragen.clone();
    tokio::spawn(async move {
        while let Ok((mut verbindung, _)) = listener.accept().await {
            let a = a.clone();
            tokio::spawn(async move {
                let mut puffer = vec![0u8; 4096];
                let mut gelesen = 0;
                while gelesen < puffer.len() {
                    match verbindung.read(&mut puffer[gelesen..]).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => gelesen += n,
                    }
                    if puffer[..gelesen].windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                let text = String::from_utf8_lossy(&puffer[..gelesen]);
                a.lock()
                    .unwrap()
                    .push(text.lines().next().unwrap_or("").to_string());
                let _ = verbindung
                    .write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n")
                    .await;
            });
        }
    });
    (port, anfragen)
}

fn anfragen(liste: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
    liste.lock().unwrap().clone()
}

/// Setzt alle üblichen Proxy-Umgebungsvariablen auf `adresse`.
fn system_proxy_setzen(adresse: &str) {
    for name in [
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
        "ALL_PROXY",
        "all_proxy",
    ] {
        std::env::set_var(name, adresse);
    }
    for name in ["NO_PROXY", "no_proxy"] {
        std::env::remove_var(name);
    }
}

#[tokio::test]
async fn ohne_einstellung_nie_ueber_system_proxy() {
    let (port, system) = proxy().await;
    system_proxy_setzen(&format!("http://127.0.0.1:{port}"));

    // Öffentliche IP-Adresse, damit keine Namensauflösung nötig ist. Ob das Bild geladen werden
    // kann, spielt keine Rolle (ohne Internet bleibt das Ergebnis leer) – nur der Proxy zählt.
    extern_inhalte::bilder_laden(&["https://93.184.215.14/bild.png".to_string()], None).await;

    assert!(
        anfragen(&system).is_empty(),
        "externe Bilder dürfen ohne Einstellung nie über den System-Proxy geladen werden"
    );
}

#[tokio::test]
async fn mit_einstellung_nur_ueber_eingetragenen_proxy() {
    let (system_port, system) = proxy().await;
    system_proxy_setzen(&format!("http://127.0.0.1:{system_port}"));
    let (port, eigener) = proxy().await;
    let adresse = ProxyAdresse::pruefen(&format!("http://127.0.0.1:{port}")).unwrap();

    let ergebnis = extern_inhalte::bilder_laden(
        &[
            "https://93.184.215.14/bild.png".to_string(),
            "https://bilder.example.org/logo.png".to_string(),
        ],
        Some(&adresse),
    )
    .await;

    // Der Proxy antwortet mit 502, also wird nichts geladen.
    assert!(ergebnis.is_empty());
    let mut gesehen = anfragen(&eigener);
    gesehen.sort();
    assert_eq!(
        gesehen,
        vec![
            "CONNECT 93.184.215.14:443 HTTP/1.1",
            // Den Namen löst der Proxy auf, nicht SicherMail.
            "CONNECT bilder.example.org:443 HTTP/1.1",
        ]
    );
    assert!(
        anfragen(&system).is_empty(),
        "auch mit Einstellung nie über den System-Proxy"
    );
}

#[tokio::test]
async fn ziele_im_eigenen_netz_gehen_nie_an_den_proxy() {
    let (port, eigener) = proxy().await;
    let adresse = ProxyAdresse::pruefen(&format!("http://127.0.0.1:{port}")).unwrap();

    let ergebnis = extern_inhalte::bilder_laden(
        &[
            "https://127.0.0.1/a.png",
            "https://10.0.0.1/a.png",
            "https://172.16.5.4/a.png",
            "https://192.168.1.1/a.png",
            "https://169.254.169.254/latest",
            "https://[::1]/a.png",
            "https://[fd00::1]/a.png",
            "https://[fe80::1]/a.png",
            "https://localhost/a.png",
            "https://drucker.localhost/a.png",
            "https://router/a.png",
            "http://bilder.example.org/a.png",
        ]
        .map(String::from),
        Some(&adresse),
    )
    .await;

    assert!(ergebnis.is_empty());
    assert!(
        anfragen(&eigener).is_empty(),
        "Adressen im eigenen Netz dürfen nicht an den Proxy gehen: {:?}",
        anfragen(&eigener)
    );
}
