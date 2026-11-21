#!/usr/bin/env bash
# HAProxy vs RustStrom benchmark. Runs on the benchmark host (roberta) as root.
#
#   RUSTSTROM_BIN=/path/to/rust-strom scripts/bench/run.sh [options]
#
# Options:
#   --force            run even if the 1-minute load average is >= 1.0 (results marked NOT VALID)
#   --smoke            short runs and low concurrency to validate the harness (marked NOT VALID)
#   --lb LIST          comma separated: haproxy,ruststrom (default both)
#   --scenarios LIST   comma separated: 1,2,3,4,5,6 (default all)
#   --duration SEC     measured seconds per step (default 30, smoke 3)
#   --no-sysctl        do not apply the temporary sysctl tuning
#   --label TEXT       free text stored in the report (e.g. git commit of the binary)
#
# Output: $RUNS_DIR/<timestamp>[-smoke]/ with raw tool outputs, CPU/RSS samples, logs, the
# generated configs, env.txt and report.md.
set -euo pipefail

HARNESS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/common.sh
source "$HARNESS_DIR/lib/common.sh"

FORCE=0 SMOKE=0 APPLY_SYSCTL=1 LABEL="" DURATION=""
LBS="haproxy,ruststrom"
SCENARIOS="1,2,3,4,5,6"
while [[ $# -gt 0 ]]; do
    case $1 in
    --force) FORCE=1 ;;
    --smoke) SMOKE=1 ;;
    --lb) LBS=$2; shift ;;
    --scenarios) SCENARIOS=$2; shift ;;
    --duration) DURATION=$2; shift ;;
    --no-sysctl) APPLY_SYSCTL=0 ;;
    --label) LABEL=$2; shift ;;
    -h | --help) sed -n '2,16p' "$0"; exit 0 ;;
    *) die "unknown option $1 (see --help)" ;;
    esac
    shift
done
IFS=, read -r -a LB_LIST <<<"$LBS"
IFS=, read -r -a SCEN_LIST <<<"$SCENARIOS"
has_scenario() { [[ ",$SCENARIOS," == *",$1,"* ]]; }

# ------------------------------------------------------------------ parameters
if [[ $SMOKE -eq 1 ]]; then
    DURATION=${DURATION:-3}
    WARMUP=1
    CONNS=32           # keep-alive connections for max-RPS and latency steps
    TLS_CONNS=16       # concurrent clients in the handshake test
    H2_CONNS=8 H2_STREAMS=10
    LARGE_CONNS=8
    IDLE_CONNS=${IDLE_CONNS:-1000}
    IDLE_RATE=200
    EVENT_DURATION=8   # failover/reload steps need room for the events
    EVENT_RATE=1000
    RELOADS=3
else
    DURATION=${DURATION:-30}
    WARMUP=5
    CONNS=256
    TLS_CONNS=120
    H2_CONNS=60 H2_STREAMS=10
    LARGE_CONNS=24
    IDLE_CONNS=${IDLE_CONNS:-10000}
    IDLE_RATE=1000
    EVENT_DURATION=30
    EVENT_RATE=${EVENT_RATE:-20000}
    RELOADS=5
fi
LATENCY_FRACTIONS=(50 80)

# ------------------------------------------------------------------ preflight
[[ $EUID -eq 0 ]] || die "run as root (taskset, sysctl, podman with host network)"
for t in "$WRK" "$WRK2" "$H2LOAD"; do [[ -x $t ]] || die "$t missing: run $HARNESS_DIR/setup.sh first"; done
[[ -f $STATE_DIR/certs/localhost.pem ]] || die "certificates missing: run $HARNESS_DIR/setup.sh first"
for lb in "${LB_LIST[@]}"; do
    case $lb in
    haproxy) ;;
    ruststrom)
        [[ -n ${RUSTSTROM_BIN:-} ]] || die "set RUSTSTROM_BIN to the rust-strom binary to test"
        [[ -x $RUSTSTROM_BIN ]] || die "RUSTSTROM_BIN=$RUSTSTROM_BIN is not executable"
        ;;
    *) die "unknown load balancer '$lb'" ;;
    esac
