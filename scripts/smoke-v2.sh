#!/bin/bash
# End-to-end smoke test for RustStrom v2 (Pingora data plane).
#
# Usage: scripts/smoke-v2.sh [path/to/rust-strom]
#   BIN   binary to test (default: target/release/rust-strom)
#   WORK  scratch directory, wiped on start (default: /tmp/ruststrom-smoke-v2)
#
# Ports (all 20xxx): 20081-20083 backends, 20080 HTTP, 20443 HTTPS, 20090 metrics.
# Needs: python3, curl, openssl, nc, bc.
set -u
BIN=$(readlink -f "${1:-${BIN:-target/release/rust-strom}}")
D=${WORK:-/tmp/ruststrom-smoke-v2}
rm -rf "$D" && mkdir -p "$D" && cd "$D" || exit 1
PASS=0; FAIL=0
ok()   { echo "PASS  $1"; PASS=$((PASS+1)); }
bad()  { echo "FAIL  $1"; FAIL=$((FAIL+1)); }
alive(){ kill -0 "$LB" 2>/dev/null; }
cleanup() { kill "$LB" "$B1" "$B2" "$B3" 2>/dev/null; [ ${#IDLE[@]} -gt 0 ] && kill "${IDLE[@]}" 2>/dev/null; true; }
IDLE=(); LB=; B1=; B2=; B3=
trap cleanup EXIT

cat > echo_backend.py <<'EOF'
import http.server, sys, json, time, base64, hashlib
NAME = sys.argv[2] if len(sys.argv) > 2 else sys.argv[1]
class H(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    def _reply(self):
        n = int(self.headers.get("content-length") or 0)
        if n: self.rfile.read(n)
        if self.path.startswith("/slow"):
            time.sleep(3)
        if self.path.startswith("/ws") and self.headers.get("upgrade", "").lower() == "websocket":
            key = self.headers.get("sec-websocket-key", "")
            accept = base64.b64encode(hashlib.sha1((key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").encode()).digest()).decode()
            self.send_response(101); self.send_header("Upgrade", "websocket"); self.send_header("Connection", "Upgrade")
            self.send_header("Sec-WebSocket-Accept", accept); self.end_headers(); self.wfile.flush()
            data = self.rfile.read(5)          # echo 5 raw bytes back over the upgraded stream
            self.wfile.write(data); self.wfile.flush(); self.close_connection = True
            return
        if self.path.startswith("/big"):
            body = (("hello compression " * 200) + "\n").encode()
            self.send_response(200); self.send_header("content-type", "text/html")
            self.send_header("content-length", str(len(body))); self.end_headers(); self.wfile.write(body)
            return
        if self.path.startswith("/missing"):
            body = b"backend says not found"
            self.send_response(404); self.send_header("content-type", "text/plain")
            self.send_header("content-length", str(len(body))); self.end_headers(); self.wfile.write(body)
            return
        body = json.dumps({k.lower(): v for k, v in self.headers.items()} | {"_all_real_ip": self.headers.get_all("x-real-ip"), "_backend": NAME, "_path": self.path}).encode()
        self.send_response(200); self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body))); self.end_headers(); self.wfile.write(body)
    do_GET = do_POST = do_OPTIONS = do_PUT = do_DELETE = _reply
    def log_message(self, *a): pass
http.server.ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1])), H).serve_forever()
EOF
python3 echo_backend.py 20081 b1 2>>backend.err & B1=$!
python3 echo_backend.py 20082 b2 2>>backend.err & B2=$!

openssl req -x509 -newkey rsa:2048 -nodes -keyout key.pem -out cert.pem -days 2 -subj "/CN=lb.test" -addext "subjectAltName=DNS:lb.test" >/dev/null 2>&1
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -keyout key2.pem -out cert2.pem -days 2 -subj "/CN=other.test" -addext "subjectAltName=DNS:other.test" >/dev/null 2>&1
mkdir -p errors && echo "<h1>custom 404</h1>" > errors/404.html

