"""Security regressions: hostile clients must never crash or stall the proxy."""

import os
import select
import socket
import threading
import time

import pytest

from conftest import make_module_stack
from harness import client
from harness import config as cfg

# Time the proxy may take to drop a client that never finishes its request head.
# v1.1 uses 30 s; override with E2E_HEADER_TIMEOUT for other builds.
HEADER_TIMEOUT = float(os.environ.get("E2E_HEADER_TIMEOUT", "30"))


@pytest.fixture(scope="module")
def secure_lb(request, binary, pki, tmp_path_factory):
    stack = make_module_stack(request, binary, pki, tmp_path_factory, "security")
    a, b = stack.backends_named("a", "b")
    pools = [
        cfg.pool("Host('sticky.test')", [a.address, b.address],
                 strategy=cfg.sticky_cookie("RSID")),
        cfg.pool("Host('body.test')", [a.address],
                 middlewares={"MaxBodySize": {"limit": 1024}}),
        cfg.pool("HostRegexp('.*')", [a.address, b.address]),
    ]
    return stack.lb(pools)


@pytest.fixture
def lb(secure_lb, request):
    request.node._e2e_stacks = [type("S", (), {"lbs": [secure_lb]})()]
    assert secure_lb.alive()
    yield secure_lb
    assert secure_lb.alive(), "rust-strom exited during the test"
    r = client.request(secure_lb.http_port)
    assert r.status == 200, "rust-strom no longer serves normal requests: %r" % r


MALFORMED_COOKIES = [
    "RSID",
    "RSID=",
    "RSID=;",
    "=RSID",
    ";;;",
    "a=b;;;RSID=;",
    "RSID=zzzzzzzzzzzzzzzz",
    "RSID=127.0.0.1:1",
    "RSID=\"\"",
    "RSID=\"unterminated",
    "RSID=" + "f" * 8000,
    "RSID=%00%ff",
    "RSID=a=b=c",
]


@pytest.mark.parametrize("cookie", MALFORMED_COOKIES, ids=range(len(MALFORMED_COOKIES)))
def test_malformed_sticky_cookie(lb, cookie):
    r = client.request(lb.http_port, host="sticky.test", headers={"Cookie": cookie})
    assert r.status == 200, r


def test_non_utf8_sticky_cookie(lb):
    raw = (b"GET / HTTP/1.1\r\nHost: sticky.test\r\nCookie: RSID=\xc3\xa4\xff\xfe\r\n"
           b"Connection: close\r\n\r\n")
    resp = client.raw(lb.http_port, raw)
    assert client.status_of(resp) in (200, 400), resp[:200]


def test_options_asterisk(lb):
    resp = client.raw(lb.http_port,
                      b"OPTIONS * HTTP/1.1\r\nHost: plain.test\r\nConnection: close\r\n\r\n")
    status = client.status_of(resp)
    assert status is not None, "no HTTP response to OPTIONS *: %r" % resp[:200]
    assert status < 500, resp[:300]


@pytest.mark.parametrize("scheme", ["http", "https"])
def test_forwarding_headers_cannot_be_spoofed(lb, pki, scheme):
    headers = [
        ("X-Real-IP", "6.6.6.6"),
        ("X-Forwarded-For", "1.2.3.4"),
        ("X-Forwarded-Proto", "gopher"),
        ("X-Forwarded-Port", "1"),
        ("Connection", "keep-alive, X-Secret"),
        ("X-Secret", "must-not-reach-backend"),
        ("Keep-Alive", "timeout=5"),
        ("Proxy-Authorization", "Basic c2VjcmV0OnNlY3JldA=="),
        ("Proxy-Connection", "keep-alive"),
        ("TE", "trailers"),
        ("X-Kept", "yes"),
    ]
    if scheme == "http":
        r = client.request(lb.http_port, host="plain.test", headers=headers)
    else:
        r = client.request(lb.https_port, host="a.test", headers=headers, tls=pki.ca_cert)
    assert r.status == 200, r
    got = r.backend_headers()
    assert got.get("x-real-ip") == ["127.0.0.1"], got.get("x-real-ip")
    assert got.get("x-forwarded-proto") == [scheme], got.get("x-forwarded-proto")
    assert got.get("x-forwarded-port", ["1"]) != ["1"], "client X-Forwarded-Port was kept"
    assert len(got.get("x-forwarded-port", [])) <= 1
    xff = got.get("x-forwarded-for", [])
    assert len(xff) == 1 and xff[0].split(",")[-1].strip() == "127.0.0.1", xff
    for name in ("x-secret", "keep-alive", "proxy-authorization", "proxy-connection", "te"):
        assert name not in got, "hop-by-hop header %s was forwarded: %s" % (name, got[name])
    for value in got.get("connection", []):
        assert "x-secret" not in value.lower()
    assert got.get("x-kept") == ["yes"]


