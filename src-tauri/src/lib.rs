//! SicherMail – Backend des Mailclients.
//!
//! Module:
//! - `konto`: Konten mit eigenem IMAP-/SMTP-Server, Passwörter im Schlüsselbund
//! - `imap`: Mails empfangen (IDLE, Synchronisation über UIDs)
//! - `smtp`: Mails senden
//! - `speicher`: verschlüsselter lokaler Zwischenspeicher (SQLCipher)
//! - `anzeige`: Mails sicher zur Anzeige aufbereiten
//! - `tls`: gemeinsame TLS-Regeln (nur rustls, mindestens TLS 1.2, Zertifikate immer geprüft)

pub mod anzeige;
pub mod befehle;
pub mod dateien;
pub mod einstellungen;
pub mod fehler;
pub mod imap;
pub mod konto;
pub mod smtp;
pub mod speicher;
pub mod tls;

use std::sync::Arc;

use tauri::{Emitter, Manager};

use befehle::AppZustand;
use einstellungen::EinstellungsSpeicher;
use imap::beobachter::{Beobachter, Melder};
use imap::dienst::ImapDienst;
use konto::schluesselbund::{Schluesselbund, SystemSchluesselbund};
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
            let schluesselbund: Arc<dyn Schluesselbund> = Arc::new(SystemSchluesselbund);
            let konten = Arc::new(KontoVerwaltung::neu(&ordner, schluesselbund.clone()));
            let einstellungen = Arc::new(EinstellungsSpeicher::neu(&ordner));
            let lokal_speichern =
                tauri::async_runtime::block_on(einstellungen.laden()).lokal_speichern;
            // Verschlüsselter Speicher (Standard) oder nur Arbeitsspeicher.
            let (cache, speicher_hinweis) =
                AppZustand::speicher_einrichten(&ordner, schluesselbund.as_ref(), lokal_speichern);
            let imap = Arc::new(ImapDienst::neu(konten.clone(), cache));
            imap.vorladen_setzen(lokal_speichern && speicher_hinweis.is_none());
            let beobachter = Arc::new(Beobachter::default());
            let senden = Arc::new(smtp::dienst::SendeDienst::neu(konten.clone(), imap.clone()));
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
                einstellungen,
                senden,
                schluesselbund,
                datenordner: ordner,
                speicher_hinweis: std::sync::Mutex::new(speicher_hinweis),
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
            befehle::mails_lokal,
            befehle::mail_gelesen_setzen,
            befehle::mail_loeschen,
            befehle::mail_verschieben,
            befehle::mail_anzeigen,
            befehle::absender_extern_erlauben,
            befehle::anhang_speichern,
            befehle::link_oeffnen,
            befehle::einstellungen_laden,
            befehle::lokal_speichern_setzen,
            befehle::lokalen_speicher_leeren,
            befehle::absender_extern_entfernen,
            befehle::proxy_setzen,
            befehle::anhang_waehlen,
            befehle::mail_anhaenge_uebernehmen,
            befehle::mail_senden,
            befehle::entwurf_speichern,
            befehle::entwuerfe_auflisten,
            befehle::entwurf_loeschen,
        ])
        .run(tauri::generate_context!())
        .expect("SicherMail konnte nicht gestartet werden");
}
