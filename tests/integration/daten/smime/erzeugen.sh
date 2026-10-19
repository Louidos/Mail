#!/usr/bin/env bash
# Erzeugt die S/MIME-Testdaten in diesem Ordner neu (nur für Tests, keine echten Personen).
# Braucht OpenSSL 3 (für RC2/DES zusätzlich den „legacy“-Provider).
set -euo pipefail
cd "$(dirname "$0")"
rm -rf .arbeit && mkdir .arbeit
PASSWORT="test-passwort"

ext() { # Name, Inhalt
  printf '%s\n' "$2" > ".arbeit/$1.ext"
}
ext ca "basicConstraints=critical,CA:TRUE
keyUsage=critical,keyCertSign,cRLSign
subjectKeyIdentifier=hash"
ext zwischen "basicConstraints=critical,CA:TRUE,pathlen:0
keyUsage=critical,keyCertSign,cRLSign
subjectKeyIdentifier=hash
authorityKeyIdentifier=keyid"
person() { # Adresse
  ext "$1" "basicConstraints=critical,CA:FALSE
keyUsage=critical,digitalSignature,keyEncipherment
extendedKeyUsage=emailProtection
subjectAltName=email:$1
subjectKeyIdentifier=hash
authorityKeyIdentifier=keyid"
}

wurzel() { # Name, CN
  openssl req -x509 -newkey rsa:3072 -nodes -keyout "$1.key" -out "$1.pem" -days 36500 \
    -subj "/O=SicherMail Test/CN=$2" -extensions v3 -config <(printf '[req]\ndistinguished_name=d\n[d]\n[v3]\n'; cat .arbeit/ca.ext) 2>/dev/null
}
ausstellen() { # Name, Aussteller, Subjekt, Erweiterung, [Bits]
  openssl req -new -newkey "rsa:${5:-2048}" -nodes -keyout "$1.key" -out ".arbeit/$1.csr" -subj "$3" 2>/dev/null
  openssl x509 -req -in ".arbeit/$1.csr" -CA "$2.pem" -CAkey "$2.key" -CAcreateserial \
    -days 36500 -sha256 -extfile ".arbeit/$4.ext" -out "$1.pem" 2>/dev/null
}

# Test-Wurzel und Zwischenstelle; eine zweite, unbekannte Wurzel
wurzel test-ca "SicherMail Test-Wurzel"
ausstellen zwischen test-ca "/O=SicherMail Test/CN=SicherMail Test-Zwischenstelle" zwischen 3072
wurzel fremd-ca "Unbekannte Test-Wurzel"

# Personen (Adressen unter example.org)
for p in anna bea eve; do person "$p@example.org"; done
# Konten des Test-Mailservers (GreenMail) für Rundlauf-Tests beim Senden
for p in test empfaenger; do person "$p@localhost"; done
ausstellen anna zwischen "/CN=Anna Test/emailAddress=anna@example.org" anna@example.org
ausstellen bea zwischen "/CN=Bea Test" bea@example.org
ausstellen eve zwischen "/CN=Eve Test" eve@example.org
ausstellen test zwischen "/CN=Test Konto" test@localhost
ausstellen empfaenger zwischen "/CN=Empfaenger Konto" empfaenger@localhost
ausstellen bea-fremd fremd-ca "/CN=Bea Test" bea@example.org
ausstellen anna-schwach zwischen "/CN=Anna Test" anna@example.org 1024
ext server "basicConstraints=critical,CA:FALSE
keyUsage=critical,digitalSignature,keyEncipherment
extendedKeyUsage=serverAuth
subjectAltName=email:anna@example.org"
ausstellen anna-server zwischen "/CN=Anna Test" server

