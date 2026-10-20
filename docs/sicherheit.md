# Sicherheitskonzept

Dieses Dokument beschreibt, was SicherMail schützt, wie es das tut und was bewusst (noch) nicht abgedeckt ist.
Sicherheitslücken bitte wie in [SECURITY.md](../SECURITY.md) beschrieben melden.

## Was geschützt wird

| Schutzgut | Bedrohung |
| --- | --- |
| **Zugangsdaten** (Passwörter, Schlüssel, private OpenPGP-Schlüssel und Passphrasen) | Mitlesen im Netz, Auslesen aus Dateien oder Logs, Weitergabe an Dritte |
| **Mailinhalte** (unterwegs und auf dem Gerät) | Mitlesen oder Verändern im Netz (z. B. gefälschte Server), Lesen der lokalen Dateien bei Diebstahl oder Schadsoftware ohne Zugriff auf den Schlüsselbund |
| **Privatsphäre beim Lesen** | Tracking-Pixel und externe Inhalte, die verraten, wann und wo eine Mail gelesen wurde |
| **Rechner des Nutzers** | Skripte in Mails, gefälschte Links (Phishing), gefährliche Anhänge |

## Wie geschützt wird

### Verbindungen (TLS)

- IMAP und SMTP nur über TLS (Ports 993/465) oder STARTTLS (143/587). Unverschlüsselte Verbindungen gibt es nicht –
  auch nicht als versteckte Option.
- STARTTLS ist Pflicht, wenn gewählt: Bietet der Server es nicht an, wird abgebrochen. Der unverschlüsselte Teil wird
  ungepuffert gelesen; vor TLS eingeschleuste Daten lassen den Handshake scheitern („STARTTLS command injection“).
  Eine IMAP-Begrüssung `PREAUTH` vor TLS führt zum Abbruch.
- rustls mit TLS 1.3 und 1.2, Zertifikate werden immer geprüft (Kette gegen die Mozilla-Wurzelzertifikate und
  Hostname). Es gibt keinen Knopf „Zertifikat trotzdem akzeptieren“.
- Einzige Ausnahme: Der Nutzer hinterlegt bewusst ein eigenes Zertifikat (Pinning) für einen selbstsignierten Server.
  Dann wird ausschliesslich genau dieses Zertifikat akzeptiert; die Handshake-Signatur wird weiterhin geprüft.
  Die Oberfläche warnt deutlich.
- Im SMTP-EHLO steht `[127.0.0.1]` statt des Rechnernamens.
- Autoconfig fragt nur die Domain der eigenen Adresse über HTTPS (mindestens TLS 1.2), keine zentralen Datenbanken.
  Weiterleitungen werden nur innerhalb dieser Domain verfolgt.

### Zugangsdaten

- Passwörter liegen nur im Schlüsselbund des Betriebssystems (Crate `keyring`): macOS-Schlüsselbund,
  Windows-Anmeldeinformationsverwaltung, Secret Service unter Linux.
- `konten.json` enthält keine Passwörter (Integrationstest durchsucht alle Dateien im Datenordner).
- Passwörter werden im Speicher mit `zeroize` gelöscht, erscheinen in keiner `Debug`-Ausgabe, keiner Fehlermeldung
  und keinem Log. SicherMail schreibt keine Logs mit Zugangsdaten.
- Ein Konto wird erst gespeichert, wenn die Anmeldung bei IMAP und SMTP geklappt hat.
- Beim Bearbeiten wird das gespeicherte Passwort nur verwendet, solange Server, Ports, Verschlüsselung, hinterlegtes
  Zertifikat und Benutzername unverändert sind. Sonst muss es neu eingegeben werden – so kann es nicht an einen
  anderen (oder vertippten) Server gehen.
- Beim Löschen eines Kontos wird zuerst der Schlüsselbund-Eintrag entfernt; scheitert das, bleibt das Konto bestehen.

### Lokale Daten (SQLCipher)

