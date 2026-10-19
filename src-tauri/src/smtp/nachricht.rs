//! Aus einem Entwurf eine Mail (RFC 5322) bauen.

use lettre::address::Envelope;
use lettre::message::header::ContentType;
use lettre::message::{Attachment, Mailbox, MessageBuilder, MultiPart, SinglePart};
use lettre::Message;

use super::entwurf::Entwurf;
use crate::fehler::Fehlermeldung;
use crate::konto::Konto;

/// Inhalt eines Anhangs zum Versand.
#[derive(Debug, Clone)]
pub struct AnhangDaten {
    pub name: String,
    pub typ: String,
    pub daten: Vec<u8>,
}

/// Fertig gebaute Mail.
#[derive(Debug, Clone)]
pub struct GebauteNachricht {
    pub umschlag: Envelope,
    /// Wird an den SMTP-Server geschickt (ohne Bcc-Kopf).
    pub zum_senden: Vec<u8>,
    /// Kopie für den Ordner „Gesendet“ (mit Bcc-Kopf, damit man sieht, wer Bcc bekam).
    pub fuer_gesendet: Vec<u8>,
    /// Message-ID mit spitzen Klammern.
    pub message_id: String,
}

/// Teilt eine Empfängerliste an Kommas oder Semikolons, ausser innerhalb von
/// Anführungszeichen oder spitzen Klammern („Muster, Anna“ <anna@example.org>).
pub fn empfaenger_teilen(liste: &str) -> Vec<String> {
    let mut teile = Vec::new();
    let mut aktuell = String::new();
    let mut in_zitat = false;
    let mut in_klammer = false;
    for zeichen in liste.chars() {
        match zeichen {
            '"' => in_zitat = !in_zitat,
            '<' if !in_zitat => in_klammer = true,
            '>' if !in_zitat => in_klammer = false,
            ',' | ';' if !in_zitat && !in_klammer => {
                teile.push(std::mem::take(&mut aktuell));
                continue;
            }
            _ => {}
        }
        aktuell.push(zeichen);
    }
    teile.push(aktuell);
    teile
        .into_iter()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

/// Liest eine Empfängerliste. Ungültige Einträge werden gesammelt gemeldet.
pub fn empfaenger_lesen(feld: &str, liste: &str) -> Result<Vec<Mailbox>, Fehlermeldung> {
    let mut gueltig = Vec::new();
    let mut ungueltig = Vec::new();
    for eintrag in empfaenger_teilen(liste) {
        match eintrag.parse::<Mailbox>() {
            Ok(adresse) => gueltig.push(adresse),
            Err(_) => ungueltig.push(eintrag),
        }
    }
    if ungueltig.is_empty() {
        Ok(gueltig)
    } else {
        Err(Fehlermeldung::neu(
            "eingabe",
            format!(
                "Ungültige Adresse im Feld „{feld}“: {}",
                ungueltig.join(", ")
            ),
        ))
    }
}

/// Einfache HTML-Fassung eines Textes: Zeichen maskiert, Zeilenumbrüche, Links, Zitate.
pub fn einfaches_html(text: &str) -> String {
    let maskieren = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    };
    let mut html = String::from(
        "<!doctype html><html><head><meta charset=\"utf-8\"></head><body><div style=\"font-family: sans-serif; font-size: 14px;\">",
    );
    for zeile in text.lines() {
        let zitat = zeile.starts_with('>');
        let inhalt: Vec<String> = zeile
            .split(' ')
            .map(|wort| {
                let ist_link = (wort.starts_with("https://") || wort.starts_with("http://"))
                    && !wort.contains(['"', '<', '>']);
                if ist_link {
                    let m = maskieren(wort);
                    format!("<a href=\"{m}\">{m}</a>")
                } else {
                    maskieren(wort)
                }
            })
            .collect();
        if zitat {
            html.push_str("<span style=\"color: #555;\">");
            html.push_str(&inhalt.join(" "));
            html.push_str("</span><br>");
        } else {
            html.push_str(&inhalt.join(" "));
            html.push_str("<br>");
        }
    }
    html.push_str("</div></body></html>");
    html
}

fn mit_klammern(id: &str) -> String {
    let id = id.trim().trim_start_matches('<').trim_end_matches('>');
    format!("<{id}>")
}

