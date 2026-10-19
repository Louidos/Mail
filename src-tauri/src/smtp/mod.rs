//! Mails über SMTP senden.
//!
//! - `verbindung`: verschlüsselte Verbindung und Anmeldung
//! - `nachricht`: aus einem Entwurf eine Mail bauen
//! - `entwurf`: Entwürfe (lokal gespeichert, verschlüsselt wenn der lokale Speicher an ist)
//! - `anhaenge`: Register der gewählten Anhänge (das Frontend sieht keine Dateipfade)
//! - `dienst`: senden, im Ordner „Gesendet“ ablegen, bei Fehlern als Entwurf sichern

pub mod anhaenge;
pub mod dienst;
pub mod entwurf;
pub mod nachricht;
pub mod verbindung;

pub use verbindung::{anmelden, SmtpVerbindung};
