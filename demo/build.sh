#!/bin/sh
set -eu

cd "$(dirname "$0")/.."
if [ "$(wasm-bindgen --version)" != 'wasm-bindgen 0.2.130' ]; then
    echo 'demo/build.sh requires wasm-bindgen-cli 0.2.130.' >&2
    exit 1
fi
cargo build -p hdfour-wasm --release --target wasm32-unknown-unknown
wasm-bindgen --target web --out-dir demo/pkg target/wasm32-unknown-unknown/release/hdfour_wasm.wasm
wc -c < demo/pkg/hdfour_wasm_bg.wasm
compressed=demo/pkg/hdfour_wasm_bg.wasm.gz
trap 'rm -f "$compressed"' EXIT HUP INT TERM
gzip -n -9 -c demo/pkg/hdfour_wasm_bg.wasm > "$compressed"
wc -c < "$compressed"
