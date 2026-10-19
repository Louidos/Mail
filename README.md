# SicherMail

SicherMail ist ein sicherer Open-Source-Mailclient für den Desktop (Windows, macOS, Linux).
Du gibst deinen eigenen IMAP- und SMTP-Server an; SicherMail verbindet sich nur verschlüsselt
und nur mit den Servern, die du selbst eingetragen hast.

Grundsätze:

- Verbindungen nur über TLS oder STARTTLS, Zertifikate werden immer geprüft.
- Passwörter liegen ausschliesslich im Schlüsselbund des Betriebssystems.
- Lokal gespeicherte Mails sind verschlüsselt (SQLCipher).
- Keine Telemetrie, keine Fremddienste.

## Projektstand

Version: **0.2.1** (siehe [Releases](https://github.com/Louidos/Mail/releases))

- [x] Grundgerüst (Tauri 2, leeres Fenster)
- [x] Konto mit eigenem Mailserver einrichten
- [x] Mails empfangen (IMAP, IDLE)
- [x] Mails sicher anzeigen
- [x] Verschlüsselter lokaler Speicher
- [x] Mails senden (SMTP)
- [x] Sicheres Design in Blau/Weiss
- [x] Sicherheitsprüfungen in CI (`cargo audit`, `npm audit`, Dependabot)
- [x] Optionaler Proxy für externe Bilder
- [x] Absender prüfen (DKIM/SPF/DMARC des eigenen Servers anzeigen, vor Fälschungen warnen)

Änderungen stehen im [CHANGELOG](CHANGELOG.md).

## Was noch zu tun ist

Wer einen Punkt erledigt, hakt ihn im selben PR hier ab bzw. entfernt ihn.

**Offen (bekannt):**

- Zurzeit sind keine Fehler- oder Sicherheits-Issues offen. Alle offenen
  [Issues](https://github.com/Louidos/Mail/issues) sind unten unter „Geplant“ verlinkt.

**Geplant:**

- [ ] Ende-zu-Ende-Verschlüsselung mit OpenPGP, später S/MIME ([#44](https://github.com/Louidos/Mail/issues/44))
- [ ] Dunkle Darstellung wieder anbieten, entfiel mit #31 ([#47](https://github.com/Louidos/Mail/issues/47))
- [ ] Fertige Installationspakete für Windows, macOS und Linux in jedem Release ([#48](https://github.com/Louidos/Mail/issues/48))
- [ ] Suche in Mails ([#49](https://github.com/Louidos/Mail/issues/49))
- [ ] Gemeinsamer Posteingang für mehrere Konten ([#50](https://github.com/Louidos/Mail/issues/50))

**Bewusst nicht geplant** (siehe [Sicherheitskonzept](docs/sicherheit.md#was-bewusst-nicht-abgedeckt-ist)):

- POP3 und Exchange/EWS – SicherMail spricht nur IMAP und SMTP.
- Telemetrie, Absturzberichte und automatische Update-Abfragen.
- Fremddienste (zentrale Konfigurationsdatenbanken, Cloud-Dienste); verbunden wird nur mit den eigenen Servern.

## Installation für Entwicklung

Voraussetzungen:

- [Rust](https://www.rust-lang.org/tools/install) (stabil, mindestens 1.90 – `rust-version` in
  `src-tauri/Cargo.toml`)
- [Node.js](https://nodejs.org/) 22 mit npm
- Systembibliotheken für Tauri 2, siehe <https://v2.tauri.app/start/prerequisites/>.
  Unter Debian/Ubuntu:

  ```sh
  sudo apt install libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf libssl-dev libdbus-1-dev
  ```

Abhängigkeiten installieren:

```sh
npm ci
```

## Start

```sh
npm run tauri dev      # Programm im Entwicklungsmodus starten
npm run tauri build    # Installationspaket bauen
```

## Tests und Prüfungen

```sh
npm test               # Frontend-Tests (Vitest)
npm run lint           # ESLint und TypeScript-Prüfung
npm run build          # Frontend bauen
cd src-tauri
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Integrationstests gegen den lokalen Test-Mailserver (GreenMail, braucht Docker):

```sh
scripts/test-mailserver.sh start
cd src-tauri && cargo test -- --include-ignored
cd .. && scripts/test-mailserver.sh stop
```

Getestet wird nie gegen echte Postfächer.

## Aufbau

| Ordner                | Inhalt                                                                                         |
| --------------------- | ---------------------------------------------------------------------------------------------- |
| `src/`                | Frontend (TypeScript, Vite, ohne Framework): `konto/` (Konto einrichten), `postfach/` (Mailliste und Leseansicht), `verfassen/` (Schreiben, Antworten, Entwürfe), `einstellungen/` |
| `src-tauri/`          | Rust-Backend (Tauri 2); Module in `src-tauri/src/`: `konto`, `imap`, `smtp`, `speicher`, `anzeige`, dazu `tls` und `befehle` |
| `public/`             | statische Dateien (Icons)                                                                      |
| `tests/`              | Frontend-Tests (Vitest)                                                                        |
| `tests/integration/`  | Rust-Integrationstests, u. a. gegen den Test-Mailserver (je Datei ein `[[test]]` in `src-tauri/Cargo.toml`) |
| `docs/`               | Doku: [Bedienung](docs/bedienung.md), [Architektur](docs/architektur.md), [Sicherheit](docs/sicherheit.md) |
| `scripts/`            | Hilfsskripte ([`test-mailserver.sh`](scripts/test-mailserver.sh) für GreenMail)                 |
| `.github/`            | CI- und Release-Workflow, Vorlagen für Issues und Pull Requests                                |

Mehr zum Aufbau des Backends steht in der [Architektur](docs/architektur.md).

## Mitmachen und Arbeitsweise

- **Ein Branch pro Thema**, immer vom aktuellen `main` (z. B. `feature/…`, `fix/…`, `docs/…`). Nie direkt auf
  `main` arbeiten; `main` ist immer lauffähig.
- **Commits** nach [Conventional Commits](https://www.conventionalcommits.org/de/) mit kurzem deutschem Text:
  `feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `chore:`, `security:`.
- **CHANGELOG**: Spürbare Änderungen kommen in [CHANGELOG.md](CHANGELOG.md) unter `## Unveröffentlicht`
  (Hinzugefügt / Geändert / Behoben / Entfernt), mit Issue-Nummer und für Nutzer geschrieben.
- **Tests**: Für neue Funktionen und behobene Fehler Tests ergänzen. Alle Prüfungen aus
  [Tests und Prüfungen](#tests-und-prüfungen) müssen grün sein; getestet wird nie gegen echte Postfächer.
- **Pull Request** gegen `main` mit der [PR-Vorlage](.github/pull_request_template.md) und `Closes #<n>`.
  Gemergt wird erst bei grüner CI, per Squash-Merge.
- **Aufgabenliste**: Wer einen Punkt aus [Was noch zu tun ist](#was-noch-zu-tun-ist) erledigt, hakt ihn im
  selben PR ab bzw. entfernt ihn.
- **Versionen** nach [SemVer](https://semver.org/lang/de/): PATCH für Fehlerbehebungen, MINOR für neue
  Funktionen, MAJOR für Inkompatibles. Release: Branch `release/X.Y.Z`, Version in `package.json`,
  `src-tauri/Cargo.toml` (+ `Cargo.lock`), `src-tauri/tauri.conf.json`, CHANGELOG und README setzen, Commit
  `chore: Version X.Y.Z`, PR und Squash-Merge; danach Tag `vX.Y.Z` auf `main`. Der
  [Release-Workflow](.github/workflows/release.yml) erstellt dann das GitHub-Release mit dem CHANGELOG-Abschnitt.
  Tags werden nie verschoben oder gelöscht.

## Sicherheit

Wie SicherMail Zugangsdaten, Inhalte und Privatsphäre schützt – und was bewusst nicht abgedeckt ist –, steht im
[Sicherheitskonzept](docs/sicherheit.md). Abhängigkeiten werden in der CI mit `cargo audit` und `npm audit`
auf bekannte Lücken geprüft.

Sicherheitslücken bitte nicht öffentlich melden, sondern wie in [SECURITY.md](SECURITY.md) beschrieben.

## Lizenz

SicherMail steht unter der [GNU General Public License v3.0 oder später](LICENSE) (GPL-3.0-or-later).