done

LOAD1=$(cut -d' ' -f1 /proc/loadavg)
VALID=1
INVALID_REASONS=()
if awk -v l="$LOAD1" 'BEGIN { exit !(l >= 1.0) }'; then
    if [[ $FORCE -eq 0 ]]; then
        die "1-minute load average is $LOAD1 (>= 1.0): the host is busy and numbers would be noise. Wait, or use --force for a smoke test."
    fi
    VALID=0
    INVALID_REASONS+=("load average $LOAD1 >= 1.0 at start (--force)")
fi
[[ $SMOKE -eq 1 ]] && { VALID=0; INVALID_REASONS+=("--smoke: short runs, low concurrency"); }
[[ $FORCE -eq 1 && $VALID -eq 1 ]] && INVALID_REASONS+=("--force given but load was fine")

for p in "$PORT_HTTP" "$PORT_HTTPS" "$PORT_RS_METRICS" "$PORT_HAPROXY_STATS" "$PORT_HAPROXY_CLI" "${BACKEND_PORTS[@]}"; do
    port_in_use "$p" && die "port $p is in use by someone else; not touching it"
done
podman ps -a --format '{{.Names}}' | grep -q "^$NAME_PREFIX-\(haproxy\|be[0-9]\)$" &&
    die "leftover $NAME_PREFIX containers exist (podman ps -a | grep $NAME_PREFIX); remove them first"

ulimit -n 1048576

# The suffix must not be a failing command substitution: under `set -e` a
# non-smoke run would exit here silently.
RUN_SUFFIX=""
if [[ $SMOKE -eq 1 ]]; then RUN_SUFFIX="-smoke"; fi
RUN_ID="$(date +%Y%m%d-%H%M%S)${RUN_SUFFIX}"
RUN_DIR="$RUNS_DIR/$RUN_ID"
mkdir -p "$RUN_DIR/conf" "$RUN_DIR/logs"
log "run directory: $RUN_DIR"
# shellcheck source=lib/lb.sh
source "$HARNESS_DIR/lib/lb.sh"

SAMPLER_PID=""
SYSCTL_APPLIED=0
CURRENT_LB=""
cleanup() {
    local rc=$?
    set +e
    [[ -n $SAMPLER_PID ]] && kill "$SAMPLER_PID" 2>/dev/null
    [[ -n $CURRENT_LB ]] && stop_lb "$CURRENT_LB" "$RUN_DIR/logs/$CURRENT_LB.log"
    stop_backends
    if [[ $SYSCTL_APPLIED -eq 1 ]]; then
        "$HARNESS_DIR/sysctl-tune.sh" restore 2>&1 | tee -a "$RUN_DIR/logs/sysctl.log" >&2
    fi
    [[ $rc -ne 0 ]] && log "run failed (exit $rc); partial results in $RUN_DIR"
    exit $rc
}
trap cleanup EXIT
trap 'exit 130' INT TERM

# ------------------------------------------------------------------ environment
if [[ $APPLY_SYSCTL -eq 1 ]]; then
    "$HARNESS_DIR/sysctl-tune.sh" show > "$RUN_DIR/logs/sysctl-before.txt"
    "$HARNESS_DIR/sysctl-tune.sh" apply 2>&1 | tee "$RUN_DIR/logs/sysctl.log" >&2
    SYSCTL_APPLIED=1
fi