- Zwischengespeicherte Mails und Entwürfe liegen in `mails.db`, vollständig verschlüsselt mit SQLCipher (AES-256).
- Der Schlüssel (32 Zufallsbytes) liegt nur im Schlüsselbund; ohne ihn ist die Datei nicht lesbar
  (Test: Öffnen ohne Schlüssel ergibt „file is not a database“, kein Klartext in der Datei).
- `secure_delete` ist eingeschaltet, Dateien im Datenordner sind unter Unix nur für den eigenen Benutzer lesbar.
- „Mails lokal speichern“ lässt sich ausschalten; „Lokalen Speicher leeren“ löscht alle zwischengespeicherten Mails.

### Suche (#49)

- Der **Suchindex** (SQLite FTS5) liegt als Tabelle in derselben verschlüsselten Datei `mails.db` – keine zweite,
  unverschlüsselte Datei, keine fremde Suchbibliothek mit eigenem Speicher. Ohne den Schlüssel aus dem
  Schlüsselbund ist weder ein Suchbegriff noch ein Mailinhalt lesbar (Tests prüfen, dass keine Begriffe und
  keine Indexstrukturen im Klartext in den Dateien stehen).
- Indiziert werden Absender, Betreff und Text. Bei **Ende-zu-Ende verschlüsselten Mails** (OpenPGP, S/MIME, auch
  wenn nur ein Teil verschlüsselt ist) nur Absender und Betreff – entschlüsselter Klartext kommt nie in den Index.
- Die lokale Suche baut keine Verbindung auf und nutzt keine Fremddienste. Eingaben werden nie als FTS5-Syntax
  oder SQL verwendet: Jedes Wort wird als Zeichenkette in Anführungszeichen gesetzt, alle Werte gehen als
  Parameter an die Datenbank.
- **Auf dem Server suchen** schickt den Suchbegriff – nur nach diesem Klick – an den eigenen IMAP-Server
  (`UID SEARCH`), über die bestehende verschlüsselte Verbindung des Kontos. Wörter mit Sonderzeichen oder Umlauten
  gehen als IMAP-Literal mit genau angekündigter Länge, Steuerzeichen (CR, LF) werden vorher entfernt; so lässt sich
  kein weiterer Befehl einschleusen. Gefundene, noch nicht gespeicherte Mails werden nur angezeigt.
- Beim Entfernen eines Kontos werden seine Einträge sofort aus allen Teilen des Index entfernt; „Lokalen Speicher
  leeren“ legt den Index neu an.

### Gemeinsamer Posteingang (#50)

- **Keine neuen Verbindungen**: „Alle Posteingänge“ liest nur aus dem lokalen Speicher. Neue Mails bringen die
  bestehenden Beobachter (IDLE) der Konten; „Ältere Mails laden“ und Aktionen laufen über die bestehende
  Befehlsverbindung des jeweiligen Kontos zu dessen eigenem Server. Ein Integrationstest prüft, dass die Ansicht
  auch ohne Netz zu einem Konto vollständig aus dem Speicher kommt.
- **Antwort immer über das Konto der Mail**: Antworten und Weiterleiten setzen das Konto der ursprünglichen Mail –
  Absenderadresse, eigener OpenPGP-Schlüssel bzw. eigenes S/MIME-Zertifikat und SMTP-Server stammen von diesem
  Konto, auch wenn ein anderes Konto das Standardkonto ist. Das Konto steht sichtbar unter „Von“. Der Versand wählt
  den SMTP-Server immer anhand des Kontos im Entwurf (Integrationstest mit zwei Konten).
- Aktionen (gelesen, verschieben, löschen) gehen immer an das Konto und den Ordner der Mail, nie an ein anderes.
- Die Kontofarbe ist nie die einzige Kennzeichnung: Kürzel und Kontoname stehen als Text dabei.

### Anzeige von Mails

- HTML wird im Backend mit `ammonia` bereinigt (Positivliste): keine Skripte, Formulare, iframes, Objekte, SVG,
  Event-Attribute, `javascript:`-Links, `<style>`-Blöcke oder CSS mit `url()`/`expression()`/`@import`.
