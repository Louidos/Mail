//! Aus einem Entwurf eine Mail (RFC 5322) bauen – auf Wunsch mit OpenPGP verschlüsselt und/oder
//! signiert (PGP/MIME nach RFC 3156).
//!
//! Bcc bei Verschlüsselung: Jeder Bcc-Empfänger bekommt eine eigene Nachricht, die nur an seinen
//! Schlüssel verschlüsselt ist. So stehen in der Nachricht für An/Cc keine Schlüssel-IDs der
//! Bcc-Empfänger, und Bcc-Empfänger sehen nicht, wer sonst Bcc bekam.

use lettre::address::Envelope;
use lettre::message::header::{ContentDisposition, ContentType};
use lettre::message::{Attachment, Mailbox, MessageBuilder, MultiPart, SinglePart};
use lettre::Message;

use super::entwurf::Entwurf;
use crate::fehler::Fehlermeldung;
use crate::konto::Konto;
use crate::pgp::schreiben;
use crate::pgp::VersandSchluessel;

/// Inhalt eines Anhangs zum Versand.
#[derive(Debug, Clone)]
pub struct AnhangDaten {
    pub name: String,
    pub typ: String,
    pub daten: Vec<u8>,
}

/// Eine Übergabe an den SMTP-Server.
#[derive(Debug, Clone)]
pub struct Sendung {
    pub umschlag: Envelope,
    /// Die Mail ohne Bcc-Kopf.
    pub daten: Vec<u8>,
}

/// Fertig gebaute Mail.
#[derive(Debug, Clone)]
pub struct GebauteNachricht {
    /// Was an den SMTP-Server geht. Ohne Verschlüsselung eine Sendung an alle (Bcc nur im
    /// Umschlag), mit Verschlüsselung zusätzlich eine eigene Sendung je Bcc-Empfänger.
    pub sendungen: Vec<Sendung>,
    /// Kopie für den Ordner „Gesendet“ (mit Bcc-Kopf, damit man sieht, wer Bcc bekam).
    pub fuer_gesendet: Vec<u8>,
    /// Message-ID mit spitzen Klammern.
    pub message_id: String,
}

/// Inhalt der Mail (Text, HTML, Anhänge) als MIME-Entität.
#[derive(Clone)]
enum Teil {
    Einzeln(SinglePart),
    Mehrteilig(MultiPart),
}

impl Teil {
    fn formatted(&self) -> Vec<u8> {
        match self {
            Teil::Einzeln(t) => t.formatted(),
            Teil::Mehrteilig(t) => t.formatted(),
        }
    }

    fn in_multipart(self, aussen: lettre::message::MultiPartBuilder) -> MultiPart {
        match self {
            Teil::Einzeln(t) => aussen.singlepart(t),
            Teil::Mehrteilig(t) => aussen.multipart(t),
        }
    }
}

/// Text, optional HTML und Anhänge.
fn inhalt(entwurf: &Entwurf, anhaenge: &[AnhangDaten]) -> Result<Teil, Fehlermeldung> {
    let text = || SinglePart::plain(entwurf.text.clone());
    let text_und_html =
        || MultiPart::alternative_plain_html(entwurf.text.clone(), einfaches_html(&entwurf.text));
    Ok(match (anhaenge.is_empty(), entwurf.html) {
        (true, false) => Teil::Einzeln(text()),
        (true, true) => Teil::Mehrteilig(text_und_html()),
        (false, html) => {
            let mut teil = if html {
                MultiPart::mixed().multipart(text_und_html())
            } else {
                MultiPart::mixed().singlepart(text())
            };
            for anhang in anhaenge {
                let typ = ContentType::parse(&anhang.typ)
                    .or_else(|_| ContentType::parse("application/octet-stream"))
                    .map_err(|_| Fehlermeldung::neu("eingabe", "Unbekannter Anhangstyp."))?;
                teil = teil.singlepart(
                    Attachment::new(anhang.name.clone()).body(anhang.daten.clone(), typ),
                );
            }
            Teil::Mehrteilig(teil)
        }
    })
}

