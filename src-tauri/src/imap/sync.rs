//! Abgleich eines Ordners mit dem Zwischenspeicher über UIDVALIDITY und UIDs.
//!
//! Ablauf:
//! 1. Ordner auswählen; ändert sich die UIDVALIDITY, wird der Zwischenspeicher des Ordners geleert.
//! 2. Neue Mails: alle UIDs oberhalb der höchsten bekannten UID holen.
//! 3. Bekannte Mails: Markierungen abgleichen; was der Server nicht mehr hat, wird lokal entfernt.
//!
//! Da Mails über ihre UID gespeichert werden, kann eine Mail nach Verbindungsabbrüchen
//! nie doppelt erscheinen; da immer ab der höchsten bekannten UID nachgeladen wird, fehlt keine.
//! Auf dem Server wird dabei nichts verändert und nichts gelöscht.

use std::collections::HashSet;

use async_imap::types::{Fetch, Flag};
use futures_util::TryStreamExt;
use mail_parser::MessageParser;

use super::cache::{MailCache, MailKopf, OrdnerStand};
use super::{fehler_einordnen, ImapSitzung};
use crate::fehler::{Fehlermeldung, VerbindungsFehler};

/// So viele Mails werden standardmässig beim ersten Abgleich eines Ordners geladen.
pub const ERSTE_SEITE: u32 = 100;

const KOPF_ABFRAGE: &str =
    "(UID FLAGS RFC822.SIZE INTERNALDATE BODY.PEEK[HEADER.FIELDS (FROM SUBJECT DATE)])";

/// Fehler beim Abgleich: Verbindung oder Zwischenspeicher.
#[derive(Debug)]
pub enum SyncFehler {
    Verbindung(VerbindungsFehler),
    Speicher(Fehlermeldung),
}

impl From<VerbindungsFehler> for SyncFehler {
    fn from(f: VerbindungsFehler) -> Self {
        Self::Verbindung(f)
    }
}

impl From<Fehlermeldung> for SyncFehler {
    fn from(f: Fehlermeldung) -> Self {
        Self::Speicher(f)
    }
}

impl From<SyncFehler> for Fehlermeldung {
    fn from(f: SyncFehler) -> Self {
        match f {
            SyncFehler::Verbindung(v) => v.into(),
            SyncFehler::Speicher(s) => s,
        }
    }
}

/// Ergebnis eines Abgleichs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncErgebnis {
    /// Anzahl Mails im Ordner auf dem Server.
    pub auf_server: u32,
    pub neu: usize,
    pub entfernt: usize,
    pub geaendert: bool,
}

/// Macht aus einer FETCH-Antwort einen Mailkopf. Mails mit \Deleted werden übersprungen.
pub fn kopf_aus_fetch(fetch: &Fetch) -> Option<MailKopf> {
    let uid = fetch.uid?;
    let mut gelesen = false;
    let mut markiert = false;
    for flag in fetch.flags() {
        match flag {
            Flag::Seen => gelesen = true,
            Flag::Flagged => markiert = true,
            Flag::Deleted => return None,
            _ => {}
        }
    }
    let kopfzeilen = fetch.header().unwrap_or_default();
    let nachricht = MessageParser::default().parse_headers(kopfzeilen);
    let (betreff, absender, absender_adresse, datum) = match &nachricht {
        Some(n) => {
            let von = n.from().and_then(|a| a.first());
            let adresse = von
                .and_then(|a| a.address())
                .unwrap_or_default()
                .to_string();
            let name = von
                .and_then(|a| a.name())
                .map(str::to_string)
                .filter(|n| !n.trim().is_empty())
                .unwrap_or_else(|| adresse.clone());
            (
                n.subject().unwrap_or_default().to_string(),
                name,
                adresse,
                n.date().map(|d| d.to_timestamp()),
            )
        }
        None => (String::new(), String::new(), String::new(), None),
    };
    let datum = datum.or_else(|| fetch.internal_date().map(|d| d.timestamp()));
    Some(MailKopf {
        uid,
        betreff,
        absender,
        absender_adresse,
        datum,
        gelesen,
        markiert,
        groesse: fetch.size.unwrap_or(0),
    })
}

async fn koepfe_holen(
    sitzung: &mut ImapSitzung,
    menge: &str,
    per_uid: bool,
) -> Result<Vec<MailKopf>, VerbindungsFehler> {
    let antworten: Vec<Fetch> = if per_uid {
        sitzung
            .uid_fetch(menge, KOPF_ABFRAGE)
            .await
            .map_err(fehler_einordnen)?
            .try_collect()
            .await
            .map_err(fehler_einordnen)?
    } else {
        sitzung
            .fetch(menge, KOPF_ABFRAGE)
            .await
            .map_err(fehler_einordnen)?
            .try_collect()
            .await
            .map_err(fehler_einordnen)?
    };
    Ok(antworten.iter().filter_map(kopf_aus_fetch).collect())
}

