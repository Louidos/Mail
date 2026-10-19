//! Fehler, die dem Nutzer verständlich angezeigt werden.
//!
//! Meldungen enthalten nie Passwörter oder andere Zugangsdaten.

use serde::Serialize;

/// Fehler beim Verbinden mit einem Mailserver.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VerbindungsFehler {
    #[error("Der Server {server} ist nicht erreichbar. Stimmen Servername und Port?")]
    NichtErreichbar { server: String },

    #[error("Der Server antwortet nicht rechtzeitig.")]
    Zeitueberschreitung,

    #[error(
        "Zertifikat ungültig: {grund} Die Verbindung wurde aus Sicherheitsgründen abgebrochen."
    )]
    ZertifikatUngueltig { grund: String },

    #[error("Der Server bietet kein STARTTLS an. Die Verbindung wurde abgebrochen, statt unverschlüsselt weiterzumachen.")]
    KeinStartTls,

    #[error("Anmeldung fehlgeschlagen. Bitte Benutzername und Passwort prüfen.")]
    AnmeldungFalsch,

    #[error("{0}")]
    Protokoll(String),

    #[error("{0}")]
    Eingabe(String),
}

impl VerbindungsFehler {
    /// Kurzer, maschinenlesbarer Name der Fehlerart (für die Oberfläche und Tests).
    pub fn art(&self) -> &'static str {
        match self {
            Self::NichtErreichbar { .. } => "nicht_erreichbar",
            Self::Zeitueberschreitung => "zeitueberschreitung",
            Self::ZertifikatUngueltig { .. } => "zertifikat_ungueltig",
            Self::KeinStartTls => "kein_starttls",
            Self::AnmeldungFalsch => "anmeldung_falsch",
            Self::Protokoll(_) => "protokoll",
            Self::Eingabe(_) => "eingabe",
        }
    }
}

/// Fehler, wie er an die Oberfläche geht.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Fehlermeldung {
    /// Maschinenlesbare Art, z. B. `anmeldung_falsch`.
    pub art: String,
    /// Verständlicher Text für den Nutzer.
    pub meldung: String,
}

impl Fehlermeldung {
    pub fn neu(art: &str, meldung: impl Into<String>) -> Self {
        Self {
            art: art.to_string(),
            meldung: meldung.into(),
        }
    }
}

impl From<VerbindungsFehler> for Fehlermeldung {
    fn from(fehler: VerbindungsFehler) -> Self {
        Self::neu(fehler.art(), fehler.to_string())
    }
}

impl std::fmt::Display for Fehlermeldung {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.meldung)
    }
}
