//! Integrationstests: Konto einrichten beim lokalen Test-Mailserver (GreenMail).

mod gemeinsam;

use std::sync::Arc;

use gemeinsam::*;
use sichermail_lib::konto::schluesselbund::{
    konto_schluessel, Schluesselbund, SpeicherSchluesselbund,
};
use sichermail_lib::konto::{KontoVerwaltung, Verschluesselung};

fn verwaltung() -> (
    tempfile::TempDir,
    Arc<SpeicherSchluesselbund>,
    KontoVerwaltung,
) {
    let ordner = tempfile::tempdir().unwrap();
    let bund = Arc::new(SpeicherSchluesselbund::default());
    let verwaltung = KontoVerwaltung::neu(ordner.path(), bund.clone());
    (ordner, bund, verwaltung)
}

/// Durchsucht alle Dateien im Ordner nach einem Text.
fn kommt_in_dateien_vor(ordner: &std::path::Path, text: &str) -> bool {
    let mut offen = vec![ordner.to_path_buf()];
    while let Some(pfad) = offen.pop() {
        for eintrag in std::fs::read_dir(&pfad).unwrap() {
            let eintrag = eintrag.unwrap().path();
            if eintrag.is_dir() {
                offen.push(eintrag);
            } else {
                let inhalt = std::fs::read(&eintrag).unwrap();
                if inhalt.windows(text.len()).any(|f| f == text.as_bytes()) {
                    return true;
                }
            }
        }
    }
    false
}

#[tokio::test]
#[ignore = "braucht den Test-Mailserver (scripts/test-mailserver.sh start)"]
async fn konto_anlegen_testen_bearbeiten_loeschen() {
    let (ordner, bund, verwaltung) = verwaltung();

    // Testen ohne Speichern
    let ergebnis = verwaltung
        .testen(&eingabe(BENUTZER, PASSWORT))
        .await
        .unwrap();
    assert!(ergebnis.erfolgreich(), "{ergebnis:?}");
    assert!(verwaltung.alle().await.unwrap().is_empty());

    // Anlegen
    let konto = verwaltung
        .speichern(&eingabe(BENUTZER, PASSWORT))
        .await
        .unwrap();
    assert_eq!(verwaltung.alle().await.unwrap(), vec![konto.clone()]);
    assert_eq!(
        bund.lesen(&konto_schluessel(&konto.id))
            .unwrap()
            .unwrap()
            .as_str(),
        PASSWORT
    );

    // Bearbeiten ohne neues Passwort: das gespeicherte wird verwendet
    let mut aenderung = eingabe(BENUTZER, PASSWORT);
    aenderung.id = Some(konto.id.clone());
    aenderung.name = "Neuer Name".into();
    aenderung.passwort = None;
    let geaendert = verwaltung.speichern(&aenderung).await.unwrap();
    assert_eq!(geaendert.name, "Neuer Name");
    assert_eq!(verwaltung.alle().await.unwrap().len(), 1);

    // Passwort steht in keiner Datei im App-Datenordner
    assert!(!kommt_in_dateien_vor(ordner.path(), PASSWORT));

    // Löschen entfernt auch den Schlüsselbund-Eintrag
    verwaltung.loeschen(&konto.id).await.unwrap();
    assert!(verwaltung.alle().await.unwrap().is_empty());
    assert_eq!(bund.anzahl(), 0);
}

#[tokio::test]
#[ignore = "braucht den Test-Mailserver (scripts/test-mailserver.sh start)"]
async fn falsches_passwort_wird_gemeldet_und_nicht_gespeichert() {
    let (ordner, bund, verwaltung) = verwaltung();
    let fehler = verwaltung
        .speichern(&eingabe(BENUTZER, "falsches-passwort"))
        .await
        .unwrap_err();
    assert_eq!(fehler.art, "anmeldung_falsch", "{fehler:?}");
    assert!(fehler.meldung.contains("Anmeldung fehlgeschlagen"));
    assert!(!fehler.meldung.contains("falsches-passwort"));
    assert!(verwaltung.alle().await.unwrap().is_empty());
    assert_eq!(bund.anzahl(), 0);
    assert!(!kommt_in_dateien_vor(ordner.path(), "falsches-passwort"));
}

