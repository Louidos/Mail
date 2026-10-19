//! Aufbau einer verschlüsselten IMAP-Verbindung mit Anmeldung.

use tokio::time::timeout;

use crate::fehler::VerbindungsFehler;
use crate::konto::{ServerEinstellung, Verschluesselung};
use crate::tls::{self, KlartextVorTls, TlsStrom, VERBINDUNGS_ZEITLIMIT};

/// Angemeldete IMAP-Sitzung über TLS.
pub type ImapSitzung = async_imap::Session<TlsStrom>;

/// Verbindet sich verschlüsselt mit dem IMAP-Server und meldet sich an.
pub async fn anmelden(
    server: &ServerEinstellung,
    benutzer: &str,
    passwort: &str,
) -> Result<ImapSitzung, VerbindungsFehler> {
    let client = verbinden(server).await?;
    match timeout(VERBINDUNGS_ZEITLIMIT, client.login(benutzer, passwort)).await {
        Err(_) => Err(VerbindungsFehler::Zeitueberschreitung),
        Ok(Ok(sitzung)) => Ok(sitzung),
        Ok(Err((fehler, _))) => Err(match fehler {
            async_imap::error::Error::No(_) | async_imap::error::Error::Bad(_) => {
                VerbindungsFehler::AnmeldungFalsch
            }
            _ => VerbindungsFehler::Protokoll(
                "Die Verbindung zum IMAP-Server wurde bei der Anmeldung unterbrochen.".into(),
            ),
        }),
    }
}

/// Baut die verschlüsselte Verbindung auf (noch ohne Anmeldung).
pub async fn verbinden(
    server: &ServerEinstellung,
) -> Result<async_imap::Client<TlsStrom>, VerbindungsFehler> {
    let tcp = tls::tcp_verbinden(&server.host, server.port).await?;
    let angeheftet = server.zertifikat.as_deref();
    match server.verschluesselung {
        Verschluesselung::Tls => {
            let strom = tls::tls_starten(tcp, &server.host, angeheftet).await?;
            let mut client = async_imap::Client::new(strom);
            begruessung_pruefen(&mut client).await?;
            Ok(client)
        }
        Verschluesselung::Starttls => {
            let tcp = starttls_aushandeln(tcp).await?;
            let strom = tls::tls_starten(tcp, &server.host, angeheftet).await?;
            Ok(async_imap::Client::new(strom))
        }
    }
}

async fn begruessung_pruefen(
    client: &mut async_imap::Client<TlsStrom>,
) -> Result<(), VerbindungsFehler> {
    let antwort = timeout(VERBINDUNGS_ZEITLIMIT, client.read_response())
        .await
        .map_err(|_| VerbindungsFehler::Zeitueberschreitung)?;
    match antwort {
        Ok(Some(daten)) => match daten.parsed() {
            async_imap::imap_proto::Response::Data {
                status: async_imap::imap_proto::Status::Ok | async_imap::imap_proto::Status::PreAuth,
                ..
            } => Ok(()),
            _ => Err(VerbindungsFehler::Protokoll(
                "Der IMAP-Server hat die Verbindung abgelehnt.".into(),
            )),
        },
        _ => Err(VerbindungsFehler::Protokoll(
            "Der Server antwortet nicht wie ein IMAP-Server.".into(),
        )),
    }
}

/// Unverschlüsselter Teil von STARTTLS: Begrüssung, CAPABILITY, STARTTLS.
///
/// Bietet der Server kein STARTTLS an, wird abgebrochen.
async fn starttls_aushandeln(
    tcp: tokio::net::TcpStream,
) -> Result<tokio::net::TcpStream, VerbindungsFehler> {
    let mut klartext = KlartextVorTls::neu(tcp);

    let begruessung = klartext.zeile_lesen().await?;
    let begruessung_gross = begruessung.to_ascii_uppercase();
    if begruessung_gross.starts_with("* PREAUTH") {
        // Eine Sitzung ohne Anmeldung vor TLS liesse sich nicht mehr absichern.
        return Err(VerbindungsFehler::Protokoll(
            "Der IMAP-Server meldet eine Sitzung ohne Verschlüsselung. Verbindung abgebrochen."
                .into(),
        ));
    }
    if !begruessung_gross.starts_with("* OK") {
        return Err(VerbindungsFehler::Protokoll(
            "Der Server antwortet nicht wie ein IMAP-Server.".into(),
        ));
    }

    klartext.zeile_schreiben("a1 CAPABILITY").await?;
    let mut faehigkeiten = Vec::new();
    loop {
        let zeile = klartext.zeile_lesen().await?;
        let gross = zeile.to_ascii_uppercase();
        if gross.starts_with("A1 ") {
            if !gross.starts_with("A1 OK") {
                return Err(VerbindungsFehler::KeinStartTls);
            }
            break;
        }
        if let Some(rest) = gross.strip_prefix("* CAPABILITY ") {
            faehigkeiten.extend(rest.split_whitespace().map(str::to_string));
        }
    }
    if !faehigkeiten.iter().any(|f| f == "STARTTLS") {
        return Err(VerbindungsFehler::KeinStartTls);
    }

    klartext.zeile_schreiben("a2 STARTTLS").await?;
    loop {
        let zeile = klartext.zeile_lesen().await?.to_ascii_uppercase();
        if zeile.starts_with("A2 ") {
            if zeile.starts_with("A2 OK") {
                break;
            }
            return Err(VerbindungsFehler::KeinStartTls);
        }
    }
    Ok(klartext.in_tcp())
}
