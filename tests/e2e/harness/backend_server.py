#!/usr/bin/env python3
"""Standalone test backend for the RustStrom e2e suite (stdlib only).

Every request is answered with a JSON document describing what the backend
received, so tests can assert on routing, forwarded headers and bodies:

    {"backend": NAME, "method": ..., "path": ..., "headers": [[name, value], ...],
     "body_len": N}

Special paths (never delayed):
    /__health              200 "ok", or 503 while marked unhealthy
    /__ctl?delay=MS&healthy=0|1   change the default delay / health state
    /__stats               {"requests": N, "inflight": N}

Any other path honours ``?sleep=MS`` (or the default delay) before answering.
A request with ``Upgrade: websocket`` gets a WebSocket handshake and an echo loop.
"""

import argparse
import base64
import hashlib
import json
import socket
import ssl
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlsplit

WS_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"


class State:
    def __init__(self, name, delay_ms):
        self.name = name
        self.delay_ms = delay_ms
        self.healthy = True
        self.requests = 0
        self.inflight = 0
        self.lock = threading.Lock()


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server_version = "e2e-backend"
    sys_version = ""

    def log_message(self, fmt, *args):  # keep the test output quiet
        pass

    def setup(self):
        if isinstance(self.request, ssl.SSLSocket):
            self.request.settimeout(10)
            self.request.do_handshake()
            self.request.settimeout(None)
        super().setup()

    # --- helpers -----------------------------------------------------------
    def _read_body(self):
        if "chunked" in self.headers.get("Transfer-Encoding", "").lower():
            total = 0
            while True:
                line = self.rfile.readline(65537)
                if not line:
                    raise ConnectionError("eof in chunked body")
                size = int(line.split(b";")[0].strip(), 16)
                if size == 0:
                    # trailers until empty line
                    while True:
                        trailer = self.rfile.readline(65537)
                        if trailer in (b"\r\n", b"\n", b""):
                            break
                    return total
                data = self.rfile.read(size)
                if len(data) != size:
                    raise ConnectionError("short chunk")
                total += size
                self.rfile.readline(3)
        length = int(self.headers.get("Content-Length") or 0)
        remaining = length
        while remaining > 0:
            data = self.rfile.read(min(remaining, 65536))
            if not data:
                raise ConnectionError("eof in body")
            remaining -= len(data)
        return length

    def _send(self, status, body, content_type="application/json", extra=None):
        payload = body if isinstance(body, bytes) else body.encode()
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(payload)))
        self.send_header("X-Backend", self.server.state.name)
        for name, value in (extra or []):
            self.send_header(name, value)
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(payload)

    # --- dispatch ----------------------------------------------------------
    def _handle(self):
        state = self.server.state
        url = urlsplit(self.path)
        query = parse_qs(url.query)
        if url.path == "/__health":
            self._read_body()
            if state.healthy:
                self._send(200, "ok", "text/plain")
            else:
                self._send(503, "unhealthy", "text/plain")
            return
        if url.path == "/__ctl":
            self._read_body()
            if "delay" in query:
                state.delay_ms = int(query["delay"][0])
            if "healthy" in query:
                state.healthy = query["healthy"][0] == "1"
            self._send(200, "ok", "text/plain")
            return
        if url.path == "/__stats":
            self._read_body()
            with state.lock:
                stats = {"requests": state.requests, "inflight": state.inflight}
            self._send(200, json.dumps(stats))
            return
        if self.headers.get("Upgrade", "").lower() == "websocket":
            self._websocket()
            return

        with state.lock:
            state.requests += 1
            state.inflight += 1
        try:
            try:
                body_len = self._read_body()
            except (ValueError, ConnectionError, OSError):
                self.close_connection = True
                self._send(400, json.dumps({"backend": state.name, "error": "bad body"}))
                return
            delay = int(query.get("sleep", [state.delay_ms])[0])
            if delay > 0:
                time.sleep(delay / 1000.0)
            doc = {
                "backend": state.name,
                "method": self.command,
                "path": self.path,
                "version": self.request_version,
                "headers": [[k.lower(), v] for k, v in self.headers.items()],
                "body_len": body_len,
            }
            self._send(200, json.dumps(doc))
        finally:
            with state.lock:
                state.inflight -= 1

    def _websocket(self):
        key = self.headers.get("Sec-WebSocket-Key", "")
        accept = base64.b64encode(hashlib.sha1((key + WS_GUID).encode()).digest()).decode()
        self.send_response(101, "Switching Protocols")
        self.send_header("Upgrade", "websocket")
        self.send_header("Connection", "Upgrade")
        self.send_header("Sec-WebSocket-Accept", accept)
        self.send_header("X-Backend", self.server.state.name)
        self.end_headers()
        self.wfile.flush()
        self.close_connection = True
        while True:
            frame = read_ws_frame(self.rfile)
            if frame is None:
                return
            opcode, payload = frame
            if opcode == 0x8:  # close
                self.wfile.write(encode_ws_frame(0x8, payload[:2], mask=False))
                self.wfile.flush()
                return
            if opcode == 0x9:  # ping
                self.wfile.write(encode_ws_frame(0xA, payload, mask=False))
            else:
                self.wfile.write(encode_ws_frame(opcode, payload, mask=False))
            self.wfile.flush()

    do_GET = do_POST = do_PUT = do_DELETE = do_PATCH = do_OPTIONS = do_HEAD = _handle


