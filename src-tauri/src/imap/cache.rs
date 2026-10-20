//! Zwischenspeicher für Mailköpfe und -inhalte.
//!
//! Die Synchronisation arbeitet nur gegen diese Schnittstelle. Es gibt eine Ablage im
//! Arbeitsspeicher (hier) und eine verschlüsselte Ablage auf der Festplatte (`speicher`).

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

use serde::Serialize;

use crate::fehler::Fehlermeldung;
use crate::speicher::suche::{self, MailEintrag, Suche};

/// Kopfdaten einer Mail für die Mailliste.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MailKopf {
    pub uid: u32,
    pub betreff: String,
    /// Anzeigename des Absenders (oder die Adresse, falls kein Name).
    pub absender: String,
    pub absender_adresse: String,
    /// Unix-Zeit in Sekunden.
    pub datum: Option<i64>,
    pub gelesen: bool,
    pub markiert: bool,
    pub groesse: u32,
}

/// Stand eines Ordners beim letzten Abgleich.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrdnerStand {
    pub uidvalidity: u32,
}

/// Ablage für Mails eines oder mehrerer Konten.
pub trait MailCache: Send + Sync {
    fn stand(&self, konto: &str, ordner: &str) -> Result<Option<OrdnerStand>, Fehlermeldung>;
    /// Setzt den Stand. Ändert sich die UIDVALIDITY, wird der Ordner vorher geleert.
    fn stand_setzen(
        &self,
        konto: &str,
        ordner: &str,
        stand: OrdnerStand,
    ) -> Result<(), Fehlermeldung>;
    /// Fügt Köpfe ein oder ersetzt vorhandene mit derselben UID.
    fn koepfe_speichern(
        &self,
        konto: &str,
        ordner: &str,
        koepfe: &[MailKopf],
    ) -> Result<(), Fehlermeldung>;
    /// Alle UIDs des Ordners, aufsteigend.
    fn uids(&self, konto: &str, ordner: &str) -> Result<Vec<u32>, Fehlermeldung>;
    fn entfernen(&self, konto: &str, ordner: &str, uids: &[u32]) -> Result<(), Fehlermeldung>;
    fn markierungen_setzen(
        &self,
        konto: &str,
        ordner: &str,
        uid: u32,
        gelesen: bool,
        markiert: bool,
    ) -> Result<(), Fehlermeldung>;
    /// Köpfe sortiert von neu nach alt (nach UID), ab `start`.
    fn seite(
        &self,
        konto: &str,
        ordner: &str,
        start: usize,
        anzahl: usize,
    ) -> Result<Vec<MailKopf>, Fehlermeldung>;
    fn kopf(&self, konto: &str, ordner: &str, uid: u32) -> Result<Option<MailKopf>, Fehlermeldung>;
    fn inhalt(&self, konto: &str, ordner: &str, uid: u32)
        -> Result<Option<Vec<u8>>, Fehlermeldung>;
    /// Ist der Inhalt der Mail gespeichert?
    fn hat_inhalt(&self, konto: &str, ordner: &str, uid: u32) -> Result<bool, Fehlermeldung> {
        Ok(self.inhalt(konto, ordner, uid)?.is_some())
    }
    fn inhalt_speichern(
        &self,
        konto: &str,
        ordner: &str,
        uid: u32,
        inhalt: &[u8],
    ) -> Result<(), Fehlermeldung>;
    /// Entfernt alles zu einem Konto (auch Entwürfe).
    fn konto_entfernen(&self, konto: &str) -> Result<(), Fehlermeldung>;
    /// Entfernt alle zwischengespeicherten Mails. Entwürfe bleiben erhalten.
    fn leeren(&self) -> Result<(), Fehlermeldung>;

    /// Speichert (oder ersetzt) einen Entwurf als JSON.
    fn entwurf_speichern(&self, id: &str, konto: &str, daten: &str) -> Result<(), Fehlermeldung>;
    /// Alle Entwürfe eines Kontos als (ID, JSON), zuletzt geänderte zuerst.
    fn entwuerfe(&self, konto: &str) -> Result<Vec<(String, String)>, Fehlermeldung>;
    fn entwurf_loeschen(&self, id: &str) -> Result<(), Fehlermeldung>;

