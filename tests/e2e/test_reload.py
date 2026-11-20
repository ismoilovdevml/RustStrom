"""Configuration hot reload."""

import time

import pytest

from harness import client
from harness import config as cfg

RELOAD_TIMEOUT = 10.0


def _wait_for_backend(lb, expected, timeout=RELOAD_TIMEOUT):
    deadline = time.monotonic() + timeout
    last = None
    while time.monotonic() < deadline:
        last = client.try_request(lb.http_port, timeout=5)
        if last is not None and last.status == 200 and last.backend == expected:
            return time.monotonic()
        time.sleep(0.1)
    raise AssertionError("reload not picked up within %.0fs, last response %r"
                         % (timeout, last))


def _pools(backend):
    return [cfg.pool("Host('host.test')", [backend.address])]


@pytest.mark.parametrize("mode", ["atomic-rename", "in-place-write"])
def test_reload_is_picked_up(stack, mode):
    a, b = stack.backends_named("a", "b")
    lb = stack.lb(_pools(a))
    assert client.request(lb.http_port).backend == "a"
    text = stack.render(lb, _pools(b))
    if mode == "atomic-rename":
        lb.write_config_atomic(text)
    else:
        lb.write_config(text)
    _wait_for_backend(lb, "b")
    assert lb.alive()


def test_successive_reloads(stack):
    backends = stack.backends_named("a", "b", "c")
    lb = stack.lb(_pools(backends[0]))
    for i in range(1, 5):
        target = backends[i % 3]
        lb.write_config_atomic(stack.render(lb, _pools(target)))
        _wait_for_backend(lb, target.name)


INVALID = {
    "toml-syntax": "this is [[[ not toml\n",
    "bad-matcher": None,  # filled in the test, needs ports
    "unknown-strategy": None,
    "truncated": None,
}


@pytest.mark.parametrize("kind", list(INVALID))
def test_invalid_config_keeps_old_one(stack, kind):
    a, b, c = stack.backends_named("a", "b", "c")
    lb = stack.lb(_pools(a))
    good_b = stack.render(lb, _pools(b))
    if kind == "toml-syntax":
        bad = INVALID[kind]
    elif kind == "bad-matcher":
        bad = stack.render(lb, [cfg.pool("Host('unclosed", [b.address])])
    elif kind == "unknown-strategy":
        bad = stack.render(lb, [cfg.pool("Host('host.test')", [b.address],
                                         strategy="{ Teleport = {} }")])
    else:
        bad = good_b[: len(good_b) // 2] + "\n[[backend_pools\n"
    lb.write_config(bad)
    time.sleep(3)
    assert lb.alive(), "invalid config killed the process"
    for _ in range(5):
        r = client.request(lb.http_port)
        assert r.status == 200 and r.backend == "a", r
    # the watcher still works after a rejected config
    lb.write_config_atomic(stack.render(lb, _pools(c)))
    _wait_for_backend(lb, "c")


def test_deleted_then_recreated_config(stack):
    """`rm config && cp new config` style deploys must not lose the config watcher."""
    import os
    a, b = stack.backends_named("a", "b")
    lb = stack.lb(_pools(a))
    os.remove(lb.config_path)
    time.sleep(2)
    assert lb.alive()
    assert client.request(lb.http_port).backend == "a"
    lb.write_config(stack.render(lb, _pools(b)))
    _wait_for_backend(lb, "b")


@pytest.mark.v2
def test_reload_under_constant_load_has_zero_errors(stack):
    backends = stack.backends_named("a", "b", "c")
    configs = [
        [cfg.pool("Host('host.test')", [backends[0].address, backends[1].address])],
        [cfg.pool("Host('host.test')", [backends[1].address, backends[2].address],
                  strategy=cfg.LEAST_CONNECTION)],
        [cfg.pool("Host('host.test')", [b.address for b in backends],
                  middlewares={"Compression": {}})],
    ]
    lb = stack.lb(configs[0])
    with client.LoadLoop(lb.http_port, threads=4, pause=0.002) as loop:
        for i in range(1, 10):
            lb.write_config_atomic(stack.render(lb, configs[i % len(configs)]))
            time.sleep(1.2)
        time.sleep(1)
    assert loop.total() > 200, loop.outcomes
    assert loop.non_2xx() == 0, "errors during reload: %s first=%s" % (
        dict(loop.outcomes), loop.errors[:5])
    assert set(loop.backends) == {"a", "b", "c"}, loop.backends
