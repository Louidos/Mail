#!/usr/bin/env bash
# Startet oder stoppt den lokalen Test-Mailserver (GreenMail) für die Integrationstests.
#
#   scripts/test-mailserver.sh start   # Zertifikat erzeugen und Server starten
#   scripts/test-mailserver.sh stop    # Server stoppen und entfernen
#
# Der Server bekommt ein frisch erzeugtes, selbstsigniertes Zertifikat.
# Die Tests hinterlegen genau dieses Zertifikat (Pinning), so wie es ein Nutzer
# für einen eigenen Server mit selbstsigniertem Zertifikat tun würde.
# Nur für Tests – nie gegen echte Postfächer testen.
#
# Ports (nur auf 127.0.0.1):
#   3025 SMTP (ohne TLS, nur zum Ablehnungs-Test), 3465 SMTP über TLS
#   3143 IMAP (ohne TLS, kein STARTTLS – nur zum Ablehnungs-Test), 3993 IMAP über TLS
set -euo pipefail

NAME="sichermail-testserver"
IMAGE="greenmail/standalone:2.1.3"
HAUPTORDNER="$(cd "$(dirname "$0")/.." && pwd)"
ORDNER="$HAUPTORDNER/.testserver"
PASSWORT_KEYSTORE="testserver-keystore"

start() {
  mkdir -p "$ORDNER"
  if [ ! -f "$ORDNER/cert.pem" ]; then
    openssl req -x509 -newkey rsa:2048 -nodes -days 30 \
      -keyout "$ORDNER/key.pem" -out "$ORDNER/cert.pem" \
      -subj "/CN=localhost" \
      -addext "subjectAltName=DNS:localhost,IP:127.0.0.1" 2>/dev/null
    openssl pkcs12 -export -in "$ORDNER/cert.pem" -inkey "$ORDNER/key.pem" \
      -out "$ORDNER/keystore.p12" -name greenmail -passout "pass:$PASSWORT_KEYSTORE"
    chmod 644 "$ORDNER/keystore.p12"
  fi

  docker rm -f "$NAME" >/dev/null 2>&1 || true
  docker run -d --name "$NAME" \
    -p 127.0.0.1:3025:3025 -p 127.0.0.1:3143:3143 \
    -p 127.0.0.1:3465:3465 -p 127.0.0.1:3993:3993 \
    -v "$ORDNER:/zertifikat:ro" \
    -e GREENMAIL_OPTS="-Dgreenmail.setup.test.smtp -Dgreenmail.setup.test.smtps -Dgreenmail.setup.test.imap -Dgreenmail.setup.test.imaps -Dgreenmail.hostname=0.0.0.0 -Dgreenmail.tls.keystore.file=/zertifikat/keystore.p12 -Dgreenmail.tls.keystore.password=$PASSWORT_KEYSTORE -Dgreenmail.auth.disabled=false -Dgreenmail.users=test:testpasswort@localhost,empfaenger:testpasswort@localhost" \
    "$IMAGE" >/dev/null

  # Warten, bis IMAP über TLS antwortet.
  for _ in $(seq 1 60); do
    if (echo | openssl s_client -connect 127.0.0.1:3993 -quiet 2>/dev/null | grep -q "OK"); then
      echo "Test-Mailserver läuft (Zertifikat: $ORDNER/cert.pem)"
      return 0
    fi
    sleep 1
  done
  echo "Test-Mailserver ist nicht rechtzeitig gestartet" >&2
  docker logs "$NAME" >&2 || true
  return 1
}

stop() {
  docker rm -f "$NAME" >/dev/null 2>&1 || true
  echo "Test-Mailserver gestoppt"
}

case "${1:-start}" in
  start) start ;;
  stop) stop ;;
  *)
    echo "Aufruf: $0 [start|stop]" >&2
    exit 2
    ;;
esac
