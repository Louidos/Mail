//! Befehle, die das Frontend über `invoke` aufrufen kann.
//!
//! Das Frontend hat selbst keinen Zugriff auf Netzwerk oder Dateisystem;
//! alles läuft über diese Befehle.

use std::sync::Arc;

use tauri::State;

use crate::fehler::Fehlermeldung;
use crate::imap::beobachter::{Beobachter, Melder};
use crate::imap::dienst::{ImapDienst, MailSeite};
use crate::imap::ordner::Ordner;
use crate::konto::autoconfig::{self, Vorschlag};
use crate::konto::{Konto, KontoEingabe, KontoVerwaltung, Pruefergebnis};

/// Höchstens so viele Mails pro Seite.
const MAX_SEITE: usize = 200;

/// Gemeinsamer Zustand der Anwendung.
pub struct AppZustand {
    pub konten: Arc<KontoVerwaltung>,
    pub imap: Arc<ImapDienst>,
    pub beobachter: Arc<Beobachter>,
    pub melder: Melder,
}

#[tauri::command]
pub async fn konten_auflisten(zustand: State<'_, AppZustand>) -> Result<Vec<Konto>, Fehlermeldung> {
    zustand.konten.alle().await
}

#[tauri::command]
pub async fn konto_testen(
    zustand: State<'_, AppZustand>,
    eingabe: KontoEingabe,
) -> Result<Pruefergebnis, Fehlermeldung> {
    zustand.konten.testen(&eingabe).await
}

#[tauri::command]
pub async fn konto_speichern(
    zustand: State<'_, AppZustand>,
    eingabe: KontoEingabe,
) -> Result<Konto, Fehlermeldung> {
    let konto = zustand.konten.speichern(&eingabe).await?;
    // Mit den neuen Einstellungen neu verbinden.
    zustand.imap.trennen(&konto.id).await;
    zustand
        .beobachter
        .starten(&konto.id, zustand.imap.clone(), zustand.melder.clone());
    Ok(konto)
}

#[tauri::command]
pub async fn konto_loeschen(
    zustand: State<'_, AppZustand>,
    id: String,
) -> Result<(), Fehlermeldung> {
    zustand.beobachter.stoppen(&id);
    zustand.imap.trennen(&id).await;
    zustand.konten.loeschen(&id).await?;
    zustand.imap.cache().konto_entfernen(&id)
}

#[tauri::command]
pub async fn autoconfig_suchen(email: String) -> Result<Option<Vorschlag>, Fehlermeldung> {
    autoconfig::suchen(&email).await
}

#[tauri::command]
pub async fn ordner_auflisten(
    zustand: State<'_, AppZustand>,
    konto: String,
) -> Result<Vec<Ordner>, Fehlermeldung> {
    zustand.imap.ordner(&konto).await
}

#[tauri::command]
pub async fn mails_auflisten(
    zustand: State<'_, AppZustand>,
    konto: String,
    ordner: String,
    start: usize,
    anzahl: usize,
) -> Result<MailSeite, Fehlermeldung> {
    zustand
        .imap
        .mails_seite(&konto, &ordner, start, anzahl.clamp(1, MAX_SEITE))
        .await
}

#[tauri::command]
pub async fn mail_gelesen_setzen(
    zustand: State<'_, AppZustand>,
    konto: String,
    ordner: String,
    uid: u32,
    gelesen: bool,
) -> Result<(), Fehlermeldung> {
    zustand
        .imap
        .gelesen_setzen(&konto, &ordner, uid, gelesen)
        .await
}

#[tauri::command]
pub async fn mail_loeschen(
    zustand: State<'_, AppZustand>,
    konto: String,
    ordner: String,
    uid: u32,
) -> Result<(), Fehlermeldung> {
    zustand.imap.loeschen(&konto, &ordner, uid).await
}

#[tauri::command]
pub async fn mail_verschieben(
    zustand: State<'_, AppZustand>,
    konto: String,
    ordner: String,
    uid: u32,
    ziel: String,
) -> Result<(), Fehlermeldung> {
    zustand.imap.verschieben(&konto, &ordner, uid, &ziel).await
}
