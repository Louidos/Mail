# Changelog

Alle wichtigen Änderungen an SicherMail stehen in dieser Datei.

Das Format folgt [Keep a Changelog](https://keepachangelog.com/de/1.1.0/),
die Versionsnummern folgen [Semantic Versioning](https://semver.org/lang/de/).

## Unveröffentlicht

### Geändert

- Neues, ruhigeres Design in Blau und Weiss mit abgerundeten Ecken, weichen Schatten und gut sichtbarem Tastaturfokus; die dunkle Darstellung entfällt dabei (#31)

### Behoben

- Eine präparierte Mail mit vielen Verweisen auf dasselbe Bild kann SicherMail beim Öffnen nicht mehr durch vollen Arbeitsspeicher zum Absturz bringen (#22)
- Bei STARTTLS-Verbindungen kann eine Gegenstelle im Netz SicherMail vor dem Start der Verschlüsselung nicht mehr endlos hinhalten oder den Speicher füllen; nach spätestens 60 Sekunden wird abgebrochen (#23)
- Die 25-MB-Grenze für Anhänge gilt jetzt auch für Dateien, die nach dem Anhängen gewachsen sind; statt abzustürzen erscheint die gewohnte Meldung (#25)
- Ordner mit sehr hohen oder ungültigen Mailnummern (UID) werden wieder abgeglichen, statt hängen zu bleiben (#26)
- Scheitert das Löschen eines Kontos (z. B. weil der Schlüsselbund gesperrt ist), kommen für dieses Konto weiterhin neue Mails an (#27)
- Scheitert das Löschen eines Entwurfs, zeigt die Entwürfe-Liste jetzt eine Meldung an (#28)
- Ein Klick auf einen Mail-Link (mailto) mit fehlerhafter Kodierung bewirkt nicht mehr einfach nichts, sondern wird normal geprüft und geöffnet (#29)

## 0.1.1 – 2026-10-19

### Behoben

- Mehr Schutz: Nach einer Änderung von Server oder Benutzername muss das Passwort neu eingegeben werden, damit es nicht an einen falschen Server geht. Konto löschen entfernt das Passwort zuverlässig. Verdächtige Links werden auch mit verstecktem oder getarntem Text erkannt. Anhangnamen mit unsichtbaren oder Schreibrichtungs-Zeichen werden entschärft. `mailto:`-Links hängen keine Dateien mehr an. Externe Bilder aus dem eigenen Netz werden nie geladen (#16)
- Neue Mails kommen auch nach einem stillen Verbindungsabbruch (Ruhezustand, WLAN-Wechsel) spätestens nach 2 Minuten an, und lehnt der Server IDLE ab, wird alle 2 Minuten abgefragt. Verschieben und die Ablage in „Gesendet“ erzeugen nach Verbindungsabbrüchen keine doppelten Mails mehr. Lokale Entwürfe gehen nicht mehr verloren, wenn die Datenbank kurz gesperrt ist. Bei dauerhaften Fehlern meldet sich SicherMail nicht mehr sekündlich neu an (#17)

## 0.1.0 – 2026-10-19

### Hinzugefügt

- Grundgerüst des Programms: SicherMail startet mit einem leeren Fenster (#1)
- Konten mit eigenem IMAP- und SMTP-Server einrichten und Verbindung testen; Passwörter liegen nur im Schlüsselbund des Betriebssystems (#2)
- Mails verschlüsselt empfangen: Ordner und Mailliste, neue Mails kommen automatisch an (IMAP IDLE), nach Verbindungsabbrüchen wird automatisch neu verbunden, ohne verlorene oder doppelte Mails; Gelesen-Status, Löschen und Verschieben werden zum Server übertragen (#3)
- Sichere Mailanzeige: Skripte und Formulare werden entfernt, externe Inhalte sind blockiert (auf Wunsch pro Mail oder pro Absender ladbar), Warnung bei Links mit abweichender Zieladresse, Anhänge nur über „Speichern unter“, Umschalter „Nur Text anzeigen“ (#4)
- Mails ohne Internet lesbar: lokal in einer verschlüsselten Datenbank gespeichert (Schlüssel nur im Schlüsselbund), Einstellung „Mails lokal speichern“ und Knopf „Lokalen Speicher leeren“ (#5)
- Mails schreiben, antworten, allen antworten und weiterleiten (mit Zitat und Anhängen), verschlüsselt über SMTP senden; gesendete Mails landen im Ordner „Gesendet“, bei Fehlern bleibt die Mail als Entwurf erhalten (#6)
- Automatische Prüfung auf bekannte Sicherheitslücken in Abhängigkeiten und dokumentiertes Sicherheitskonzept (#7)
