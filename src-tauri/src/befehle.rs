//! Befehle, die das Frontend über `invoke` aufrufen kann.
//!
//! Das Frontend hat selbst keinen Zugriff auf Netzwerk oder Dateisystem;
//! alles läuft über diese Befehle.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use crate::anzeige::{self, extern_inhalte, AufbereiteteMail};
use crate::einstellungen::EinstellungsSpeicher;
use crate::fehler::Fehlermeldung;
use crate::imap::beobachter::{Beobachter, Melder};
use crate::imap::cache::{MailCache, SpeicherCache};
use crate::imap::dienst::{ImapDienst, MailSeite};
use crate::imap::ordner::Ordner;
use crate::konto::autoconfig::{self, Vorschlag};
use crate::konto::schluesselbund::Schluesselbund;
use crate::konto::{Konto, KontoEingabe, KontoVerwaltung, Pruefergebnis};
use crate::smtp::anhaenge::{AnhangAuswahl, Eintrag, Quelle};
use crate::smtp::dienst::{SendeDienst, SendeErgebnis};
use crate::smtp::entwurf::Entwurf;
use crate::speicher::{self, SqlCache};

/// Höchstens so viele Mails pro Seite.
const MAX_SEITE: usize = 200;

/// Gemeinsamer Zustand der Anwendung.
pub struct AppZustand {
    pub konten: Arc<KontoVerwaltung>,
    pub imap: Arc<ImapDienst>,
    pub beobachter: Arc<Beobachter>,
    pub melder: Melder,
    pub einstellungen: Arc<EinstellungsSpeicher>,
    pub senden: Arc<SendeDienst>,
    pub schluesselbund: Arc<dyn Schluesselbund>,
    pub datenordner: PathBuf,
    /// Hinweis, falls der verschlüsselte Speicher nicht geöffnet werden konnte.
    pub speicher_hinweis: Mutex<Option<String>>,
}

impl AppZustand {
    /// Richtet den Zwischenspeicher ein: verschlüsselt auf der Festplatte oder nur im
    /// Arbeitsspeicher. Gibt einen Hinweis zurück, falls der verschlüsselte Speicher
    /// nicht verfügbar ist.
    pub fn speicher_einrichten(
        datenordner: &std::path::Path,
        schluesselbund: &dyn Schluesselbund,
        lokal_speichern: bool,
    ) -> (Arc<dyn MailCache>, Option<String>) {
        if !lokal_speichern {
            return (Arc::new(SpeicherCache::default()), None);
        }
        match SqlCache::mit_schluesselbund(datenordner, schluesselbund) {
            Ok(cache) => (Arc::new(cache), None),
            Err(fehler) => (
                Arc::new(SpeicherCache::default()),
                Some(format!(
                    "Der verschlüsselte lokale Speicher ist nicht verfügbar ({}). Mails werden nur bis zum Beenden im Arbeitsspeicher gehalten.",
                    fehler.meldung
                )),
            ),
        }
    }
}

