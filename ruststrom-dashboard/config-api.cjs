#!/usr/bin/env node
/**
 * Simple Config API Server for RustStrom Dashboard
 * Provides endpoints for reading/writing configuration
 */

const http = require('http');
const fs = require('fs');
const path = require('path');

const CONFIG_FILE = path.join(__dirname, '../test-config.toml');
const PORT = 9091;

const server = http.createServer((req, res) => {
  // Enable CORS
  res.setHeader('Access-Control-Allow-Origin', '*');
  res.setHeader('Access-Control-Allow-Methods', 'GET, POST, OPTIONS');
  res.setHeader('Access-Control-Allow-Headers', 'Content-Type');

  if (req.method === 'OPTIONS') {
    res.writeHead(200);
    res.end();
    return;
  }

  // GET /metrics - Proxy to RustStrom metrics
  if (req.url === '/metrics' && req.method === 'GET') {
    const metricsReq = http.request({
      host: '127.0.0.1',
      port: 9091,
      path: '/metrics',
      method: 'GET'
    }, (metricsRes) => {
      res.writeHead(metricsRes.statusCode, metricsRes.headers);
      metricsRes.pipe(res);
    });

    metricsReq.on('error', (error) => {
      console.error('Metrics proxy error:', error);
      res.writeHead(500, { 'Content-Type': 'text/plain' });
      res.end('Error fetching metrics');
    });

    metricsReq.end();
    return;
  }

  // GET /config - Read configuration file
  if (req.url === '/config' && req.method === 'GET') {
    try {
      const config = fs.readFileSync(CONFIG_FILE, 'utf8');
      res.writeHead(200, { 'Content-Type': 'text/plain' });
      res.end(config);
    } catch (error) {
      res.writeHead(500, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({ error: 'Failed to read config: ' + error.message }));
    }
    return;
  }

  // POST /config - Write configuration file
  if (req.url === '/config' && req.method === 'POST') {
    let body = '';
    req.on('data', chunk => {
      body += chunk.toString();
    });

    req.on('end', () => {
      try {
        fs.writeFileSync(CONFIG_FILE, body, 'utf8');
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ success: true, message: 'Config saved' }));
      } catch (error) {
        res.writeHead(500, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ error: 'Failed to write config: ' + error.message }));
      }
    });
    return;
  }

  // POST /reload - Reload configuration
  if (req.url === '/reload' && req.method === 'POST') {
    res.writeHead(200, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify({
      success: true,
      message: 'Config reload triggered (restart RustStrom manually)'
    }));
    return;
  }

  // GET /logs - Return recent logs
  if (req.url === '/logs' && req.method === 'GET') {
    try {
      const logs = fs.readFileSync('/tmp/ruststrom.log', 'utf8').split('\n').slice(-100).join('\n');
      res.writeHead(200, { 'Content-Type': 'text/plain' });
      res.end(logs);
    } catch (error) {
      res.writeHead(200, { 'Content-Type': 'text/plain' });
      res.end('No logs available');
    }
    return;
  }

  // 404
  res.writeHead(404, { 'Content-Type': 'text/plain' });
  res.end('Not Found');
});

server.listen(PORT, '127.0.0.1', () => {
  console.log(`🚀 Config API server running on http://127.0.0.1:${PORT}`);
  console.log(`   GET  /metrics - Prometheus metrics`);
  console.log(`   GET  /config  - Read configuration`);
  console.log(`   POST /config  - Write configuration`);
  console.log(`   POST /reload  - Reload configuration`);
  console.log(`   GET  /logs    - View logs`);
});
