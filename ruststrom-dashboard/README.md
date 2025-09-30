# 🎨 RustStrom Dashboard

**Professional, Real-time Monitoring Dashboard for RustStrom Load Balancer**

Built with Vue 3, Vite, and Chart.js - A modern, responsive, and feature-rich web interface for monitoring and managing your RustStrom load balancer.

![Version](https://img.shields.io/badge/version-1.0.0-blue.svg)
![Vue](https://img.shields.io/badge/Vue-3.5-green.svg)
![License](https://img.shields.io/badge/license-MIT-blue.svg)

---

## ✨ Features

### 📊 **Overview Page**
- **System Health Score** - Real-time health monitoring (0-100 score)
- **Live Activity Feed** - Watch requests in real-time with pause/resume
- **Quick Actions** - Reload config, export metrics/logs, clear cache
- **Performance Stats** - Total requests, response time, success rate, backend health
- **Request Distribution Chart** - Visual breakdown by HTTP status codes
- **Backend Status** - Real-time backend server monitoring

### 📈 **Metrics Page**
- **Request Metrics** - Total requests, RPS, active connections
- **Response Time** - Average response time with histogram data
- **Error Metrics** - Total errors, timeouts, error rate (shows "No errors" when clean)
- **Backend Performance** - Per-backend stats (requests, avg time, failures)
- **Status Code Distribution** - Visual cards for 200, 404, 500, etc.
- **Real-Time Performance Chart** - Dual-axis chart with 1m/5m/15m ranges

### 🔬 **Advanced Page**
- **7 Advanced Charts** - Request Rate, Response Time, Error Rate, Connection Pool, Backend Performance, Load Distribution, HTTP Status Codes
- **Time Range Selector** - 5m, 15m, 1h, 6h views

### 📝 **Logs Page**
- **Search, Copy, Download** - Professional log management
- **Auto-refresh** - Updates every 5 seconds

### ⚙️ **Config Page**
- **TOML Syntax Highlighting** - Beautiful code editor
- **Save & Reload** - Hot configuration updates

### 🖥️ **Backends Page**
- **Test & Logs** - Per-backend health checks and log streaming

---

## 🚀 Quick Start

```bash
cd ruststrom-dashboard
npm install
npm run dev              # Dashboard on :3000
node config-api.cjs      # Config API on :9091
```

---

**Made with ❤️ for RustStrom**
