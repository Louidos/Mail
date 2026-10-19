//! Mails sicher zur Anzeige aufbereiten.
//!
//! - MIME wird mit `mail-parser` gelesen, Text- und HTML-Teil werden erkannt.
//! - HTML wird im Backend bereinigt (`html`), externe Inhalte sind standardmässig blockiert.
//! - Anhänge werden nur aufgelistet; gespeichert werden sie ausschliesslich über „Speichern unter“.

pub mod absender;
pub mod extern_inhalte;
pub mod html;

use std::collections::HashMap;

use base64::Engine as _;
use mail_parser::{Address, HeaderValue, Message, MessageParser, MimeHeaders, PartType};
use serde::Serialize;

use crate::fehler::Fehlermeldung;

/// Eine Adresse für die Anzeige.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Adresse {
    pub name: Option<String>,
    pub adresse: String,
}

/// Ein Anhang (ohne Inhalt).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Anhang {
    pub index: usize,
    pub name: String,
    pub typ: String,
    pub groesse: usize,
}

/// Für die Anzeige aufbereitete Mail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AufbereiteteMail {
    pub betreff: String,
    pub von: Vec<Adresse>,
    pub an: Vec<Adresse>,
    pub cc: Vec<Adresse>,
    pub antwort_an: Vec<Adresse>,
    pub datum: Option<i64>,
    pub message_id: Option<String>,
    pub referenzen: Vec<String>,
    /// Text der Mail (bei reinen HTML-Mails aus dem HTML erzeugt).
    pub text: String,
    /// Bereinigtes HTML, falls die Mail einen HTML-Teil hat.
    pub html: Option<String>,
    /// Anzahl blockierter externer Inhalte.
    pub extern_blockiert: usize,
    /// Wurden externe Inhalte geladen?
    pub extern_geladen: bool,
    pub anhaenge: Vec<Anhang>,
    /// Ergebnis der Absenderprüfung des eigenen Servers (DKIM/SPF/DMARC, #45).
    pub absender_pruefung: absender::Absenderpruefung,
    /// Ergebnis von Entschlüsselung und Signaturprüfung (OpenPGP, #44).
    pub pgp: crate::pgp::lesen::PgpStatus,
}

/// Wie mit externen Inhalten umgegangen wird.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternModus {
    Blockieren,
    Laden,
}

fn adressen(wert: Option<&Address>) -> Vec<Adresse> {
    let Some(wert) = wert else {
        return Vec::new();
    };
    wert.iter()
        .filter_map(|a| {
            Some(Adresse {
                name: a
                    .name()
                    .map(str::to_string)
                    .filter(|n| !n.trim().is_empty()),
                adresse: a.address()?.to_string(),
            })
        })
        .collect()
}

fn liste(wert: &HeaderValue) -> Vec<String> {
    match wert {
        HeaderValue::Text(t) => vec![t.to_string()],
        HeaderValue::TextList(l) => l.iter().map(|t| t.to_string()).collect(),
        _ => Vec::new(),
    }
}

/// Liest eine Mail ein.
pub fn parsen(roh: &[u8]) -> Result<Message<'_>, Fehlermeldung> {
    MessageParser::default()
        .parse(roh)
        .ok_or_else(|| Fehlermeldung::neu("mail", "Die Mail konnte nicht gelesen werden."))
}

/// Eingebettete Bilder (Content-ID → `data:`-Adresse), nur übliche Bildformate.
fn eingebettete_bilder(nachricht: &Message) -> HashMap<String, String> {
    let mut bilder = HashMap::new();
    for teil in &nachricht.parts {
        let (Some(cid), Some(typ)) = (teil.content_id(), teil.content_type()) else {
            continue;
        };
        let mime = format!(
            "{}/{}",
            typ.ctype().to_ascii_lowercase(),
            typ.subtype().unwrap_or_default().to_ascii_lowercase()
        );
        if !["image/png", "image/jpeg", "image/gif", "image/webp"].contains(&mime.as_str()) {
            continue;
        }
        let daten = base64::engine::general_purpose::STANDARD.encode(teil.contents());
        bilder.insert(
            cid.trim_matches(|c| c == '<' || c == '>')
                .to_ascii_lowercase(),
            format!("data:{mime};base64,{daten}"),
        );
    }
    bilder
}