{
    echo "run_id=$RUN_ID"
    echo "valid=$VALID"
    echo "invalid_reasons=$(IFS='; '; echo "${INVALID_REASONS[*]:-}")"
    echo "label=$LABEL"
    echo "harness_rev=${HARNESS_REV:-unknown}"
    echo "date=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "host=$(hostname)"
    echo "kernel=$(uname -r)"
    echo "os=$(. /etc/os-release && echo "$PRETTY_NAME")"
    echo "cpu_model=$(lscpu | sed -n 's/^Model name: *//p' | head -1)"
    echo "cpus=$(nproc)"
    echo "numa=$(lscpu | sed -n 's/^NUMA node[0-9] CPU(s): *//p' | paste -sd' ' | sed 's/ / | /g')"
    echo "mem_total=$(free -g | awk '/^Mem:/ {print $2 " GiB"}')"
    echo "loadavg_start=$(cut -d' ' -f1-3 /proc/loadavg)"
    echo "cpus_loadgen=$LOADGEN_CPUS"
    echo "cpus_lb=$LB_CPUS"
    echo "cpus_backends=$BACKEND_CPUS"
    echo "lb_threads=$LB_THREADS"
    echo "loadgen_threads=$LOADGEN_THREADS"
    echo "backends=${#BACKEND_PORTS[@]} x nginx ($NGINX_WORKERS_PER_BACKEND workers each)"
    echo "target=$TARGET_HOST -> $(getent ahosts "$TARGET_HOST" | awk '/STREAM/ {print $1; exit}'), LB listen $LISTEN_ADDR"
    echo "duration_s=$DURATION"
    echo "warmup_s=$WARMUP"
    echo "event_duration_s=$EVENT_DURATION"
    echo "conns=$CONNS tls_conns=$TLS_CONNS h2_conns=$H2_CONNS h2_streams=$H2_STREAMS large_conns=$LARGE_CONNS idle_conns=$IDLE_CONNS idle_rate=$IDLE_RATE event_rate=$EVENT_RATE reloads=$RELOADS"
    echo "sysctl_tuning=$([[ $APPLY_SYSCTL -eq 1 ]] && echo applied || echo not applied)"
    "$HARNESS_DIR/sysctl-tune.sh" show | sed 's/^/sysctl: /'
    echo "haproxy_image=$HAPROXY_IMAGE@$(podman image inspect -f '{{.Digest}}' "$HAPROXY_IMAGE" 2>/dev/null)"
    echo "nginx_image=$NGINX_IMAGE@$(podman image inspect -f '{{.Digest}}' "$NGINX_IMAGE" 2>/dev/null)"
    for lb in "${LB_LIST[@]}"; do echo "version_$lb=$(lb_version "$lb")"; done
    if [[ -n ${RUSTSTROM_BIN:-} ]]; then
        echo "ruststrom_bin=$RUSTSTROM_BIN"
        echo "ruststrom_sha256=$(sha256sum "$RUSTSTROM_BIN" | cut -c1-16)"
    fi
    echo "podman=$(podman --version)"
    sed 's/^/tool: /' "$TOOLS_DIR/VERSIONS"
    echo "cert=$(openssl x509 -in "$STATE_DIR/certs/localhost.crt" -noout -text | grep -m1 'Public Key Algorithm' | sed 's/.*: //') $(openssl x509 -in "$STATE_DIR/certs/localhost.crt" -noout -text | grep -m1 -o 'NIST CURVE: .*\|Public-Key: ([0-9]* bit)')"
} > "$RUN_DIR/env.txt"

# ------------------------------------------------------------------ measurement helpers

