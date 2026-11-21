"""Process management: backend servers and the rust-strom binary under test."""

import os
import signal
import socket
import subprocess
import sys
import time
import urllib.request

from . import config as cfg
from .ports import free_port

HERE = os.path.dirname(os.path.abspath(__file__))
BACKEND_SCRIPT = os.path.join(HERE, "backend_server.py")


def wait_for_port(port, timeout=10.0, host="127.0.0.1", proc=None):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if proc is not None and proc.poll() is not None:
            return False
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
            sock.settimeout(0.5)
            if sock.connect_ex((host, port)) == 0:
                return True
        time.sleep(0.05)
    return False


class Backend:
    """A backend_server.py subprocess."""

    def __init__(self, name, workdir, delay_ms=0, tls=None, port=None):
        self.name = name
        self.port = port or free_port()
        self.address = "127.0.0.1:%d" % self.port
        self.log_path = os.path.join(workdir, "backend-%s.log" % name)
        args = [sys.executable, BACKEND_SCRIPT, "--port", str(self.port), "--name", name,
                "--delay", str(delay_ms)]
        if tls:
            args += ["--tls-cert", tls[0], "--tls-key", tls[1]]
        self.tls = bool(tls)
        self._log = open(self.log_path, "ab")
        self.proc = subprocess.Popen(args, stdout=self._log, stderr=subprocess.STDOUT)
        if not wait_for_port(self.port, proc=self.proc):
            self.kill()
            raise RuntimeError("backend %s did not start, see %s" % (name, self.log_path))

    def ctl(self, **params):
        """Changes backend state directly (not through the LB): delay=MS, healthy=0|1."""
        if self.tls:
            raise RuntimeError("ctl is not supported on TLS backends")
        query = "&".join("%s=%s" % kv for kv in params.items())
        with urllib.request.urlopen("http://%s/__ctl?%s" % (self.address, query), timeout=5):
            pass

    def stats(self):
        import json
        with urllib.request.urlopen("http://%s/__stats" % self.address, timeout=5) as r:
            return json.loads(r.read())

    def alive(self):
        return self.proc.poll() is None

    def kill(self):
        if self.proc.poll() is None:
            self.proc.kill()
            self.proc.wait(5)
        self._log.close()


class LoadBalancer:
    """The rust-strom binary running with a config file in its own directory."""

    def __init__(self, binary, workdir, http_port, https_port, metrics_port):
        self.binary = binary
        self.workdir = workdir
        self.http_port = http_port
        self.https_port = https_port
        self.metrics_port = metrics_port
        self.config_path = os.path.join(workdir, "config.toml")
        self.log_path = os.path.join(workdir, "ruststrom.log")
        self.proc = None
        self._log = None

    # --- config --------------------------------------------------------------
    def write_config(self, text):
        """In-place write (truncate + write), like `cat > config.toml`."""
        with open(self.config_path, "w") as f:
            f.write(text)
            f.flush()
            os.fsync(f.fileno())

    def write_config_atomic(self, text):
        """Write to a temp file in the same directory, then rename over the config."""
        tmp = os.path.join(self.workdir, ".config.toml.tmp")
        with open(tmp, "w") as f:
            f.write(text)
            f.flush()
            os.fsync(f.fileno())
        os.rename(tmp, self.config_path)

    # --- lifecycle -------------------------------------------------------------
    def start(self, text, wait_https=True, wait_metrics=True, timeout=15.0):
        self.write_config(text)
        env = dict(os.environ)
        env.setdefault("RUST_LOG", "info")
        env.setdefault("LOG_LEVEL", env["RUST_LOG"])
        self._log = open(self.log_path, "ab")
        self.proc = subprocess.Popen([self.binary, "-c", self.config_path], cwd=self.workdir,
                                     stdout=self._log, stderr=subprocess.STDOUT, env=env)
        ports = [self.http_port]
        if wait_https:
            ports.append(self.https_port)
        if wait_metrics:
            ports.append(self.metrics_port)
        for port in ports:
            if not wait_for_port(port, timeout=timeout, proc=self.proc):
                tail = self.log_tail()
                self.kill()
                raise RuntimeError("rust-strom did not listen on %d:\n%s" % (port, tail))
        return self

    def alive(self):
        return self.proc is not None and self.proc.poll() is None

    def signal(self, sig):
        self.proc.send_signal(sig)

    def wait(self, timeout):
        return self.proc.wait(timeout)

    def stop(self, timeout=10.0):
        if self.proc is None:
            return
        if self.proc.poll() is None:
            self.proc.send_signal(signal.SIGTERM)
            try:
                self.proc.wait(timeout)
            except subprocess.TimeoutExpired:
                self.kill()
        if self._log:
            self._log.close()
            self._log = None

    def kill(self):
        if self.proc is not None and self.proc.poll() is None:
            self.proc.kill()
            self.proc.wait(5)
        if self._log:
            self._log.close()
            self._log = None

    def log_tail(self, lines=40):
        try:
            with open(self.log_path, "rb") as f:
                data = f.read().decode("utf-8", "replace").splitlines()
            return "\n".join(data[-lines:])
        except OSError:
            return "<no log>"

    def log_text(self):
        with open(self.log_path, "rb") as f:
            return f.read().decode("utf-8", "replace")


class Stack:
    """Owns everything a test starts and tears it down afterwards."""

    def __init__(self, binary, workdir, pki=None):
        self.binary = binary
        self.workdir = str(workdir)
        self.pki = pki
        self.backends = []
        self.lbs = []
        os.makedirs(self.workdir, exist_ok=True)

    def backend(self, name, delay_ms=0, tls=False):
        tls_files = (self.pki.cert, self.pki.key) if tls else None
        backend = Backend(name, self.workdir, delay_ms=delay_ms, tls=tls_files)
        self.backends.append(backend)
        return backend

    def backends_named(self, *names, **kwargs):
        return [self.backend(n, **kwargs) for n in names]

    def lb_ports(self):
        return free_port(), free_port(), free_port()

    def certificates(self, names=None):
        names = names or self.pki.hosts
        return {name: (self.pki.cert, self.pki.key) for name in names}

    def lb(self, pools, certificates=None, check_every=1, extra_global=None, start=True,
           subdir=None, **start_kwargs):
        """Builds a config, starts rust-strom and waits until it listens."""
        http_port, https_port, metrics_port = self.lb_ports()
        workdir = os.path.join(self.workdir, subdir or "lb%d" % len(self.lbs))
        os.makedirs(workdir, exist_ok=True)
        lb = LoadBalancer(self.binary, workdir, http_port, https_port, metrics_port)
        if certificates is None and self.pki is not None:
            certificates = self.certificates()
        lb.config_args = dict(certificates=certificates, check_every=check_every,
                              extra_global=extra_global)
        text = self.render(lb, pools)
        self.lbs.append(lb)
        if start:
            lb.start(text, **start_kwargs)
        else:
            lb.write_config(text)
        return lb

    @staticmethod
    def render(lb, pools, **overrides):
        args = dict(lb.config_args)
        args.update(overrides)
        return cfg.config(lb.http_port, lb.https_port, lb.metrics_port, pools, **args)

    def close(self):
        for lb in self.lbs:
            lb.kill()
        for backend in self.backends:
            backend.kill()
