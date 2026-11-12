#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
target=
case "$#" in
    0) ;;
    2)
        if [ "$1" != --target ]; then
            echo 'Usage: sh bench/simd.sh [--target TARGET]' >&2; exit 2
        fi
        target=$2 ;;
    *) echo 'Usage: sh bench/simd.sh [--target TARGET]' >&2; exit 2 ;;
esac
architecture=${target:-$(uname -m)}
case "$architecture" in
    x86_64|x86_64-*|arm64|aarch64|aarch64-*) ;;
    *) echo 'Only x86-64 and ARM64 assembly are supported.' >&2; exit 2 ;;
esac
if [ -n "$target" ]; then
    if ! rustup target list --installed | grep -Fx "$target" >/dev/null; then
        echo 'Requested target is not installed.' >&2; exit 2
    fi
fi
inspection=$(mktemp -d "${TMPDIR:-/tmp}/hdfour-simd.XXXXXXXX")
trap 'rm -r "$inspection"' EXIT HUP INT TERM
if [ -n "$target" ]; then
    cargo rustc -p hdfour --release --lib --target-dir "$inspection" --target "$target" -- --emit=asm
    directory="$inspection/$target/release/deps"
else
    cargo rustc -p hdfour --release --lib --target-dir "$inspection" -- --emit=asm
    directory="$inspection/release/deps"
fi
set -- "$directory"/hdfour-*.s
if [ ! -f "$1" ]; then
    echo 'No assembly output found.' >&2; exit 1
fi
awk -f bench/simd.awk "$@"
