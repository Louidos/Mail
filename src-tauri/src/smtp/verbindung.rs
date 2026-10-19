//! Aufbau einer verschlüsselten SMTP-Verbindung mit Anmeldung.
//!
//! Die TLS-Verbindung baut SicherMail selbst auf (gleiche Regeln wie bei IMAP,
//! inklusive Zertifikats-Pinning) und übergibt sie danach an `lettre`.

use std::io;
use std::net::{Ipv4Addr, SocketAddr};
use std::pin::Pin;
use std::task::{Context, Poll};

use lettre::transport::smtp::authentication::{Credentials, Mechanism};
use lettre::transport::smtp::client::{AsyncSmtpConnection, AsyncTokioStream};
use lettre::transport::smtp::extension::ClientId;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::time::timeout;

use crate::fehler::VerbindungsFehler;
use crate::konto::{ServerEinstellung, Verschluesselung};
use crate::tls::{self, KlartextVorTls, TlsStrom, VERBINDUNGS_ZEITLIMIT};

/// Angemeldete SMTP-Verbindung.
pub type SmtpVerbindung = AsyncSmtpConnection;

/// Name im EHLO. Bewusst neutral, damit der Rechnername nicht verraten wird.
pub fn client_id() -> ClientId {
    ClientId::Ipv4(Ipv4Addr::LOCALHOST)
}

/// Verbindet sich verschlüsselt mit dem SMTP-Server und meldet sich an.
pub async fn anmelden(
    server: &ServerEinstellung,
    benutzer: &str,
    passwort: &str,
) -> Result<SmtpVerbindung, VerbindungsFehler> {
    let mut verbindung = verbinden(server).await?;
    let zugang = Credentials::new(benutzer.to_string(), passwort.to_string());
    let anmeldung = verbindung.auth(&[Mechanism::Plain, Mechanism::Login], &zugang);
    match timeout(VERBINDUNGS_ZEITLIMIT, anmeldung).await {
        Err(_) => Err(VerbindungsFehler::Zeitueberschreitung),
        Ok(Ok(_)) => Ok(verbindung),
        Ok(Err(fehler)) if fehler.is_permanent() => Err(VerbindungsFehler::AnmeldungFalsch),
        Ok(Err(fehler)) if fehler.is_client() => Err(VerbindungsFehler::Protokoll(
            "Der SMTP-Server bietet kein unterstütztes Anmeldeverfahren an.".into(),
        )),
        Ok(Err(_)) => Err(VerbindungsFehler::Protokoll(
            "Die Anmeldung beim SMTP-Server ist fehlgeschlagen.".into(),
        )),
    }
}

/// Baut die verschlüsselte Verbindung auf (noch ohne Anmeldung).
pub async fn verbinden(server: &ServerEinstellung) -> Result<SmtpVerbindung, VerbindungsFehler> {
    let tcp = tls::tcp_verbinden(&server.host, server.port).await?;
    let angeheftet = server.zertifikat.as_deref();
    let strom: Box<dyn AsyncTokioStream> = match server.verschluesselung {
        Verschluesselung::Tls => Box::new(SmtpStrom {
            vorspann: Vec::new(),
            tls: tls::tls_starten(tcp, &server.host, angeheftet).await?,
        }),
        Verschluesselung::Starttls => {
            let tcp = tls::aushandeln_mit_zeitlimit(starttls_aushandeln(tcp)).await?;
            Box::new(SmtpStrom {
                // lettre erwartet eine Begrüssung; nach STARTTLS sendet der Server keine
                // neue. Deshalb wird hier eine eingesetzt, danach folgt EHLO über TLS.
                vorspann: b"220 STARTTLS aktiv\r\n".to_vec(),
                tls: tls::tls_starten(tcp, &server.host, angeheftet).await?,
            })
        }
    };
    match timeout(
        VERBINDUNGS_ZEITLIMIT,
        AsyncSmtpConnection::connect_with_transport(strom, &client_id()),
    )
    .await
    {
        Err(_) => Err(VerbindungsFehler::Zeitueberschreitung),
        Ok(Err(_)) => Err(VerbindungsFehler::Protokoll(
            "Der Server antwortet nicht wie ein SMTP-Server.".into(),
        )),
        Ok(Ok(verbindung)) => Ok(verbindung),
    }
}

/// Liest eine (ggf. mehrzeilige) SMTP-Antwort und gibt Code und Zeilen zurück.
async fn antwort_lesen(
    klartext: &mut KlartextVorTls,
) -> Result<(u16, Vec<String>), VerbindungsFehler> {
    let mut zeilen = Vec::new();
    loop {
        let zeile = klartext.zeile_lesen().await?;
        let code = zeile
            .get(..3)
            .and_then(|c| c.parse::<u16>().ok())
            .ok_or_else(|| {
                VerbindungsFehler::Protokoll(
                    "Der Server antwortet nicht wie ein SMTP-Server.".into(),
                )
            })?;
        let fortsetzung = zeile.as_bytes().get(3) == Some(&b'-');
        zeilen.push(zeile.get(4..).unwrap_or_default().to_string());
        if !fortsetzung {
            return Ok((code, zeilen));
        }
        if zeilen.len() > 100 {
            return Err(VerbindungsFehler::Protokoll(
                "Der SMTP-Server sendet eine ungültige Antwort.".into(),
            ));
        }
    }
}

/// Unverschlüsselter Teil von STARTTLS: Begrüssung, EHLO, STARTTLS.
///
/// Bietet der Server kein STARTTLS an, wird abgebrochen.
async fn starttls_aushandeln(
    tcp: tokio::net::TcpStream,
) -> Result<tokio::net::TcpStream, VerbindungsFehler> {
    let mut klartext = KlartextVorTls::neu(tcp);

    let (code, _) = antwort_lesen(&mut klartext).await?;
    if code != 220 {
        return Err(VerbindungsFehler::Protokoll(
            "Der SMTP-Server hat die Verbindung abgelehnt.".into(),
        ));
    }

    klartext
        .zeile_schreiben(&format!("EHLO {}", client_id()))
        .await?;
    let (code, zeilen) = antwort_lesen(&mut klartext).await?;
    if code != 250 {
        return Err(VerbindungsFehler::KeinStartTls);
    }
    let bietet_starttls = zeilen.iter().any(|zeile| {
        zeile
            .split_whitespace()
            .next()
            .is_some_and(|wort| wort.eq_ignore_ascii_case("STARTTLS"))
    });
    if !bietet_starttls {
        return Err(VerbindungsFehler::KeinStartTls);
    }

    klartext.zeile_schreiben("STARTTLS").await?;
    let (code, _) = antwort_lesen(&mut klartext).await?;
    if code != 220 {
        return Err(VerbindungsFehler::KeinStartTls);
    }
    Ok(klartext.in_tcp())
}

/// TLS-Verbindung für lettre, optional mit vorangestellten Bytes beim Lesen.
#[derive(Debug)]
struct SmtpStrom {
    vorspann: Vec<u8>,
    tls: TlsStrom,
}

impl AsyncTokioStream for SmtpStrom {
    fn peer_addr(&self) -> io::Result<SocketAddr> {
        self.tls.gegenstelle()
    }
}

impl AsyncRead for SmtpStrom {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if !self.vorspann.is_empty() {
            let anzahl = self.vorspann.len().min(buf.remaining());
            let teil: Vec<u8> = self.vorspann.drain(..anzahl).collect();
            buf.put_slice(&teil);
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut self.tls).poll_read(cx, buf)
    }
}

impl AsyncWrite for SmtpStrom {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.tls).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.tls).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.tls).poll_shutdown(cx)
    }
}