    /// Lokale Suche (#49): Treffer im Suchbereich, neueste zuerst, ab `start`.
    fn suchen(
        &self,
        suche: &Suche,
        start: usize,
        anzahl: usize,
    ) -> Result<Vec<MailEintrag>, Fehlermeldung>;

    /// Gemeinsamer Posteingang (#50): die Mails des Ordners `ordner` aller `konten` in einer
    /// Liste, neueste zuerst (nach Datum), ab `start`.
    fn ueber_konten(
        &self,
        konten: &[String],
        ordner: &str,
        start: usize,
        anzahl: usize,
    ) -> Result<Vec<MailEintrag>, Fehlermeldung>;

    /// Anzahl ungelesener Mails im Ordner `ordner` der `konten`.
    fn ungelesen(&self, konten: &[String], ordner: &str) -> Result<usize, Fehlermeldung>;
}

#[derive(Default)]
struct OrdnerDaten {
    stand: Option<OrdnerStand>,
    koepfe: BTreeMap<u32, MailKopf>,
    inhalte: HashMap<u32, Vec<u8>>,
}

/// Ablage im Arbeitsspeicher (wird beim Beenden verworfen).
#[derive(Default)]
pub struct SpeicherCache {
    daten: Mutex<HashMap<(String, String), OrdnerDaten>>,
    /// Entwürfe: ID → (Konto, JSON, laufende Nummer für die Reihenfolge)
    entwuerfe: Mutex<HashMap<String, (String, String, u64)>>,
    zaehler: std::sync::atomic::AtomicU64,
}

impl SpeicherCache {
    fn mit<T>(
        &self,
        konto: &str,
        ordner: &str,
        f: impl FnOnce(&mut OrdnerDaten) -> T,
    ) -> Result<T, Fehlermeldung> {
        let mut daten = self
            .daten
            .lock()
            .map_err(|_| Fehlermeldung::neu("intern", "Interner Fehler im Zwischenspeicher."))?;
        Ok(f(daten
            .entry((konto.to_string(), ordner.to_string()))
            .or_default()))
    }
}

impl MailCache for SpeicherCache {
    fn stand(&self, konto: &str, ordner: &str) -> Result<Option<OrdnerStand>, Fehlermeldung> {
        self.mit(konto, ordner, |d| d.stand)
    }

    fn stand_setzen(
        &self,
        konto: &str,
        ordner: &str,
        stand: OrdnerStand,
    ) -> Result<(), Fehlermeldung> {
        self.mit(konto, ordner, |d| {
            if d.stand.map(|s| s.uidvalidity) != Some(stand.uidvalidity) {
                d.koepfe.clear();
                d.inhalte.clear();
            }
            d.stand = Some(stand);
        })
    }

    fn koepfe_speichern(
        &self,
        konto: &str,
        ordner: &str,
        koepfe: &[MailKopf],
    ) -> Result<(), Fehlermeldung> {
        self.mit(konto, ordner, |d| {
            for kopf in koepfe {
                d.koepfe.insert(kopf.uid, kopf.clone());
            }
        })
    }

    fn uids(&self, konto: &str, ordner: &str) -> Result<Vec<u32>, Fehlermeldung> {
        self.mit(konto, ordner, |d| d.koepfe.keys().copied().collect())
    }

    fn entfernen(&self, konto: &str, ordner: &str, uids: &[u32]) -> Result<(), Fehlermeldung> {
        self.mit(konto, ordner, |d| {
            for uid in uids {
                d.koepfe.remove(uid);
                d.inhalte.remove(uid);
            }
        })
    }

    fn markierungen_setzen(
        &self,
        konto: &str,
        ordner: &str,
        uid: u32,
        gelesen: bool,
        markiert: bool,
    ) -> Result<(), Fehlermeldung> {
        self.mit(konto, ordner, |d| {
            if let Some(kopf) = d.koepfe.get_mut(&uid) {
                kopf.gelesen = gelesen;
                kopf.markiert = markiert;
            }
        })
    }

