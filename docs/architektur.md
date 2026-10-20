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
| `pgp`      | OpenPGP: Schlüssel verwalten, Mails entschlüsseln und prüfen       |
| `smime`    | S/MIME: Zertifikate verwalten, Mails entschlüsseln und prüfen      |
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
- Optional pro Konto: `absender_kennung` (Server-Kennung für die Absenderprüfung; leer = IMAP-Host). Ältere
  `konten.json` ohne das Feld laden unverändert.

## Tauri-Sicherheit

- Strenge Content-Security-Policy in `src-tauri/tauri.conf.json`: nur eigene Skripte,
  keine Remote-URLs, keine Formulare, keine Fremd-Frames.
- Capabilities (`src-tauri/capabilities/default.json`): nur das Empfangen von Ereignissen.
  Kein Shell-, Dateisystem- oder HTTP-Zugriff aus dem Frontend.

## Mails empfangen (`imap`)

- **Zwei Verbindungen pro Konto**: eine für Befehle der Oberfläche (`dienst`, wird wiederverwendet und bei
  Abbruch einmal automatisch neu aufgebaut) und eine zum Beobachten des Posteingangs (`beobachter`).
  COPY und APPEND werden nach einem Abbruch nicht wiederholt, sobald sie gesendet wurden (sonst könnten Mails
  doppelt entstehen); vorher wird die Verbindung mit `SELECT` bzw. `NOOP` geprüft.
- **Beobachter**: IMAP IDLE, alle 2 Minuten erneuert (eine still abgerissene Verbindung fällt so spätestens
  dann auf), danach Abgleich. Ohne IDLE – oder wenn der Server IDLE wiederholt ablehnt – NOOP und Abgleich
  alle 2 Minuten. Bei Abbruch neu verbinden mit wachsender Wartezeit (1 s bis 5 min); zurückgesetzt wird sie
  erst nach einem erfolgreichen Abgleich. Die Oberfläche erfährt über das Ereignis
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
- **Zwischenspeicher** über die Schnittstelle `MailCache`: verschlüsselt auf der Festplatte (`speicher::SqlCache`,
  Standard) oder nur im Arbeitsspeicher (`SpeicherCache`, wenn „Mails lokal speichern“ aus ist).
- **Robustheit**: Nach jedem Fehler wird die Befehlsverbindung verworfen, damit nie mit halb gelesenen Antworten
  weitergearbeitet wird. Fehlt in einer Antwort Unerwartetes (z. B. der Posteingang in der Ordnerliste oder eine
  gerade gemeldete Mail), wird einmal mit frischer Verbindung wiederholt.

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
  Erlaubte Absender stehen in `einstellungen.json`. Standardmässig `no_proxy()` und ein eigener DNS-Resolver, der nur
  öffentliche Adressen zulässt. Ist „Proxy für externe Bilder verwenden“ eingeschaltet (`proxy_externe_bilder`,
  `proxy_adresse`), gehen alle Anfragen über `reqwest::Proxy::all` mit genau dieser geprüften Adresse (`ProxyAdresse`);
  vorher werden Ziele mit IP-Adressen aus dem eigenen Netz und lokale Namen abgelehnt.
- **Anzeige** im Frontend (`src/postfach/sicher.ts`): `iframe` mit `sandbox="allow-same-origin"` (also ohne
  `allow-scripts`, Formulare, Popups und Navigation des Hauptfensters) und eigener CSP
  `default-src 'none'; img-src data:; style-src 'unsafe-inline'`. Links im iframe werden vom Hauptfenster
  auf `about:blank#sichermail-link-<n>` umgeschrieben; ein Klick lädt nur ein leeres Dokument im iframe,
  das Hauptfenster erkennt die Marke, stellt den Inhalt wieder her, prüft den Link (`linkPruefen`) und
  öffnet ihn über den Backend-Befehl `link_oeffnen` im Standardbrowser.
