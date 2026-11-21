#!/usr/bin/env bash
# Runs the RustStrom e2e suite. Creates a virtualenv on first use.
#
#   RUSTSTROM_BIN=/path/to/rust-strom tests/e2e/run.sh [pytest args]
#
# E2E_VENV        virtualenv location (default: tests/e2e/.venv)
# E2E_PORT_RANGE  TCP ports the suite may use (default: 21000-21999)
# RUSTSTROM_TARGET=v1|v2  override version detection for the `v2` marker
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
VENV="${E2E_VENV:-$HERE/.venv}"

if [ ! -x "$VENV/bin/pytest" ]; then
    python3 -m venv "$VENV"
    "$VENV/bin/pip" install -q --upgrade pip
    "$VENV/bin/pip" install -q -r "$HERE/requirements.txt"
fi

cd "$HERE"
exec "$VENV/bin/python" -m pytest "$@"
