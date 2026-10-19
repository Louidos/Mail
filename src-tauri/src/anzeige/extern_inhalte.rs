//! Externe Bilder auf ausdrücklichen Wunsch des Nutzers laden.
//!
//! - Nur HTTPS (mindestens TLS 1.2, Zertifikate geprüft), höchstens 3 Weiterleitungen, nur auf HTTPS.
//! - Nur Bilder (PNG, JPEG, GIF, WebP), höchstens 5 MB pro Bild und 40 Bilder pro Mail.
//! - Keine Cookies, kein Referer.
//! - Keine Adressen im eigenen Netz (localhost, private, Link-Local …), auch nicht über
//!   Weiterleitungen oder DNS: Eine Mail soll so keine Geräte im Heimnetz ansprechen können.
//!
//! Die Bilder werden als `data:`-Adressen in die bereinigte Mail eingesetzt, damit die
//! Anzeige selbst nie Verbindungen aufbaut.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
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
        .redirect(reqwest::redirect::Policy::custom(|versuch| {
            if versuch.previous().len() >= 3 {
                versuch.error("zu viele Weiterleitungen")
            } else if !adresse_erlaubt(versuch.url()) {
                versuch.stop()
            } else {
                versuch.follow()
            }
        }))
        .dns_resolver(Arc::new(NurOeffentlich))
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

/// Löst Namen auf und lässt nur öffentliche Adressen zu.
struct NurOeffentlich;

impl reqwest::dns::Resolve for NurOeffentlich {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        Box::pin(async move {
            let adressen: Vec<SocketAddr> = tokio::net::lookup_host((name.as_str(), 0))
                .await?
                .filter(|a| oeffentlich(a.ip()))
                .collect();
            if adressen.is_empty() {
                return Err("keine öffentliche Adresse".into());
            }
            let adressen: reqwest::dns::Addrs = Box::new(adressen.into_iter());
            Ok(adressen)
        })
    }
}

/// Darf diese Adresse geladen werden? (Nur HTTPS, kein lokaler Name, keine interne IP.)
fn adresse_erlaubt(url: &reqwest::Url) -> bool {
    if url.scheme() != "https" {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = host.parse::<IpAddr>() {
        return oeffentlich(ip);
    }
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    host.contains('.') && host != "localhost" && !host.ends_with(".localhost")
}

/// Öffentliche Internetadresse (nicht lokal, privat, Link-Local, Multicast …)?
fn oeffentlich(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => oeffentlich_v4(v4),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => oeffentlich_v4(v4),
            None => oeffentlich_v6(v6),
        },
    }
}

fn oeffentlich_v4(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_multicast()
        || a == 0
        || a >= 240
        || (a == 100 && (64..128).contains(&b))
        || (a == 198 && (b == 18 || b == 19))
        || (a == 192 && b == 0 && ip.octets()[2] == 0))
}

fn oeffentlich_v6(ip: Ipv6Addr) -> bool {
    let erstes = ip.segments()[0];
    !(ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || (erstes & 0xfe00) == 0xfc00
        || (erstes & 0xffc0) == 0xfe80
        || (erstes == 0x2001 && ip.segments()[1] == 0x0db8)
        || ip.to_ipv4().is_some_and(|v4| !oeffentlich_v4(v4)))
}

async fn bild_laden(client: &reqwest::Client, adresse: &str) -> Option<String> {
    let url = reqwest::Url::parse(adresse).ok()?;
    if !adresse_erlaubt(&url) {
        return None;
    }
    let mut antwort = client.get(url).send().await.ok()?;
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

    #[test]
    fn keine_adressen_im_eigenen_netz() {
        let erlaubt = |a: &str| adresse_erlaubt(&reqwest::Url::parse(a).unwrap());
        assert!(erlaubt("https://bilder.example.org/a.png"));
        assert!(erlaubt("https://93.184.215.14/a.png"));
        for gesperrt in [
            "http://bilder.example.org/a.png",
            "https://localhost/a.png",
            "https://drucker.localhost/a.png",
            "https://router/a.png",
            "https://127.0.0.1/a.png",
            "https://192.168.1.1/a.png",
            "https://10.0.0.1/a.png",
            "https://172.16.5.4/a.png",
            "https://169.254.169.254/latest",
            "https://100.64.0.1/a.png",
            "https://0.0.0.0/a.png",
            "https://[::1]/a.png",
            "https://[fe80::1]/a.png",
            "https://[fd00::1]/a.png",
            "https://[::ffff:192.168.1.1]/a.png",
        ] {
            assert!(!erlaubt(gesperrt), "{gesperrt} müsste gesperrt sein");
        }
    }

    #[tokio::test]
    async fn namen_mit_interner_adresse_werden_nicht_aufgeloest() {
        use reqwest::dns::Resolve;
        let name: reqwest::dns::Name = "localhost".parse().unwrap();
        assert!(NurOeffentlich.resolve(name).await.is_err());
    }
}
