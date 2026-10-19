//! Sicheres Schreiben von Dateien im App-Datenordner.

use std::fs;
use std::io::Write;
use std::path::Path;

/// Schreibt eine Datei atomar (erst in eine temporäre Datei, dann umbenennen),
/// unter Unix nur für den eigenen Benutzer lesbar (`0600`).
pub fn atomar_schreiben(pfad: &Path, inhalt: &[u8]) -> std::io::Result<()> {
    if let Some(ordner) = pfad.parent() {
        fs::create_dir_all(ordner)?;
    }
    let mut temp_name = pfad.as_os_str().to_owned();
    temp_name.push(".neu");
    let temp = Path::new(&temp_name);
    {
        let mut optionen = fs::OpenOptions::new();
        optionen.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            optionen.mode(0o600);
        }
        let mut datei = optionen.open(temp)?;
        datei.write_all(inhalt)?;
        datei.sync_all()?;
    }
    fs::rename(temp, pfad)
}

/// Macht aus einem Dateinamen aus einer Mail einen ungefährlichen Namen
/// (keine Pfade, keine Steuerzeichen, keine reservierten Zeichen oder Namen).
///
/// Unsichtbare Zeichen und Schreibrichtungs-Steuerzeichen werden ersetzt: Mit ihnen lässt
/// sich z. B. `Rechnung<U+202E>fdp.exe` als „Rechnungexe.pdf“ anzeigen.
pub fn dateiname_saeubern(name: &str, ersatz: &str) -> String {
    let sauber: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() || unsichtbar(c) => '_',
            c => c,
        })
        .collect();
    let sauber = sauber.trim().trim_matches('.').trim();
    let sauber: String = sauber.chars().take(150).collect();
    if sauber.is_empty() {
        ersatz.to_string()
    } else if windows_reserviert(&sauber) {
        format!("_{sauber}")
    } else {
        sauber
    }
}

/// Unsichtbare Formatzeichen (Unicode-Kategorie Cf) und Schreibrichtungs-Steuerzeichen.
fn unsichtbar(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{061C}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206F}'
            | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}'
    )
}

/// Unter Windows reservierte Gerätenamen (auch mit Endung, z. B. `NUL.txt`).
fn windows_reserviert(name: &str) -> bool {
    let stamm = name
        .split('.')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_uppercase();
    matches!(stamm.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stamm.starts_with("COM") || stamm.starts_with("LPT"))
            && stamm.len() == 4
            && stamm.as_bytes()[3].is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomar_schreiben_legt_ordner_an_und_ersetzt() {
        let ordner = tempfile::tempdir().unwrap();
        let pfad = ordner.path().join("a").join("b.json");
        atomar_schreiben(&pfad, b"eins").unwrap();
        atomar_schreiben(&pfad, b"zwei").unwrap();
        assert_eq!(fs::read(&pfad).unwrap(), b"zwei");
        assert!(!ordner.path().join("a").join("b.json.neu").exists());
    }

    #[test]
    fn dateinamen_werden_gesaeubert() {
        assert_eq!(
            dateiname_saeubern("../../etc/passwd", "x"),
            "_.._etc_passwd"
        );
        assert_eq!(
            dateiname_saeubern("C:\\Windows\\a.exe", "x"),
            "C__Windows_a.exe"
        );
        assert_eq!(
            dateiname_saeubern("Rechnung\n2026.pdf", "x"),
            "Rechnung_2026.pdf"
        );
        assert_eq!(dateiname_saeubern("...", "anhang"), "anhang");
        assert_eq!(dateiname_saeubern("  Bericht.pdf ", "x"), "Bericht.pdf");
    }

    #[test]
    fn schreibrichtung_und_unsichtbare_zeichen_werden_ersetzt() {
        assert_eq!(
            dateiname_saeubern("Rechnung\u{202E}fdp.exe", "x"),
            "Rechnung_fdp.exe"
        );
        assert_eq!(
            dateiname_saeubern("a\u{200B}b\u{2067}.pdf", "x"),
            "a_b_.pdf"
        );
    }

    #[test]
    fn reservierte_windows_namen_werden_umbenannt() {
        assert_eq!(dateiname_saeubern("NUL.txt", "x"), "_NUL.txt");
        assert_eq!(dateiname_saeubern("com1", "x"), "_com1");
        assert_eq!(dateiname_saeubern("Console.txt", "x"), "Console.txt");
        assert_eq!(dateiname_saeubern("COM10.txt", "x"), "COM10.txt");
    }
}
