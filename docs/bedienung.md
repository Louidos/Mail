# Bedienung

## Programm starten

Siehe [README](../README.md#start).

## Konto einrichten

1. Links auf **Konto hinzufügen** klicken.
2. **Name** (wird als Absender angezeigt), **E-Mail-Adresse**, **Benutzername** und **Passwort** eintragen.
3. Optional **Automatisch ausfüllen**: SicherMail fragt über HTTPS bei der Domain deiner Adresse nach
   den Servereinstellungen (`https://autoconfig.<domain>/mail/config-v1.1.xml` bzw.
   `https://<domain>/.well-known/autoconfig/mail/config-v1.1.xml`). Es werden keine anderen Dienste gefragt.
   Gefundene Werte sind nur ein Vorschlag und lassen sich ändern.
4. **Posteingang (IMAP)** und **Postausgang (SMTP)**: Server, Port und Verschlüsselung eintragen.

   | Verschlüsselung | IMAP-Port | SMTP-Port | Bedeutung                                                    |
   | --------------- | --------- | --------- | ------------------------------------------------------------ |
   | TLS             | 993       | 465       | von Anfang an verschlüsselt                                  |
   | STARTTLS        | 143       | 587       | startet unverschlüsselt und wird sofort verschlüsselt        |

   Beim Wechsel der Verschlüsselung schlägt SicherMail den passenden Standardport vor.
   Eine unverschlüsselte Verbindung gibt es nicht. Bietet ein Server bei STARTTLS keine
   Verschlüsselung an, bricht SicherMail die Verbindung ab.
5. **Verbindung testen** meldet sich bei beiden Servern an und wieder ab, ohne etwas zu speichern.
6. **Konto anlegen** prüft die Verbindung erneut und speichert das Konto nur, wenn beide Server funktionieren.

### Fehlermeldungen beim Verbindungstest

| Meldung                                   | Was tun?                                                                 |
| ----------------------------------------- | ------------------------------------------------------------------------ |
| Server ist nicht erreichbar               | Servername und Port prüfen, Internetverbindung prüfen.                   |
| Zertifikat ungültig                       | Der Server weist sich nicht korrekt aus. Servernamen prüfen; bei eigenen Servern siehe unten. |
| Server bietet kein STARTTLS an            | Verschlüsselung auf TLS stellen (Port 993 bzw. 465) oder Serverbetreiber fragen. |
| Anmeldung fehlgeschlagen                  | Benutzername und Passwort prüfen.                                        |
| Server antwortet nicht rechtzeitig        | Später erneut versuchen.                                                 |
| Schlüsselbund nicht erreichbar            | Schlüsselbund des Betriebssystems entsperren (unter Linux z. B. GNOME Keyring oder KWallet). |

### Eigener Server mit selbstsigniertem Zertifikat

Zertifikate werden immer geprüft; es gibt keinen Knopf „trotzdem verbinden“. Betreibst du einen
eigenen Server mit selbstsigniertem Zertifikat, kannst du dessen Zertifikat bewusst hinterlegen:
Unter **Eigenes Zertifikat hinterlegen** das Serverzertifikat im PEM-Format einfügen
(`-----BEGIN CERTIFICATE-----` … `-----END CERTIFICATE-----`).

SicherMail akzeptiert dann für diesen Server **nur noch genau dieses Zertifikat**. Hinterlege es nur,
wenn du es selbst vom Serverbetreiber erhalten und geprüft hast. Wird das Zertifikat am Server
erneuert, musst du das neue hinterlegen.

## Konto bearbeiten und löschen

- **Bearbeiten**: Alle Angaben lassen sich ändern. Bleibt das Passwortfeld leer, wird das gespeicherte
  Passwort weiterverwendet. Gespeichert wird nur nach erfolgreichem Verbindungstest.
- **Löschen**: Entfernt das Konto aus SicherMail und das Passwort aus dem Schlüsselbund.
  Die Mails auf dem Server bleiben unverändert.

## Mails empfangen

- Links stehen unter jedem Konto seine **Ordner**. Der Posteingang wird nach dem Start automatisch geöffnet.
- Die **Mailliste** zeigt Absender, Betreff und Datum, neueste zuerst. Ungelesene Mails sind fett.
  Weitere Mails lädt **Ältere Mails laden**.
- **Neue Mails kommen von selbst**: SicherMail hält pro Konto eine Verbindung zum Posteingang offen
  (IMAP IDLE). Kann der Server kein IDLE, wird alle 2 Minuten nachgesehen.
- Bricht die Verbindung ab, zeigt SicherMail unten einen Hinweis und verbindet automatisch neu
  (Wartezeit 1 s, 2 s, 4 s … bis höchstens 5 Minuten). Danach wird abgeglichen, damit keine Mail fehlt
  und keine doppelt erscheint.
- Ist der Server nicht erreichbar, zeigt SicherMail die bereits geladenen Mails und einen Hinweis.

### Mails bearbeiten

Eine Mail anklicken öffnet sie rechts und markiert sie als gelesen. Darüber:

- **Als ungelesen markieren** / **Als gelesen markieren**
- **Verschieben nach …** in einen anderen Ordner
- **Löschen**: verschiebt die Mail in den Papierkorb. Gibt es keinen Papierkorb oder liegt die Mail
  schon darin, fragt SicherMail nach und löscht sie dann endgültig auf dem Server.

Alle Änderungen werden sofort zum Server übertragen und gelten damit auch für andere Programme.
SicherMail löscht nie von sich aus Mails auf dem Server.

## Mails sicher lesen

SicherMail zeigt Mails so an, dass Skripte, Tracking und gefälschte Links keinen Schaden anrichten:

- **Skripte, Formulare und eingebettete Fenster** werden vor der Anzeige entfernt. HTML-Mails werden in
  einem abgeschotteten Bereich angezeigt, in dem nichts ausgeführt und nichts aus dem Internet geladen wird.
- **Externe Inhalte** (z. B. Bilder von Webseiten, oft unsichtbare „Tracking-Pixel“) sind blockiert.
  Eine Leiste zeigt, wie viele blockiert wurden:
  - **Externe Inhalte laden** lädt sie für diese eine Mail – nur über verschlüsseltes HTTPS.
  - **Für … immer erlauben** lädt sie bei diesem Absender künftig automatisch.
- **Links** öffnen sich erst nach einem Klick im Standardbrowser. Beim Überfahren mit der Maus siehst du das
  echte Ziel. Zeigt der Linktext eine andere Adresse als das echte Ziel (typisch für Phishing), warnt
  SicherMail und öffnet den Link nur, wenn du **Trotzdem öffnen** wählst. Geöffnet werden nur
  `https:`, `http:` und `mailto:`.
- **Anhänge** werden nur aufgelistet und nie automatisch geöffnet. Mit **Speichern unter …** legst du sie
  ab. Prüfe Dateien, bevor du sie öffnest.
- **Nur Text anzeigen** zeigt statt der HTML-Fassung den reinen Text der Mail.

## Lesen ohne Internet (lokaler Speicher)

Unter **Einstellungen** steht **Mails lokal speichern (verschlüsselt)** – standardmässig eingeschaltet.

- Geladene Mails und die neuesten 50 Mails im Posteingang werden in einer verschlüsselten Datenbank auf dem
  Gerät abgelegt. Ohne Internet zeigt SicherMail diese Mails weiterhin an (mit einem Hinweis oben in der Liste).
- Der Schlüssel wird beim ersten Start zufällig erzeugt und liegt nur im Schlüsselbund des Betriebssystems.
  Ohne ihn ist die Datenbank nicht lesbar. Geht er verloren, legt SicherMail die Datenbank neu an und lädt
  die Mails wieder vom Server.
- **Lokalen Speicher leeren** löscht alle zwischengespeicherten Mails vom Gerät (auf dem Server bleibt alles).
- Wird die Einstellung ausgeschaltet, werden alle lokal gespeicherten Mails gelöscht; Mails werden dann nur
  noch bis zum Beenden im Arbeitsspeicher gehalten.

Unter **Externe Inhalte immer erlaubt für** lassen sich freigegebene Absender wieder entfernen.

## Wo liegen meine Daten?

- Kontoeinstellungen ohne Passwort: `konten.json` im App-Datenordner
  (Linux: `~/.local/share/ch.sichermail.app/`, macOS: `~/Library/Application Support/ch.sichermail.app/`,
  Windows: `%APPDATA%\ch.sichermail.app\`).
- Passwörter: ausschliesslich im Schlüsselbund des Betriebssystems (Eintrag „SicherMail“).
- Zwischengespeicherte Mails: `mails.db` im App-Datenordner, verschlüsselt mit SQLCipher; der Schlüssel liegt im
  Schlüsselbund (Eintrag `speicher:schluessel`).
- Einstellungen: `einstellungen.json` im App-Datenordner.
