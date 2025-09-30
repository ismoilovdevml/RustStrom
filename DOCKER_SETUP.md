# 🐳 Docker Setup - RustStrom with Dashboard

Complete Docker setup for running RustStrom load balancer with monitoring dashboard.

## 🚀 Quick Start

### One Command Setup

```bash
docker-compose up --build
```

This will start:
- ✅ 3 backend test servers (ports 8080, 8081, 8082)
- ✅ RustStrom load balancer (port 8000)
- ✅ Monitoring dashboard (port 3000)
- ✅ Metrics endpoint (port 9090)

### Access Services

- **Load Balancer**: http://localhost:8000
- **Dashboard**: http://localhost:3000
- **Metrics**: http://localhost:9090/metrics

## 📋 Services

### Backend Servers
Three Python HTTP servers for testing:
- `backend1` - Internal port 8080
- `backend2` - Internal port 8081
- `backend3` - Internal port 8082

### RustStrom
High-performance load balancer:
- Port `8000` - HTTP traffic (load balanced)
- Port `9090` - Prometheus metrics endpoint
- Round-robin load balancing strategy
- Health checking enabled

### Dashboard
Vue.js monitoring interface:
- Port `3000` - Web interface
- Real-time metrics visualization
- Configuration management
- Backend health monitoring

## 🛠️ Docker Commands

### Start Services

```bash
# Start in foreground
docker-compose up

# Start in background
docker-compose up -d

# Build and start
docker-compose up --build

# Start specific service
docker-compose up ruststrom
```

### Stop Services

```bash
# Stop all services
docker-compose down

# Stop and remove volumes
docker-compose down -v

# Stop specific service
docker-compose stop dashboard
```

### View Logs

```bash
# All services
docker-compose logs -f

# Specific service
docker-compose logs -f ruststrom

# Last 100 lines
docker-compose logs --tail=100 ruststrom
```

### Restart Services

```bash
# Restart all
docker-compose restart

# Restart specific service
docker-compose restart ruststrom
```

## 📊 Testing the Setup

### 1. Test Load Balancer

```bash
# Send requests (should round-robin between backends)
for i in {1..10}; do
  curl http://localhost:8000
  echo ""
done
```

### 2. Check Metrics

```bash
# View Prometheus metrics
curl http://localhost:9090/metrics

# Filter specific metrics
curl http://localhost:9090/metrics | grep requests_total
```

### 3. Open Dashboard

Open http://localhost:3000 in your browser to see:
- Real-time request statistics
- Backend health status
- Performance charts
- Configuration editor

## ⚙️ Configuration

### Custom Configuration

Edit `docker-config.toml` to customize:
- Backend addresses
- Load balancing strategy
- Health check settings
- Port bindings

```toml
[[backend_pools]]
matcher = "Path('/')"
addresses = ["backend1:8080", "backend2:8081", "backend3:8082"]
schemes = ["HTTP"]
strategy = { RoundRobin = {} }  # or Random, IPHash, LeastConnection, StickyCookie
```

### Environment Variables

Modify `docker-compose.yml` to add environment variables:

```yaml
services:
  ruststrom:
    environment:
      - RUST_LOG=info
      - RUST_BACKTRACE=1
```

## 🏗️ Building Images

### Build Individually

```bash
# RustStrom
docker build -t ruststrom:latest .

# Dashboard
docker build -t ruststrom-dashboard:latest ./ruststrom-dashboard
```

### Using Docker Compose

```bash
# Build all services
docker-compose build

# Build specific service
docker-compose build ruststrom

# Build with no cache
docker-compose build --no-cache
```

## 🔍 Troubleshooting

### Check Service Status

```bash
docker-compose ps
```

### Check Service Health

```bash
# Health check status
docker inspect --format='{{.State.Health.Status}}' ruststrom_ruststrom_1

# All health checks
docker-compose ps | grep healthy
```

### Access Container Shell

```bash
# RustStrom
docker-compose exec ruststrom sh

# Dashboard
docker-compose exec dashboard sh
```

### View Resource Usage

```bash
docker stats
```

### Common Issues

**Issue**: Cannot connect to backend servers
```bash
# Check if backends are running
docker-compose ps backend1 backend2 backend3

# Check backend logs
docker-compose logs backend1
```

**Issue**: Dashboard not loading
```bash
# Check dashboard logs
docker-compose logs dashboard

# Verify nginx is running
docker-compose exec dashboard nginx -t
```

**Issue**: Metrics endpoint not responding
```bash
# Check RustStrom logs
docker-compose logs ruststrom

# Test metrics endpoint directly
docker-compose exec ruststrom curl http://localhost:9090/metrics
```

## 📦 Production Deployment

### 1. Use Environment-Specific Configs

Create `docker-compose.prod.yml`:

```yaml
version: '3.8'

services:
  ruststrom:
    deploy:
      replicas: 2
      resources:
        limits:
          cpus: '2'
          memory: 1G
    logging:
      driver: "json-file"
      options:
        max-size: "10m"
        max-file: "3"
```

Deploy:
```bash
docker-compose -f docker-compose.yml -f docker-compose.prod.yml up -d
```

### 2. Use Docker Swarm

```bash
# Initialize swarm
docker swarm init

# Deploy stack
docker stack deploy -c docker-compose.yml ruststrom
```

### 3. Use Kubernetes

Convert to Kubernetes manifests:
```bash
kompose convert -f docker-compose.yml
```

## 🔐 Security

### Network Isolation

```yaml
networks:
  frontend:
    driver: bridge
  backend:
    driver: bridge
    internal: true  # No external access
```

### Resource Limits

```yaml
services:
  ruststrom:
    deploy:
      resources:
        limits:
          cpus: '2'
          memory: 1G
        reservations:
          cpus: '1'
          memory: 512M
```

### Read-Only Filesystem

```yaml
services:
  ruststrom:
    read_only: true
    tmpfs:
      - /tmp
```

## 📈 Monitoring

### Integration with Prometheus

Add to your `prometheus.yml`:

```yaml
scrape_configs:
  - job_name: 'ruststrom'
    static_configs:
      - targets: ['localhost:9090']
```

### Integration with Grafana

Import RustStrom dashboard from `grafana-dashboard.json`

## 🚀 Performance Tuning

### Increase Worker Threads

```yaml
services:
  ruststrom:
    environment:
      - TOKIO_WORKER_THREADS=8
```

### Adjust Health Check Intervals

```yaml
healthcheck:
  interval: 5s
  timeout: 3s
  retries: 3
  start_period: 10s
```

## 📝 Notes

- Default configuration uses Round Robin load balancing
- Health checks run every 10 seconds
- Metrics are exposed on port 9090
- Dashboard auto-refreshes every 5 seconds
- All services are on the same Docker network for communication

---

🤖 Generated with [Claude Code](https://claude.com/claude-code)
