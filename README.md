# ⚡ RustStrom

High-Performance Load Balancer Built with Rust

[![License](https://img.shields.io/badge/license-GPL--3.0-blue.svg)](LICENSE)
[![Version](https://img.shields.io/badge/version-1.0.3-green.svg)](https://github.com/ismoilovdevml/RustStrom/releases)
[![Rust](https://img.shields.io/badge/rust-1.70+-orange.svg)](https://www.rust-lang.org)

---

## 📖 What is RustStrom?

RustStrom is a modern load balancer written in Rust. It distributes HTTP/HTTPS traffic across multiple backend servers, manages SSL certificates automatically, and delivers high performance.

### Key Features

- ⚡ **High Performance** - Rust's speed and safety
- 🔄 **Load Balancing Algorithms** - Round Robin, Random, IP Hash, Least Connection, Sticky Cookie
- 💚 **Health Checking** - Automatic backend server monitoring
- 🔐 **SSL/TLS** - Automatic certificates via Let's Encrypt
- 🔧 **Middlewares** - Compression, Rate Limiting, HTTPS Redirect, and more
- 🔥 **Hot Reload** - Configuration changes without restart
- 📊 **Prometheus Metrics** - Built-in monitoring support

---

## 🚀 Installation

### 1. Quick Install (Linux/macOS)

```bash
curl -sSL https://raw.githubusercontent.com/ismoilovdevml/RustStrom/main/install.sh | bash
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

---

## ⚙️ Configuration

### Basic Configuration

Create `/etc/rust-strom/config.toml`:

```toml
# Network settings
http_address = "[::]:80"
https_address = "[::]:443"

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

#### Let's Encrypt (Automatic)

```toml
[certificates."example.com"]
ACME = {
    staging = false,
    email = "admin@example.com",
    persist_dir = "/var/lib/rust-strom/acme"
}
```

#### Local Certificate

```toml
[certificates."example.com"]
Local = {
    certificate_path = "/etc/ssl/certs/example.com.crt",
    private_key_path = "/etc/ssl/private/example.com.key"
}
```

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

# Sticky Cookie - Session persistence
[backend_pools.strategy.StickyCookie]
cookie_name = "RUSTSTROM_SESSION"
http_only = true
secure = true
same_site = "Strict"
inner = { RoundRobin = {} }
```

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
After=network.target

[Service]
Type=simple
User=rust-strom
Group=rust-strom
ExecStart=/usr/local/bin/rust-strom --config /etc/rust-strom/config.toml
Restart=on-failure
RestartSec=5s

[Install]
WantedBy=multi-user.target
```

```bash
# Start service
sudo systemctl daemon-reload
sudo systemctl enable rust-strom
sudo systemctl start rust-strom
sudo systemctl status rust-strom
```

### Direct Run

```bash
rust-strom --config /etc/rust-strom/config.toml
```

---

## 📊 Monitoring

Prometheus metrics available at `/metrics` endpoint:

```bash
curl http://localhost:9090/metrics
```

### Key Metrics

- `ruststrom_requests_total` - Total requests
- `ruststrom_request_duration_seconds` - Request duration
- `ruststrom_active_connections` - Active connections
- `ruststrom_backend_health` - Backend server health
- `ruststrom_rate_limit_exceeded_total` - Rate limit hits

---

## 🔧 Troubleshooting

### View Logs

```bash
# Systemd logs
sudo journalctl -u rust-strom -f

# Debug mode
RUST_LOG=debug rust-strom --config /etc/rust-strom/config.toml
```

### Check Configuration

```bash
rust-strom --config /etc/rust-strom/config.toml --check
```

---

## 📄 License

RustStrom is distributed under the [GPL-3.0](LICENSE) license.

---