    fn seite(
        &self,
        konto: &str,
        ordner: &str,
        start: usize,
        anzahl: usize,
    ) -> Result<Vec<MailKopf>, Fehlermeldung> {
        self.mit(konto, ordner, |d| {
            d.koepfe
                .values()
                .rev()
                .skip(start)
                .take(anzahl)
                .cloned()
                .collect()
        })
    }

    fn kopf(&self, konto: &str, ordner: &str, uid: u32) -> Result<Option<MailKopf>, Fehlermeldung> {
        self.mit(konto, ordner, |d| d.koepfe.get(&uid).cloned())
    }

    fn inhalt(
        &self,
        konto: &str,
        ordner: &str,
        uid: u32,
    ) -> Result<Option<Vec<u8>>, Fehlermeldung> {
        self.mit(konto, ordner, |d| d.inhalte.get(&uid).cloned())
    }

    fn inhalt_speichern(
        &self,
        konto: &str,
        ordner: &str,
        uid: u32,
        inhalt: &[u8],
    ) -> Result<(), Fehlermeldung> {
        self.mit(konto, ordner, |d| {
            if d.koepfe.contains_key(&uid) {
                d.inhalte.insert(uid, inhalt.to_vec());
            }
        })
    }

    fn konto_entfernen(&self, konto: &str) -> Result<(), Fehlermeldung> {
        let mut daten = self
            .daten
            .lock()
            .map_err(|_| Fehlermeldung::neu("intern", "Interner Fehler im Zwischenspeicher."))?;
        daten.retain(|(k, _), _| k != konto);
        if let Ok(mut entwuerfe) = self.entwuerfe.lock() {
            entwuerfe.retain(|_, (k, _, _)| k != konto);
        }
        Ok(())
    }

    fn leeren(&self) -> Result<(), Fehlermeldung> {
        let mut daten = self
            .daten
            .lock()
            .map_err(|_| Fehlermeldung::neu("intern", "Interner Fehler im Zwischenspeicher."))?;
        daten.clear();
        Ok(())
    }

    fn entwurf_speichern(&self, id: &str, konto: &str, daten: &str) -> Result<(), Fehlermeldung> {
        let nummer = self
            .zaehler
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.entwuerfe
            .lock()
            .map_err(|_| Fehlermeldung::neu("intern", "Interner Fehler im Zwischenspeicher."))?
            .insert(
                id.to_string(),
                (konto.to_string(), daten.to_string(), nummer),
            );
        Ok(())
    }

    fn entwuerfe(&self, konto: &str) -> Result<Vec<(String, String)>, Fehlermeldung> {
        let entwuerfe = self
            .entwuerfe
            .lock()
            .map_err(|_| Fehlermeldung::neu("intern", "Interner Fehler im Zwischenspeicher."))?;
        let mut liste: Vec<_> = entwuerfe
            .iter()
            .filter(|(_, (k, _, _))| k == konto)
            .map(|(id, (_, daten, nummer))| (*nummer, id.clone(), daten.clone()))
            .collect();
        liste.sort_by_key(|eintrag| std::cmp::Reverse(eintrag.0));
        Ok(liste
            .into_iter()
            .map(|(_, id, daten)| (id, daten))
            .collect())
    }

    fn entwurf_loeschen(&self, id: &str) -> Result<(), Fehlermeldung> {
        self.entwuerfe
            .lock()
            .map_err(|_| Fehlermeldung::neu("intern", "Interner Fehler im Zwischenspeicher."))?
            .remove(id);
        Ok(())
    }