/// Gleicht einen Ordner ab. Beim ersten Abgleich werden die neuesten `erste_seite` Mails geladen.
pub async fn ordner_abgleichen(
    sitzung: &mut ImapSitzung,
    cache: &dyn MailCache,
    konto: &str,
    ordner: &str,
    erste_seite: u32,
) -> Result<SyncErgebnis, SyncFehler> {
    let postfach = sitzung.select(ordner).await.map_err(fehler_einordnen)?;
    let uidvalidity = postfach.uid_validity.unwrap_or(0);
    let uidnext = postfach.uid_next.unwrap_or(0);
    let mut ergebnis = SyncErgebnis {
        auf_server: postfach.exists,
        ..Default::default()
    };

    let vorher = cache.stand(konto, ordner)?;
    if vorher.map(|s| s.uidvalidity) != Some(uidvalidity) {
        ergebnis.geaendert = vorher.is_some();
    }
    // Leert den Ordner im Zwischenspeicher, falls sich die UIDVALIDITY geändert hat.
    cache.stand_setzen(
        konto,
        ordner,
        OrdnerStand {
            uidvalidity,
            uidnext,
        },
    )?;

    let bekannt = cache.uids(konto, ordner)?;
    if postfach.exists == 0 {
        if !bekannt.is_empty() {
            cache.entfernen(konto, ordner, &bekannt)?;
            ergebnis.entfernt = bekannt.len();
            ergebnis.geaendert = true;
        }
        return Ok(ergebnis);
    }

    let (Some(&kleinste), Some(&groesste)) = (bekannt.first(), bekannt.last()) else {
        // Erster Abgleich: die neuesten Mails über Sequenznummern.
        let start = postfach
            .exists
            .saturating_sub(erste_seite.max(1) - 1)
            .max(1);
        let koepfe = koepfe_holen(sitzung, &format!("{start}:*"), false).await?;
        cache.koepfe_speichern(konto, ordner, &koepfe)?;
        ergebnis.neu = koepfe.len();
        ergebnis.geaendert |= !koepfe.is_empty();
        return Ok(ergebnis);
    };

    // Neue Mails. „n:*“ liefert immer mindestens die letzte Mail, daher filtern.
    let neue: Vec<MailKopf> = koepfe_holen(sitzung, &format!("{}:*", groesste + 1), true)
        .await?
        .into_iter()
        .filter(|k| k.uid > groesste)
        .collect();
    cache.koepfe_speichern(konto, ordner, &neue)?;
    ergebnis.neu = neue.len();

    // Markierungen und gelöschte Mails im bekannten Bereich.
    let stand: Vec<Fetch> = sitzung
        .uid_fetch(format!("{kleinste}:{groesste}"), "(UID FLAGS)")
        .await
        .map_err(fehler_einordnen)?
        .try_collect()
        .await
        .map_err(fehler_einordnen)?;
    let bekannt_menge: HashSet<u32> = bekannt.iter().copied().collect();
    let mut vorhanden = HashSet::new();
    for antwort in &stand {
        let Some(uid) = antwort.uid else { continue };
        if !bekannt_menge.contains(&uid) {
            continue;
        }
        let flags: Vec<Flag> = antwort.flags().collect();
        if flags.contains(&Flag::Deleted) {
            continue;
        }
        vorhanden.insert(uid);
        let gelesen = flags.contains(&Flag::Seen);
        let markiert = flags.contains(&Flag::Flagged);
        if let Some(kopf) = cache.kopf(konto, ordner, uid)? {
            if kopf.gelesen != gelesen || kopf.markiert != markiert {
                cache.markierungen_setzen(konto, ordner, uid, gelesen, markiert)?;
                ergebnis.geaendert = true;
            }
        }
    }
    let weg: Vec<u32> = bekannt
        .iter()
        .copied()
        .filter(|uid| !vorhanden.contains(uid))
        .collect();
    if !weg.is_empty() {
        cache.entfernen(konto, ordner, &weg)?;
    }
    ergebnis.entfernt = weg.len();
    ergebnis.geaendert |= ergebnis.neu > 0 || ergebnis.entfernt > 0;
    Ok(ergebnis)
}

/// Lädt bis zu `anzahl` ältere Mails als die älteste bekannte (für „Mehr laden“).
///
/// Der Ordner muss vorher mit [`ordner_abgleichen`] ausgewählt worden sein.
pub async fn aeltere_laden(
    sitzung: &mut ImapSitzung,
    cache: &dyn MailCache,
    konto: &str,
    ordner: &str,
    anzahl: usize,
) -> Result<usize, SyncFehler> {
    let bekannt = cache.uids(konto, ordner)?;
    let grenze = match bekannt.first() {
        Some(1) => return Ok(0),
        Some(&kleinste) => kleinste - 1,
        None => u32::MAX,
    };
    let mut aeltere: Vec<u32> = sitzung
        .uid_search(format!("UID 1:{grenze} UNDELETED"))
        .await
        .map_err(fehler_einordnen)?
        .into_iter()
        .filter(|uid| *uid <= grenze)
        .collect();
    aeltere.sort_unstable();
    let auswahl: Vec<String> = aeltere
        .iter()
        .rev()
        .take(anzahl)
        .map(u32::to_string)
        .collect();
    if auswahl.is_empty() {
        return Ok(0);
    }
    let koepfe = koepfe_holen(sitzung, &auswahl.join(","), true).await?;
    cache.koepfe_speichern(konto, ordner, &koepfe)?;
    Ok(koepfe.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kopfzeilen_mit_kodiertem_betreff() {
        let kopf = "From: =?UTF-8?Q?J=C3=BCrg_M=C3=BCller?= <juerg@example.org>\r\nSubject: =?UTF-8?B?R3LDvGVzc2U=?=\r\nDate: Mon, 19 Oct 2026 10:00:00 +0200\r\n\r\n";
        let nachricht = MessageParser::default()
            .parse_headers(kopf.as_bytes())
            .unwrap();
        assert_eq!(nachricht.subject(), Some("Grüesse"));
        let von = nachricht.from().unwrap().first().unwrap();
        assert_eq!(von.name(), Some("Jürg Müller"));
        assert_eq!(von.address(), Some("juerg@example.org"));
        // 10:00 Uhr MESZ = 08:00 Uhr UTC
        assert_eq!(nachricht.date().unwrap().to_timestamp(), 1_792_396_800);
    }
}
