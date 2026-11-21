# Backends and load balancers. Sourced by run.sh after common.sh; expects RUN_DIR.

HEALTH_INTERVAL_S="${HEALTH_INTERVAL_S:-1}"
LISTEN_ADDR="${LISTEN_ADDR:-[::1]}"
HAPROXY_CTR="$NAME_PREFIX-haproxy"
RS_PID=""
RS_GENERATION=0

backend_ctr() { echo "$NAME_PREFIX-be$1"; }

# ---------------------------------------------------------------- backends (nginx)

start_backends() {
    local i port
    mkdir -p "$RUN_DIR/conf"
    for i in "${!BACKEND_PORTS[@]}"; do
        port=${BACKEND_PORTS[$i]}
        sed -e "s/@PORT@/$port/g" -e "s/@WORKERS@/$NGINX_WORKERS_PER_BACKEND/g" \
            "$HARNESS_DIR/conf/nginx.conf.tmpl" > "$RUN_DIR/conf/nginx-$port.conf"
        podman rm -f "$(backend_ctr "$i")" >/dev/null 2>&1 || true
        podman run -d --name "$(backend_ctr "$i")" --network host --cpuset-cpus "$BACKEND_CPUS" \
            --ulimit nofile=1048576:1048576 --security-opt label=disable \
            -v "$RUN_DIR/conf/nginx-$port.conf:/etc/nginx/nginx.conf:ro" \
            -v "$STATE_DIR/www:/www:ro" \
            "$NGINX_IMAGE" nginx -g 'daemon off;' >/dev/null
    done
    for port in "${BACKEND_PORTS[@]}"; do
        wait_http_ok "http://127.0.0.1:$port/1k" 20 || die "backend on port $port did not come up"
    done
    log "backends up on ports ${BACKEND_PORTS[*]} (cpus $BACKEND_CPUS)"
}

stop_backends() {
    local i
    for i in "${!BACKEND_PORTS[@]}"; do podman rm -f -t 2 "$(backend_ctr "$i")" >/dev/null 2>&1 || true; done
}

# Kill one backend hard (SIGKILL, like a crashed host) and bring it back.
kill_backend() { podman kill -s KILL "$(backend_ctr "$1")" >/dev/null; }
revive_backend() { podman start "$(backend_ctr "$1")" >/dev/null; }

backend_pids() {
    local i pids=()
    for i in "${!BACKEND_PORTS[@]}"; do
        pids+=("$(podman inspect -f '{{.State.Pid}}' "$(backend_ctr "$i")")")
    done
    local IFS=,
    echo "${pids[*]}"
}

# ---------------------------------------------------------------- config rendering

render_haproxy_conf() {
    local servers="" i
    for i in "${!BACKEND_PORTS[@]}"; do
        servers+="    server be$((i + 1)) 127.0.0.1:${BACKEND_PORTS[$i]}"$'\n'
    done
    LB_THREADS=$LB_THREADS LB_CPUS=$LB_CPUS LISTEN_ADDR=$LISTEN_ADDR PORT_HTTP=$PORT_HTTP \
        PORT_HTTPS=$PORT_HTTPS PORT_HAPROXY_STATS=$PORT_HAPROXY_STATS PORT_HAPROXY_CLI=$PORT_HAPROXY_CLI CERT_PEM=/certs/localhost.pem \
        HEALTH_INTERVAL_S=$HEALTH_INTERVAL_S BACKEND_SERVERS="${servers%$'\n'}" \
        python3 -c '
import os, re, sys
s = re.sub(r"@([A-Z_]+)@", lambda m: os.environ[m.group(1)], open(sys.argv[1]).read())
open(sys.argv[2], "w").write(s)
' "$HARNESS_DIR/conf/haproxy.cfg.tmpl" "$RUN_DIR/conf/haproxy.cfg"
}

# Writes the RustStrom config atomically (temp file + rename), as config management would.
render_ruststrom_conf() {
    local addrs="" port
    for port in "${BACKEND_PORTS[@]}"; do addrs+="\"127.0.0.1:$port\", "; done
    sed -e "s/@LISTEN_ADDR@/$LISTEN_ADDR/g" -e "s/@PORT_HTTP@/$PORT_HTTP/g" \
        -e "s/@PORT_HTTPS@/$PORT_HTTPS/g" -e "s/@PORT_RS_METRICS@/$PORT_RS_METRICS/g" \
        -e "s/@LB_THREADS@/$LB_THREADS/g" -e "s/@HEALTH_INTERVAL_S@/$HEALTH_INTERVAL_S/g" \
        -e "s|@BACKEND_ADDRESSES@|${addrs%, }|" \
        -e "s|@CERT_CRT@|$STATE_DIR/certs/localhost.crt|" -e "s|@CERT_KEY@|$STATE_DIR/certs/localhost.key|" \
        -e "s/@GENERATION@/$RS_GENERATION/" \
        "$HARNESS_DIR/conf/ruststrom.toml.tmpl" > "$RUN_DIR/conf/.ruststrom.toml.tmp"
    mv "$RUN_DIR/conf/.ruststrom.toml.tmp" "$RUN_DIR/conf/ruststrom.toml"
}