@pytest.mark.parametrize("scheme", ["http", "https"])
def test_x_forwarded_port_is_the_listener_port(lb, pki, scheme):
    """X-Forwarded-Port must be the port the client connected to (Traefik, ELB, nginx
    $server_port semantics), not a constant 80/443."""
    if scheme == "http":
        port = lb.http_port
        r = client.request(port, host="plain.test")
    else:
        port = lb.https_port
        r = client.request(port, host="a.test", tls=pki.ca_cert)
    assert r.backend_headers().get("x-forwarded-port") == [str(port)]


def test_rate_limit_is_per_ip_across_new_connections(stack):
    backend = stack.backend("a")
    lb = stack.lb([cfg.pool("Host('host.test')", [backend.address],
                            middlewares={"RateLimiter": {"limit": 5, "window_sec": 60}})])
    statuses = [client.request(lb.http_port, source_ip="127.0.0.1").status for _ in range(5)]
    assert statuses == [200] * 5
    # every request on a fresh TCP connection: the limit must still apply
    for _ in range(3):
        assert client.request(lb.http_port, source_ip="127.0.0.1").status == 429
    # another client IP has its own budget
    assert client.request(lb.http_port, source_ip="127.0.0.2").status == 200


def test_max_body_size_with_content_length(lb):
    r = client.request(lb.http_port, "/upload", method="POST", host="body.test",
                       body=b"x" * 2048)
    assert r.status == 413, r
    r = client.request(lb.http_port, "/upload", method="POST", host="body.test",
                       body=b"x" * 512)
    assert r.status == 200 and r.json()["body_len"] == 512, r


def test_max_body_size_invalid_content_length(lb):
    resp = client.raw(lb.http_port, b"POST / HTTP/1.1\r\nHost: body.test\r\n"
                                    b"Content-Length: abc\r\nConnection: close\r\n\r\n")
    status = client.status_of(resp)
    assert status is None or 400 <= status < 500, resp[:200]


def test_max_body_size_chunked_within_limit(lb):
    r = client.request(lb.http_port, "/upload", method="POST", host="body.test",
                       chunks=[b"a" * 300, b"b" * 300])
    assert r.status == 200 and r.json()["body_len"] == 600, r


def test_max_body_size_chunked_over_limit_is_not_forwarded(lb):
    """A chunked body over the limit must never reach the backend completely."""
    r = client.try_request(lb.http_port, "/upload", method="POST", host="body.test",
                           chunks=[b"c" * 1000] * 4)
    if r is not None and r.status == 200:
        assert r.json()["body_len"] <= 1024, "backend received %d bytes" % r.json()["body_len"]


@pytest.mark.v2
def test_max_body_size_chunked_over_limit_is_413(lb):
    r = client.request(lb.http_port, "/upload", method="POST", host="body.test",
                       chunks=[b"c" * 1000] * 4)
    assert r.status == 413, r


def test_idle_tls_sockets_do_not_block_https(lb, pki):
    """200 clients that connect to the HTTPS port and never send a ClientHello."""
    idle = []
    try:
        for _ in range(200):
            idle.append(socket.create_connection(("127.0.0.1", lb.https_port), timeout=5))
        start = time.monotonic()
        r = client.request(lb.https_port, host="a.test", tls=pki.ca_cert, timeout=5)
        elapsed = time.monotonic() - start
        assert r.status == 200, r
        assert elapsed < 2.0, "HTTPS request took %.2fs behind idle sockets" % elapsed
    finally:
        for s in idle:
            s.close()


