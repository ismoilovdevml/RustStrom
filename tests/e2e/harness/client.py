"""Small HTTP/1.1, HTTP/2 and WebSocket clients with full control over the
connection (source IP, SNI, raw bytes)."""

import base64
import http.client
import json
import os
import socket
import ssl
import threading
import time
from collections import Counter

from .backend_server import encode_ws_frame, read_ws_frame


class Response:
    def __init__(self, status, headers, body):
        self.status = status
        self.headers = headers  # list of (name, value)
        self.body = body

    def header(self, name, default=None):
        name = name.lower()
        for k, v in self.headers:
            if k.lower() == name:
                return v
        return default

    def header_all(self, name):
        name = name.lower()
        return [v for k, v in self.headers if k.lower() == name]

    def json(self):
        return json.loads(self.body)

    @property
    def backend(self):
        return self.header("x-backend")

    def backend_headers(self):
        """Headers the backend received, as {name: [values]}."""
        out = {}
        for k, v in self.json()["headers"]:
            out.setdefault(k.lower(), []).append(v)
        return out

    def __repr__(self):
        return "<Response %s backend=%s %r>" % (self.status, self.backend, self.body[:200])


def tls_context(ca_file, alpn=None):
    ctx = ssl.create_default_context(cafile=ca_file)
    if alpn:
        ctx.set_alpn_protocols(alpn)
    return ctx


def connect(port, tls=None, sni=None, source_ip=None, timeout=10.0, alpn=None):
    """TCP (and optionally TLS) connection to 127.0.0.1:port.

    tls: path of the CA file, or None for plain TCP.
    """
    source = (source_ip, 0) if source_ip else None
    sock = socket.create_connection(("127.0.0.1", port), timeout=timeout, source_address=source)
    if tls:
        ctx = tls_context(tls, alpn)
        sock = ctx.wrap_socket(sock, server_hostname=sni or "localhost")
    return sock


def request(port, path="/", method="GET", host="host.test", headers=None, body=None,
            chunks=None, tls=None, sni=None, source_ip=None, timeout=10.0):
    """One HTTP/1.1 request on a fresh connection. `headers` may be a dict or a
    list of pairs (to send duplicates). Returns Response."""
    sock = connect(port, tls=tls, sni=sni or host, source_ip=source_ip, timeout=timeout)
    conn = http.client.HTTPConnection("127.0.0.1", port, timeout=timeout)
    conn.sock = sock
    try:
        conn.putrequest(method, path, skip_host=True, skip_accept_encoding=True)
        if host is not None:
            conn.putheader("Host", host)
        pairs = headers.items() if isinstance(headers, dict) else (headers or [])
        for name, value in pairs:
            conn.putheader(name, value)
        if chunks is not None:
            conn.putheader("Transfer-Encoding", "chunked")
            conn.endheaders()
            for chunk in chunks:
                conn.send(b"%x\r\n%s\r\n" % (len(chunk), chunk))
            conn.send(b"0\r\n\r\n")
        elif body is not None:
            if isinstance(body, str):
                body = body.encode()
            names = {n.lower() for n, _ in pairs}
            if "content-length" not in names:
                conn.putheader("Content-Length", str(len(body)))
            conn.endheaders(body)
        else:
            conn.endheaders()
        resp = conn.getresponse()
        data = resp.read()
        return Response(resp.status, resp.getheaders(), data)
    finally:
        conn.close()


def try_request(*args, **kwargs):
    """Like request() but returns None instead of raising on network errors."""
    try:
        return request(*args, **kwargs)
    except (OSError, http.client.HTTPException):
        return None


def raw(port, data, tls=None, sni="host.test", read_timeout=3.0, chunks_delay=None):
    """Sends raw bytes and returns everything received until the peer closes the
    connection or `read_timeout` passes without data."""
    sock = connect(port, tls=tls, sni=sni, timeout=read_timeout)
    received = b""
    try:
        if isinstance(data, (list, tuple)):
            for part in data:
                sock.sendall(part)
                if chunks_delay:
                    time.sleep(chunks_delay)
        else:
            sock.sendall(data)
        while True:
            try:
                chunk = sock.recv(65536)
            except socket.timeout:
                break
            except (ConnectionResetError, ssl.SSLError, OSError):
                break
            if not chunk:
                break
            received += chunk
    except (BrokenPipeError, ConnectionResetError, ssl.SSLError):
        pass
    finally:
        sock.close()
    return received


def status_of(raw_response):
    """Status code from a raw HTTP/1.x response, or None."""
    if not raw_response.startswith(b"HTTP/"):
        return None
    try:
        return int(raw_response.split(b" ", 2)[1])
    except (IndexError, ValueError):
        return None


