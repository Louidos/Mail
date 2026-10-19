//! Automatisches Ausfüllen der Servereinstellungen über Autoconfig.
//!
//! Abgefragt wird nur die Domain der E-Mail-Adresse selbst, ausschliesslich über HTTPS:
//! - `https://autoconfig.<domain>/mail/config-v1.1.xml`
//! - `https://<domain>/.well-known/autoconfig/mail/config-v1.1.xml`
//!
//! Es werden keine Fremddienste (z. B. zentrale Anbieter-Datenbanken) gefragt.
//! Unverschlüsselte Servereinträge werden ignoriert. Das Ergebnis ist nur ein Vorschlag.

use std::time::Duration;

use serde::Serialize;

use super::{ServerEinstellung, Verschluesselung};
use crate::fehler::Fehlermeldung;

/// Maximale Grösse der Autoconfig-Datei.
const MAX_GROESSE: usize = 256 * 1024;

/// Vorschlag für die Servereinstellungen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Vorschlag {
    pub benutzername: String,
    pub imap: ServerEinstellung,
    pub smtp: ServerEinstellung,
}

/// Sucht Servereinstellungen für eine E-Mail-Adresse.
///
/// `Ok(None)` heisst: nichts gefunden, der Nutzer trägt die Server selbst ein.
pub async fn suchen(email: &str) -> Result<Option<Vorschlag>, Fehlermeldung> {
    let email = email.trim();
    let domain = match email.rsplit_once('@') {
        Some((lokal, domain)) if !lokal.is_empty() && gueltige_domain(domain) => {
            domain.to_ascii_lowercase()
        }
        _ => {
            return Err(Fehlermeldung::neu(
                "eingabe",
                "Bitte zuerst eine gültige E-Mail-Adresse eingeben.",
            ))
        }
    };

    let client = reqwest::Client::builder()
        .https_only(true)
        .min_tls_version(reqwest::tls::Version::TLS_1_2)
        .redirect(nur_innerhalb(domain.clone()))
        .timeout(Duration::from_secs(10))
        .user_agent("SicherMail")
        .build()
        .map_err(|_| Fehlermeldung::neu("autoconfig", "Autoconfig ist nicht verfügbar."))?;

    let adressen = [
        format!("https://autoconfig.{domain}/mail/config-v1.1.xml"),
        format!("https://{domain}/.well-known/autoconfig/mail/config-v1.1.xml"),
    ];
    for adresse in adressen {
        if let Some(xml) = laden(&client, &adresse).await {
            if let Some(vorschlag) = auswerten(&xml, email) {
                return Ok(Some(vorschlag));
            }
        }
    }
    Ok(None)
}

/// Weiterleitungen nur innerhalb der Domain der Adresse (höchstens 3), damit die Suche
/// keine fremden Server anspricht.
fn nur_innerhalb(domain: String) -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(move |versuch| {
        let erlaubt = versuch
            .url()
            .host_str()
            .is_some_and(|host| gleiche_domain(host, &domain));
        if versuch.previous().len() >= 3 || !erlaubt {
            versuch.stop()
        } else {
            versuch.follow()
        }
    })
}

fn gleiche_domain(host: &str, domain: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    host == domain || host.ends_with(&format!(".{domain}"))
}

fn gueltige_domain(domain: &str) -> bool {
    !domain.is_empty()
        && domain.len() <= 253
        && domain.contains('.')
        && domain
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
        && !domain.starts_with('.')
        && !domain.starts_with('-')
}

async fn laden(client: &reqwest::Client, adresse: &str) -> Option<String> {
    let mut antwort = client.get(adresse).send().await.ok()?;
    if !antwort.status().is_success() {
        return None;
    }
    let mut daten = Vec::new();
    while let Some(teil) = antwort.chunk().await.ok()? {
        daten.extend_from_slice(&teil);
        if daten.len() > MAX_GROESSE {
            return None;
        }
    }
    String::from_utf8(daten).ok()
}

