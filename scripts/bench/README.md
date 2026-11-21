# RustStrom vs HAProxy benchmark harness

Reproducible side-by-side benchmark of RustStrom and HAProxy on one Linux host with core
pinning. It produces the "benchmark report with HAProxy side by side" required by the v2.0
definition of done (docs/v2/DESIGN.md): every scenario where RustStrom is slower is reported
as slower.

Nothing runs on a laptop. Builds and load run on the benchmark host (default roberta,
`root@172.17.4.60`, private directory `/data/ruststrom/bench`, TCP ports 22000-22999 only).

## Run it

```sh
# once per host: builds wrk/wrk2/h2load in containers, pulls images, creates cert and bodies
scripts/bench/remote.sh setup

# a real run (refuses to start unless the 1-minute load average is below 1.0)
RUSTSTROM_BIN=/data/ruststrom/target/release/rust-strom scripts/bench/remote.sh run

# harness smoke test on a busy host: short steps, low concurrency, report marked NOT VALID
RUSTSTROM_BIN=/data/ruststrom/target/release/rust-strom scripts/bench/remote.sh run --smoke --force
```

`remote.sh run` copies `scripts/bench/` to `/data/ruststrom/bench/harness/` and runs `run.sh`
there; the same `run.sh` can be started directly on the host. Options (`run.sh --help`):
`--force`, `--smoke`, `--lb haproxy,ruststrom`, `--scenarios 1,2,3,4,5,6`, `--duration SEC`,
`--no-sysctl`, `--label TEXT`. Every core/port/image setting in `lib/common.sh` can be
overridden from the environment (for example `LB_CPUS=8-11 LB_THREADS=4`).

A full run with the defaults (30 s steps) takes about 15 minutes. Fetch a run directory with
`scripts/bench/remote.sh fetch <run-id> <dest>`.

## Validity rules

- The run refuses to start if the 1-minute load average is >= 1.0. `--force` overrides it and
  marks the report **NOT VALID**; so does `--smoke`. Invalid reports print differences without
  better/worse verdicts.
- The load average is recorded before and after every step; a step that started above 1.0 is
  listed under "Warnings".
- CPU of the load generator and of the backends is sampled during every step. If either
  averages above 85% of its cores, the step is flagged: the load balancer was not the only
  bottleneck and the result understates it.
- wrk2 steps that achieve less than 95% of the target rate are flagged.
- For the TLS handshake step, HAProxy's own counters are read mid-run; if it reports more than
  5% session reuse the step is flagged as invalid (it would be measuring resumptions).

## Layout on one 16 vCPU host

| Role | Cores | Notes |
|---|---|---|
| Load generator (wrk, wrk2, h2load) | 0-5 | 6 threads |
| Load balancer under test | 6-9 | HAProxy `nbthread 4` + `cpu-map`; RustStrom 4 worker threads |
| Backends: 4 x nginx (2 workers each) | 10-15 | static files from page cache, `return 200` health |

Only one load balancer runs at a time and both listen on the same ports
(HTTP 22080, HTTPS 22443 on `[::1]`), so the clients use identical URLs. Backends listen on
127.0.0.1:22101-22104. HAProxy stats 22091, HAProxy CLI 22092, RustStrom metrics 22090.

Each scenario starts a fresh load balancer process, waits for health checks to mark all
backends up, and runs a short warm-up before measuring.

## Scenarios

| # | Scenario | Tool | What is reported |
|---|---|---|---|
| 1 | Plain HTTP/1.1 keep-alive, 1 KB | wrk (max), wrk2 (fixed rate) | max req/s; p50/p99/p99.9 at 50% and 80% of the lower of the two max results (same offered load for both); LB CPU, RSS, req per LB core-second |
| 2 | TLS 1.3 full handshakes | wrk, `Connection: close`, patched to never resume | new TLS connections/s, LB CPU, errors; negotiated protocol/cipher/group |
| 3 | HTTPS + HTTP/2 keep-alive, 1 KB | h2load (ALPN h2, 10 streams per connection) | req/s, mean/max request time, errors |
| 4 | 100 KB and 10 MB responses | wrk | req/s, Gbit/s, p99, LB CPU and RSS |
| 5 | 10k mostly-idle keep-alive connections | wrk2 at 1000 req/s total; step extended by the ~9 s connection ramp (wrk2 opens connections 5 ms apart per thread) | idle RSS, RSS with the connections, memory per connection, connections established at the LB (`ss`), connect errors |
| 6a | Backend SIGKILLed under load, restarted later | wrk2 at fixed rate | errors (with breakdown), requests completed, p99.9 |
| 6b | Five config reloads under load | wrk2 at fixed rate | errors, reloads applied / requested |

CPU% (100 = one core) and RSS of the load balancer (sum over its process tree: HAProxy master +
workers), the backends and the load generator are sampled every 0.5 s from `/proc`
(`lib/sample.py`).

### Equivalent configuration

`conf/haproxy.cfg.tmpl` and `conf/ruststrom.toml.tmpl` are kept equivalent:

