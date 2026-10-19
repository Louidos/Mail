//! Absender prüfen: Ergebnisse von DKIM, SPF und DMARC aus dem Header `Authentication-Results`
//! (RFC 8601) des eigenen Servers auswerten (#45).
//!
//! - Keine eigenen Prüfungen, keine DNS-Abfragen, keine Verbindungen: Ausgewertet wird nur, was
//!   der eigene Server beim Empfang festgestellt hat.
//! - Vertraut wird nur dem obersten `Authentication-Results`-Header mit der Server-Kennung
//!   (authserv-id) des Kontos. Alle anderen – mit fremder Kennung oder weiter unten mit derselben
//!   Kennung (z. B. vom Absender eingeschleust) – werden ignoriert.
//! - Das wirkt nur, wenn der eigene Server prüft und eingehende Header mit seiner Kennung entfernt
//!   oder seinen eigenen darüber setzt (RFC 8601, Abschnitt 5).

use mail_parser::Message;
use serde::Serialize;

/// Name des Headers.
const HEADER: &str = "Authentication-Results";

/// Ergebnis der Absenderprüfung in drei Stufen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Stufe {
    /// Der eigene Server hat den Absender bestätigt (DMARC bestanden bzw. DKIM oder SPF
    /// bestanden und passend zur Absenderdomain).
    Bestaetigt,
    /// Kein Prüfergebnis des eigenen Servers oder keine Aussage möglich.
    #[default]
    NichtGeprueft,
    /// Der Absender ist wahrscheinlich gefälscht.
    Warnung,
}

/// Ergebnis einer Prüfmethode (DKIM, SPF oder DMARC).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Methode {
    /// Ergebnis in Kleinbuchstaben, z. B. `pass`, `fail`, `none`.
    pub ergebnis: String,
    /// Geprüfte Domain (`header.d`/`header.i` bei DKIM, `smtp.mailfrom` bei SPF,
    /// `header.from` bei DMARC), sofern angegeben.
    pub domain: Option<String>,
    /// Passt die Domain zur Absenderdomain (From)?
    pub passend: bool,
}

/// Ergebnis der Absenderprüfung für die Anzeige.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct Absenderpruefung {
    pub stufe: Stufe,
    /// Server-Kennung (authserv-id), der vertraut wird.
    pub kennung: String,
    /// Wurde ein Header mit dieser Kennung gefunden?
    pub gefunden: bool,
    /// Domain der Absenderadresse (From), kleingeschrieben.
    pub absender_domain: Option<String>,
    pub dkim: Vec<Methode>,
    pub spf: Option<Methode>,
    pub dmarc: Option<Methode>,
}

/// Eine Angabe aus dem Header: `methode=ergebnis` mit Eigenschaften wie `header.d=…`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Angabe {
    pub methode: String,
    pub ergebnis: String,
    /// Eigenschaften (`header.d`, `smtp.mailfrom`, `reason` …), Namen kleingeschrieben.
    pub eigenschaften: Vec<(String, String)>,
}

impl Angabe {
    fn eigenschaft(&self, name: &str) -> Option<&str> {
        self.eigenschaften
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, w)| w.as_str())
    }
}

/// Ein gelesener `Authentication-Results`-Header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthErgebnisse {
    /// Server-Kennung (authserv-id), kleingeschrieben.
    pub kennung: String,
    pub angaben: Vec<Angabe>,
}

