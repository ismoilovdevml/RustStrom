# RustStrom v2 data plane: status

Core workstream on Pingora 0.9.1. Evidence comes from `cargo test`,
`cargo clippy --all-targets -- -D warnings`, `scripts/smoke-v2.sh` and `tests/e2e/run.sh` on
roberta (the last run is recorded at the end of this file).

## M1 HTTP proxy: done

- Pingora `Server` bootstrap with own CLI (`-c/--config`, `--check`, `--version`, `-u/--upgrade`,
  `-d/--daemon`); v1.1 TOML plus the optional v2 keys of DESIGN.md.
- v1 matcher language, pre-indexed router (exact host map + ordered fallback, `:authority` for
  h2), first match in config order like v1.1.
- Per-pool working snapshot (`ArcSwap`), rebuilt only on health changes. Strategies RoundRobin,
  Random, IPHash (consistent hash ring, 160 points per backend), LeastConnection (P2C over
  in-flight requests, two different backends, RAII guard), StickyCookie (v1.1 opaque FNV id,
  `Path=/`, falls back to `inner`).
- Forwarding headers like v1.1 (X-Real-IP / X-Forwarded-Proto / -Server overwritten, XFF
  appended) except X-Forwarded-Port, which is now the real listener port. Hop-by-hop headers are
  removed from requests and from backend responses (except `101` upgrades).
- Prometheus metrics with the v1.1 names on `prometheus_metrics_address`, plus
  `upstream_retries_total` and `backend_health`.

## M2 production behaviour: done

- HTTPS with per-SNI certificates (exact name, then `*.parent`), ALPN h2/http1.1. TLS is
  terminated in `src/frontend.rs` (10 s handshake deadline); HTTP/1 request heads must arrive
  within 30 s in total (slowloris), otherwise 408 + close.
- Active health checks: `kind = "http" | "tcp"`, `expected_status`, `rise`/`fall`,
  `slow_threshold` (slow backends only get traffic when no backend is healthy), DNS re-resolution
  every round. Passive health: a failed connect counts like a failed check.
- Timeouts per pool (`connect/read/write/idle_timeout_ms`), retries to another backend on
  connect failure (any method) and on post-connect upstream errors (idempotent, body-less
  requests only, never after a timeout or once the client got a response).
- Status mapping: connect errors 502, upstream timeouts 504, no working backend 502 (v1.1),
  `max_connections` (global and per pool, counts in-flight requests) 503.
- Hot reload: directory watch, invalid config keeps the old one, health state, resolved addresses
  and in-flight counters carry over, in-flight requests keep their pool.
- SIGTERM: Pingora stops accepting, the drain service exits as soon as nothing is in flight (at the
  latest after `grace_period_secs`). Graceful upgrade: `--upgrade` + SIGQUIT to the old process
  (`upgrade_sock`, `pid_file` keys).
- WebSocket passthrough (HTTP and TLS), upstream TLS (`tls`, `sni`, `verify_cert`) and upstream
  h2 (`h2 = true`, requires `tls`).
- ACME: `crates/ruststrom-acme` wired in (`src/acme.rs`), challenges answered before routing,
  renewal loop per account group, certificates installed into the SNI store without reload.

## M3 middlewares: done

RateLimiter (per IP fixed window, 64 lock shards, at most 100k tracked clients), MaxBodySize
(413 for declared and streamed bodies), Compression (Pingora module driven by the v1.1 rules:
q-values, br 4 / gzip 5, skip < 1 KiB, binary and SSE, `Vary`), HttpsRedirector (301),
CustomErrorPages (cached at load, for backend and proxy generated errors), Authentication (LDAP,
`ldap_escape`, empty password rejected, 5 s timeout, 30 s success / 5 s failure cache keyed by a
salted SHA-256 of user and password, at most `max_concurrent` (32) LDAP exchanges, 503 above).
Request-phase middlewares run in TOML order.

## Review fixes (before 2.0.0)

- Slow clients: `client_header_timeout_ms` (30 s head), `client_body_idle_timeout_ms` (60 s
  pause, 408), `client_body_min_rate` (500 B/s after 10 s, 408), `h2_idle_timeout_ms` (60 s
  without an open stream; also ends a never-completed HEADERS frame). Smoke tests: body never
  sent (408 after 2.0 s), body trickled 1 B/s (408 after 10.0 s), h2 HEADERS trickled 1 B/s
  (closed after 3.0 s).
- LDAP: negative cache, salted SHA-256 key, bounded concurrency failing fast with 503.
- Health transitions are compare-and-swap; only one caller rebuilds snapshot and ring.
- Reload removes the Prometheus series of backends that are gone.
- `check_every = 0`: passive health stays on, a warning is logged, passively failed backends are
  retried after 10 s.
- Host handling: duplicate `Host` → 400 (own check), absolute-form target with a different
  `Host` → 400 (Pingora), h2 `:authority` ≠ `Host` → 400 (Pingora); routing on h2 uses
  `:authority`, the value Pingora forwards. All three are smoke tests.
- README/CHANGELOG: XFF semantics, peer-IP only, rate limiter eviction, `ZLIB_NG_SIMD`.

## Deviations from DESIGN.md / v1.1

- `rise`/`fall` default to 1 (v1.1 timing: one check changes the state; the e2e suite relies on
  it). `rise = 2, fall = 3` is the recommended explicit setting.
- No working backend answers 502 (v1.1 status), not 503.
- Compression supports br and gzip only; `deflate` is no longer negotiated (Pingora has no
  deflate encoder).
- `pool_max_idle_per_host` is accepted but ignored (warning); Pingora's pool is global
  (`upstream_keepalive_pool_size`).
- CustomErrorPages cannot give a body to a backend response announced as empty
  (`Content-Length: 0`, 204, 304, HEAD); those pass through unchanged.
- ACME challenge paths are proxied when no ACME certificate is configured (v1.1 always answered
  404), so backends can run their own ACME client.
- `vendor/libz-ng-sys`: patched build so that no `cmake` is needed (see its README).

## Not done / not verified

- LDAP authentication is only unit tested (no LDAP server on roberta).
- ACME issuance through the proxy was not run end to end here (the crate has Pebble tests).
- Static musl build: not attempted on roberta (no musl-gcc).

## Last verification on roberta (2026-11-21, release build)

| Check | Result |
|---|---|
| `cargo fmt --all --check` | clean |
| `cargo clippy --locked --all-targets -- -D warnings` | clean |
| `cargo test --workspace` | rust-strom 67 passed; ruststrom-acme 12 + 1 + 1 passed (11 Pebble tests ignored) |
| `scripts/smoke-v2.sh` (release) | 44 passed, 0 failed |
| `tests/e2e/run.sh` (whole suite incl. slow, `v2` marker strict) | 134 passed |
| `--check` on `configs/config.toml`, `docker-config.toml` | valid |