- round robin over the same 4 backends;
- health check `GET /health` every 1 s, timeout 500 ms, rise 2 / fall 3 (RustStrom v1.1 has
  no rise/fall: it ignores those keys);
- one retry on another backend when connecting fails (HAProxy `retries 1` +
  `option redispatch 1`; RustStrom v2 `[backend_pools.upstream] retries = 1`; v1.1 has no
  retries);
- same ECDSA P-256 certificate for `localhost`, TLS 1.2 and 1.3 allowed, ALPN `h2,http/1.1`;
- HAProxy `http-reuse safe` (its default) and no access log on either side;
- 4 threads each (HAProxy `nbthread 4`; RustStrom `threads = 4` for v2 and
  `TOKIO_WORKER_THREADS=4` for v1.1).

Reloads: HAProxy master-worker reload (SIGUSR2; listening sockets passed to the new worker via
`expose-fd listeners`); RustStrom config file replaced atomically (temp file + rename), picked up
by its file watcher. The report shows how many reloads each actually applied.

### sysctl tuning

`sysctl-tune.sh apply|restore|show` saves the current values to
`$BENCH_ROOT/state/sysctl.saved`, raises `net.core.somaxconn`, `net.ipv4.tcp_max_syn_backlog`,
`fs.file-max` and the ephemeral port range (to 16384-65535, with 22000-22999 added to
`ip_local_reserved_ports` so our listening ports are never handed out), and keeps
`tcp_tw_reuse` at 2 (loopback reuse, which covers this benchmark). It never lowers a value
(the host is shared). `run.sh` applies it at the start and restores it on exit, including on
failure or Ctrl-C; `apply` refuses to run while a save file from a crashed run exists. Use
`--no-sysctl` to skip it.

## Load generator patches

The tools are built from pinned sources (`setup.sh`) and patched where the stock client would
blame the load balancer for its own behaviour:

- `patches/wrk-no-tls-resumption.patch`: wrk reuses its `SSL` object across reconnects and
  `SSL_clear` keeps the session, so every "new" connection was a resumed PSK handshake
  (HAProxy reported 99% reuse). The patch drops the session so every handshake is full. h2load
  also resumes, which is why scenario 2 uses wrk.
- `patches/wrk2-eof-idle.patch`: upstream wrk2 ignores EOF, so a connection the server closes
  (HAProxy does this to idle keep-alive connections on reload) silently stops sending and the
  run under-delivers; it also counts every idle connection older than the timeout as a
  "timeout" error. The patch reconnects on EOF (an error only if a request was in flight) and
  only counts in-flight requests as timeouts.

## Output

`$BENCH_ROOT/runs/<timestamp>[-smoke]/`:

- `report.md`: environment (kernel, CPU, NUMA, versions, image digests, binary sha256, sysctls,
  tool versions) and one table per scenario, HAProxy and RustStrom side by side;
- `<scenario>/<lb>-<step>.txt`: raw tool output; `.csv`: CPU/RSS samples; `.meta`: exact
  command, load average before/after, injected events, LB counters;
- `conf/`: the generated haproxy.cfg, ruststrom.toml and nginx configs;
- `logs/`: load balancer logs, TLS probe, sysctl changes;
- `env.txt`: the raw environment record.

## Limitations

- **One host instead of three machines.** Load generator, load balancer and backends share the
  memory bus, the last-level cache and the kernel network stack. Traffic goes over loopback:
  there is no NIC, no interrupt/softirq distribution, no GRO/TSO and an MTU of 64 KiB, so large
  bodies look much cheaper than on a real network and per-packet costs are understated. Numbers
  are only meaningful as a comparison between the two load balancers under identical
  conditions, not as absolute capacity.
- **NUMA.** roberta has two NUMA nodes (0-7, 8-15). The LB cores 6-9 straddle both nodes, so
  memory locality of the LB differs from a single-socket deployment. Override `LB_CPUS` if a
  single-node layout is needed (and move the other roles accordingly).
- **Shared host.** Other workloads on the host disturb results; the load-average gate and the
  per-step warnings catch obvious cases, not short bursts. Repeat runs and compare.
- **HAProxy runs in podman** (host network, cpuset, seccomp unconfined), RustStrom runs
  natively. The container adds no network hop but HAProxy's RSS includes its master process
  and the file-descriptor tables it preallocates for `maxconn 200000`; memory *per connection*
  (scenario 5) subtracts the idle baseline and is not affected by that.
- **TLS stacks differ by design**: HAProxy uses the OpenSSL in its image (recorded in the
  report), RustStrom uses rustls. The certificate, key type and offered protocols are the
  same; the negotiated cipher and group are recorded.
- **h2load reports no percentiles**, only mean and max request time.
- **Client side TLS cost.** In the handshake test the client does as much asymmetric crypto as
  the server; with 6 client cores vs 4 LB cores the client can become the bottleneck. The
  report flags it when load generator CPU is above 85%.
- **Behaviour findings show up as errors.** For example RustStrom 1.1.0 answers a
  `Connection: close` HTTP/1.1 request with `connection: keep-alive` (copied from the backend)
  and then closes, which wrk counts as a read error per connection in scenario 2. The report
  explains such cases instead of hiding them.
