//! Integrationstest: Externe Bilder werden nie über einen im System eingestellten Proxy geladen (#24).
//!
//! Eigene Testdatei und damit eigener Prozess, weil hier Umgebungsvariablen gesetzt werden,
//! die sonst andere Tests beeinflussen würden.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use sichermail_lib::anzeige::extern_inhalte;

/// Zählt alle eingehenden Verbindungen auf einem lokalen Port („Proxy“).
async fn proxy() -> (u16, Arc<AtomicUsize>) {
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

#[tokio::test]
async fn externe_bilder_nie_ueber_system_proxy() {
    let (port, zaehler) = proxy().await;
    let adresse = format!("http://127.0.0.1:{port}");
    for name in ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"] {
        std::env::set_var(name, &adresse);
    }
    for name in ["NO_PROXY", "no_proxy"] {
        std::env::remove_var(name);
    }

    // Öffentliche IP-Adresse, damit keine Namensauflösung nötig ist. Ob das Bild geladen werden
    // kann, spielt keine Rolle (ohne Internet bleibt das Ergebnis leer) – nur der Proxy zählt.
    extern_inhalte::bilder_laden(&["https://93.184.215.14/bild.png".to_string()]).await;

    assert_eq!(
        zaehler.load(Ordering::SeqCst),
        0,
        "externe Bilder dürfen nie über den System-Proxy geladen werden"
    );
}
