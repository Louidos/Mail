//! IMAP-Befehle für die Oberfläche, mit einer wiederverwendeten Verbindung pro Konto.
//!
//! Bricht eine Verbindung ab, wird einmal automatisch neu verbunden und der Befehl wiederholt –
//! ausser bei Befehlen, die beim Wiederholen doppelte Mails erzeugen könnten (COPY, APPEND),
//! sobald sie gesendet wurden.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use async_imap::types::Fetch;
use futures_util::TryStreamExt;
use serde::Serialize;
use tokio::sync::Mutex;
use tokio::time::timeout;

pub use super::beobachter::POSTEINGANG;
use super::cache::{MailCache, MailKopf};
use super::ordner::{ordner_laden, Ordner, Sonderordner};
use super::suche::{suchbefehl, uid_suchen, MAX_SERVER_TREFFER};
use super::sync::{
    aeltere_laden, koepfe_holen, ordner_abgleichen, SyncErgebnis, SyncFehler, ERSTE_SEITE,
};
use super::{anmelden, fehler_einordnen, ImapSitzung};
use crate::fehler::{Fehlermeldung, VerbindungsFehler};
use crate::konto::KontoVerwaltung;
use crate::speicher::suche::{self, MailEintrag, SuchFeld, SuchSeite, Suche};

/// Höchstdauer eines einzelnen Befehls.
const BEFEHL_ZEITLIMIT: Duration = Duration::from_secs(120);

/// Mails bis zu dieser Grösse werden für das Lesen ohne Internet vorgeladen.
pub const MAX_VORLADEN_GROESSE: u32 = 5 * 1024 * 1024;

type Sitzungsplatz = Arc<Mutex<Option<ImapSitzung>>>;
type Zukunft<'a, T> = Pin<Box<dyn Future<Output = Result<T, SyncFehler>> + Send + 'a>>;

/// Eine Seite der Mailliste.
#[derive(Debug, Clone, Serialize)]
pub struct MailSeite {
    pub mails: Vec<MailKopf>,
    /// Anzahl Mails im Ordner auf dem Server (nur beim Laden der ersten Seite bekannt).
    pub auf_server: Option<u32>,
    /// Gesetzt, wenn der Server nicht erreichbar war und lokale Daten gezeigt werden.
    pub offline: Option<String>,
    /// Gibt es vermutlich weitere (ältere) Mails?
    pub weitere: bool,
}

/// Eine Seite des gemeinsamen Posteingangs (#50).
#[derive(Debug, Clone, Serialize)]
pub struct PosteingangSeite {
    /// Mails der Posteingänge aller Konten, neueste zuerst; jede mit ihrem Konto.
    pub mails: Vec<MailEintrag>,
    pub weitere: bool,
    /// Ungelesene Mails in allen Posteingängen (soweit lokal bekannt).
    pub ungelesen: usize,
    /// Gesetzt, wenn ältere Mails eines Kontos nicht geladen werden konnten (z. B. offline).
    pub hinweis: Option<String>,
}

/// Ist der Fehler ein Verbindungsproblem (dann werden lokale Daten gezeigt)?
pub fn ist_offline_fehler(fehler: &Fehlermeldung) -> bool {
    matches!(
        fehler.art.as_str(),
        "nicht_erreichbar" | "zeitueberschreitung" | "unterbrochen"
    )
}

pub struct ImapDienst {
    konten: Arc<KontoVerwaltung>,
    cache: RwLock<Arc<dyn MailCache>>,
    sitzungen: std::sync::Mutex<HashMap<String, Sitzungsplatz>>,
    ordner: std::sync::Mutex<HashMap<String, Vec<Ordner>>>,
    erste_seite: u32,
    vorladen: AtomicBool,
}

