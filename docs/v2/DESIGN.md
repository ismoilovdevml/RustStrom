# RustStrom v2.0 design: hyper 1.x data plane

Goal: an internet-facing, high-load HTTP(S) load balancer that can replace HAProxy for L7 HTTP
workloads, with measurements behind every performance claim.

## Why hyper and not Pingora

A first v2 built on Cloudflare Pingora 0.9.1 (branch `v2-pingora`, status in
`docs/v2/pingora/STATUS.md`) passed the full e2e suite but was measured slower than v1.1.1. The
setup was plain HTTP/1.1 keep-alive, 1 KB responses, `wrk -t6 -c256`, the LB pinned to 4 cores and
4 nginx backends, on roberta on 2026-11-21:

| Data plane | req/s |
|---|---|
| HAProxy 3.2 (nbthread 4) | ~150k |
| v1.1.1 (hyper 0.14 + tokio work-stealing) | ~98-111k |
| Bare Pingora proxy, no filters (work_stealing off) | ~86-97k |
| v2 on Pingora | ~70k |

Pingora's own ceiling is below our hyper code, so v2 builds on hyper 1.x. The Pingora branch
still supplies proven logic to port: retry rules, passive health, slow-client deadlines, the
consistent-hash ring, P2C, the LDAP hardening and the forwarding-header rules.

## Non-negotiable requirements

1. **Config compatibility.** Every v1.1 TOML file that passes `--check` keeps working. New keys
   are optional and have safe defaults. Use the same new keys as `v2-pingora` (see its README
   "v2 options") so configs written for that branch also load.
2. **No panics on input.** `#![deny(clippy::unwrap_used, clippy::expect_used)]` outside tests,
   and no slicing or indexing on data that comes from the network.
3. **CLI:** `-c/--config`, `--check`, `--version`. Linux only.
4. **Metrics:** keep the v1.1 Prometheus names; new ones may be added.
5. **Evidence:** `tests/e2e` (134 black-box tests, with `@pytest.mark.v2` tests strict on 2.x)
   is the functional gate. `scripts/bench` on roberta is the performance gate.

## Performance architecture

- **Shared-nothing workers.** `threads` (default: number of CPUs) OS threads, each running a
  tokio `current_thread` runtime and owning its own `SO_REUSEPORT` listener socket, so the
  kernel spreads accepts across threads. There is no work stealing and no cross-thread wakeups
  on the hot path. Optional `cpu_affinity = true` pins thread i to core i of the process
  affinity mask.
- **Per-thread upstream pools.** Each worker owns its own hyper-util client and connection pool
  per backend. There is no shared pool lock. Pool size and idle timeout come from config.
- **Zero-allocation hot path where practical:**
  - pre-built `HeaderValue`s for the hostname and ports;
  - routing by a pre-indexed host map plus an ordered fallback list;
  - backend selection reads an `ArcSwap` snapshot of healthy backends;
  - no `String` formatting per request;
  - the request is rewritten in place (`into_parts`).
- **Per-thread metrics.** Counters and histograms are thread-local and summed at scrape time.
  Per-backend handles are resolved once per config generation; the hot path makes no
  `with_label_values` calls.
- **Allocator:** mimalloc (already in v1.1).
- **Memory per idle connection:** target under 8 KiB. Avoid per-connection buffers larger than
  hyper's defaults, and do not keep upstream state for idle client connections. Measure with
  bench scenario 5.
- **TLS:** rustls 0.23 with the aws-lc-rs provider, session tickets on, ALPN `h2` and
  `http/1.1`, a handshake deadline, and SNI resolution through an `ArcSwap` certificate map.
- **Socket options:** TCP_NODELAY, configurable backlog, SO_REUSEPORT. Keepalive on upstream
  sockets.

## Behaviour (same as v2-pingora, already covered by e2e)

- **Strategies:** RoundRobin, Random, IPHash (Ketama ring), LeastConnection (P2C on in-flight
  requests with a RAII guard), StickyCookie (opaque FNV id, `Path=/`).
- **Health:** active HTTP checks with rise/fall and slow_threshold; passive marking on connect
  failure; state carried over on reload; atomic transitions.
- **Retries:** connect failures are retried on another backend. After connect, a request is
  retried only if it is idempotent and has no body and nothing has been sent to the client yet.
  Retries are bounded by `retries`.
- **Timeouts:**
  - `connect_timeout_ms`, `read_timeout_ms` (504), `write_timeout_ms`, `idle_timeout_ms`;
  - client header deadline 30 s (408);
  - client body idle timeout and minimum rate;
  - h2 keepalive and idle timeouts;
  - TLS handshake deadline 10 s.
- **Limits:** `max_connections` globally and per pool, with 503.
- **Forwarding headers:**
  - X-Real-IP, X-Forwarded-Proto, X-Forwarded-Port (real listener port) and X-Forwarded-Server
    are overwritten; X-Forwarded-For is appended.
  - Hop-by-hop headers are stripped from requests and responses (responses keep them on 101).
  - Duplicate Host headers get 400. For h2, `:authority` is used and must match Host.
- **Protocols:** WebSocket and other Upgrade requests (hyper upgrade + copy_bidirectional);
  upstream TLS with SNI and verification; optional upstream h2.
- **Middlewares:**
  - RateLimiter (per IP, sharded, bounded);
  - MaxBodySize (413 for both declared and streamed bodies);
  - Compression (br 4 / gzip 5, skip small, binary and SSE bodies, add Vary);
  - HttpsRedirector;
  - CustomErrorPages (cached);
  - Authentication (LDAP: escaped input, rejects empty passwords, timeout, success and failure
    caches, concurrency semaphore).
- **Operations:**
  - Hot reload with a directory watch, atomic rename and Kubernetes `..data` support; an invalid
    config keeps the old one.
  - SIGTERM drains and exits as soon as no request is in flight, or when the grace period ends.
  - ACME through `crates/ruststrom-acme`.

## Milestones and gates

| Milestone | Content | Gate (roberta) |
|---|---|---|
| M1 | Worker runtime + SO_REUSEPORT + hyper 1.x HTTP/1 proxy + routing + strategies + forwarding headers + metrics | e2e non-v2 tests pass; three-way plain HTTP ≥ v1.1.1 (≥ 100k) |
| M2 | TLS + h2, health/passive/retries/timeouts, limits, hot reload, drain | all e2e tests pass (v2 strict); TLS handshakes ≥ HAProxy −10% |
| M3 | Middlewares, WebSocket, upstream TLS/h2, ACME | e2e 134/134; memory/idle conn < 8 KiB |
| M4 | Optimisation pass driven by perf profiles | plain HTTP ≥ 85% of HAProxy; full bench report |
