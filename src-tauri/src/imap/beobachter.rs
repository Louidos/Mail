//! Neue Mails ohne manuelles Aktualisieren.
//!
//! Pro Konto läuft eine eigene Verbindung, die den Posteingang beobachtet:
//! - mit IMAP IDLE, wenn der Server es kann. IDLE wird alle 2 Minuten erneuert; so fällt
//!   auch eine still abgerissene Verbindung (NAT, Ruhezustand, WLAN-Wechsel) spätestens
//!   dann auf, und der Posteingang wird abgeglichen.
//! - sonst – oder wenn der Server IDLE wiederholt ablehnt – durch Abfrage alle 2 Minuten.
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

/// IDLE wird spätestens nach dieser Zeit erneuert (RFC 2177 verlangt unter 29 Minuten;
/// kürzer, damit eine still abgerissene Verbindung schnell auffällt).
const IDLE_DAUER: Duration = ABFRAGE_ABSTAND;

/// Nach so vielen gescheiterten IDLE-Starts in Folge wird nur noch abgefragt.
const IDLE_MAX_FEHLSTARTS: u32 = 2;

/// So viele der neuesten Mails im Posteingang werden für das Lesen ohne Internet vorgeladen.
pub const VORLADEN_ANZAHL: usize = 50;

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
    let mut idle_fehlstarts = 0;
    loop {
        let fehler = match einmal_beobachten(
            &konto,
            &dienst,
            &melden,
            &mut wartezeit,
            &mut idle_fehlstarts,
        )
        .await
        {
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
///
/// `idle_fehlstarts` zählt gescheiterte IDLE-Starts in Folge (über Verbindungen hinweg):
/// Lehnt der Server IDLE wiederholt ab, wird stattdessen alle 2 Minuten abgefragt.
async fn einmal_beobachten(
    konto_id: &str,
    dienst: &ImapDienst,
    melden: &Melder,
    wartezeit: &mut Wartezeit,
    idle_fehlstarts: &mut u32,
) -> Result<(), Fehlermeldung> {
    let konto = dienst.konten().finden(konto_id).await?;
    let passwort = dienst.konten().passwort(konto_id)?;
    let mut sitzung = anmelden(&konto.imap, &konto.benutzername, &passwort).await?;
    drop(passwort);

    let faehigkeiten = sitzung.capabilities().await.map_err(fehler_einordnen)?;
    let idle = faehigkeiten.has_str("IDLE") && *idle_fehlstarts < IDLE_MAX_FEHLSTARTS;
    // Nur lesend öffnen: Die Beobachtung ändert nichts am Postfach.
    sitzung
        .examine(POSTEINGANG)
        .await
        .map_err(fehler_einordnen)?;

    melden(Ereignis::Verbunden {
        konto: konto_id.to_string(),
        idle,
    });
    abgleichen_und_melden(konto_id, dienst, melden).await?;
    // Erst nach einem erfolgreichen Abgleich: Scheitert er dauerhaft, wächst die Wartezeit
    // weiter, statt sich jede Sekunde neu anzumelden.
    wartezeit.zuruecksetzen();

    loop {
        if idle {
            sitzung = match idle_warten(sitzung).await {
                Ok(sitzung) => {
                    *idle_fehlstarts = 0;
                    sitzung
                }
                Err(IdleFehler::Start(fehler)) => {
                    // Nur echte Ablehnungen zählen, keine Verbindungsabbrüche.
                    if !fehler.neu_verbinden_sinnvoll() {
                        *idle_fehlstarts += 1;
                    }
                    return Err(fehler.into());
                }
                Err(IdleFehler::Warten(fehler)) => return Err(fehler.into()),
            };
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
    // Neue Mails gleich mitladen, damit sie auch ohne Internet lesbar sind.
    if dienst.vorladen() {
        if let Err(fehler) = dienst
            .inhalte_vorladen(konto_id, POSTEINGANG, VORLADEN_ANZAHL)
            .await
        {
            if crate::imap::dienst::ist_offline_fehler(&fehler) {
                return Err(fehler);
            }
        }
    }
    Ok(())
}

/// Wo IDLE gescheitert ist: beim Start (Server lehnt ab) oder danach (Verbindung).
enum IdleFehler {
    Start(VerbindungsFehler),
    Warten(VerbindungsFehler),
}

/// Wartet per IDLE auf eine Änderung (oder bis zur Erneuerung) und gibt die Sitzung zurück.
///
/// Das abschliessende `DONE` muss innerhalb einer Minute bestätigt werden; so fällt eine
/// tote Verbindung spätestens bei der Erneuerung auf.
async fn idle_warten(sitzung: ImapSitzung) -> Result<ImapSitzung, IdleFehler> {
    let mut griff = sitzung.idle();
    timeout(Duration::from_secs(60), griff.init())
        .await
        .map_err(|_| IdleFehler::Start(VerbindungsFehler::Zeitueberschreitung))?
        .map_err(|f| IdleFehler::Start(fehler_einordnen(f)))?;
    let (warten, _stopp) = griff.wait_with_timeout(IDLE_DAUER);
    let antwort = warten
        .await
        .map_err(|f| IdleFehler::Warten(fehler_einordnen(f)))?;
    let sitzung = timeout(Duration::from_secs(60), griff.done())
        .await
        .map_err(|_| IdleFehler::Warten(VerbindungsFehler::Zeitueberschreitung))?
        .map_err(|f| IdleFehler::Warten(fehler_einordnen(f)))?;
    match antwort {
        IdleResponse::NewData(_) | IdleResponse::Timeout => Ok(sitzung),
        IdleResponse::ManualInterrupt => Err(IdleFehler::Warten(VerbindungsFehler::Unterbrochen)),
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
