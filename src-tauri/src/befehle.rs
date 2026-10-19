//! Befehle, die das Frontend über `invoke` aufrufen kann.
//!
//! Das Frontend hat selbst keinen Zugriff auf Netzwerk oder Dateisystem;
//! alles läuft über diese Befehle.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;
use zeroize::Zeroizing;

use crate::anzeige::{self, extern_inhalte, AufbereiteteMail};
use crate::einstellungen::{Darstellung, EinstellungsSpeicher};
use crate::fehler::Fehlermeldung;
use crate::imap::beobachter::{Beobachter, Melder};
use crate::imap::cache::{MailCache, SpeicherCache};
use crate::imap::dienst::{ImapDienst, MailSeite};
use crate::imap::ordner::Ordner;
use crate::konto::autoconfig::{self, Vorschlag};
use crate::konto::schluesselbund::Schluesselbund;
use crate::konto::{Konto, KontoEingabe, KontoVerwaltung, Pruefergebnis};
use crate::pgp::lesen::{Verarbeitet, Verschluesselung};
use crate::pgp::schluessel::SchluesselInfo;
use crate::pgp::{PgpDienst, Uebersicht, VersandPruefung};
use crate::smime::zertifikat::{ZertifikatInfo, MAX_ZERTIFIKATSDATEI};
use crate::smime::SmimeDienst;
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
    pub pgp: Arc<PgpDienst>,
    pub smime: Arc<SmimeDienst>,
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
    pub proxy_externe_bilder: bool,
    pub proxy_adresse: String,
    pub speicher_hinweis: Option<String>,
    pub darstellung: Darstellung,
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
    if let Err(fehler) = zustand.konten.loeschen(&id).await {
        // Das Konto besteht weiter (z. B. Schlüsselbund gesperrt): weiter auf neue Mails achten.
        if zustand.konten.finden(&id).await.is_ok() {
            zustand
                .beobachter
                .starten(&id, zustand.imap.clone(), zustand.melder.clone());
        }
        return Err(fehler);
    }
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
/// (`extern_laden`) oder für den Absender dauerhaft erlaubt hat. Die dauerhafte Freigabe gilt
/// nicht, wenn der eigene Server den Absender als wahrscheinlich gefälscht erkannt hat (#45).
#[tauri::command]
pub async fn mail_anzeigen(
    zustand: State<'_, AppZustand>,
    konto: String,
    ordner: String,
    uid: u32,
    extern_laden: bool,
) -> Result<AufbereiteteMail, Fehlermeldung> {
    let kennung = zustand
        .konten
        .finden(&konto)
        .await?
        .absender_kennung()
        .to_string();
    let (roh, verarbeitet) = anzeige_inhalt(&zustand, &konto, &ordner, uid).await?;
    // Die Absenderprüfung gilt der äusseren Mail, wie sie der eigene Server empfangen hat.
    let pruefung = anzeige::absender_pruefen(&roh, &kennung)?;
    // Verschlüsselte Mails werden entschlüsselt angezeigt; der Klartext bleibt im Speicher.
    let anzeige_roh: &[u8] = verarbeitet.roh.as_deref().map_or(&roh[..], |r| &r[..]);
    let mut mail = anzeige::aufbereiten(anzeige_roh, None)?;
    mail.absender_pruefung = pruefung.clone();
    mail.pgp = verarbeitet.status.clone();
    if mail.extern_blockiert == 0 {
        return Ok(mail);
    }
    if !extern_laden && !zustand.einstellungen.extern_erlaubt_fuer(&mail).await {
        return Ok(mail);
    }
    // Ein eingeschalteter, aber ungültiger Proxy lädt nichts: Bilder bleiben blockiert.
    let Ok(proxy) = zustand.einstellungen.bilder_proxy().await else {
        return Ok(mail);
    };
    let adressen = anzeige::externe_adressen(anzeige_roh)?;
    let geladen = extern_inhalte::bilder_laden(&adressen, proxy.as_ref()).await;
    let mut mail = anzeige::aufbereiten(anzeige_roh, Some(&geladen))?;
    mail.absender_pruefung = pruefung;
    mail.pgp = verarbeitet.status;
    Ok(mail)
}

