#!/usr/bin/env bash
# One-time setup on the benchmark host (roberta).
#
# - builds wrk, wrk2 and h2load inside containers and installs them into $BENCH_ROOT/tools/bin,
#   so the host needs no -devel packages and its system OpenSSL is never touched:
#     * wrk and wrk2 in a Rocky Linux 9 container (same glibc ABI as the host) against a static
#       OpenSSL built from source. They cannot be fully static: wrk loads its embedded Lua
#       module through dlsym, which static musl binaries do not support.
#     * h2load fully static (musl) in an Alpine container.
#   All three end up with OpenSSL $OPENSSL_VERSION-ish; the exact versions go to tools/VERSIONS.
# - pulls the HAProxy and nginx images
# - generates the TLS certificate (ECDSA P-256, SAN localhost) and the static response bodies
#
# Idempotent: existing tools/certs/bodies are kept. Use --rebuild-tools to force a rebuild.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/common.sh
source "$HERE/lib/common.sh"

REBUILD=0
[[ "${1:-}" == "--rebuild-tools" ]] && REBUILD=1

# Pinned versions: change deliberately and record in the report.
WRK_REF="4.2.0"
WRK2_REF="44a94c17d8e6a0bac8559b53da76848e430cb7a7"
NGHTTP2_VERSION="1.64.0"
OPENSSL_VERSION="3.3.7" # matches the Alpine 3.20 OpenSSL used by h2load
ROCKY_IMAGE="docker.io/rockylinux/rockylinux:9.6"
ALPINE_IMAGE="docker.io/library/alpine:3.20"

mkdir -p "$TOOLS_DIR/bin" "$TOOLS_DIR/src" "$STATE_DIR"

# Pull only when missing (Docker Hub rate-limits anonymous pulls). The digest of the image
# actually used is recorded in every report.
pull() { podman image exists "$1" || podman pull -q "$1" >/dev/null; }

build_wrk() {
    log "building wrk $WRK_REF and wrk2 $WRK2_REF with static OpenSSL $OPENSSL_VERSION in $ROCKY_IMAGE"
    pull "$ROCKY_IMAGE"
    # Pinned to the load generator cores so the build stays off the LB and backend cores.
    podman run --rm --name "$NAME_PREFIX-build-wrk" --cpuset-cpus "$LOADGEN_CPUS" \
        --security-opt label=disable -v "$TOOLS_DIR:/tools" -v "$HERE/patches:/patches:ro" \
        -e WRK_REF="$WRK_REF" -e WRK2_REF="$WRK2_REF" -e OPENSSL_VERSION="$OPENSSL_VERSION" \
        "$ROCKY_IMAGE" bash -euxo pipefail -c '
dnf -q -y install gcc make perl-core git zlib-devel unzip >/dev/null
JOBS=$(nproc)
mkdir -p /tools/src && cd /tools/src
if [ ! -f "/tools/src/openssl-root/lib64/libssl.a" ] && [ ! -f "/tools/src/openssl-root/lib/libssl.a" ]; then
    rm -rf "openssl-$OPENSSL_VERSION"
    curl -sSfL "https://github.com/openssl/openssl/releases/download/openssl-$OPENSSL_VERSION/openssl-$OPENSSL_VERSION.tar.gz" | tar xz
    cd "openssl-$OPENSSL_VERSION"
    ./Configure --prefix=/tools/src/openssl-root --libdir=lib no-shared no-tests >/dev/null
    make -j"$JOBS" >/dev/null && make install_sw >/dev/null
    cd ..
fi
OSSL=/tools/src/openssl-root

rm -rf wrk && git clone -q https://github.com/wg/wrk.git && cd wrk && git checkout -q "$WRK_REF"
# Without this every reconnect is a resumed TLS handshake (see the patch).
git apply /patches/wrk-no-tls-resumption.patch
make -j"$JOBS" WITH_OPENSSL="$OSSL" >/dev/null
install -m 755 wrk /tools/bin/wrk
cd ..

rm -rf wrk2 && git clone -q https://github.com/giltene/wrk2.git && cd wrk2 && git checkout -q "$WRK2_REF"
# Fixes two wrk2 client bugs that would be blamed on the load balancer (see the patch).
git apply /patches/wrk2-eof-idle.patch
# Point the Makefile at the static OpenSSL; libssl must precede libcrypto on the link line.
sed -i -e "s|^CFLAGS  := .*|& -I$OSSL/include|" \
    -e "s|-lcrypto -lssl|-L$OSSL/lib -lssl -lcrypto|" Makefile
make -j"$JOBS" >/dev/null
install -m 755 wrk /tools/bin/wrk2
'
}