# measure SCENARIO LB STEP CMD...: runs CMD pinned to the load generator cores, writes its
# output to $RUN_DIR/SCENARIO/LB-STEP.txt and samples CPU/RSS of the LB, backends and the
# load generator into LB-STEP.csv. If $DURING names a function, it runs while CMD runs.
measure() {
    local scen=$1 lb=$2 step=$3
    shift 3
    local dir="$RUN_DIR/$scen" out gen_pid rc=0
    mkdir -p "$dir"
    out="$dir/$lb-$step"
    {
        echo "cmd=$*"
        echo "loadavg_before=$(cut -d' ' -f1-3 /proc/loadavg)"
    } > "$out.meta"
    log "$scen/$lb/$step: $*"
    taskset -c "$LOADGEN_CPUS" "$@" > "$out.txt" 2>&1 &
    gen_pid=$!
    taskset -c "$LOADGEN_CPUS" python3 "$HARNESS_DIR/lib/sample.py" --out "$out.csv" --interval 0.5 \
        --group "lb=$(lb_pid "$lb")" --group "backends=$(backend_pids)" --group "loadgen=$gen_pid" &
    SAMPLER_PID=$!
    if [[ -n ${DURING:-} ]]; then "$DURING" "$lb" "$out"; fi
    wait "$gen_pid" || rc=$?
    kill "$SAMPLER_PID" 2>/dev/null; wait "$SAMPLER_PID" 2>/dev/null || true
    SAMPLER_PID=""
    {
        echo "exit_code=$rc"
        echo "loadavg_after=$(cut -d' ' -f1-3 /proc/loadavg)"
    } >> "$out.meta"
    [[ $rc -eq 0 ]] || log "WARNING: $scen/$lb/$step exited with $rc (see $out.txt)"
}

# Short unmeasured load so connection pools, caches and the allocator are warm.
warmup() { taskset -c "$LOADGEN_CPUS" "$WRK" -t "$LOADGEN_THREADS" -c "$CONNS" -d "${WARMUP}s" "$1" >/dev/null 2>&1 || true; }

# Samples the idle LB for 2 s (memory baseline before a connection-heavy step).
idle_sample() { # SCENARIO LB STEP
    mkdir -p "$RUN_DIR/$1"
    taskset -c "$LOADGEN_CPUS" python3 "$HARNESS_DIR/lib/sample.py" --out "$RUN_DIR/$1/$2-$3.csv" \
        --interval 0.5 --group "lb=$(lb_pid "$2")" &
    SAMPLER_PID=$!
    sleep 2
    kill "$SAMPLER_PID"; wait "$SAMPLER_PID" 2>/dev/null || true
    SAMPLER_PID=""
}

with_lb() { # LB FUNCTION: start a fresh LB process, run FUNCTION LB, stop it
    local lb=$1 fn=$2
    CURRENT_LB=$lb
    start_lb "$lb" "$RUN_DIR/logs/$lb.log"
    "$fn" "$lb"
    stop_lb "$lb" "$RUN_DIR/logs/$lb.log"
    CURRENT_LB=""
}

HTTP_URL="http://$TARGET_HOST:$PORT_HTTP"
HTTPS_URL="https://$TARGET_HOST:$PORT_HTTPS"
T=$LOADGEN_THREADS

# ------------------------------------------------------------------ scenarios

# 1. Plain HTTP keep-alive, 1 KB: max RPS (wrk), then latency at fixed rates (wrk2).
s1_max() {
    warmup "$HTTP_URL/1k"
    measure s1_http_1k "$1" max "$WRK" -t "$T" -c "$CONNS" -d "${DURATION}s" --latency "$HTTP_URL/1k"
}
s1_latency() {
    local f
    warmup "$HTTP_URL/1k"
    for f in "${LATENCY_FRACTIONS[@]}"; do
        measure s1_http_1k "$1" "rate$f" "$WRK2" -t "$T" -c "$CONNS" -d "${DURATION}s" \
            -R "${S1_RATE[$f]}" --latency "$HTTP_URL/1k"
    done
}