write_config() { cat > "$1" <<EOF
http_address = "127.0.0.1:20080"
https_address = "127.0.0.1:20443"
prometheus_metrics_address = "127.0.0.1:20090"
threads = 4
grace_period_secs = 10
[health_interval]
check_every = 1

[[backend_pools]]
matcher = "Host('fail.test')"
addresses = ["127.0.0.1:20089", "127.0.0.1:20081"]
schemes = ["HTTP"]
strategy = { RoundRobin = {} }
[backend_pools.health_config]
fall = 1000

[[backend_pools]]
matcher = "Host('features.test')"
addresses = ["127.0.0.1:20081"]
schemes = ["HTTP", "HTTPS"]
strategy = { RoundRobin = {} }
[backend_pools.middlewares.Compression]
[backend_pools.middlewares.CustomErrorPages]
location = "$D/errors"
errors = [404, 502]

[[backend_pools]]
matcher = "Host('redirect.test')"
addresses = ["127.0.0.1:20081"]
schemes = ["HTTP", "HTTPS"]
strategy = { Random = {} }
[backend_pools.middlewares.HttpsRedirector]

[[backend_pools]]
matcher = "Host('down.test')"
addresses = ["127.0.0.1:20088"]
schemes = ["HTTP"]
strategy = { IPHash = {} }
[backend_pools.health_config]
rise = 1
fall = 1

[[backend_pools]]
matcher = "Host('slow.test')"
addresses = ["127.0.0.1:20081"]
schemes = ["HTTP"]
strategy = { LeastConnection = {} }
[backend_pools.upstream]
read_timeout_ms = 1000

[[backend_pools]]
matcher = "$2"
addresses = ["127.0.0.1:20081", "127.0.0.1:20082"]
schemes = ["HTTP", "HTTPS"]
strategy = { StickyCookie = { cookie_name = "RUSTSTROM_SESSION", http_only = true, secure = false, same_site = "Lax", inner = { LeastConnection = {} } } }
[backend_pools.middlewares.RateLimiter]
limit = 60
window_sec = 60
[backend_pools.middlewares.MaxBodySize]
limit = 1024

[certificates."lb.test"]
type = "Local"
certificate_path = "cert.pem"
private_key_path = "key.pem"

[certificates."other.test"]
type = "Local"
certificate_path = "cert2.pem"
private_key_path = "key2.pem"
EOF
}
write_config config.toml "Host('lb.test') || PathRegexp('^/')"

"$BIN" --version | grep -q "RustStrom" && ok "--version: $("$BIN" --version)" || bad "--version"
"$BIN" --config config.toml --check >check.log 2>&1 && ok "--check accepts valid config" || bad "--check: $(tail -1 check.log)"
write_config broken.toml "Host('broken'"
"$BIN" --config broken.toml --check >check2.log 2>&1 && bad "--check accepted an invalid matcher" || ok "--check rejects invalid matcher (exit $?)"
LOG_LEVEL=info "$BIN" --config config.toml >lb.log 2>&1 & LB=$!
for i in $(seq 50); do curl -s -o /dev/null --max-time 1 http://127.0.0.1:20090/metrics && break; sleep 0.1; done
alive || { echo "LB did not start:"; tail -20 lb.log; exit 1; }
C="curl -s -o /dev/null -w %{http_code} --max-time 5"

[ "$($C http://127.0.0.1:20080/)" = 200 ] && ok "plain HTTP proxy 200" || bad "plain HTTP proxy"