build_h2load() {
    log "building static h2load (nghttp2 $NGHTTP2_VERSION) in $ALPINE_IMAGE"
    pull "$ALPINE_IMAGE"
    podman run --rm --name "$NAME_PREFIX-build-h2load" --cpuset-cpus "$LOADGEN_CPUS" \
        --security-opt label=disable -v "$TOOLS_DIR:/tools" \
        -e NGHTTP2_VERSION="$NGHTTP2_VERSION" \
        "$ALPINE_IMAGE" sh -euxc '
apk add -q build-base linux-headers curl xz pkgconf \
    openssl-dev openssl-libs-static libev-dev zlib-dev zlib-static c-ares-dev c-ares-static
JOBS=$(nproc)
mkdir -p /tools/src && cd /tools/src
rm -rf "nghttp2-$NGHTTP2_VERSION"
curl -sSfL "https://github.com/nghttp2/nghttp2/releases/download/v$NGHTTP2_VERSION/nghttp2-$NGHTTP2_VERSION.tar.xz" | tar xJ
cd "nghttp2-$NGHTTP2_VERSION"
./configure -q --enable-app --disable-shared --enable-static --disable-examples \
    --without-systemd --without-jemalloc --without-libxml2 \
    --without-mruby --without-neverbleed --without-libbpf --without-libngtcp2 --without-libnghttp3 \
    >/dev/null
make -j"$JOBS" -C third-party >/dev/null
make -j"$JOBS" -C lib >/dev/null
# libtool needs -all-static for a fully static binary.
make -j"$JOBS" -C src h2load LDFLAGS="-all-static" >/dev/null
install -m 755 src/h2load /tools/bin/h2load
'
}

openssl_of() {
    local v
    v=$(strings "$1" | awk '/^OpenSSL [0-9]+\.[0-9]+\.[0-9]+/ && !f { print; f = 1 }')
    echo "${v:-OpenSSL unknown}"
}

[[ $REBUILD -eq 1 ]] && rm -f "$TOOLS_DIR/bin/wrk" "$TOOLS_DIR/bin/wrk2" "$TOOLS_DIR/bin/h2load"
if [[ ! -x "$TOOLS_DIR/bin/wrk" || ! -x "$TOOLS_DIR/bin/wrk2" ]]; then build_wrk; fi
if [[ ! -x "$TOOLS_DIR/bin/h2load" ]]; then build_h2load; fi
for t in wrk wrk2 h2load; do
    # Must run on the host (catches glibc/loader mismatches).
    "$TOOLS_DIR/bin/$t" --version >/dev/null 2>&1 || "$TOOLS_DIR/bin/$t" -v >/dev/null 2>&1 ||
        [[ $t == wrk* ]] || die "$t does not run on this host"
done
{
    echo "wrk $WRK_REF + patches/wrk-no-tls-resumption.patch, $(openssl_of "$TOOLS_DIR/bin/wrk") (static), glibc dynamic"
    echo "wrk2 $WRK2_REF + patches/wrk2-eof-idle.patch, $(openssl_of "$TOOLS_DIR/bin/wrk2") (static), glibc dynamic"
    echo "h2load nghttp2 $NGHTTP2_VERSION, $(openssl_of "$TOOLS_DIR/bin/h2load") (fully static, musl)"
} > "$TOOLS_DIR/VERSIONS"

log "images: $HAPROXY_IMAGE, $NGINX_IMAGE"
pull "$HAPROXY_IMAGE"
pull "$NGINX_IMAGE"

# TLS material shared by both load balancers. ECDSA P-256 is what most internet-facing
# deployments use today; the key type is recorded in the report.
if [[ ! -f "$STATE_DIR/certs/localhost.pem" ]]; then
    log "generating ECDSA P-256 certificate for localhost"
    mkdir -p "$STATE_DIR/certs"
    openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -days 3650 \
        -subj "/CN=localhost" -addext "subjectAltName=DNS:localhost" \
        -keyout "$STATE_DIR/certs/localhost.key" -out "$STATE_DIR/certs/localhost.crt" 2>/dev/null
    # HAProxy wants certificate and key in one file.
    cat "$STATE_DIR/certs/localhost.crt" "$STATE_DIR/certs/localhost.key" > "$STATE_DIR/certs/localhost.pem"
fi

# Static bodies served by nginx with sendfile (page cache after the first read).
mkdir -p "$STATE_DIR/www"
make_body() { # name bytes
    if [[ ! -f "$STATE_DIR/www/$1" ]] || [[ $(stat -c %s "$STATE_DIR/www/$1") -ne $2 ]]; then
        head -c "$2" /dev/urandom > "$STATE_DIR/www/$1"
    fi
}
make_body 1k 1024
make_body 100k 102400
make_body 10m 10485760
echo ok > "$STATE_DIR/www/health"

log "setup done; tools: $(tr '\n' ';' < "$TOOLS_DIR/VERSIONS")"
