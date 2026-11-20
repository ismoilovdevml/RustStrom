#!/usr/bin/env node
/**
 * Config API server for the RustStrom dashboard (local admin helper).
 *
 * Security model:
 * - Binds to 127.0.0.1 only and sends no CORS headers: browsers reach it only
 *   through the dashboard's same-origin `/api` proxy (vite dev server).
 * - Every request must carry `Authorization: Bearer $RUSTSTROM_ADMIN_TOKEN`.
 *   The proxy injects the token, so it never reaches the browser.
 * - Every request must carry `X-RustStrom-Admin: 1`. Cross-site forms can not
 *   set custom headers, which blocks CSRF through the proxy.
 * - The server refuses to start without a token.
 *
 * Environment:
 *   RUSTSTROM_ADMIN_TOKEN  required, at least 32 characters
 *   RUSTSTROM_CONFIG       config file path (default ../test-config.toml)
 *   RUSTSTROM_BIN          optional rust-strom binary; when set, a new config is
 *                          validated with `--check` before it replaces the old one
 *   RUSTSTROM_METRICS_URL  default http://127.0.0.1:9090/metrics
 *   RUSTSTROM_LOG_FILE     default /tmp/ruststrom.log
 *   PORT                   default 9091
 */

const http = require('http');
const fs = require('fs');
const path = require('path');
const crypto = require('crypto');
const { execFile } = require('child_process');

const TOKEN = process.env.RUSTSTROM_ADMIN_TOKEN || '';
const CONFIG_FILE = process.env.RUSTSTROM_CONFIG || path.join(__dirname, '../test-config.toml');
const RUSTSTROM_BIN = process.env.RUSTSTROM_BIN || '';
const METRICS_URL = new URL(process.env.RUSTSTROM_METRICS_URL || 'http://127.0.0.1:9090/metrics');
const LOG_FILE = process.env.RUSTSTROM_LOG_FILE || '/tmp/ruststrom.log';
const PORT = Number(process.env.PORT || 9091);
const MAX_BODY_BYTES = 1024 * 1024;

if (TOKEN.length < 32) {
  console.error('RUSTSTROM_ADMIN_TOKEN must be set to a random value of at least 32 characters.');
  console.error('Example: export RUSTSTROM_ADMIN_TOKEN=$(openssl rand -hex 32)');
  process.exit(1);
}

function send(res, status, body, contentType = 'application/json') {
  res.writeHead(status, {
    'Content-Type': contentType,
    'Cache-Control': 'no-store',
    'X-Content-Type-Options': 'nosniff',
  });
  res.end(typeof body === 'string' ? body : JSON.stringify(body));
}

function isAuthorized(req) {
  const header = req.headers.authorization || '';
  const expected = Buffer.from(`Bearer ${TOKEN}`);
  const actual = Buffer.from(header);
  return actual.length === expected.length && crypto.timingSafeEqual(actual, expected);
}

function readBody(req, res, callback) {
  const chunks = [];
  let size = 0;
  req.on('data', (chunk) => {
    size += chunk.length;
    if (size > MAX_BODY_BYTES) {
      send(res, 413, { error: 'Config too large' });
      req.destroy();
      return;
    }
    chunks.push(chunk);
  });
  req.on('end', () => {
    if (size <= MAX_BODY_BYTES) callback(Buffer.concat(chunks).toString('utf8'));
  });
}

function validateConfig(file, callback) {
  if (!RUSTSTROM_BIN) return callback(null);
  execFile(RUSTSTROM_BIN, ['--check', '--config', file], { timeout: 10000 }, (error, stdout, stderr) => {
    callback(error ? (stderr || stdout || error.message).trim() : null);
  });
}

function writeConfig(body, res) {
  // Write next to the target and rename, so the update is atomic and the
  // RustStrom file watcher never sees a half written file.
  const tmp = path.join(path.dirname(CONFIG_FILE), `.${path.basename(CONFIG_FILE)}.${process.pid}.tmp`);
  try {
    fs.writeFileSync(tmp, body, { encoding: 'utf8', mode: 0o640 });
  } catch (error) {
    return send(res, 500, { error: 'Failed to write config: ' + error.message });
  }
  validateConfig(tmp, (validationError) => {
    if (validationError) {
      fs.rmSync(tmp, { force: true });
      return send(res, 400, { error: 'Invalid configuration', details: validationError });
    }
    try {
      fs.renameSync(tmp, CONFIG_FILE);
      send(res, 200, { success: true, message: 'Config saved' });
    } catch (error) {
      fs.rmSync(tmp, { force: true });
      send(res, 500, { error: 'Failed to write config: ' + error.message });
    }
  });
}

const server = http.createServer((req, res) => {
  if (!isAuthorized(req) || req.headers['x-ruststrom-admin'] !== '1') {
    return send(res, 401, { error: 'Unauthorized' });
  }

  if (req.url === '/metrics' && req.method === 'GET') {
    const metricsReq = http.request(METRICS_URL, { method: 'GET', timeout: 5000 }, (metricsRes) => {
      res.writeHead(metricsRes.statusCode, {
        'Content-Type': metricsRes.headers['content-type'] || 'text/plain',
        'Cache-Control': 'no-store',
      });
      metricsRes.pipe(res);
    });
    metricsReq.on('timeout', () => metricsReq.destroy(new Error('timeout')));
    metricsReq.on('error', (error) => {
      console.error('Metrics proxy error:', error.message);
      send(res, 502, 'Error fetching metrics', 'text/plain');
    });
    metricsReq.end();
    return;
  }

  if (req.url === '/config' && req.method === 'GET') {
    try {
      return send(res, 200, fs.readFileSync(CONFIG_FILE, 'utf8'), 'text/plain');
    } catch (error) {
      return send(res, 500, { error: 'Failed to read config: ' + error.message });
    }
  }

  if (req.url === '/config' && req.method === 'POST') {
    return readBody(req, res, (body) => writeConfig(body, res));
  }

  if (req.url === '/reload' && req.method === 'POST') {
    // RustStrom reloads automatically when the config file changes.
    return send(res, 200, { success: true, message: 'RustStrom reloads the config automatically on file change' });
  }

  if (req.url === '/logs' && req.method === 'GET') {
    try {
      const logs = fs.readFileSync(LOG_FILE, 'utf8').split('\n').slice(-100).join('\n');
      return send(res, 200, logs, 'text/plain');
    } catch (error) {
      return send(res, 200, 'No logs available', 'text/plain');
    }
  }

  send(res, 404, 'Not Found', 'text/plain');
});

server.listen(PORT, '127.0.0.1', () => {
  console.log(`Config API listening on http://127.0.0.1:${PORT} (config: ${CONFIG_FILE})`);
});
