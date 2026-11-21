"""TCP port allocation restricted to a fixed range (default 21000-21999).

The range can be changed with E2E_PORT_RANGE="start-end". Under pytest-xdist
each worker gets its own slice of the range so workers never collide.
"""

import itertools
import os
import socket
import threading

_lock = threading.Lock()
_iterator = None


def _range():
    spec = os.environ.get("E2E_PORT_RANGE", "21000-21999")
    start, end = (int(x) for x in spec.split("-"))
    worker = os.environ.get("PYTEST_XDIST_WORKER", "")
    count = int(os.environ.get("PYTEST_XDIST_WORKER_COUNT", "1") or 1)
    if worker.startswith("gw") and count > 1:
        index = int(worker[2:])
        size = (end - start + 1) // count
        start = start + index * size
        end = start + size - 1
    return start, end


def _is_free(port):
    for family, host in ((socket.AF_INET, "127.0.0.1"),):
        with socket.socket(family, socket.SOCK_STREAM) as sock:
            sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            try:
                sock.bind((host, port))
            except OSError:
                return False
    # Also make sure nothing listens on the wildcard address
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.settimeout(0.2)
        if sock.connect_ex(("127.0.0.1", port)) == 0:
            return False
    return True


def free_port():
    """Returns a currently free port from the allowed range (round robin)."""
    global _iterator
    with _lock:
        start, end = _range()
        if _iterator is None:
            _iterator = itertools.cycle(range(start, end + 1))
        for _ in range(end - start + 1):
            port = next(_iterator)
            if _is_free(port):
                return port
    raise RuntimeError("no free port in range %s-%s" % (start, end))