/// `multipart/encrypted` mit einer ASCII-armored OpenPGP-Nachricht (RFC 3156, Abschnitt 4).
fn verschluesselt_teil(armored: String) -> Result<MultiPart, Fehlermeldung> {
    let typ = |t: &str| {
        ContentType::parse(t)
            .map_err(|_| Fehlermeldung::neu("intern", "Die Mail konnte nicht erstellt werden."))
    };
    Ok(MultiPart::encrypted("application/pgp-encrypted".into())
        .singlepart(
            SinglePart::builder()
                .header(typ("application/pgp-encrypted")?)
                .body(String::from("Version: 1\r\n")),
        )
        .singlepart(
            SinglePart::builder()
                .header(typ("application/octet-stream; name=\"encrypted.asc\"")?)
                .header(ContentDisposition::inline_with_name("encrypted.asc"))
                .body(armored),
        ))
}

/// `multipart/signed` mit abgetrennter Signatur (RFC 3156, Abschnitt 5).
fn signiert_teil(inhalt: Teil, signatur: String) -> Result<MultiPart, Fehlermeldung> {
    let typ = ContentType::parse("application/pgp-signature; name=\"signature.asc\"")
        .map_err(|_| Fehlermeldung::neu("intern", "Die Mail konnte nicht erstellt werden."))?;
    let aussen = MultiPart::signed("application/pgp-signature".into(), schreiben::MICALG.into());
    Ok(inhalt.in_multipart(aussen).singlepart(
        SinglePart::builder()
            .header(typ)
            .header(ContentDisposition::attachment("signature.asc"))
            .body(signatur),
    ))
}

