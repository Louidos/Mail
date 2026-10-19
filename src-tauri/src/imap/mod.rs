//! Mails über IMAP empfangen.

pub mod verbindung;

pub use verbindung::{anmelden, ImapSitzung};
