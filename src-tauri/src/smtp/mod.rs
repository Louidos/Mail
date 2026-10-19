//! Mails über SMTP senden.

pub mod verbindung;

pub use verbindung::{anmelden, SmtpVerbindung};
