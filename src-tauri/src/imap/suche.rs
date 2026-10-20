//! Suche auf dem Server (#49) mit `UID SEARCH`.
//!
//! Nur auf ausdrücklichen Wunsch und nur über die bestehende verschlüsselte Verbindung des
//! Kontos. Suchbegriffe werden nie roh in den Befehl kopiert: Anführungszeichen trennen Wörter
//! (wie bei der lokalen Suche). Wörter aus druckbarem ASCII ohne `\` gehen als IMAP-String in
//! Anführungszeichen, alle anderen (z. B. mit Umlauten) als Literal `{n}` (RFC 3501,
//! Abschnitt 4.3), bei Nicht-ASCII mit `CHARSET UTF-8`. Ein Literal wird nicht maskiert, sondern
//! in genau der angekündigten Länge gesendet. Steuerzeichen wie CR und LF sind vorher entfernt,
//! sie können also keinen zweiten Befehl einschleusen.

use async_imap::imap_proto::{MailboxDatum, Response, Status};

use super::{fehler_einordnen, ImapSitzung};
use crate::fehler::VerbindungsFehler;
use crate::speicher::suche::{woerter, SuchFeld};

/// Höchstens so viele Treffer zeigt die Server-Suche (die neuesten).
pub const MAX_SERVER_TREFFER: usize = 200;

/// Teil eines Suchbefehls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Teil {
    /// Wird unverändert gesendet (Schlüsselwörter und maskierte Zeichenketten).
    Text(String),
    /// Wird als Literal gesendet: `{Länge}`, warten auf `+`, dann die Bytes.
    Literal(String),
}

/// Druckbares ASCII ohne `"` und `\`: darf ohne Maskierung in einen IMAP-String in
/// Anführungszeichen. (Maskierte Zeichen lehnen manche Server ab; solche Wörter gehen als Literal.
/// `"` kommt nicht vor, es trennt Wörter – manche Server lehnen es sogar in Literalen ab.)
fn ist_einfach(wort: &str) -> bool {
    wort.bytes()
        .all(|b| (0x20..0x7f).contains(&b) && b != b'"' && b != b'\\')
}

/// Baut `UID SEARCH` für einen Suchbegriff, oder `None`, wenn nichts Suchbares übrig bleibt.
///
/// Alle Wörter müssen vorkommen. `Alle` sucht in Absender, Betreff und Inhalt (`BODY`).
/// Gelöschte (`\Deleted`) Mails zählen nicht.
pub fn suchbefehl(begriff: &str, feld: SuchFeld) -> Option<Vec<Teil>> {
    let woerter = woerter(&begriff.replace('"', " "));
    if woerter.is_empty() {
        return None;
    }
    let mit_utf8 = woerter.iter().any(|w| !w.is_ascii());
    let mut teile = vec![Teil::Text(if mit_utf8 {
        "UID SEARCH CHARSET UTF-8".into()
    } else {
        "UID SEARCH".into()
    })];
    let text = |t: &str, teile: &mut Vec<Teil>| match teile.last_mut() {
        Some(Teil::Text(vorher)) => vorher.push_str(t),
        _ => teile.push(Teil::Text(t.into())),
    };
    for wort in &woerter {
        let schluessel: &[&str] = match feld {
            SuchFeld::Alle => &["OR FROM", "OR SUBJECT", "BODY"],
            SuchFeld::Absender => &["FROM"],
            SuchFeld::Betreff => &["SUBJECT"],
            SuchFeld::Inhalt => &["BODY"],
        };
        for s in schluessel {
            text(&format!(" {s} "), &mut teile);
            if ist_einfach(wort) {
                text(&format!("\"{wort}\""), &mut teile);
            } else {
                teile.push(Teil::Literal(wort.clone()));
            }
        }
    }
    text(" UNDELETED", &mut teile);
    Some(teile)
}

/// Zeilen, wie sie gesendet werden: Vor jedem Literal endet eine Zeile mit `{Länge}`; die
/// nächste beginnt mit den Bytes des Literals.
pub fn zeilen(teile: &[Teil]) -> Vec<String> {
    let mut zeilen = Vec::new();
    let mut aktuell = String::new();
    for teil in teile {
        match teil {
            Teil::Text(t) => aktuell.push_str(t),
            Teil::Literal(l) => {
                aktuell.push_str(&format!("{{{}}}", l.len()));
                zeilen.push(std::mem::take(&mut aktuell));
                aktuell.push_str(l);
            }
        }
    }
    zeilen.push(aktuell);
    zeilen
}

fn abgelehnt() -> VerbindungsFehler {
    VerbindungsFehler::Protokoll("Der IMAP-Server hat die Suche abgelehnt.".into())
}