- **Absenderprüfung** (`absender`, #45): liest alle `Authentication-Results`-Header roh (mit `mail-parser`-Offsets),
  entfernt Kommentare und Leerraum, teilt nach RFC 8601 in authserv-id und `methode=ergebnis` mit Eigenschaften
  (`header.d`, `header.i`, `smtp.mailfrom`, `header.from`). Ausgewertet wird nur der oberste Header mit der Kennung des
  Kontos (`Konto::absender_kennung()`, Standard IMAP-Host). `bewerten()` ist eine reine Funktion und ergibt die Stufe
  (`bestaetigt`, `nicht_geprueft`, `warnung`); sie steht in `AufbereiteteMail::absender_pruefung`.
  `EinstellungsSpeicher::extern_erlaubt_fuer` lässt die dauerhafte Freigabe externer Inhalte bei `warnung` nicht gelten.
- **Anhänge** speichert der Backend-Befehl `anhang_speichern` über den Dialog „Speichern unter“ (Dateiname
  gesäubert). Das Frontend hat weder Dialog- noch Dateisystemrechte.

## Lokaler Speicher (`speicher`)

- SQLite mit SQLCipher (`rusqlite`, Feature `bundled-sqlcipher-vendored-openssl`), Datei `mails.db`.
- Schlüssel: 32 Zufallsbytes (`ring::rand::SystemRandom`), als Hex im Schlüsselbund (`speicher:schluessel`), als
  Rohschlüssel gesetzt (`PRAGMA key = "x'…'"`), nie in Dateien oder Logs. Passt der Schlüssel nicht oder fehlt er,
  wird die Datei verworfen und neu angelegt.
- `PRAGMA secure_delete = ON`, Dateirechte `0600`. „Lokalen Speicher leeren“ löscht alle Zeilen und führt `VACUUM` aus.
- Tabellen `ordner_stand` (UIDVALIDITY/UIDNEXT je Ordner) und `mails` (Kopfdaten und optional Inhalt je UID).
- **Suchindex** (#49, Schema-Version 2, `PRAGMA user_version`): FTS5-Tabelle `mails_suche` (`absender`, `betreff`,
  `text`; Tokenizer `unicode61 remove_diacritics 2`) in derselben Datei. Die Zeilennummer steht in der Spalte
  `mails.suche_id` (mit Index); der Trigger `mails_suche_entfernen` löscht den Eintrag mit der Mail, sodass alle
  Wege (Entfernen, neue UIDVALIDITY, Konto löschen) den Index mitnehmen. `SqlCache` trägt neue Köpfe ein, übernimmt
  geänderte Kopfdaten und setzt beim Speichern des Inhalts den Text (`speicher::suche::suchtext`: Textteil von
  `mail-parser`, höchstens 100 000 Zeichen; leer bei `multipart/encrypted`, `application/(x-)pkcs7-mime` oder
  Inline-PGP in irgendeinem Teil).
- **Migration** (additiv): Beim Öffnen werden fehlende Spalte, Tabelle, Index und Trigger angelegt und alle Mails
  ohne `suche_id` eingetragen – beim ersten Start nach dem Update also alle schon gespeicherten. Ältere Versionen
  können die Datei weiter öffnen (zusätzliche Spalte und Trigger stören sie nicht); was sie ohne Index speichern,
  wird beim nächsten Start nachgetragen. Das verwendete SQLCipher (`libsqlite3-sys`, gebündelt) ist mit
  `SQLITE_ENABLE_FTS5` gebaut; ein Test prüft das über `PRAGMA compile_options`.
- **Abfrage**: `speicher::suche::fts_ausdruck` macht aus jedem Wort `"wort"*` (innere `"` verdoppelt), optional mit
  Spaltenfilter `{betreff} : (…)`; Konto, Ordner, Grenze und Versatz sind SQL-Parameter. Sortiert nach Datum,
  200 Treffer je Seite (`mails_suchen`). Der Zwischenspeicher im Arbeitsspeicher sucht ohne Index (Teilwort,
  Gross-/Kleinschreibung egal).
- **Server-Suche** (`imap::suche`, Befehl `mails_server_suchen`): `UID SEARCH` mit `FROM`, `SUBJECT`, `BODY` (bei
  „Alle“ mit `OR` verknüpft) je Wort und `UNDELETED`. ASCII-Wörter als String in Anführungszeichen; Wörter mit `\`
  oder Nicht-ASCII als synchrones Literal (`{n}`, Warten auf `+`, mit `CHARSET UTF-8` bei Nicht-ASCII);
  Anführungszeichen trennen Wörter. Die Antwort wird mit `read_response` gelesen. Die neuesten 200 UIDs je Ordner;
  fehlende Köpfe holt `UID FETCH`, sie werden nicht in den Zwischenspeicher übernommen (sonst übersprünge
  „Ältere Mails laden“ die Lücke).
- **Oberfläche** (`src/postfach/suche.ts`): Suchleiste über der Mailliste (bleibt beim Neuzeichnen bestehen, damit
  Eingabe und Fokus erhalten bleiben), 300 ms Verzögerung, Ctrl/Cmd+F, Hervorheben mit `<mark>` ohne HTML. Einträge
  der Mailliste und Treffer sind `MailEintrag` (Konto, Ordner, Kopfdaten); Öffnen, Markieren, Verschieben und
  Löschen verwenden Konto und Ordner der Mail, nicht die Auswahl in der Seitenleiste.
- Die Oberfläche zeigt die Mailliste sofort aus dem Speicher (`mails_lokal`) und gleicht danach mit dem Server ab.
  Der Beobachter lädt die Inhalte der neuesten 50 Mails im Posteingang (bis 5 MB je Mail) für das Lesen ohne Internet vor.

## Gemeinsamer Posteingang (#50)

- `MailCache::ueber_konten` liest die Mails eines Ordners (hier `INBOX`) mehrerer Konten in einer Abfrage,
  sortiert nach Datum (ohne Datum am Ende, dann UID), seitenweise mit `LIMIT`/`OFFSET`; `MailCache::ungelesen`
  zählt ungelesene. In SQLCipher eine `SELECT … WHERE konto IN (…) AND ordner = ?` mit Parametern.
- `ImapDienst::alle_posteingaenge` (Befehl `alle_posteingaenge`) liest nur lokal. Mit `nachladen`
  („Ältere Mails laden“) ruft es für jedes Konto `mails_seite` ab dessen Anzahl gespeicherter Mails auf (bestehende
  Befehlsverbindung, `UID SEARCH`/`FETCH` für Älteres); ein Konto ohne Verbindung ergibt nur einen Hinweis. Live
  aktualisiert die Oberfläche bei jedem `mails_geaendert` für `INBOX` eines Kontos.
- Suche: `mails_suchen`/`mails_server_suchen` ohne Konto durchsuchen die `INBOX` aller Konten
  (`ImapDienst::server_suchen_posteingaenge` je Konto über dessen Verbindung, Treffer nach Datum zusammengeführt).
- Oberfläche: Auswahl `{ konto: null, ordner: "INBOX" }` in `src/app.ts`; Eintrag nur ab zwei Konten.
  `src/postfach/konten.ts` bildet Kürzel, Namen (bei gleichen Namen die Adresse) und Kontofarbe (`--konto-1` …
  `--konto-6`, hell und dunkel, mindestens 3:1 gegen die Flächen – `tests/design.test.ts`). Antworten und
  Weiterleiten nehmen das Konto der geöffneten Mail (`kontoDerMail`), „Neue Mail“ das erste Konto.

## Mails senden (`smtp`)

- **Verbindung** wie bei IMAP über `tls` (TLS oder STARTTLS-Pflicht, gleiches Pinning), danach Übergabe an `lettre`.
- **OpenPGP beim Senden**: `Entwurf` hat `verschluesseln` und `signieren` (Standard aus, ältere Entwürfe laden
  unverändert). `SendeDienst::pgp_vorbereiten` holt über `PgpDienst::versand_vorbereiten` die Schlüssel aller
  Empfänger, den eigenen Schlüssel und – zum Signieren – die entsperrte Passphrase; fehlt etwas, bricht es mit einer
  Fehlerart (`pgp_schluessel_fehlt`, `pgp_kein_eigener`, `pgp_passphrase`) ab, bevor irgendetwas gesendet wird. Die
  Oberfläche fragt vorab mit `pgp_versand_pruefen`. `nachricht::bauen_mit` erzeugt aus derselben inneren MIME-Entität
  `multipart/encrypted` bzw. `multipart/signed` (signiert wird die Entität ohne das CRLF, das nach RFC 2046 zum
  folgenden Begrenzer gehört). Ergebnis sind `Sendung`en: eine für An/Cc und bei Verschlüsselung eine eigene je
  Bcc-Empfänger; die Kopie für „Gesendet“ ist die An/Cc-Fassung mit Bcc-Kopf. Bauen und Verschlüsseln laufen in
  `spawn_blocking`.
- **Nachricht** (`nachricht`): `lettre`-Builder mit Message-ID `<uuid@domain-des-absenders>`, `In-Reply-To` und
  `References` bei Antworten, `text/plain` (optional `multipart/alternative` mit einfachem, maskiertem HTML) und
  Anhängen als `multipart/mixed`. Bcc steht nur im Umschlag; die Kopie für „Gesendet“ behält den Bcc-Kopf.
- **Anhänge** (`anhaenge`): Das Frontend kennt nur Tokens. Dateien kommen ausschliesslich über den Backend-Dialog
  (`anhang_waehlen`) oder als Anhänge einer weitergeleiteten Mail ins Register; so kann Code in der Oberfläche keine
  beliebigen Dateien verschicken. Höchstens 25 MB pro Mail.
- **Ablauf** (`dienst::SendeDienst`): Die Oberfläche speichert den Entwurf, dann wird gesendet. Erfolg → Entwurf löschen,
  per `IMAP APPEND` mit `\Seen` in „Gesendet“ ablegen (Ordner bei Bedarf anlegen; ein Fehler dabei ist nur ein Hinweis).
  Fehler → Entwurf erneut sichern, Fehlermeldung mit Hinweis „als Entwurf gespeichert“.
- **Entwürfe** liegen im Zwischenspeicher (`MailCache`): in SQLCipher (`entwuerfe`) oder im Arbeitsspeicher.
  Anhänge werden mit ihrer Quelle gespeichert und beim Laden neu ins Register eingetragen. „Lokalen Speicher leeren“
  lässt Entwürfe stehen; beim Umschalten des Speichers werden sie übernommen.

## OpenPGP (`pgp`)

**Bibliothek: rPGP** (Crate `pgp`, MIT/Apache-2.0). Gründe:

- Reines Rust ohne Systembibliotheken (Standard-Feature `bzip2` abgeschaltet): baut unter Windows, macOS und Linux
  ohne zusätzliche Abhängigkeiten. Sequoia-PGP bräuchte für die Kryptografie Nettle oder OpenSSL als
  Systembibliothek (die reine Rust-Variante gilt dort als experimentell), GPGME bräuchte eine installierte GnuPG.
- Unterstützt RFC 9580 und RFC 4880 inkl. Ed25519/Cv25519, RSA anderer, AEAD und das AEAD-Format von GnuPG 2.4
  (OCB), mit dem GnuPG an eigene GnuPG-Schlüssel verschlüsselt; wird u. a. von Delta Chat verwendet.
- Niedrige Ebene: Semantik (Ablauf, Widerruf, Vertrauen) setzt rPGP bewusst nicht um; SicherMail prüft Eigensignaturen
  beim Import und verlangt für „gültig“ die passende User-ID.
- `cargo audit`: Über rPGP kommt `rsa` mit RUSTSEC-2023-0071 (siehe [Sicherheit](sicherheit.md#bekannte-hinweise-aus-cargo-audit)).

Aufbau:

- `schluessel`: Erzeugen (v4, Ed25519 primär zum Zertifizieren und Signieren, Cv25519-Unterschlüssel zum
  Verschlüsseln, Passphrase-geschützt), Einlesen (armored oder binär, Eigensignaturen geprüft), Infos (Fingerabdruck,
  User-IDs, Adressen), Passphrase prüfen.
- `speicher`: `schluessel.db` (SQLCipher, Schlüssel im Schlüsselbund unter `pgp:schluessel`) mit den Tabellen
  `eigene` (Konto, Fingerabdruck, privater Schlüssel armored und S2K-geschützt) und `fremde` (öffentliche Schlüssel).
  Der Öffner für SQLCipher (`speicher::verschluesselt_oeffnen`) wird mit dem Mailspeicher geteilt.
- `lesen`: erkennt `multipart/encrypted`, `multipart/signed` (RFC 3156) und Inline-PGP; entschlüsselt mit
  `Message::decrypt_the_ring` (GnuPG-AEAD erlaubt, SED nicht), prüft eingebettete Signaturen
  (`verify_nested_explicit`), abgetrennte Signaturen (`DetachedSignature`) über die CRLF-kanonische Form des ersten
  Teils und Klartextsignaturen. Ergebnis: `PgpStatus` (Verschlüsselung, Signatur, Signierer, Meldung, ggf.
  Fingerabdruck, dessen Passphrase fehlt) und eine anzeigbare Fassung (äussere Kopfzeilen ohne `Content-*` plus
  entschlüsselte innere MIME-Entität).
- `schreiben`: verschlüsselt eine MIME-Entität (SEIPD v1 mit MDC, AES-256, Signatur innerhalb; Unterschlüssel mit
  Verschlüsselungsrecht, sonst der Hauptschlüssel) bzw. erzeugt eine abgetrennte Signatur (SHA-256, `micalg=pgp-sha256`);
  signiert wird mit einem Unterschlüssel mit Signierrecht, sonst dem Hauptschlüssel.
- `PgpDienst`: öffnet den Speicher beim ersten Zugriff, hält Passphrasen nur im Arbeitsspeicher und baut daraus den
  Schlüsselring. Rechenintensives (S2K, Entschlüsseln) läuft in `spawn_blocking`. `mail_anzeigen`,
  `anhang_speichern`, `mail_anhaenge_uebernehmen` und der Versand weitergeleiteter Anhänge verwenden die entschlüsselte
  Fassung; gespeichert wird immer nur die Rohmail vom Server.
- Oberfläche: `src/schluessel/dialog.ts` (Dialog „Schlüssel und Zertifikate“), Statusleiste und Passphrase-Abfrage in
  `src/postfach/lesen.ts`. Datei-Dialoge laufen im Backend; die Capabilities bleiben unverändert.
- Die S2K-Schlüsselableitung ist ohne Optimierung sehr langsam; `Cargo.toml` optimiert deshalb `pgp` und die
  Hash-Crates auch im Dev-Profil (Tests).

## S/MIME (`smime`)

**Bibliothek: OpenSSL über das Crate `openssl`** (Apache-2.0), mit `vendored`. Gründe:

- OpenSSL wird für SQLCipher (`rusqlite` mit `bundled-sqlcipher-vendored-openssl`) ohnehin schon mitkompiliert;
  `openssl` nutzt genau diese statisch gelinkte Bibliothek (`openssl-sys` mit `vendored`). Es kommt keine
  Systembibliothek zur Laufzeit dazu und keine zweite Kryptobibliothek.
- Die reinen Rust-Crates von RustCrypto (`cms`, `x509-cert`, `der`, `pkcs12`) bilden die Formate ab, bieten aber kein
  Entschlüsseln von CMS, keine Prüfung von Zertifikatsketten (Pfadaufbau, Zeitpunkt, Schlüsselzweck,
  basicConstraints) und PKCS#12 nur als Datenstruktur. Das alles selbst zu bauen wäre fehleranfällig. OpenSSL deckt
  PKCS#12, CMS (EnvelopedData, AuthEnvelopedData mit AES-GCM, SignedData) und `X509_verify_cert` seit Jahren
  zuverlässig ab und ist die Grundlage vieler S/MIME-Programme.
- RSA-Entschlüsselung (PKCS#1 v1.5) schützt OpenSSL 3 mit „implicit rejection“ gegen Bleichenbacher/Marvin; das
  `rsa`-Crate (RUSTSEC-2023-0071) wird für S/MIME nicht verwendet.
- Nicht in `openssl-sys` enthalten sind `CMS_get0_signers`, `CMS_get1_certs` und `X509_check_ca`; sie werden direkt
  aus der mitgelieferten OpenSSL deklariert (kleine, kommentierte `unsafe`-Blöcke). Verfahren (Inhaltsverschlüsselung,
  Hashfunktion), Empfänger und Signaturzeit, die OpenSSL nicht herausgibt, liest `smime::der` (BER wird vorher von
  OpenSSL in DER umgewandelt).
- Wurzeln des Betriebssystems liefert `rustls-native-certs` (Windows-Zertifikatsspeicher, macOS-Schlüsselbund,
  unter Linux die Zertifikatsdateien der Distribution).

Aufbau:

- `zertifikat`: Zertifikate lesen (PEM, DER, PKCS#7 `.p7c`), beschreiben (SHA-256-Fingerabdruck, Name, Adressen aus
  SAN `rfc822Name` bzw. Subject `E=`, Aussteller, Gültigkeit), Schlüsselzweck (KeyUsage, ExtendedKeyUsage
  `emailProtection`) und Schlüsselstärke (RSA ab 2048 Bit, EC ab 256 Bit) prüfen, PKCS#12 importieren. Der private
  Schlüssel wird als PKCS#8 mit der Passphrase geschützt (scrypt N=2^15, r=8, p=1 → AES-256-GCM, Format `SMK1`).
- `speicher`: Tabellen `smime_eigene` (Konto, Zertifikat, Kette, geschützter Schlüssel), `smime_fremde` und
  `smime_wurzeln` in `schluessel.db`. Sie werden beim Öffnen mit `CREATE TABLE IF NOT EXISTS` ergänzt; die Tabellen
  von OpenPGP bleiben unverändert, alte Dateien öffnen weiter. OpenPGP und S/MIME teilen sich eine Verbindung
  (`pgp::speicher::SchluesselDb`).
- `lesen`: erkennt `application/pkcs7-mime` (auch `x-pkcs7-mime`; enveloped-data, authEnveloped-data, signed-data) und
  `multipart/signed` mit `application/pkcs7-signature` (auch `x-pkcs7-signature`). Entschlüsselt mit `CMS_decrypt`
  und dem Zertifikat des passenden Empfängers (Aussteller+Seriennummer oder Schlüssel-ID), nur AES-CBC und AES-GCM.
  Signaturen: erst `CMS_verify` ohne Kettenprüfung (unverändert?), dann eigene Kettenprüfung mit `X509StoreContext`
  gegen System- und übernommene Wurzeln, zum Signaturzeitpunkt (`signingTime`, höchstens jetzt) und mit Zweck
  `smime_sign`, zuletzt Adressvergleich. Das Ergebnis steht im gemeinsamen `PgpStatus` mit `verfahren: smime`,
  zusätzlichen `hinweise` und `zertifikat_uebernehmbar`; die Anzeige baut wie bei OpenPGP auf der Rohmail auf.
- `schreiben`: verschlüsselt eine MIME-Entität (`CMS_encrypt`, EnvelopedData, AES-256-CBC, RSA-Schlüsseltransport
  bzw. ECDH bei EC) und erzeugt abgetrennte Signaturen (`CMS_sign`, SHA-256, mit Zwischenzertifikaten). Beim
  Verschlüsseln wird zuerst ein `multipart/signed` gebaut und dieses verschlüsselt.
- `SmimeDienst`: Verwaltung (Import, Wurzeln mit zweistufiger Bestätigung, Löschen, Entsperren), hält Passphrasen
  nur im Arbeitsspeicher. `mail_anzeigen` fragt zuerst S/MIME, dann OpenPGP. `versand_pruefen`/`versand_vorbereiten`
  wählen je Empfänger das neueste jetzt gültige Zertifikat mit vertrauenswürdiger Kette (Zweck `smime_encrypt`).
- Versand (`smtp::nachricht`): `Schutz::Pgp` bzw. `Schutz::Smime` teilen sich den Ablauf (An/Cc in einer Nachricht
  samt eigenem Zertifikat, je Bcc eine eigene, nur signieren als `multipart/signed`). Das Verfahren steht im Entwurf
  (`verfahren`, Standard `openpgp`, ältere Entwürfe laden unverändert); `pgp_versand_pruefen` prüft je nach Verfahren.
- Oberfläche: Bereich „S/MIME-Zertifikate“ in `src/schluessel/smime.ts`, Statusleiste in `src/postfach/lesen.ts`
  (gleicher Stil wie OpenPGP, Texte mit „S/MIME“ und „Zertifikat“).

## Design der Oberfläche

Alle Farben, Abstände, Radien, Schatten und Schriftgrössen stehen zentral als CSS-Variablen
am Anfang von `src/styles.css` (Hauptfarbe `#2563EB` auf Weiss, Ecken 8px für Knöpfe und
Felder, 12px für Karten und Dialoge, Abstände im 4er/8er-Raster). Im übrigen Code werden
keine festen Farbwerte verwendet; `tests/design.test.ts` prüft das. Die Mailanzeige im
iframe übernimmt ihre Grundfarben beim Aufbau aus den Variablen `--mail-*`.

