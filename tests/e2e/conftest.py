"""Fixtures for the RustStrom black-box e2e suite.

The binary under test comes from $RUSTSTROM_BIN (default: target/release/rust-strom
of this repository). Tests marked `v2` cover features that only v2 must have:
against a v1.x binary they run as non-strict xfail, against v2 they must pass.
Set RUSTSTROM_TARGET=v1|v2 to override the version detection.
"""

import os
import re
import subprocess

import pytest

from harness import certs
from harness.procs import Stack

REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))


def _binary():
    path = os.environ.get("RUSTSTROM_BIN") or os.path.join(REPO_ROOT, "target", "release",
                                                            "rust-strom")
    return os.path.abspath(path)


def _detect_major(binary):
    override = os.environ.get("RUSTSTROM_TARGET", "").strip().lower()
    if override in ("v1", "v2"):
        return int(override[1])
    try:
        out = subprocess.run([binary, "--version"], capture_output=True, text=True,
                             timeout=10).stdout
    except OSError:
        return None
    match = re.search(r"(\d+)\.(\d+)\.(\d+)", out)
    return int(match.group(1)) if match else None


def pytest_configure(config):
    config.addinivalue_line("markers", "v2: feature required from v2 only (xfail on v1.x)")
    config.addinivalue_line("markers", "slow: takes more than ~20 seconds")
    binary = _binary()
    config._ruststrom_bin = binary
    config._ruststrom_major = _detect_major(binary) if os.path.exists(binary) else None


def pytest_report_header(config):
    return "rust-strom binary: %s (major version: %s)" % (config._ruststrom_bin,
                                                        config._ruststrom_major)


def pytest_collection_modifyitems(config, items):
    if config._ruststrom_major is not None and config._ruststrom_major >= 2:
        return
    for item in items:
        if item.get_closest_marker("v2"):
            item.add_marker(pytest.mark.xfail(reason="v2-only feature", strict=False))


@pytest.fixture(scope="session")
def binary(pytestconfig):
    path = pytestconfig._ruststrom_bin
    if not os.access(path, os.X_OK):
        pytest.exit("rust-strom binary not found or not executable: %s (set RUSTSTROM_BIN)"
                    % path, returncode=2)
    return path


@pytest.fixture(scope="session")
def is_v2(pytestconfig):
    return (pytestconfig._ruststrom_major or 0) >= 2


@pytest.fixture(scope="session")
def pki(tmp_path_factory):
    return certs.generate(str(tmp_path_factory.mktemp("pki")))


@pytest.fixture
def stack(request, binary, pki, tmp_path):
    s = Stack(binary, tmp_path, pki)
    request.node._e2e_stacks = [s]
    yield s
    s.close()


def make_module_stack(request, binary, pki, tmp_path_factory, name):
    """For module-scoped fixtures that share one LB between tests."""
    s = Stack(binary, tmp_path_factory.mktemp(name), pki)
    request.addfinalizer(s.close)
    return s


@pytest.hookimpl(hookwrapper=True)
def pytest_runtest_makereport(item, call):
    outcome = yield
    report = outcome.get_result()
    if report.when == "call" and report.failed:
        stacks = list(getattr(item, "_e2e_stacks", []))
        for fixture_value in item.funcargs.values():
            lbs = getattr(fixture_value, "lbs", None)
            if lbs is None and hasattr(fixture_value, "log_tail"):
                lbs = [fixture_value]
            if lbs:
                stacks.append(type("S", (), {"lbs": lbs})())
        seen = set()
        for s in stacks:
            for lb in s.lbs:
                if id(lb) in seen:
                    continue
                seen.add(id(lb))
                report.sections.append(("rust-strom log %s" % lb.log_path, lb.log_tail(30)))