/// Führt die Suche im ausgewählten Ordner aus und gibt die gefundenen UIDs zurück.
pub async fn uid_suchen(
    sitzung: &mut ImapSitzung,
    teile: &[Teil],
) -> Result<Vec<u32>, VerbindungsFehler> {
    let zeilen = zeilen(teile);
    let Some((erste, weitere)) = zeilen.split_first() else {
        return Ok(Vec::new());
    };
    let kennung = sitzung.run_command(erste).await.map_err(fehler_einordnen)?;
    for zeile in weitere {
        // Vor jedem Literal auf die Aufforderung „+“ des Servers warten.
        loop {
            let antwort = sitzung
                .read_response()
                .await
                .map_err(|_| VerbindungsFehler::Unterbrochen)?
                .ok_or(VerbindungsFehler::Unterbrochen)?;
            match antwort.parsed() {
                Response::Continue { .. } => break,
                Response::Done { tag, .. } if tag == &kennung => return Err(abgelehnt()),
                _ => {}
            }
        }
        sitzung
            .run_command_untagged(zeile)
            .await
            .map_err(fehler_einordnen)?;
    }
    let mut uids = Vec::new();
    loop {
        let antwort = sitzung
            .read_response()
            .await
            .map_err(|_| VerbindungsFehler::Unterbrochen)?
            .ok_or(VerbindungsFehler::Unterbrochen)?;
        match antwort.parsed() {
            Response::MailboxData(MailboxDatum::Search(liste)) => uids.extend(liste.iter()),
            Response::Done { tag, status, .. } if tag == &kennung => {
                return match status {
                    Status::Ok => {
                        uids.retain(|&uid| uid != 0);
                        uids.sort_unstable();
                        uids.dedup();
                        Ok(uids)
                    }
                    _ => Err(abgelehnt()),
                };
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gesendet(begriff: &str, feld: SuchFeld) -> String {
        zeilen(&suchbefehl(begriff, feld).unwrap()).join("\r\n")
    }

    #[test]
    fn ascii_als_maskierter_string() {
        assert_eq!(
            gesendet("Rechnung", SuchFeld::Betreff),
            "UID SEARCH SUBJECT \"Rechnung\" UNDELETED"
        );
        assert_eq!(
            gesendet("a\"b\\c", SuchFeld::Absender),
            "UID SEARCH FROM \"a\" FROM {3}\r\nb\\c UNDELETED"
        );
        assert_eq!(
            gesendet("x y", SuchFeld::Inhalt),
            "UID SEARCH BODY \"x\" BODY \"y\" UNDELETED"
        );
        assert_eq!(
            gesendet("velo", SuchFeld::Alle),
            "UID SEARCH OR FROM \"velo\" OR SUBJECT \"velo\" BODY \"velo\" UNDELETED"
        );
    }

    #[test]
    fn umlaute_als_literal_mit_utf8() {
        assert_eq!(
            gesendet("Zürich", SuchFeld::Betreff),
            "UID SEARCH CHARSET UTF-8 SUBJECT {7}\r\nZürich UNDELETED"
        );
        assert_eq!(
            gesendet("Grüezi Velo", SuchFeld::Alle),
            "UID SEARCH CHARSET UTF-8 OR FROM {7}\r\nGrüezi OR SUBJECT {7}\r\nGrüezi BODY {7}\r\nGrüezi OR FROM \"Velo\" OR SUBJECT \"Velo\" BODY \"Velo\" UNDELETED"
        );
    }

    #[test]
    fn keine_befehlsinjektion() {
        // CR/LF und andere Steuerzeichen werden entfernt, nichts kann einen neuen Befehl beginnen.
        let befehl = gesendet("x\r\nA1 DELETE INBOX", SuchFeld::Betreff);
        assert_eq!(
            befehl,
            "UID SEARCH SUBJECT \"x\" SUBJECT \"A1\" SUBJECT \"DELETE\" SUBJECT \"INBOX\" UNDELETED"
        );
        let befehl = gesendet("ä\r\n} {5}\u{0}", SuchFeld::Inhalt);
        assert_eq!(
            befehl,
            "UID SEARCH CHARSET UTF-8 BODY {2}\r\nä BODY \"{5}\" UNDELETED"
        );
        // Literale sind genau so lang wie angekündigt (Bytes, nicht Zeichen).
        for teil in suchbefehl("Öl ½ 😀x", SuchFeld::Inhalt).unwrap() {
            if let Teil::Literal(l) = teil {
                assert!(!l.contains(['\r', '\n', '\0']));
            }
        }
        assert_eq!(suchbefehl("\" * \\", SuchFeld::Alle), None);
        assert_eq!(suchbefehl("", SuchFeld::Alle), None);
    }
}
