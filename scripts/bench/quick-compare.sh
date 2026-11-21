#!/bin/bash
# Quick plain-HTTP/1.1 1 KB throughput comparison on the benchmark host (run as root on roberta).
# Same pinning as run.sh: load generator 0-5, LB 6-9, 4 nginx backends on 10-15.
#
# Usage: quick-compare.sh [rounds] NAME=BINARY:CONFIG ...  (HAProxy is always included)
#   quick-compare.sh 2 v1=/data/ruststrom/target/release/rust-strom:/data/ruststrom/prof/v1.toml \
#                      new=/path/rust-strom:/path/config.toml
# The configs must listen on [::1]:22080 and use backends 127.0.0.1:22101-22104.
# This is a development gate, not a report: use run.sh for published numbers.
set -euo pipefail
BENCH_ROOT=${BENCH_ROOT:-/data/ruststrom/bench}
WRK=$BENCH_ROOT/tools/bin/wrk
STATE=$BENCH_ROOT/state
HARNESS=$(cd "$(dirname "$0")" && pwd)
ROUNDS=${1:-2}; shift || true
WORK=$(mktemp -d /tmp/rsbench-quick.XXXXXX)
NGINX_IMAGE=docker.io/library/nginx:1.27-alpine
HAPROXY_IMAGE=docker.io/library/haproxy:3.2

cleanup() {
    for i in 1 2 3 4; do podman rm -f "quick-be$i" >/dev/null 2>&1 || true; done
    podman rm -f quick-haproxy >/dev/null 2>&1 || true
    [[ -n ${LBPID:-} ]] && kill "$LBPID" 2>/dev/null || true
    rm -rf "$WORK"
}
trap cleanup EXIT

for i in 1 2 3 4; do
    port=$((22100 + i))
    sed -e "s/@PORT@/$port/g" -e "s/@WORKERS@/2/g" "$HARNESS/conf/nginx.conf.tmpl" > "$WORK/nginx-$port.conf"
    podman rm -f "quick-be$i" >/dev/null 2>&1 || true
    podman run -d --name "quick-be$i" --network host --cpuset-cpus 10-15 \
        --ulimit nofile=1048576:1048576 --security-opt label=disable \
        -v "$WORK/nginx-$port.conf:/etc/nginx/nginx.conf:ro" -v "$STATE/www:/www:ro" \
        "$NGINX_IMAGE" nginx -g 'daemon off;' >/dev/null
done
sleep 2

# HAProxy config with the same settings as the full harness (no TLS needed here)
latest_run=$(ls -td "$BENCH_ROOT"/runs/*/ | head -1)
cp "$latest_run/conf/haproxy.cfg" "$WORK/haproxy.cfg"

measure() {
    local r
    r=$(taskset -c 0-5 "$WRK" -t6 -c256 -d20s http://localhost:22080/1k 2>&1 | awk '/Requests\/sec/{print $2}')
    echo "${r:-0}"
}

for round in $(seq "$ROUNDS"); do
    podman run -d --name quick-haproxy --network host --cpuset-cpus 6-9 \
        --ulimit nofile=1048576:1048576 --security-opt label=disable --security-opt seccomp=unconfined \
        -v "$WORK/haproxy.cfg:/usr/local/etc/haproxy/haproxy.cfg:ro" -v "$STATE/certs:/certs:ro" \
        "$HAPROXY_IMAGE" >/dev/null
    sleep 3
    echo "round$round haproxy rps=$(measure) load1=$(cut -d' ' -f1 /proc/loadavg)"
    podman rm -f quick-haproxy >/dev/null
    sleep 1
    for spec in "$@"; do
        name=${spec%%=*}; rest=${spec#*=}; bin=${rest%%:*}; conf=${rest#*:}
        TOKIO_WORKER_THREADS=4 taskset -c 6-9 "$bin" --config "$conf" >"$WORK/$name.log" 2>&1 &
        LBPID=$!
        sleep 3
        echo "round$round $name rps=$(measure) load1=$(cut -d' ' -f1 /proc/loadavg)"
        kill "$LBPID" 2>/dev/null || true; wait "$LBPID" 2>/dev/null || true; LBPID=""
        sleep 1
    done
done