def test_idle_plain_sockets_do_not_block_http(lb):
    idle = []
    try:
        for _ in range(200):
            idle.append(socket.create_connection(("127.0.0.1", lb.http_port), timeout=5))
        start = time.monotonic()
        r = client.request(lb.http_port, timeout=5)
        assert r.status == 200 and time.monotonic() - start < 2.0, r
    finally:
        for s in idle:
            s.close()


@pytest.mark.slow
@pytest.mark.timeout(HEADER_TIMEOUT + 90)
def test_slowloris_is_dropped_by_header_timeout(lb):
    """Clients that trickle header bytes forever are disconnected, and the proxy keeps
    serving everyone else meanwhile."""
    conns = []
    for _ in range(50):
        s = socket.create_connection(("127.0.0.1", lb.http_port), timeout=5)
        s.sendall(b"GET / HTTP/1.1\r\nHost: plain.test\r\n")
        conns.append(s)
    open_conns = set(conns)
    start = time.monotonic()
    deadline = start + HEADER_TIMEOUT + 20
    closed_at = []
    served_during_attack = 0
    i = 0
    while open_conns and time.monotonic() < deadline:
        for s in list(open_conns):
            try:
                s.sendall(b"X-Slow-%d: a\r\n" % i)
            except OSError:
                open_conns.discard(s)
                closed_at.append(time.monotonic() - start)
        readable, _, _ = select.select(list(open_conns), [], [], 1.0)
        for s in readable:
            try:
                data = s.recv(4096)
            except OSError:
                data = b""
            # a 408 or a plain close both count as dropped
            open_conns.discard(s)
            closed_at.append(time.monotonic() - start)
        if i % 5 == 0:
            r = client.try_request(lb.http_port, timeout=5)
            assert r is not None and r.status == 200, "proxy stalled under slowloris"
            served_during_attack += 1
        i += 1
    for s in conns:
        s.close()
    assert not open_conns, "%d/50 slow connections still open after %.0fs" % (
        len(open_conns), time.monotonic() - start)
    assert served_during_attack >= 3


@pytest.mark.slow
def test_slow_tls_handshake_is_dropped(lb):
    """A client that sends half a ClientHello is disconnected (v1.1: 10 s)."""
    s = socket.create_connection(("127.0.0.1", lb.https_port), timeout=5)
    s.sendall(b"\x16\x03\x01\x02\x00\x01\x00\x01\xfc\x03\x03")
    s.settimeout(HEADER_TIMEOUT + 15)
    start = time.monotonic()
    try:
        data = s.recv(1024)
    except (ConnectionResetError, socket.timeout):
        data = b""
    elapsed = time.monotonic() - start
    s.close()
    assert data == b"" and elapsed < HEADER_TIMEOUT + 15


