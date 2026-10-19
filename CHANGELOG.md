# Changelog

Alle wichtigen Änderungen an SicherMail stehen in dieser Datei.

Das Format folgt [Keep a Changelog](https://keepachangelog.com/de/1.1.0/),
die Versionsnummern folgen [Semantic Versioning](https://semver.org/lang/de/).

## Unveröffentlicht

### Hinzugefügt

- Grundgerüst des Programms: SicherMail startet mit einem leeren Fenster (#1)
- Konten mit eigenem IMAP- und SMTP-Server einrichten und Verbindung testen; Passwörter liegen nur im Schlüsselbund des Betriebssystems (#2)
- Mails verschlüsselt empfangen: Ordner und Mailliste, neue Mails kommen automatisch an (IMAP IDLE), nach Verbindungsabbrüchen wird automatisch neu verbunden, ohne verlorene oder doppelte Mails; Gelesen-Status, Löschen und Verschieben werden zum Server übertragen (#3)
- Sichere Mailanzeige: Skripte und Formulare werden entfernt, externe Inhalte sind blockiert (auf Wunsch pro Mail oder pro Absender ladbar), Warnung bei Links mit abweichender Zieladresse, Anhänge nur über „Speichern unter“, Umschalter „Nur Text anzeigen“ (#4)
- Mails ohne Internet lesbar: lokal in einer verschlüsselten Datenbank gespeichert (Schlüssel nur im Schlüsselbund), Einstellung „Mails lokal speichern“ und Knopf „Lokalen Speicher leeren“ (#5)
- Mails schreiben, antworten, allen antworten und weiterleiten (mit Zitat und Anhängen), verschlüsselt über SMTP senden; gesendete Mails landen im Ordner „Gesendet“, bei Fehlern bleibt die Mail als Entwurf erhalten (#6)
