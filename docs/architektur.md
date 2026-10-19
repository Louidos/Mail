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

## Tauri-Sicherheit

- Strenge Content-Security-Policy in `src-tauri/tauri.conf.json`: nur eigene Skripte,
  keine Remote-URLs, keine Formulare, keine Fremd-Frames.
- Capabilities (`src-tauri/capabilities/default.json`): nur das Empfangen von Ereignissen.
  Kein Shell-, Dateisystem- oder HTTP-Zugriff aus dem Frontend.