    fn suchen(
        &self,
        suche: &Suche,
        start: usize,
        anzahl: usize,
    ) -> Result<Vec<MailEintrag>, Fehlermeldung> {
        let daten = self
            .daten
            .lock()
            .map_err(|_| Fehlermeldung::neu("intern", "Interner Fehler im Zwischenspeicher."))?;
        let mut treffer: Vec<MailEintrag> = daten
            .iter()
            .filter(|((konto, ordner), _)| suche.umfasst(konto, ordner))
            .flat_map(|((konto, ordner), d)| {
                d.koepfe
                    .values()
                    .filter(|kopf| {
                        suche::passt(&suche.begriff, suche.feld, kopf, || {
                            d.inhalte
                                .get(&kopf.uid)
                                .and_then(|roh| suche::suchtext(roh))
                        })
                    })
                    .map(|kopf| MailEintrag {
                        konto: konto.clone(),
                        ordner: ordner.clone(),
                        kopf: kopf.clone(),
                    })
            })
            .collect();
        treffer.sort_by(suche::nach_datum);
        Ok(treffer.into_iter().skip(start).take(anzahl).collect())
    }

    fn ueber_konten(
        &self,
        konten: &[String],
        ordner: &str,
        start: usize,
        anzahl: usize,
    ) -> Result<Vec<MailEintrag>, Fehlermeldung> {
        let daten = self
            .daten
            .lock()
            .map_err(|_| Fehlermeldung::neu("intern", "Interner Fehler im Zwischenspeicher."))?;
        let mut mails: Vec<MailEintrag> = daten
            .iter()
            .filter(|((konto, o), _)| o == ordner && konten.contains(konto))
            .flat_map(|((konto, o), d)| {
                d.koepfe.values().map(|kopf| MailEintrag {
                    konto: konto.clone(),
                    ordner: o.clone(),
                    kopf: kopf.clone(),
                })
            })
            .collect();
        mails.sort_by(suche::nach_datum);
        Ok(mails.into_iter().skip(start).take(anzahl).collect())
    }

