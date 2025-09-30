#!/bin/bash
# 🚀 RustStrom Complete Stack Starter
# Ushbu skript hamma narsani ishga tushiradi

set -e

echo "🧹 Cleaning up old processes..."
lsof -ti:8080,8081,8082,8000,9090,3000,3001 2>/dev/null | xargs kill -9 2>/dev/null || true
sleep 2

echo "🚀 Starting Backend Servers..."

# Backend 1 (8080)
python3 -c "
from http.server import HTTPServer, BaseHTTPRequestHandler
import json
class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Access-Control-Allow-Origin', '*')
        self.end_headers()
        self.wfile.write(json.dumps({'server': 'backend-1', 'port': 8080, 'status': 'healthy'}).encode())
    def log_message(self, format, *args): pass
HTTPServer(('127.0.0.1', 8080), Handler).serve_forever()
" > /tmp/backend1.log 2>&1 &
echo "  ✅ Backend 1 started on port 8080 (PID: $!)"

# Backend 2 (8081)
python3 -c "
from http.server import HTTPServer, BaseHTTPRequestHandler
import json
class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Access-Control-Allow-Origin', '*')
        self.end_headers()
        self.wfile.write(json.dumps({'server': 'backend-2', 'port': 8081, 'status': 'healthy'}).encode())
    def log_message(self, format, *args): pass
HTTPServer(('127.0.0.1', 8081), Handler).serve_forever()
" > /tmp/backend2.log 2>&1 &
echo "  ✅ Backend 2 started on port 8081 (PID: $!)"

# Backend 3 (8082)
python3 -c "
from http.server import HTTPServer, BaseHTTPRequestHandler
import json
class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Access-Control-Allow-Origin', '*')
        self.end_headers()
        self.wfile.write(json.dumps({'server': 'backend-3', 'port': 8082, 'status': 'healthy'}).encode())
    def log_message(self, format, *args): pass
HTTPServer(('127.0.0.1', 8082), Handler).serve_forever()
" > /tmp/backend3.log 2>&1 &
echo "  ✅ Backend 3 started on port 8082 (PID: $!)"

sleep 2

echo ""
echo "⚡ Starting RustStrom Load Balancer..."
./target/release/rust-strom --config test-config.toml > /tmp/ruststrom.log 2>&1 &
RUST_PID=$!
echo "  ✅ RustStrom started (PID: $RUST_PID)"
echo "     - HTTP: http://127.0.0.1:8000"
echo "     - Metrics: http://127.0.0.1:9090/metrics"

sleep 3

echo ""
echo "🎨 Starting Dashboard..."
cd ruststrom-dashboard
npm run dev > /tmp/dashboard.log 2>&1 &
DASH_PID=$!
echo "  ✅ Dashboard starting (PID: $DASH_PID)"
echo "     - URL: http://localhost:3000"

sleep 5

echo ""
echo "🧪 Testing Services..."
echo ""
echo "Backend 1:"
curl -s http://127.0.0.1:8080 | jq 2>/dev/null || curl -s http://127.0.0.1:8080
echo ""
echo "Load Balancer:"
curl -s http://127.0.0.1:8000 | jq 2>/dev/null || curl -s http://127.0.0.1:8000
echo ""
echo "Metrics:"
curl -s http://127.0.0.1:9090/metrics | head -5

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "✅ All services started successfully!"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""
echo "📊 Dashboard:    http://localhost:3000"
echo "⚡ Load Balancer: http://127.0.0.1:8000"
echo "📈 Metrics:       http://127.0.0.1:9090/metrics"
echo ""
echo "View logs:"
echo "  tail -f /tmp/ruststrom.log"
echo "  tail -f /tmp/dashboard.log"
echo ""
echo "Stop all:"
echo "  lsof -ti:8080,8081,8082,8000,9090,3000 | xargs kill -9"
echo ""
