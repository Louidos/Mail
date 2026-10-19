# Testdaten OpenPGP

Nur für Tests. Die Schlüssel wurden eigens dafür mit GnuPG 2.4 erzeugt und schützen nichts:

- `anna-privat.asc` – privater Schlüssel „Anna Test <anna@example.org>“ (Ed25519/Cv25519),
  Passphrase `test-passphrase`
- `anna-oeffentlich.asc`, `bea-oeffentlich.asc` – öffentliche Schlüssel
- `verschluesselt-signiert.asc` – von Bea signiert und an Anna verschlüsselt (GnuPG-AEAD/OCB),
  Inhalt: MIME-Teil `text/plain`
- `inline.asc` – Inline-PGP, von Bea signiert und an Anna verschlüsselt
- `klartext-signiert.asc` – Klartext-Signatur von Bea
- `signiert-teil.txt` + `teil1.sig` – MIME-Teil (CRLF) mit abgetrennter Signatur von Bea (PGP/MIME)

Nie echte Schlüssel oder Daten hier ablegen.