# ---------------------------------------------------------------- load balancers

# start_lb NAME LOGFILE
start_lb() {
    local lb=$1 logf=$2
    case $lb in
    haproxy)
        render_haproxy_conf
        podman rm -f "$HAPROXY_CTR" >/dev/null 2>&1 || true
        # Host network and no seccomp filter, so the container adds no syscall overhead
        # compared with the natively started RustStrom.
        podman run -d --name "$HAPROXY_CTR" --network host --cpuset-cpus "$LB_CPUS" \
            --ulimit nofile=1048576:1048576 --security-opt label=disable \
            --security-opt seccomp=unconfined \
            -v "$RUN_DIR/conf/haproxy.cfg:/usr/local/etc/haproxy/haproxy.cfg:ro" \
            -v "$STATE_DIR/certs:/certs:ro" \
            "$HAPROXY_IMAGE" >/dev/null
        ;;
    ruststrom)
        render_ruststrom_conf
        TOKIO_WORKER_THREADS=$LB_THREADS LOG_LEVEL=info \
            "$RUSTSTROM_BIN" --check --config "$RUN_DIR/conf/ruststrom.toml" >>"$logf" 2>&1 ||
            die "rust-strom --check failed, see $logf"
        (
            ulimit -n 1048576
            # Same thread count as HAProxy nbthread. v2 reads `threads` from the config;
            # v1.1 (tokio::main) honours TOKIO_WORKER_THREADS.
            export TOKIO_WORKER_THREADS=$LB_THREADS LOG_LEVEL=info
            exec taskset -c "$LB_CPUS" "$RUSTSTROM_BIN" --config "$RUN_DIR/conf/ruststrom.toml"
        ) >>"$logf" 2>&1 &
        RS_PID=$!
        ;;
    *) die "unknown load balancer $lb" ;;
    esac
    wait_http_ok "http://$TARGET_HOST:$PORT_HTTP/1k" 30 || { lb_logs "$lb" "$logf"; die "$lb did not come up"; }
    wait_http_ok "https://$TARGET_HOST:$PORT_HTTPS/1k" 10 || { lb_logs "$lb" "$logf"; die "$lb HTTPS did not come up"; }
    # Give health checks time to see every backend as up (rise 2 at 1 s interval).
    sleep $((HEALTH_INTERVAL_S * 3))
}

lb_logs() { # NAME LOGFILE: append container logs to the log file
    [[ $1 == haproxy ]] && podman logs "$HAPROXY_CTR" >>"$2" 2>&1 || true
}

stop_lb() {
    local lb=$1 logf=$2
    case $lb in
    haproxy)
        podman logs "$HAPROXY_CTR" >>"$logf" 2>&1 || true
        podman rm -f -t 5 "$HAPROXY_CTR" >/dev/null 2>&1 || true
        ;;
    ruststrom)
        if [[ -n $RS_PID ]]; then
            kill -TERM "$RS_PID" 2>/dev/null || true
            # Drain finishes immediately when idle; force after 10 s.
            for _ in $(seq 100); do kill -0 "$RS_PID" 2>/dev/null || break; sleep 0.1; done
            kill -KILL "$RS_PID" 2>/dev/null || true
            wait "$RS_PID" 2>/dev/null || true
            RS_PID=""
        fi
        ;;
    esac
    # Both load balancers use the same ports; make sure they are free for the next one.
    for _ in $(seq 50); do port_in_use "$PORT_HTTP" || break; sleep 0.1; done
}

lb_pid() {
    case $1 in
    haproxy) podman inspect -f '{{.State.Pid}}' "$HAPROXY_CTR" ;;
    ruststrom) echo "$RS_PID" ;;
    esac
}

# Triggers a configuration reload the way each load balancer is reloaded in production.
reload_lb() {
    case $1 in
    haproxy) podman kill -s USR2 "$HAPROXY_CTR" >/dev/null ;; # master-worker reload
    ruststrom)
        RS_GENERATION=$((RS_GENERATION + 1))
        render_ruststrom_conf # file watcher picks up the atomic rename
        ;;
    esac
}

# haproxy_info FIELD: one value from the HAProxy CLI "show info".
haproxy_info() {
    local line
    exec 9<>"/dev/tcp/127.0.0.1/$PORT_HAPROXY_CLI"
    echo "show info" >&9
    line=$(grep -m1 "^$1:" <&9 || true)
    exec 9>&-
    echo "${line#*: }"
}

lb_version() {
    case $1 in
    haproxy)
        podman run --rm "$HAPROXY_IMAGE" haproxy -vv 2>&1 |
            awk '/^HAProxy version/ { v = $1 " " $3 } /^Running on SSL library/ { sub(/.*: /, ""); s = $0 }
                 END { print v ", " s }'
        ;;
    ruststrom) echo "$("$RUSTSTROM_BIN" --version 2>&1 | head -1), TLS: rustls (built in)" ;;
    esac
}
