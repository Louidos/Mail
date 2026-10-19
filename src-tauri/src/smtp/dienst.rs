//! Mails senden, im Ordner „Gesendet“ ablegen und bei Fehlern als Entwurf sichern.

use std::sync::Arc;

use serde::Serialize;
use tokio::time::timeout;
use zeroize::Zeroizing;

use super::anhaenge::{datei_lesen_begrenzt, AnhangRegister, Quelle, MAX_GESAMT};
use super::entwurf::{aus_speicherform, speicherform, Entwurf};
use super::nachricht::{self, AnhangDaten};
use crate::fehler::{Fehlermeldung, VerbindungsFehler};
use crate::imap::dienst::ImapDienst;
use crate::konto::KontoVerwaltung;
use crate::pgp::lesen::{Verfahren, Verschluesselung};
use crate::pgp::{PgpDienst, VersandSchluessel};
use crate::smime::schreiben::Versand as SmimeVersand;
use crate::smime::SmimeDienst;
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
    /// Zum Entschlüsseln weitergeleiteter Anhänge aus verschlüsselten Mails.
    pgp: Option<Arc<PgpDienst>>,
    /// S/MIME (#56): Verschlüsseln, Signieren und weitergeleitete Anhänge.
    smime: Option<Arc<SmimeDienst>>,
    pub register: AnhangRegister,
}

/// Vorbereiteter Schutz für den Versand.
enum Vorbereitet {
    Pgp(Box<VersandSchluessel>),
    Smime(SmimeVersand),
}

impl SendeDienst {
    pub fn neu(konten: Arc<KontoVerwaltung>, imap: Arc<ImapDienst>) -> Self {
        Self {
            konten,
            imap,
            pgp: None,
            smime: None,
            register: AnhangRegister::default(),
        }
    }

    /// Mit OpenPGP: Anhänge verschlüsselter Mails werden aus dem Klartext übernommen.
    pub fn mit_pgp(mut self, pgp: Arc<PgpDienst>) -> Self {
        self.pgp = Some(pgp);
        self
    }

    /// Mit S/MIME: Verschlüsseln, Signieren und Anhänge S/MIME-verschlüsselter Mails.
    pub fn mit_smime(mut self, smime: Arc<SmimeDienst>) -> Self {
        self.smime = Some(smime);
        self
    }

