//! Ordnerliste eines IMAP-Kontos.

use async_imap::types::NameAttribute;
use base64::Engine as _;
use futures_util::TryStreamExt;
use serde::Serialize;

use super::ImapSitzung;
use crate::fehler::VerbindungsFehler;

/// Besondere Ordner (RFC 6154 bzw. INBOX).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Sonderordner {
    Posteingang,
    Gesendet,
    Entwuerfe,
    Papierkorb,
    Spam,
    Archiv,
}

/// Ein Ordner auf dem Server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Ordner {
    /// Name, wie ihn der Server verwendet (für alle Befehle).
    pub name: String,
    /// Lesbarer Name (aus modifiziertem UTF-7 dekodiert).
    pub anzeigename: String,
    pub trenner: Option<String>,
    pub sonder: Option<Sonderordner>,
    /// `false` bei reinen Gliederungsordnern (\Noselect).
    pub waehlbar: bool,
}

/// Lädt alle Ordner des Kontos, sortiert: Posteingang zuerst, dann Sonderordner, dann alphabetisch.
pub async fn ordner_laden(sitzung: &mut ImapSitzung) -> Result<Vec<Ordner>, VerbindungsFehler> {
    let namen: Vec<_> = sitzung
        .list(Some(""), Some("*"))
        .await
        .map_err(super::fehler_einordnen)?
        .try_collect()
        .await
        .map_err(super::fehler_einordnen)?;

    let mut ordner: Vec<Ordner> = namen
        .iter()
        .map(|n| {
            let mut sonder = None;
            let mut waehlbar = true;
            for attribut in n.attributes() {
                match attribut {
                    NameAttribute::NoSelect => waehlbar = false,
                    NameAttribute::Sent => sonder = Some(Sonderordner::Gesendet),
                    NameAttribute::Drafts => sonder = Some(Sonderordner::Entwuerfe),
                    NameAttribute::Trash => sonder = Some(Sonderordner::Papierkorb),
                    NameAttribute::Junk => sonder = Some(Sonderordner::Spam),
                    NameAttribute::Archive => sonder = Some(Sonderordner::Archiv),
                    NameAttribute::Extension(e) if e.eq_ignore_ascii_case("\\NonExistent") => {
                        waehlbar = false
                    }
                    _ => {}
                }
            }
            if n.name().eq_ignore_ascii_case("INBOX") {
                sonder = Some(Sonderordner::Posteingang);
            }
            let anzeigename = if sonder == Some(Sonderordner::Posteingang) {
                "Posteingang".to_string()
            } else {
                utf7_dekodieren(n.name())
            };
            Ordner {
                name: n.name().to_string(),
                anzeigename,
                trenner: n.delimiter().map(str::to_string),
                sonder,
                waehlbar,
            }
        })
        .collect();

    // Server ohne Sonderordner-Kennzeichnung: übliche Namen erkennen.
    for o in ordner.iter_mut().filter(|o| o.sonder.is_none()) {
        let kurz = o
            .trenner
            .as_deref()
            .and_then(|t| o.anzeigename.rsplit(t).next())
            .unwrap_or(&o.anzeigename)
            .to_lowercase();
        o.sonder = match kurz.as_str() {
            "sent" | "gesendet" | "sent items" | "sent messages" | "gesendete elemente"
            | "gesendete objekte" => Some(Sonderordner::Gesendet),
            "drafts" | "entwürfe" | "entwuerfe" => Some(Sonderordner::Entwuerfe),
            "trash"
            | "papierkorb"
            | "deleted items"
            | "deleted messages"
            | "gelöschte elemente" => Some(Sonderordner::Papierkorb),
            "junk" | "spam" => Some(Sonderordner::Spam),
            "archive" | "archiv" => Some(Sonderordner::Archiv),
            _ => None,
        };
    }

    ordner.sort_by_key(|o| {
        let rang = match o.sonder {
            Some(Sonderordner::Posteingang) => 0,
            Some(Sonderordner::Entwuerfe) => 1,
            Some(Sonderordner::Gesendet) => 2,
            Some(Sonderordner::Archiv) => 3,
            Some(Sonderordner::Spam) => 4,
            Some(Sonderordner::Papierkorb) => 5,
            None => 6,
        };
        (rang, o.anzeigename.to_lowercase())
    });
    Ok(ordner)
}

/// Dekodiert einen Ordnernamen in modifiziertem UTF-7 (RFC 3501, Abschnitt 5.1.3).
///
/// Ungültige Folgen bleiben unverändert stehen.
pub fn utf7_dekodieren(name: &str) -> String {
    let mut ergebnis = String::new();
    let mut rest = name;
    while let Some(start) = rest.find('&') {
        ergebnis.push_str(&rest[..start]);
        let nach = &rest[start + 1..];
        let Some(ende) = nach.find('-') else {
            ergebnis.push_str(&rest[start..]);
            return ergebnis;
        };
        let kodiert = &nach[..ende];
        if kodiert.is_empty() {
            ergebnis.push('&');
        } else {
            let b64 = kodiert.replace(',', "/");
            let dekodiert = base64::engine::general_purpose::STANDARD_NO_PAD
                .decode(b64.as_bytes())
                .ok()
                .filter(|b| b.len() % 2 == 0)
                .and_then(|bytes| {
                    let einheiten: Vec<u16> = bytes
                        .chunks(2)
                        .map(|c| u16::from_be_bytes([c[0], c[1]]))
                        .collect();
                    String::from_utf16(&einheiten).ok()
                });
            match dekodiert {
                Some(text) => ergebnis.push_str(&text),
                None => {
                    ergebnis.push('&');
                    ergebnis.push_str(kodiert);
                    ergebnis.push('-');
                }
            }
        }
        rest = &nach[ende + 1..];
    }
    ergebnis.push_str(rest);
    ergebnis
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf7_beispiele() {
        assert_eq!(utf7_dekodieren("INBOX"), "INBOX");
        assert_eq!(utf7_dekodieren("Entw&APw-rfe"), "Entwürfe");
        assert_eq!(
            utf7_dekodieren("Gel&APY-schte Elemente"),
            "Gelöschte Elemente"
        );
        assert_eq!(utf7_dekodieren("A&-B"), "A&B");
        assert_eq!(utf7_dekodieren("&ZeVnLIqe-"), "日本語");
        assert_eq!(utf7_dekodieren("kaputt&"), "kaputt&");
        assert_eq!(utf7_dekodieren("&!!!-"), "&!!!-");
    }
}
