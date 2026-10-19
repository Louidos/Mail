# Changelog

Alle wichtigen Änderungen an SicherMail stehen in dieser Datei.

Das Format folgt [Keep a Changelog](https://keepachangelog.com/de/1.1.0/),
die Versionsnummern folgen [Semantic Versioning](https://semver.org/lang/de/).

## Unveröffentlicht

### Hinzugefügt

- Grundgerüst des Programms: SicherMail startet mit einem leeren Fenster (#1)
- Konten mit eigenem IMAP- und SMTP-Server einrichten und Verbindung testen; Passwörter liegen nur im Schlüsselbund des Betriebssystems (#2)
- Mails verschlüsselt empfangen: Ordner und Mailliste, neue Mails kommen automatisch an (IMAP IDLE), nach Verbindungsabbrüchen wird automatisch neu verbunden, ohne verlorene oder doppelte Mails; Gelesen-Status, Löschen und Verschieben werden zum Server übertragen (#3)
