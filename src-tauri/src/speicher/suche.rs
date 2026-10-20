//! Suche in Mails (#49): Suchbegriffe zerlegen, sicher in FTS5-Ausdrücke übersetzen und den
//! Text bestimmen, der in den Suchindex kommt.
//!
//! - Der Index liegt als FTS5-Tabelle `mails_suche` in derselben SQLCipher-Datenbank wie die
//!   Mails (`mails.db`); es gibt keine zweite Datei und keine fremde Suchbibliothek.
//! - Indiziert werden Absender (Name und Adresse), Betreff und der Textinhalt. Bei Ende-zu-Ende
//!   verschlüsselten Mails (OpenPGP, S/MIME) nur die Kopfdaten – entschlüsselter Klartext wird
//!   nie gespeichert.
//! - Eingaben des Nutzers werden nie direkt als FTS5-Syntax oder SQL verwendet: Jedes Wort wird
//!   als Zeichenkette in Anführungszeichen gesetzt (`"wort"*`), die SQL-Werte sind Parameter.

use mail_parser::{MessageParser, MimeHeaders};
use serde::{Deserialize, Serialize};

use crate::imap::cache::MailKopf;

/// Höchstens so viele Wörter eines Suchbegriffs werden berücksichtigt.
pub const MAX_WOERTER: usize = 10;

/// Höchstens so viele Zeichen pro Wort.
const MAX_WORTLAENGE: usize = 100;

/// Höchstens so viele Zeichen Textinhalt pro Mail kommen in den Index.
pub const MAX_TEXT: usize = 100_000;

/// In welchem Teil der Mail gesucht wird.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuchFeld {
    /// Absender, Betreff und Inhalt.
    #[default]
    Alle,
    /// Name und Adresse des Absenders.
    Absender,
    Betreff,
    /// Textinhalt (bei Ende-zu-Ende verschlüsselten Mails nicht indiziert).
    Inhalt,
}

impl SuchFeld {
    /// Spalte der FTS5-Tabelle, oder `None` für alle Spalten.
    fn spalte(self) -> Option<&'static str> {
        match self {
            Self::Alle => None,
            Self::Absender => Some("absender"),
            Self::Betreff => Some("betreff"),
            Self::Inhalt => Some("text"),
        }
    }
}

/// Wo gesucht wird: ein Konto, optional nur ein Ordner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuchOrt {
    pub konto: String,
    /// `None` = alle Ordner des Kontos.
    pub ordner: Option<String>,
}

/// Eine lokale Suche.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suche {
    pub begriff: String,
    pub feld: SuchFeld,
    pub orte: Vec<SuchOrt>,
}

impl Suche {
    /// Liegt die Mail im Suchbereich?
    pub fn umfasst(&self, konto: &str, ordner: &str) -> bool {
        self.orte
            .iter()
            .any(|o| o.konto == konto && o.ordner.as_deref().is_none_or(|x| x == ordner))
    }
}

/// Ein Treffer bzw. eine Mail mit ihrem Konto und Ordner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MailEintrag {
    pub konto: String,
    pub ordner: String,
    #[serde(flatten)]
    pub kopf: MailKopf,
}

/// Eine Seite mit Suchtreffern.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SuchSeite {
    pub mails: Vec<MailEintrag>,
    /// Lokal: Es gibt weitere Treffer („Mehr laden“). Server: Es gab mehr Treffer als angezeigt.
    pub weitere: bool,
}

/// Reihenfolge der Trefferliste: neueste zuerst, Mails ohne Datum am Ende.
pub fn nach_datum(a: &MailEintrag, b: &MailEintrag) -> std::cmp::Ordering {
    let schluessel = |m: &MailEintrag| (m.kopf.datum.is_some(), m.kopf.datum, m.kopf.uid);
    schluessel(b).cmp(&schluessel(a))
}

/// Zerlegt einen Suchbegriff in Wörter: an Leerraum getrennt, ohne Steuerzeichen, nur Wörter
/// mit mindestens einem Buchstaben oder einer Ziffer, höchstens [`MAX_WOERTER`].
pub fn woerter(begriff: &str) -> Vec<String> {
    begriff
        .split_whitespace()
        .map(|wort| {
            wort.chars()
                .filter(|z| !z.is_control())
                .take(MAX_WORTLAENGE)
                .collect::<String>()
        })
        .filter(|wort| wort.chars().any(char::is_alphanumeric))
        .take(MAX_WOERTER)
        .collect()
}