#[tokio::test]
#[ignore = "braucht den Test-Mailserver (scripts/test-mailserver.sh start)"]
async fn ungueltiges_zertifikat_wird_gemeldet_und_nicht_gespeichert() {
    let (_ordner, bund, verwaltung) = verwaltung();
    // Ohne hinterlegtes Zertifikat ist das selbstsignierte Zertifikat ungültig.
    let mut e = eingabe(BENUTZER, PASSWORT);
    e.imap = server(IMAP_TLS, Verschluesselung::Tls, false);
    e.smtp = server(SMTP_TLS, Verschluesselung::Tls, false);
    let fehler = verwaltung.speichern(&e).await.unwrap_err();
    assert_eq!(fehler.art, "zertifikat_ungueltig", "{fehler:?}");
    assert!(fehler.meldung.contains("Zertifikat"));
    assert!(verwaltung.alle().await.unwrap().is_empty());
    assert_eq!(bund.anzahl(), 0);
}

#[tokio::test]
#[ignore = "braucht den Test-Mailserver (scripts/test-mailserver.sh start)"]
async fn fremdes_hinterlegtes_zertifikat_wird_abgelehnt() {
    let (_ordner, _bund, verwaltung) = verwaltung();
    // Ein anderes, gültig aufgebautes Zertifikat für denselben Namen darf nicht passen.
    let fremd = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])
        .unwrap()
        .cert
        .pem();
    let mut e = eingabe(BENUTZER, PASSWORT);
    e.imap.zertifikat = Some(fremd.clone());
    e.smtp.zertifikat = Some(fremd);
    let ergebnis = verwaltung.testen(&e).await.unwrap();
    assert_eq!(ergebnis.imap.as_ref().unwrap().art, "zertifikat_ungueltig");
    assert_eq!(ergebnis.smtp.as_ref().unwrap().art, "zertifikat_ungueltig");
    assert!(ergebnis
        .imap
        .unwrap()
        .meldung
        .contains("stimmt nicht mit dem hinterlegten Zertifikat"));
}

#[tokio::test]
#[ignore = "braucht den Test-Mailserver (scripts/test-mailserver.sh start)"]
async fn server_ohne_starttls_wird_abgelehnt() {
    let (_ordner, _bund, verwaltung) = verwaltung();
    let mut e = eingabe(BENUTZER, PASSWORT);
    e.imap = server(IMAP_KLARTEXT, Verschluesselung::Starttls, true);
    e.smtp = server(SMTP_KLARTEXT, Verschluesselung::Starttls, true);
    let ergebnis = verwaltung.testen(&e).await.unwrap();
    assert_eq!(ergebnis.imap.as_ref().unwrap().art, "kein_starttls");
    assert_eq!(ergebnis.smtp.as_ref().unwrap().art, "kein_starttls");
    assert!(verwaltung.speichern(&e).await.is_err());
    assert!(verwaltung.alle().await.unwrap().is_empty());
}

#[tokio::test]
#[ignore = "braucht den Test-Mailserver (scripts/test-mailserver.sh start)"]
async fn tls_auf_klartext_port_gibt_verstaendlichen_fehler() {
    let (_ordner, _bund, verwaltung) = verwaltung();
    let mut e = eingabe(BENUTZER, PASSWORT);
    e.imap = server(IMAP_KLARTEXT, Verschluesselung::Tls, true);
    let ergebnis = verwaltung.testen(&e).await.unwrap();
    assert_eq!(ergebnis.imap.as_ref().unwrap().art, "protokoll");
}

#[tokio::test]
#[ignore = "braucht den Test-Mailserver (scripts/test-mailserver.sh start)"]
async fn nicht_erreichbarer_server_wird_gemeldet() {
    let (_ordner, _bund, verwaltung) = verwaltung();
    let mut e = eingabe(BENUTZER, PASSWORT);
    e.imap.port = 1; // dort lauscht nichts
    let ergebnis = verwaltung.testen(&e).await.unwrap();
    assert_eq!(ergebnis.imap.as_ref().unwrap().art, "nicht_erreichbar");
}