- Zusätzlich Anzeige in einem `iframe` mit `sandbox` ohne `allow-scripts` (auch ohne Formulare, Popups und
  Navigation des Hauptfensters) und eigener Content-Security-Policy, die nur eingebettete `data:`-Bilder erlaubt.
- Externe Inhalte sind standardmässig blockiert. Auf Wunsch (pro Mail oder dauerhaft pro Absender) lädt das Backend
  Bilder – nur über HTTPS, nur Bildformate, mit Grössengrenzen, ohne Cookies und Referer – und setzt sie als
  `data:`-Adressen ein. Die Anzeige selbst baut nie Verbindungen auf. Adressen im eigenen Netz (localhost, private
  und Link-Local-Adressen, Namen ohne Punkt) werden nie geladen – auch nicht über Weiterleitungen, DNS oder einen
  Proxy. Weil ein Proxy Namen selbst auflöst und diese Prüfung so umgehen würde, verbindet sich SicherMail für externe
  Bilder standardmässig immer direkt und benutzt nie einen im System eingestellten Proxy (`HTTPS_PROXY` usw.).
- **Proxy für externe Bilder** (Einstellung, standardmässig aus): Wer nur über einen Proxy ins Internet kommt, kann eine
  eigene Proxy-Adresse eintragen (nur `http://` oder `https://` mit Server und Port, ohne Zugangsdaten; ungültige
  Adressen werden abgelehnt, dann bleiben Bilder blockiert). Dann geht jede Bildanfrage – auch Weiterleitungen – nur
  über diese Adresse, nie über System- oder Umgebungs-Proxys. Weiterhin gilt: nur HTTPS, gleiche Grössen-, Anzahl- und
  Zeitgrenzen; IP-Adressen aus dem eigenen Netz (127/8, 10/8, 172.16/12, 192.168/16, 169.254/16, ::1, fc00::/7,
  fe80::/10 …), `localhost` und Namen ohne Punkt werden abgelehnt, bevor sie den Proxy erreichen. **Grenze:** Normale
  Namen löst der Proxy selbst auf. Zeigt ein solcher Name auf eine Adresse im eigenen Netz, kann SicherMail das nicht
  mehr erkennen. Der Einstellungsdialog sagt das deutlich.
- Die Freigabe „für diesen Absender immer erlauben“ hängt an der Absenderadresse, die gefälscht sein kann; die
  Oberfläche weist darauf hin. Erkennt die Absenderprüfung einen wahrscheinlich gefälschten Absender, gilt die Freigabe
  nicht und wird nicht angeboten.
- **Absenderprüfung** (DKIM/SPF/DMARC): SicherMail wertet nur den `Authentication-Results`-Header (RFC 8601) des
  eigenen Servers aus – keine DNS-Abfragen, keine Verbindungen. Vertraut wird ausschliesslich dem **obersten** Header
  mit der Server-Kennung (authserv-id) des Kontos (Standard: Name des IMAP-Servers). Header mit fremder Kennung und
  weiter unten stehende Header mit derselben Kennung (z. B. vom Absender eingeschleust) werden ignoriert. Kommentare,
  Gross-/Kleinschreibung und gefaltete Zeilen werden beim Lesen berücksichtigt. Drei Stufen: bestätigt, nicht geprüft,
  Warnung (Symbol und Text, nicht nur Farbe). **Grenze:** Das wirkt nur, wenn der eigene Server tatsächlich prüft und
  von aussen kommende `Authentication-Results`-Header mit seiner Kennung entfernt oder seinen eigenen Header darüber
  setzt (RFC 8601, Abschnitt 5). Prüft er nicht, steht „nicht geprüft“. Schreibt der Server mehrere Header (z. B.
  Postfix mit OpenDKIM und OpenDMARC getrennt), zählt nur der oberste. Die Ausrichtung von DKIM-/SPF-Domain zur
  Absenderdomain ist vereinfacht (gleiche Domain oder Subdomain, ohne Public Suffix List).
