"""Process lifecycle: graceful shutdown and `--check`."""

import os
import signal
import socket
import subprocess
import threading
import time

import pytest

from harness import client
from harness import config as cfg
from harness.ports import free_port

GRACE_SECS = 10  # v2: grace_period_secs; v1.1 drains for up to 30 s


def _lb_for(stack, backend, **kwargs):
    return stack.lb([cfg.pool("Host('host.test')", [backend.address])],
                    extra_global={"grace_period_secs": GRACE_SECS}, **kwargs)


def test_sigterm_drains_in_flight_request_and_exits_0(stack):
    sig = signal.SIGTERM
    backend = stack.backend("a")
    lb = _lb_for(stack, backend)
    holder = {}

    def slow_request():
        try:
            holder["r"] = client.request(lb.http_port, "/?sleep=3000", timeout=30)
        except Exception as e:  # noqa: BLE001
            holder["error"] = e

    t = threading.Thread(target=slow_request)
    t.start()
    deadline = time.monotonic() + 3
    while backend.stats()["inflight"] == 0 and time.monotonic() < deadline:
        time.sleep(0.05)
    assert backend.stats()["inflight"] == 1
    sent_at = time.monotonic()
    lb.signal(sig)
    t.join(30)
    assert "error" not in holder, "in-flight request failed: %r" % holder.get("error")
    assert holder["r"].status == 200, holder["r"]
    code = lb.wait(GRACE_SECS + 10)
    elapsed = time.monotonic() - sent_at
    assert code == 0, "exit code %s" % code
    assert elapsed < GRACE_SECS + 10, "exit took %.1fs" % elapsed


def test_sigint_exits_without_crashing(stack):
    backend = stack.backend("a")
    lb = _lb_for(stack, backend)
    assert client.request(lb.http_port).status == 200
    lb.signal(signal.SIGINT)
    code = lb.wait(GRACE_SECS + 10)
    assert code >= 0, "killed by signal %d" % -code


def test_sigterm_when_idle_exits_quickly(stack):
    backend = stack.backend("a")
    lb = _lb_for(stack, backend)
    assert client.request(lb.http_port).status == 200
    start = time.monotonic()
    lb.signal(signal.SIGTERM)
    assert lb.wait(GRACE_SECS + 10) == 0
    assert time.monotonic() - start < GRACE_SECS + 10


def test_no_new_connections_after_sigterm(stack):
    backend = stack.backend("a")
    lb = _lb_for(stack, backend)
    holder = {}
    t = threading.Thread(target=lambda: holder.setdefault(
        "r", client.request(lb.http_port, "/?sleep=3000", timeout=30)))
    t.start()
    time.sleep(0.5)
    lb.signal(signal.SIGTERM)
    time.sleep(0.5)
    r = client.try_request(lb.http_port, timeout=3)
    t.join(30)
    assert holder["r"].status == 200
    assert r is None or r.status in (200, 503), \
        "new request during drain got %r" % r
    lb.wait(GRACE_SECS + 10)


def _check(binary, path, cwd=None):
    return subprocess.run([binary, "--config", path, "--check"], capture_output=True,
                          text=True, timeout=30, cwd=cwd)


def _write(tmp_path, text, name="config.toml"):
    path = os.path.join(str(tmp_path), name)
    with open(path, "w") as f:
        f.write(text)
    return path


def _ports():
    return free_port(), free_port(), free_port()


def test_check_valid_config_exits_0(binary, pki, tmp_path):
    text = cfg.config(*_ports(), [cfg.pool("Host('host.test')", ["127.0.0.1:1"])],
                      certificates={"a.test": (pki.cert, pki.key)})
    result = _check(binary, _write(tmp_path, text))
    assert result.returncode == 0, result.stdout + result.stderr


def test_check_does_not_bind_ports(binary, tmp_path):
    http_port = free_port()
    holder = socket.socket()
    holder.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    holder.bind(("127.0.0.1", http_port))
    holder.listen(1)
    try:
        text = cfg.config(http_port, free_port(), free_port(),
                          [cfg.pool("Host('host.test')", ["127.0.0.1:1"])])
        result = _check(binary, _write(tmp_path, text))
        assert result.returncode == 0, result.stdout + result.stderr
    finally:
        holder.close()


def test_check_shipped_example_configs(binary, tmp_path):
    repo = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
    for rel in ("configs/config.toml", "docker-config.toml"):
        path = os.path.join(repo, rel)
        if not os.path.exists(path):
            pytest.skip("%s not available next to the suite" % rel)
        result = _check(binary, path)
        assert result.returncode == 0, "%s: %s" % (rel, result.stdout + result.stderr)


INVALID_CONFIGS = {
    "toml-syntax": "http_address = [unclosed\n",
    "bad-matcher": "[[backend_pools]]\nmatcher = \"Host('x\"\naddresses = [\"127.0.0.1:1\"]\n"
                   "schemes = [\"HTTP\"]\nstrategy = { RoundRobin = {} }\n",
    "unknown-strategy": "[[backend_pools]]\nmatcher = \"Host('x')\"\naddresses = [\"127.0.0.1:1\"]"
                        "\nschemes = [\"HTTP\"]\nstrategy = { Teleport = {} }\n",
    "unknown-scheme": "[[backend_pools]]\nmatcher = \"Host('x')\"\naddresses = [\"127.0.0.1:1\"]"
                      "\nschemes = [\"GOPHER\"]\nstrategy = { RoundRobin = {} }\n",
    "bad-address": "http_address = \"not an address\"\n",
    "missing-cert-file": "[certificates.\"a.test\"]\ntype = \"Local\"\n"
                         "certificate_path = \"/nonexistent/cert.pem\"\n"
                         "private_key_path = \"/nonexistent/key.pem\"\n",
}


@pytest.mark.parametrize("kind", list(INVALID_CONFIGS))
def test_check_invalid_config_exits_nonzero(binary, tmp_path, kind):
    http_port, https_port, metrics_port = _ports()
    head = ""
    if kind != "bad-address":
        head = ("http_address = \"127.0.0.1:%d\"\n" % http_port)
    head += ("https_address = \"127.0.0.1:%d\"\nprometheus_metrics_address = \"127.0.0.1:%d\"\n"
             % (https_port, metrics_port))
    if kind == "toml-syntax":
        text = INVALID_CONFIGS[kind]
    else:
        text = head + INVALID_CONFIGS[kind]
    result = _check(binary, _write(tmp_path, text))
    assert result.returncode != 0, "%s accepted: %s" % (kind, result.stdout + result.stderr)


def test_check_missing_file_exits_nonzero(binary, tmp_path):
    result = _check(binary, os.path.join(str(tmp_path), "does-not-exist.toml"))
    assert result.returncode != 0


def test_version_flag(binary):
    result = subprocess.run([binary, "--version"], capture_output=True, text=True, timeout=10)
    assert result.returncode == 0
    assert any(ch.isdigit() for ch in result.stdout)


def test_invalid_config_at_startup_exits_nonzero(binary, tmp_path):
    path = _write(tmp_path, "this is [[[ not toml\n")
    proc = subprocess.run([binary, "-c", path], capture_output=True, text=True, timeout=30,
                          cwd=str(tmp_path))
    assert proc.returncode != 0
