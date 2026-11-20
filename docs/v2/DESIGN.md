# RustStrom v2.0 design: Pingora data plane

Goal: an internet-facing, high-load HTTP(S) load balancer that can replace HAProxy for L7 HTTP
workloads, with measurements to back every performance claim.

## Non-negotiable requirements

1. **Config compatibility.** Every v1.1 TOML file that passes `rust-strom --check` keeps working
   (same keys, same matcher language, same strategy and middleware names). New features are
   new optional keys with safe defaults.
2. **No panics on input.** No `unwrap`/`expect`/indexing on data derived from requests, config
   or the network. `#![deny(clippy::unwrap_used, clippy::expect_used)]` outside tests.
3. **Same CLI:** `--config/-c`, `--check`, `--version`, plus Pingora's `--upgrade` / `--daemon`.
4. **Observability compatibility:** keep the v1.1 Prometheus metric names. New metrics may be added.
5. **Evidence:** every feature has an e2e test (tests/e2e); every performance claim comes from
   scripts/bench on roberta with core pinning.

## Architecture

```
            ┌───────────────────────── control plane ─────────────────────────┐
 TOML ──► config::load ──► RuntimeConfig ──► ArcSwap<Router> + ArcSwap<Certs>   │
            │  file watcher (dir watch, atomic rename, k8s ..data)            │
            │  health service (active HTTP/TCP checks, rise/fall)              │
            │  ACME service (instant-acme, HTTP-01, renew < 30 days)          │
            │  metrics service (Prometheus, configurable address)              │
            └──────────────────────────────────────────────────────────────────┘
            ┌───────────────────────── data plane (Pingora) ──────────────────┐
 client ──► listeners (HTTP, HTTPS with SNI cert callback, h2 via ALPN)        │
            └► RustStromProxy: ProxyHttp                                      │
                 request_filter       ACME challenge, route lookup, redirect,  │
                                      rate limit, auth, max_connections        │
                 request_body_filter  MaxBodySize (413 on streamed bodies too) │
                 upstream_peer        strategy pick among healthy backends,    │
                                      retry/failover (fail_to_connect)         │
                 upstream_request_filter  X-Forwarded-*, X-Real-IP, hop-by-hop │
                 response_filter      sticky Set-Cookie, error pages, metrics  │
                 compression          Pingora ResponseCompression (level 4-5)  │
                 logging              access log + metrics                     │
            └──────────────────────────────────────────────────────────────────┘
```

### Modules (src/)

| Module | Owner | Responsibility |
|---|---|---|
| `main.rs` | core | CLI, Pingora `Server` bootstrap, services wiring |
| `config/` | core | TOML structs (moved from v1 `configuration.rs`, extended), validation, watcher |
| `matcher.rs` | core | v1 `backend_pool_matcher.rs`, unchanged language |
| `router.rs` | core | pre-indexed router: exact-host `HashMap`, then ordered list of non-host matchers |
| `upstream/` | core | backend set per pool, health state (rise/fall), selection strategies, per-backend in-flight counters |
| `proxy.rs` | core | `ProxyHttp` implementation and per-request `Ctx` |
| `middleware/` | middleware | RateLimiter, MaxBodySize, Compression, HttpsRedirector, CustomErrorPages, Authentication |
| `tls.rs` | core | SNI certificate store (`ArcSwap<HashMap<name, cert>>`) + listener callback |
| `acme/` | acme | instant-acme client, challenge store, renewal service |
| `metrics.rs` | core | metric definitions with pre-resolved per-backend handles |

### Strategies

| Config name | v2 implementation |
|---|---|
| `RoundRobin` | atomic counter over the healthy snapshot |
| `Random` | uniform over the healthy snapshot |
| `IPHash` | consistent hashing (Ketama) on client IP, so adding or removing a backend moves ~1/N clients |
| `LeastConnection` | power of two choices on **in-flight requests** (atomic per backend, RAII guard) |
| `StickyCookie` | opaque backend id cookie (same as v1.1), falls back to `inner` |

The healthy snapshot is an `ArcSwap<Vec<Arc<Backend>>>` rebuilt only when health changes, so
selection does no allocation.

### New optional config keys (all with defaults)

Per pool (`[backend_pools.client]` and new `[backend_pools.upstream]`):

```toml
[backend_pools.upstream]
connect_timeout_ms = 3000
read_timeout_ms = 30000
write_timeout_ms = 30000
idle_timeout_ms = 60000
retries = 1                  # extra attempts on another backend; idempotent methods only
tls = false                  # TLS to backends
sni = ""                     # SNI and verification name when tls = true
verify_cert = true
h2 = false                   # HTTP/2 to backends (requires tls)
max_connections = 0          # 0 = unlimited; above it the pool answers 503
```

Health (`[backend_pools.health_config]`): existing `slow_threshold`, `timeout`, `path`, plus
`rise = 2`, `fall = 3`, `kind = "http" | "tcp"`, `expected_status = [200, 399]` range.

Global: `max_connections` (whole process, 503 above it), `threads` (Pingora worker threads,
default = number of CPUs), `grace_period_secs` (default 30), `upstream_keepalive_pool_size`.

### Behaviour changes allowed in v2

- Chunked bodies over `MaxBodySize` get `413` (Pingora `request_body_filter` can reject).
- WebSocket and other `Upgrade` requests are proxied (Pingora supports them natively).
- `IPHash` mapping changes once (consistent hashing).

## Workstreams

1. **core**: everything above except `middleware/` and `acme/`, with v1 middlewares ported last
   if the middleware stream is not done. Critical path.
2. **middleware**: port the six v1.1 middlewares to Pingora phases once core exposes `Ctx`.
3. **acme**: standalone `acme/` module on instant-acme, tested against Pebble in podman.
4. **e2e**: black-box test suite in `tests/e2e/` driving the binary with TOML configs. It must
   run against v1.1.0 (to show what v1 fails) and v2.
5. **bench**: `scripts/bench/` comparing HAProxy and RustStrom on roberta with taskset core
   pinning: plain HTTP keep-alive, TLS 1.3 handshakes, h2, 1-10 MB bodies, 10k slow clients,
   failover and reload under load.

## Definition of done for v2.0

- All e2e tests pass. They cover failover, retry, slowloris, malformed HTTP, hot reload under
  load with zero errors, SIGTERM drain with in-flight requests, WebSocket, h2, SNI, ACME
  against Pebble.
- 1 hour soak under load with no RSS growth trend and no errors.
- Benchmark report with HAProxy side by side. Any scenario where RustStrom is slower is stated
  as slower.
- `cargo clippy -D warnings`, fmt and tests are green in CI, plus static musl and macOS builds.