- Links öffnen sich erst nach einem Klick im Standardbrowser (nur `https:`, `http:`, `mailto:`). Weicht der angezeigte
  Text von der echten Adresse ab (oder enthält die Adresse Punycode), warnt SicherMail. Geprüft wird der sichtbare,
  vereinheitlichte Text ohne unsichtbare Zeichen; jede Adresse im Linktext zählt, nicht nur ein reiner Adresstext.
- Bei `mailto:`-Links gibt SicherMail nur `subject`, `body`, `cc` und `bcc` an das Mailprogramm weiter, nie z. B.
  `attach=` (Anhängen lokaler Dateien).
- Anhänge werden nie automatisch geöffnet, nur über „Speichern unter“ abgelegt. Der Dateiname wird gesäubert: keine
  Pfade, keine Steuer-, Schreibrichtungs- oder unsichtbaren Zeichen (`Rechnung<U+202E>fdp.exe`), keine reservierten
  Windows-Namen.

### Ende-zu-Ende-Verschlüsselung (OpenPGP)

- Bibliothek: rPGP (`pgp`), reines Rust ohne Systembibliotheken (Begründung in der [Architektur](architektur.md)).
- **Private Schlüssel** liegen nie im Klartext auf der Platte: in `schluessel.db`, vollständig verschlüsselt mit
  SQLCipher; deren Schlüssel (32 Zufallsbytes) liegt nur im Schlüsselbund (`pgp:schluessel`). Zusätzlich ist jeder
  private Schlüssel mit der Passphrase des Nutzers geschützt (S2K). Die Datei ist unabhängig von „Mails lokal speichern“;
  „Lokalen Speicher leeren“ und das Ausschalten der lokalen Speicherung lassen sie unberührt. Fehlt der Schlüssel im
  Schlüsselbund, wird die Datei nicht gelöscht, sondern beiseitegelegt (`schluessel.db.unlesbar-<Zeit>`).
- **Passphrasen** werden nie gespeichert. Nach dem Entsperren bleiben sie höchstens bis zum Programmende im
  Arbeitsspeicher (`zeroize`); neue Schlüssel brauchen mindestens 8 Zeichen. Geheimes (Passphrasen, private Schlüssel,
  entschlüsselter Text) wird im Speicher mit `zeroize` gelöscht; rPGP löscht Schlüsselmaterial selbst.
- **Neue Schlüssel**: Ed25519 (Signieren) und Cv25519 (Verschlüsseln) im v4-Format wie GnuPG.
- **Import**: Eigene private Schlüssel nur mit richtiger Passphrase und nur, wenn eine User-ID zur Adresse des Kontos
  passt; Eigensignaturen werden geprüft. Schlüsselserver und WKD werden nicht abgefragt – Schlüssel kommen nur aus
  Dateien oder Mail-Anhängen. Das Frontend sieht nie einen privaten Schlüssel; Dateien wählt und speichert das Backend.
- **Entschlüsseln**: nur integritätsgeschützte Formate (SEIPD mit MDC, SEIPDv2/AEAD, GnuPG-AEAD/OCB); ungeschützte
  Altformate (SED) werden abgelehnt. Entschlüsselt wird nur, wenn die **ganze** Mail verschlüsselt ist (oberster
  MIME-Teil `multipart/encrypted` bzw. der ganze Text bei Inline-PGP). Ein verschlüsselter Teil zwischen
  unverschlüsselten Teilen wird nur als „teilweise verschlüsselt“ gekennzeichnet und nicht entschlüsselt (Schutz vor
  EFAIL-ähnlichen Tricks). Höchstens 64 MB Klartext (Schutz vor Kompressionsbomben).
- Der **Klartext** wird nie gespeichert: Im lokalen Speicher liegt nur die verschlüsselte Fassung. Entschlüsselter
  Inhalt läuft durch dieselbe sichere Anzeige (ammonia, abgeschottetes iframe, externe Inhalte blockiert).
