//! SicherMail – Backend des Mailclients.
//!
//! Module:
//! - `konto`: Konten mit eigenem IMAP-/SMTP-Server, Passwörter im Schlüsselbund
//! - `imap`: Mails empfangen (IDLE, Synchronisation über UIDs)
//! - `smtp`: Mails senden
//! - `tls`: gemeinsame TLS-Regeln (nur rustls, mindestens TLS 1.2, Zertifikate immer geprüft)

pub mod anzeige;
pub mod befehle;
pub mod dateien;
pub mod einstellungen;
pub mod fehler;
pub mod imap;
pub mod konto;
pub mod smtp;
pub mod tls;

use std::sync::Arc;

use tauri::{Emitter, Manager};

use befehle::AppZustand;
use einstellungen::EinstellungsSpeicher;
use imap::beobachter::{Beobachter, Melder};
use imap::cache::SpeicherCache;
use imap::dienst::ImapDienst;
use konto::schluesselbund::SystemSchluesselbund;
use konto::KontoVerwaltung;

/// Name des Ereignisses, über das die Oberfläche von neuen Mails erfährt.
pub const MAIL_EREIGNIS: &str = "mail-ereignis";

/// Startet die Tauri-Anwendung.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Dialog und Browser werden nur vom Backend aus verwendet; das Frontend hat
        // dafür keine Rechte (siehe capabilities/default.json).
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let ordner = app.path().app_data_dir()?;
            std::fs::create_dir_all(&ordner)?;
            let konten = Arc::new(KontoVerwaltung::neu(
                &ordner,
                Arc::new(SystemSchluesselbund),
            ));
            let imap = Arc::new(ImapDienst::neu(
                konten.clone(),
                Arc::new(SpeicherCache::default()),
            ));
            let beobachter = Arc::new(Beobachter::default());
            let griff = app.handle().clone();
            let melder: Melder = Arc::new(move |ereignis| {
                let _ = griff.emit(MAIL_EREIGNIS, ereignis);
            });

            // Für alle Konten neue Mails beobachten.
            {
                let konten = konten.clone();
                let imap = imap.clone();
                let beobachter = beobachter.clone();
                let melder = melder.clone();
                tauri::async_runtime::spawn(async move {
                    for konto in konten.alle().await.unwrap_or_default() {
                        beobachter.starten(&konto.id, imap.clone(), melder.clone());
                    }
                });
            }

            app.manage(AppZustand {
                konten,
                imap,
                beobachter,
                melder,
                einstellungen: Arc::new(EinstellungsSpeicher::neu(&ordner)),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            befehle::konten_auflisten,
            befehle::konto_testen,
            befehle::konto_speichern,
            befehle::konto_loeschen,
            befehle::autoconfig_suchen,
            befehle::ordner_auflisten,
            befehle::mails_auflisten,
            befehle::mail_gelesen_setzen,
            befehle::mail_loeschen,
            befehle::mail_verschieben,
            befehle::mail_anzeigen,
            befehle::absender_extern_erlauben,
            befehle::anhang_speichern,
            befehle::link_oeffnen,
        ])
        .run(tauri::generate_context!())
        .expect("SicherMail konnte nicht gestartet werden");
}