def h2_request(port, authority, path="/", ca=None, sni=None, method="GET", timeout=10.0):
    """One HTTP/2 request over TLS (ALPN h2) with an explicit :authority and no
    Host header. Returns Response (status int, headers, body)."""
    import h2.connection
    import h2.config
    import h2.events

    sock = connect(port, tls=ca, sni=sni or authority, timeout=timeout, alpn=["h2"])
    try:
        if sock.selected_alpn_protocol() != "h2":
            raise AssertionError("server did not negotiate h2: %r" % sock.selected_alpn_protocol())
        conn = h2.connection.H2Connection(config=h2.config.H2Configuration(client_side=True))
        conn.initiate_connection()
        sock.sendall(conn.data_to_send())
        stream_id = conn.get_next_available_stream_id()
        conn.send_headers(stream_id, [
            (":method", method), (":scheme", "https"), (":authority", authority),
            (":path", path), ("user-agent", "e2e-h2"),
        ], end_stream=True)
        sock.sendall(conn.data_to_send())
        status, headers, body, done = None, [], b"", False
        while not done:
            data = sock.recv(65536)
            if not data:
                break
            for event in conn.receive_data(data):
                if isinstance(event, h2.events.ResponseReceived) and event.stream_id == stream_id:
                    for k, v in event.headers:
                        k = k.decode() if isinstance(k, bytes) else k
                        v = v.decode() if isinstance(v, bytes) else v
                        if k == ":status":
                            status = int(v)
                        else:
                            headers.append((k, v))
                elif isinstance(event, h2.events.DataReceived) and event.stream_id == stream_id:
                    body += event.data
                    conn.acknowledge_received_data(event.flow_controlled_length, stream_id)
                elif isinstance(event, h2.events.StreamEnded) and event.stream_id == stream_id:
                    done = True
                elif isinstance(event, (h2.events.StreamReset, h2.events.ConnectionTerminated)):
                    done = True
            out = conn.data_to_send()
            if out:
                sock.sendall(out)
        return Response(status, headers, body)
    finally:
        sock.close()


def websocket_echo(port, host, messages, path="/ws", tls=None, timeout=5.0):
    """Performs a WebSocket handshake through the LB and echoes `messages`.
    Returns (status, [echoed payloads])."""
    sock = connect(port, tls=tls, sni=host, timeout=timeout)
    try:
        key = base64.b64encode(os.urandom(16)).decode()
        sock.sendall((
            "GET %s HTTP/1.1\r\nHost: %s\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n"
            "Sec-WebSocket-Key: %s\r\nSec-WebSocket-Version: 13\r\n\r\n" % (path, host, key)
        ).encode())
        rfile = sock.makefile("rb")
        status_line = rfile.readline()
        status = int(status_line.split(b" ", 2)[1]) if status_line.startswith(b"HTTP/") else None
        while True:
            line = rfile.readline()
            if line in (b"\r\n", b"\n", b""):
                break
        echoed = []
        if status != 101:
            return status, echoed
        for message in messages:
            payload = message.encode() if isinstance(message, str) else message
            sock.sendall(encode_ws_frame(0x1, payload, mask=True))
            frame = read_ws_frame(rfile)
            if frame is None:
                break
            echoed.append(frame[1].decode())
        sock.sendall(encode_ws_frame(0x8, b"\x03\xe8", mask=True))
        return status, echoed
    finally:
        sock.close()


class LoadLoop:
    """Background client threads that hammer the LB and count outcomes.

    Each request uses a fresh connection unless keepalive=True.
    """

    def __init__(self, port, threads=4, host="host.test", path="/", pause=0.005, tls=None):
        self.port = port
        self.host = host
        self.path = path
        self.pause = pause
        self.tls = tls
        self.outcomes = Counter()
        self.backends = Counter()
        self.errors = []
        self._stop = threading.Event()
        self._lock = threading.Lock()
        self._threads = [threading.Thread(target=self._run, daemon=True) for _ in range(threads)]

    def _run(self):
        while not self._stop.is_set():
            t = time.monotonic()
            try:
                r = request(self.port, self.path, host=self.host, tls=self.tls, timeout=10)
                key = r.status
                backend = r.backend
            except Exception as e:  # noqa: BLE001 - every failure counts
                key = "exception"
                backend = None
                with self._lock:
                    self.errors.append((t, repr(e)))
            with self._lock:
                self.outcomes[key] += 1
                if backend:
                    self.backends[backend] += 1
                if key != 200 and key != "exception":
                    self.errors.append((t, key))
            if self.pause:
                time.sleep(self.pause)

    def __enter__(self):
        for t in self._threads:
            t.start()
        return self

    def __exit__(self, *exc):
        self.stop()

    def stop(self):
        self._stop.set()
        for t in self._threads:
            t.join(15)

    def non_2xx(self):
        with self._lock:
            return sum(v for k, v in self.outcomes.items() if not (isinstance(k, int) and 200 <= k < 300))

    def total(self):
        with self._lock:
            return sum(self.outcomes.values())