/// Wertet eine Autoconfig-Datei (config-v1.1.xml) aus.
pub fn auswerten(xml: &str, email: &str) -> Option<Vorschlag> {
    let dokument = roxmltree::Document::parse(xml).ok()?;
    let anbieter = dokument
        .descendants()
        .find(|k| k.has_tag_name("emailProvider"))?;

    let (lokal, domain) = email.rsplit_once('@')?;
    let ersetzen = |text: &str| {
        text.replace("%EMAILADDRESS%", email)
            .replace("%EMAILLOCALPART%", lokal)
            .replace("%EMAILDOMAIN%", domain)
    };

    let server_lesen = |knoten: roxmltree::Node| -> Option<(ServerEinstellung, String)> {
        let text = |name: &str| {
            knoten
                .children()
                .find(|k| k.has_tag_name(name))
                .and_then(|k| k.text())
                .map(|t| t.trim().to_string())
        };
        let verschluesselung = match text("socketType")?.to_ascii_uppercase().as_str() {
            "SSL" | "TLS" => Verschluesselung::Tls,
            "STARTTLS" => Verschluesselung::Starttls,
            // Unverschlüsselte Einträge werden nie vorgeschlagen.
            _ => return None,
        };
        let host = ersetzen(&text("hostname")?);
        if !gueltige_domain(&host) {
            return None;
        }
        let port = text("port")?.parse::<u16>().ok().filter(|p| *p != 0)?;
        let benutzername = ersetzen(&text("username").unwrap_or_else(|| email.to_string()));
        Some((
            ServerEinstellung {
                host,
                port,
                verschluesselung,
                zertifikat: None,
            },
            benutzername,
        ))
    };

    let (imap, benutzername) = anbieter
        .children()
        .filter(|k| k.has_tag_name("incomingServer") && k.attribute("type") == Some("imap"))
        .find_map(server_lesen)?;
    let (smtp, _) = anbieter
        .children()
        .filter(|k| k.has_tag_name("outgoingServer") && k.attribute("type") == Some("smtp"))
        .find_map(server_lesen)?;

    Some(Vorschlag {
        benutzername,
        imap,
        smtp,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BEISPIEL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<clientConfig version="1.1">
  <emailProvider id="example.org">
    <domain>example.org</domain>
    <incomingServer type="pop3">
      <hostname>pop.example.org</hostname>
      <port>995</port>
      <socketType>SSL</socketType>
      <username>%EMAILADDRESS%</username>
    </incomingServer>
    <incomingServer type="imap">
      <hostname>imap.example.org</hostname>
      <port>143</port>
      <socketType>plain</socketType>
      <username>%EMAILADDRESS%</username>
    </incomingServer>
    <incomingServer type="imap">
      <hostname>imap.example.org</hostname>
      <port>993</port>
      <socketType>SSL</socketType>
      <username>%EMAILLOCALPART%</username>
    </incomingServer>
    <outgoingServer type="smtp">
      <hostname>smtp.%EMAILDOMAIN%</hostname>
      <port>587</port>
      <socketType>STARTTLS</socketType>
      <username>%EMAILADDRESS%</username>
    </outgoingServer>
  </emailProvider>
</clientConfig>"#;

    #[test]
    fn weiterleitung_nur_innerhalb_der_domain() {
        assert!(gleiche_domain("autoconfig.example.org", "example.org"));
        assert!(gleiche_domain("Example.org.", "example.org"));
        assert!(!gleiche_domain(
            "example.org.angreifer.example",
            "example.org"
        ));
        assert!(!gleiche_domain("boese-example.org", "example.org"));
    }

    #[test]
    fn beispiel_wird_ausgewertet_und_klartext_ignoriert() {
        let vorschlag = auswerten(BEISPIEL, "anna@example.org").unwrap();
        assert_eq!(vorschlag.benutzername, "anna");
        assert_eq!(vorschlag.imap.host, "imap.example.org");
        assert_eq!(vorschlag.imap.port, 993);
        assert_eq!(vorschlag.imap.verschluesselung, Verschluesselung::Tls);
        assert_eq!(vorschlag.smtp.host, "smtp.example.org");
        assert_eq!(vorschlag.smtp.port, 587);
        assert_eq!(vorschlag.smtp.verschluesselung, Verschluesselung::Starttls);
    }

    #[test]
    fn nur_klartext_ergibt_keinen_vorschlag() {
        let xml = BEISPIEL.replace(
            "<socketType>SSL</socketType>",
            "<socketType>plain</socketType>",
        );
        let xml = xml.replace(
            "<socketType>STARTTLS</socketType>",
            "<socketType>plain</socketType>",
        );
        assert!(auswerten(&xml, "anna@example.org").is_none());
    }

    #[test]
    fn kaputtes_xml_ergibt_keinen_vorschlag() {
        assert!(auswerten("<clientConfig>", "anna@example.org").is_none());
    }

    #[test]
    fn domain_pruefung() {
        assert!(gueltige_domain("example.org"));
        assert!(!gueltige_domain("example"));
        assert!(!gueltige_domain("exa mple.org"));
        assert!(!gueltige_domain("example.org/pfad"));
        assert!(!gueltige_domain(".example.org"));
    }

    #[tokio::test]
    async fn ungueltige_adresse_wird_abgelehnt() {
        assert!(suchen("keine-adresse").await.is_err());
        assert!(suchen("a@exa mple.org").await.is_err());
    }
}
