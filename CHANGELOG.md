# Changelog

All notable changes to RustStrom are documented in this file.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [1.1.0] - 2026-11-20

### Breaking changes

- The Prometheus metrics endpoint now listens on `127.0.0.1:9090` by default (was hardcoded `0.0.0.0:9090`).
  Set `prometheus_metrics_address = "0.0.0.0:9090"` to scrape remotely; the endpoint is unauthenticated, so firewall it.
- Sticky cookie values are now opaque hashes. Cookies issued by older versions are ignored once and then reissued.

### Security

- Fixed a crash triggered by a malformed sticky cookie.
- TLS handshakes no longer run serially in the accept loop (a single slow client could stall all HTTPS accepts); handshakes now have a 10 s timeout.
- Rate limiter is keyed by client IP (previously per connection) and its memory use is bounded.
- `MaxBodySize` is enforced for chunked and HTTP/2 request bodies, not only when `Content-Length` is present. Bodies with a declared length still get `413`; streamed bodies are cut off once the limit is reached and the client receives `502`.
- LDAP authentication: escaped user input in search filters (filter injection) and rejected empty passwords (unauthenticated bind).
- `X-Real-IP`, `X-Forwarded-Proto`, `X-Forwarded-Port` and `X-Forwarded-Server` are always overwritten by the proxy, so clients can not spoof them; `X-Forwarded-For` keeps the standard append behaviour (client value, then the peer address).
- Hop-by-hop headers (`Connection` and the headers it lists, `Keep-Alive`, `TE`, `Upgrade`, ...) are stripped from requests forwarded to backends.
- Metrics endpoint bound to localhost by default.
- Dashboard config API (`ruststrom-dashboard/config-api.cjs`): removed `Access-Control-Allow-Origin: *`, requires `RUSTSTROM_ADMIN_TOKEN` (injected by the dashboard proxy, never sent to the browser) plus an `X-RustStrom-Admin` header against CSRF, limits the body to 1 MB, writes the config atomically and validates it with `rust-strom --check` when `RUSTSTROM_BIN` is set.

### Stability

- Removed panic paths for requests without a path (e.g. `OPTIONS *`), peer connection resets right after accept, and system clock (NTP) jumps in health checks.
- An invalid matcher during hot reload no longer crashes the process; the old configuration is kept.
- `accept()` errors such as `EMFILE` no longer stop the listener.
- Release builds use `panic = "unwind"`, so a panic is isolated to a single connection.
- Fixed a counter underflow in the `LeastConnection` strategy.
- Backend health state is preserved across configuration reloads.
- Invalid or unknown middleware entries are logged instead of being silently dropped.
- Hot reload works with editors that save via atomic rename and with Kubernetes ConfigMap updates.

### Performance

- mimalloc as global allocator.
- HTTP/2 over TLS via ALPN.
- Compression: brotli quality 4 / gzip level 5; small, already-compressed (binary) and SSE responses are skipped; `Vary: Accept-Encoding` is set.
- Custom error pages are cached in memory at config load.
- Hostname is cached instead of looked up per request.
- Headers are rewritten in place instead of copying the request.
- Certificates are shared via `Arc` instead of being cloned per handshake.
- Finer latency histogram buckets.

### Operations

- Graceful shutdown on SIGTERM and SIGINT (in-flight requests drain for up to 30 s).
- New `--check` flag validates the configuration and exits.
- New `prometheus_metrics_address` configuration key.
- Upstream connect timeout of 5 s; request header read timeout of 30 s.
- Log level via `LOG_LEVEL` (`RUST_LOG` accepted as fallback).
- Fixed Dockerfile (`--config` CMD, vendored OpenSSL build deps, curl for healthcheck), docker-compose healthcheck and `install.sh` systemd unit (`CAP_NET_BIND_SERVICE`, `--check` pre-start, graceful stop timeout).
- Removed `warp` and other unused dependencies.
- Documentation corrected to match the actual config format: `--config` flag, `type = "Local"` / `type = "ACME"` certificates, `CustomErrorPages` (`location` + `errors`), `LOG_LEVEL`, metric names.

### Known limitations

- No TLS to upstream backends and no WebSocket (HTTP Upgrade) proxying yet.
- No retries or circuit breaker for failing backends.
- ACME certificates are requested on the first configuration reload rather than at startup, and are not renewed automatically.
