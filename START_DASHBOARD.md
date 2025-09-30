# 🚀 RustStrom Dashboard - Quick Start Guide

Ushbu qo'llanma dashboardni ishga tushirish uchun.

## Muammolar va Yechimlar

### Asosiy Muammo
- Config API va Metrics endpoint bir xil port (9091) ishlatishga urinayotgan edi
- Dashboard disconnect bo'layotgan edi
- Advanced metrics va syntax highlighting yo'q edi

### Yechim
Eng oddiy yo'l - barcha servislarni alohida portlarda ishlatish va to'g'ri proxy sozlash.

## To'liq Yangi Setup (Oddiy va Ishlaydigan)

### 1. Backend Serverlar (8080, 8081, 8082)
```bash
# Terminal 1 - Backend 1
python3 -c "
from http.server import HTTPServer, BaseHTTPRequestHandler
import json
class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.end_headers()
        self.wfile.write(json.dumps({'server': 'backend-1', 'port': 8080}).encode())
    def log_message(self, format, *args): pass
HTTPServer(('127.0.0.1', 8080), Handler).serve_forever()
"

# Terminal 2 - Backend 2
python3 -c "
from http.server import HTTPServer, BaseHTTPRequestHandler
import json
class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.end_headers()
        self.wfile.write(json.dumps({'server': 'backend-2', 'port': 8081}).encode())
    def log_message(self, format, *args): pass
HTTPServer(('127.0.0.1', 8081), Handler).serve_forever()
"

# Terminal 3 - Backend 3
python3 -c "
from http.server import HTTPServer, BaseHTTPRequestHandler
import json
class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.end_headers()
        self.wfile.write(json.dumps({'server': 'backend-3', 'port': 8082}).encode())
    def log_message(self, format, *args): pass
HTTPServer(('127.0.0.1', 8082), Handler).serve_forever()
"
```

### 2. RustStrom (8000 HTTP, 9090 Metrics)
```bash
# test-config.toml ni yangilang:
# prometheus_metrics_address = "0.0.0.0:9090"

# Terminal 4 - RustStrom
./target/release/rust-strom --config test-config.toml
```

### 3. Dashboard (3000)
```bash
# vite.config.js ni yangilang:
# target: 'http://127.0.0.1:9090'

# Terminal 5 - Dashboard
cd ruststrom-dashboard
npm run dev
```

## Portlar Jadvali

| Service | Port | URL |
|---------|------|-----|
| Backend 1 | 8080 | http://127.0.0.1:8080 |
| Backend 2 | 8081 | http://127.0.0.1:8081 |
| Backend 3 | 8082 | http://127.0.0.1:8082 |
| RustStrom HTTP | 8000 | http://127.0.0.1:8000 |
| RustStrom Metrics | 9090 | http://127.0.0.1:9090/metrics |
| Dashboard | 3000 | http://localhost:3000 |

## Kerakli O'zgarishlar

### 1. test-config.toml
```toml
prometheus_metrics_address = "0.0.0.0:9090"  # 9091 emas!
```

### 2. vite.config.js
```js
proxy: {
  '/api': {
    target: 'http://127.0.0.1:9090',  # Metrics endpoint
    changeOrigin: true,
    rewrite: (path) => path.replace(/^\/api/, '')
  }
}
```

## Test

```bash
# Backend test
curl http://127.0.0.1:8080

# Load balancer test
curl http://127.0.0.1:8000

# Metrics test
curl http://127.0.0.1:9090/metrics

# Dashboard (browser)
http://localhost:3000
```

## Keyingi Qadamlar

1. ✅ Barcha portlarni to'g'ri sozlash
2. ✅ Disconnect muammosini hal qilish
3. 🔄 Advanced metrics page qo'shish
4. 🔄 TOML syntax highlighting
5. 🔄 Logs viewer
6. 🔄 Better error handling

---

Men hozir hamma narsani to'xtatib, to'g'ri sozlab, qaytadan ishga tushiraman.