    /// Die Mail mit Anhängen so, wie sie angezeigt wird (bei OpenPGP bzw. S/MIME entschlüsselt).
    async fn mail_lesbar(
        &self,
        konto: &str,
        ordner: &str,
        uid: u32,
    ) -> Result<Zeroizing<Vec<u8>>, Fehlermeldung> {
        let roh = self.imap.inhalt(konto, ordner, uid).await?;
        if self.pgp.is_none() && self.smime.is_none() {
            return Ok(Zeroizing::new(roh));
        }
        let (pgp, smime) = (self.pgp.clone(), self.smime.clone());
        let kopie = roh.clone();
        let verarbeitet = tokio::task::spawn_blocking(move || {
            if let Some(smime) = smime {
                let ergebnis = smime.verarbeiten(&kopie);
                if ergebnis.status != Default::default() {
                    return ergebnis;
                }
            }
            pgp.map(|p| p.verarbeiten(&kopie)).unwrap_or_default()
        })
        .await
        .map_err(|_| Fehlermeldung::neu("intern", "Interner Fehler beim Entschlüsseln."))?;
        if verarbeitet.status.verschluesselung == Verschluesselung::NichtEntschluesselt {
            return Err(Fehlermeldung::neu(
                "pgp",
                "Die Anhänge der weitergeleiteten Mail können nicht entschlüsselt werden.",
            ));
        }
        Ok(verarbeitet.roh.unwrap_or_else(|| Zeroizing::new(roh)))
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

    /// Bei „Verschlüsseln“ oder „Signieren“: Schlüssel bzw. Zertifikate des gewählten Verfahrens
    /// zusammenstellen oder mit klarer Meldung abbrechen.
    async fn schutz_vorbereiten(
        &self,
        entwurf: &Entwurf,
    ) -> Result<Option<Vorbereitet>, Fehlermeldung> {
        if !entwurf.verschluesseln && !entwurf.signieren {
            return Ok(None);
        }
        let adressen = nachricht::alle_empfaenger(entwurf)?;
        let (konto, verschluesseln, signieren) = (
            entwurf.konto.clone(),
            entwurf.verschluesseln,
            entwurf.signieren,
        );
        match entwurf.verfahren {
            Verfahren::OpenPgp => {
                let pgp = self
                    .pgp
                    .clone()
                    .ok_or_else(|| Fehlermeldung::neu("pgp", "OpenPGP ist nicht verfügbar."))?;
                tokio::task::spawn_blocking(move || {
                    pgp.versand_vorbereiten(&konto, &adressen, verschluesseln, signieren)
                })
                .await
                .map_err(|_| Fehlermeldung::neu("intern", "Interner Fehler bei OpenPGP."))?
                .map(|v| Some(Vorbereitet::Pgp(Box::new(v))))
            }
            Verfahren::Smime => {
                let smime = self
                    .smime
                    .clone()
                    .ok_or_else(|| Fehlermeldung::neu("smime", "S/MIME ist nicht verfügbar."))?;
                tokio::task::spawn_blocking(move || {
                    smime.versand_vorbereiten(&konto, &adressen, verschluesseln, signieren)
                })
                .await
                .map_err(|_| Fehlermeldung::neu("intern", "Interner Fehler bei S/MIME."))?
                .map(|v| Some(Vorbereitet::Smime(v)))
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
                    let roh = self.mail_lesbar(konto, ordner, *uid).await?;
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
        // Schlüssel zuerst: Fehlt etwas, wird gar nichts gesendet (nie unbemerkt unverschlüsselt).
        let schutz = self.schutz_vorbereiten(entwurf).await?;
        let anhaenge = self.anhaenge_laden(entwurf).await?;
        let nachricht = {
            let (konto, entwurf) = (konto.clone(), entwurf.clone());
            // Verschlüsseln und Schlüsselableitung brauchen Rechenzeit.
            tokio::task::spawn_blocking(move || {
                let schutz = schutz.as_ref().map(|s| match s {
                    Vorbereitet::Pgp(p) => nachricht::Schutz::Pgp(p),
                    Vorbereitet::Smime(s) => nachricht::Schutz::Smime(s),
                });
                nachricht::bauen_geschuetzt(&konto, &entwurf, &anhaenge, schutz)
            })
            .await
            .map_err(|_| {
                Fehlermeldung::neu("intern", "Interner Fehler beim Erstellen der Mail.")
            })??
        };

        let passwort = self.konten.passwort(&konto.id)?;
        let mut verbindung = super::anmelden(&konto.smtp, &konto.benutzername, &passwort).await?;
        drop(passwort);
        for (nr, sendung) in nachricht.sendungen.iter().enumerate() {
            let versand = verbindung.send(&sendung.umschlag, &sendung.daten);
            let fehler = match timeout(VERBINDUNGS_ZEITLIMIT * 6, versand).await {
                Err(_) => Some(Fehlermeldung::from(VerbindungsFehler::Zeitueberschreitung)),
                Ok(Err(fehler)) => Some(Fehlermeldung::neu(
                    "senden",
                    if fehler.is_permanent() {
                        "Der SMTP-Server hat die Mail abgelehnt. Bitte Empfänger und Anhänge prüfen."
                    } else {
                        "Die Mail konnte nicht an den SMTP-Server übergeben werden. Bitte später erneut versuchen."
                    },
                )),
                Ok(Ok(_)) => None,
            };
            if let Some(mut fehler) = fehler {
                if nr > 0 {
                    let fehlend: Vec<String> = nachricht.sendungen[nr..]
                        .iter()
                        .flat_map(|s| s.umschlag.to().iter().map(|a| a.to_string()))
                        .collect();
                    fehler.meldung = format!(
                        "{} Die Mail ging bereits an einen Teil der Empfänger, nicht aber an: {}.",
                        fehler.meldung,
                        fehlend.join(", ")
                    );
                }
                return Err(fehler);
            }
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