# Abgelaufenes Zertifikat für Bea (gültig 2020–2021), mit „openssl ca“
mkdir -p .arbeit/ca/neu && : > .arbeit/ca/index.txt && echo 1000 > .arbeit/ca/serial
cat > .arbeit/ca.cnf <<CNF
[ca]
default_ca=t
[t]
dir=.arbeit/ca
database=\$dir/index.txt
new_certs_dir=\$dir/neu
serial=\$dir/serial
default_md=sha256
policy=p
copy_extensions=none
unique_subject=no
[p]
commonName=supplied
CNF
openssl req -new -newkey rsa:2048 -nodes -keyout bea-abgelaufen.key -out .arbeit/abgelaufen.csr -subj "/CN=Bea Test" 2>/dev/null
openssl ca -batch -config .arbeit/ca.cnf -cert zwischen.pem -keyfile zwischen.key -in .arbeit/abgelaufen.csr \
  -startdate 20200101000000Z -enddate 20210101000000Z -extfile .arbeit/bea@example.org.ext -out bea-abgelaufen.pem 2>/dev/null
openssl x509 -in bea-abgelaufen.pem -out bea-abgelaufen.pem

# PKCS#12-Dateien (mit Zwischenstelle), Passwort: test-passwort
p12() { # Name
  openssl pkcs12 -export -inkey "$1.key" -in "$1.pem" -certfile zwischen.pem -name "$1" \
    -passout "pass:$PASSWORT" -out "$1.p12"
}
p12 anna; p12 bea; p12 eve; p12 test; p12 empfaenger; p12 anna-schwach; p12 anna-server

# Unsichere Verfahren (nur zum Ablehnen): an Anna mit RC2 bzw. DES verschlüsselt, mit MD5 signiert
mail() { printf 'From: Bea Test <bea@example.org>\r\nTo: Anna Test <anna@example.org>\r\nSubject: %s\r\nDate: Mon, 19 Oct 2026 10:00:00 +0200\r\nMessage-ID: <%s@example.org>\r\n' "$1" "$2"; }
inhalt() { printf 'Content-Type: text/plain; charset=utf-8\r\n\r\nGeheimer Testinhalt\r\n'; }
inhalt > .arbeit/inhalt.txt
for verfahren in rc2-40 des; do
  { mail "Unsicher $verfahren" "$verfahren"; openssl cms -encrypt -provider legacy -provider default -"$verfahren" -in .arbeit/inhalt.txt anna.pem | tr -d '\r' | sed 's/$/\r/'; } > "unsicher-$verfahren.eml"
done
{ mail "MD5 signiert" md5; openssl cms -sign -md md5 -signer bea.pem -inkey bea.key -certfile zwischen.pem -in .arbeit/inhalt.txt | tr -d '\r' | sed 's/$/\r/'; } > unsicher-md5.eml

# Mit der Kommandozeile erzeugt (Interop): signiert (multipart/signed) und, als BER mit
# unbestimmter Länge („-stream“, wie z. B. Thunderbird es schreibt), signiert und dann verschlüsselt.
{ mail "Kommandozeile signiert" cli-signiert; openssl cms -sign -md sha256 -signer bea.pem -inkey bea.key -certfile zwischen.pem -in .arbeit/inhalt.txt | tr -d '\r' | sed 's/$/\r/'; } > cli-signiert.eml
openssl cms -sign -md sha256 -signer bea.pem -inkey bea.key -certfile zwischen.pem -in .arbeit/inhalt.txt -out .arbeit/signiert.txt
{ mail "Kommandozeile verschlüsselt" cli-verschluesselt; openssl cms -encrypt -stream -aes256 -in .arbeit/signiert.txt anna.pem | tr -d '\r' | sed 's/$/\r/'; } > cli-verschluesselt.eml

rm -rf .arbeit test-ca.srl zwischen.srl fremd-ca.srl ./*.srl
# Schlüssel der Stellen werden nicht gebraucht und nicht aufbewahrt.
rm -f test-ca.key fremd-ca.key zwischen.key anna.key eve.key anna-schwach.key anna-server.key test.key empfaenger.key
ls