- **Signaturen** gelten nur als gültig, wenn eine User-ID des Schlüssels zur Absenderadresse passt; sonst steht ein
  deutlicher Hinweis. Unbekannte Schlüssel, ungültige (veränderte) Signaturen und nur teilweise signierte Mails
  werden ausdrücklich so gekennzeichnet. Bei PGP/MIME wird über die kanonische Form (CRLF) geprüft.
- **Verschlüsselt senden** (PGP/MIME, RFC 3156): Ist „Verschlüsseln“ gewählt, braucht jeder Empfänger in An, Cc und
  Bcc einen importierten öffentlichen Schlüssel, sonst wird **nicht** gesendet und der fehlende Empfänger genannt –
  SicherMail sendet nie unbemerkt unverschlüsselt. Das prüft die Oberfläche vorab und das Backend beim Senden noch
  einmal. Text und Anhänge werden gemeinsam verschlüsselt (SEIPD mit MDC, AES-256), immer auch an den eigenen
  Schlüssel, damit „Gesendet“ lesbar bleibt. Signiert wird innerhalb der Verschlüsselung (SHA-256).
- **Bcc**: Jeder Bcc-Empfänger bekommt eine eigene Nachricht, nur an seinen Schlüssel verschlüsselt. In der Nachricht
  für An/Cc stehen so keine Schlüssel-IDs der Bcc-Empfänger, und kein Bcc-Empfänger erfährt von anderen. Gewählt statt
  „versteckter Schlüssel-IDs“ in einer gemeinsamen Nachricht, weil dort jeder Empfänger alle Schlüssel durchprobieren
  müsste und manche Programme solche Nachrichten schlecht lesen.
- **Nur signiert**: `multipart/signed` mit abgetrennter Signatur über die kanonische Form des Inhalts.
- **Entwürfe** liegen nie auf dem Server, sondern nur auf diesem Gerät: in `mails.db` (SQLCipher), bzw. nur im
  Arbeitsspeicher, wenn „Mails lokal speichern“ aus ist. Das gilt auch für Entwürfe verschlüsselter Mails.
- **Passphrase zum Signieren** wird beim Senden abgefragt und – wie beim Lesen – nur bis zum Programmende im
  Arbeitsspeicher gehalten.

### Ende-zu-Ende-Verschlüsselung (S/MIME)

