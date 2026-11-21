#!/usr/bin/env bash
# Copies scripts/bench to the benchmark host and runs it there. Nothing runs locally.
#
#   scripts/bench/remote.sh setup                      # once: tools, images, certs
#   RUSTSTROM_BIN=/path/on/host scripts/bench/remote.sh run [run.sh options]
#   scripts/bench/remote.sh fetch RUN_ID [DEST]        # copy a run directory back
#
# BENCH_HOST (default root@172.17.4.60) and BENCH_ROOT (default /data/ruststrom/bench) select
# the host and its private directory.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BENCH_HOST="${BENCH_HOST:-root@172.17.4.60}"
BENCH_ROOT="${BENCH_ROOT:-/data/ruststrom/bench}"
REV="$(git -C "$HERE" describe --always --dirty 2>/dev/null || echo unknown)"

sync() {
    ssh "$BENCH_HOST" "mkdir -p '$BENCH_ROOT/harness'"
    rsync -a --delete --exclude __pycache__ "$HERE/" "$BENCH_HOST:$BENCH_ROOT/harness/"
}

cmd="${1:-}"
shift || true
case $cmd in
setup)
    sync
    ssh "$BENCH_HOST" "BENCH_ROOT='$BENCH_ROOT' '$BENCH_ROOT/harness/setup.sh' $*"
    ;;
run)
    sync
    args=$(printf '%q ' "$@")
    # Forward the documented overrides that are set locally.
    envs=""
    for v in IDLE_CONNS EVENT_RATE LOADGEN_CPUS LB_CPUS BACKEND_CPUS LB_THREADS LOADGEN_THREADS \
        HAPROXY_IMAGE NGINX_IMAGE HEALTH_INTERVAL_S; do
        [[ -n ${!v:-} ]] && envs+="$v=$(printf '%q' "${!v}") "
    done
    ssh "$BENCH_HOST" "BENCH_ROOT='$BENCH_ROOT' HARNESS_REV='$REV' RUSTSTROM_BIN='${RUSTSTROM_BIN:-}' \
        $envs'$BENCH_ROOT/harness/run.sh' $args"
    ;;
fetch)
    run_id=${1:?run id}
    dest=${2:-.}
    rsync -a "$BENCH_HOST:$BENCH_ROOT/runs/$run_id" "$dest/"
    ;;
*)
    sed -n '2,10p' "$0"
    exit 2
    ;;
esac
