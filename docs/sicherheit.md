# Sicherheitskonzept

Dieses Dokument beschreibt, was SicherMail schützt, wie es das tut und was bewusst (noch) nicht abgedeckt ist.
Sicherheitslücken bitte wie in [SECURITY.md](../SECURITY.md) beschrieben melden.

## Was geschützt wird

| Schutzgut | Bedrohung |
| --- | --- |
| **Zugangsdaten** (Passwörter, Schlüssel) | Mitlesen im Netz, Auslesen aus Dateien oder Logs, Weitergabe an Dritte |
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
  Oberfläche weist darauf hin.
- Links öffnen sich erst nach einem Klick im Standardbrowser (nur `https:`, `http:`, `mailto:`). Weicht der angezeigte
  Text von der echten Adresse ab (oder enthält die Adresse Punycode), warnt SicherMail. Geprüft wird der sichtbare,
  vereinheitlichte Text ohne unsichtbare Zeichen; jede Adresse im Linktext zählt, nicht nur ein reiner Adresstext.
- Bei `mailto:`-Links gibt SicherMail nur `subject`, `body`, `cc` und `bcc` an das Mailprogramm weiter, nie z. B.
  `attach=` (Anhängen lokaler Dateien).
- Anhänge werden nie automatisch geöffnet, nur über „Speichern unter“ abgelegt. Der Dateiname wird gesäubert: keine
  Pfade, keine Steuer-, Schreibrichtungs- oder unsichtbaren Zeichen (`Rechnung<U+202E>fdp.exe`), keine reservierten
  Windows-Namen.

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

## Bekannte Hinweise aus `cargo audit`

`cargo audit` meldet derzeit keine Sicherheitslücken, aber Warnungen zu indirekten Abhängigkeiten des
Tauri-/GTK-Unterbaus unter Linux (z. B. `glib` „unsound“ in `VariantStrIter`, nicht mehr gepflegte `zvariant`,
`zbus_names`, `proc-macro-error`). SicherMail verwendet die betroffenen Funktionen nicht direkt. Die Warnungen
verschwinden mit Updates von Tauri bzw. seinen Abhängigkeiten (Dependabot).

## Was bewusst nicht abgedeckt ist

- **Keine Ende-zu-Ende-Verschlüsselung** (OpenPGP, S/MIME): Mails liegen auf dem Server so, wie der Server sie
  speichert. Kommt später in einem eigenen Auftrag.
- **Keine Signaturprüfung** von Absendern (DKIM/SPF/DMARC werden nicht angezeigt); Absenderadressen können gefälscht sein.
- **Schutz nur so gut wie das Gerät**: Wer den entsperrten Rechner bzw. den entsperrten Schlüsselbund kontrolliert,
  kann Passwörter und lokale Mails lesen. Der Arbeitsspeicher des laufenden Programms ist nicht gesondert geschützt.
- **Metadaten**: Server und Netzbetreiber sehen, wann und mit welchem Server verbunden wird.
- **Anhänge** werden nicht auf Schadsoftware geprüft.
- **Nur IMAP/SMTP**: kein POP3, kein Exchange/EWS.
- **Externe Bilder über einen Proxy**: Standardmässig nie. Mit der Einstellung „Proxy für externe Bilder verwenden“
  löst der Proxy Namen selbst auf; dass ein Name auf eine Adresse im eigenen Netz zeigt, lässt sich dann nicht mehr
  ausschliessen. SOCKS-Proxys und Proxys mit Anmeldung werden nicht unterstützt.
- Server, die nur veraltete TLS-Versionen (unter 1.2) können, werden nicht unterstützt.
- **Hinterlegte Zertifikate** (Pinning) werden genau so akzeptiert, wie sie hinterlegt wurden; Ablaufdatum und
  Hostname des hinterlegten Zertifikats werden nicht zusätzlich geprüft. Läuft es ab, sollte es ersetzt werden.
