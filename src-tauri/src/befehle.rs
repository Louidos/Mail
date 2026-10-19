//! Befehle, die das Frontend über `invoke` aufrufen kann.
//!
//! Das Frontend hat selbst keinen Zugriff auf Netzwerk oder Dateisystem;
//! alles läuft über diese Befehle.

use std::sync::Arc;

use tauri::State;

use crate::fehler::Fehlermeldung;
use crate::konto::autoconfig::{self, Vorschlag};
use crate::konto::{Konto, KontoEingabe, KontoVerwaltung, Pruefergebnis};

/// Gemeinsamer Zustand der Anwendung.
pub struct AppZustand {
    pub konten: Arc<KontoVerwaltung>,
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
    zustand.konten.speichern(&eingabe).await
}

#[tauri::command]
pub async fn konto_loeschen(
    zustand: State<'_, AppZustand>,
    id: String,
) -> Result<(), Fehlermeldung> {
    zustand.konten.loeschen(&id).await
}

#[tauri::command]
pub async fn autoconfig_suchen(email: String) -> Result<Option<Vorschlag>, Fehlermeldung> {
    autoconfig::suchen(&email).await
}
