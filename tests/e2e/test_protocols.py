"""Protocols: TLS/ALPN, HTTP/2, SNI, WebSocket, TLS to upstreams."""

import socket
import ssl

import pytest

from conftest import make_module_stack
from harness import client
from harness import config as cfg


@pytest.fixture(scope="module")
def tls_lb(request, binary, pki, tmp_path_factory):
    stack = make_module_stack(request, binary, pki, tmp_path_factory, "protocols")
    backend = stack.backend("a")
    return stack.lb([cfg.pool("HostRegexp('.*')", [backend.address])])


def _alpn(port, ca, offer, sni="a.test"):
    sock = client.connect(port, tls=ca, sni=sni, alpn=offer)
    try:
        return sock.selected_alpn_protocol(), sock.version()
    finally:
        sock.close()


def test_alpn_negotiates_h2(tls_lb, pki):
    proto, _ = _alpn(tls_lb.https_port, pki.ca_cert, ["h2", "http/1.1"])
    assert proto == "h2"


def test_alpn_http11_only_client(tls_lb, pki):
    proto, _ = _alpn(tls_lb.https_port, pki.ca_cert, ["http/1.1"])
    assert proto == "http/1.1"
    r = client.request(tls_lb.https_port, host="a.test", tls=pki.ca_cert)
    assert r.status == 200 and r.json()["version"] == "HTTP/1.1"


def test_tls_version_is_modern(tls_lb, pki):
    _, version = _alpn(tls_lb.https_port, pki.ca_cert, ["http/1.1"])
    assert version in ("TLSv1.2", "TLSv1.3")
    ctx = client.tls_context(pki.ca_cert)
    ctx.maximum_version = ssl.TLSVersion.TLSv1_1
    ctx.minimum_version = ssl.TLSVersion.TLSv1
    with socket.create_connection(("127.0.0.1", tls_lb.https_port), timeout=5) as raw:
        with pytest.raises((ssl.SSLError, OSError)):
            ctx.wrap_socket(raw, server_hostname="a.test").close()


def test_sni_selects_matching_certificate(tls_lb, pki):
    for name in ("a.test", "h2.test", "localhost"):
        sock = client.connect(tls_lb.https_port, tls=pki.ca_cert, sni=name)
        sock.close()  # verification against SAN + CA succeeded


def test_h2_multiple_requests(tls_lb, pki):
    for path in ("/one", "/two?x=1"):
        r = client.h2_request(tls_lb.https_port, "a.test", path, ca=pki.ca_cert)
        assert r.status == 200 and r.json()["path"] == path, r


@pytest.mark.parametrize("size", [1024 * 1024, 8 * 1024 * 1024], ids=["1MB", "8MB"])
def test_large_request_body_is_forwarded_intact(tls_lb, pki, size):
    body = b"z" * size
    r = client.request(tls_lb.http_port, "/upload", method="POST", host="a.test", body=body,
                       timeout=30)
    assert r.status == 200 and r.json()["body_len"] == size, r
    r = client.request(tls_lb.https_port, "/upload", method="POST", host="a.test",
                       chunks=[body[i:i + 65536] for i in range(0, size, 65536)],
                       tls=pki.ca_cert, timeout=30)
    assert r.status == 200 and r.json()["body_len"] == size, r


def test_h2c_prior_knowledge_on_http_port_does_not_crash(tls_lb):
    raw = client.raw(tls_lb.http_port, b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n"
                                       b"\x00\x00\x00\x04\x00\x00\x00\x00\x00", read_timeout=2)
    assert tls_lb.alive()
    assert client.request(tls_lb.http_port, host="a.test").status == 200


@pytest.mark.v2
def test_websocket_echo_through_proxy(stack):
    backend = stack.backend("ws")
    lb = stack.lb([cfg.pool("Host('ws.test')", [backend.address])])
    status, echoed = client.websocket_echo(lb.http_port, "ws.test", ["hello", "world" * 100])
    assert status == 101
    assert echoed == ["hello", "world" * 100]


@pytest.mark.v2
def test_websocket_echo_over_tls(stack, pki):
    backend = stack.backend("ws")
    lb = stack.lb([cfg.pool("Host('ws.test')", [backend.address])])
    status, echoed = client.websocket_echo(lb.https_port, "ws.test", ["over tls"],
                                           tls=pki.ca_cert)
    assert status == 101 and echoed == ["over tls"]


@pytest.mark.v2
def test_upstream_tls_backend(stack):
    backend = stack.backend("tls", tls=True)
    lb = stack.lb([cfg.pool("Host('host.test')", [backend.address],
                            upstream={"tls": True, "sni": "backend.test",
                                      "verify_cert": False})])
    r = client.request(lb.http_port)
    assert r.status == 200 and r.backend == "tls", r