impl ImapDienst {
    pub fn neu(konten: Arc<KontoVerwaltung>, cache: Arc<dyn MailCache>) -> Self {
        Self {
            konten,
            cache: RwLock::new(cache),
            sitzungen: Default::default(),
            ordner: Default::default(),
            erste_seite: ERSTE_SEITE,
            vorladen: AtomicBool::new(false),
        }
    }

    /// Sollen Inhalte neuer Mails für das Lesen ohne Internet vorgeladen werden?
    pub fn vorladen(&self) -> bool {
        self.vorladen.load(Ordering::Relaxed)
    }

    pub fn vorladen_setzen(&self, an: bool) {
        self.vorladen.store(an, Ordering::Relaxed);
    }

    /// Ändert, wie viele Mails beim ersten Abgleich eines Ordners geladen werden.
    pub fn mit_erster_seite(mut self, anzahl: u32) -> Self {
        self.erste_seite = anzahl.max(1);
        self
    }

    pub fn konten(&self) -> &Arc<KontoVerwaltung> {
        &self.konten
    }

    /// Der aktuelle Zwischenspeicher.
    pub fn cache(&self) -> Arc<dyn MailCache> {
        match self.cache.read() {
            Ok(c) => c.clone(),
            Err(vergiftet) => vergiftet.into_inner().clone(),
        }
    }

    /// Ersetzt den Zwischenspeicher (z. B. beim Umschalten „Mails lokal speichern“).
    pub fn cache_ersetzen(&self, cache: Arc<dyn MailCache>) {
        match self.cache.write() {
            Ok(mut c) => *c = cache,
            Err(vergiftet) => *vergiftet.into_inner() = cache,
        }
    }

