"""Load balancing strategies."""

import re
import time
from collections import Counter

import pytest

from harness import client
from harness import config as cfg


def _wait_health_round(seconds=1.5):
    time.sleep(seconds)


def test_round_robin_distributes_evenly(stack):
    backends = stack.backends_named("a", "b", "c")
    lb = stack.lb([cfg.pool("Host('host.test')", [b.address for b in backends],
                            strategy=cfg.ROUND_ROBIN)])
    counts = Counter(client.request(lb.http_port).backend for _ in range(60))
    assert set(counts) == {"a", "b", "c"}, counts
    for name, count in counts.items():
        assert abs(count - 20) <= 2, counts


def test_random_uses_every_backend(stack):
    backends = stack.backends_named("a", "b", "c")
    lb = stack.lb([cfg.pool("Host('host.test')", [b.address for b in backends],
                            strategy=cfg.RANDOM)])
    counts = Counter(client.request(lb.http_port).backend for _ in range(60))
    assert set(counts) == {"a", "b", "c"}, counts


def test_ip_hash_is_sticky_per_client_ip(stack):
    backends = stack.backends_named("a", "b", "c")
    lb = stack.lb([cfg.pool("Host('host.test')", [b.address for b in backends],
                            strategy=cfg.IP_HASH)])
    chosen = {}
    for i in range(1, 31):
        ip = "127.0.0.%d" % i
        seen = {client.request(lb.http_port, source_ip=ip).backend for _ in range(4)}
        assert len(seen) == 1, "client %s was spread over %s" % (ip, seen)
        chosen[ip] = seen.pop()
    assert len(set(chosen.values())) >= 2, "30 client IPs all hashed to one backend: %s" % chosen


@pytest.mark.v2
def test_ip_hash_is_consistent_when_a_backend_is_removed(stack):
    """v2 uses consistent hashing: removing one of three backends moves only the
    clients of that backend. Modulo hashing (v1.1) moves about two thirds."""
    backends = stack.backends_named("a", "b", "c")
    lb = stack.lb([cfg.pool("Host('host.test')", [b.address for b in backends],
                            strategy=cfg.IP_HASH)])
    ips = ["127.0.0.%d" % i for i in range(1, 61)]
    before = {ip: client.request(lb.http_port, source_ip=ip).backend for ip in ips}
    lb.write_config_atomic(stack.render(lb, [cfg.pool(
        "Host('host.test')", [backends[0].address, backends[1].address],
        strategy=cfg.IP_HASH)]))
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if all(client.request(lb.http_port, source_ip=ip).backend != "c"
               for ip in ips if before[ip] == "c"):
            break
        time.sleep(0.2)
    after = {ip: client.request(lb.http_port, source_ip=ip).backend for ip in ips}
    kept = [ip for ip in ips if before[ip] != "c"]
    moved = [ip for ip in kept if after[ip] != before[ip]]
    assert kept, before
    assert len(moved) <= len(kept) // 10, "%d of %d clients of the remaining backends moved" % (
        len(moved), len(kept))


def _parse_set_cookie(value):
    parts = [p.strip() for p in value.split(";")]
    name, _, cookie_value = parts[0].partition("=")
    attrs = {}
    for p in parts[1:]:
        k, _, v = p.partition("=")
        attrs[k.lower()] = v
    return name, cookie_value, attrs


def test_sticky_cookie_is_opaque_and_sticky(stack):
    backends = stack.backends_named("a", "b", "c")
    lb = stack.lb([cfg.pool("Host('host.test')", [b.address for b in backends],
                            strategy=cfg.sticky_cookie("RSID", http_only=True, secure=False))])
    first = client.request(lb.http_port)
    assert first.status == 200
    set_cookies = [v for v in first.header_all("set-cookie") if v.startswith("RSID=")]
    assert len(set_cookies) == 1, first.headers
    name, value, attrs = _parse_set_cookie(set_cookies[0])
    assert value, "empty sticky cookie"
    # Opaque: must not leak backend addresses or ports
    for b in backends:
        assert str(b.port) not in value, "cookie leaks backend port: %s" % value
    assert "127.0.0.1" not in value and ":" not in value, value
    assert re.fullmatch(r"[A-Za-z0-9_\-.~+/=]+", value), "unexpected cookie format %r" % value
    assert attrs.get("path") == "/", "sticky cookie must have Path=/: %s" % set_cookies[0]
    assert "httponly" in attrs

    # Requests carrying the cookie stick to the first backend
    for _ in range(12):
        r = client.request(lb.http_port, headers={"Cookie": "other=1; RSID=%s" % value})
        assert r.status == 200
        assert r.backend == first.backend
        assert not [v for v in r.header_all("set-cookie") if v.startswith("RSID=")], \
            "a valid sticky cookie should not be re-issued"

    # Without the cookie the inner RoundRobin spreads new sessions
    fresh = Counter(client.request(lb.http_port).backend for _ in range(6))
    assert len(fresh) >= 2, fresh