# 1. StickyCookie: crash payloads, opaque cookie, pinning
$C -H 'Cookie: RUSTSTROM_SESSION=127.0.0.1:8080' http://127.0.0.1:20080/ >/dev/null
$C -H 'Cookie: a=b; RUSTSTROM_SESSION' http://127.0.0.1:20080/ >/dev/null
alive && ok "sticky cookie crash payload: process alive" || bad "sticky cookie payload killed process"
SC=$(curl -s -D - -o /dev/null http://127.0.0.1:20080/ | grep -i set-cookie)
echo "$SC" | grep -q "Path=/" && ! echo "$SC" | grep -q "127.0.0.1" && ok "Set-Cookie opaque + Path=/ ($(echo $SC | tr -d '\r'))" || bad "Set-Cookie: $SC"
CK=$(echo "$SC" | sed -E 's/.*(RUSTSTROM_SESSION=[0-9a-f]+).*/\1/' | tr -d '\r')
BACKENDS=$(for i in $(seq 10); do curl -s -H "Cookie: $CK" http://127.0.0.1:20080/ | python3 -c 'import sys,json; print(json.load(sys.stdin)["_backend"])'; done | sort -u | wc -l)
[ "$BACKENDS" = 1 ] && ok "sticky cookie pins 10 requests to one backend" || bad "sticky cookie spread over $BACKENDS backends"

# 2. OPTIONS *
printf 'OPTIONS * HTTP/1.1\r\nHost: lb.test\r\nConnection: close\r\n\r\n' | timeout 3 nc 127.0.0.1 20080 >/dev/null 2>&1
alive && ok "OPTIONS * : process alive" || bad "OPTIONS * killed process"

# 3. 200 idle TCP connections on the TLS port, then a real client
for i in $(seq 200); do (exec 3<>/dev/tcp/127.0.0.1/20443; sleep 30) 2>/dev/null & IDLE+=($!); done
sleep 1
T0=$(date +%s.%N)
CODE=$(curl -sk -o /dev/null -w %{http_code} --max-time 5 --resolve lb.test:20443:127.0.0.1 https://lb.test:20443/)
T1=$(date +%s.%N)
[ "$CODE" = 200 ] && ok "HTTPS works with 200 idle TLS sockets open (took $(echo "$T1-$T0" | bc)s)" || bad "HTTPS blocked by idle sockets (code $CODE)"

# 4. HTTP/2 via ALPN, SNI certificates
V=$(curl -sk --http2 -o /dev/null -w %{http_version} --max-time 5 --resolve lb.test:20443:127.0.0.1 https://lb.test:20443/)
[ "$V" = 2 ] && ok "HTTPS negotiates HTTP/2 (ALPN)" || bad "HTTP version over TLS: $V"
HB=$(curl -sk --http2 --max-time 5 --resolve lb.test:20443:127.0.0.1 https://lb.test:20443/)
echo "$HB" | grep -q '"host": "lb.test' && ok "HTTP/2 request forwarded with Host header" || bad "h2 host: $HB"
echo "$HB" | grep -q '"x-forwarded-proto": "https"' && ok "X-Forwarded-Proto https over TLS" || bad "h2 proto: $HB"
CN1=$(echo | openssl s_client -connect 127.0.0.1:20443 -servername lb.test 2>/dev/null | openssl x509 -noout -subject 2>/dev/null)
CN2=$(echo | openssl s_client -connect 127.0.0.1:20443 -servername other.test 2>/dev/null | openssl x509 -noout -subject 2>/dev/null)
echo "$CN1" | grep -q lb.test && echo "$CN2" | grep -q other.test && ok "SNI selects per-name certificate (RSA + ECDSA)" || bad "SNI: '$CN1' '$CN2'"
CODE=$(curl -sk -o /dev/null -w %{http_code} --max-time 5 --resolve nocert.test:20443:127.0.0.1 https://nocert.test:20443/)
[ "$CODE" = 000 ] && alive && ok "unknown SNI: handshake refused, process alive" || bad "unknown SNI code $CODE"

# 5. Header spoofing
H=$(curl -s --max-time 5 -H 'X-Real-IP: 6.6.6.6' -H 'X-Forwarded-Proto: https' -H 'X-Forwarded-For: 1.2.3.4' -H 'Connection: keep-alive, X-Secret' -H 'X-Secret: 1' -H 'Proxy-Authorization: Basic eDp5' http://127.0.0.1:20080/)
echo "$H" | grep -q '"_all_real_ip": \["127.0.0.1"\]' && echo "$H" | grep -q '"x-forwarded-proto": "http"' && echo "$H" | grep -q '"x-forwarded-for": "1.2.3.4, 127.0.0.1"' && echo "$H" | grep -q '"x-forwarded-port": "20080"' && ! echo "$H" | grep -q x-secret && ! echo "$H" | grep -qi proxy-authorization \
  && ok "X-Real-IP/X-Forwarded-* overwritten, XFF appended, hop-by-hop stripped" || bad "headers: $H"

# 6. MaxBodySize
head -c 200000 /dev/urandom > big.bin
CODE=$(curl -s -o /dev/null -w %{http_code} --max-time 10 -H 'Transfer-Encoding: chunked' --data-binary @big.bin http://127.0.0.1:20080/)
[ "$CODE" = 413 ] && ok "chunked 200 KB body over 1 KB limit -> 413" || bad "chunked body over limit: code $CODE"
CODE=$(curl -s -o /dev/null -w %{http_code} --max-time 5 --data-binary @big.bin http://127.0.0.1:20080/)
[ "$CODE" = 413 ] && ok "Content-Length over limit -> 413" || bad "content-length limit code $CODE"
CODE=$(curl -s -o /dev/null -w %{http_code} --max-time 5 -H 'Transfer-Encoding: chunked' --data-binary 'small' http://127.0.0.1:20080/)
[ "$CODE" = 200 ] && ok "small chunked body passes" || bad "small chunked body code $CODE"

# 7. Failover: one backend refuses connections, every request still succeeds
CODES=$(for i in $(seq 10); do $C -H 'Host: fail.test' http://127.0.0.1:20080/; echo; done | sort | uniq -c | tr -s ' ' | tr '\n' ';')
[ "$CODES" = " 10 200;" ] && ok "connect failure retried on the other backend (10/10 200)" || bad "failover codes: $CODES"
CODE=$($C -X POST -H 'Host: fail.test' http://127.0.0.1:20080/)
CODE2=$($C -X POST -H 'Host: fail.test' http://127.0.0.1:20080/)
[ "$CODE" = 200 ] && [ "$CODE2" = 200 ] && ok "POST also retried after connect failure (nothing was sent)" || bad "POST failover $CODE $CODE2"

# 8. All backends down -> 502 after health checks (v1.1 status)
sleep 2.5
CODE=$($C -H 'Host: down.test' http://127.0.0.1:20080/)
[ "$CODE" = 502 ] && ok "pool without healthy backend -> 502" || bad "down pool code $CODE"

# 9. Read timeout -> 504
CODE=$($C -H 'Host: slow.test' http://127.0.0.1:20080/slow)
[ "$CODE" = 504 ] && ok "upstream read timeout -> 504" || bad "slow upstream code $CODE"

# 10. HttpsRedirector
CODE=$($C -H 'Host: redirect.test' "http://127.0.0.1:20080/a?b=c")
LOC=$(curl -s -D - -o /dev/null --max-time 5 -H 'Host: redirect.test' "http://127.0.0.1:20080/a?b=c" | grep -i '^location' | tr -d '\r')
[ "$CODE" = 301 ] && echo "$LOC" | grep -q "https://redirect.test/a?b=c" && ok "HttpsRedirector 301 ($LOC)" || bad "redirect: $CODE $LOC"
CODE=$(curl -sk -o /dev/null -w %{http_code} --max-time 5 -H 'Host: redirect.test' --resolve lb.test:20443:127.0.0.1 https://lb.test:20443/)
[ "$CODE" = 200 ] && ok "HttpsRedirector passes HTTPS through" || bad "redirector over https: $CODE"

# 11. Compression and custom error pages
ENC=$(curl -s -D - -o /dev/null --max-time 5 -H 'Host: features.test' -H 'Accept-Encoding: gzip;q=0.5, br' http://127.0.0.1:20080/big | tr -d '\r' | grep -i -E '^(content-encoding|vary)' | sort | tr '\n' ' ')
echo "$ENC" | grep -qi 'content-encoding: br' && echo "$ENC" | grep -qi 'vary: accept-encoding' && ok "Compression: br by q-value, Vary set ($ENC)" || bad "compression headers: $ENC"
BODY=$(curl -s --compressed --max-time 5 -H 'Host: features.test' -H 'Accept-Encoding: gzip' http://127.0.0.1:20080/big | head -c 17)
[ "$BODY" = "hello compression" ] && ok "gzip body decodes" || bad "gzip body: $BODY"
ENC=$(curl -s -D - -o /dev/null --max-time 5 -H 'Host: features.test' -H 'Accept-Encoding: gzip' http://127.0.0.1:20080/ | tr -d '\r' | grep -i '^content-encoding')
[ -z "$ENC" ] && ok "small response not compressed" || bad "small response compressed: $ENC"
P=$(curl -s --max-time 5 -H 'Host: features.test' http://127.0.0.1:20080/missing)
[ "$P" = "<h1>custom 404</h1>" ] && ok "CustomErrorPages replaces backend 404 body" || bad "error page: $P"

# 12. WebSocket upgrade passthrough
WS=$(python3 - <<'EOF'
import socket
s = socket.create_connection(("127.0.0.1", 20080), timeout=5)
s.sendall(b"GET /ws HTTP/1.1\r\nHost: lb.test\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n")
head = b""
while b"\r\n\r\n" not in head:
    chunk = s.recv(4096)
    if not chunk: break
    head += chunk
rest = head.split(b"\r\n\r\n", 1)[1] if b"\r\n\r\n" in head else b""
s.sendall(b"hello")
while len(rest) < 5:
    chunk = s.recv(16)
    if not chunk: break
    rest += chunk
print(head.split(b"\r\n")[0].decode(), rest[:5].decode())
EOF
)
echo "$WS" | grep -q "101" && echo "$WS" | grep -q "hello" && ok "WebSocket upgrade proxied, bytes echoed ($WS)" || bad "websocket: $WS"

# 13. Rate limit is per IP: new connection (new source port) per request
CODES=""; for i in $(seq 65); do CODES="$CODES $($C http://127.0.0.1:20080/)"; done
echo "$CODES" | grep -q 429 && ok "rate limiter returns 429 across new connections" || bad "no 429 after 65 requests"

# 14. Metrics
M=$(curl -s --max-time 5 http://127.0.0.1:20090/metrics)
echo "$M" | grep -q 'http_request_duration_seconds_bucket{le="0.0005"}' && ok "metrics on configured address, sub-ms buckets" || bad "metrics"
for name in http_requests_total backend_response_time_seconds_bucket backend_failures_total 'http_status_codes_total{code="200"}' http_errors_total http_timeouts_total; do
  echo "$M" | grep -q "^$name" || MISSING="${MISSING:-} $name"
done
[ -z "${MISSING:-}" ] && ok "v1.1 metric names present" || bad "missing metrics:$MISSING"
G=$(echo "$M" | grep '^active_http_connections' | awk '{print $2}')
[ "$G" = 0 ] && ok "active_http_connections back to 0 when idle" || bad "active gauge = $G"

# 15. Hot reload: invalid matcher keeps old config; atomic mv is picked up
write_config bad.toml "Host('broken'"; mv bad.toml config.toml; sleep 3
alive && [ "$($C -H 'Host: features.test' http://127.0.0.1:20080/)" = 200 ] && grep -q "Keeping old configuration" lb.log && ok "invalid matcher on reload: old config kept, process alive" || bad "invalid reload"
write_config new.toml "Host('other.test')"; mv new.toml config.toml; sleep 3
CODE=$($C -H 'Host: unknown.test' http://127.0.0.1:20080/)
[ "$CODE" = 404 ] && grep -q "Reloaded configuration" lb.log && ok "atomic mv reload picked up (now 404 for unmatched host)" || bad "atomic reload, code $CODE"

# 16. Graceful SIGTERM: an in-flight request finishes, then the process exits quickly
curl -s -o inflight.out -w '%{http_code}' --max-time 10 -H 'Host: other.test' http://127.0.0.1:20080/slow > inflight.code & CURL=$!
sleep 0.5
kill -TERM $LB; T0=$(date +%s.%N)
wait $CURL
for i in $(seq 100); do alive || break; sleep 0.1; done
T1=$(date +%s.%N)
[ "$(cat inflight.code)" = 200 ] && ok "in-flight request completed during SIGTERM drain" || bad "in-flight request during drain: $(cat inflight.code)"
alive && bad "SIGTERM did not stop process within 10s" || { wait $LB; ok "SIGTERM -> exit code $? after $(echo "$T1-$T0" | bc)s"; }

echo "---- lb.log (tail) ----"; tail -8 lb.log
echo "RESULT: $PASS passed, $FAIL failed"
[ "$FAIL" = 0 ]
