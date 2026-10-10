#!/usr/bin/env bash
# One-shot certificate setup for the "secure" profile. Runs as root inside the
# Elasticsearch image (compose service secure-setup); idempotent.
#
#   config/certs/ca/        CA from elasticsearch-certutil (ca.p12 + PEM ca.crt)
#   config/certs/strict/    node cert signed with keytool: 397 days, SAN, and
#                           extendedKeyUsage=serverAuth,clientAuth
#   config/certs/certutil/  node cert from elasticsearch-certutil's defaults:
#                           1095 days, SAN, no extendedKeyUsage
#
# Why two: macOS's TLS policy (Security.framework) rejects server certificates
# valid for more than 825 days or without the serverAuth EKU, even under a
# trusted custom root. curl and Python don't apply that policy, but esticli's
# reqwest/rustls platform verifier does. "strict" (the default) is what macOS
# accepts; ES_SECURE_CERT=certutil serves the certutil-default certificate many
# real clusters run, to reproduce that failure. The CA is the same for both, so
# docker/certs/ca.crt works with either.
set -euo pipefail

ES_HOME=/usr/share/elasticsearch
CERTS="$ES_HOME/config/certs"
KEYTOOL="$ES_HOME/jdk/bin/keytool"
JAVA="$ES_HOME/jdk/bin/java"
# Only protects the intermediate PKCS#12 files inside the docker volume.
PW=esticli-setup
SAN_DNS=(es-secure localhost host.docker.internal)
SAN_IP=(127.0.0.1 ::1)

mkdir -p "$CERTS"
# Relative paths below are under $CERTS; certutil resolves relative paths
# against ES_HOME instead, so it is always given absolute ones.
cd "$CERTS"

if [ ! -f ca/ca.p12 ]; then
  echo "creating CA (elasticsearch-certutil)"
  rm -rf ca strict certutil  # certificates signed by a previous CA are useless now
  mkdir -p ca
  "$ES_HOME/bin/elasticsearch-certutil" ca --silent --days 3650 --out "$CERTS/ca/ca.p12" --pass "$PW"
fi
"$KEYTOOL" -exportcert -rfc -alias ca -keystore ca/ca.p12 -storepass "$PW" -file ca/ca.crt 2>/dev/null

if [ ! -f strict/node.crt ]; then
  echo "creating node certificate 'strict' (keytool: 397 days, EKU serverAuth)"
  rm -rf strict && mkdir strict
  san="$(printf 'dns:%s,' "${SAN_DNS[@]}")$(printf 'ip:%s,' "${SAN_IP[@]}")"
  san="${san%,}"
  "$KEYTOOL" -genkeypair -alias node -keyalg RSA -keysize 2048 -dname CN=es-secure -validity 397 \
    -keystore strict/node.p12 -storetype PKCS12 -storepass "$PW" 2>/dev/null
  "$KEYTOOL" -certreq -alias node -keystore strict/node.p12 -storepass "$PW" -file strict/node.csr
  "$KEYTOOL" -gencert -alias ca -keystore ca/ca.p12 -storepass "$PW" -rfc -validity 397 \
    -infile strict/node.csr -outfile strict/node.crt \
    -ext "SAN=$san" -ext EKU=serverAuth,clientAuth -ext KU=digitalSignature,keyEncipherment
  # keytool can't write a private key out; a few lines of Java can.
  "$JAVA" /setup/PemKey.java strict/node.p12 "$PW" node > strict/node.key
  rm strict/node.csr strict/node.p12
fi

if [ ! -f certutil/node.crt ]; then
  echo "creating node certificate 'certutil' (elasticsearch-certutil defaults: 1095 days, no EKU)"
  rm -rf certutil /tmp/certutil.zip /tmp/certutil
  dns="$(IFS=,; echo "${SAN_DNS[*]}")"
  ip="$(IFS=,; echo "${SAN_IP[*]}")"
  "$ES_HOME/bin/elasticsearch-certutil" cert --silent --pem --name node --dns "$dns" --ip "$ip" \
    --ca "$CERTS/ca/ca.p12" --ca-pass "$PW" --out /tmp/certutil.zip
  mkdir -p /tmp/certutil && (cd /tmp/certutil && unzip -oq /tmp/certutil.zip)
  mkdir certutil
  mv /tmp/certutil/node/node.crt /tmp/certutil/node/node.key certutil/
fi

chown -R 1000:0 "$CERTS"
find "$CERTS" -type d -exec chmod 750 {} +
find "$CERTS" -type f -exec chmod 640 {} +

cp ca/ca.crt /export/ca.crt
chmod 644 /export/ca.crt
echo "CA exported to docker/certs/ca.crt; node certificates: strict (default), certutil"
