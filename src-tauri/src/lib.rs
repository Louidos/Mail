//! SicherMail – Backend des Mailclients.
//!
//! Module:
//! - `konto`: Konten mit eigenem IMAP-/SMTP-Server, Passwörter im Schlüsselbund
//! - `imap`: Mails empfangen
//! - `smtp`: Mails senden
//! - `tls`: gemeinsame TLS-Regeln (nur rustls, mindestens TLS 1.2, Zertifikate immer geprüft)

pub mod befehle;
pub mod fehler;
pub mod imap;
pub mod konto;
pub mod smtp;
pub mod tls;

use std::sync::Arc;

use tauri::Manager;

use befehle::AppZustand;
use konto::schluesselbund::SystemSchluesselbund;
use konto::KontoVerwaltung;

/// Startet die Tauri-Anwendung.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let ordner = app.path().app_data_dir()?;
            std::fs::create_dir_all(&ordner)?;
            let konten = Arc::new(KontoVerwaltung::neu(
                &ordner,
                Arc::new(SystemSchluesselbund),
            ));
            app.manage(AppZustand { konten });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            befehle::konten_auflisten,
            befehle::konto_testen,
            befehle::konto_speichern,
            befehle::konto_loeschen,
            befehle::autoconfig_suchen,
        ])
        .run(tauri::generate_context!())
        .expect("SicherMail konnte nicht gestartet werden");
}
