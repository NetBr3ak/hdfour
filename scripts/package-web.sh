#!/bin/sh
# Build the same browser package for a release download or an npm publish.
set -eu
cd "$(dirname "$0")/.."
mkdir -p "${1:-dist}"
output=$(cd "${1:-dist}" && pwd)
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT HUP INT TERM

sh demo/build.sh
cp crates/hdfour-wasm/package.json README.md LICENSE-MIT LICENSE-APACHE NOTICE "$stage/"
cp demo/pkg/hdfour_wasm.js demo/pkg/hdfour_wasm_bg.wasm demo/pkg/*.d.ts "$stage/"
(cd "$stage" && npm pack --offline --ignore-scripts --pack-destination "$output")
