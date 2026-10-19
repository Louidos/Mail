//! Mails senden, im Ordner „Gesendet“ ablegen und bei Fehlern als Entwurf sichern.

use std::sync::Arc;

use serde::Serialize;
use tokio::time::timeout;

use super::anhaenge::{datei_lesen_begrenzt, AnhangRegister, Quelle, MAX_GESAMT};
use super::entwurf::{aus_speicherform, speicherform, Entwurf};
use super::nachricht::{self, AnhangDaten};
use crate::fehler::{Fehlermeldung, VerbindungsFehler};
use crate::imap::dienst::ImapDienst;
use crate::konto::KontoVerwaltung;
use crate::tls::VERBINDUNGS_ZEITLIMIT;

/// Ergebnis eines erfolgreichen Versands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SendeErgebnis {
    /// Wurde die Mail im Ordner „Gesendet“ abgelegt?
    pub abgelegt: bool,
    /// Hinweis, z. B. wenn das Ablegen nicht geklappt hat.
    pub hinweis: Option<String>,
}

pub struct SendeDienst {
    konten: Arc<KontoVerwaltung>,
    imap: Arc<ImapDienst>,
    pub register: AnhangRegister,
}

impl SendeDienst {
    pub fn neu(konten: Arc<KontoVerwaltung>, imap: Arc<ImapDienst>) -> Self {
        Self {
            konten,
            imap,
            register: AnhangRegister::default(),
        }
    }

    /// Sendet einen Entwurf. Schlägt das Senden fehl, wird der Entwurf gespeichert,
    /// damit kein Text verloren geht. Nach Erfolg wird ein gespeicherter Entwurf gelöscht.
    pub async fn senden(&self, entwurf: &Entwurf) -> Result<SendeErgebnis, Fehlermeldung> {
        match self.senden_ohne_sicherung(entwurf).await {
            Ok(ergebnis) => {
                if let Some(id) = &entwurf.id {
                    let _ = self.imap.cache().entwurf_loeschen(id);
                }
                Ok(ergebnis)
            }
            Err(fehler) => {
                let zusatz = match self.entwurf_speichern(entwurf).await {
                    Ok(_) => "Die Mail wurde nicht gesendet und als Entwurf gespeichert.",
                    Err(_) => "Die Mail wurde nicht gesendet. Bitte den Text sichern, bevor du das Fenster schliesst.",
                };
                Err(Fehlermeldung::neu(
                    &fehler.art,
                    format!("{} {zusatz}", fehler.meldung),
                ))
            }
        }
    }

    async fn anhaenge_laden(&self, entwurf: &Entwurf) -> Result<Vec<AnhangDaten>, Fehlermeldung> {
        let zu_gross = || {
            Fehlermeldung::neu(
                "anhang",
                "Die Anhänge sind zusammen grösser als 25 MB. Viele Mailserver nehmen solche Mails nicht an.",
            )
        };
        let mut daten = Vec::new();
        let mut gesamt: u64 = 0;
        for auswahl in &entwurf.anhaenge {
            let eintrag = self.register.holen(&auswahl.token)?;
            // Schnelle Vorprüfung mit der bekannten Grösse, bevor etwas gelesen wird.
            if gesamt.saturating_add(eintrag.groesse) > MAX_GESAMT {
                return Err(zu_gross());
            }
            let inhalt = match &eintrag.quelle {
                Quelle::Datei { pfad } => datei_lesen_begrenzt(pfad, MAX_GESAMT - gesamt)
                    .await
                    .map_err(|_| {
                        Fehlermeldung::neu(
                            "anhang",
                            format!("Der Anhang „{}“ konnte nicht gelesen werden.", eintrag.name),
                        )
                    })?
                    .ok_or_else(zu_gross)?,
                Quelle::Mail {
                    konto,
                    ordner,
                    uid,
                    index,
                } => {
                    let roh = self.imap.inhalt(konto, ordner, *uid).await?;
                    crate::anzeige::anhang_inhalt(&roh, *index)?.1
                }
            };
            // Massgebend ist die tatsächlich gelesene Grösse.
            gesamt += inhalt.len() as u64;
            if gesamt > MAX_GESAMT {
                return Err(zu_gross());
            }
            daten.push(AnhangDaten {
                name: eintrag.name,
                typ: eintrag.typ,
                daten: inhalt,
            });
        }
        Ok(daten)
    }

    async fn senden_ohne_sicherung(
        &self,
        entwurf: &Entwurf,
    ) -> Result<SendeErgebnis, Fehlermeldung> {
        let konto = self.konten.finden(&entwurf.konto).await?;
        let anhaenge = self.anhaenge_laden(entwurf).await?;
        let nachricht = nachricht::bauen(&konto, entwurf, anhaenge)?;

        let passwort = self.konten.passwort(&konto.id)?;
        let mut verbindung = super::anmelden(&konto.smtp, &konto.benutzername, &passwort).await?;
        drop(passwort);
        let versand = verbindung.send(&nachricht.umschlag, &nachricht.zum_senden);
        match timeout(VERBINDUNGS_ZEITLIMIT * 6, versand).await {
            Err(_) => return Err(VerbindungsFehler::Zeitueberschreitung.into()),
            Ok(Err(fehler)) => {
                let meldung = if fehler.is_permanent() {
                    "Der SMTP-Server hat die Mail abgelehnt. Bitte Empfänger und Anhänge prüfen."
                } else {
                    "Die Mail konnte nicht an den SMTP-Server übergeben werden. Bitte später erneut versuchen."
                };
                return Err(Fehlermeldung::neu("senden", meldung));
            }
            Ok(Ok(_)) => {}
        }
        let _ = verbindung.quit().await;

        if !konto.gesendet_ablegen {
            return Ok(SendeErgebnis {
                abgelegt: false,
                hinweis: None,
            });
        }
        match self
            .imap
            .gesendet_ablegen(&konto.id, &nachricht.fuer_gesendet)
            .await
        {
            Ok(()) => Ok(SendeErgebnis {
                abgelegt: true,
                hinweis: None,
            }),
            Err(fehler) => Ok(SendeErgebnis {
                abgelegt: false,
                hinweis: Some(format!(
                    "Die Mail wurde gesendet, konnte aber nicht im Ordner „Gesendet“ abgelegt werden: {}",
                    fehler.meldung
                )),
            }),
        }
    }

    /// Speichert einen Entwurf (neu oder überschreibend) und gibt ihn mit ID zurück.
    pub async fn entwurf_speichern(&self, entwurf: &Entwurf) -> Result<Entwurf, Fehlermeldung> {
        let mut gespeichert = entwurf.clone();
        let id = entwurf
            .id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        gespeichert.id = Some(id.clone());
        let daten = speicherform(&gespeichert, &self.register)?;
        self.imap
            .cache()
            .entwurf_speichern(&id, &entwurf.konto, &daten)?;
        Ok(gespeichert)
    }

    /// Alle gespeicherten Entwürfe eines Kontos, zuletzt geänderte zuerst.
    pub fn entwuerfe(&self, konto: &str) -> Result<Vec<Entwurf>, Fehlermeldung> {
        self.imap
            .cache()
            .entwuerfe(konto)?
            .into_iter()
            .map(|(id, daten)| aus_speicherform(&id, &daten, &self.register))
            .collect()
    }

    pub fn entwurf_loeschen(&self, id: &str) -> Result<(), Fehlermeldung> {
        self.imap.cache().entwurf_loeschen(id)
    }
}
