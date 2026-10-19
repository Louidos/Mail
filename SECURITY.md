# Sicherheitsrichtlinie

## Unterstützte Versionen

Sicherheitskorrekturen gibt es für die jeweils neueste veröffentlichte Version.

## Sicherheitslücke melden

Bitte melde Sicherheitslücken **nicht** über öffentliche Issues.

1. Nutze auf GitHub unter **Security → Report a vulnerability** die private Meldung
   („Private vulnerability reporting“) dieses Repositorys.
2. Beschreibe, was betroffen ist, wie man die Lücke nachstellen kann und welche Auswirkung du erwartest.
3. Schicke keine echten Zugangsdaten oder echten Mailinhalte mit.

Wir bestätigen den Eingang innerhalb von 7 Tagen, halten dich über den Stand auf dem Laufenden
und veröffentlichen eine Korrektur als neue PATCH-Version. Auf Wunsch wirst du im CHANGELOG genannt.

## Grundregeln von SicherMail

Ausführlich im [Sicherheitskonzept](docs/sicherheit.md).

- Nur verschlüsselte Verbindungen (TLS oder STARTTLS), Zertifikate werden immer geprüft.
- Passwörter nur im Schlüsselbund des Betriebssystems.
- Keine Telemetrie, keine Verbindungen ausser zu den vom Nutzer angegebenen Servern.

## Automatische Prüfungen

- `cargo audit` und `npm audit --audit-level=high` laufen in der CI bei jedem Push, jedem Pull Request und wöchentlich.
- Dependabot schlägt Updates für Cargo, npm und GitHub Actions vor.
