//! Neue Mails ohne manuelles Aktualisieren.
//!
//! Pro Konto läuft eine eigene Verbindung, die den Posteingang beobachtet:
//! - mit IMAP IDLE, wenn der Server es kann (spätestens alle 25 Minuten erneuert),
//! - sonst durch Abfrage alle 2 Minuten.
//!
//! Bricht die Verbindung ab, wird automatisch neu verbunden, mit wachsender Wartezeit
//! (1 s, 2 s, 4 s … bis höchstens 5 Minuten). Nach jeder (Wieder-)Verbindung wird der
//! Posteingang abgeglichen, damit keine Mail fehlt.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_imap::extensions::idle::IdleResponse;
use serde::Serialize;
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout};

use super::dienst::ImapDienst;
use super::{anmelden, fehler_einordnen, ImapSitzung};
use crate::fehler::{Fehlermeldung, VerbindungsFehler};

/// Beobachteter Ordner.
pub const POSTEINGANG: &str = "INBOX";

/// Abfrageabstand, wenn der Server kein IDLE kann.
pub const ABFRAGE_ABSTAND: Duration = Duration::from_secs(120);

/// IDLE wird spätestens nach dieser Zeit erneuert (RFC 2177: unter 29 Minuten).
const IDLE_DAUER: Duration = Duration::from_secs(25 * 60);

/// Längste Wartezeit zwischen zwei Verbindungsversuchen.
const MAX_WARTEZEIT: Duration = Duration::from_secs(300);

/// Meldungen an die Oberfläche.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "art", rename_all = "snake_case")]
pub enum Ereignis {
    /// Im Ordner hat sich etwas geändert (neue, gelöschte oder geänderte Mails).
    MailsGeaendert { konto: String, ordner: String },
    /// Die Beobachtung ist verbunden.
    Verbunden { konto: String, idle: bool },
    /// Die Verbindung ist unterbrochen; nächster Versuch in `sekunden`.
    Getrennt {
        konto: String,
        meldung: String,
        sekunden: u64,
    },
}

pub type Melder = Arc<dyn Fn(Ereignis) + Send + Sync>;

/// Wartezeiten zwischen Verbindungsversuchen: 1 s, 2 s, 4 s … höchstens 5 Minuten.
#[derive(Debug, Clone)]
pub struct Wartezeit {
    aktuell: Duration,
    start: Duration,
    hoechstens: Duration,
}

impl Wartezeit {
    pub fn neu(start: Duration, hoechstens: Duration) -> Self {
        Self {
            aktuell: start,
            start,
            hoechstens,
        }
    }

    /// Nächste Wartezeit; verdoppelt sich bei jedem Aufruf.
    pub fn naechste(&mut self) -> Duration {
        let jetzt = self.aktuell;
        self.aktuell = (self.aktuell * 2).min(self.hoechstens);
        jetzt
    }

    pub fn zuruecksetzen(&mut self) {
        self.aktuell = self.start;
    }
}

impl Default for Wartezeit {
    fn default() -> Self {
        Self::neu(Duration::from_secs(1), MAX_WARTEZEIT)
    }
}

/// Startet und stoppt die Beobachtung pro Konto.
#[derive(Default)]
pub struct Beobachter {
    aufgaben: Mutex<HashMap<String, JoinHandle<()>>>,
}

impl Beobachter {
    /// Startet (oder startet neu) die Beobachtung eines Kontos.
    pub fn starten(&self, konto_id: &str, dienst: Arc<ImapDienst>, melden: Melder) {
        self.starten_mit(konto_id, dienst, melden, Wartezeit::default());
    }

    /// Wie [`Beobachter::starten`], mit eigener Wartezeit (für Tests).
    pub fn starten_mit(
        &self,
        konto_id: &str,
        dienst: Arc<ImapDienst>,
        melden: Melder,
        wartezeit: Wartezeit,
    ) {
        let id = konto_id.to_string();
        let aufgabe = tokio::spawn(beobachten(id.clone(), dienst, melden, wartezeit));
        if let Ok(mut aufgaben) = self.aufgaben.lock() {
            if let Some(alt) = aufgaben.insert(id, aufgabe) {
                alt.abort();
            }
        }
    }

    /// Beendet die Beobachtung eines Kontos.
    pub fn stoppen(&self, konto_id: &str) {
        if let Ok(mut aufgaben) = self.aufgaben.lock() {
            if let Some(aufgabe) = aufgaben.remove(konto_id) {
                aufgabe.abort();
            }
        }
    }
}