# 2. TLS 1.3 full handshakes per second: every request on a new connection
#    (Connection: close). wrk never calls SSL_set_session, so there is no session resumption
#    (h2load does resume, which is why it is not used here). For HAProxy the harness reads its
#    TLS counters mid-run to prove that every handshake was a full one.
haproxy_tls_counters() { # LB OUT (DURING hook)
    [[ $1 == haproxy ]] || return 0
    sleep "$(awk -v d="$DURATION" 'BEGIN { printf "%.1f", d / 2 }')"
    {
        echo "haproxy_mid_SslRate=$(haproxy_info SslRate)"
        echo "haproxy_mid_SslFrontendKeyRate=$(haproxy_info SslFrontendKeyRate)"
        echo "haproxy_mid_SslFrontendSessionReuse_pct=$(haproxy_info SslFrontendSessionReuse_pct)"
    } >> "$2.meta"
}
s2() {
    local before
    tls_probe "$1"
    [[ $1 == haproxy ]] && before=$(haproxy_info CumSslConns)
    DURING=haproxy_tls_counters measure s2_tls_handshake "$1" handshake "$WRK" -t "$T" \
        -c "$TLS_CONNS" -d "${DURATION}s" -H "Connection: close" "$HTTPS_URL/1k"
    if [[ $1 == haproxy ]]; then
        echo "haproxy_new_tls_sessions=$(($(haproxy_info CumSslConns) - before))" \
            >> "$RUN_DIR/s2_tls_handshake/haproxy-handshake.meta"
    fi
    # Response headers to a "Connection: close" request (explains read errors, if any).
    curl -sk --http1.1 -o /dev/null -D - -H "Connection: close" "$HTTPS_URL/1k" | tr -d '\r' \
        > "$RUN_DIR/s2_tls_handshake/$1-close-headers.txt" || true
}

# 3. HTTPS with HTTP/2 (ALPN), keep-alive, 1 KB.
s3() {
    measure s3_h2 "$1" h2 "$H2LOAD" -t "$T" -c "$H2_CONNS" -m "$H2_STREAMS" -D "$DURATION" \
        --warm-up-time "$WARMUP" "$HTTPS_URL/1k"
}

# 4. Large responses: 100 KB and 10 MB, plain HTTP keep-alive.
s4() {
    warmup "$HTTP_URL/100k"
    measure s4_large "$1" 100k "$WRK" -t "$T" -c "$LARGE_CONNS" -d "${DURATION}s" --latency "$HTTP_URL/100k"
    measure s4_large "$1" 10m "$WRK" -t "$T" -c "$LARGE_CONNS" -d "${DURATION}s" --latency "$HTTP_URL/10m"
}

# 5. Many mostly-idle keep-alive connections at a low constant rate: memory per connection.
# wrk2 opens a thread's connections 5 ms apart, so 10k connections over 6 threads need ~8.4 s
# before all are open. The step is extended by that ramp so every connection is held for the
# full DURATION, and the established count is taken after the ramp.
S5_RAMP=$(((IDLE_CONNS / LOADGEN_THREADS * 5 + 999) / 1000 + 1))
count_client_conns() { # LB OUT (DURING hook): proves the connections were really open
    sleep "$(awk -v r="$S5_RAMP" -v d="$DURATION" 'BEGIN { printf "%.1f", r + d * 0.7 }')"
    echo "established_mid_run=$(ss -Htn state established "( sport = :$PORT_HTTP )" | wc -l)" >> "$2.meta"
}
s5() {
    idle_sample s5_idle_conns "$1" baseline
    DURING=count_client_conns measure s5_idle_conns "$1" conns "$WRK2" -t "$T" -c "$IDLE_CONNS" \
        -d "$((S5_RAMP + DURATION))s" -R "$IDLE_RATE" --latency "$HTTP_URL/1k"
}

