"""Prometheus metrics endpoint (names must stay compatible with v1.1)."""

import re
import time
import urllib.error
import urllib.request

from harness import client
from harness import config as cfg

# Metric families v1.1 exposes once traffic (incl. one 502) has been served.
EXPECTED = [
    "http_requests_total",
    "active_http_connections",
    "http_request_duration_seconds",
    "backend_response_time_seconds",
    "http_status_codes_total",
    "http_errors_total",
    "backend_failures_total",
]


def _scrape(port, path="/metrics"):
    with urllib.request.urlopen("http://127.0.0.1:%d%s" % (port, path), timeout=5) as r:
        return r.status, r.headers.get("Content-Type", ""), r.read().decode()


def _sample(text, name, labels=None):
    total = 0.0
    found = False
    for line in text.splitlines():
        if line.startswith("#"):
            continue
        m = re.match(r"([a-zA-Z_:][a-zA-Z0-9_:]*)(\{[^}]*\})?\s+(\S+)", line)
        if not m or m.group(1) != name:
            continue
        if labels and not all('%s="%s"' % kv in (m.group(2) or "") for kv in labels.items()):
            continue
        total += float(m.group(3))
        found = True
    return total if found else None


def test_metrics_endpoint(stack):
    backend = stack.backend("a")
    doomed = stack.backend("doomed")
    lb = stack.lb([cfg.pool("Host('host.test')", [backend.address]),
                   cfg.pool("Host('dead.test')", [doomed.address])], check_every=60)
    # Kill it after the first health round: the proxied request then fails with 502.
    # (v1.1 does not count backend_failures_total when the pool has no healthy backend.)
    time.sleep(1)
    doomed.kill()
    for _ in range(5):
        assert client.request(lb.http_port).status == 200
    assert client.request(lb.http_port, host="dead.test").status == 502
    assert client.request(lb.http_port, host="nomatch.test").status == 404

    status, content_type, text = _scrape(lb.metrics_port)
    assert status == 200
    assert content_type.startswith("text/plain")
    missing = [m for m in EXPECTED if not re.search(r"^%s(_bucket|_sum|_count)?[{ ]" % m, text,
                                                     re.M)]
    assert not missing, "missing metrics: %s\n%s" % (missing, text[:2000])

    assert _sample(text, "http_requests_total") >= 6
    assert _sample(text, "http_status_codes_total", {"code": "200"}) >= 5
    assert _sample(text, "http_status_codes_total", {"code": "502"}) >= 1
    assert _sample(text, "http_errors_total") >= 1
    assert _sample(text, "backend_response_time_seconds_count",
                   {"backend": backend.address}) >= 5
    assert _sample(text, "backend_failures_total", {"backend": doomed.address}) >= 1


def test_metrics_only_on_metrics_path(stack):
    backend = stack.backend("a")
    lb = stack.lb([cfg.pool("Host('host.test')", [backend.address])])
    try:
        status, _, _ = _scrape(lb.metrics_port, "/not-metrics")
    except urllib.error.HTTPError as e:
        status = e.code
    assert status == 404
    # the proxy port does not expose metrics
    r = client.request(lb.http_port, "/metrics", host="nomatch.test")
    assert r.status == 404