/// Entfernt Kommentare in Klammern (auch verschachtelt) ausserhalb von Anführungszeichen und
/// vereinheitlicht Leerraum (auch Zeilenumbrüche gefalteter Header).
fn ohne_kommentare(wert: &str) -> String {
    let mut ergebnis = String::with_capacity(wert.len());
    let mut tiefe = 0usize;
    let mut in_anfuehrung = false;
    let mut zeichen = wert.chars();
    while let Some(z) = zeichen.next() {
        match z {
            '\\' if tiefe > 0 || in_anfuehrung => {
                let naechstes = zeichen.next();
                if tiefe == 0 {
                    ergebnis.push(z);
                    if let Some(n) = naechstes {
                        ergebnis.push(n);
                    }
                }
            }
            '"' if tiefe == 0 => {
                in_anfuehrung = !in_anfuehrung;
                ergebnis.push(z);
            }
            '(' if !in_anfuehrung => {
                tiefe += 1;
                if tiefe == 1 {
                    ergebnis.push(' ');
                }
            }
            ')' if !in_anfuehrung && tiefe > 0 => tiefe -= 1,
            _ if tiefe > 0 => {}
            z if z.is_whitespace() => ergebnis.push(' '),
            z => ergebnis.push(z),
        }
    }
    ergebnis
}

/// Teilt an `trenner`, aber nicht innerhalb von Anführungszeichen.
fn teilen(text: &str, trenner: impl Fn(char) -> bool) -> Vec<String> {
    let mut teile = Vec::new();
    let mut aktuell = String::new();
    let mut in_anfuehrung = false;
    let mut zeichen = text.chars();
    while let Some(z) = zeichen.next() {
        match z {
            '\\' if in_anfuehrung => {
                aktuell.push(z);
                if let Some(n) = zeichen.next() {
                    aktuell.push(n);
                }
            }
            '"' => {
                in_anfuehrung = !in_anfuehrung;
                aktuell.push(z);
            }
            z if !in_anfuehrung && trenner(z) => teile.push(std::mem::take(&mut aktuell)),
            z => aktuell.push(z),
        }
    }
    teile.push(aktuell);
    teile
}

fn ohne_anfuehrung(wert: &str) -> String {
    let wert = wert.trim();
    match wert.strip_prefix('"').and_then(|w| w.strip_suffix('"')) {
        Some(innen) => innen.replace("\\\"", "\"").replace("\\\\", "\\"),
        None => wert.to_string(),
    }
}

/// Liest den Wert eines `Authentication-Results`-Headers (ohne den Namen).
pub fn header_lesen(wert: &str) -> Option<AuthErgebnisse> {
    let bereinigt = ohne_kommentare(wert);
    let mut abschnitte = teilen(&bereinigt, |z| z == ';').into_iter();
    // authserv-id, optional gefolgt von einer Versionsnummer
    let kennung = abschnitte
        .next()?
        .split_whitespace()
        .next()
        .map(ohne_anfuehrung)?
        .to_ascii_lowercase();
    if kennung.is_empty() {
        return None;
    }
    let mut angaben = Vec::new();
    for abschnitt in abschnitte {
        // Leerraum um „=“ entfernen („dkim = pass“ → „dkim=pass“).
        let mut kompakt = String::new();
        for (i, teil) in teilen(&abschnitt, |z| z == '=').iter().enumerate() {
            if i > 0 {
                kompakt.push('=');
            }
            kompakt.push_str(if i == 0 { teil.trim_end() } else { teil.trim() });
        }
        let mut woerter = teilen(&kompakt, char::is_whitespace)
            .into_iter()
            .filter(|w| !w.is_empty());
        let Some(erstes) = woerter.next() else {
            continue;
        };
        let Some((methode, ergebnis)) = erstes.split_once('=') else {
            // „none“ ohne Methode: keine Prüfung durchgeführt.
            continue;
        };
        let methode = methode
            .split('/')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        let ergebnis = ohne_anfuehrung(ergebnis).to_ascii_lowercase();
        if methode.is_empty() || ergebnis.is_empty() {
            continue;
        }
        let eigenschaften = woerter
            .filter_map(|wort| {
                let (name, wert) = wort.split_once('=')?;
                Some((name.trim().to_ascii_lowercase(), ohne_anfuehrung(wert)))
            })
            .collect();
        angaben.push(Angabe {
            methode,
            ergebnis,
            eigenschaften,
        });
    }
    Some(AuthErgebnisse { kennung, angaben })
}

