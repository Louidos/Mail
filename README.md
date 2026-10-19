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

Version: **0.1.1** (siehe [Releases](https://github.com/Louidos/Mail/releases))

- [x] Grundgerüst (Tauri 2, leeres Fenster)
- [x] Konto mit eigenem Mailserver einrichten
- [x] Mails empfangen (IMAP, IDLE)
- [x] Mails sicher anzeigen
- [x] Verschlüsselter lokaler Speicher
- [x] Mails senden (SMTP)

Änderungen stehen im [CHANGELOG](CHANGELOG.md).

## Installation für Entwicklung

Voraussetzungen:

- [Rust](https://www.rust-lang.org/tools/install) (stabil, mindestens 1.90)
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
cd src-tauri
cargo fmt --check
cargo clippy -- -D warnings
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

| Ordner       | Inhalt                                                           |
| ------------ | ---------------------------------------------------------------- |
| `src/`       | Frontend (TypeScript, Vite)                                      |
| `src-tauri/` | Rust-Backend (Tauri 2)                                           |
| `public/`    | statische Dateien (Icons)                                        |
| `tests/`     | Frontend-Tests und Integrationstests gegen einen Test-Mailserver |
| `docs/`      | Doku: [Bedienung](docs/bedienung.md), [Architektur](docs/architektur.md), [Sicherheit](docs/sicherheit.md) |
| `scripts/`   | Hilfsskripte                                                     |

## Sicherheit

Wie SicherMail Zugangsdaten, Inhalte und Privatsphäre schützt – und was bewusst nicht abgedeckt ist –, steht im
[Sicherheitskonzept](docs/sicherheit.md). Abhängigkeiten werden in der CI mit `cargo audit` und `npm audit`
auf bekannte Lücken geprüft.

Sicherheitslücken bitte nicht öffentlich melden, sondern wie in [SECURITY.md](SECURITY.md) beschrieben.

## Lizenz

SicherMail steht unter der [GNU General Public License v3.0 oder später](LICENSE) (GPL-3.0-or-later).
