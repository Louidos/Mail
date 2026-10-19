//! Kleiner DER-Leser für die wenigen Felder, die OpenSSL nicht herausgibt: Verfahren
//! (Inhaltsverschlüsselung, Hashfunktionen), Empfänger und Signaturzeitpunkt.
//!
//! Gelesen wird nur DER mit festen Längen. BER von anderen Programmen (z. B. unbestimmte Längen)
//! wandelt vorher OpenSSL in DER um (`CmsContentInfo::from_der` + `to_der`).

/// Ein DER-Element: Tag (erstes Byte) und Inhalt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Element<'a> {
    pub tag: u8,
    pub inhalt: &'a [u8],
}

pub const SEQUENCE: u8 = 0x30;
pub const SET: u8 = 0x31;
pub const OID: u8 = 0x06;
pub const INTEGER: u8 = 0x02;
pub const OCTET_STRING: u8 = 0x04;
pub const UTC_TIME: u8 = 0x17;
pub const GENERALIZED_TIME: u8 = 0x18;

/// `[n]` kontextspezifisch, zusammengesetzt.
pub const fn kontext(n: u8) -> u8 {
    0xa0 | n
}

/// `[n]` kontextspezifisch, einfach (z. B. `subjectKeyIdentifier` in `RecipientIdentifier`).
pub const fn kontext_einfach(n: u8) -> u8 {
    0x80 | n
}

/// Liest ein Element vom Anfang und gibt es mit dem Rest zurück.
pub fn lesen(daten: &[u8]) -> Option<(Element<'_>, &[u8])> {
    let tag = *daten.first()?;
    // Mehrbyte-Tags kommen in CMS an diesen Stellen nicht vor.
    if tag & 0x1f == 0x1f {
        return None;
    }
    let erstes = *daten.get(1)? as usize;
    let (laenge, kopf) = if erstes < 0x80 {
        (erstes, 2)
    } else {
        let anzahl = erstes & 0x7f;
        // 0x80 wäre eine unbestimmte Länge (nur BER); mehr als 4 Bytes Länge gibt es hier nicht.
        if anzahl == 0 || anzahl > 4 {
            return None;
        }
        let mut laenge = 0usize;
        for i in 0..anzahl {
            laenge = (laenge << 8) | *daten.get(2 + i)? as usize;
        }
        (laenge, 2 + anzahl)
    };
    let ende = kopf.checked_add(laenge)?;
    let inhalt = daten.get(kopf..ende)?;
    Some((Element { tag, inhalt }, &daten[ende..]))
}

/// Alle Elemente in einem Inhalt (z. B. einer SEQUENCE). `None`, wenn etwas nicht passt.
pub fn kinder(inhalt: &[u8]) -> Option<Vec<Element<'_>>> {
    let mut rest = inhalt;
    let mut alle = Vec::new();
    while !rest.is_empty() {
        let (element, weiter) = lesen(rest)?;
        alle.push(element);
        rest = weiter;
    }
    Some(alle)
}

/// Erwartet ein Element mit diesem Tag und genau diesen Daten.
pub fn ganz(daten: &[u8], tag: u8) -> Option<Element<'_>> {
    let (element, rest) = lesen(daten)?;
    (element.tag == tag && rest.is_empty()).then_some(element)
}

/// Objektkennung als Punktschreibweise, z. B. `2.16.840.1.101.3.4.1.42`.
pub fn oid_text(inhalt: &[u8]) -> Option<String> {
    let (&erstes, rest) = inhalt.split_first()?;
    let mut teile = vec![(erstes / 40).min(2) as u64];
    teile.push(erstes as u64 - teile[0] * 40);
    let mut wert = 0u64;
    for &b in rest {
        wert = wert.checked_mul(128)? | (b & 0x7f) as u64;
        if b & 0x80 == 0 {
            teile.push(wert);
            wert = 0;
        }
    }
    if rest.last().is_some_and(|b| b & 0x80 != 0) {
        return None;
    }
    Some(
        teile
            .iter()
            .map(|t| t.to_string())
            .collect::<Vec<_>>()
            .join("."),
    )
}

