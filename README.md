# ⚡ RustStrom

HTTP/HTTPS Load Balancer Built with Rust

[![License](https://img.shields.io/badge/license-GPL--3.0-blue.svg)](LICENSE)
[![Version](https://img.shields.io/badge/version-1.1.0-green.svg)](https://github.com/ismoilovdevml/RustStrom/releases)
[![Rust](https://img.shields.io/badge/rust-1.70+-orange.svg)](https://www.rust-lang.org)

---

## 📖 What is RustStrom?

RustStrom is a load balancer written in Rust (tokio + hyper). It distributes HTTP/HTTPS traffic across multiple backend servers, terminates TLS (local certificates or Let's Encrypt) and exposes Prometheus metrics.

### Key Features

- ⚡ **Async I/O** - tokio/hyper, HTTP/1.1 and HTTP/2 (HTTP/2 over TLS via ALPN)
- 🔄 **Load Balancing Algorithms** - Round Robin, Random, IP Hash, Least Connection, Sticky Cookie
- 💚 **Health Checking** - Automatic backend server monitoring
- 🔐 **SSL/TLS** - Local certificates or Let's Encrypt (ACME)
- 🔧 **Middlewares** - Compression, Rate Limiting, HTTPS Redirect, and more
- 🔥 **Hot Reload** - Configuration changes without restart; an invalid config is rejected and the old one kept
- 🛑 **Graceful Shutdown** - SIGTERM/SIGINT drain in-flight requests for up to 30 s
- 📊 **Prometheus Metrics** - Built-in monitoring support

---

## 🚀 Installation

### 1. Quick Install (Linux/macOS)

```bash
curl -sSL https://raw.githubusercontent.com/ismoilovdevml/RustStrom/main/install.sh | sudo bash
```

### 2. Build from Source

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Clone repository
git clone https://github.com/ismoilovdevml/RustStrom.git
cd RustStrom

# Build
cargo build --release

# Binary location
./target/release/rust-strom
```

### 3. Docker Compose (functional demo)

```bash
docker compose up --build
curl http://localhost:8000/
```

Uses `docker-config.toml` and three `python http.server` backends. It is a functional demo, not a benchmark setup.

---

## ⚙️ Configuration

### Basic Configuration

Create `/etc/rust-strom/config.toml`:

```toml
# Network settings
http_address = "[::]:80"
https_address = "[::]:443"

# Prometheus metrics endpoint (optional, default "127.0.0.1:9090")
# prometheus_metrics_address = "127.0.0.1:9090"

# Health check interval
[health_interval]
check_every = 10  # seconds

# Backend pool
[[backend_pools]]
matcher = "Host('example.com')"
addresses = ["127.0.0.1:8080", "127.0.0.1:8081", "127.0.0.1:8082"]
schemes = ["HTTP", "HTTPS"]
strategy = { RoundRobin = {} }

# Health check settings
[backend_pools.health_config]
slow_threshold = 300  # milliseconds
timeout = 500         # milliseconds
path = "/"
```

### SSL/TLS Configuration

#### Local Certificate

```toml
[certificates."example.com"]
type = "Local"
certificate_path = "/etc/ssl/certs/example.com.crt"
private_key_path = "/etc/ssl/private/example.com.key"
```

RSA, ECDSA and Ed25519 keys (PKCS#1 / PKCS#8 PEM) are supported.

#### Let's Encrypt (ACME)

```toml
[certificates."example.com"]
type = "ACME"
staging = false
email = "admin@example.com"
persist_dir = "/var/lib/rust-strom/acme"
```

Limitations: ACME certificates are not requested at startup but on the first configuration reload (e.g. `touch` the config file once port 80 is reachable), and they are not renewed automatically - a new certificate is only ordered on a reload after the old one has expired.

HTTPS listeners negotiate HTTP/2 or HTTP/1.1 via ALPN.

### Matchers

```toml
matcher = "Host('example.com') && PathRegexp('^/api/')"
```

Available: `Host`, `HostRegexp`, `Path` (exact match), `PathRegexp`, `Method`, `Query('key', 'value')`, combined with `&&`, `||` and parentheses.

### Load Balancing Algorithms

```toml
# Round Robin - Sequential distribution
strategy = { RoundRobin = {} }

# Random - Random selection
strategy = { Random = {} }

# IP Hash - IP-based routing
strategy = { IPHash = {} }

# Least Connection - Least active connections
strategy = { LeastConnection = {} }

# Sticky Cookie - Session persistence (must be one inline table;
# `inner` picks the backend for new sessions, same_site: Strict | Lax | None)
strategy = { StickyCookie = { cookie_name = "RUSTSTROM_SESSION", http_only = true, secure = true, same_site = "Strict", inner = { RoundRobin = {} } } }
```

Sticky cookie values are opaque hashes. Cookies issued by versions before 1.1.0 are ignored once and then reissued.

### Middlewares

```toml
[backend_pools.middlewares.RateLimiter]
limit = 100          # requests per client IP
window_sec = 60

[backend_pools.middlewares.MaxBodySize]
limit = 10485760     # bytes

[backend_pools.middlewares.Compression]

[backend_pools.middlewares.HttpsRedirector]

[backend_pools.middlewares.CustomErrorPages]
location = "/var/www/errors"   # serves <location>/<code>.html, read once at config load
errors = [404, 502]
```

See [`configs/config.toml`](configs/config.toml) for a fully commented example.

---

## 🏃 Running

### Systemd Service (Linux)

```bash
# Create systemd service file
sudo nano /etc/systemd/system/rust-strom.service
```

```ini
[Unit]
Description=RustStrom Load Balancer
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=rust-strom
Group=rust-strom
ExecStartPre=/usr/local/bin/rust-strom --config /etc/rust-strom/config.toml --check
ExecStart=/usr/local/bin/rust-strom --config /etc/rust-strom/config.toml
Restart=on-failure
RestartSec=5
KillSignal=SIGTERM
TimeoutStopSec=35
AmbientCapabilities=CAP_NET_BIND_SERVICE
CapabilityBoundingSet=CAP_NET_BIND_SERVICE
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/var/log/rust-strom /etc/rust-strom
LimitNOFILE=1048576

[Install]
WantedBy=multi-user.target
```

`install.sh` writes this unit for you.

```bash
# Start service
sudo systemctl daemon-reload
sudo systemctl enable rust-strom
sudo systemctl start rust-strom
sudo systemctl status rust-strom
```

### Direct Run

```bash
rust-strom --config /etc/rust-strom/config.toml   # or -c
rust-strom --version
```

### Logging

The log level is set with the `LOG_LEVEL` environment variable (`RUST_LOG` is accepted as a fallback): `off`, `error`, `warn`, `info` (default), `debug`, `trace`.

```bash
LOG_LEVEL=debug rust-strom --config /etc/rust-strom/config.toml
```

---

## 📊 Monitoring

Prometheus metrics are served at `/metrics` on `prometheus_metrics_address` (default `127.0.0.1:9090`):

```bash
curl http://127.0.0.1:9090/metrics
```

> **Breaking change in 1.1.0:** metrics used to listen on `0.0.0.0:9090`. To scrape remotely, set
> `prometheus_metrics_address = "0.0.0.0:9090"`. The endpoint is unauthenticated, so restrict it with a firewall.

### Key Metrics

- `http_requests_total` - Total requests
- `http_request_duration_seconds` - Request duration histogram
- `active_http_connections` - Active connections
- `http_status_codes_total{code}` - Responses by status code
- `http_errors_total` / `http_timeouts_total` - 5xx responses / timeouts
- `backend_failures_total{backend}` - Failed backend requests
- `backend_response_time_seconds{backend}` - Backend latency histogram
- `bytes_transferred_total{direction}` - Bytes in/out

---

## 🔧 Troubleshooting

### View Logs

```bash
# Systemd logs
sudo journalctl -u rust-strom -f

# Debug mode
LOG_LEVEL=debug rust-strom --config /etc/rust-strom/config.toml
```

### Check Configuration

Parses and validates the configuration and exits with status 0 if it is valid:

```bash
rust-strom --config /etc/rust-strom/config.toml --check
```

## 📝 Changelog

See [CHANGELOG.md](CHANGELOG.md).

---

## Dashboard (development)

`ruststrom-dashboard/` is a Vue dashboard served by the Vite dev server. Its config API
(`config-api.cjs`) can read and replace the RustStrom config file, so it is protected:

```bash
export RUSTSTROM_ADMIN_TOKEN=$(openssl rand -hex 32)     # required, >= 32 chars
export RUSTSTROM_CONFIG=/etc/rust-strom/config.toml
export RUSTSTROM_BIN=/usr/local/bin/rust-strom            # optional: validate with --check before saving
node ruststrom-dashboard/config-api.cjs &                 # listens on 127.0.0.1:9091
cd ruststrom-dashboard && npm install && npm run dev      # proxies /api and injects the token
```

The token is added by the dev server proxy and never reaches the browser; requests also need the
`X-RustStrom-Admin: 1` header, which the dashboard sends and cross-site forms cannot.
Keep both processes on localhost (or behind an authenticated reverse proxy).


## 📄 License

RustStrom is distributed under the [GPL-3.0](LICENSE) license.

---
