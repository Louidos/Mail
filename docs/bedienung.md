# Bedienung

## Programm installieren und starten

Fertige Installationspakete für Windows, macOS und Linux hängen an jedem
[Release](https://github.com/Louidos/Mail/releases). Wie du sie herunterlädst, Prüfsumme und Herkunft prüfst und
was bei Warnungen wegen fehlender Signatur zu tun ist, steht im [README unter „Installation“](../README.md#installation).
Nach der Installation startest du SicherMail wie jedes andere Programm.

Zum Starten aus dem Quellcode siehe [README](../README.md#start).

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

- **Bearbeiten**: Alle Angaben lassen sich ändern (unter **Erweitert** auch die Server-Kennung für die Absenderprüfung). Bleibt das Passwortfeld leer, wird das gespeicherte
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
  - **Externe Inhalte laden** lädt sie für diese eine Mail – nur über verschlüsseltes HTTPS und standardmässig immer
    direkt, nie über einen Proxy. Kommt dein Rechner nur über einen Proxy ins Internet, bleiben externe Bilder leer,
    bis du unter **Einstellungen** den Proxy für externe Bilder einschaltest (siehe unten).
  - **Für … immer erlauben** lädt sie bei diesem Absender künftig automatisch.
- **Absender prüfen**: Neben dem Absender steht, ob dein Mailserver ihn mit DKIM, SPF und DMARC bestätigt hat
  (siehe unten).
- **Links** öffnen sich erst nach einem Klick im Standardbrowser. Beim Überfahren mit der Maus siehst du das
  echte Ziel. Zeigt der Linktext eine andere Adresse als das echte Ziel (typisch für Phishing), warnt
  SicherMail und öffnet den Link nur, wenn du **Trotzdem öffnen** wählst. Geöffnet werden nur
  `https:`, `http:` und `mailto:`.
- **Anhänge** werden nur aufgelistet und nie automatisch geöffnet. Mit **Speichern unter …** legst du sie
  ab. Prüfe Dateien, bevor du sie öffnest.
- **Nur Text anzeigen** zeigt statt der HTML-Fassung den reinen Text der Mail.

### Absender prüfen (DKIM, SPF, DMARC)

Absenderadressen lassen sich fälschen. Viele Mailserver prüfen deshalb beim Empfang mit DKIM, SPF und DMARC,
ob eine Mail wirklich vom angegebenen Absender stammt, und schreiben das Ergebnis in die Kopfzeile
`Authentication-Results`. SicherMail zeigt dieses Ergebnis neben dem Absender an – ohne selbst etwas im Internet
abzufragen:

| Anzeige | Bedeutung |
| --- | --- |
| ✓ **Absender bestätigt** | Dein Server hat den Absender bestätigt (DMARC bestanden, bzw. DKIM oder SPF bestanden und passend zur Absenderdomain). |
| – **Absender nicht geprüft** | Dein Server hat kein (verwertbares) Ergebnis geliefert, oder die Absenderdomain hat keine DMARC-Regel. Das ist kein Hinweis auf Betrug, aber auch keine Bestätigung. |
| ⚠ **Absender wahrscheinlich gefälscht** | DMARC ist fehlgeschlagen, oder DKIM und SPF sind fehlgeschlagen bzw. gehören nicht zur Absenderdomain. |

- Ein Klick auf das Feld zeigt die Einzelheiten: Ergebnisse von DKIM, SPF und DMARC und die Kennung des Servers,
  der geprüft hat.
- Bei **wahrscheinlich gefälscht** erscheint oben ein deutlicher Warnbalken: „Der Absender ist wahrscheinlich gefälscht.
  Antworte nicht, öffne keine Links und Anhänge, wenn du unsicher bist.“ Für solche Mails gilt die Freigabe
  **Für … immer erlauben** nicht und wird auch nicht angeboten; **Externe Inhalte laden** für diese eine Mail geht weiterhin.
- SicherMail vertraut nur dem Ergebnis deines eigenen Servers: dem obersten `Authentication-Results`-Header mit der
  **Server-Kennung für die Absenderprüfung**. Standard ist der Name des IMAP-Servers. Schreibt dein Server eine andere
  Kennung (sie steht in der Kopfzeile ganz vorne, z. B. `mx.example.org`), trägst du sie im Konto unter
  **Bearbeiten** → **Erweitert** ein. Gmail verwendet z. B. `mx.google.com`.

## Mails schreiben

- **Neue Mail** (oben links) öffnet das Fenster zum Verfassen. Bei mehreren Konten wählst du unter **Von** das Konto.
- **An**, **Cc** und **Bcc**: mehrere Adressen mit Komma trennen, z. B. `Anna Muster <anna@example.org>, bea@example.org`.
  Bcc-Empfänger sehen die anderen Empfänger nicht und werden selbst nicht angezeigt.
- Standardmässig wird reiner Text gesendet. **Zusätzlich als einfaches HTML senden** fügt eine einfache HTML-Fassung
  hinzu (Links anklickbar, keine Bilder oder Skripte).
- **Datei anhängen …** öffnet die Dateiauswahl; insgesamt sind bis 25 MB möglich.
- **Verschlüsseln** und **Signieren** mit OpenPGP: siehe [Verschlüsselung mit OpenPGP](#verschlüsselt-und-signiert-senden).
- **Senden**: SicherMail sichert den Text zuerst als Entwurf und verschickt die Mail dann verschlüsselt über SMTP.
  Danach wird sie im Ordner „Gesendet“ abgelegt (siehe unten).
- Schlägt das Senden fehl (z. B. Server nicht erreichbar), bleibt das Fenster offen, du siehst eine klare Meldung,
  und der Text liegt als **Entwurf** bereit. Escape schliesst das Fenster nicht, damit nichts aus Versehen verloren geht.
- **Als Entwurf speichern** und **Verwerfen** (mit Rückfrage).

### Antworten und weiterleiten

In der Leseansicht:

- **Antworten**: an den Absender (bzw. an die Antwortadresse „Reply-To“), Betreff mit „Re:“, Originaltext zitiert.
- **Allen antworten**: zusätzlich an alle Empfänger und in Cc, ohne deine eigene Adresse.
- **Weiterleiten**: Betreff mit „Fwd:“, Kopf und Text der Originalmail, Anhänge werden übernommen.

Antworten setzen die Kopfzeilen `In-Reply-To` und `References`, damit andere Programme sie dem Gespräch zuordnen.

### Entwürfe

Unter jedem Konto öffnet **Entwürfe** die Liste der gespeicherten Entwürfe (zum Öffnen oder Löschen). Entwürfe liegen nur
auf diesem Gerät – verschlüsselt, solange „Mails lokal speichern“ eingeschaltet ist.

### Ordner „Gesendet“

Gesendete Mails werden per IMAP im Ordner „Gesendet“ des Kontos abgelegt (gibt es keinen, legt SicherMail „Sent“ an).
Manche Anbieter (z. B. Gmail) tun das selbst; dann im Konto **Gesendete Mails im Ordner „Gesendet“ ablegen** ausschalten,
sonst liegen sie doppelt dort.

## Darstellung hell oder dunkel

Unter **Einstellungen** → **Darstellung** wählst du **Automatisch (wie das System)**, **Hell** oder **Dunkel**.

- Standard ist **Automatisch**: SicherMail folgt der Hell-/Dunkel-Einstellung deines Systems, auch wenn sie sich
  während des Betriebs ändert.
- Die Wahl wirkt sofort und bleibt nach einem Neustart erhalten; auch die Titelleiste des Fensters zieht mit,
  soweit das System das unterstützt.
- Der Inhalt von Mails bleibt auch in der dunklen Darstellung auf hellem Grund, weil viele HTML-Mails nur für
  weissen Hintergrund gestaltet sind. Nur der Rahmen drumherum ist dunkel.

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

## Externe Bilder über einen Proxy

Kommt dein Rechner nur über einen Proxy ins Internet (z. B. im Firmennetz), kannst du unter **Einstellungen** →
**Externe Bilder** den Schalter **Proxy für externe Bilder verwenden** einschalten, die Adresse eintragen
(z. B. `http://proxy.example.org:3128`) und **Proxy übernehmen** klicken.

- Standardmässig ist der Schalter aus: Dann lädt SicherMail externe Bilder immer direkt und nie über einen Proxy –
  auch nicht über einen im System eingestellten (`HTTPS_PROXY` usw.).
- Erlaubt ist nur `http://` oder `https://` mit Server und Port, ohne Benutzername und Passwort. Bei einer ungültigen
  Adresse erscheint eine Meldung, und externe Bilder bleiben blockiert. SOCKS wird nicht unterstützt.
- Geladen wird weiterhin nur nach deiner Freigabe (pro Mail oder für einen Absender).
- **Einschränkung:** Der Proxy löst Adressen selbst auf. SicherMail kann dann nicht mehr vollständig verhindern, dass
  Bilder aus deinem eigenen Netz geladen werden. Offensichtliche Adressen im eigenen Netz (z. B. `192.168.…`,
  `localhost`) lehnt SicherMail trotzdem ab, bevor sie den Proxy erreichen.

## Verschlüsselung mit OpenPGP

Mit OpenPGP verschlüsselte Mails kann nur lesen, wer den passenden privaten Schlüssel hat – auch der Mailserver nicht.
Signaturen zeigen, ob eine Mail wirklich vom Inhaber eines Schlüssels stammt und unverändert ist.

### Schlüssel verwalten

Links unten öffnet **Schlüssel (OpenPGP)** die Schlüsselverwaltung.

- **Eigener Schlüssel** (einer pro Konto):
  - **Schlüssel erzeugen**: Passphrase (mindestens 8 Zeichen) zweimal eingeben. SicherMail erzeugt einen modernen
    Schlüssel (Ed25519/Cv25519) mit deinem Namen und deiner Adresse.
  - **Eigenen Schlüssel importieren …**: Passphrase eingeben, dann die Datei wählen (ASCII-armored, z. B. aus
    `gpg --armor --export-secret-keys`). Die Passphrase muss stimmen, und eine User-ID des Schlüssels muss die Adresse
    des Kontos enthalten.
  - **Öffentlichen Schlüssel exportieren …** speichert deinen öffentlichen Schlüssel als `.asc`-Datei. Gib ihn an
    deine Kontakte weiter, damit sie dir verschlüsselt schreiben können. Der private Schlüssel verlässt das Gerät nie.
  - **Löschen** (mit Rückfrage) entfernt den Schlüssel endgültig. Mails, die an ihn verschlüsselt sind, kannst du
    danach nicht mehr lesen – bewahre eine Sicherung auf, wenn du sie brauchst.
- **Schlüssel anderer**: **Öffentlichen Schlüssel importieren …** liest eine `.asc`-Datei. Hängt ein Schlüssel an einer
  Mail, steht dort beim Anhang **Schlüssel importieren**. Vergleiche den angezeigten **Fingerabdruck** mit der Person
  (z. B. am Telefon), bevor du dich darauf verlässt.
- Schlüsselserver werden nie abgefragt.

### Verschlüsselte und signierte Mails lesen

Über dem Inhalt einer Mail steht, was SicherMail festgestellt hat (immer mit Symbol und Text):

| Anzeige | Bedeutung |
| --- | --- |
| 🔒 Verschlüsselt | Die Mail war verschlüsselt und wurde entschlüsselt. |
| 🔒 Kann nicht entschlüsselt werden | Kein passender Schlüssel, Passphrase fehlt, oder die Mail ist beschädigt bzw. manipuliert. |
| ⚠ Nur teilweise verschlüsselt | Nur ein Teil der Mail ist verschlüsselt. Er wird aus Sicherheitsgründen nicht automatisch entschlüsselt. |
| ✓ Signiert und gültig (Schlüssel von …) | Die Signatur stimmt, und der Schlüssel gehört zur Absenderadresse. |
| ⚠ Signatur gültig, aber der Schlüssel gehört nicht zur Absenderadresse | Signiert hat jemand anderes als der angegebene Absender. |
| ✗ Signatur ungültig | Die Mail wurde nach dem Signieren verändert, oder die Signatur ist beschädigt. |
| ? Signiert, Schlüssel unbekannt | Den Schlüssel des Absenders hast du (noch) nicht importiert. |
| ⚠ Nur ein Teil dieser Mail ist signiert | Was ausserhalb des signierten Teils steht, kann von jemand anderem stammen. |

- Braucht der Schlüssel seine **Passphrase**, erscheint ein Eingabefeld. Nach **Entsperren** zeigt SicherMail die Mail
  entschlüsselt an. Die Passphrase wird nicht gespeichert, sondern nur bis zum Beenden von SicherMail im
  Arbeitsspeicher gehalten.
- Entschlüsselte Mails werden genauso sicher angezeigt wie alle anderen (keine Skripte, externe Inhalte blockiert).
- Im lokalen Speicher bleibt nur die verschlüsselte Fassung; der entschlüsselte Text wird nirgends abgelegt.
- Erkannt werden PGP/MIME (wie von Thunderbird, Enigmail, Mailvelope …) und Inline-PGP im Text.

### Verschlüsselt und signiert senden

Im Schreibfenster stehen zwei Schalter:

- **Verschlüsseln (OpenPGP)**: Nur Empfänger mit dem passenden privaten Schlüssel können Text und Anhänge lesen.
  Dafür brauchst du von **jedem** Empfänger (An, Cc und Bcc) den öffentlichen Schlüssel und einen eigenen Schlüssel.
  Fehlt einer, nennt SicherMail die Empfänger und **sendet nicht** – auch nicht unverschlüsselt. Importiere dann die
  Schlüssel oder schalte „Verschlüsseln“ bewusst aus.
- **Signieren (OpenPGP)**: Empfänger können prüfen, dass die Mail von dir stammt und unverändert ist. Beim ersten
  Senden fragt SicherMail nach der Passphrase deines Schlüssels; sie bleibt bis zum Beenden im Arbeitsspeicher.
- Antwortest du auf eine verschlüsselte Mail (oder leitest sie weiter), sind beide Schalter schon eingeschaltet.
- Verschlüsselte Mails werden immer auch an deinen eigenen Schlüssel verschlüsselt; so kannst du sie im Ordner
  „Gesendet“ weiterhin lesen.
- **Bcc** bei Verschlüsselung: Jeder Bcc-Empfänger bekommt eine eigene, nur für ihn verschlüsselte Mail. So erfahren
  die anderen Empfänger nichts von ihm.
- **Nicht verschlüsselt** werden Betreff, Absender, Empfänger und Datum – schreibe Vertrauliches nicht in den Betreff.
- Entwürfe liegen nur auf diesem Gerät (nie auf dem Server); in der Entwürfe-Liste sind verschlüsselte mit 🔒
  markiert.

## Wo liegen meine Daten?

- Kontoeinstellungen ohne Passwort: `konten.json` im App-Datenordner
  (Linux: `~/.local/share/ch.sichermail.app/`, macOS: `~/Library/Application Support/ch.sichermail.app/`,
  Windows: `%APPDATA%\ch.sichermail.app\`).
- Passwörter: ausschliesslich im Schlüsselbund des Betriebssystems (Eintrag „SicherMail“).
- Zwischengespeicherte Mails: `mails.db` im App-Datenordner, verschlüsselt mit SQLCipher; der Schlüssel liegt im
  Schlüsselbund (Eintrag `speicher:schluessel`).
- Einstellungen (inkl. freigegebener Absender und Proxy-Adresse): `einstellungen.json` im App-Datenordner.
- OpenPGP-Schlüssel: `schluessel.db` im App-Datenordner, verschlüsselt mit SQLCipher; der Schlüssel liegt im
  Schlüsselbund (Eintrag `pgp:schluessel`). Private Schlüssel sind zusätzlich mit deiner Passphrase geschützt.
  „Lokalen Speicher leeren“ lässt diese Datei unberührt.