/// Anhänge in der Reihenfolge von `Message::attachments()`.
pub fn anhaenge(nachricht: &Message) -> Vec<Anhang> {
    nachricht
        .attachments()
        .enumerate()
        .map(|(index, teil)| {
            let typ = teil
                .content_type()
                .map(|t| match t.subtype() {
                    Some(sub) => format!("{}/{}", t.ctype(), sub),
                    None => t.ctype().to_string(),
                })
                .unwrap_or_else(|| "application/octet-stream".into());
            let name = teil
                .attachment_name()
                .map(str::to_string)
                .unwrap_or_else(|| format!("anhang-{}", index + 1));
            Anhang {
                index,
                name: crate::dateien::dateiname_saeubern(&name, &format!("anhang-{}", index + 1)),
                typ,
                groesse: teil.contents().len(),
            }
        })
        .collect()
}

/// Inhalt eines Anhangs mit Dateiname.
pub fn anhang_inhalt(roh: &[u8], index: usize) -> Result<(String, Vec<u8>), Fehlermeldung> {
    let nachricht = parsen(roh)?;
    let name = anhaenge(&nachricht)
        .into_iter()
        .find(|a| a.index == index)
        .map(|a| a.name)
        .ok_or_else(|| Fehlermeldung::neu("nicht_gefunden", "Diesen Anhang gibt es nicht."))?;
    let teil = nachricht
        .attachments()
        .nth(index)
        .ok_or_else(|| Fehlermeldung::neu("nicht_gefunden", "Diesen Anhang gibt es nicht."))?;
    Ok((name, teil.contents().to_vec()))
}

/// Bereitet eine Mail für die Anzeige auf.
///
/// `extern_geladen` enthält bereits geladene externe Bilder (bei [`ExternModus::Laden`]).
pub fn aufbereiten(
    roh: &[u8],
    extern_geladen: Option<&HashMap<String, String>>,
) -> Result<AufbereiteteMail, Fehlermeldung> {
    let nachricht = parsen(roh)?;
    let text = nachricht
        .body_text(0)
        .map(|t| t.into_owned())
        .unwrap_or_default();

    let html_quelle = nachricht.html_part(0).and_then(|teil| match &teil.body {
        PartType::Html(html) => Some(html.as_ref()),
        _ => None,
    });
    let (html, extern_blockiert) = match html_quelle {
        Some(quelle) => {
            let bereinigt =
                html::bereinigen(quelle, &eingebettete_bilder(&nachricht), extern_geladen);
            (Some(bereinigt.html), bereinigt.blockiert.len())
        }
        None => (None, 0),
    };

    Ok(AufbereiteteMail {
        betreff: nachricht.subject().unwrap_or_default().to_string(),
        von: adressen(nachricht.from()),
        an: adressen(nachricht.to()),
        cc: adressen(nachricht.cc()),
        antwort_an: adressen(nachricht.reply_to()),
        datum: nachricht.date().map(|d| d.to_timestamp()),
        message_id: nachricht.message_id().map(str::to_string),
        referenzen: liste(nachricht.references()),
        text,
        html,
        extern_blockiert,
        extern_geladen: extern_geladen.is_some(),
        anhaenge: anhaenge(&nachricht),
        absender_pruefung: absender::Absenderpruefung::default(),
        pgp: crate::pgp::lesen::PgpStatus::default(),
    })
}

/// Prüft den Absender anhand des obersten `Authentication-Results`-Headers mit `kennung`.
pub fn absender_pruefen(
    roh: &[u8],
    kennung: &str,
) -> Result<absender::Absenderpruefung, Fehlermeldung> {
    let nachricht = parsen(roh)?;
    Ok(absender::pruefen(roh, &nachricht, kennung))
}