/// Sucht den obersten `Authentication-Results`-Header mit der Kennung `kennung`.
///
/// `header` sind die Werte aller `Authentication-Results`-Header in der Reihenfolge der Mail
/// (oben zuerst).
pub fn vertrauenswuerdig<'a>(
    header: impl IntoIterator<Item = &'a str>,
    kennung: &str,
) -> Option<AuthErgebnisse> {
    let kennung = kennung.trim().to_ascii_lowercase();
    if kennung.is_empty() {
        return None;
    }
    header
        .into_iter()
        .filter_map(header_lesen)
        .find(|e| e.kennung == kennung)
}

/// Domain aus `user@domain`, `@domain` oder `domain`, kleingeschrieben, ohne Punkt am Ende.
fn domain_von(wert: &str) -> Option<String> {
    let wert = wert.trim().trim_matches(|z| z == '<' || z == '>');
    let domain = wert.rsplit_once('@').map_or(wert, |(_, d)| d);
    let domain = domain.trim().trim_end_matches('.').to_ascii_lowercase();
    (!domain.is_empty() && domain.contains('.')).then_some(domain)
}

/// Gehören zwei Domains zusammen (gleich oder eine ist Subdomain der anderen)?
///
/// Vereinfachte „relaxed alignment“ nach DMARC (ohne Public Suffix List): `mail.example.org`
/// passt zu `example.org`. Die kürzere Domain muss mindestens zwei Teile haben, damit z. B.
/// `org` nicht zu allem passt.
fn passt(a: &str, b: &str) -> bool {
    let (kurz, lang) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    kurz.contains('.') && (kurz == lang || lang.ends_with(&format!(".{kurz}")))
}

/// Bewertet die Angaben des vertrauenswürdigen Headers für die Absenderdomain.
///
/// - **bestätigt**: `dmarc=pass` (zur Absenderdomain), oder – wenn der Server kein bzw. kein
///   verwertbares DMARC-Ergebnis meldet – DKIM oder SPF bestanden und passend zur Absenderdomain.
/// - **Warnung**: `dmarc=fail`, oder DKIM und SPF beide fehlgeschlagen bzw. nicht zur
///   Absenderdomain passend (mindestens eines ausdrücklich `fail`/`softfail`), sofern der Server
///   kein DMARC-Ergebnis `none` meldet.
/// - sonst **nicht geprüft** (auch: kein passender Header, keine Absenderdomain, `dmarc=none`
///   ohne passenden DKIM-/SPF-Erfolg).
pub fn bewerten(
    ergebnisse: Option<&AuthErgebnisse>,
    absender_domain: Option<&str>,
    kennung: &str,
) -> Absenderpruefung {
    let mut pruefung = Absenderpruefung {
        stufe: Stufe::NichtGeprueft,
        kennung: kennung.trim().to_ascii_lowercase(),
        gefunden: ergebnisse.is_some(),
        absender_domain: absender_domain.and_then(domain_von),
        ..Default::default()
    };
    let Some(ergebnisse) = ergebnisse else {
        return pruefung;
    };
    let von = pruefung.absender_domain.clone();
    let methode = |angabe: &Angabe, felder: &[&str]| {
        let domain = felder
            .iter()
            .find_map(|f| angabe.eigenschaft(f))
            .and_then(domain_von);
        let passend = match (&domain, &von) {
            (Some(d), Some(v)) => passt(d, v),
            _ => false,
        };
        Methode {
            ergebnis: angabe.ergebnis.clone(),
            domain,
            passend,
        }
    };
    for angabe in &ergebnisse.angaben {
        match angabe.methode.as_str() {
            "dkim" => pruefung
                .dkim
                .push(methode(angabe, &["header.d", "header.i"])),
            "spf" if pruefung.spf.is_none() => {
                pruefung.spf = Some(methode(angabe, &["smtp.mailfrom", "smtp.helo"]))
            }
            "dmarc" if pruefung.dmarc.is_none() => {
                pruefung.dmarc = Some(methode(angabe, &["header.from"]))
            }
            _ => {}
        }
    }
    let Some(von) = von else {
        return pruefung;
    };
    pruefung.stufe = stufe(&pruefung, &von);
    pruefung
}