### Hell und dunkel (#47)

- Jede Farbvariable hat einen hellen Wert (`:root, ::backdrop`) und einen dunklen. Die dunklen Werte stehen
  zweimal gleich im zentralen Bereich: in `@media (prefers-color-scheme: dark)` für „Automatisch“ und unter
  `:root[data-darstellung="dunkel"]` zum Erzwingen; `data-darstellung="hell"` schaltet die Media-Query ab.
  `color-scheme` wird passend gesetzt, damit auch Bildlaufleisten und Formularelemente mitziehen. Statt
  `--weiss` gibt es dafür Rollen: `--grund` (Fenster), `--flaeche` (Karten, Dialoge, Felder), `--auf-farbe`
  (Text auf blauen und roten Flächen).
- Der Mailinhalt im iframe bleibt immer hell (`--mail-grund`, `--mail-text`, `--mail-link` sind in beiden
  Darstellungen gleich), weil viele HTML-Mails nur für Weiss gestaltet sind; nur der Rahmen ist dunkel.
- `tests/design.test.ts` prüft, dass es zu jeder hellen Farbe eine dunkle gibt, dass beide dunklen Blöcke
  gleich sind und dass die wichtigen Paare (Text, leiser Text, Warnung, Fehler, Erfolg, Knöpfe, Fokus) in beiden
  Darstellungen mindestens 4,5:1 Kontrast nach WCAG erreichen.
- Gespeichert wird die Wahl als `darstellung` (`automatisch` | `hell` | `dunkel`) in `einstellungen.json`;
  fehlt das Feld (ältere Dateien) oder ist es unbekannt, gilt `automatisch`.
- Kein helles Aufblitzen: Das Hauptfenster steht in `tauri.conf.json` mit `create: false` und wird in
  `lib.rs` aus dieser Konfiguration erzeugt – mit passendem Fenster-Thema (Titelleiste), dunklem
  Fenstergrund und einem Startskript, das `data-darstellung` setzt, bevor die Seite gezeichnet wird. Bei
  „Automatisch“ genügt die Media-Query; `src/einstellungen/darstellung.ts` reagiert zusätzlich mit
  `matchMedia(...).addEventListener("change", …)` auf Wechsel im Betrieb.
- Die Titelleiste stellt der Befehl `darstellung_setzen` im Backend um (`set_theme`); das Frontend braucht
  dafür keine zusätzliche Berechtigung, die Capabilities bleiben unverändert.
