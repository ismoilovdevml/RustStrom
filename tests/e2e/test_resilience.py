"""Backend failures: failover, retries, timeouts, total outage."""

import time
from collections import Counter

import pytest

from harness import client
from harness import config as cfg
from harness.ports import free_port
from harness.procs import Backend

# v1.1 has a fixed 30 s backend timeout; v2 reads [backend_pools.upstream].
V1_BACKEND_TIMEOUT = 30.0
V2_READ_TIMEOUT_MS = 3000


def test_killed_backend_is_removed_within_health_interval(stack):
    check_every = 1
    backends = stack.backends_named("a", "b", "c")
    lb = stack.lb([cfg.pool("Host('host.test')", [b.address for b in backends])],
                  check_every=check_every)
    assert {client.request(lb.http_port).backend for _ in range(9)} == {"a", "b", "c"}

    victim = backends[2]
    victim.kill()
    killed_at = time.monotonic()
    # interval + health timeout (1 s) + one more round of slack
    budget = check_every + 1.0 + 2.0
    last_failure = None
    while time.monotonic() - killed_at < budget:
        r = client.try_request(lb.http_port, timeout=5)
        if r is None or r.status != 200:
            last_failure = time.monotonic() - killed_at
        time.sleep(0.05)
    counts = Counter()
    for _ in range(30):
        r = client.request(lb.http_port)
        assert r.status == 200, "request failed %.1fs after the kill: %r" % (
            time.monotonic() - killed_at, r)
        counts[r.backend] += 1
    assert set(counts) == {"a", "b"}, counts
    if last_failure is not None:
        assert last_failure < budget


def test_backend_comes_back_after_restart(stack):
    a = stack.backend("a")
    b = stack.backend("b")
    lb = stack.lb([cfg.pool("Host('host.test')", [a.address, b.address])])
    port = b.port
    b.kill()
    time.sleep(2.5)
    assert {client.request(lb.http_port).backend for _ in range(6)} == {"a"}
    stack.backends.append(Backend("b", stack.workdir, port=port))
    deadline = time.monotonic() + 5
    seen = set()
    while time.monotonic() < deadline and "b" not in seen:
        seen.add(client.request(lb.http_port).backend)
    assert "b" in seen, "revived backend never received traffic again"


@pytest.mark.v2
def test_retry_on_connect_failure_gives_zero_client_errors(stack):
    """A backend dies between health checks: idempotent requests are retried on
    another backend instead of failing."""
    a = stack.backend("a")
    b = stack.backend("b")
    lb = stack.lb([cfg.pool("Host('host.test')", [a.address, b.address],
                            upstream={"retries": 1})], check_every=60)
    time.sleep(1)  # first health round sees both healthy
    b.kill()
    results = Counter()
    for _ in range(40):
        r = client.try_request(lb.http_port, timeout=10)
        results[r.status if r else "error"] += 1
    assert results == Counter({200: 40}), results


@pytest.mark.slow
@pytest.mark.timeout(120)
def test_slow_backend_gets_504_within_timeout(stack):
    backend = stack.backend("slow")
    lb = stack.lb([cfg.pool("Host('host.test')", [backend.address],
                            upstream={"read_timeout_ms": V2_READ_TIMEOUT_MS})])
    start = time.monotonic()
    r = client.request(lb.http_port, "/?sleep=60000", timeout=V1_BACKEND_TIMEOUT + 20)
    elapsed = time.monotonic() - start
    assert r.status == 504, r
    assert elapsed < V1_BACKEND_TIMEOUT + 5, "504 took %.1fs" % elapsed
    assert lb.alive()
    assert client.request(lb.http_port).status == 200


@pytest.mark.v2
def test_slow_backend_honours_configured_read_timeout(stack):
    backend = stack.backend("slow")
    lb = stack.lb([cfg.pool("Host('host.test')", [backend.address],
                            upstream={"read_timeout_ms": V2_READ_TIMEOUT_MS})])
    start = time.monotonic()
    r = client.request(lb.http_port, "/?sleep=60000", timeout=10)
    elapsed = time.monotonic() - start
    assert r.status == 504, r
    assert elapsed < V2_READ_TIMEOUT_MS / 1000.0 + 2, "504 took %.1fs" % elapsed


def test_all_backends_down_is_502(stack):
    dead = ["127.0.0.1:%d" % free_port() for _ in range(2)]
    lb = stack.lb([cfg.pool("Host('host.test')", dead)])
    r = client.request(lb.http_port)  # possibly before the first health round
    assert r.status == 502, r
    time.sleep(2.5)  # after health checks marked everything down
    for _ in range(3):
        r = client.request(lb.http_port)
        assert r.status == 502, r
    assert lb.alive()


def test_backend_closing_connection_mid_response_is_5xx(stack):
    """A backend that dies while a request is in flight yields a 5xx, not a hang."""
    backend = stack.backend("a")
    lb = stack.lb([cfg.pool("Host('host.test')", [backend.address])], check_every=60)
    import threading
    holder = {}
    t = threading.Thread(target=lambda: holder.setdefault(
        "r", client.try_request(lb.http_port, "/?sleep=3000", timeout=40)))
    t.start()
    time.sleep(0.5)
    backend.kill()
    t.join(40)
    r = holder.get("r")
    assert r is not None and r.status in (502, 503, 504), r
    assert lb.alive()