def read_ws_frame(rfile):
    header = rfile.read(2)
    if len(header) < 2:
        return None
    opcode = header[0] & 0x0F
    masked = header[1] & 0x80
    length = header[1] & 0x7F
    if length == 126:
        length = int.from_bytes(rfile.read(2), "big")
    elif length == 127:
        length = int.from_bytes(rfile.read(8), "big")
    mask = rfile.read(4) if masked else b""
    payload = rfile.read(length)
    if masked:
        payload = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
    return opcode, payload


def encode_ws_frame(opcode, payload, mask=True):
    import os

    head = bytearray([0x80 | opcode])
    mask_bit = 0x80 if mask else 0
    if len(payload) < 126:
        head.append(mask_bit | len(payload))
    elif len(payload) < 65536:
        head.append(mask_bit | 126)
        head += len(payload).to_bytes(2, "big")
    else:
        head.append(mask_bit | 127)
        head += len(payload).to_bytes(8, "big")
    if mask:
        key = os.urandom(4)
        head += key
        payload = bytes(b ^ key[i % 4] for i, b in enumerate(payload))
    return bytes(head) + payload


class Server(ThreadingHTTPServer):
    daemon_threads = True
    allow_reuse_address = True
    request_queue_size = 1024

    def __init__(self, address, state, tls_context=None):
        self.state = state
        self.tls_context = tls_context
        super().__init__(address, Handler)

    def get_request(self):
        sock, addr = self.socket.accept()
        if self.tls_context is not None:
            sock = self.tls_context.wrap_socket(
                sock, server_side=True, do_handshake_on_connect=False
            )
        return sock, addr

    def handle_error(self, request, client_address):  # broken clients are expected
        pass


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--name", required=True)
    parser.add_argument("--delay", type=int, default=0, help="default delay in ms")
    parser.add_argument("--tls-cert")
    parser.add_argument("--tls-key")
    args = parser.parse_args()

    tls_context = None
    if args.tls_cert:
        tls_context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        tls_context.load_cert_chain(args.tls_cert, args.tls_key)

    server = Server((args.host, args.port), State(args.name, args.delay), tls_context)
    server.socket.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    print("READY", flush=True)
    try:
        server.serve_forever(poll_interval=0.1)
    except KeyboardInterrupt:
        pass
    return 0


if __name__ == "__main__":
    sys.exit(main())