impl Drop for Beobachter {
    fn drop(&mut self) {
        if let Ok(aufgaben) = self.aufgaben.lock() {
            for aufgabe in aufgaben.values() {
                aufgabe.abort();
            }
        }
    }
}

async fn beobachten(
    konto: String,
    dienst: Arc<ImapDienst>,
    melden: Melder,
    mut wartezeit: Wartezeit,
) {
    loop {
        let fehler = match einmal_beobachten(&konto, &dienst, &melden, &mut wartezeit).await {
            Ok(()) => continue,
            Err(fehler) => fehler,
        };
        // Konto gelöscht: Beobachtung beenden.
        if fehler.art == "unbekannt" {
            return;
        }
        let pause = wartezeit.naechste();
        melden(Ereignis::Getrennt {
            konto: konto.clone(),
            meldung: fehler.meldung,
            sekunden: pause.as_secs().max(1),
        });
        sleep(pause).await;
    }
}

/// Eine Verbindung aufbauen und beobachten, bis sie abbricht.
async fn einmal_beobachten(
    konto_id: &str,
    dienst: &ImapDienst,
    melden: &Melder,
    wartezeit: &mut Wartezeit,
) -> Result<(), Fehlermeldung> {
    let konto = dienst.konten().finden(konto_id).await?;
    let passwort = dienst.konten().passwort(konto_id)?;
    let mut sitzung = anmelden(&konto.imap, &konto.benutzername, &passwort).await?;
    drop(passwort);

    let faehigkeiten = sitzung.capabilities().await.map_err(fehler_einordnen)?;
    let idle = faehigkeiten.has_str("IDLE");
    // Nur lesend öffnen: Die Beobachtung ändert nichts am Postfach.
    sitzung
        .examine(POSTEINGANG)
        .await
        .map_err(fehler_einordnen)?;

    wartezeit.zuruecksetzen();
    melden(Ereignis::Verbunden {
        konto: konto_id.to_string(),
        idle,
    });
    abgleichen_und_melden(konto_id, dienst, melden).await?;

    loop {
        if idle {
            sitzung = idle_warten(sitzung).await?;
        } else {
            sleep(ABFRAGE_ABSTAND).await;
            timeout(Duration::from_secs(60), sitzung.noop())
                .await
                .map_err(|_| VerbindungsFehler::Zeitueberschreitung)?
                .map_err(fehler_einordnen)?;
        }
        abgleichen_und_melden(konto_id, dienst, melden).await?;
    }
}

async fn abgleichen_und_melden(
    konto_id: &str,
    dienst: &ImapDienst,
    melden: &Melder,
) -> Result<(), Fehlermeldung> {
    let ergebnis = dienst.abgleichen(konto_id, POSTEINGANG).await?;
    if ergebnis.geaendert {
        melden(Ereignis::MailsGeaendert {
            konto: konto_id.to_string(),
            ordner: POSTEINGANG.to_string(),
        });
    }
    Ok(())
}

/// Wartet per IDLE auf eine Änderung (oder bis zur Erneuerung) und gibt die Sitzung zurück.
async fn idle_warten(sitzung: ImapSitzung) -> Result<ImapSitzung, VerbindungsFehler> {
    let mut griff = sitzung.idle();
    timeout(Duration::from_secs(60), griff.init())
        .await
        .map_err(|_| VerbindungsFehler::Zeitueberschreitung)?
        .map_err(fehler_einordnen)?;
    let (warten, _stopp) = griff.wait_with_timeout(IDLE_DAUER);
    let antwort = warten.await.map_err(fehler_einordnen)?;
    let sitzung = timeout(Duration::from_secs(60), griff.done())
        .await
        .map_err(|_| VerbindungsFehler::Zeitueberschreitung)?
        .map_err(fehler_einordnen)?;
    match antwort {
        IdleResponse::NewData(_) | IdleResponse::Timeout => Ok(sitzung),
        IdleResponse::ManualInterrupt => Err(VerbindungsFehler::Unterbrochen),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wartezeit_waechst_bis_zur_grenze() {
        let mut w = Wartezeit::default();
        let folge: Vec<u64> = (0..12).map(|_| w.naechste().as_secs()).collect();
        assert_eq!(folge, vec![1, 2, 4, 8, 16, 32, 64, 128, 256, 300, 300, 300]);
        w.zuruecksetzen();
        assert_eq!(w.naechste().as_secs(), 1);
    }
}
