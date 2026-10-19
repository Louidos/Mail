//! Externe Bilder auf ausdrücklichen Wunsch des Nutzers laden.
//!
//! - Nur HTTPS (mindestens TLS 1.2, Zertifikate geprüft), höchstens 3 Weiterleitungen, nur auf HTTPS.
//! - Nur Bilder (PNG, JPEG, GIF, WebP), höchstens 5 MB pro Bild und 40 Bilder pro Mail.
//! - Keine Cookies, kein Referer.
//!
//! Die Bilder werden als `data:`-Adressen in die bereinigte Mail eingesetzt, damit die
//! Anzeige selbst nie Verbindungen aufbaut.

use std::collections::HashMap;
use std::time::Duration;

use base64::Engine as _;

const MAX_BILDER: usize = 40;
const MAX_GROESSE: usize = 5 * 1024 * 1024;
const ERLAUBTE_TYPEN: [&str; 4] = ["image/png", "image/jpeg", "image/gif", "image/webp"];

/// Lädt die Bilder und gibt Adresse → `data:`-Adresse zurück. Nicht ladbare fehlen.
pub async fn bilder_laden(adressen: &[String]) -> HashMap<String, String> {
    let mut ergebnis = HashMap::new();
    let Ok(client) = reqwest::Client::builder()
        .https_only(true)
        .min_tls_version(reqwest::tls::Version::TLS_1_2)
        .redirect(reqwest::redirect::Policy::limited(3))
        .timeout(Duration::from_secs(15))
        .referer(false)
        .build()
    else {
        return ergebnis;
    };

    let mut eindeutig: Vec<&String> = Vec::new();
    for adresse in adressen {
        if adresse.to_ascii_lowercase().starts_with("https://") && !eindeutig.contains(&adresse) {
            eindeutig.push(adresse);
        }
    }
    let auftraege = eindeutig.into_iter().take(MAX_BILDER).map(|adresse| {
        let client = client.clone();
        async move { (adresse.clone(), bild_laden(&client, adresse).await) }
    });
    for (adresse, daten) in futures_util::future::join_all(auftraege).await {
        if let Some(daten) = daten {
            ergebnis.insert(adresse, daten);
        }
    }
    ergebnis
}

async fn bild_laden(client: &reqwest::Client, adresse: &str) -> Option<String> {
    let mut antwort = client.get(adresse).send().await.ok()?;
    if !antwort.status().is_success() {
        return None;
    }
    let typ = antwort
        .headers()
        .get(reqwest::header::CONTENT_TYPE)?
        .to_str()
        .ok()?
        .split(';')
        .next()?
        .trim()
        .to_ascii_lowercase();
    if !ERLAUBTE_TYPEN.contains(&typ.as_str()) {
        return None;
    }
    let mut daten = Vec::new();
    while let Some(teil) = antwort.chunk().await.ok()? {
        daten.extend_from_slice(&teil);
        if daten.len() > MAX_GROESSE {
            return None;
        }
    }
    Some(format!(
        "data:{typ};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&daten)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn http_wird_nie_geladen() {
        let ergebnis = bilder_laden(&["http://127.0.0.1:9/bild.png".to_string()]).await;
        assert!(ergebnis.is_empty());
    }
}
