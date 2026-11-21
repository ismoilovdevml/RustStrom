# Changelog

All notable changes to RustStrom are documented in this file.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [2.0.0] - Unreleased

### Platforms

- Linux only. Release binaries: `rust-strom-linux-x86_64` (static musl) and the Docker image. macOS binaries are no longer built and `install.sh` refuses macOS.

### Changed

- Data plane rewritten on Cloudflare Pingora 0.9. Every v1.1 configuration that passes `--check` still loads; new keys are optional (see README, "v2 options").
- HTTP/1 request heads must arrive within `client_header_timeout_ms` (30 s), request bodies may not pause longer than `client_body_idle_timeout_ms` (60 s) or average less than `client_body_min_rate` (500 B/s) after 10 s (`408`), HTTP/2 connections without open streams close after `h2_idle_timeout_ms` (60 s). TLS handshakes have 10 s.
- Retries on another backend after a failed connect; failed connects also mark a backend down (passive health). Without active checks (`check_every = 0`) such a backend is retried after 10 s.
- `IPHash` uses consistent hashing: clients map to different backends than in v1 once, afterwards adding or removing a backend moves only ~1/N of them.
- `LeastConnection` picks the less loaded of two random backends by in-flight requests.
- Chunked and HTTP/2 bodies over `MaxBodySize` get `413` (v1: `502`).
- WebSocket and other upgrades are proxied; upstream TLS and HTTP/2 are available.
- Compression negotiates `br` and `gzip` only (no `deflate`).
- Requests with more than one `Host` header, absolute-form targets that disagree with `Host`, and HTTP/2 requests whose `Host` disagrees with `:authority` are rejected with `400`.

### Security

- `X-Forwarded-For` keeps append semantics: backends must use `X-Real-IP` or the last entry. The client IP is always the socket peer (no trusted-proxy mode yet).
- LDAP authentication: successful logins are cached for 30 s and failed ones for 5 s, keyed by a salted SHA-256. At most 32 LDAP exchanges run at once (`max_concurrent`); further requests get `503` instead of queueing LDAP connections.
- Hop-by-hop headers are also removed from backend responses.
- The rate limiter evicts the oldest clients when more than 100 000 IPs are tracked; a distributed flood can exceed the per-IP limit.
- Builds use a vendored, portable zlib-ng; never set `ZLIB_NG_SIMD` for binaries that run elsewhere.

## [1.1.1] - 2026-11-20

### Fixed

- `X-Forwarded-Port` now carries the port of the listener the client connected to (e.g. `8443`) instead of a fixed `80`/`443`, so backends build correct redirect URLs on non-standard ports. Found by the new end-to-end suite.
- A `502` caused by every backend of a pool being unhealthy now increments `backend_failures_total{backend="no_healthy_backend"}`, so alerts on that metric also fire on a complete outage.

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