/// FTS5-Ausdruck für einen Suchbegriff, oder `None`, wenn nichts Suchbares übrig bleibt.
///
/// Jedes Wort wird zu einer Zeichenkette in Anführungszeichen (innere `"` verdoppelt) mit `*`
/// für die Suche nach Wortanfängen. Operatoren wie `AND`, `NEAR`, `*` oder `spalte:` in der
/// Eingabe sind damit nur Text. Alle Wörter müssen vorkommen.
pub fn fts_ausdruck(begriff: &str, feld: SuchFeld) -> Option<String> {
    let teile: Vec<String> = woerter(begriff)
        .iter()
        .map(|wort| format!("\"{}\"*", wort.replace('"', "\"\"")))
        .collect();
    if teile.is_empty() {
        return None;
    }
    let ausdruck = teile.join(" ");
    Some(match feld.spalte() {
        None => ausdruck,
        Some(spalte) => format!("{{{spalte}}} : ({ausdruck})"),
    })
}

/// Absender für den Index: Name und Adresse.
pub fn absender_text(kopf: &MailKopf) -> String {
    if kopf.absender == kopf.absender_adresse {
        kopf.absender.clone()
    } else {
        format!("{} {}", kopf.absender, kopf.absender_adresse)
    }
}

/// Inhaltstypen, die Ende-zu-Ende verschlüsselte Inhalte tragen (OpenPGP, S/MIME).
fn ist_verschluesselt_typ(typ: &str) -> bool {
    matches!(
        typ,
        "multipart/encrypted"
            | "application/pgp-encrypted"
            | "application/pkcs7-mime"
            | "application/x-pkcs7-mime"
    )
}

/// Text der Mail für den Suchindex (höchstens [`MAX_TEXT`] Zeichen).
///
/// `None` bei Ende-zu-Ende verschlüsselten Mails – auch wenn nur ein Teil verschlüsselt ist –
/// und bei Mails, die sich nicht lesen lassen. Dann kommen nur die Kopfdaten in den Index.
pub fn suchtext(roh: &[u8]) -> Option<String> {
    let nachricht = MessageParser::default().parse(roh)?;
    let verschluesselt = nachricht.parts.iter().any(|teil| {
        teil.content_type().is_some_and(|typ| {
            let voll = format!("{}/{}", typ.ctype(), typ.subtype().unwrap_or_default())
                .to_ascii_lowercase();
            ist_verschluesselt_typ(&voll)
        })
    });
    if verschluesselt {
        return None;
    }
    let text = nachricht.body_text(0)?;
    // Inline-PGP: verschlüsselter Block im gewöhnlichen Text.
    if text.contains("-----BEGIN PGP MESSAGE-----") {
        return None;
    }
    Some(text.chars().take(MAX_TEXT).collect())
}