- Bibliothek: OpenSSL (Crate `openssl`), dieselbe mitkompilierte Fassung wie für SQLCipher, keine Systembibliothek
  zur Laufzeit (Begründung in der [Architektur](architektur.md#smime-smime)).
- **Eigene Zertifikate** kommen aus PKCS#12-Dateien (`.p12`/`.pfx`) und werden nur angenommen, wenn eine Adresse im
  Zertifikat (SAN `rfc822Name`, ersatzweise Subject `E=`) zur Kontoadresse passt, der Schlüssel zum Zertifikat gehört,
  der Zweck passt (KeyUsage, ExtendedKeyUsage „E-Mail-Schutz“), es nicht abgelaufen ist und der Schlüssel stark genug
  ist (RSA ab 2048 Bit). Der private Schlüssel liegt in `schluessel.db` (SQLCipher) und ist zusätzlich mit der
  Passphrase geschützt (scrypt + AES-256-GCM); die Passphrase ist das Passwort der Datei. Passphrasen werden nie
  gespeichert und nach dem Entsperren höchstens bis zum Programmende im Arbeitsspeicher gehalten (`zeroize`). Das
  Frontend sieht nie einen privaten Schlüssel; Dateien wählt das Backend.
- **Zertifikate anderer** kommen nur aus Dateien (`.cer`, `.crt`, `.pem`, `.p7c`) oder auf Klick „Zertifikat
  übernehmen“ aus einer gültig und ganz signierten Mail. Es gibt keine Verzeichnisdienste (LDAP) und keine
  Online-Abfragen.
- **Vertrauen**: Eine Kette muss zu einer Wurzel im Zertifikatsspeicher des Betriebssystems oder zu einer vom Nutzer
  übernommenen Wurzel führen. Eine Wurzel wird erst übernommen, nachdem Name und SHA-256-Fingerabdruck angezeigt und
  bestätigt wurden; sie lässt sich wieder entfernen. Mitgelieferte Zwischenzertifikate helfen nur beim Aufbau der
  Kette. **Sperrlisten (CRL) und OCSP werden nicht abgefragt**; bei jeder geprüften Signatur steht dezent
  „Sperrstatus nicht online geprüft“. Ein gestohlenes, aber gesperrtes Zertifikat erkennt SicherMail also nicht.
- **Entschlüsseln** nach denselben Regeln wie bei OpenPGP: nur wenn die **ganze** Mail verschlüsselt ist (EFAIL),
  verschlüsselte Teile neben unverschlüsselten nur als „teilweise verschlüsselt“ gekennzeichnet, höchstens 64 MB,
  Klartext nie gespeichert, Anzeige durch dieselbe sichere Darstellung. Nur AES-GCM (AuthEnvelopedData) und AES-CBC;
  RC2, DES und Triple-DES werden abgelehnt. Bei AES-CBC ohne gültige innere Signatur steht der Hinweis „nicht gegen
  Veränderung geschützt“, weil CBC allein Veränderungen nicht erkennt.
- **Signaturen** gelten nur als gültig, wenn die Kette vertrauenswürdig ist, das Zertifikat zum Zeitpunkt der Signatur
  gültig war, der Zweck passt und eine Adresse im Zertifikat zum Absender passt. Sonst steht deutlich da: unbekannte
  Stelle, abgelaufen, Adresse passt nicht, ungültig (verändert) oder nur teilweise signiert. MD5 und RSA unter
  2048 Bit werden abgelehnt, SHA-1 als „schwach“ gekennzeichnet. Der Zeitpunkt stammt aus der Signatur (`signingTime`)
  und ist nicht später als jetzt; ist das Zertifikat inzwischen abgelaufen, steht ein Hinweis dabei.
- **Verschlüsselt senden** (RFC 8551): Jeder Empfänger in An, Cc und Bcc braucht ein jetzt gültiges Zertifikat mit
  passender Adresse, Zweck „Verschlüsseln“, ausreichend starkem Schlüssel (RSA ab 2048 Bit oder EC) und
  vertrauenswürdiger Kette. Fehlt eines, wird **nicht** gesendet und der Empfänger genannt – das prüft die Oberfläche
  vorab und das Backend beim Senden noch einmal. Text und Anhänge werden gemeinsam als EnvelopedData mit AES-256-CBC
  verschlüsselt (breite Verträglichkeit), immer auch an das eigene Zertifikat, damit „Gesendet“ lesbar bleibt.
  Signiert wird vorher, innerhalb der Verschlüsselung (SHA-256), damit Empfänger Veränderungen erkennen.
- **Bcc** wie bei OpenPGP: Jeder Bcc-Empfänger bekommt eine eigene Nachricht, nur an sein Zertifikat verschlüsselt;
  die Nachricht für An/Cc nennt keine Bcc-Empfänger, auch nicht in den Empfängerangaben der Verschlüsselung.
- **Nur signiert**: `multipart/signed` mit abgetrennter Signatur (`application/pkcs7-signature`, SHA-256), mit eigenem
  Zertifikat und Zwischenzertifikaten. Die Passphrase wird beim Senden abgefragt und nur bis Programmende im
  Arbeitsspeicher gehalten. Entwürfe verschlüsselter Mails bleiben nur auf dem Gerät.

### Tauri und Oberfläche

- Strenge CSP im Hauptfenster: nur eigene Skripte, keine Remote-URLs, keine Formulare, keine fremden Frames.
- Capabilities: Das Frontend darf nur Ereignisse empfangen. Kein Shell-, Dateisystem-, HTTP- oder Dialogzugriff –
  Dialoge („Speichern unter“, „Datei anhängen“) und das Öffnen von Links laufen im Backend.
- Das Frontend kennt keine Dateipfade: Anhänge werden über Tokens angesprochen, die nur der Backend-Dialog vergibt.
  So kann auch eingeschleuster Code in der Oberfläche keine beliebigen Dateien verschicken.
- Texte werden in der Oberfläche nie als HTML gesetzt (Tests prüfen das).

### Keine Telemetrie

SicherMail baut nur Verbindungen auf zu den vom Nutzer eingetragenen Servern, zur Domain der eigenen Adresse
(Autoconfig, nur auf Klick) und zu externen Bildquellen (nur auf ausdrücklichen Wunsch, ggf. über den selbst
eingetragenen Proxy). Es gibt keine Telemetrie,
keine Absturzberichte und keine Update-Abfragen.

### Mails auf dem Server

SicherMail löscht nie von sich aus Mails auf dem Server: kein `CLOSE`, kein allgemeines `EXPUNGE`. Endgültig gelöscht
wird nur auf Wunsch des Nutzers und nur die eine Mail (`UID EXPUNGE`).

### Entwicklung und Lieferkette

- CI: Tests (Frontend, Rust, Integrationstests gegen GreenMail), ESLint, `cargo clippy -D warnings`, `cargo fmt`.
- `cargo audit` und `npm audit --audit-level=high` bei jedem Push, jedem Pull Request und wöchentlich;
  sie brechen bei bekannten Lücken ab.
- Dependabot schlägt wöchentlich Updates für Cargo, npm und GitHub Actions vor.
- Lockfiles (`Cargo.lock`, `package-lock.json`) sind eingecheckt.
- **Installationspakete** (#48) baut ausschliesslich die CI (`.github/workflows/pakete.yml`), aus genau dem Commit
  des Releases und erst nach den Prüfungen „Commit liegt auf main“ und „Version stimmt“. Gebaut wird mit gesperrten
  Lockfiles (`npm ci`, `cargo --locked`), der Rust-Version aus `rust-version` in `Cargo.toml`, ohne Build-Cache und
  mit `SOURCE_DATE_EPOCH` aus dem Commit-Zeitpunkt; fremde Actions sind auf Commits festgelegt. Bit-für-Bit
  reproduzierbar sind die Pakete damit noch nicht (u. a. AppImage-Werkzeuge und Zeitstempel der Paketformate).
- Jedes Release enthält `SHA256SUMS.txt` und einen Herkunftsnachweis (Build Provenance, `actions/attest-build-provenance`)
  für jedes Paket, prüfbar mit `gh attestation verify <paket> --repo Louidos/Mail`. Release, Tag und Pakete entstehen in
  einem Workflow-Lauf; scheitert ein Paket, gibt es kein Release.
- **Code-Signierung** für macOS (mit Notarisierung) und Windows nur, wenn die Zertifikate als Secrets hinterlegt sind;
  sonst sind die Pakete unsigniert (macOS nur ad hoc signiert), und der Release-Text sagt das. Linux-Pakete sind
  nicht signiert.
- Die Pakete enthalten keine Update-Funktion (kein `tauri-plugin-updater`, keine Update-Adresse); ein Test prüft das.
  Das Windows-Installationsprogramm lädt die WebView2-Laufzeit von Microsoft nach, falls sie fehlt – das ist die
  einzige Verbindung bei der Installation; SicherMail selbst verbindet sich nicht mit Microsoft.

## Bekannte Hinweise aus `cargo audit`

**Bewusste Ausnahme** (in `src-tauri/.cargo/audit.toml`): RUSTSEC-2023-0071 im Crate `rsa` („Marvin Attack“). Die
RSA-Entschlüsselung läuft nicht in konstanter Zeit; ausnutzbar ist das nur, wenn ein Angreifer die Rechenzeit vieler
Entschlüsselungen messen kann, etwa über das Netz. `rsa` kommt über rPGP, das RSA-Schlüssel anderer lesen können muss.
SicherMail entschlüsselt nur lokal auf Wunsch des Nutzers und gibt keine zeitabhängige Rückmeldung nach aussen; neu
erzeugte Schlüssel sind Ed25519/Cv25519. Das Advisory selbst nennt lokale Nutzung als unbedenklich. Eine behobene
Version gibt es noch nicht; die Ausnahme wird entfernt, sobald es sie gibt.

Sonst meldet `cargo audit` derzeit keine Sicherheitslücken, aber Warnungen zu indirekten Abhängigkeiten des
Tauri-/GTK-Unterbaus unter Linux (z. B. `glib` „unsound“ in `VariantStrIter`, nicht mehr gepflegte `zvariant`,
`zbus_names`, `proc-macro-error`). SicherMail verwendet die betroffenen Funktionen nicht direkt. Die Warnungen
verschwinden mit Updates von Tauri bzw. seinen Abhängigkeiten (Dependabot).

## Was bewusst nicht abgedeckt ist

- **Mit OpenPGP bzw. S/MIME geschützt ist nur der Inhalt**: Text und Anhänge verschlüsselter Mails liegen auch auf dem Server
  verschlüsselt. **Metadaten** wie Betreff, Absender, Empfänger und Datum stehen weiterhin im Klartext im Kopf der Mail.
  Unverschlüsselte Mails liegen auf dem Server so, wie der Server sie speichert.
- **S/MIME ohne Sperrprüfung**: CRL und OCSP werden bewusst nicht abgefragt (keine Verbindungen zu fremden Diensten);
  gesperrte Zertifikate erkennt SicherMail nicht, weder beim Lesen noch beim Verschlüsseln an andere.
- **OpenPGP-Schlüsselverwaltung ohne Vertrauensnetz**: Ablauf und Widerruf von Schlüsseln werden nicht ausgewertet,
  Beglaubigungen (Web of Trust) nicht geprüft. Ob ein Schlüssel wirklich der Person gehört, muss der Nutzer über den
  Fingerabdruck selbst prüfen.
- **Keine eigene Absenderprüfung**: DKIM, SPF und DMARC prüft SicherMail nicht selbst (keine DNS-Abfragen), sondern
  zeigt nur das Ergebnis des eigenen Servers. Ohne dieses Ergebnis bleibt ein Absender „nicht geprüft“ und kann
  gefälscht sein.
- **Schutz nur so gut wie das Gerät**: Wer den entsperrten Rechner bzw. den entsperrten Schlüsselbund kontrolliert,
  kann Passwörter und lokale Mails lesen. Der Arbeitsspeicher des laufenden Programms ist nicht gesondert geschützt.
- **Metadaten**: Server und Netzbetreiber sehen, wann und mit welchem Server verbunden wird.
- **Server-Suche**: Wer „Auf dem Server suchen“ nutzt, gibt dem eigenen Mailserver den Suchbegriff preis. Die
  lokale Suche bleibt auf dem Gerät.
- **Anhänge** werden nicht auf Schadsoftware geprüft.
- **Nur IMAP/SMTP**: kein POP3, kein Exchange/EWS.
- **Externe Bilder über einen Proxy**: Standardmässig nie. Mit der Einstellung „Proxy für externe Bilder verwenden“
  löst der Proxy Namen selbst auf; dass ein Name auf eine Adresse im eigenen Netz zeigt, lässt sich dann nicht mehr
  ausschliessen. SOCKS-Proxys und Proxys mit Anmeldung werden nicht unterstützt.
- Server, die nur veraltete TLS-Versionen (unter 1.2) können, werden nicht unterstützt.
- **Hinterlegte Zertifikate** (Pinning) werden genau so akzeptiert, wie sie hinterlegt wurden; Ablaufdatum und
  Hostname des hinterlegten Zertifikats werden nicht zusätzlich geprüft. Läuft es ab, sollte es ersetzt werden.