/// Rohe Mail vom Server bzw. aus dem Speicher und – bei OpenPGP – die entschlüsselte bzw.
/// geprüfte Fassung zur Anzeige. Die entschlüsselte Fassung wird nie gespeichert.
async fn anzeige_inhalt(
    zustand: &AppZustand,
    konto: &str,
    ordner: &str,
    uid: u32,
) -> Result<(Vec<u8>, Verarbeitet), Fehlermeldung> {
    let roh = zustand.imap.inhalt(konto, ordner, uid).await?;
    let pgp = zustand.pgp.clone();
    let smime = zustand.smime.clone();
    let kopie = roh.clone();
    // Entschlüsseln und Schlüsselableitung brauchen Rechenzeit: nicht im Ereignis-Thread.
    let verarbeitet = tokio::task::spawn_blocking(move || {
        // S/MIME zuerst: Eine S/MIME-signierte Mail ist auch `multipart/signed`.
        let smime = smime.verarbeiten(&kopie);
        if smime.status != Default::default() {
            return smime;
        }
        pgp.verarbeiten(&kopie)
    })
    .await
    .map_err(|_| Fehlermeldung::neu("intern", "Interner Fehler beim Entschlüsseln."))?;
    Ok((roh, verarbeitet))
}

/// Die Mail so, wie sie angezeigt wird (bei verschlüsselten Mails der Klartext), z. B. für
/// Anhänge. Kann sie nicht entschlüsselt werden, gibt es einen Fehler.
async fn anzeige_roh(
    zustand: &AppZustand,
    konto: &str,
    ordner: &str,
    uid: u32,
) -> Result<Zeroizing<Vec<u8>>, Fehlermeldung> {
    let (roh, verarbeitet) = anzeige_inhalt(zustand, konto, ordner, uid).await?;
    if verarbeitet.status.verschluesselung == Verschluesselung::NichtEntschluesselt {
        return Err(Fehlermeldung::neu(
            "pgp",
            verarbeitet
                .status
                .meldung
                .unwrap_or_else(|| "Die Mail kann nicht entschlüsselt werden.".into()),
        ));
    }
    Ok(verarbeitet.roh.unwrap_or_else(|| Zeroizing::new(roh)))
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
    let roh = anzeige_roh(&zustand, &konto, &ordner, uid).await?;
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

/// Bei `mailto:`-Links nur harmlose Angaben an das Mailprogramm weitergeben.
///
/// Parameter wie `attach=` könnten manche Mailprogramme dazu bringen, beliebige lokale
/// Dateien anzuhängen. Erlaubt sind nur `subject`, `body`, `cc` und `bcc`.
pub fn link_entschaerfen(url: &str) -> String {
    if !url
        .get(..7)
        .is_some_and(|s| s.eq_ignore_ascii_case("mailto:"))
    {
        return url.to_string();
    }
    let Some((adresse, abfrage)) = url.split_once('?') else {
        return url.to_string();
    };
    let erlaubt: Vec<&str> = abfrage
        .split('&')
        .filter(|teil| {
            let name = teil.split('=').next().unwrap_or("").to_ascii_lowercase();
            matches!(name.as_str(), "subject" | "body" | "cc" | "bcc")
        })
        .collect();
    if erlaubt.is_empty() {
        adresse.to_string()
    } else {
        format!("{adresse}?{}", erlaubt.join("&"))
    }
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
        .open_url(link_entschaerfen(&url), None::<&str>)
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
        proxy_externe_bilder: einstellungen.proxy_externe_bilder,
        proxy_adresse: einstellungen.proxy_adresse,
        speicher_hinweis: zustand.speicher_hinweis.lock().ok().and_then(|h| h.clone()),
        darstellung: einstellungen.darstellung,
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

/// Schaltet den Proxy für externe Bilder ein (nur mit gültiger Adresse) oder aus (#46).
#[tauri::command]
pub async fn proxy_setzen(
    zustand: State<'_, AppZustand>,
    an: bool,
    adresse: String,
) -> Result<(), Fehlermeldung> {
    zustand.einstellungen.proxy_setzen(an, &adresse).await
}

/// Speichert die Darstellung (#47) und passt die Titelleiste des Fensters an. Die Farben der
/// Oberfläche setzt das Frontend selbst.
#[tauri::command]
pub async fn darstellung_setzen(
    app: AppHandle,
    zustand: State<'_, AppZustand>,
    darstellung: Darstellung,
) -> Result<(), Fehlermeldung> {
    zustand
        .einstellungen
        .darstellung_setzen(darstellung)
        .await?;
    if let Some(fenster) = app.get_webview_window(crate::HAUPTFENSTER) {
        // Nur Optik: Scheitert es (z. B. ohne Unterstützung des Systems), bleibt die Wahl gespeichert.
        let _ = fenster.set_theme(darstellung.fenster_thema());
    }
    Ok(())
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
    let roh = anzeige_roh(&zustand, &konto, &ordner, uid).await?;
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

// ---- OpenPGP-Schlüssel (#44) ----

/// Alle Schlüssel (ohne geheime Teile).
#[tauri::command]
pub async fn pgp_uebersicht(zustand: State<'_, AppZustand>) -> Result<Uebersicht, Fehlermeldung> {
    let pgp = zustand.pgp.clone();
    blockierend(move || pgp.uebersicht()).await
}

/// Führt rechenintensive OpenPGP-Arbeit (Schlüsselableitung) ausserhalb des Ereignis-Threads aus.
async fn blockierend<T: Send + 'static>(
    arbeit: impl FnOnce() -> Result<T, Fehlermeldung> + Send + 'static,
) -> Result<T, Fehlermeldung> {
    tokio::task::spawn_blocking(arbeit)
        .await
        .map_err(|_| Fehlermeldung::neu("intern", "Interner Fehler bei der Verschlüsselung."))?
}

/// Erzeugt einen eigenen Schlüssel für ein Konto, geschützt mit der Passphrase.
#[tauri::command]
pub async fn pgp_erzeugen(
    zustand: State<'_, AppZustand>,
    konto: String,
    passphrase: Zeroizing<String>,
) -> Result<SchluesselInfo, Fehlermeldung> {
    let konto = zustand.konten.finden(&konto).await?;
    let pgp = zustand.pgp.clone();
    blockierend(move || pgp.erzeugen(&konto.id, &konto.name, &konto.email, &passphrase)).await
}

/// Liest eine Datei, die der Nutzer im Backend-Dialog wählt (höchstens 1 MB).
async fn schluesseldatei_waehlen(
    app: &AppHandle,
    titel: &str,
) -> Result<Option<Zeroizing<Vec<u8>>>, Fehlermeldung> {
    datei_waehlen(
        app,
        titel,
        ("OpenPGP-Schlüssel", &["asc", "gpg", "pgp", "key"]),
        crate::pgp::schluessel::MAX_SCHLUESSELDATEI,
    )
    .await
    .map_err(|e| match e.art.as_str() {
        "zu_gross" => Fehlermeldung::neu("pgp", "Die Schlüsseldatei ist zu gross."),
        _ => e,
    })
}

/// Liest eine Datei, die der Nutzer im Backend-Dialog wählt (höchstens `max` Bytes).
async fn datei_waehlen(
    app: &AppHandle,
    titel: &str,
    (filter, endungen): (&str, &[&str]),
    max: usize,
) -> Result<Option<Zeroizing<Vec<u8>>>, Fehlermeldung> {
    let (sender, empfaenger) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title(titel)
        .add_filter(filter, endungen)
        .pick_file(move |datei| {
            let _ = sender.send(datei);
        });
    let Some(datei) = empfaenger.await.ok().flatten() else {
        return Ok(None);
    };
    let pfad = datei
        .into_path()
        .map_err(|_| Fehlermeldung::neu("datei", "Ungültige Datei."))?;
    let daten = crate::smtp::anhaenge::datei_lesen_begrenzt(&pfad, max as u64)
        .await
        .map_err(|_| Fehlermeldung::neu("datei", "Die Datei konnte nicht gelesen werden."))?
        .ok_or_else(|| Fehlermeldung::neu("zu_gross", "Die Datei ist zu gross."))?;
    Ok(Some(Zeroizing::new(daten)))
}

/// Importiert einen eigenen privaten Schlüssel (Dateidialog im Backend). `None` bei Abbruch.
#[tauri::command]
pub async fn pgp_eigenen_importieren(
    app: AppHandle,
    zustand: State<'_, AppZustand>,
    konto: String,
    passphrase: Zeroizing<String>,
) -> Result<Option<SchluesselInfo>, Fehlermeldung> {
    let konto = zustand.konten.finden(&konto).await?;
    let Some(daten) =
        schluesseldatei_waehlen(&app, "Eigenen privaten Schlüssel importieren").await?
    else {
        return Ok(None);
    };
    let pgp = zustand.pgp.clone();
    blockierend(move || pgp.eigenen_importieren(&konto.id, &konto.email, &daten, &passphrase))
        .await
        .map(Some)
}

/// Importiert öffentliche Schlüssel anderer aus einer Datei (Dateidialog im Backend).
#[tauri::command]
pub async fn pgp_oeffentliche_importieren(
    app: AppHandle,
    zustand: State<'_, AppZustand>,
) -> Result<Vec<SchluesselInfo>, Fehlermeldung> {
    let Some(daten) = schluesseldatei_waehlen(&app, "Öffentlichen Schlüssel importieren").await?
    else {
        return Ok(Vec::new());
    };
    let pgp = zustand.pgp.clone();
    blockierend(move || pgp.oeffentliche_importieren(&daten)).await
}

/// Importiert öffentliche Schlüssel aus einem Anhang einer Mail.
#[tauri::command]
pub async fn pgp_aus_anhang_importieren(
    zustand: State<'_, AppZustand>,
    konto: String,
    ordner: String,
    uid: u32,
    index: usize,
) -> Result<Vec<SchluesselInfo>, Fehlermeldung> {
    let roh = anzeige_roh(&zustand, &konto, &ordner, uid).await?;
    let (_, daten) = anzeige::anhang_inhalt(&roh, index)?;
    let pgp = zustand.pgp.clone();
    blockierend(move || pgp.oeffentliche_importieren(&daten)).await
}

/// Speichert einen öffentlichen Schlüssel über „Speichern unter“. `None` bei Abbruch.
#[tauri::command]
pub async fn pgp_exportieren(
    app: AppHandle,
    zustand: State<'_, AppZustand>,
    fingerabdruck: String,
) -> Result<Option<String>, Fehlermeldung> {
    let pgp = zustand.pgp.clone();
    let (name, text) = blockierend(move || pgp.exportieren(&fingerabdruck)).await?;
    let (sender, empfaenger) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Öffentlichen Schlüssel speichern unter …")
        .set_file_name(crate::dateien::dateiname_saeubern(&name, "schluessel.asc"))
        .save_file(move |pfad| {
            let _ = sender.send(pfad);
        });
    let Some(pfad) = empfaenger.await.ok().flatten() else {
        return Ok(None);
    };
    let pfad = pfad
        .into_path()
        .map_err(|_| Fehlermeldung::neu("datei", "Ungültiger Speicherort."))?;
    tokio::fs::write(&pfad, text.as_bytes())
        .await
        .map_err(|_| {
            Fehlermeldung::neu("datei", "Der Schlüssel konnte nicht gespeichert werden.")
        })?;
    Ok(Some(
        pfad.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or(name),
    ))
}

/// Löscht einen Schlüssel endgültig (die Oberfläche fragt vorher nach).
#[tauri::command]
pub async fn pgp_loeschen(
    zustand: State<'_, AppZustand>,
    fingerabdruck: String,
) -> Result<(), Fehlermeldung> {
    let pgp = zustand.pgp.clone();
    blockierend(move || pgp.loeschen(&fingerabdruck)).await
}

/// Prüft vor dem Senden, ob für „Verschlüsseln“ bzw. „Signieren“ alles da ist: Schlüssel aller
/// Empfänger (An, Cc, Bcc), eigener Schlüssel und – zum Signieren – die Passphrase.
#[tauri::command]
pub async fn pgp_versand_pruefen(
    zustand: State<'_, AppZustand>,
    entwurf: Entwurf,
) -> Result<VersandPruefung, Fehlermeldung> {
    let adressen = crate::smtp::nachricht::alle_empfaenger(&entwurf)?;
    let pgp = zustand.pgp.clone();
    blockierend(move || {
        pgp.versand_pruefen(
            &entwurf.konto,
            &adressen,
            entwurf.verschluesseln,
            entwurf.signieren,
        )
    })
    .await
}

/// Entsperrt einen eigenen Schlüssel bis zum Programmende (Passphrase nur im Arbeitsspeicher).
#[tauri::command]
pub async fn pgp_entsperren(
    zustand: State<'_, AppZustand>,
    fingerabdruck: String,
    passphrase: Zeroizing<String>,
) -> Result<(), Fehlermeldung> {
    let pgp = zustand.pgp.clone();
    blockierend(move || pgp.entsperren(&fingerabdruck, &passphrase)).await
}

/// S/MIME-Zertifikate für die Oberfläche (#56).
#[tauri::command]
pub async fn smime_uebersicht(
    zustand: State<'_, AppZustand>,
) -> Result<crate::smime::Uebersicht, Fehlermeldung> {
    let smime = zustand.smime.clone();
    blockierend(move || smime.uebersicht()).await
}

/// Liest eine Zertifikatsdatei, die der Nutzer im Backend-Dialog wählt.
async fn zertifikatsdatei_waehlen(
    app: &AppHandle,
    titel: &str,
    filter: (&str, &[&str]),
) -> Result<Option<Zeroizing<Vec<u8>>>, Fehlermeldung> {
    datei_waehlen(app, titel, filter, MAX_ZERTIFIKATSDATEI)
        .await
        .map_err(|e| match e.art.as_str() {
            "zu_gross" => Fehlermeldung::neu("smime", "Die Zertifikatsdatei ist zu gross."),
            _ => e,
        })
}

/// Importiert ein eigenes Zertifikat mit privatem Schlüssel (`.p12`/`.pfx`, Dateidialog im
/// Backend). `None` bei Abbruch.
#[tauri::command]
pub async fn smime_eigenes_importieren(
    app: AppHandle,
    zustand: State<'_, AppZustand>,
    konto: String,
    passwort: Zeroizing<String>,
) -> Result<Option<ZertifikatInfo>, Fehlermeldung> {
    let konto = zustand.konten.finden(&konto).await?;
    let Some(daten) = zertifikatsdatei_waehlen(
        &app,
        "Eigenes S/MIME-Zertifikat importieren",
        ("PKCS#12 (.p12, .pfx)", &["p12", "pfx"]),
    )
    .await?
    else {
        return Ok(None);
    };
    let smime = zustand.smime.clone();
    blockierend(move || smime.eigenes_importieren(&konto.id, &konto.email, &daten, &passwort))
        .await
        .map(Some)
}

const ZERTIFIKATSDATEIEN: (&str, &[&str]) = (
    "Zertifikate (.cer, .crt, .pem, .p7c, .der)",
    &["cer", "crt", "pem", "p7c", "p7b", "der"],
);

/// Importiert Zertifikate anderer aus einer Datei.
#[tauri::command]
pub async fn smime_fremde_importieren(
    app: AppHandle,
    zustand: State<'_, AppZustand>,
) -> Result<Vec<ZertifikatInfo>, Fehlermeldung> {
    let Some(daten) =
        zertifikatsdatei_waehlen(&app, "Zertifikat importieren", ZERTIFIKATSDATEIEN).await?
    else {
        return Ok(Vec::new());
    };
    let smime = zustand.smime.clone();
    blockierend(move || smime.fremde_importieren(&daten)).await
}

/// Übernimmt das Zertifikat des Absenders einer gültig signierten Mail.
#[tauri::command]
pub async fn smime_aus_mail_uebernehmen(
    zustand: State<'_, AppZustand>,
    konto: String,
    ordner: String,
    uid: u32,
) -> Result<ZertifikatInfo, Fehlermeldung> {
    let roh = zustand.imap.inhalt(&konto, &ordner, uid).await?;
    let smime = zustand.smime.clone();
    blockierend(move || smime.aus_mail_uebernehmen(&roh)).await
}

/// Liest ein Wurzelzertifikat zur Bestätigung (noch nicht übernommen). `None` bei Abbruch.
#[tauri::command]
pub async fn smime_wurzel_lesen(
    app: AppHandle,
    zustand: State<'_, AppZustand>,
) -> Result<Option<ZertifikatInfo>, Fehlermeldung> {
    let Some(daten) =
        zertifikatsdatei_waehlen(&app, "Wurzelzertifikat übernehmen", ZERTIFIKATSDATEIEN).await?
    else {
        return Ok(None);
    };
    let smime = zustand.smime.clone();
    blockierend(move || smime.wurzel_lesen(&daten))
        .await
        .map(Some)
}

/// Übernimmt die zuvor gelesene Wurzel, nachdem der Nutzer den Fingerabdruck bestätigt hat.
#[tauri::command]
pub async fn smime_wurzel_bestaetigen(
    zustand: State<'_, AppZustand>,
    fingerabdruck: String,
) -> Result<ZertifikatInfo, Fehlermeldung> {
    let smime = zustand.smime.clone();
    blockierend(move || smime.wurzel_bestaetigen(&fingerabdruck)).await
}

/// Löscht ein S/MIME-Zertifikat endgültig (die Oberfläche fragt vorher nach).
#[tauri::command]
pub async fn smime_loeschen(
    zustand: State<'_, AppZustand>,
    fingerabdruck: String,
) -> Result<(), Fehlermeldung> {
    let smime = zustand.smime.clone();
    blockierend(move || smime.loeschen(&fingerabdruck)).await
}

/// Entsperrt ein eigenes S/MIME-Zertifikat bis zum Programmende (Passphrase nur im
/// Arbeitsspeicher).
#[tauri::command]
pub async fn smime_entsperren(
    zustand: State<'_, AppZustand>,
    fingerabdruck: String,
    passphrase: Zeroizing<String>,
) -> Result<(), Fehlermeldung> {
    let smime = zustand.smime.clone();
    blockierend(move || smime.entsperren(&fingerabdruck, &passphrase)).await
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

    #[test]
    fn mailto_gibt_keine_anhaenge_weiter() {
        assert_eq!(
            link_entschaerfen("mailto:x@example.org?attach=~/.ssh/id_rsa&subject=Hallo"),
            "mailto:x@example.org?subject=Hallo"
        );
        assert_eq!(
            link_entschaerfen("MAILTO:x@example.org?Attachment=/etc/passwd"),
            "MAILTO:x@example.org"
        );
        assert_eq!(
            link_entschaerfen("mailto:x@example.org?cc=y@example.org&body=Text"),
            "mailto:x@example.org?cc=y@example.org&body=Text"
        );
        assert_eq!(
            link_entschaerfen("https://example.org/?attach=1"),
            "https://example.org/?attach=1"
        );
    }
}
