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
/// (keine Pfade, keine Steuerzeichen, keine reservierten Zeichen).
pub fn dateiname_saeubern(name: &str, ersatz: &str) -> String {
    let sauber: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let sauber = sauber.trim().trim_matches('.').trim();
    let sauber: String = sauber.chars().take(150).collect();
    if sauber.is_empty() {
        ersatz.to_string()
    } else {
        sauber
    }
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
}
