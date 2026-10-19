//! Gemeinsame TLS-Regeln für alle Verbindungen zu Mailservern.
//!
//! - Nur rustls, mindestens TLS 1.2.
//! - Zertifikate werden immer geprüft (Kette gegen die Mozilla-Wurzelzertifikate und Hostname).
//! - Einzige Ausnahme: Der Nutzer hinterlegt bewusst ein eigenes Zertifikat (Pinning).
//!   Dann wird ausschliesslich genau dieses Zertifikat akzeptiert.
//!
//! Es gibt bewusst keine Möglichkeit, die Prüfung abzuschalten.

use std::fmt;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, CryptoProvider};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{
    CertificateError, ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;
use tokio_rustls::TlsConnector;

use crate::fehler::VerbindungsFehler;

/// Zeitlimit für Verbindungsaufbau und TLS-Handshake.
pub const VERBINDUNGS_ZEITLIMIT: Duration = Duration::from_secs(20);

/// Zeitlimit für einzelne Antworten im unverschlüsselten Teil von STARTTLS.
const ANTWORT_ZEITLIMIT: Duration = Duration::from_secs(20);

/// Maximale Länge einer Zeile im unverschlüsselten Teil von STARTTLS.
const MAX_ZEILENLAENGE: usize = 4096;

fn krypto() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

const PEM_ANFANG: &str = "-----BEGIN CERTIFICATE-----";
const PEM_ENDE: &str = "-----END CERTIFICATE-----";

/// Liest ein vom Nutzer hinterlegtes Zertifikat (PEM) ein.
///
/// Leerraum und fehlende Zeilenumbrüche (z. B. nach Kopieren aus einer Webseite)
/// werden toleriert. Es muss genau ein Zertifikat sein.
pub fn zertifikat_lesen(pem: &str) -> Result<CertificateDer<'static>, VerbindungsFehler> {
    let ungueltig = || {
        VerbindungsFehler::Eingabe(
            "Das hinterlegte Zertifikat ist kein gültiges PEM-Zertifikat.".into(),
        )
    };
    if pem.matches(PEM_ANFANG).count() > 1 {
        return Err(VerbindungsFehler::Eingabe(
            "Bitte genau ein Zertifikat hinterlegen (das Zertifikat des Servers).".into(),
        ));
    }
    let start = pem.find(PEM_ANFANG).ok_or_else(ungueltig)? + PEM_ANFANG.len();
    let laenge = pem[start..].find(PEM_ENDE).ok_or_else(ungueltig)?;
    let inhalt: String = pem[start..start + laenge]
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let mut sauber = String::from(PEM_ANFANG);
    sauber.push('\n');
    for teil in inhalt.as_bytes().chunks(64) {
        sauber.push_str(std::str::from_utf8(teil).map_err(|_| ungueltig())?);
        sauber.push('\n');
    }
    sauber.push_str(PEM_ENDE);
    sauber.push('\n');
    CertificateDer::from_pem_slice(sauber.as_bytes()).map_err(|_| ungueltig())
}

/// Baut die rustls-Konfiguration.
///
/// Ohne `angeheftet` wird gegen die Mozilla-Wurzelzertifikate geprüft (inklusive Hostname).
/// Mit `angeheftet` wird nur genau dieses Zertifikat akzeptiert.
pub fn client_config(angeheftet: Option<&str>) -> Result<Arc<ClientConfig>, VerbindungsFehler> {
    let provider = krypto();
    let builder = ClientConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
        .map_err(|_| {
            VerbindungsFehler::Protokoll("TLS konnte nicht eingerichtet werden.".into())
        })?;

    let config = match angeheftet {
        None => {
            let wurzeln = RootCertStore {
                roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
            };
            builder
                .with_root_certificates(wurzeln)
                .with_no_client_auth()
        }
        Some(pem) => {
            let zertifikat = zertifikat_lesen(pem)?;
            builder
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(AngeheftetesZertifikat {
                    zertifikat,
                    provider,
                }))
                .with_no_client_auth()
        }
    };
    Ok(Arc::new(config))
}