/// Baut die Mail.
pub fn bauen(
    konto: &Konto,
    entwurf: &Entwurf,
    anhaenge: Vec<AnhangDaten>,
) -> Result<GebauteNachricht, Fehlermeldung> {
    let an = empfaenger_lesen("An", &entwurf.an)?;
    let cc = empfaenger_lesen("Cc", &entwurf.cc)?;
    let bcc = empfaenger_lesen("Bcc", &entwurf.bcc)?;
    if an.is_empty() && cc.is_empty() && bcc.is_empty() {
        return Err(Fehlermeldung::neu(
            "eingabe",
            "Bitte mindestens einen Empfänger angeben.",
        ));
    }
    let absender = Mailbox::new(
        Some(konto.name.clone()),
        konto.email.parse().map_err(|_| {
            Fehlermeldung::neu("eingabe", "Die Absenderadresse des Kontos ist ungültig.")
        })?,
    );
    let domain = konto
        .email
        .rsplit_once('@')
        .map(|(_, d)| d.to_string())
        .unwrap_or_else(|| "localhost".into());
    let message_id = format!("<{}@{}>", uuid::Uuid::new_v4().simple(), domain);

    let referenzen: Vec<String> = {
        let mut liste: Vec<String> = entwurf.referenzen.iter().map(|r| mit_klammern(r)).collect();
        if let Some(id) = &entwurf.antwort_auf {
            let id = mit_klammern(id);
            if !liste.contains(&id) {
                liste.push(id);
            }
        }
        liste
    };

    let kopf = || -> MessageBuilder {
        let mut b = Message::builder()
            .from(absender.clone())
            .subject(entwurf.betreff.trim())
            .message_id(Some(message_id.clone()))
            .date_now();
        for m in &an {
            b = b.to(m.clone());
        }
        for m in &cc {
            b = b.cc(m.clone());
        }
        for m in &bcc {
            b = b.bcc(m.clone());
        }
        if let Some(id) = &entwurf.antwort_auf {
            b = b.in_reply_to(mit_klammern(id));
        }
        if !referenzen.is_empty() {
            b = b.references(referenzen.join(" "));
        }
        b
    };

    let fertig = |builder: MessageBuilder| -> Result<Message, Fehlermeldung> {
        let text = || SinglePart::plain(entwurf.text.clone());
        let text_und_html = || {
            MultiPart::alternative_plain_html(entwurf.text.clone(), einfaches_html(&entwurf.text))
        };
        let ergebnis = match (anhaenge.is_empty(), entwurf.html) {
            (true, false) => builder.singlepart(text()),
            (true, true) => builder.multipart(text_und_html()),
            (false, html) => {
                let mut teil = if html {
                    MultiPart::mixed().multipart(text_und_html())
                } else {
                    MultiPart::mixed().singlepart(text())
                };
                for anhang in &anhaenge {
                    let typ = ContentType::parse(&anhang.typ)
                        .or_else(|_| ContentType::parse("application/octet-stream"))
                        .map_err(|_| Fehlermeldung::neu("eingabe", "Unbekannter Anhangstyp."))?;
                    teil = teil.singlepart(
                        Attachment::new(anhang.name.clone()).body(anhang.daten.clone(), typ),
                    );
                }
                builder.multipart(teil)
            }
        };
        ergebnis
            .map_err(|_| Fehlermeldung::neu("eingabe", "Die Mail konnte nicht erstellt werden."))
    };

    let zum_senden = fertig(kopf())?;
    let fuer_gesendet = fertig(kopf().keep_bcc())?;
    Ok(GebauteNachricht {
        umschlag: zum_senden.envelope().clone(),
        zum_senden: zum_senden.formatted(),
        fuer_gesendet: fuer_gesendet.formatted(),
        message_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::konto::{ServerEinstellung, Verschluesselung};

    fn konto() -> Konto {
        let server = ServerEinstellung {
            host: "mail.example.org".into(),
            port: 465,
            verschluesselung: Verschluesselung::Tls,
            zertifikat: None,
        };
        Konto {
            id: "k".into(),
            name: "Anna Muster".into(),
            email: "anna@example.org".into(),
            benutzername: "anna".into(),
            imap: server.clone(),
            smtp: server,
            gesendet_ablegen: true,
            absender_kennung: String::new(),
        }
    }

    fn entwurf() -> Entwurf {
        Entwurf {
            konto: "k".into(),
            an: "\"Muster, Bea\" <bea@example.org>; carl@example.org".into(),
            cc: "dora@example.org".into(),
            bcc: "geheim@example.org".into(),
            betreff: "Grüsse aus Bern".into(),
            text: "Hallo Bea,\n\nsiehe https://example.org/a?b=1&c=2\n> Zitat <b>\n".into(),
            ..Default::default()
        }
    }

    fn parsen(daten: &[u8]) -> mail_parser::Message<'_> {
        mail_parser::MessageParser::default().parse(daten).unwrap()
    }

    #[test]
    fn empfaenger_mit_komma_im_namen() {
        assert_eq!(
            empfaenger_teilen("\"Muster, Bea\" <bea@example.org>, carl@example.org;; "),
            vec!["\"Muster, Bea\" <bea@example.org>", "carl@example.org"]
        );
        assert!(empfaenger_lesen("An", "keine adresse, a@example.org").is_err());
        let fehler = empfaenger_lesen("Cc", "a@@b").unwrap_err();
        assert!(fehler.meldung.contains("Cc"));
    }

    #[test]
    fn einfache_mail_mit_bcc_nur_im_umschlag() {
        let n = bauen(&konto(), &entwurf(), vec![]).unwrap();
        let empfaenger: Vec<String> = n.umschlag.to().iter().map(|a| a.to_string()).collect();
        assert_eq!(
            empfaenger,
            vec![
                "bea@example.org",
                "carl@example.org",
                "dora@example.org",
                "geheim@example.org"
            ]
        );
        let gesendet = String::from_utf8_lossy(&n.zum_senden);
        assert!(
            !gesendet.contains("geheim@example.org"),
            "Bcc darf nicht im Kopf stehen"
        );
        let kopie = String::from_utf8_lossy(&n.fuer_gesendet);
        assert!(
            kopie.contains("geheim@example.org"),
            "Kopie für „Gesendet“ behält Bcc"
        );

        let m = parsen(&n.zum_senden);
        assert_eq!(m.subject(), Some("Grüsse aus Bern"));
        assert_eq!(
            m.from().unwrap().first().unwrap().address(),
            Some("anna@example.org")
        );
        assert_eq!(m.to().unwrap().first().unwrap().name(), Some("Muster, Bea"));
        assert!(m.body_text(0).unwrap().contains("Hallo Bea"));
        assert_eq!(m.message_id(), Some(n.message_id.trim_matches(['<', '>'])));
        assert!(n.message_id.ends_with("@example.org>"));
        assert!(m.in_reply_to().is_empty());
    }

    #[test]
    fn antwort_setzt_in_reply_to_und_references() {
        let mut e = entwurf();
        e.antwort_auf = Some("orig@example.org".into());
        e.referenzen = vec!["eins@example.org".into(), "<zwei@example.org>".into()];
        let n = bauen(&konto(), &e, vec![]).unwrap();
        let roh = String::from_utf8_lossy(&n.zum_senden);
        assert!(roh.contains("In-Reply-To: <orig@example.org>"), "{roh}");
        assert!(
            roh.contains("References: <eins@example.org> <zwei@example.org> <orig@example.org>"),
            "{roh}"
        );
        let m = parsen(&n.zum_senden);
        assert_eq!(m.in_reply_to().as_text(), Some("orig@example.org"));
    }

    #[test]
    fn anhang_und_html() {
        let mut e = entwurf();
        e.html = true;
        let n = bauen(
            &konto(),
            &e,
            vec![AnhangDaten {
                name: "Bericht.pdf".into(),
                typ: "application/pdf".into(),
                daten: b"%PDF-1.4".to_vec(),
            }],
        )
        .unwrap();
        let m = parsen(&n.zum_senden);
        assert_eq!(m.attachment_count(), 1);
        let anhang = m.attachment(0).unwrap();
        assert_eq!(
            mail_parser::MimeHeaders::attachment_name(anhang),
            Some("Bericht.pdf")
        );
        assert_eq!(anhang.contents(), b"%PDF-1.4");
        let html = m.body_html(0).unwrap();
        assert!(html.contains("<a href=\"https://example.org/a?b=1&amp;c=2\">"));
        assert!(html.contains("&lt;b&gt;"), "Text wird maskiert");
        assert!(m.body_text(0).unwrap().contains("Hallo Bea"));
    }

    #[test]
    fn ohne_empfaenger_gibt_es_einen_fehler() {
        let e = Entwurf {
            konto: "k".into(),
            betreff: "x".into(),
            ..Default::default()
        };
        assert!(bauen(&konto(), &e, vec![]).is_err());
    }
}
