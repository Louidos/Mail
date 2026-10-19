# Testdaten S/MIME

Nur für Tests. Alles hier wurde eigens dafür mit OpenSSL 3 erzeugt (`./erzeugen.sh`, erzeugt alles neu) und
schützt nichts. Adressen unter `example.org`, keine echten Personen. Passwort aller `.p12`-Dateien:
`test-passwort`.

Test-PKI (alle Zertifikate gültig bis 2125, sofern nicht anders angegeben):

- `test-ca.pem` – „SicherMail Test-Wurzel“ (RSA 3072). Die Tests übernehmen sie als vertrauenswürdige Wurzel.
- `zwischen.pem` – „SicherMail Test-Zwischenstelle“, von der Test-Wurzel ausgestellt; stellt die Personen aus.
- `fremd-ca.pem` – „Unbekannte Test-Wurzel“ (wird nie übernommen).
- Die privaten Schlüssel der Stellen werden nach dem Erzeugen gelöscht.

Personen (KeyUsage digitalSignature + keyEncipherment, ExtendedKeyUsage emailProtection, SAN `email:`):

- `anna.p12`, `anna.pem` – Anna Test `anna@example.org` (RSA 2048, Adresse zusätzlich im Subject `E=`)
- `bea.p12`, `bea.pem`, `bea.key` – Bea Test `bea@example.org`; mit `bea.key` signieren die Tests Mails von Bea
- `eve.p12`, `eve.pem` – Eve Test `eve@example.org` (für „passt nicht zu diesem Konto“)
- `bea-fremd.pem`, `bea-fremd.key` – Bea, aber von der unbekannten Wurzel ausgestellt („unbekannte Stelle“)
- `bea-abgelaufen.pem`, `bea-abgelaufen.key` – Bea, gültig nur 2020–2021 („abgelaufen“)
- `test.p12`, `empfaenger.p12` (+ `.pem`) – Konten `test@localhost` und `empfaenger@localhost` des
  Test-Mailservers (Rundlauf senden → empfangen)
- `anna-schwach.p12` – Anna mit RSA 1024 (wird abgelehnt)
- `anna-server.p12` – Anna, aber ExtendedKeyUsage nur serverAuth (wird abgelehnt)

Fertige Mails (an Anna, von Bea):

- `unsicher-rc2-40.eml`, `unsicher-des.eml` – mit RC2 bzw. DES verschlüsselt (werden abgelehnt)
- `unsicher-md5.eml` – mit MD5 signiert (wird abgelehnt)
- `cli-signiert.eml` – mit `openssl cms -sign` signiert (`multipart/signed`, SHA-256)
- `cli-verschluesselt.eml` – signiert und dann mit `openssl cms -encrypt -stream -aes256` verschlüsselt (BER mit
  unbestimmter Länge, wie es z. B. Thunderbird schreibt)

Alle anderen Testmails (AES-CBC, AES-GCM, signiert, teilweise …) baut `tests/integration/smime.rs` selbst.

Nie echte Zertifikate, Schlüssel oder Daten hier ablegen.