    fn platz(&self, konto_id: &str) -> Sitzungsplatz {
        let mut sitzungen = match self.sitzungen.lock() {
            Ok(s) => s,
            Err(vergiftet) => vergiftet.into_inner(),
        };
        sitzungen
            .entry(konto_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(None)))
            .clone()
    }

    /// Trennt die Verbindung eines Kontos (z. B. nach Bearbeiten oder Löschen).
    pub async fn trennen(&self, konto_id: &str) {
        let platz = match self.sitzungen.lock() {
            Ok(mut s) => s.remove(konto_id),
            Err(vergiftet) => vergiftet.into_inner().remove(konto_id),
        };
        if let Some(platz) = platz {
            if let Some(mut sitzung) = platz.lock().await.take() {
                let _ = timeout(Duration::from_secs(5), sitzung.logout()).await;
            }
        }
        if let Ok(mut ordner) = self.ordner.lock() {
            ordner.remove(konto_id);
        }
    }

    /// Führt einen Befehl mit der Verbindung des Kontos aus; verbindet bei Bedarf (neu).
    async fn ausfuehren<T: Send>(
        &self,
        konto_id: &str,
        befehl: impl for<'a> Fn(&'a mut ImapSitzung) -> Zukunft<'a, T>,
    ) -> Result<T, SyncFehler> {
        self.ausfuehren_mit(konto_id, &AtomicBool::new(false), befehl)
            .await
    }

    /// Wie [`ImapDienst::ausfuehren`]. Setzt der Befehl `gesendet`, bevor die Verbindung
    /// abbricht, wird er nicht wiederholt: Der Server hat ihn womöglich schon ausgeführt.
    async fn ausfuehren_mit<T: Send>(
        &self,
        konto_id: &str,
        gesendet: &AtomicBool,
        befehl: impl for<'a> Fn(&'a mut ImapSitzung) -> Zukunft<'a, T>,
    ) -> Result<T, SyncFehler> {
        let platz = self.platz(konto_id);
        let mut sitzung = platz.lock().await;
        let mut versuch = 0;
        loop {
            versuch += 1;
            if sitzung.is_none() {
                let konto = self.konten.finden(konto_id).await?;
                let passwort = self.konten.passwort(konto_id)?;
                *sitzung = Some(anmelden(&konto.imap, &konto.benutzername, &passwort).await?);
            }
            let Some(aktiv) = sitzung.as_mut() else {
                continue;
            };
            let ergebnis = match timeout(BEFEHL_ZEITLIMIT, befehl(aktiv)).await {
                Ok(ergebnis) => ergebnis,
                Err(_) => Err(VerbindungsFehler::Zeitueberschreitung.into()),
            };
            match ergebnis {
                Err(SyncFehler::Verbindung(fehler)) => {
                    // Nach jedem Fehler die Sitzung verwerfen: Sie könnte noch unvollständig
                    // gelesene Antworten enthalten und darf so nicht weiterverwendet werden.
                    *sitzung = None;
                    if versuch >= 2
                        || !fehler.neu_verbinden_sinnvoll()
                        || gesendet.load(Ordering::SeqCst)
                    {
                        return Err(fehler.into());
                    }
                }
                anderes => return anderes,
            }
        }
    }

    /// Ordnerliste vom Server.
    pub async fn ordner(&self, konto_id: &str) -> Result<Vec<Ordner>, Fehlermeldung> {
        let versuche = Arc::new(AtomicUsize::new(0));
        let liste = self
            .ausfuehren(konto_id, move |s| {
                let versuche = versuche.clone();
                Box::pin(async move {
                    let liste = ordner_laden(s).await?;
                    // Jeder IMAP-Server hat einen Posteingang. Fehlt er, war die Antwort
                    // unvollständig: einmal mit frischer Verbindung wiederholen.
                    if !liste
                        .iter()
                        .any(|o| o.sonder == Some(Sonderordner::Posteingang))
                        && versuche.fetch_add(1, Ordering::SeqCst) == 0
                    {
                        return Err(VerbindungsFehler::Unterbrochen.into());
                    }
                    Ok(liste)
                })
            })
            .await?;
        if let Ok(mut ordner) = self.ordner.lock() {
            ordner.insert(konto_id.to_string(), liste.clone());
        }
        Ok(liste)
    }

    async fn sonderordner(
        &self,
        konto_id: &str,
        art: Sonderordner,
    ) -> Result<Option<String>, Fehlermeldung> {
        let bekannt = self
            .ordner
            .lock()
            .ok()
            .and_then(|o| o.get(konto_id).cloned());
        let liste = match bekannt {
            Some(liste) => liste,
            None => self.ordner(konto_id).await?,
        };
        Ok(liste
            .into_iter()
            .find(|o| o.sonder == Some(art) && o.waehlbar)
            .map(|o| o.name))
    }

    /// Gleicht einen Ordner mit dem Server ab.
    pub async fn abgleichen(
        &self,
        konto_id: &str,
        ordner: &str,
    ) -> Result<SyncErgebnis, Fehlermeldung> {
        let cache = self.cache();
        let konto = konto_id.to_string();
        let ordner = ordner.to_string();
        let erste_seite = self.erste_seite;
        Ok(self
            .ausfuehren(konto_id, move |s| {
                let cache = cache.clone();
                let konto = konto.clone();
                let ordner = ordner.clone();
                Box::pin(async move {
                    ordner_abgleichen(s, cache.as_ref(), &konto, &ordner, erste_seite).await
                })
            })
            .await?)
    }

    /// Eine Seite der Mailliste nur aus dem lokalen Zwischenspeicher (ohne Netz, sofort).
    pub fn mails_lokal(
        &self,
        konto_id: &str,
        ordner: &str,
        start: usize,
        anzahl: usize,
    ) -> Result<MailSeite, Fehlermeldung> {
        let mails = self.cache().seite(konto_id, ordner, start, anzahl)?;
        let weitere = mails.len() == anzahl;
        Ok(MailSeite {
            mails,
            auf_server: None,
            offline: None,
            weitere,
        })
    }

    /// Lokale Suche (#49) im Zwischenspeicher – ohne Netz. Gibt höchstens `anzahl` Treffer ab
    /// `start` zurück und ob es weitere gibt.
    pub fn suchen(
        &self,
        suche: &Suche,
        start: usize,
        anzahl: usize,
    ) -> Result<SuchSeite, Fehlermeldung> {
        let mut mails = self
            .cache()
            .suchen(suche, start, anzahl.saturating_add(1))?;
        let weitere = mails.len() > anzahl;
        mails.truncate(anzahl);
        Ok(SuchSeite { mails, weitere })
    }

    /// Gemeinsamer Posteingang (#50): die Posteingänge aller `konten` in einer Liste, nach Datum.
    ///
    /// Gelesen wird nur aus dem lokalen Zwischenspeicher; neue Mails bringen die bestehenden
    /// Beobachter (IDLE) hinein. Nur mit `nachladen` („Ältere Mails laden“) holt jedes Konto über
    /// seine bestehende Befehlsverbindung ältere Mails nach – es entstehen keine neuen Arten von
    /// Verbindungen.
    pub async fn alle_posteingaenge(
        &self,
        konten: &[String],
        start: usize,
        anzahl: usize,
        nachladen: bool,
    ) -> Result<PosteingangSeite, Fehlermeldung> {
        let mut hinweis = None;
        let mut auf_server_weitere = false;
        if nachladen {
            for konto in konten {
                let vorhanden = self.cache().uids(konto, POSTEINGANG)?.len();
                match self
                    .mails_seite(konto, POSTEINGANG, vorhanden, anzahl)
                    .await
                {
                    Ok(seite) => {
                        auf_server_weitere |= seite.weitere;
                        if let Some(meldung) = seite.offline {
                            hinweis.get_or_insert(meldung);
                        }
                    }
                    // Ein Konto ohne Verbindung hält die anderen nicht auf.
                    Err(fehler) => {
                        hinweis.get_or_insert(fehler.meldung);
                    }
                }
            }
        }
        let cache = self.cache();
        let mut mails = cache.ueber_konten(konten, POSTEINGANG, start, anzahl.saturating_add(1))?;
        let weitere = mails.len() > anzahl || auf_server_weitere || !nachladen;
        mails.truncate(anzahl);
        Ok(PosteingangSeite {
            mails,
            weitere,
            ungelesen: cache.ungelesen(konten, POSTEINGANG)?,
            hinweis,
        })
    }

    /// Server-Suche (#49) in den Posteingängen aller `konten` (#50), je über die Verbindung des
    /// Kontos. Konten ohne Verbindung werden übersprungen; scheitern alle, kommt der Fehler.
    pub async fn server_suchen_posteingaenge(
        &self,
        konten: &[String],
        begriff: &str,
        feld: SuchFeld,
    ) -> Result<SuchSeite, Fehlermeldung> {
        let mut seite = SuchSeite::default();
        let mut erster_fehler = None;
        let mut erfolgreich = konten.is_empty();
        for konto in konten {
            match self
                .server_suchen(konto, Some(POSTEINGANG), begriff, feld)
                .await
            {
                Ok(teil) => {
                    erfolgreich = true;
                    seite.weitere |= teil.weitere;
                    seite.mails.extend(teil.mails);
                }
                Err(fehler) => {
                    erster_fehler.get_or_insert(fehler);
                }
            }
        }
        if let (false, Some(fehler)) = (erfolgreich, erster_fehler) {
            return Err(fehler);
        }
        seite.mails.sort_by(suche::nach_datum);
        if seite.mails.len() > MAX_SERVER_TREFFER {
            seite.mails.truncate(MAX_SERVER_TREFFER);
            seite.weitere = true;
        }
        Ok(seite)
    }

    /// Eine Seite der Mailliste (neueste zuerst).
    ///
    /// Für die erste Seite wird vorher mit dem Server abgeglichen. Ist der Server nicht
    /// erreichbar, werden die lokal gespeicherten Mails gezeigt.
    pub async fn mails_seite(
        &self,
        konto_id: &str,
        ordner: &str,
        start: usize,
        anzahl: usize,
    ) -> Result<MailSeite, Fehlermeldung> {
        let mut offline = None;
        let mut auf_server = None;
        if start == 0 {
            match self.abgleichen(konto_id, ordner).await {
                Ok(ergebnis) => auf_server = Some(ergebnis.auf_server),
                Err(fehler) if ist_offline_fehler(&fehler) => offline = Some(fehler.meldung),
                Err(fehler) => return Err(fehler),
            }
        }
        let cache = self.cache();
        let mut mails = cache.seite(konto_id, ordner, start, anzahl)?;
        let mut weitere = mails.len() == anzahl;

        if mails.len() < anzahl && offline.is_none() {
            let fehlend = anzahl - mails.len();
            let konto = konto_id.to_string();
            let ordner_name = ordner.to_string();
            let cache_kopie = cache.clone();
            let geladen = self
                .ausfuehren(konto_id, move |s| {
                    let cache = cache_kopie.clone();
                    let konto = konto.clone();
                    let ordner = ordner_name.clone();
                    Box::pin(async move {
                        s.select(&ordner).await.map_err(fehler_einordnen)?;
                        aeltere_laden(s, cache.as_ref(), &konto, &ordner, fehlend).await
                    })
                })
                .await;
            match geladen {
                Ok(anzahl_geladen) => {
                    weitere = anzahl_geladen == fehlend;
                    mails = cache.seite(konto_id, ordner, start, anzahl)?;
                }
                Err(fehler) => {
                    let fehler: Fehlermeldung = fehler.into();
                    if !ist_offline_fehler(&fehler) {
                        return Err(fehler);
                    }
                    offline = Some(fehler.meldung);
                }
            }
        }
        Ok(MailSeite {
            mails,
            auf_server,
            offline,
            weitere,
        })
    }

    /// Suche auf dem Server (#49) mit `UID SEARCH` im Ordner `ordner` bzw. in allen Ordnern des
    /// Kontos. Nur über die bestehende Verbindung des Kontos. Gezeigt werden die neuesten
    /// [`MAX_SERVER_TREFFER`] Treffer; Kopfdaten noch nicht gespeicherter Mails werden vom Server
    /// geladen, aber nicht in den Zwischenspeicher übernommen (sonst würden beim späteren
    /// „Ältere Mails laden“ die Mails dazwischen übersprungen).
    pub async fn server_suchen(
        &self,
        konto_id: &str,
        ordner: Option<&str>,
        begriff: &str,
        feld: SuchFeld,
    ) -> Result<SuchSeite, Fehlermeldung> {
        let Some(befehl) = suchbefehl(begriff, feld) else {
            return Ok(SuchSeite::default());
        };
        let befehl = Arc::new(befehl);
        let ordner_liste: Vec<String> = match ordner {
            Some(o) => vec![o.to_string()],
            None => self
                .ordner(konto_id)
                .await?
                .into_iter()
                .filter(|o| o.waehlbar)
                .map(|o| o.name)
                .collect(),
        };
        let mut seite = SuchSeite::default();
        for name in ordner_liste {
            let cache = self.cache();
            let konto = konto_id.to_string();
            let befehl = befehl.clone();
            let ordner_name = name.clone();
            let (koepfe, mehr) = self
                .ausfuehren(konto_id, move |s| {
                    let (cache, konto, befehl, ordner) = (
                        cache.clone(),
                        konto.clone(),
                        befehl.clone(),
                        ordner_name.clone(),
                    );
                    Box::pin(async move {
                        s.select(&ordner).await.map_err(fehler_einordnen)?;
                        let uids = uid_suchen(s, &befehl).await?;
                        let mehr = uids.len() > MAX_SERVER_TREFFER;
                        let mut koepfe = Vec::new();
                        let mut fehlend = Vec::new();
                        for uid in uids.into_iter().rev().take(MAX_SERVER_TREFFER) {
                            match cache.kopf(&konto, &ordner, uid)? {
                                Some(kopf) => koepfe.push(kopf),
                                None => fehlend.push(uid.to_string()),
                            }
                        }
                        if !fehlend.is_empty() {
                            koepfe.extend(koepfe_holen(s, &fehlend.join(","), true).await?);
                        }
                        Ok((koepfe, mehr))
                    })
                })
                .await?;
            seite.weitere |= mehr;
            seite
                .mails
                .extend(koepfe.into_iter().map(|kopf| MailEintrag {
                    konto: konto_id.to_string(),
                    ordner: name.clone(),
                    kopf,
                }));
        }
        seite.mails.sort_by(suche::nach_datum);
        if seite.mails.len() > MAX_SERVER_TREFFER {
            seite.mails.truncate(MAX_SERVER_TREFFER);
            seite.weitere = true;
        }
        Ok(seite)
    }

    /// Vollständige Mail (RFC 822). Aus dem Zwischenspeicher oder vom Server.
    ///
    /// Beim Laden vom Server wird `BODY.PEEK[]` verwendet, damit die Mail nicht
    /// automatisch als gelesen markiert wird.
    pub async fn inhalt(
        &self,
        konto_id: &str,
        ordner: &str,
        uid: u32,
    ) -> Result<Vec<u8>, Fehlermeldung> {
        let cache = self.cache();
        if let Some(inhalt) = cache.inhalt(konto_id, ordner, uid)? {
            return Ok(inhalt);
        }
        let ordner_name = ordner.to_string();
        let versuche = Arc::new(AtomicUsize::new(0));
        let daten = self
            .ausfuehren(konto_id, move |s| {
                let ordner = ordner_name.clone();
                let versuche = versuche.clone();
                Box::pin(async move {
                    s.select(&ordner).await.map_err(fehler_einordnen)?;
                    let antworten: Vec<Fetch> = s
                        .uid_fetch(uid.to_string(), "(UID BODY.PEEK[])")
                        .await
                        .map_err(fehler_einordnen)?
                        .try_collect()
                        .await
                        .map_err(fehler_einordnen)?;
                    match antworten
                        .iter()
                        .find(|f| f.uid == Some(uid))
                        .and_then(|f| f.body())
                    {
                        Some(inhalt) => Ok(inhalt.to_vec()),
                        // Erst nach einem zweiten Versuch mit frischer Verbindung aufgeben.
                        None if versuche.fetch_add(1, Ordering::SeqCst) == 0 => {
                            Err(VerbindungsFehler::Unterbrochen.into())
                        }
                        None => Err(SyncFehler::Speicher(Fehlermeldung::neu(
                            "nicht_gefunden",
                            "Diese Mail gibt es auf dem Server nicht mehr.",
                        ))),
                    }
                })
            })
            .await?;
        cache.inhalt_speichern(konto_id, ordner, uid, &daten)?;
        Ok(daten)
    }

    /// Lädt die Inhalte der neuesten `anzahl` Mails, die noch nicht gespeichert sind
    /// (höchstens 5 MB pro Mail), damit sie ohne Internet lesbar sind.
    pub async fn inhalte_vorladen(
        &self,
        konto_id: &str,
        ordner: &str,
        anzahl: usize,
    ) -> Result<usize, Fehlermeldung> {
        let cache = self.cache();
        let mut fehlend = Vec::new();
        for kopf in cache.seite(konto_id, ordner, 0, anzahl)? {
            if kopf.groesse <= MAX_VORLADEN_GROESSE
                && !cache.hat_inhalt(konto_id, ordner, kopf.uid)?
            {
                fehlend.push(kopf.uid);
            }
        }
        if fehlend.is_empty() {
            return Ok(0);
        }
        let menge = fehlend
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let ordner_name = ordner.to_string();
        let inhalte = self
            .ausfuehren(konto_id, move |s| {
                let ordner = ordner_name.clone();
                let menge = menge.clone();
                Box::pin(async move {
                    s.select(&ordner).await.map_err(fehler_einordnen)?;
                    let antworten: Vec<Fetch> = s
                        .uid_fetch(&menge, "(UID BODY.PEEK[])")
                        .await
                        .map_err(fehler_einordnen)?
                        .try_collect()
                        .await
                        .map_err(fehler_einordnen)?;
                    Ok(antworten
                        .iter()
                        .filter_map(|f| Some((f.uid?, f.body()?.to_vec())))
                        .collect::<Vec<_>>())
                })
            })
            .await?;
        for (uid, inhalt) in &inhalte {
            cache.inhalt_speichern(konto_id, ordner, *uid, inhalt)?;
        }
        Ok(inhalte.len())
    }

    /// Legt eine gesendete Mail im Ordner „Gesendet“ ab (IMAP APPEND, als gelesen markiert).
    /// Gibt es keinen solchen Ordner, wird „Sent“ angelegt.
    pub async fn gesendet_ablegen(
        &self,
        konto_id: &str,
        inhalt: &[u8],
    ) -> Result<(), Fehlermeldung> {
        let ziel = match self.sonderordner(konto_id, Sonderordner::Gesendet).await? {
            Some(ordner) => ordner,
            None => {
                let _ = self
                    .ausfuehren(konto_id, |s| {
                        Box::pin(async move {
                            s.create("Sent").await.map_err(fehler_einordnen)?;
                            Ok(())
                        })
                    })
                    .await;
                if let Ok(mut ordner) = self.ordner.lock() {
                    ordner.remove(konto_id);
                }
                "Sent".to_string()
            }
        };
        let inhalt = Arc::new(inhalt.to_vec());
        let gesendet = Arc::new(AtomicBool::new(false));
        let merker = gesendet.clone();
        self.ausfuehren_mit(konto_id, &gesendet, move |s| {
            let ziel = ziel.clone();
            let inhalt = inhalt.clone();
            let merker = merker.clone();
            Box::pin(async move {
                // Erst prüfen, ob die Verbindung noch steht (darf wiederholt werden) …
                s.noop().await.map_err(fehler_einordnen)?;
                // … danach nicht mehr: Ein wiederholtes APPEND legt die Mail doppelt ab.
                merker.store(true, Ordering::SeqCst);
                s.append(&ziel, Some("(\\Seen)"), None, inhalt.as_slice())
                    .await
                    .map_err(fehler_einordnen)?;
                Ok(())
            })
        })
        .await?;
        Ok(())
    }

    /// Markiert eine Mail als gelesen oder ungelesen (auch auf dem Server).
    pub async fn gelesen_setzen(
        &self,
        konto_id: &str,
        ordner: &str,
        uid: u32,
        gelesen: bool,
    ) -> Result<(), Fehlermeldung> {
        let ordner_name = ordner.to_string();
        self.ausfuehren(konto_id, move |s| {
            let ordner = ordner_name.clone();
            Box::pin(async move {
                s.select(&ordner).await.map_err(fehler_einordnen)?;
                let aenderung = if gelesen {
                    "+FLAGS.SILENT (\\Seen)"
                } else {
                    "-FLAGS.SILENT (\\Seen)"
                };
                let _: Vec<Fetch> = s
                    .uid_store(uid.to_string(), aenderung)
                    .await
                    .map_err(fehler_einordnen)?
                    .try_collect()
                    .await
                    .map_err(fehler_einordnen)?;
                Ok(())
            })
        })
        .await?;
        let cache = self.cache();
        if let Some(kopf) = cache.kopf(konto_id, ordner, uid)? {
            cache.markierungen_setzen(konto_id, ordner, uid, gelesen, kopf.markiert)?;
        }
        Ok(())
    }

    /// Verschiebt eine Mail in einen anderen Ordner (auch auf dem Server).
    pub async fn verschieben(
        &self,
        konto_id: &str,
        ordner: &str,
        uid: u32,
        ziel: &str,
    ) -> Result<(), Fehlermeldung> {
        if ordner == ziel {
            return Ok(());
        }
        let ordner_name = ordner.to_string();
        let ziel_name = ziel.to_string();
        let gesendet = Arc::new(AtomicBool::new(false));
        let merker = gesendet.clone();
        self.ausfuehren_mit(konto_id, &gesendet, move |s| {
            let ordner = ordner_name.clone();
            let ziel = ziel_name.clone();
            let merker = merker.clone();
            Box::pin(async move {
                s.select(&ordner).await.map_err(fehler_einordnen)?;
                let faehigkeiten = s.capabilities().await.map_err(fehler_einordnen)?;
                if faehigkeiten.has_str("MOVE") {
                    s.uid_mv(uid.to_string(), &ziel)
                        .await
                        .map_err(fehler_einordnen)?;
                } else {
                    // Ein wiederholtes COPY legt die Mail doppelt im Ziel ab.
                    merker.store(true, Ordering::SeqCst);
                    s.uid_copy(uid.to_string(), &ziel)
                        .await
                        .map_err(fehler_einordnen)?;
                    entfernen_markieren(s, uid, faehigkeiten.has_str("UIDPLUS")).await?;
                }
                Ok(())
            })
        })
        .await?;
        self.cache().entfernen(konto_id, ordner, &[uid])?;
        Ok(())
    }

    /// Löscht eine Mail: verschiebt sie in den Papierkorb. Liegt sie bereits im
    /// Papierkorb (oder gibt es keinen), wird sie auf dem Server endgültig gelöscht.
    pub async fn loeschen(
        &self,
        konto_id: &str,
        ordner: &str,
        uid: u32,
    ) -> Result<(), Fehlermeldung> {
        match self
            .sonderordner(konto_id, Sonderordner::Papierkorb)
            .await?
        {
            Some(papierkorb) if papierkorb != ordner => {
                self.verschieben(konto_id, ordner, uid, &papierkorb).await
            }
            _ => {
                let ordner_name = ordner.to_string();
                self.ausfuehren(konto_id, move |s| {
                    let ordner = ordner_name.clone();
                    Box::pin(async move {
                        s.select(&ordner).await.map_err(fehler_einordnen)?;
                        let faehigkeiten = s.capabilities().await.map_err(fehler_einordnen)?;
                        entfernen_markieren(s, uid, faehigkeiten.has_str("UIDPLUS")).await
                    })
                })
                .await?;
                self.cache().entfernen(konto_id, ordner, &[uid])?;
                Ok(())
            }
        }
    }
}

/// Markiert genau eine Mail als gelöscht und entfernt nur diese (UID EXPUNGE).
///
/// Ohne UIDPLUS wird kein EXPUNGE gesendet, weil das auch andere als gelöscht
/// markierte Mails endgültig entfernen würde. Die Mail bleibt dann markiert und wird
/// in SicherMail ausgeblendet.
async fn entfernen_markieren(
    sitzung: &mut ImapSitzung,
    uid: u32,
    uidplus: bool,
) -> Result<(), SyncFehler> {
    let _: Vec<Fetch> = sitzung
        .uid_store(uid.to_string(), "+FLAGS.SILENT (\\Deleted)")
        .await
        .map_err(fehler_einordnen)?
        .try_collect()
        .await
        .map_err(fehler_einordnen)?;
    if uidplus {
        let _: Vec<u32> = sitzung
            .uid_expunge(uid.to_string())
            .await
            .map_err(fehler_einordnen)?
            .try_collect()
            .await
            .map_err(fehler_einordnen)?;
    }
    Ok(())
}