# 6a. One backend killed (SIGKILL) a third into the run, restarted at two thirds.
failover_events() { # LB OUT
    local third=$((EVENT_DURATION / 3)) victim=$((${#BACKEND_PORTS[@]} - 1))
    sleep "$third"
    kill_backend "$victim"
    echo "backend_killed_at_s=$third port=${BACKEND_PORTS[$victim]}" >> "$2.meta"
    sleep "$third"
    revive_backend "$victim"
    echo "backend_restarted_at_s=$((2 * third))" >> "$2.meta"
}
s6_failover() {
    warmup "$HTTP_URL/1k"
    DURING=failover_events measure s6_failover "$1" failover "$WRK2" -t "$T" -c "$CONNS" \
        -d "${EVENT_DURATION}s" -R "$EVENT_RATE" --latency "$HTTP_URL/1k"
    wait_http_ok "http://127.0.0.1:${BACKEND_PORTS[-1]}/health" 20 || die "killed backend did not come back"
}

# 6b. Configuration reloads under load.
reload_events() { # LB OUT
    local gap i
    gap=$(awk -v d="$EVENT_DURATION" -v n="$RELOADS" 'BEGIN { printf "%.2f", d / (n + 1) }')
    for ((i = 1; i <= RELOADS; i++)); do
        sleep "$gap"
        reload_lb "$1"
    done
    echo "reloads_requested=$RELOADS" >> "$2.meta"
}
s6_reload() {
    local logf="$RUN_DIR/logs/$1.log" before after
    warmup "$HTTP_URL/1k"
    before=$(reload_count "$1" "$logf")
    DURING=reload_events measure s6_reload "$1" reload "$WRK2" -t "$T" -c "$CONNS" \
        -d "${EVENT_DURATION}s" -R "$EVENT_RATE" --latency "$HTTP_URL/1k"
    sleep 2
    after=$(reload_count "$1" "$logf")
    echo "reloads_applied=$((after - before))" >> "$RUN_DIR/s6_reload/$1-reload.meta"
}
reload_count() { # LB LOGFILE: number of successful reloads seen so far
    case $1 in
    haproxy) podman logs "$HAPROXY_CTR" 2>&1 | grep -c "Loading success" | awk '{print $1 - 1}' ;; # minus the initial load
    ruststrom) grep -c "Reloaded configuration" "$2" || true ;;
    esac
}

# Applied TLS parameters, recorded once per LB (proves TLS 1.3 / h2 were negotiated).
tls_probe() {
    {
        echo "== $1"
        echo | openssl s_client -connect "$TARGET_HOST:$PORT_HTTPS" -servername "$TARGET_HOST" \
            -alpn h2,http/1.1 2>/dev/null | grep -E '^(New|Protocol|ALPN|Server Temp Key)|Cipher is|Peer signature type' | sort -u
    } >> "$RUN_DIR/logs/tls-probe.txt"
}

# ------------------------------------------------------------------ main
start_backends

declare -A S1_RATE
if has_scenario 1; then
    for lb in "${LB_LIST[@]}"; do with_lb "$lb" s1_max; done
    # Latency is compared at the same offered load for both: a fraction of the lower max.
    min_max=$(for lb in "${LB_LIST[@]}"; do
        sed -n 's/^Requests\/sec: *\([0-9.]*\).*/\1/p' "$RUN_DIR/s1_http_1k/$lb-max.txt"
    done | sort -g | head -1)
    [[ -n $min_max ]] || die "could not parse max RPS from scenario 1"
    for f in "${LATENCY_FRACTIONS[@]}"; do S1_RATE[$f]=$(awk -v m="$min_max" -v f="$f" 'BEGIN { printf "%d", m * f / 100 }'); done
    echo "s1_reference_max_rps=$min_max" >> "$RUN_DIR/env.txt"
    for lb in "${LB_LIST[@]}"; do with_lb "$lb" s1_latency; done
fi
if has_scenario 2; then for lb in "${LB_LIST[@]}"; do with_lb "$lb" s2; done; fi
if has_scenario 3; then for lb in "${LB_LIST[@]}"; do with_lb "$lb" s3; done; fi
if has_scenario 4; then for lb in "${LB_LIST[@]}"; do with_lb "$lb" s4; done; fi
if has_scenario 5; then for lb in "${LB_LIST[@]}"; do with_lb "$lb" s5; done; fi
if has_scenario 6; then
    for lb in "${LB_LIST[@]}"; do with_lb "$lb" s6_failover; done
    for lb in "${LB_LIST[@]}"; do with_lb "$lb" s6_reload; done
fi

echo "loadavg_end=$(cut -d' ' -f1-3 /proc/loadavg)" >> "$RUN_DIR/env.txt"
python3 "$HARNESS_DIR/report.py" "$RUN_DIR" > "$RUN_DIR/report.md"
log "report: $RUN_DIR/report.md"