/// Einfache Suche ohne Index (für den Zwischenspeicher im Arbeitsspeicher): Jedes Wort muss –
/// ohne Rücksicht auf Gross- und Kleinschreibung – im gewählten Feld vorkommen.
pub fn passt(
    begriff: &str,
    feld: SuchFeld,
    kopf: &MailKopf,
    text: impl FnOnce() -> Option<String>,
) -> bool {
    let woerter: Vec<String> = woerter(begriff).iter().map(|w| w.to_lowercase()).collect();
    if woerter.is_empty() {
        return false;
    }
    let absender = absender_text(kopf).to_lowercase();
    let betreff = kopf.betreff.to_lowercase();
    let inhalt = match feld {
        SuchFeld::Alle | SuchFeld::Inhalt => text().unwrap_or_default().to_lowercase(),
        _ => String::new(),
    };
    woerter.iter().all(|wort| match feld {
        SuchFeld::Alle => {
            absender.contains(wort.as_str())
                || betreff.contains(wort.as_str())
                || inhalt.contains(wort.as_str())
        }
        SuchFeld::Absender => absender.contains(wort.as_str()),
        SuchFeld::Betreff => betreff.contains(wort.as_str()),
        SuchFeld::Inhalt => inhalt.contains(wort.as_str()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn woerter_ohne_steuerzeichen_und_satzzeichen() {
        assert_eq!(woerter("  Rechnung   März "), vec!["Rechnung", "März"]);
        assert_eq!(woerter("\" * \\ -- ()"), Vec::<String>::new());
        assert_eq!(woerter("a\u{0}b\r\nc"), vec!["ab", "c"]);
        assert_eq!(woerter(&"x ".repeat(50)).len(), MAX_WOERTER);
        assert_eq!(woerter(&"y".repeat(500))[0].chars().count(), 100);
    }

    #[test]
    fn fts_ausdruck_ist_immer_nur_text() {
        assert_eq!(
            fts_ausdruck("Rechnung", SuchFeld::Alle).unwrap(),
            "\"Rechnung\"*"
        );
        assert_eq!(
            fts_ausdruck("Grüezi Zürich", SuchFeld::Betreff).unwrap(),
            "{betreff} : (\"Grüezi\"* \"Zürich\"*)"
        );
        assert_eq!(
            fts_ausdruck("say\"hi", SuchFeld::Absender).unwrap(),
            "{absender} : (\"say\"\"hi\"*)"
        );
        assert_eq!(
            fts_ausdruck("a OR b NOT c", SuchFeld::Inhalt).unwrap(),
            "{text} : (\"a\"* \"OR\"* \"b\"* \"NOT\"* \"c\"*)"
        );
        assert_eq!(fts_ausdruck("\"*\\", SuchFeld::Alle), None);
        assert_eq!(fts_ausdruck("   ", SuchFeld::Alle), None);
    }

    fn kopf() -> MailKopf {
        MailKopf {
            uid: 1,
            betreff: "Offerte für Zürich".into(),
            absender: "Jürg Müller".into(),
            absender_adresse: "juerg@example.org".into(),
            datum: None,
            gelesen: false,
            markiert: false,
            groesse: 1,
        }
    }

    #[test]
    fn einfache_suche_nach_feldern() {
        let text = || Some("Der Text mit Velo".to_string());
        assert!(passt("müller", SuchFeld::Absender, &kopf(), text));
        assert!(passt("EXAMPLE.org", SuchFeld::Absender, &kopf(), text));
        assert!(!passt("Offerte", SuchFeld::Absender, &kopf(), text));
        assert!(passt("zürich offerte", SuchFeld::Betreff, &kopf(), text));
        assert!(passt("velo", SuchFeld::Inhalt, &kopf(), text));
        assert!(!passt("velo", SuchFeld::Betreff, &kopf(), text));
        assert!(passt("velo jürg", SuchFeld::Alle, &kopf(), text));
        assert!(!passt("\"*", SuchFeld::Alle, &kopf(), text));
    }

    #[test]
    fn suchtext_aus_normaler_mail() {
        let roh = b"From: a@example.org\r\nSubject: Hallo\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nGr\xc3\xbcezi mitenand\r\n";
        assert_eq!(suchtext(roh).unwrap().trim(), "Grüezi mitenand");
    }

    #[test]
    fn suchtext_aus_html_mail() {
        let roh = b"From: a@example.org\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>Nur <b>HTML</b></p>\r\n";
        assert!(suchtext(roh).unwrap().contains("HTML"));
    }

    #[test]
    fn verschluesselte_inhalte_kommen_nicht_in_den_index() {
        let pgp_mime = b"From: a@example.org\r\nSubject: Geheim\r\nContent-Type: multipart/encrypted; protocol=\"application/pgp-encrypted\"; boundary=\"g\"\r\n\r\n--g\r\nContent-Type: application/pgp-encrypted\r\n\r\nVersion: 1\r\n--g\r\nContent-Type: application/octet-stream\r\n\r\n-----BEGIN PGP MESSAGE-----\r\nhQEMA\r\n-----END PGP MESSAGE-----\r\n--g--\r\n";
        assert_eq!(suchtext(pgp_mime), None);
        let inline = b"From: a@example.org\r\nContent-Type: text/plain\r\n\r\n-----BEGIN PGP MESSAGE-----\r\nhQEMA\r\n-----END PGP MESSAGE-----\r\n";
        assert_eq!(suchtext(inline), None);
        let smime = b"From: a@example.org\r\nContent-Type: application/pkcs7-mime; smime-type=enveloped-data; name=smime.p7m\r\nContent-Transfer-Encoding: base64\r\n\r\nMIAGCSqGSIb3DQEHA6CAMIACAQA=\r\n";
        assert_eq!(suchtext(smime), None);
        let x_smime = b"From: a@example.org\r\nContent-Type: application/x-pkcs7-mime; smime-type=enveloped-data\r\n\r\nMIAG\r\n";
        assert_eq!(suchtext(x_smime), None);
        // Nur ein Teil verschlüsselt: auch dann kein Inhalt im Index.
        let teilweise = b"From: a@example.org\r\nContent-Type: multipart/mixed; boundary=\"m\"\r\n\r\n--m\r\nContent-Type: text/plain\r\n\r\nSichtbar\r\n--m\r\nContent-Type: application/pkcs7-mime; smime-type=enveloped-data\r\n\r\nMIAG\r\n--m--\r\n";
        assert_eq!(suchtext(teilweise), None);
    }

    #[test]
    fn suchtext_ist_begrenzt() {
        let mut roh = b"From: a@example.org\r\nContent-Type: text/plain\r\n\r\n".to_vec();
        roh.extend(std::iter::repeat_n(b'a', MAX_TEXT + 500));
        assert_eq!(suchtext(&roh).unwrap().chars().count(), MAX_TEXT);
    }
}