/// Signiert wird die Entität genau so, wie sie zwischen den Begrenzern steht: ohne das letzte
/// CRLF, das nach RFC 2046 zum folgenden Begrenzer gehört.
fn signierte_daten(inhalt: &Teil) -> Vec<u8> {
    let mut daten = inhalt.formatted();
    if daten.ends_with(b"\r\n") {
        daten.truncate(daten.len() - 2);
    }
    daten
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

/// Alle Empfängeradressen (An, Cc, Bcc), z. B. um die passenden Schlüssel zu suchen.
pub fn alle_empfaenger(entwurf: &Entwurf) -> Result<Vec<String>, Fehlermeldung> {
    let mut adressen = Vec::new();
    for (feld, liste) in [
        ("An", &entwurf.an),
        ("Cc", &entwurf.cc),
        ("Bcc", &entwurf.bcc),
    ] {
        adressen.extend(
            empfaenger_lesen(feld, liste)?
                .into_iter()
                .map(|m| m.email.to_string().to_lowercase()),
        );
    }
    Ok(adressen)
}

/// Baut die Mail ohne OpenPGP.
pub fn bauen(
    konto: &Konto,
    entwurf: &Entwurf,
    anhaenge: Vec<AnhangDaten>,
) -> Result<GebauteNachricht, Fehlermeldung> {
    bauen_mit(konto, entwurf, &anhaenge, None)
}

/// Baut die Mail, mit OpenPGP, wenn `pgp` gesetzt ist (Verschlüsseln, wenn
/// `pgp.empfaenger` gesetzt ist; Signieren, wenn `pgp.signierer` gesetzt ist).
pub fn bauen_mit(
    konto: &Konto,
    entwurf: &Entwurf,
    anhaenge: &[AnhangDaten],
    pgp: Option<&VersandSchluessel>,
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

    let kopf = |mit_bcc: bool| -> MessageBuilder {
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
        if mit_bcc {
            for m in &bcc {
                b = b.bcc(m.clone());
            }
        }
        if let Some(id) = &entwurf.antwort_auf {
            b = b.in_reply_to(mit_klammern(id));
        }
        if !referenzen.is_empty() {
            b = b.references(referenzen.join(" "));
        }
        b
    };

    let erstellt = |ergebnis: Result<Message, lettre::error::Error>| {
        ergebnis
            .map_err(|_| Fehlermeldung::neu("eingabe", "Die Mail konnte nicht erstellt werden."))
    };
    let mit_teil = |builder: MessageBuilder, teil: Teil| match teil {
        Teil::Einzeln(t) => erstellt(builder.singlepart(t)),
        Teil::Mehrteilig(t) => erstellt(builder.multipart(t)),
    };
    let inhalt = inhalt(entwurf, anhaenge)?;

    let Some(pgp) = pgp else {
        // Ohne OpenPGP: eine Sendung an alle, Bcc nur im Umschlag.
        let zum_senden = mit_teil(kopf(true), inhalt.clone())?;
        let fuer_gesendet = mit_teil(kopf(true).keep_bcc(), inhalt)?;
        return Ok(GebauteNachricht {
            sendungen: vec![Sendung {
                umschlag: zum_senden.envelope().clone(),
                daten: zum_senden.formatted(),
            }],
            fuer_gesendet: fuer_gesendet.formatted(),
            message_id,
        });
    };

    let Some(schluessel) = &pgp.empfaenger else {
        // Nur signieren: eine Sendung an alle, Bcc nur im Umschlag.
        let signierer = pgp
            .signierer
            .as_ref()
            .ok_or_else(|| Fehlermeldung::neu("intern", "Kein Schlüssel zum Signieren."))?;
        let signatur = schreiben::signieren(&signierte_daten(&inhalt), signierer)?;
        let teil = Teil::Mehrteilig(signiert_teil(inhalt, signatur)?);
        let zum_senden = mit_teil(kopf(true), teil.clone())?;
        let fuer_gesendet = mit_teil(kopf(true).keep_bcc(), teil)?;
        return Ok(GebauteNachricht {
            sendungen: vec![Sendung {
                umschlag: zum_senden.envelope().clone(),
                daten: zum_senden.formatted(),
            }],
            fuer_gesendet: fuer_gesendet.formatted(),
            message_id,
        });
    };

    // Verschlüsseln (optional mit Signatur innerhalb der Verschlüsselung).
    let schluessel_von = |m: &Mailbox| {
        schluessel
            .get(&m.email.to_string().to_lowercase())
            .ok_or_else(|| {
                Fehlermeldung::neu(
                    "pgp_schluessel_fehlt",
                    format!("Für {} fehlt ein öffentlicher Schlüssel.", m.email),
                )
            })
    };
    let daten = inhalt.formatted();
    let signierer = pgp.signierer.as_ref();

    // An und Cc und immer auch der eigene Schlüssel (damit „Gesendet“ lesbar bleibt).
    let mut offen: Vec<&pgp::composed::SignedPublicKey> = vec![&pgp.eigener];
    for m in an.iter().chain(cc.iter()) {
        let k = schluessel_von(m)?;
        // Jeden Schlüssel nur einmal (z. B. bei einer Mail an sich selbst).
        if !offen.iter().any(|o| o.primary_key == k.primary_key) {
            offen.push(k);
        }
    }
    let haupt = verschluesselt_teil(schreiben::verschluesseln(&daten, &offen, signierer, false)?)?;
    let mut sendungen = Vec::new();
    if !an.is_empty() || !cc.is_empty() {
        let zum_senden = mit_teil(kopf(false), Teil::Mehrteilig(haupt.clone()))?;
        sendungen.push(Sendung {
            umschlag: zum_senden.envelope().clone(),
            daten: zum_senden.formatted(),
        });
    }
    for m in &bcc {
        let armored = schreiben::verschluesseln(&daten, &[schluessel_von(m)?], signierer, false)?;
        let umschlag = Envelope::new(Some(absender.email.clone()), vec![m.email.clone()])
            .map_err(|_| Fehlermeldung::neu("eingabe", "Ungültige Bcc-Adresse."))?;
        let nachricht = mit_teil(
            kopf(false).envelope(umschlag.clone()),
            Teil::Mehrteilig(verschluesselt_teil(armored)?),
        )?;
        sendungen.push(Sendung {
            umschlag,
            daten: nachricht.formatted(),
        });
    }
    let fuer_gesendet = mit_teil(kopf(true).keep_bcc(), Teil::Mehrteilig(haupt))?;
    Ok(GebauteNachricht {
        sendungen,
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
        assert_eq!(n.sendungen.len(), 1);
        let empfaenger: Vec<String> = n.sendungen[0]
            .umschlag
            .to()
            .iter()
            .map(|a| a.to_string())
            .collect();
        assert_eq!(
            empfaenger,
            vec![
                "bea@example.org",
                "carl@example.org",
                "dora@example.org",
                "geheim@example.org"
            ]
        );
        let gesendet = String::from_utf8_lossy(&n.sendungen[0].daten);
        assert!(
            !gesendet.contains("geheim@example.org"),
            "Bcc darf nicht im Kopf stehen"
        );
        let kopie = String::from_utf8_lossy(&n.fuer_gesendet);
        assert!(
            kopie.contains("geheim@example.org"),
            "Kopie für „Gesendet“ behält Bcc"
        );

        let m = parsen(&n.sendungen[0].daten);
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
        let roh = String::from_utf8_lossy(&n.sendungen[0].daten);
        assert!(roh.contains("In-Reply-To: <orig@example.org>"), "{roh}");
        assert!(
            roh.contains("References: <eins@example.org> <zwei@example.org> <orig@example.org>"),
            "{roh}"
        );
        let m = parsen(&n.sendungen[0].daten);
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
        let m = parsen(&n.sendungen[0].daten);
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

    mod openpgp {
        use std::collections::HashMap;

        use pgp::composed::SignedSecretKey;
        use pgp::types::KeyDetails;
        use zeroize::Zeroizing;

        use super::*;
        use crate::pgp::lesen::tests::{schluessel, PASSPHRASE};
        use crate::pgp::lesen::{verarbeiten, Schluesselring, Signatur, Verschluesselung};
        use crate::pgp::schreiben::Signierer;

        fn ring(eigen: &SignedSecretKey) -> Schluesselring {
            let s = schluessel();
            Schluesselring {
                eigene: vec![(eigen.clone(), Some(Zeroizing::new(PASSPHRASE.into())))],
                oeffentliche: [&s.anna, &s.bea, &s.carl]
                    .iter()
                    .map(|k| k.to_public_key())
                    .collect(),
            }
        }

        fn versand(verschluesseln: bool, signieren: bool) -> VersandSchluessel {
            let s = schluessel();
            VersandSchluessel {
                empfaenger: verschluesseln.then(|| {
                    HashMap::from([
                        ("bea@example.org".to_string(), s.bea.to_public_key()),
                        ("carl@example.org".to_string(), s.carl.to_public_key()),
                    ])
                }),
                eigener: s.anna.to_public_key(),
                signierer: signieren.then(|| Signierer {
                    schluessel: s.anna.clone(),
                    passphrase: Zeroizing::new(PASSPHRASE.into()),
                }),
            }
        }

        fn pgp_entwurf() -> Entwurf {
            Entwurf {
                konto: "k".into(),
                an: "Bea <bea@example.org>".into(),
                bcc: "carl@example.org".into(),
                betreff: "Vertraulich".into(),
                text: "Geheimer Text für Bea.\n".into(),
                verschluesseln: true,
                signieren: true,
                ..Default::default()
            }
        }

        fn anhang() -> Vec<AnhangDaten> {
            vec![AnhangDaten {
                name: "Vertrag.pdf".into(),
                typ: "application/pdf".into(),
                daten: b"%PDF-1.4 geheim".to_vec(),
            }]
        }

        fn enthaelt(daten: &[u8], text: &str) -> bool {
            String::from_utf8_lossy(daten).contains(text)
        }

        #[test]
        fn verschluesselt_und_signiert_mit_anhang_und_bcc() {
            let s = schluessel();
            let n = bauen_mit(
                &konto(),
                &pgp_entwurf(),
                &anhang(),
                Some(&versand(true, true)),
            )
            .unwrap();
            assert_eq!(
                n.sendungen.len(),
                2,
                "An/Cc und eine eigene Sendung für Bcc"
            );

            // Hauptsendung an Bea: verschlüsselt, ohne Bcc, ohne Klartext und Anhang.
            let haupt = &n.sendungen[0];
            let an: Vec<String> = haupt.umschlag.to().iter().map(|a| a.to_string()).collect();
            assert_eq!(an, vec!["bea@example.org"]);
            assert!(enthaelt(&haupt.daten, "multipart/encrypted"));
            assert!(!enthaelt(&haupt.daten, "Geheimer Text"));
            assert!(!enthaelt(&haupt.daten, "Vertrag.pdf"));
            assert!(!enthaelt(&haupt.daten, "carl@example.org"));
            assert!(enthaelt(&haupt.daten, "Subject: Vertraulich"));
            let ergebnis = verarbeiten(&haupt.daten, &ring(&s.bea));
            assert_eq!(
                ergebnis.status.verschluesselung,
                Verschluesselung::Entschluesselt
            );
            assert_eq!(ergebnis.status.signatur, Signatur::Gueltig);
            let lesbar = crate::anzeige::aufbereiten(&ergebnis.roh.unwrap(), None).unwrap();
            assert!(lesbar.text.contains("Geheimer Text für Bea."));
            assert_eq!(lesbar.anhaenge[0].name, "Vertrag.pdf");
            let (_, inhalt) = crate::anzeige::anhang_inhalt(
                &verarbeiten(&haupt.daten, &ring(&s.bea)).roh.unwrap(),
                0,
            )
            .unwrap();
            assert_eq!(inhalt, b"%PDF-1.4 geheim");
            // Carl (Bcc) kann die Hauptsendung nicht lesen: nicht an seinen Schlüssel verschlüsselt.
            let carl = verarbeiten(&haupt.daten, &ring(&s.carl));
            assert_eq!(
                carl.status.verschluesselung,
                Verschluesselung::NichtEntschluesselt
            );
            // Und seine Schlüssel-IDs stehen nicht darin.
            let carl_id = hex::encode_upper(s.carl.secret_subkeys[0].key.legacy_key_id().as_ref());
            assert!(!enthaelt(&haupt.daten, &carl_id));

            // Bcc-Sendung an Carl: nur für Carl lesbar.
            let bcc = &n.sendungen[1];
            let an: Vec<String> = bcc.umschlag.to().iter().map(|a| a.to_string()).collect();
            assert_eq!(an, vec!["carl@example.org"]);
            assert!(!enthaelt(&bcc.daten, "Bcc:"));
            assert_eq!(
                verarbeiten(&bcc.daten, &ring(&s.carl))
                    .status
                    .verschluesselung,
                Verschluesselung::Entschluesselt
            );
            assert_eq!(
                verarbeiten(&bcc.daten, &ring(&s.bea))
                    .status
                    .verschluesselung,
                Verschluesselung::NichtEntschluesselt
            );

            // Kopie für „Gesendet“: mit Bcc-Kopf, für Anna lesbar.
            assert!(enthaelt(&n.fuer_gesendet, "carl@example.org"));
            let eigene = verarbeiten(&n.fuer_gesendet, &ring(&s.anna));
            assert_eq!(
                eigene.status.verschluesselung,
                Verschluesselung::Entschluesselt
            );
        }

        #[test]
        fn nur_signiert_ist_lesbar_und_gueltig() {
            let s = schluessel();
            let mut e = pgp_entwurf();
            e.verschluesseln = false;
            e.bcc.clear();
            e.html = true;
            let n = bauen_mit(&konto(), &e, &anhang(), Some(&versand(false, true))).unwrap();
            assert_eq!(n.sendungen.len(), 1);
            let daten = &n.sendungen[0].daten;
            assert!(enthaelt(daten, "multipart/signed"));
            assert!(enthaelt(daten, "pgp-sha256"), "micalg fehlt");
            let ergebnis = verarbeiten(daten, &ring(&s.bea));
            assert_eq!(ergebnis.status.verschluesselung, Verschluesselung::Keine);
            assert_eq!(ergebnis.status.signatur, Signatur::Gueltig);
            let lesbar = crate::anzeige::aufbereiten(&ergebnis.roh.unwrap(), None).unwrap();
            assert!(lesbar.text.contains("Geheimer Text für Bea."));
            assert!(lesbar.html.is_some());
            assert_eq!(lesbar.anhaenge.len(), 1);
        }

        #[test]
        fn fehlender_schluessel_baut_nichts() {
            let mut e = pgp_entwurf();
            e.cc = "dora@example.org".into();
            let fehler = bauen_mit(&konto(), &e, &[], Some(&versand(true, false))).unwrap_err();
            assert_eq!(fehler.art, "pgp_schluessel_fehlt");
            assert!(fehler.meldung.contains("dora@example.org"));
        }

        #[test]
        fn nur_bcc_ergibt_nur_bcc_sendungen() {
            let s = schluessel();
            let mut e = pgp_entwurf();
            e.an.clear();
            let n = bauen_mit(&konto(), &e, &[], Some(&versand(true, false))).unwrap();
            assert_eq!(n.sendungen.len(), 1);
            assert_eq!(
                verarbeiten(&n.sendungen[0].daten, &ring(&s.carl))
                    .status
                    .verschluesselung,
                Verschluesselung::Entschluesselt
            );
            assert_eq!(
                verarbeiten(&n.fuer_gesendet, &ring(&s.anna))
                    .status
                    .verschluesselung,
                Verschluesselung::Entschluesselt
            );
        }
    }
}