/// Adressen externer Bilder, die beim Blockieren entfernt würden.
pub fn externe_adressen(roh: &[u8]) -> Result<Vec<String>, Fehlermeldung> {
    let nachricht = parsen(roh)?;
    Ok(match nachricht.html_part(0).map(|t| &t.body) {
        Some(PartType::Html(quelle)) => html::bereinigen(quelle, &HashMap::new(), None).blockiert,
        _ => Vec::new(),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const MULTIPART: &str = "From: =?UTF-8?Q?J=C3=BCrg?= <juerg@example.org>\r\n\
To: Anna <anna@example.org>, bea@example.org\r\n\
Cc: chef@example.org\r\n\
Reply-To: antwort@example.org\r\n\
Subject: =?UTF-8?Q?Gr=C3=BCsse?=\r\n\
Date: Mon, 19 Oct 2026 10:00:00 +0200\r\n\
Message-ID: <abc@example.org>\r\n\
References: <eins@example.org> <zwei@example.org>\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"aussen\"\r\n\
\r\n\
--aussen\r\n\
Content-Type: multipart/alternative; boundary=\"innen\"\r\n\
\r\n\
--innen\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
Hallo Anna, das ist der Text.\r\n\
--innen\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<p>Hallo <b>Anna</b></p><script>alert(1)</script><img src=\"https://tracker.example/p.gif\" onerror=\"alert(2)\">\r\n\
--innen--\r\n\
--aussen\r\n\
Content-Type: application/pdf; name=\"../../Rechnung.pdf\"\r\n\
Content-Disposition: attachment; filename=\"../../Rechnung.pdf\"\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
JVBERi0xLjQK\r\n\
--aussen--\r\n";

    #[test]
    fn multipart_wird_aufbereitet() {
        let mail = aufbereiten(MULTIPART.as_bytes(), None).unwrap();
        assert_eq!(mail.betreff, "Grüsse");
        assert_eq!(mail.von[0].name.as_deref(), Some("Jürg"));
        assert_eq!(mail.von[0].adresse, "juerg@example.org");
        assert_eq!(mail.an.len(), 2);
        assert_eq!(mail.cc[0].adresse, "chef@example.org");
        assert_eq!(mail.antwort_an[0].adresse, "antwort@example.org");
        assert_eq!(mail.message_id.as_deref(), Some("abc@example.org"));
        assert_eq!(
            mail.referenzen,
            vec!["eins@example.org", "zwei@example.org"]
        );
        assert!(mail.text.contains("das ist der Text"));
        let html = mail.html.unwrap();
        assert!(html.contains("<b>Anna</b>"));
        assert!(!html.contains("script") && !html.contains("onerror") && !html.contains("tracker"));
        assert_eq!(mail.extern_blockiert, 1);
        assert_eq!(
            mail.anhaenge,
            vec![Anhang {
                index: 0,
                name: "_.._Rechnung.pdf".into(),
                typ: "application/pdf".into(),
                groesse: 9
            }]
        );
        let (name, inhalt) = anhang_inhalt(MULTIPART.as_bytes(), 0).unwrap();
        assert_eq!(name, "_.._Rechnung.pdf");
        assert_eq!(inhalt, b"%PDF-1.4\n");
        assert!(anhang_inhalt(MULTIPART.as_bytes(), 1).is_err());
        assert_eq!(
            externe_adressen(MULTIPART.as_bytes()).unwrap(),
            vec!["https://tracker.example/p.gif"]
        );
    }

    #[test]
    fn nur_text_hat_kein_html() {
        let roh = "From: a@example.org\r\nSubject: Text\r\n\r\nNur <b>Text</b>\r\n";
        let mail = aufbereiten(roh.as_bytes(), None).unwrap();
        assert!(mail.html.is_none());
        assert!(mail.text.contains("Nur <b>Text</b>"));
    }

    #[test]
    fn nur_html_ergibt_auch_text() {
        let roh = "From: a@example.org\r\nSubject: HTML\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>Erster Absatz</p><p>Zweiter</p>\r\n";
        let mail = aufbereiten(roh.as_bytes(), None).unwrap();
        assert!(mail.html.is_some());
        assert!(mail.text.contains("Erster Absatz"));
        assert!(!mail.text.contains("<p>"));
    }

    #[test]
    fn eingebettete_bilder_werden_eingesetzt() {
        let roh = "From: a@example.org\r\nSubject: Bild\r\nMIME-Version: 1.0\r\n\
Content-Type: multipart/related; boundary=\"g\"\r\n\r\n\
--g\r\nContent-Type: text/html\r\n\r\n<img src=\"cid:Logo@Firma\">\r\n\
--g\r\nContent-Type: image/png\r\nContent-ID: <logo@firma>\r\nContent-Transfer-Encoding: base64\r\n\r\niVBORw0KGgo=\r\n--g--\r\n";
        let mail = aufbereiten(roh.as_bytes(), None).unwrap();
        assert!(mail
            .html
            .unwrap()
            .contains("data:image/png;base64,iVBORw0KGgo="));
        assert_eq!(mail.extern_blockiert, 0);
    }
}
