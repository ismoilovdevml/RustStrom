# RustStrom end-to-end tests

A black-box suite: it starts the `rust-strom` binary with generated TOML configs,
real backend processes (`harness/backend_server.py`, stdlib only) and a throwaway
CA, then talks to it over TCP, TLS, HTTP/1.1, HTTP/2 and WebSocket. It does not
link any RustStrom code, so the same suite runs against v1.1 and v2.

## Running

```sh
RUSTSTROM_BIN=/path/to/rust-strom tests/e2e/run.sh            # whole suite
RUSTSTROM_BIN=/path/to/rust-strom tests/e2e/run.sh -m "not slow"  # skip ~75 s of timeout tests
RUSTSTROM_BIN=/path/to/rust-strom tests/e2e/run.sh test_reload.py -k atomic
```

`run.sh` creates a virtualenv (`tests/e2e/.venv`, or `$E2E_VENV`) with pytest,
pytest-timeout and h2 on first use. Requirements: Linux, python3.9+, openssl CLI.
Without `RUSTSTROM_BIN` the suite uses `target/release/rust-strom`.

| Variable | Default | Meaning |
|---|---|---|
| `RUSTSTROM_BIN` | `target/release/rust-strom` | binary under test |
| `RUSTSTROM_TARGET` | detected from `--version` | `v1` or `v2`, controls the `v2` marker |
| `E2E_PORT_RANGE` | `21000-21999` | the only TCP ports the suite binds (LB, backends, metrics) |
| `E2E_HEADER_TIMEOUT` | `30` | seconds the proxy may take to drop a slowloris client |
| `E2E_VENV` | `tests/e2e/.venv` | virtualenv location |

Every listener in every generated config is explicit (`http_address`,
`https_address`, `prometheus_metrics_address`), so the binary never binds a port
outside the range.

## Markers

- `v2`: a feature only v2 must have (retries, configurable read timeout,
  WebSocket, upstream TLS, chunked 413, in-flight LeastConnection, zero-error
  reload). Against a 1.x binary these run as non-strict `xfail`, so the report
  shows what v1 cannot do. Against 2.x they must pass.
- `slow`: waits for a built-in timeout (30 s backend timeout, slowloris header
  timeout, TLS handshake timeout).

## Layout

```
conftest.py           binary/version detection, v2 -> xfail, PKI and Stack fixtures
harness/
  backend_server.py   JSON echo backend: /__health, /__ctl, /__stats, ?sleep=MS, WebSocket echo, TLS
  procs.py            Backend, LoadBalancer (start/stop/reload/log) and Stack (teardown)
  config.py           TOML builders: config(), pool(), strategies
  client.py           HTTP/1.1 (source IP, SNI, chunked), raw bytes, HTTP/2, WebSocket, LoadLoop
  certs.py            CA + leaf certificate with SAN for every test host
  ports.py            port allocator restricted to E2E_PORT_RANGE
test_routing.py       matcher language, 404, HTTPS, HTTP/2 :authority
test_strategies.py    RoundRobin, Random, IPHash, StickyCookie, LeastConnection
test_security.py      cookies, OPTIONS *, header spoofing, rate limit, MaxBodySize,
                      idle sockets, slowloris, malformed HTTP/TLS
test_resilience.py    failover, retry, 504, 502
test_reload.py        atomic rename, in-place write, invalid config, reload under load
test_lifecycle.py     SIGTERM/SIGINT drain, --check, --version
test_protocols.py     ALPN, TLS versions, SNI, h2, large bodies, WebSocket, upstream TLS
test_metrics.py       Prometheus endpoint and metric names
```

On failure the last lines of the rust-strom log are attached to the pytest report.