/// OID eines `AlgorithmIdentifier` (SEQUENCE { OID, Parameter }).
pub fn algorithmus(element: &Element) -> Option<String> {
    if element.tag != SEQUENCE {
        return None;
    }
    let (oid, _) = lesen(element.inhalt)?;
    (oid.tag == OID).then(|| oid_text(oid.inhalt)).flatten()
}

/// UTCTime bzw. GeneralizedTime (nur die in CMS vorgeschriebene Form `…Z`) als Unix-Zeit.
pub fn zeit(element: &Element) -> Option<i64> {
    let text = std::str::from_utf8(element.inhalt).ok()?;
    let text = text.strip_suffix('Z')?;
    let (jahr, rest) = match element.tag {
        UTC_TIME if text.len() == 12 => {
            let j: i64 = text[..2].parse().ok()?;
            (if j >= 50 { 1900 + j } else { 2000 + j }, &text[2..])
        }
        GENERALIZED_TIME if text.len() == 14 => (text[..4].parse().ok()?, &text[4..]),
        _ => return None,
    };
    if !rest.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let zahl = |i: usize| rest[i..i + 2].parse::<i64>().ok();
    let (monat, tag, stunde, minute, sekunde) = (zahl(0)?, zahl(2)?, zahl(4)?, zahl(6)?, zahl(8)?);
    if !(1..=12).contains(&monat)
        || !(1..=31).contains(&tag)
        || stunde > 23
        || minute > 59
        || sekunde > 60
    {
        return None;
    }
    Some(tage_seit_1970(jahr, monat, tag) * 86_400 + stunde * 3600 + minute * 60 + sekunde)
}

/// Tage seit dem 1.1.1970 (gregorianischer Kalender).
fn tage_seit_1970(jahr: i64, monat: i64, tag: i64) -> i64 {
    let j = if monat <= 2 { jahr - 1 } else { jahr };
    let ara = j.div_euclid(400);
    let jahr_der_ara = j - ara * 400;
    let m = (monat + 9) % 12;
    let tag_im_jahr = (153 * m + 2) / 5 + tag - 1;
    let tag_der_ara = jahr_der_ara * 365 + jahr_der_ara / 4 - jahr_der_ara / 100 + tag_im_jahr;
    ara * 146_097 + tag_der_ara - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn liest_oid_und_zeit() {
        // 2.16.840.1.101.3.4.1.42 (AES-256-CBC)
        let oid = [0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x01, 0x2a];
        assert_eq!(oid_text(&oid).unwrap(), "2.16.840.1.101.3.4.1.42");
        let utc = Element {
            tag: UTC_TIME,
            inhalt: b"261019080000Z",
        };
        assert_eq!(zeit(&utc), Some(1_792_396_800));
        let gen = Element {
            tag: GENERALIZED_TIME,
            inhalt: b"19700101000001Z",
        };
        assert_eq!(zeit(&gen), Some(1));
        assert_eq!(
            zeit(&Element {
                tag: UTC_TIME,
                inhalt: b"2610190800Z"
            }),
            None
        );
    }

    #[test]
    fn lehnt_kaputtes_und_unbestimmte_laengen_ab() {
        assert!(lesen(&[0x30, 0x80, 0x00, 0x00]).is_none());
        assert!(lesen(&[0x30, 0x05, 0x01]).is_none());
        assert!(lesen(&[]).is_none());
        let (element, rest) = lesen(&[0x04, 0x01, 0xff, 0x05, 0x00]).unwrap();
        assert_eq!(element.inhalt, &[0xff]);
        assert_eq!(rest, &[0x05, 0x00]);
        assert_eq!(kinder(&[0x05, 0x00, 0x05, 0x00]).unwrap().len(), 2);
    }
}
