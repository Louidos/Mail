//! SicherMail – Backend des Mailclients.
//!
//! Module:
//! - `konto`: Konten mit eigenem IMAP-/SMTP-Server, Passwörter im Schlüsselbund
//! - `imap`: Mails empfangen (IDLE, Synchronisation über UIDs)
//! - `smtp`: Mails senden
//! - `speicher`: verschlüsselter lokaler Zwischenspeicher (SQLCipher)
//! - `anzeige`: Mails sicher zur Anzeige aufbereiten
//! - `pgp`: OpenPGP-Schlüssel verwalten, Mails entschlüsseln und Signaturen prüfen
//! - `tls`: gemeinsame TLS-Regeln (nur rustls, mindestens TLS 1.2, Zertifikate immer geprüft)

pub mod anzeige;
pub mod befehle;
pub mod dateien;
pub mod einstellungen;
pub mod fehler;
pub mod imap;
pub mod konto;
pub mod pgp;
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

/// Kennung des Hauptfensters (siehe `tauri.conf.json` und `capabilities/default.json`).
pub const HAUPTFENSTER: &str = "main";

/// Grundfarbe der dunklen Darstellung (`--grund` in `src/styles.css`), damit das Fenster
/// schon vor dem ersten Zeichnen dunkel ist.
const DUNKLER_GRUND: tauri::window::Color = tauri::window::Color(0x0f, 0x17, 0x2a, 0xff);

/// Erzeugt das Hauptfenster aus `tauri.conf.json` (dort `create: false`) mit der gespeicherten
/// Darstellung (#47): Titelleiste und Fenstergrund passen sofort, und das Startskript setzt die
/// Wahl, bevor die Seite gezeichnet wird – so blitzt nichts hell auf.
fn hauptfenster_erzeugen(
    app: &tauri::App,
    darstellung: einstellungen::Darstellung,
) -> tauri::Result<()> {
    let konfiguration = app
        .config()
        .app
        .windows
        .iter()
        .find(|f| f.label == HAUPTFENSTER)
        .cloned()
        .unwrap_or_default();
    let mut bauer = tauri::WebviewWindowBuilder::from_config(app.handle(), &konfiguration)?
        .theme(darstellung.fenster_thema());
    if let Some(skript) = darstellung.start_skript() {
        bauer = bauer.initialization_script(skript);
    }
    if darstellung == einstellungen::Darstellung::Dunkel {
        bauer = bauer.background_color(DUNKLER_GRUND);
    }
    let fenster = bauer.build()?;
    if darstellung == einstellungen::Darstellung::Automatisch
        && fenster.theme().ok() == Some(tauri::Theme::Dark)
    {
        let _ = fenster.set_background_color(Some(DUNKLER_GRUND));
    }
    Ok(())
}

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
            let gespeichert = tauri::async_runtime::block_on(einstellungen.laden());
            let lokal_speichern = gespeichert.lokal_speichern;
            hauptfenster_erzeugen(app, gespeichert.darstellung)?;
            // Verschlüsselter Speicher (Standard) oder nur Arbeitsspeicher.
            let (cache, speicher_hinweis) =
                AppZustand::speicher_einrichten(&ordner, schluesselbund.as_ref(), lokal_speichern);
            let imap = Arc::new(ImapDienst::neu(konten.clone(), cache));
            imap.vorladen_setzen(lokal_speichern && speicher_hinweis.is_none());
            let beobachter = Arc::new(Beobachter::default());
            // OpenPGP-Schlüssel: eigener verschlüsselter Speicher, unabhängig vom Mailspeicher.
            let pgp = Arc::new(pgp::PgpDienst::neu(&ordner, schluesselbund.clone()));
            let senden = Arc::new(
                smtp::dienst::SendeDienst::neu(konten.clone(), imap.clone()).mit_pgp(pgp.clone()),
            );
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
                pgp,
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
            befehle::darstellung_setzen,
            befehle::anhang_waehlen,
            befehle::mail_anhaenge_uebernehmen,
            befehle::mail_senden,
            befehle::entwurf_speichern,
            befehle::entwuerfe_auflisten,
            befehle::entwurf_loeschen,
            befehle::pgp_uebersicht,
            befehle::pgp_erzeugen,
            befehle::pgp_eigenen_importieren,
            befehle::pgp_oeffentliche_importieren,
            befehle::pgp_aus_anhang_importieren,
            befehle::pgp_exportieren,
            befehle::pgp_loeschen,
            befehle::pgp_entsperren,
            befehle::pgp_versand_pruefen,
        ])
        .run(tauri::generate_context!())
        .expect("SicherMail konnte nicht gestartet werden");
}
