#!/bin/sh
set -eu
cd "$(dirname "$0")/.."

wheel_dir=$(mktemp -d "${TMPDIR:-/tmp}/hdfour-wheel.XXXXXX")
trap 'rm -rf "$wheel_dir"' EXIT HUP INT TERM
python -m maturin build --locked --out "$wheel_dir"
python -m pip install --force-reinstall "$wheel_dir"/*.whl
python -m pytest -q python/tests

sh demo/build.sh
node --test demo/*.test.mjs demo/smoke.mjs