    fn ungelesen(&self, konten: &[String], ordner: &str) -> Result<usize, Fehlermeldung> {
        let daten = self
            .daten
            .lock()
            .map_err(|_| Fehlermeldung::neu("intern", "Interner Fehler im Zwischenspeicher."))?;
        Ok(daten
            .iter()
            .filter(|((konto, o), _)| o == ordner && konten.contains(konto))
            .map(|(_, d)| d.koepfe.values().filter(|k| !k.gelesen).count())
            .sum())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::speicher::suche::{SuchFeld, SuchOrt};

    pub(crate) fn kopf(uid: u32) -> MailKopf {
        MailKopf {
            uid,
            betreff: format!("Mail {uid}"),
            absender: "Anna".into(),
            absender_adresse: "anna@example.org".into(),
            datum: Some(1_700_000_000 + i64::from(uid)),
            gelesen: false,
            markiert: false,
            groesse: 100,
        }
    }

    /// Prüft das Verhalten, das jede Ablage erfüllen muss.
    pub(crate) fn verhalten_pruefen(cache: &dyn MailCache) {
        let stand = OrdnerStand { uidvalidity: 7 };
        cache.stand_setzen("k", "INBOX", stand).unwrap();
        cache
            .koepfe_speichern("k", "INBOX", &[kopf(1), kopf(2), kopf(3)])
            .unwrap();
        // Doppelt speichern erzeugt keine Duplikate.
        cache.koepfe_speichern("k", "INBOX", &[kopf(2)]).unwrap();
        assert_eq!(cache.uids("k", "INBOX").unwrap(), vec![1, 2, 3]);

        let seite = cache.seite("k", "INBOX", 0, 2).unwrap();
        assert_eq!(seite.iter().map(|k| k.uid).collect::<Vec<_>>(), vec![3, 2]);
        let seite = cache.seite("k", "INBOX", 2, 2).unwrap();
        assert_eq!(seite.iter().map(|k| k.uid).collect::<Vec<_>>(), vec![1]);

        cache
            .markierungen_setzen("k", "INBOX", 2, true, true)
            .unwrap();
        let k2 = cache.kopf("k", "INBOX", 2).unwrap().unwrap();
        assert!(k2.gelesen && k2.markiert);

        cache.inhalt_speichern("k", "INBOX", 3, b"Inhalt").unwrap();
        assert_eq!(
            cache.inhalt("k", "INBOX", 3).unwrap().as_deref(),
            Some(&b"Inhalt"[..])
        );

        cache.entfernen("k", "INBOX", &[3]).unwrap();
        assert_eq!(cache.uids("k", "INBOX").unwrap(), vec![1, 2]);
        assert!(cache.inhalt("k", "INBOX", 3).unwrap().is_none());

        // Gleiche UIDVALIDITY: Inhalt bleibt.
        cache
            .stand_setzen("k", "INBOX", OrdnerStand { uidvalidity: 7 })
            .unwrap();
        assert_eq!(cache.uids("k", "INBOX").unwrap(), vec![1, 2]);
        // Neue UIDVALIDITY: Ordner wird geleert.
        cache
            .stand_setzen("k", "INBOX", OrdnerStand { uidvalidity: 8 })
            .unwrap();
        assert!(cache.uids("k", "INBOX").unwrap().is_empty());

        cache.koepfe_speichern("k", "INBOX", &[kopf(5)]).unwrap();
        cache
            .koepfe_speichern("anders", "INBOX", &[kopf(5)])
            .unwrap();
        cache.konto_entfernen("k").unwrap();
        assert!(cache.uids("k", "INBOX").unwrap().is_empty());
        assert_eq!(cache.uids("anders", "INBOX").unwrap(), vec![5]);
        cache.leeren().unwrap();
        assert!(cache.uids("anders", "INBOX").unwrap().is_empty());

        // Entwürfe: überleben „leeren“, neueste zuerst, werden mit dem Konto entfernt.
        cache.entwurf_speichern("e1", "k", "{\"a\":1}").unwrap();
        cache.entwurf_speichern("e2", "k", "{\"a\":2}").unwrap();
        cache.entwurf_speichern("e3", "anders", "{}").unwrap();
        cache.entwurf_speichern("e1", "k", "{\"a\":3}").unwrap();
        cache.leeren().unwrap();
        let liste = cache.entwuerfe("k").unwrap();
        assert_eq!(
            liste,
            vec![
                ("e1".to_string(), "{\"a\":3}".to_string()),
                ("e2".to_string(), "{\"a\":2}".to_string())
            ]
        );
        cache.entwurf_loeschen("e2").unwrap();
        assert_eq!(cache.entwuerfe("k").unwrap().len(), 1);
        cache.konto_entfernen("k").unwrap();
        assert!(cache.entwuerfe("k").unwrap().is_empty());
        assert_eq!(cache.entwuerfe("anders").unwrap().len(), 1);
    }

    fn mit_betreff(uid: u32, betreff: &str, absender: &str, adresse: &str) -> MailKopf {
        MailKopf {
            betreff: betreff.into(),
            absender: absender.into(),
            absender_adresse: adresse.into(),
            ..kopf(uid)
        }
    }

    fn uids_von(treffer: &[MailEintrag]) -> Vec<(String, String, u32)> {
        treffer
            .iter()
            .map(|t| (t.konto.clone(), t.ordner.clone(), t.kopf.uid))
            .collect()
    }

    fn suche(begriff: &str, feld: SuchFeld, orte: &[(&str, Option<&str>)]) -> Suche {
        Suche {
            begriff: begriff.into(),
            feld,
            orte: orte
                .iter()
                .map(|(konto, ordner)| SuchOrt {
                    konto: konto.to_string(),
                    ordner: ordner.map(str::to_string),
                })
                .collect(),
        }
    }

    /// Prüft die lokale Suche (#49), die jede Ablage erfüllen muss.
    pub(crate) fn suche_pruefen(cache: &dyn MailCache) {
        let ort = |o: &str, u: u32| ("k".to_string(), o.to_string(), u);
        cache
            .koepfe_speichern(
                "k",
                "INBOX",
                &[
                    mit_betreff(1, "Rechnung März", "Jürg Müller", "juerg@example.org"),
                    mit_betreff(2, "Ferien", "Anna", "anna@example.org"),
                ],
            )
            .unwrap();
        cache
            .koepfe_speichern(
                "k",
                "Archiv",
                &[mit_betreff(3, "Rechnung alt", "Bea", "bea@example.org")],
            )
            .unwrap();
        cache
            .koepfe_speichern(
                "anders",
                "INBOX",
                &[mit_betreff(4, "Rechnung", "Bea", "bea@example.org")],
            )
            .unwrap();
        cache
            .inhalt_speichern(
                "k",
                "INBOX",
                2,
                b"From: anna@example.org\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nDas Velo ist kaputt.\r\n",
            )
            .unwrap();

        let dieser_ordner = [("k", Some("INBOX"))];
        let ganzes_konto = [("k", None)];
        let finden = |begriff: &str, feld: SuchFeld, orte: &[(&str, Option<&str>)]| {
            uids_von(&cache.suchen(&suche(begriff, feld, orte), 0, 200).unwrap())
        };

        // Absender, Betreff, Inhalt; Wortanfänge genügen.
        assert_eq!(
            finden("rechnung", SuchFeld::Alle, &dieser_ordner),
            vec![ort("INBOX", 1)]
        );
        assert_eq!(
            finden("Rech", SuchFeld::Betreff, &dieser_ordner),
            vec![ort("INBOX", 1)]
        );
        assert_eq!(
            finden("müller", SuchFeld::Absender, &dieser_ordner),
            vec![ort("INBOX", 1)]
        );
        assert_eq!(
            finden("juerg@example.org", SuchFeld::Absender, &dieser_ordner),
            vec![ort("INBOX", 1)]
        );
        assert_eq!(
            finden("velo", SuchFeld::Inhalt, &dieser_ordner),
            vec![ort("INBOX", 2)]
        );
        assert_eq!(
            finden("velo", SuchFeld::Alle, &dieser_ordner),
            vec![ort("INBOX", 2)]
        );
        assert!(finden("velo", SuchFeld::Betreff, &dieser_ordner).is_empty());
        assert!(finden("Ferien", SuchFeld::Absender, &dieser_ordner).is_empty());
        // Alle Wörter müssen passen.
        assert_eq!(
            finden("rechnung märz", SuchFeld::Alle, &dieser_ordner),
            vec![ort("INBOX", 1)]
        );
        assert!(finden("rechnung ferien", SuchFeld::Alle, &dieser_ordner).is_empty());

        // Alle Ordner des Kontos, neueste zuerst; andere Konten nie.
        assert_eq!(
            finden("rechnung", SuchFeld::Alle, &ganzes_konto),
            vec![ort("Archiv", 3), ort("INBOX", 1)]
        );
        let seite = cache
            .suchen(&suche("rechnung", SuchFeld::Alle, &ganzes_konto), 1, 1)
            .unwrap();
        assert_eq!(uids_von(&seite), vec![ort("INBOX", 1)]);

        // Sonderzeichen sind nur Text: keine Fehler, keine falschen Treffer.
        for begriff in [
            "\"",
            "*",
            "\\",
            "\"*\\",
            "Rech\"nung",
            "NOT -x",
            "betreff:x",
            "NEAR(a b)",
            "😀",
            "",
        ] {
            let treffer = cache
                .suchen(&suche(begriff, SuchFeld::Alle, &ganzes_konto), 0, 200)
                .unwrap();
            assert!(treffer.is_empty(), "{begriff}: {treffer:?}");
        }

        // Entfernte Mails werden nicht mehr gefunden, geänderte mit neuem Text.
        cache.entfernen("k", "INBOX", &[1]).unwrap();
        assert!(finden("rechnung", SuchFeld::Alle, &dieser_ordner).is_empty());
        cache
            .koepfe_speichern(
                "k",
                "INBOX",
                &[mit_betreff(2, "Velo-Reparatur", "Anna", "anna@example.org")],
            )
            .unwrap();
        assert_eq!(
            finden("reparatur", SuchFeld::Betreff, &dieser_ordner),
            vec![ort("INBOX", 2)]
        );
        assert!(finden("ferien", SuchFeld::Alle, &dieser_ordner).is_empty());
        // Der Inhalt bleibt beim Aktualisieren der Kopfdaten im Index.
        assert_eq!(
            finden("kaputt", SuchFeld::Inhalt, &dieser_ordner),
            vec![ort("INBOX", 2)]
        );

        // Neue UIDVALIDITY leert den Ordner – auch im Index.
        cache
            .stand_setzen("k", "INBOX", OrdnerStand { uidvalidity: 1 })
            .unwrap();
        cache
            .stand_setzen("k", "INBOX", OrdnerStand { uidvalidity: 2 })
            .unwrap();
        assert!(finden("velo", SuchFeld::Alle, &dieser_ordner).is_empty());

        cache.konto_entfernen("k").unwrap();
        assert!(finden("rechnung", SuchFeld::Alle, &ganzes_konto).is_empty());
        assert_eq!(
            finden("rechnung", SuchFeld::Alle, &[("anders", None)]).len(),
            1
        );
        cache.leeren().unwrap();
        assert!(finden("rechnung", SuchFeld::Alle, &[("anders", None)]).is_empty());
    }

    /// Prüft den gemeinsamen Posteingang (#50): Mails mehrerer Konten gemischt nach Datum.
    pub(crate) fn ueber_konten_pruefen(cache: &dyn MailCache) {
        let mit_datum = |uid: u32, datum: Option<i64>, gelesen: bool| MailKopf {
            datum,
            gelesen,
            ..kopf(uid)
        };
        // Konto A hat hohe UIDs mit alten Daten, Konto B kleine UIDs mit neuen: sortiert wird
        // nach Datum, nicht nach UID.
        cache
            .koepfe_speichern(
                "a",
                "INBOX",
                &[
                    mit_datum(500, Some(100), true),
                    mit_datum(501, Some(300), false),
                    mit_datum(502, None, false),
                ],
            )
            .unwrap();
        cache
            .koepfe_speichern(
                "b",
                "INBOX",
                &[
                    mit_datum(1, Some(200), false),
                    mit_datum(2, Some(400), true),
                ],
            )
            .unwrap();
        cache
            .koepfe_speichern("a", "Archiv", &[mit_datum(9, Some(999), false)])
            .unwrap();
        cache
            .koepfe_speichern("c", "INBOX", &[mit_datum(7, Some(999), false)])
            .unwrap();
        let konten = ["a".to_string(), "b".to_string()];
        let liste =
            |start, anzahl| uids_von(&cache.ueber_konten(&konten, "INBOX", start, anzahl).unwrap());
        let ort = |k: &str, u: u32| (k.to_string(), "INBOX".to_string(), u);
        assert_eq!(
            liste(0, 10),
            vec![
                ort("b", 2),
                ort("a", 501),
                ort("b", 1),
                ort("a", 500),
                ort("a", 502)
            ]
        );
        // Seitenweise ohne Lücken und Doppelte.
        assert_eq!(liste(0, 2), vec![ort("b", 2), ort("a", 501)]);
        assert_eq!(liste(2, 2), vec![ort("b", 1), ort("a", 500)]);
        assert_eq!(liste(4, 2), vec![ort("a", 502)]);
        assert_eq!(cache.ungelesen(&konten, "INBOX").unwrap(), 3);
        assert_eq!(cache.ungelesen(&["c".to_string()], "INBOX").unwrap(), 1);
        assert!(cache.ueber_konten(&[], "INBOX", 0, 10).unwrap().is_empty());
        assert_eq!(cache.ungelesen(&[], "INBOX").unwrap(), 0);
    }

    #[test]
    fn speicher_cache_ueber_konten() {
        ueber_konten_pruefen(&SpeicherCache::default());
    }

    #[test]
    fn speicher_cache_verhalten() {
        verhalten_pruefen(&SpeicherCache::default());
    }

    #[test]
    fn speicher_cache_suche() {
        suche_pruefen(&SpeicherCache::default());
    }
}
