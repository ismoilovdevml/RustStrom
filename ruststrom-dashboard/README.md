# RustStrom Dashboard

Modern, real-time dashboard for monitoring and managing RustStrom load balancer.

## ✨ Features

- 📊 **Real-time Metrics** - Live visualization of requests, response times, and errors
- 🎨 **Dark Mode UI** - Beautiful, modern interface with glassmorphism design
- ⚙️ **Configuration Management** - Edit and reload config without restarting
- 🖥️ **Backend Monitoring** - Track health and performance of all backend servers
- 📈 **Historical Data** - View performance trends over time (1m, 5m, 15m)
- 🔄 **Auto-refresh** - Metrics update every 5 seconds automatically

## 🚀 Quick Start

### Prerequisites

- Node.js 18+ or npm
- RustStrom running on `http://127.0.0.1:8000`

### Installation

```bash
cd ruststrom-dashboard
npm install
```

### Development

```bash
npm run dev
```

Dashboard will be available at `http://localhost:3000`

### Production Build

```bash
npm run build
npm run preview
```

## 📊 Screenshots

### Overview Tab
- Total requests counter with real-time rate
- Average response time monitoring
- Success rate tracking
- Backend health status
- Request distribution chart
- Backend status list

### Metrics Tab
- Real-time performance charts
- Request metrics breakdown
- Response time statistics
- Error tracking
- Backend-specific metrics
- Time range filters (1m, 5m, 15m)

### Configuration Tab
- Live TOML editor with syntax highlighting
- Line count and modification status
- Save and reload functionality
- Quick reference guide
- Copy to clipboard

### Backends Tab
- Detailed backend server cards
- Health status indicators
- Performance metrics per backend
- Filter by status (All, Healthy, Slow, Unhealthy)
- Test and view logs actions

## 🔧 Configuration

### API Proxy

The dashboard proxies API requests to RustStrom. Configure in `vite.config.js`:

```javascript
server: {
  proxy: {
    '/api': {
      target: 'http://127.0.0.1:8000',
      changeOrigin: true,
      rewrite: (path) => path.replace(/^\/api/, '')
    }
  }
}
```

### Required RustStrom Endpoints

The dashboard expects these endpoints from RustStrom:

- `GET /metrics` - Prometheus metrics endpoint
- `GET /config` - Current configuration (optional)
- `POST /config` - Update configuration (optional)
- `POST /reload` - Reload configuration (optional)

## 🎨 Design Features

- **Glassmorphism** - Modern glass-like UI elements
- **Smooth Animations** - Fade-in effects and transitions
- **Responsive Layout** - Works on all screen sizes
- **Dark Theme** - Easy on the eyes with GitHub Dark color scheme
- **Real-time Updates** - Live data without page refresh
- **Chart.js Integration** - Beautiful, interactive charts

## 📦 Dependencies

- **Vue 3** - Progressive JavaScript framework
- **Chart.js** - Flexible charting library
- **Axios** - HTTP client for API requests
- **Vite** - Fast build tool and dev server

## 🛠️ Development

### Project Structure

```
ruststrom-dashboard/
├── src/
│   ├── components/
│   │   ├── OverviewTab.vue    # Main dashboard overview
│   │   ├── MetricsTab.vue     # Detailed metrics charts
│   │   ├── ConfigTab.vue      # Configuration editor
│   │   └── BackendsTab.vue    # Backend server monitoring
│   ├── App.vue                # Main app component
│   ├── main.js                # App entry point
│   └── style.css              # Global styles
├── index.html
├── vite.config.js
└── package.json
```

### Adding New Features

1. Create new component in `src/components/`
2. Add route/tab in `App.vue`
3. Update navigation in sidebar
4. Add necessary API calls

## 📝 Notes

- Metrics are stored in memory (last 60 data points = 5 minutes at 5s interval)
- Backend status is parsed from Prometheus metrics
- Configuration management requires backend API support
- Charts use Chart.js with custom dark theme colors

## 🔒 Security

- No authentication implemented (add reverse proxy auth if needed)
- CORS should be configured on RustStrom backend
- Use HTTPS in production
- Consider rate limiting for configuration changes

## 🤝 Contributing

Contributions welcome! This dashboard is part of the RustStrom project.

## 📄 License

Same as RustStrom main project.

---

🤖 Generated with [Claude Code](https://claude.com/claude-code)