/// Einstellungen für die Oberfläche.
#[derive(Debug, Clone, Serialize)]
pub struct EinstellungenAnsicht {
    pub lokal_speichern: bool,
    pub externe_inhalte_erlaubt: Vec<String>,
    pub speicher_hinweis: Option<String>,
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

/// Mailliste sofort aus dem lokalen Speicher (danach gleicht `mails_auflisten` ab).
#[tauri::command]
pub async fn mails_lokal(
    zustand: State<'_, AppZustand>,
    konto: String,
    ordner: String,
    start: usize,
    anzahl: usize,
) -> Result<MailSeite, Fehlermeldung> {
    zustand
        .imap
        .mails_lokal(&konto, &ordner, start, anzahl.clamp(1, MAX_SEITE))
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

/// Bereitet eine Mail sicher zur Anzeige auf.
///
/// Externe Inhalte werden nur geladen, wenn der Nutzer es für diese Mail verlangt
/// (`extern_laden`) oder für den Absender dauerhaft erlaubt hat.
#[tauri::command]
pub async fn mail_anzeigen(
    zustand: State<'_, AppZustand>,
    konto: String,
    ordner: String,
    uid: u32,
    extern_laden: bool,
) -> Result<AufbereiteteMail, Fehlermeldung> {
    let roh = zustand.imap.inhalt(&konto, &ordner, uid).await?;
    let mail = anzeige::aufbereiten(&roh, None)?;
    if mail.extern_blockiert == 0 {
        return Ok(mail);
    }
    let absender = mail
        .von
        .first()
        .map(|a| a.adresse.clone())
        .unwrap_or_default();
    if !extern_laden && !zustand.einstellungen.extern_erlaubt(&absender).await {
        return Ok(mail);
    }
    let adressen = anzeige::externe_adressen(&roh)?;
    let geladen = extern_inhalte::bilder_laden(&adressen).await;
    anzeige::aufbereiten(&roh, Some(&geladen))
}

/// Erlaubt externe Inhalte für einen Absender dauerhaft.
#[tauri::command]
pub async fn absender_extern_erlauben(
    zustand: State<'_, AppZustand>,
    adresse: String,
) -> Result<(), Fehlermeldung> {
    zustand.einstellungen.extern_erlauben(&adresse).await
}

/// Speichert einen Anhang über den Dialog „Speichern unter“. Gibt den Dateinamen
/// zurück oder `None`, wenn abgebrochen wurde. Anhänge werden nie geöffnet.
#[tauri::command]
pub async fn anhang_speichern(
    app: AppHandle,
    zustand: State<'_, AppZustand>,
    konto: String,
    ordner: String,
    uid: u32,
    index: usize,
) -> Result<Option<String>, Fehlermeldung> {
    let roh = zustand.imap.inhalt(&konto, &ordner, uid).await?;
    let (name, daten) = anzeige::anhang_inhalt(&roh, index)?;
    let (sender, empfaenger) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Anhang speichern unter …")
        .set_file_name(&name)
        .save_file(move |pfad| {
            let _ = sender.send(pfad);
        });
    let Some(pfad) = empfaenger.await.ok().flatten() else {
        return Ok(None);
    };
    let pfad = pfad
        .into_path()
        .map_err(|_| Fehlermeldung::neu("datei", "Ungültiger Speicherort."))?;
    tokio::fs::write(&pfad, &daten)
        .await
        .map_err(|_| Fehlermeldung::neu("datei", "Der Anhang konnte nicht gespeichert werden."))?;
    Ok(Some(
        pfad.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or(name),
    ))
}

/// Prüft, ob eine Adresse im Standardbrowser bzw. Mailprogramm geöffnet werden darf.
pub fn link_erlaubt(url: &str) -> bool {
    let klein = url.to_ascii_lowercase();
    url.len() <= 8192
        && !url.chars().any(|c| c.is_control() || c.is_whitespace())
        && (klein.starts_with("https://")
            || klein.starts_with("http://")
            || klein.starts_with("mailto:"))
}

/// Öffnet einen Link im Standardbrowser (nur http, https und mailto).
#[tauri::command]
pub async fn link_oeffnen(app: AppHandle, url: String) -> Result<(), Fehlermeldung> {
    if !link_erlaubt(&url) {
        return Err(Fehlermeldung::neu(
            "eingabe",
            "Dieser Link wird aus Sicherheitsgründen nicht geöffnet.",
        ));
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|_| Fehlermeldung::neu("link", "Der Link konnte nicht geöffnet werden."))
}

#[tauri::command]
pub async fn einstellungen_laden(
    zustand: State<'_, AppZustand>,
) -> Result<EinstellungenAnsicht, Fehlermeldung> {
    let einstellungen = zustand.einstellungen.laden().await;
    Ok(EinstellungenAnsicht {
        lokal_speichern: einstellungen.lokal_speichern,
        externe_inhalte_erlaubt: einstellungen.externe_inhalte_erlaubt,
        speicher_hinweis: zustand.speicher_hinweis.lock().ok().and_then(|h| h.clone()),
    })
}

/// Schaltet „Mails lokal speichern“ um. Beim Ausschalten wird der lokale Speicher
/// geleert und die Datei gelöscht.
#[tauri::command]
pub async fn lokal_speichern_setzen(
    zustand: State<'_, AppZustand>,
    an: bool,
) -> Result<(), Fehlermeldung> {
    zustand
        .einstellungen
        .aendern(|e| e.lokal_speichern = an)
        .await?;
    let alt = zustand.imap.cache();
    let (neu, hinweis) =
        AppZustand::speicher_einrichten(&zustand.datenordner, zustand.schluesselbund.as_ref(), an);
    // Entwürfe sind Texte des Nutzers: in den neuen Speicher übernehmen (älteste zuerst).
    for konto in zustand.konten.alle().await? {
        for (id, daten) in alt.entwuerfe(&konto.id)?.into_iter().rev() {
            neu.entwurf_speichern(&id, &konto.id, &daten)?;
        }
    }
    zustand.imap.cache_ersetzen(neu);
    zustand.imap.vorladen_setzen(an && hinweis.is_none());
    if let Ok(mut h) = zustand.speicher_hinweis.lock() {
        *h = hinweis;
    }
    if !an {
        alt.leeren()?;
        drop(alt);
        let _ = std::fs::remove_file(zustand.datenordner.join(speicher::DATEINAME));
        zustand
            .schluesselbund
            .loeschen(speicher::SCHLUESSEL_EINTRAG)?;
    }
    Ok(())
}

/// Entfernt alle zwischengespeicherten Mails (lokal; auf dem Server bleibt alles).
#[tauri::command]
pub async fn lokalen_speicher_leeren(zustand: State<'_, AppZustand>) -> Result<(), Fehlermeldung> {
    zustand.imap.cache().leeren()
}

#[tauri::command]
pub async fn absender_extern_entfernen(
    zustand: State<'_, AppZustand>,
    adresse: String,
) -> Result<(), Fehlermeldung> {
    zustand.einstellungen.extern_entfernen(&adresse).await
}

/// Öffnet den Dialog „Datei anhängen“ (im Backend) und trägt die Dateien ins Register ein.
#[tauri::command]
pub async fn anhang_waehlen(
    app: AppHandle,
    zustand: State<'_, AppZustand>,
) -> Result<Vec<AnhangAuswahl>, Fehlermeldung> {
    let (sender, empfaenger) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Dateien anhängen")
        .pick_files(move |dateien| {
            let _ = sender.send(dateien);
        });
    let Some(dateien) = empfaenger.await.ok().flatten() else {
        return Ok(Vec::new());
    };
    dateien
        .into_iter()
        .map(|datei| {
            let pfad = datei
                .into_path()
                .map_err(|_| Fehlermeldung::neu("anhang", "Ungültige Datei."))?;
            zustand.senden.register.datei_eintragen(pfad)
        })
        .collect()
}

/// Übernimmt die Anhänge einer Mail (zum Weiterleiten).
#[tauri::command]
pub async fn mail_anhaenge_uebernehmen(
    zustand: State<'_, AppZustand>,
    konto: String,
    ordner: String,
    uid: u32,
) -> Result<Vec<AnhangAuswahl>, Fehlermeldung> {
    let roh = zustand.imap.inhalt(&konto, &ordner, uid).await?;
    let nachricht = anzeige::parsen(&roh)?;
    Ok(anzeige::anhaenge(&nachricht)
        .into_iter()
        .map(|a| {
            zustand.senden.register.eintragen(Eintrag {
                name: a.name,
                typ: a.typ,
                groesse: a.groesse as u64,
                quelle: Quelle::Mail {
                    konto: konto.clone(),
                    ordner: ordner.clone(),
                    uid,
                    index: a.index,
                },
            })
        })
        .collect())
}

/// Sendet eine Mail. Bei einem Fehler wird sie als Entwurf gespeichert.
#[tauri::command]
pub async fn mail_senden(
    zustand: State<'_, AppZustand>,
    entwurf: Entwurf,
) -> Result<SendeErgebnis, Fehlermeldung> {
    zustand.senden.senden(&entwurf).await
}

#[tauri::command]
pub async fn entwurf_speichern(
    zustand: State<'_, AppZustand>,
    entwurf: Entwurf,
) -> Result<Entwurf, Fehlermeldung> {
    zustand.senden.entwurf_speichern(&entwurf).await
}

#[tauri::command]
pub async fn entwuerfe_auflisten(
    zustand: State<'_, AppZustand>,
    konto: String,
) -> Result<Vec<Entwurf>, Fehlermeldung> {
    zustand.senden.entwuerfe(&konto)
}

#[tauri::command]
pub async fn entwurf_loeschen(
    zustand: State<'_, AppZustand>,
    id: String,
) -> Result<(), Fehlermeldung> {
    zustand.senden.entwurf_loeschen(&id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nur_sichere_links_werden_geoeffnet() {
        assert!(link_erlaubt("https://example.org/pfad?x=1"));
        assert!(link_erlaubt("http://example.org"));
        assert!(link_erlaubt("mailto:anna@example.org"));
        assert!(!link_erlaubt("javascript:alert(1)"));
        assert!(!link_erlaubt("file:///etc/passwd"));
        assert!(!link_erlaubt("data:text/html,<script>"));
        assert!(!link_erlaubt("smb://server/freigabe"));
        assert!(!link_erlaubt("https://example.org/\nboese"));
        assert!(!link_erlaubt("https://example.org/ leer"));
    }
}
