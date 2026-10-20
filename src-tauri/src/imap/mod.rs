//! Mails über IMAP empfangen.
//!
//! - `verbindung`: verschlüsselte Verbindung und Anmeldung
//! - `ordner`: Ordnerliste
//! - `cache`: Schnittstelle zum Zwischenspeicher
//! - `sync`: Abgleich über UIDVALIDITY und UIDs
//! - `dienst`: Befehle mit wiederverwendeter Verbindung pro Konto
//! - `suche`: Suche auf dem Server (`UID SEARCH`, nur auf Wunsch)
//! - `beobachter`: neue Mails per IDLE (oder Abfrage alle 2 Minuten), automatisches Wiederverbinden
//!
//! Mails werden auf dem Server nie automatisch gelöscht: Es gibt kein EXPUNGE ohne
//! ausdrückliche Aktion des Nutzers, und CLOSE (das gelöschte Mails endgültig entfernt)
//! wird nie gesendet.

pub mod beobachter;
pub mod cache;
pub mod dienst;
pub mod ordner;
pub mod suche;
pub mod sync;
pub mod verbindung;

pub use verbindung::{anmelden, ImapSitzung};

use crate::fehler::VerbindungsFehler;

/// Ordnet Fehler von async-imap verständlichen Meldungen zu.
pub(crate) fn fehler_einordnen(fehler: async_imap::error::Error) -> VerbindungsFehler {
    use async_imap::error::Error;
    match fehler {
        Error::Io(_) | Error::ConnectionLost => VerbindungsFehler::Unterbrochen,
        Error::No(_) | Error::Bad(_) => {
            VerbindungsFehler::Protokoll("Der IMAP-Server hat den Befehl abgelehnt.".into())
        }
        _ => VerbindungsFehler::Protokoll("Der IMAP-Server hat unerwartet geantwortet.".into()),
    }
}
