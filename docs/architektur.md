# Architektur

SicherMail ist eine [Tauri-2](https://v2.tauri.app/)-Anwendung:

- **Frontend** (`src/`): TypeScript ohne Framework, gebaut mit Vite. Es zeigt nur an und ruft
  Backend-Befehle über `invoke` auf. Es hat keinen Zugriff auf Dateisystem, Shell oder Netzwerk.
- **Backend** (`src-tauri/src/`): Rust. Alle Verbindungen zu Mailservern, das Speichern von
  Daten und das Aufbereiten von Mails passieren hier.

## Backend-Module

| Modul      | Aufgabe                                                           |
| ---------- | ----------------------------------------------------------------- |
| `konto`    | Konten mit eigenem IMAP-/SMTP-Server, Passwörter im Schlüsselbund |
| `imap`     | Mails empfangen, IDLE, Synchronisation                            |
| `smtp`     | Mails senden                                                      |
| `speicher` | verschlüsselter lokaler Zwischenspeicher                          |
| `anzeige`  | Mails parsen und HTML bereinigen                                  |
| `tls`      | gemeinsame TLS-Regeln für IMAP und SMTP                           |
| `befehle`  | Befehle, die das Frontend über `invoke` aufrufen darf             |

## Verbindungen

Alle Verbindungen zu Mailservern baut `tls` auf:

- rustls mit den Protokollen TLS 1.3 und TLS 1.2, Zertifikatsprüfung gegen die Mozilla-Wurzelzertifikate
  (`webpki-roots`) inklusive Hostname.
- Hat der Nutzer ein Zertifikat hinterlegt (Pinning), wird ausschliesslich dieses Zertifikat akzeptiert
  (byteweiser Vergleich), die Handshake-Signatur wird weiterhin geprüft.
- STARTTLS: Der unverschlüsselte Teil (Begrüssung, CAPABILITY/EHLO, STARTTLS) wird ohne Puffer Byte
  für Byte gelesen. Fehlt STARTTLS in der Antwort des Servers, wird abgebrochen. Vor TLS eingeschleuste
  Daten lassen den Handshake scheitern.
- SMTP: Die TLS-Verbindung wird an `lettre` übergeben (`AsyncSmtpConnection::connect_with_transport`),
  damit für SMTP dieselben Regeln gelten wie für IMAP. Im EHLO steht `[127.0.0.1]` statt des Rechnernamens.

## Konten

- `konten.json` im App-Datenordner (Version, Liste der Konten, ohne Passwörter), atomar geschrieben,
  unter Unix mit Rechten `0600`.
- Passwörter im Schlüsselbund (Crate `keyring`, Dienst „SicherMail“, Eintrag `konto:<id>`).
- Gespeichert wird erst nach erfolgreichem Verbindungstest zu IMAP und SMTP.

## Tauri-Sicherheit

- Strenge Content-Security-Policy in `src-tauri/tauri.conf.json`: nur eigene Skripte,
  keine Remote-URLs, keine Formulare, keine Fremd-Frames.
- Capabilities (`src-tauri/capabilities/default.json`): nur das Empfangen von Ereignissen.
  Kein Shell-, Dateisystem- oder HTTP-Zugriff aus dem Frontend.

## Mails empfangen (`imap`)

- **Zwei Verbindungen pro Konto**: eine für Befehle der Oberfläche (`dienst`, wird wiederverwendet und bei
  Abbruch einmal automatisch neu aufgebaut) und eine zum Beobachten des Posteingangs (`beobachter`).
- **Beobachter**: IMAP IDLE (alle 25 Minuten erneuert), sonst NOOP und Abgleich alle 2 Minuten. Bei Abbruch
  neu verbinden mit wachsender Wartezeit (1 s bis 5 min). Die Oberfläche erfährt über das Ereignis
  `mail-ereignis` von Änderungen (`mails_geaendert`, `verbunden`, `getrennt`).
- **Abgleich** (`sync`): Ordner auswählen; ändert sich UIDVALIDITY, wird der lokale Stand verworfen.
  Neue Mails: `UID FETCH <höchste bekannte UID + 1>:*`. Bekannte Mails: Markierungen abgleichen, was
  der Server nicht mehr hat, lokal entfernen. Mails werden über ihre UID gespeichert, daher gibt es keine
  Duplikate; da immer ab der höchsten bekannten UID nachgeladen wird, fehlt nach Abbrüchen keine.
- **Seitenweise**: Erster Abgleich lädt die neuesten 100 Köpfe, ältere werden mit `UID SEARCH` nachgeladen.
- **Inhalte** mit `BODY.PEEK[]`, damit Mails nicht ungefragt als gelesen gelten.
- **Nie automatisch löschen**: kein `CLOSE`, kein allgemeines `EXPUNGE`. Gelöscht wird nur auf Wunsch des
  Nutzers: Verschieben in den Papierkorb (`UID MOVE`, sonst `UID COPY` + `\Deleted`), endgültig nur die
  eine Mail per `UID EXPUNGE` (UIDPLUS). Ohne UIDPLUS bleibt sie als gelöscht markiert und wird ausgeblendet.
- **Zwischenspeicher** über die Schnittstelle `MailCache`; derzeit im Arbeitsspeicher (`SpeicherCache`).

## Mails anzeigen (`anzeige`)

- **MIME** mit `mail-parser`: Betreff, Adressen, Datum, Message-ID und References, Text- und HTML-Teil,
  eingebettete Bilder (`cid:`) und Anhänge.
- **HTML-Bereinigung** mit `ammonia` im Backend: nur eine Liste harmloser Tags und Attribute; entfernt werden
  u. a. `script`, `style`, `iframe`, `object`, `form`, `input`, `svg`, alle Event-Attribute, Kommentare,
  CSS mit `url()`/`expression()`/`@import` und Links ausser `http`, `https`, `mailto`.
  Bilder: nur `data:` (PNG, JPEG, GIF, WebP) und eingebettete `cid:`-Bilder; externe Bilder werden entfernt
  und gezählt.
- **Externe Inhalte** lädt auf Wunsch das Backend (`extern_inhalte`): nur HTTPS, nur Bilder, höchstens
  5 MB je Bild und 40 Bilder, ohne Cookies und Referer; eingesetzt als `data:`-Adressen.
  Erlaubte Absender stehen in `einstellungen.json`.
- **Anzeige** im Frontend (`src/postfach/sicher.ts`): `iframe` mit `sandbox="allow-same-origin"` (also ohne
  `allow-scripts`, Formulare, Popups und Navigation des Hauptfensters) und eigener CSP
  `default-src 'none'; img-src data:; style-src 'unsafe-inline'`. Links im iframe werden vom Hauptfenster
  auf `about:blank#sichermail-link-<n>` umgeschrieben; ein Klick lädt nur ein leeres Dokument im iframe,
  das Hauptfenster erkennt die Marke, stellt den Inhalt wieder her, prüft den Link (`linkPruefen`) und
  öffnet ihn über den Backend-Befehl `link_oeffnen` im Standardbrowser.
- **Anhänge** speichert der Backend-Befehl `anhang_speichern` über den Dialog „Speichern unter“ (Dateiname
  gesäubert). Das Frontend hat weder Dialog- noch Dateisystemrechte.