fn stufe(pruefung: &Absenderpruefung, von: &str) -> Stufe {
    // DMARC-Ergebnis zählt nur für die eigene Absenderdomain (fehlt header.from, gilt es ihr).
    let dmarc = pruefung
        .dmarc
        .as_ref()
        .filter(|d| d.domain.as_deref().is_none_or(|d| passt(d, von)));
    match dmarc.map(|d| d.ergebnis.as_str()) {
        Some("pass") => return Stufe::Bestaetigt,
        Some("fail") => return Stufe::Warnung,
        _ => {}
    }
    let erfolg = |m: &Methode| m.ergebnis == "pass" && m.passend;
    let dkim_erfolg = pruefung.dkim.iter().any(erfolg);
    let spf_erfolg = pruefung.spf.as_ref().is_some_and(erfolg);
    if dkim_erfolg || spf_erfolg {
        return Stufe::Bestaetigt;
    }
    if dmarc.is_some_and(|d| d.ergebnis == "none") {
        return Stufe::NichtGeprueft;
    }
    // Fehlgeschlagen oder bestanden, aber für eine fremde Domain.
    let negativ = |m: &Methode, fehler: &[&str]| {
        fehler.contains(&m.ergebnis.as_str()) || (m.ergebnis == "pass" && !m.passend)
    };
    let ausdruecklich = |m: &Methode, fehler: &[&str]| fehler.contains(&m.ergebnis.as_str());
    let dkim_negativ =
        !pruefung.dkim.is_empty() && pruefung.dkim.iter().all(|m| negativ(m, &["fail"]));
    let spf_negativ = pruefung
        .spf
        .as_ref()
        .is_some_and(|m| negativ(m, &["fail", "softfail"]));
    let fehlgeschlagen = pruefung.dkim.iter().any(|m| ausdruecklich(m, &["fail"]))
        || pruefung
            .spf
            .as_ref()
            .is_some_and(|m| ausdruecklich(m, &["fail", "softfail"]));
    if dkim_negativ && spf_negativ && fehlgeschlagen {
        Stufe::Warnung
    } else {
        Stufe::NichtGeprueft
    }
}