/// Prüfer, der ausschliesslich ein vom Nutzer hinterlegtes Zertifikat akzeptiert.
///
/// Das Serverzertifikat muss Byte für Byte dem hinterlegten entsprechen. Die Signatur
/// im Handshake wird wie üblich geprüft, damit nur der Besitzer des privaten Schlüssels
/// die Verbindung aufbauen kann.
#[derive(Debug)]
struct AngeheftetesZertifikat {
    zertifikat: CertificateDer<'static>,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for AngeheftetesZertifikat {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if end_entity.as_ref() == self.zertifikat.as_ref() {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// Öffnet eine TCP-Verbindung mit Zeitlimit.
pub async fn tcp_verbinden(host: &str, port: u16) -> Result<TcpStream, VerbindungsFehler> {
    match tokio::time::timeout(VERBINDUNGS_ZEITLIMIT, TcpStream::connect((host, port))).await {
        Err(_) => Err(VerbindungsFehler::Zeitueberschreitung),
        Ok(Err(_)) => Err(VerbindungsFehler::NichtErreichbar {
            server: format!("{host}:{port}"),
        }),
        Ok(Ok(strom)) => {
            let _ = strom.set_nodelay(true);
            Ok(strom)
        }
    }
}

/// Startet TLS auf einer bestehenden TCP-Verbindung.
pub async fn tls_starten(
    strom: TcpStream,
    host: &str,
    angeheftet: Option<&str>,
) -> Result<TlsStrom, VerbindungsFehler> {
    let config = client_config(angeheftet)?;
    let name = ServerName::try_from(host.to_owned()).map_err(|_| {
        VerbindungsFehler::Eingabe(format!("„{host}“ ist kein gültiger Servername."))
    })?;
    let verbinder = TlsConnector::from(config);
    match tokio::time::timeout(VERBINDUNGS_ZEITLIMIT, verbinder.connect(name, strom)).await {
        Err(_) => Err(VerbindungsFehler::Zeitueberschreitung),
        Ok(Err(fehler)) => Err(tls_fehler_einordnen(&fehler, angeheftet.is_some())),
        Ok(Ok(tls)) => Ok(TlsStrom(tls)),
    }
}

/// Ordnet einen Fehler aus dem TLS-Handshake einer verständlichen Meldung zu.
pub fn tls_fehler_einordnen(fehler: &io::Error, angeheftet: bool) -> VerbindungsFehler {
    let rustls_fehler = fehler
        .get_ref()
        .and_then(|innen| innen.downcast_ref::<rustls::Error>());
    match rustls_fehler {
        Some(rustls::Error::InvalidCertificate(grund)) => {
            let beschreibung = match grund {
                CertificateError::Expired | CertificateError::ExpiredContext { .. } => {
                    "Das Zertifikat ist abgelaufen.".to_string()
                }
                CertificateError::NotValidYet | CertificateError::NotValidYetContext { .. } => {
                    "Das Zertifikat ist noch nicht gültig.".to_string()
                }
                CertificateError::NotValidForName
                | CertificateError::NotValidForNameContext { .. } => {
                    "Das Zertifikat gehört nicht zu diesem Servernamen.".to_string()
                }
                CertificateError::UnknownIssuer => {
                    "Das Zertifikat stammt von keiner vertrauenswürdigen Stelle (z. B. selbstsigniert)."
                        .to_string()
                }
                CertificateError::Revoked => "Das Zertifikat wurde widerrufen.".to_string(),
                CertificateError::ApplicationVerificationFailure if angeheftet => {
                    "Das Zertifikat des Servers stimmt nicht mit dem hinterlegten Zertifikat überein."
                        .to_string()
                }
                _ => "Das Zertifikat des Servers wird nicht anerkannt (z. B. selbstsigniert oder fehlerhaft)."
                    .to_string(),
            };
            VerbindungsFehler::ZertifikatUngueltig { grund: beschreibung }
        }
        Some(rustls::Error::PeerIncompatible(_)) => VerbindungsFehler::Protokoll(
            "Der Server unterstützt keine sichere TLS-Version (mindestens TLS 1.2 nötig).".into(),
        ),
        Some(rustls::Error::InvalidMessage(_)) => VerbindungsFehler::Protokoll(
            "Der Server antwortet nicht verschlüsselt. Spricht der Server an diesem Port TLS (z. B. 993 bzw. 465)?"
                .into(),
        ),
        Some(_) => VerbindungsFehler::Protokoll(
            "Die verschlüsselte Verbindung konnte nicht aufgebaut werden.".into(),
        ),
        // Verbindung während des Handshakes abgebrochen (z. B. Netz weg).
        None => VerbindungsFehler::Unterbrochen,
    }
}

/// Verschlüsselte Verbindung zu einem Mailserver.
pub struct TlsStrom(pub TlsStream<TcpStream>);

impl fmt::Debug for TlsStrom {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TlsStrom")
    }
}

impl TlsStrom {
    /// Adresse der Gegenstelle.
    pub fn gegenstelle(&self) -> io::Result<std::net::SocketAddr> {
        self.0.get_ref().0.peer_addr()
    }
}

