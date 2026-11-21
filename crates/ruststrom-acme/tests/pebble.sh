#!/usr/bin/env bash
# Starts or stops a Pebble ACME test CA for the ruststrom-acme integration tests.
#
#   tests/pebble.sh start    # start pebble + pebble-challtestsrv (DNS) with podman
#   tests/pebble.sh stop     # stop and remove both containers
#   tests/pebble.sh status   # show containers and probe the directory
#   tests/pebble.sh env      # print the variables the tests read
#
# Then: cargo test -- --include-ignored
#
# Layout (host network, loopback only, ports 23400-23499):
#   127.0.0.1:23400  Pebble ACME API (TLS, cert signed by pebble.minica.pem)
#   127.0.0.1:23401  Pebble management API
#   23402            port Pebble's VA connects to for HTTP-01; the tests serve the
#                    ChallengeStore there
#   127.0.0.1:23453  pebble-challtestsrv DNS: every name resolves to 127.0.0.1
#   127.0.0.1:23455  pebble-challtestsrv management API
#
# PEBBLE_AUTHZREUSE=0 makes every order perform a real HTTP-01 validation.
set -euo pipefail

WORKDIR="${PEBBLE_WORKDIR:-/data/ruststrom/acme/pebble}"
PEBBLE_IMAGE="${PEBBLE_IMAGE:-ghcr.io/letsencrypt/pebble:latest}"
CHALL_IMAGE="${CHALL_IMAGE:-ghcr.io/letsencrypt/pebble-challtestsrv:latest}"
PEBBLE_NAME=ruststrom-acme-pebble
CHALL_NAME=ruststrom-acme-challtestsrv

ACME_PORT=23400
PEBBLE_MGMT_PORT=23401
VA_HTTP_PORT=23402
VA_TLS_PORT=23403
DNS_PORT=23453
CHALL_MGMT_PORT=23455

print_env() {
  echo "export PEBBLE_DIRECTORY_URL=https://127.0.0.1:${ACME_PORT}/dir"
  echo "export PEBBLE_CA_ROOT=${WORKDIR}/pebble.minica.pem"
  echo "export PEBBLE_CHALLENGE_PORT=${VA_HTTP_PORT}"
}

stop() {
  podman rm -f "$PEBBLE_NAME" "$CHALL_NAME" >/dev/null 2>&1 || true
}

start() {
  stop
  mkdir -p "$WORKDIR"

  # Root that signed Pebble's API certificate. Only the tests trust it, via
  # AcmeConfig::ca_root_pem; it is never installed system-wide.
  local cid
  cid=$(podman create "$PEBBLE_IMAGE")
  podman cp "$cid:/test/certs/pebble.minica.pem" "$WORKDIR/pebble.minica.pem"
  podman rm "$cid" >/dev/null

  cat >"$WORKDIR/pebble-config.json" <<EOF
{
  "pebble": {
    "listenAddress": "127.0.0.1:${ACME_PORT}",
    "managementListenAddress": "127.0.0.1:${PEBBLE_MGMT_PORT}",
    "certificate": "/test/certs/localhost/cert.pem",
    "privateKey": "/test/certs/localhost/key.pem",
    "httpPort": ${VA_HTTP_PORT},
    "tlsPort": ${VA_TLS_PORT},
    "ocspResponderURL": "",
    "externalAccountBindingRequired": false,
    "domainBlocklist": ["blocked-domain.example"],
    "retryAfter": { "authz": 1, "order": 1 },
    "keyAlgorithm": "ecdsa",
    "profiles": {
      "default": { "description": "90 days", "validityPeriod": 7776000 }
    }
  }
}
EOF

  podman run -d --name "$CHALL_NAME" --network host "$CHALL_IMAGE" \
    -dnsserver "127.0.0.1:${DNS_PORT}" \
    -management "127.0.0.1:${CHALL_MGMT_PORT}" \
    -defaultIPv4 127.0.0.1 -defaultIPv6 "" \
    -http01 "" -https01 "" -tlsalpn01 "" -doh "" >/dev/null

  podman run -d --name "$PEBBLE_NAME" --network host \
    -v "$WORKDIR/pebble-config.json:/config/pebble-config.json:ro,Z" \
    -e PEBBLE_VA_NOSLEEP=1 \
    -e PEBBLE_WFE_NONCEREJECT=0 \
    -e PEBBLE_AUTHZREUSE=0 \
    "$PEBBLE_IMAGE" -config /config/pebble-config.json \
    -dnsserver "127.0.0.1:${DNS_PORT}" >/dev/null

  for _ in $(seq 1 30); do
    if curl -fsS --cacert "$WORKDIR/pebble.minica.pem" \
      "https://127.0.0.1:${ACME_PORT}/dir" >/dev/null 2>&1; then
      echo "pebble is up"
      print_env
      return 0
    fi
    sleep 1
  done
  echo "pebble did not come up; logs:" >&2
  podman logs "$PEBBLE_NAME" >&2 || true
  podman logs "$CHALL_NAME" >&2 || true
  return 1
}

status() {
  podman ps -a --filter "name=ruststrom-acme-" --format '{{.Names}} {{.Status}}'
  curl -fsS --cacert "$WORKDIR/pebble.minica.pem" "https://127.0.0.1:${ACME_PORT}/dir"
}

case "${1:-}" in
  start) start ;;
  stop) stop ;;
  status) status ;;
  env) print_env ;;
  *)
    echo "usage: $0 start|stop|status|env" >&2
    exit 2
    ;;
esac
