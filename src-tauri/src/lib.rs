//! SicherMail – Backend des Mailclients.
//!
//! Module (werden schrittweise ergänzt):
//! - `konto`: Konten mit eigenem IMAP-/SMTP-Server
//! - `imap`: Mails empfangen
//! - `smtp`: Mails senden
//! - `speicher`: verschlüsselter lokaler Zwischenspeicher
//! - `anzeige`: sichere Aufbereitung von Mails zur Anzeige

/// Startet die Tauri-Anwendung.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("SicherMail konnte nicht gestartet werden");
}