impl AsyncRead for TlsStrom {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_read(cx, buf)
    }
}

impl AsyncWrite for TlsStrom {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
}

/// Unverschlüsselter Teil einer STARTTLS-Verbindung.
///
/// Liest bewusst Byte für Byte und ohne Puffer: So kann nichts, was ein Angreifer
/// vor dem TLS-Start einschleust, in die verschlüsselte Sitzung gelangen
/// (Schutz gegen „STARTTLS command injection“). Übrig gebliebene Bytes würden den
/// TLS-Handshake scheitern lassen.
pub struct KlartextVorTls {
    strom: TcpStream,
}

impl KlartextVorTls {
    pub fn neu(strom: TcpStream) -> Self {
        Self { strom }
    }

    /// Liest eine Zeile (ohne CRLF).
    pub async fn zeile_lesen(&mut self) -> Result<String, VerbindungsFehler> {
        let mut zeile = Vec::new();
        let lesen = async {
            loop {
                let mut byte = [0u8; 1];
                let n = self.strom.read(&mut byte).await?;
                if n == 0 {
                    return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
                }
                if byte[0] == b'\n' {
                    break;
                }
                zeile.push(byte[0]);
                if zeile.len() > MAX_ZEILENLAENGE {
                    return Err(io::Error::from(io::ErrorKind::InvalidData));
                }
            }
            Ok(())
        };
        match tokio::time::timeout(ANTWORT_ZEITLIMIT, lesen).await {
            Err(_) => return Err(VerbindungsFehler::Zeitueberschreitung),
            Ok(Err(_)) => {
                return Err(VerbindungsFehler::Protokoll(
                    "Der Server hat die Verbindung unerwartet beendet.".into(),
                ))
            }
            Ok(Ok(())) => {}
        }
        if zeile.last() == Some(&b'\r') {
            zeile.pop();
        }
        Ok(String::from_utf8_lossy(&zeile).into_owned())
    }

    /// Schreibt eine Befehlszeile (CRLF wird angehängt).
    pub async fn zeile_schreiben(&mut self, zeile: &str) -> Result<(), VerbindungsFehler> {
        let mut daten = zeile.as_bytes().to_vec();
        daten.extend_from_slice(b"\r\n");
        self.strom.write_all(&daten).await.map_err(|_| {
            VerbindungsFehler::Protokoll("Der Server hat die Verbindung unerwartet beendet.".into())
        })?;
        self.strom.flush().await.map_err(|_| {
            VerbindungsFehler::Protokoll("Der Server hat die Verbindung unerwartet beendet.".into())
        })
    }

    /// Gibt die TCP-Verbindung für den TLS-Start zurück.
    pub fn in_tcp(self) -> TcpStream {
        self.strom
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn beispiel_pem() -> String {
        rcgen::generate_simple_self_signed(vec!["localhost".to_string()])
            .unwrap()
            .cert
            .pem()
    }

    #[test]
    fn pem_ohne_zeilenumbrueche_wird_gelesen() {
        let pem = beispiel_pem();
        let erwartet = zertifikat_lesen(&pem).unwrap();
        let einzeilig = pem.replace('\n', "");
        assert_eq!(zertifikat_lesen(&einzeilig).unwrap(), erwartet);
        let mit_leerraum = format!("  \r\n{}\r\n  ", pem.replace('\n', "\r\n "));
        assert_eq!(zertifikat_lesen(&mit_leerraum).unwrap(), erwartet);
    }

    #[test]
    fn mehrere_zertifikate_werden_abgelehnt() {
        let doppelt = format!("{}{}", beispiel_pem(), beispiel_pem());
        assert!(zertifikat_lesen(&doppelt).is_err());
    }

    #[test]
    fn ungueltiges_pem_wird_abgelehnt() {
        assert!(
            zertifikat_lesen("-----BEGIN CERTIFICATE-----\n!!!\n-----END CERTIFICATE-----")
                .is_err()
        );
        let fehler = zertifikat_lesen("kein Zertifikat").unwrap_err();
        assert!(matches!(fehler, VerbindungsFehler::Eingabe(_)));
    }

    #[test]
    fn standard_konfiguration_laesst_sich_bauen() {
        let config = client_config(None).unwrap();
        // Nur TLS 1.2 und 1.3 sind erlaubt.
        assert!(config
            .crypto_provider()
            .cipher_suites
            .iter()
            .all(|suite| matches!(
                suite.version().version,
                rustls::ProtocolVersion::TLSv1_2 | rustls::ProtocolVersion::TLSv1_3
            )));
    }
}