/// Prüft eine Mail: oberster `Authentication-Results`-Header mit `kennung`, bewertet für die
/// Domain der (ersten) Absenderadresse.
pub fn pruefen(roh: &[u8], nachricht: &Message, kennung: &str) -> Absenderpruefung {
    let werte: Vec<String> = nachricht
        .headers()
        .iter()
        .filter(|h| h.name.as_str().eq_ignore_ascii_case(HEADER))
        .filter_map(|h| roh.get(h.offset_start as usize..h.offset_end as usize))
        .map(|w| String::from_utf8_lossy(w).into_owned())
        .collect();
    let ergebnisse = vertrauenswuerdig(werte.iter().map(String::as_str), kennung);
    // Mehrere Absender: keine eindeutige Domain, also keine Bestätigung.
    let von = nachricht
        .from()
        .filter(|a| a.iter().count() == 1)
        .and_then(|a| a.first())
        .and_then(|a| a.address())
        .and_then(|a| a.rsplit_once('@').map(|(_, d)| d.to_string()));
    bewerten(ergebnisse.as_ref(), von.as_deref(), kennung)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mail(kopf: &str, von: &str) -> String {
        format!(
            "{kopf}From: Absender <{von}>\r\nTo: anna@example.ch\r\nSubject: Test\r\n\r\nHallo\r\n"
        )
    }

    fn stufe_fuer(kopf: &str, von: &str, kennung: &str) -> Absenderpruefung {
        let roh = mail(kopf, von);
        let nachricht = crate::anzeige::parsen(roh.as_bytes()).unwrap();
        pruefen(roh.as_bytes(), &nachricht, kennung)
    }

    /// Mailcow/rspamd: ein Header, gefaltet mit Tabs, Kommentare mit Anführungszeichen.
    const RSPAMD: &str = "Authentication-Results: mail.example.ch;\r\n\
\tdkim=pass header.d=example.org header.s=dkim header.b=AbC123+/;\r\n\
\tdmarc=pass (policy=reject) header.from=example.org;\r\n\
\tspf=pass (mail.example.ch: domain of \"bounce@example.org\" designates 192.0.2.1 as permitted sender) smtp.mailfrom=\"bounce@example.org\"\r\n";

    /// Postfix mit OpenDKIM, OpenDMARC und policyd-spf: mehrere Header derselben Kennung.
    const POSTFIX: &str = "Authentication-Results: mx.example.ch; dmarc=pass (p=none dis=none) header.from=example.org\r\n\
Authentication-Results: mx.example.ch;\r\n\
\tdkim=pass (2048-bit key; unprotected) header.d=example.org header.i=@example.org header.b=\"AbCd1234\";\r\n\
\tdkim-atps=neutral\r\n\
Received: from mail.example.org (mail.example.org [192.0.2.1]) by mx.example.ch (Postfix) with ESMTPS id 4Ab; Mon, 19 Oct 2026 10:00:00 +0200 (CEST)\r\n";

    /// Gmail: Kommentar mit Doppelpunkt, header.i statt header.d.
    const GMAIL: &str = "Authentication-Results: mx.google.com;\r\n       dkim=pass header.i=@example.org header.s=s1 header.b=\"x/Y+z\";\r\n       spf=pass (google.com: domain of anna@example.org designates 192.0.2.1 as permitted sender) smtp.mailfrom=anna@example.org;\r\n       dmarc=pass (p=REJECT sp=REJECT dis=NONE) header.from=example.org\r\n";

    #[test]
    fn rspamd_wird_bestaetigt() {
        let p = stufe_fuer(RSPAMD, "news@example.org", "mail.example.ch");
        assert_eq!(p.stufe, Stufe::Bestaetigt);
        assert!(p.gefunden);
        assert_eq!(p.kennung, "mail.example.ch");
        assert_eq!(p.absender_domain.as_deref(), Some("example.org"));
        assert_eq!(
            p.dkim,
            vec![Methode {
                ergebnis: "pass".into(),
                domain: Some("example.org".into()),
                passend: true
            }]
        );
        assert_eq!(p.spf.unwrap().domain.as_deref(), Some("example.org"));
        assert_eq!(p.dmarc.unwrap().ergebnis, "pass");
    }

    #[test]
    fn postfix_nur_oberster_header() {
        let p = stufe_fuer(POSTFIX, "info@example.org", "MX.Example.CH");
        assert_eq!(p.stufe, Stufe::Bestaetigt);
        assert_eq!(p.dmarc.as_ref().unwrap().ergebnis, "pass");
        // Der DKIM-Header darunter hat dieselbe Kennung, zählt aber nicht.
        assert!(p.dkim.is_empty());
    }

    #[test]
    fn gmail_wird_bestaetigt() {
        let p = stufe_fuer(GMAIL, "anna@example.org", "mx.google.com");
        assert_eq!(p.stufe, Stufe::Bestaetigt);
        assert_eq!(p.dkim[0].domain.as_deref(), Some("example.org"));
        assert_eq!(p.spf.unwrap().domain.as_deref(), Some("example.org"));
    }

    #[test]
    fn dmarc_fail_warnt() {
        let kopf = "Authentication-Results: mail.example.ch; dkim=none; spf=fail smtp.mailfrom=bank.example; dmarc=fail (p=reject) header.from=bank.example\r\n";
        let p = stufe_fuer(kopf, "konto@bank.example", "mail.example.ch");
        assert_eq!(p.stufe, Stufe::Warnung);
    }

    #[test]
    fn fremde_kennung_wird_ignoriert() {
        // Vom Absender eingeschleust: „alles bestanden“ mit fremder Kennung.
        let kopf = "Authentication-Results: mail.example.ch; dmarc=fail header.from=bank.example\r\n\
Authentication-Results: mx.angreifer.example; dkim=pass header.d=bank.example; dmarc=pass header.from=bank.example\r\n";
        let p = stufe_fuer(kopf, "konto@bank.example", "mail.example.ch");
        assert_eq!(p.stufe, Stufe::Warnung);

        // Nur der fremde Header vorhanden → nicht geprüft.
        let kopf =
            "Authentication-Results: mx.angreifer.example; dmarc=pass header.from=bank.example\r\n";
        let p = stufe_fuer(kopf, "konto@bank.example", "mail.example.ch");
        assert_eq!(p.stufe, Stufe::NichtGeprueft);
        assert!(!p.gefunden);
    }

    #[test]
    fn eigene_kennung_weiter_unten_wird_ignoriert() {
        let kopf = "Authentication-Results: mail.example.ch; dkim=fail header.d=bank.example; spf=fail smtp.mailfrom=bank.example\r\n\
Received: from boese.example by mail.example.ch; Mon, 19 Oct 2026 10:00:00 +0200\r\n\
Authentication-Results: mail.example.ch; dmarc=pass header.from=bank.example\r\n";
        let p = stufe_fuer(kopf, "konto@bank.example", "mail.example.ch");
        assert_eq!(p.stufe, Stufe::Warnung);
        assert!(p.dmarc.is_none());
    }

    #[test]
    fn fehlender_header_ist_nicht_geprueft() {
        let p = stufe_fuer("", "anna@example.org", "mail.example.ch");
        assert_eq!(p.stufe, Stufe::NichtGeprueft);
        assert!(!p.gefunden);
        // Leere Kennung vertraut nie.
        let p = stufe_fuer(RSPAMD, "news@example.org", "  ");
        assert_eq!(p.stufe, Stufe::NichtGeprueft);
    }

    #[test]
    fn gross_kleinschreibung_und_leerraum() {
        let kopf = "AUTHENTICATION-RESULTS:   Mail.Example.CH 1 ;  DKIM = PASS header.D=Example.ORG ;  DMARC=None header.from=EXAMPLE.org\r\n";
        let p = stufe_fuer(kopf, "anna@Example.Org", "mail.example.ch");
        assert_eq!(p.kennung, "mail.example.ch");
        assert_eq!(p.dkim[0].ergebnis, "pass");
        assert_eq!(p.dkim[0].domain.as_deref(), Some("example.org"));
        // dmarc=none, aber DKIM passend bestanden
        assert_eq!(p.stufe, Stufe::Bestaetigt);
    }

    #[test]
    fn kommentare_in_klammern() {
        let werte = header_lesen(
            "mail.example.ch (rspamd (Version 3.8); \"x\"); dkim=pass (gute Signatur; header.d=boese.example) header.d=example.org; spf=pass (Kommentar \\) mit Klammer) smtp.mailfrom=example.org",
        )
        .unwrap();
        assert_eq!(werte.kennung, "mail.example.ch");
        assert_eq!(werte.angaben.len(), 2);
        assert_eq!(
            werte.angaben[0].eigenschaft("header.d"),
            Some("example.org")
        );
        assert_eq!(werte.angaben[1].methode, "spf");
        assert_eq!(
            werte.angaben[1].eigenschaft("smtp.mailfrom"),
            Some("example.org")
        );
    }

    #[test]
    fn none_und_leere_header() {
        assert_eq!(
            header_lesen("mail.example.ch; none"),
            Some(AuthErgebnisse {
                kennung: "mail.example.ch".into(),
                angaben: Vec::new()
            })
        );
        assert_eq!(header_lesen(""), None);
        assert_eq!(header_lesen(" ; dkim=pass"), None);
    }

    #[test]
    fn dmarc_none_ohne_passenden_erfolg_ist_nicht_geprueft() {
        // Newsletter über einen Versanddienst: DKIM und SPF bestanden, aber für dessen Domain.
        let kopf = "Authentication-Results: mail.example.ch; dkim=pass header.d=versand.example; spf=pass smtp.mailfrom=bounce.versand.example; dmarc=none header.from=laden.example\r\n";
        let p = stufe_fuer(kopf, "news@laden.example", "mail.example.ch");
        assert_eq!(p.stufe, Stufe::NichtGeprueft);
        assert!(!p.dkim[0].passend);
    }

    #[test]
    fn ohne_dmarc_dkim_und_spf_fehlgeschlagen_warnt() {
        let kopf = "Authentication-Results: mail.example.ch; dkim=fail header.d=bank.example; spf=softfail smtp.mailfrom=bank.example\r\n";
        assert_eq!(
            stufe_fuer(kopf, "konto@bank.example", "mail.example.ch").stufe,
            Stufe::Warnung
        );
        // SPF bestanden, aber für eine fremde Domain; DKIM fehlgeschlagen.
        let kopf = "Authentication-Results: mail.example.ch; dkim=fail header.d=bank.example; spf=pass smtp.mailfrom=boese.example\r\n";
        assert_eq!(
            stufe_fuer(kopf, "konto@bank.example", "mail.example.ch").stufe,
            Stufe::Warnung
        );
        // Nur SPF geprüft und fehlgeschlagen, DKIM fehlt: keine eindeutige Aussage.
        let kopf =
            "Authentication-Results: mail.example.ch; spf=fail smtp.mailfrom=bank.example\r\n";
        assert_eq!(
            stufe_fuer(kopf, "konto@bank.example", "mail.example.ch").stufe,
            Stufe::NichtGeprueft
        );
        // Ohne DMARC, aber DKIM passend bestanden.
        let kopf = "Authentication-Results: mail.example.ch; dkim=pass header.d=mail.bank.example; spf=fail smtp.mailfrom=bank.example\r\n";
        assert_eq!(
            stufe_fuer(kopf, "konto@bank.example", "mail.example.ch").stufe,
            Stufe::Bestaetigt
        );
    }

    #[test]
    fn dmarc_fuer_andere_domain_zaehlt_nicht() {
        let kopf =
            "Authentication-Results: mail.example.ch; dmarc=pass header.from=boese.example\r\n";
        assert_eq!(
            stufe_fuer(kopf, "konto@bank.example", "mail.example.ch").stufe,
            Stufe::NichtGeprueft
        );
    }

    #[test]
    fn domains_passen() {
        assert!(passt("example.org", "example.org"));
        assert!(passt("mail.example.org", "example.org"));
        assert!(passt("example.org", "news.example.org"));
        assert!(!passt("example.org", "badexample.org"));
        assert!(!passt("org", "example.org"));
        assert_eq!(
            domain_von("<Anna@Example.ORG.>"),
            Some("example.org".into())
        );
        assert_eq!(domain_von("@example.org"), Some("example.org".into()));
        assert_eq!(domain_von("localhost"), None);
    }

    #[test]
    fn mehrere_absender_werden_nicht_bestaetigt() {
        let roh =
            format!("{RSPAMD}From: a@example.org, b@boese.example\r\nSubject: x\r\n\r\nx\r\n");
        let nachricht = crate::anzeige::parsen(roh.as_bytes()).unwrap();
        let p = pruefen(roh.as_bytes(), &nachricht, "mail.example.ch");
        assert_eq!(p.stufe, Stufe::NichtGeprueft);
    }
}