MALFORMED = {
    "garbage": b"\x00\x01\x02\xffgarbage\r\n\r\n",
    "binary-noise": bytes(range(256)) * 8,
    "no-version": b"GET /\r\n\r\n",
    "bad-version": b"GET / HTTP/9.9\r\nHost: plain.test\r\n\r\n",
    "space-in-method": b"G ET / HTTP/1.1\r\nHost: plain.test\r\n\r\n",
    "header-without-colon": b"GET / HTTP/1.1\r\nHost: plain.test\r\nNoColonHere\r\n\r\n",
    "nul-in-header": b"GET / HTTP/1.1\r\nHost: plain.test\r\nX-A: a\x00b\r\n\r\n",
    "oversized-header": b"GET / HTTP/1.1\r\nHost: plain.test\r\nX-Big: " + b"a" * 600_000
                        + b"\r\n\r\n",
    "many-headers": b"GET / HTTP/1.1\r\nHost: plain.test\r\n"
                    + b"".join(b"X-H%d: v\r\n" % i for i in range(5000)) + b"\r\n",
    "oversized-uri": b"GET /" + b"a" * 200_000 + b" HTTP/1.1\r\nHost: plain.test\r\n\r\n",
    "invalid-chunk-size": b"POST / HTTP/1.1\r\nHost: plain.test\r\nTransfer-Encoding: chunked"
                          b"\r\n\r\nzz\r\nhello\r\n0\r\n\r\n",
    "chunk-size-overflow": b"POST / HTTP/1.1\r\nHost: plain.test\r\nTransfer-Encoding: chunked"
                           b"\r\n\r\nffffffffffffffffffff\r\nhello\r\n0\r\n\r\n",
    "chunk-missing-crlf": b"POST / HTTP/1.1\r\nHost: plain.test\r\nTransfer-Encoding: chunked"
                          b"\r\n\r\n5\r\nhelloXX0\r\n\r\n",
    "cl-and-te": b"POST / HTTP/1.1\r\nHost: plain.test\r\nContent-Length: 4\r\n"
                 b"Transfer-Encoding: chunked\r\n\r\n0\r\n\r\n",
    "conflicting-cl": b"POST / HTTP/1.1\r\nHost: plain.test\r\nContent-Length: 5\r\n"
                      b"Content-Length: 6\r\n\r\nhello!",
    "negative-cl": b"POST / HTTP/1.1\r\nHost: plain.test\r\nContent-Length: -1\r\n\r\n",
    "huge-cl-short-body": b"POST / HTTP/1.1\r\nHost: plain.test\r\n"
                          b"Content-Length: 99999999999999999999\r\n\r\nabc",
    "h2-preface-then-garbage": b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n" + b"\xff" * 64,
    "lf-only": b"GET / HTTP/1.1\nHost: plain.test\n\n",
    "obs-fold": b"GET / HTTP/1.1\r\nHost: plain.test\r\nX-A: a\r\n  folded\r\n\r\n",
    "absolute-uri-other-host": b"GET http://evil.test/ HTTP/1.1\r\nHost: plain.test\r\n"
                               b"Connection: close\r\n\r\n",
    "connect-method": b"CONNECT 127.0.0.1:22 HTTP/1.1\r\nHost: 127.0.0.1:22\r\n\r\n",
}

# For these the request must be rejected (no 2xx); others only must not crash anything.
MUST_REJECT = {"garbage", "binary-noise", "space-in-method", "header-without-colon",
               "oversized-header", "many-headers", "invalid-chunk-size", "chunk-size-overflow",
               "conflicting-cl", "negative-cl", "connect-method"}


@pytest.mark.parametrize("name", list(MALFORMED))
def test_malformed_http_never_kills_the_process(lb, name):
    resp = client.raw(lb.http_port, MALFORMED[name], read_timeout=3)
    status = client.status_of(resp)
    if name in MUST_REJECT:
        assert status is None or status >= 400, "%s answered with %r" % (name, resp[:200])
    if status is not None:
        assert status not in (500,), "%s caused an internal error: %r" % (name, resp[:200])


@pytest.mark.parametrize("payload", [
    b"\x16\x03\x01\x00\x05garbage",
    b"GET / HTTP/1.1\r\nHost: a.test\r\n\r\n",
    os.urandom(4096),
], ids=["truncated-hello", "plain-http-on-tls", "random"])
def test_malformed_tls_never_kills_the_process(lb, pki, payload):
    s = socket.create_connection(("127.0.0.1", lb.https_port), timeout=3)
    try:
        s.sendall(payload)
        try:
            s.recv(1024)
        except OSError:
            pass
    finally:
        s.close()
    r = client.request(lb.https_port, host="a.test", tls=pki.ca_cert)
    assert r.status == 200


def test_parallel_malformed_flood(lb):
    """Many concurrent malformed connections; the proxy stays up and serving."""
    payloads = list(MALFORMED.values())
    errors = []

    def worker(offset):
        for i in range(20):
            try:
                client.raw(lb.http_port, payloads[(offset + i) % len(payloads)], read_timeout=1)
            except OSError as e:
                errors.append(e)

    threads = [threading.Thread(target=worker, args=(n,)) for n in range(8)]
    for t in threads:
        t.start()
    for t in threads:
        t.join(120)
    assert client.request(lb.http_port).status == 200
