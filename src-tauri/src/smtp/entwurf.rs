//! Entwürfe: was der Nutzer im Verfassen-Fenster eingibt.

use serde::{Deserialize, Serialize};

use super::anhaenge::{AnhangAuswahl, AnhangRegister, Eintrag};
use crate::fehler::Fehlermeldung;

/// Ein Entwurf, wie ihn die Oberfläche sendet und erhält.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entwurf {
    /// Leer bei einem neuen Entwurf.
    #[serde(default)]
    pub id: Option<String>,
    pub konto: String,
    /// Empfänger, durch Komma oder Semikolon getrennt, z. B. `Anna <anna@example.org>, bea@example.org`.
    #[serde(default)]
    pub an: String,
    #[serde(default)]
    pub cc: String,
    #[serde(default)]
    pub bcc: String,
    #[serde(default)]
    pub betreff: String,
    #[serde(default)]
    pub text: String,
    /// Zusätzlich eine einfache HTML-Fassung des Textes mitsenden.
    #[serde(default)]
    pub html: bool,
    /// Message-ID der beantworteten Mail (ohne spitze Klammern).
    #[serde(default)]
    pub antwort_auf: Option<String>,
    /// References der beantworteten Mail (ohne spitze Klammern).
    #[serde(default)]
    pub referenzen: Vec<String>,
    #[serde(default)]
    pub anhaenge: Vec<AnhangAuswahl>,
}

/// So wird ein Entwurf gespeichert: Anhänge mit ihrer Quelle statt mit Token,
/// damit sie nach einem Neustart wieder verfügbar sind.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct GespeicherterEntwurf {
    #[serde(flatten)]
    entwurf: Entwurf,
    #[serde(default)]
    anhang_quellen: Vec<Eintrag>,
}

/// Macht aus einem Entwurf den zu speichernden Text (JSON).
pub fn speicherform(entwurf: &Entwurf, register: &AnhangRegister) -> Result<String, Fehlermeldung> {
    let anhang_quellen = entwurf
        .anhaenge
        .iter()
        .map(|a| register.holen(&a.token))
        .collect::<Result<Vec<_>, _>>()?;
    let mut ohne_token = entwurf.clone();
    ohne_token.anhaenge.clear();
    serde_json::to_string(&GespeicherterEntwurf {
        entwurf: ohne_token,
        anhang_quellen,
    })
    .map_err(|_| Fehlermeldung::neu("intern", "Der Entwurf konnte nicht gespeichert werden."))
}

/// Liest einen gespeicherten Entwurf und trägt seine Anhänge neu ins Register ein.
pub fn aus_speicherform(
    id: &str,
    daten: &str,
    register: &AnhangRegister,
) -> Result<Entwurf, Fehlermeldung> {
    let gespeichert: GespeicherterEntwurf = serde_json::from_str(daten)
        .map_err(|_| Fehlermeldung::neu("intern", "Ein Entwurf ist beschädigt."))?;
    let mut entwurf = gespeichert.entwurf;
    entwurf.id = Some(id.to_string());
    entwurf.anhaenge = gespeichert
        .anhang_quellen
        .into_iter()
        .map(|e| register.eintragen(e))
        .collect();
    Ok(entwurf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::smtp::anhaenge::Quelle;

    #[test]
    fn entwurf_mit_anhang_speichern_und_laden() {
        let register = AnhangRegister::default();
        let anhang = register.eintragen(Eintrag {
            name: "a.pdf".into(),
            typ: "application/pdf".into(),
            groesse: 3,
            quelle: Quelle::Mail {
                konto: "k".into(),
                ordner: "INBOX".into(),
                uid: 7,
                index: 0,
            },
        });
        let entwurf = Entwurf {
            konto: "k".into(),
            an: "anna@example.org".into(),
            betreff: "Hallo".into(),
            text: "Text".into(),
            anhaenge: vec![anhang.clone()],
            ..Default::default()
        };
        let daten = speicherform(&entwurf, &register).unwrap();
        assert!(
            !daten.contains(&anhang.token),
            "Token wird nicht gespeichert"
        );

        let neues_register = AnhangRegister::default();
        let geladen = aus_speicherform("e1", &daten, &neues_register).unwrap();
        assert_eq!(geladen.id.as_deref(), Some("e1"));
        assert_eq!(geladen.betreff, "Hallo");
        assert_eq!(geladen.anhaenge.len(), 1);
        assert_eq!(geladen.anhaenge[0].name, "a.pdf");
        assert!(neues_register.holen(&geladen.anhaenge[0].token).is_ok());
    }
}