def test_sticky_cookie_for_unhealthy_backend_falls_back(stack):
    backends = stack.backends_named("a", "b")
    lb = stack.lb([cfg.pool("Host('host.test')", [b.address for b in backends],
                            strategy=cfg.sticky_cookie("RSID"))])
    first = client.request(lb.http_port)
    _, value, _ = _parse_set_cookie([v for v in first.header_all("set-cookie")
                                     if v.startswith("RSID=")][0])
    owner = next(b for b in backends if b.name == first.backend)
    owner.kill()
    deadline = time.monotonic() + 6
    r = None
    while time.monotonic() < deadline:
        r = client.request(lb.http_port, headers={"Cookie": "RSID=%s" % value})
        if r.status == 200:
            break
        time.sleep(0.2)
    assert r.status == 200 and r.backend != owner.name, r
    assert any(v.startswith("RSID=") for v in r.header_all("set-cookie")), \
        "a new sticky cookie must be issued after failover"


def test_least_connection_prefers_idle_backend(stack):
    """While one backend serves a long request, new requests go to the idle one.
    Round robin or random would send ~50% of them to the busy backend."""
    backends = stack.backends_named("a", "b")
    lb = stack.lb([cfg.pool("Host('host.test')", [b.address for b in backends],
                            strategy=cfg.LEAST_CONNECTION)])
    import threading
    holder = {}
    slow = threading.Thread(target=lambda: holder.setdefault(
        "r", client.request(lb.http_port, "/?sleep=4000", timeout=15)))
    slow.start()
    deadline = time.monotonic() + 3
    busy = None
    while busy is None and time.monotonic() < deadline:
        for b in backends:
            if b.stats()["inflight"] > 0:
                busy = b.name
        time.sleep(0.05)
    assert busy, "the long request did not reach a backend"
    counts = Counter(client.request(lb.http_port).backend for _ in range(12))
    slow.join()
    assert holder["r"].status == 200
    # v1.1 counts TCP connections (a short race may open one extra), v2 in-flight requests
    assert counts[busy] <= 2, "busy backend %s got %s" % (busy, counts)


@pytest.mark.v2
def test_least_connection_under_concurrent_load(stack):
    """A slow backend and a fast one under 4 concurrent clients: the fast one must get
    most of the traffic. v1.1 counts pooled TCP connections, so a burst of idle
    keep-alive connections to the fast backend makes it look busy."""
    slow = stack.backend("slow", delay_ms=1000)
    fast = stack.backend("fast")
    lb = stack.lb([cfg.pool("Host('host.test')", [slow.address, fast.address],
                            strategy=cfg.LEAST_CONNECTION)])
    with client.LoadLoop(lb.http_port, threads=4, pause=0.01) as loop:
        time.sleep(4)
    assert loop.non_2xx() == 0, loop.outcomes
    total = sum(loop.backends.values())
    assert total > 20, loop.backends
    assert loop.backends["fast"] / total >= 0.8, loop.backends


@pytest.mark.v2
def test_least_connection_counts_in_flight_requests_not_idle_connections(stack):
    """v2: LeastConnection uses in-flight requests. Idle keep-alive connections
    left over from earlier traffic must not make a backend look busy."""
    a = stack.backend("a")
    b = stack.backend("b")
    lb = stack.lb([cfg.pool("Host('host.test')", [a.address, b.address],
                            strategy=cfg.LEAST_CONNECTION)])
    # Phase 1: only `a` is healthy; open several concurrent connections to it
    b.ctl(healthy=0)
    _wait_health_round(2.5)
    with client.LoadLoop(lb.http_port, threads=6, path="/?sleep=300", pause=0) as loop:
        time.sleep(1.5)
    assert loop.backends.get("b", 0) == 0 and loop.non_2xx() == 0, loop.outcomes
    # Phase 2: both healthy and idle; `a` still has idle pooled connections
    b.ctl(healthy=1)
    _wait_health_round(2.5)
    counts = Counter(client.request(lb.http_port).backend for _ in range(40))
    assert counts["a"] >= 8, "idle connections were counted as load: %s" % counts
