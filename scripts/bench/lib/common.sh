# Shared settings and helpers. Sourced by setup.sh, run.sh and sysctl-tune.sh.
# Every value can be overridden from the environment.

BENCH_ROOT="${BENCH_ROOT:-/data/ruststrom/bench}"
TOOLS_DIR="${TOOLS_DIR:-$BENCH_ROOT/tools}"
STATE_DIR="${STATE_DIR:-$BENCH_ROOT/state}"
RUNS_DIR="${RUNS_DIR:-$BENCH_ROOT/runs}"

# Core layout on one 16 vCPU box (see README: limitations).
LOADGEN_CPUS="${LOADGEN_CPUS:-0-5}"
LB_CPUS="${LB_CPUS:-6-9}"
BACKEND_CPUS="${BACKEND_CPUS:-10-15}"
LB_THREADS="${LB_THREADS:-4}"
LOADGEN_THREADS="${LOADGEN_THREADS:-6}"

# All ports stay inside 22000-22999.
PORT_HTTP="${PORT_HTTP:-22080}"
PORT_HTTPS="${PORT_HTTPS:-22443}"
PORT_RS_METRICS="${PORT_RS_METRICS:-22090}"
PORT_HAPROXY_STATS="${PORT_HAPROXY_STATS:-22091}"
PORT_HAPROXY_CLI="${PORT_HAPROXY_CLI:-22092}"
BACKEND_PORTS=(${BACKEND_PORTS:-22101 22102 22103 22104})
NGINX_WORKERS_PER_BACKEND="${NGINX_WORKERS_PER_BACKEND:-2}"

HAPROXY_IMAGE="${HAPROXY_IMAGE:-docker.io/library/haproxy:3.2}"
NGINX_IMAGE="${NGINX_IMAGE:-docker.io/library/nginx:1.27-alpine}"

# Hostname used by the clients: TLS needs a name for SNI (RustStrom v1 rejects handshakes
# without SNI). On roberta "localhost" resolves to ::1 first, so all client traffic goes over
# IPv6 loopback for both load balancers; both bind dual-stack.
TARGET_HOST="${TARGET_HOST:-localhost}"

# Every container and file we create carries this prefix; cleanup only touches these.
NAME_PREFIX="rsbench"

WRK="$TOOLS_DIR/bin/wrk"
WRK2="$TOOLS_DIR/bin/wrk2"
H2LOAD="$TOOLS_DIR/bin/h2load"

log() { printf '[%(%H:%M:%S)T] %s\n' -1 "$*" >&2; }
die() { log "ERROR: $*"; exit 1; }

# Runs a load generator pinned to the load generator cores.
gen() { taskset -c "$LOADGEN_CPUS" "$@"; }

# Prints the PID plus all descendant PIDs of $1 (one per line).
pid_tree() {
    local root=$1 kids k
    echo "$root"
    kids=$(cat /proc/"$root"/task/*/children 2>/dev/null || true)
    for k in $kids; do pid_tree "$k"; done
}

port_in_use() { ss -Htln "sport = :$1" | grep -q .; }

# Waits until $1 (URL) answers with HTTP 200 or the timeout (seconds) expires.
wait_http_ok() {
    local url=$1 timeout=${2:-20} i
    for ((i = 0; i < timeout * 10; i++)); do
        if [[ $(curl -sk -o /dev/null -w '%{http_code}' --max-time 1 "$url" 2>/dev/null) == 200 ]]; then
            return 0
        fi
        sleep 0.1
    done
    return 1
}
