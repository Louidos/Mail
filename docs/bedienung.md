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

## Wo liegen meine Daten?

- Kontoeinstellungen ohne Passwort: `konten.json` im App-Datenordner
  (Linux: `~/.local/share/ch.sichermail.app/`, macOS: `~/Library/Application Support/ch.sichermail.app/`,
  Windows: `%APPDATA%\ch.sichermail.app\`).
- Passwörter: ausschliesslich im Schlüsselbund des Betriebssystems (Eintrag „SicherMail“).
