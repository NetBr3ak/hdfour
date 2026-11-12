#!/bin/sh
set -eu

cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --exclude hdfour-py
RUSTDOCFLAGS=-Dwarnings cargo doc --workspace --no-deps
ruff check
ruff format --check

# A module that parses file bytes must keep its lint list: a malformed file gives an error, never a panic.
for module in chunk codec dd element eos nt odl read sd special vset; do
    if ! grep -q 'clippy::indexing_slicing' "crates/hdfour/src/$module.rs"; then
        echo "crates/hdfour/src/$module.rs lost its lint list." >&2
        exit 1
    fi
done

if grep -Enr '\bunsafe\b' crates --include='*.rs'; then
    echo 'Handwritten unsafe code is not allowed.' >&2
    exit 1
else
    status=$?
    if [ "$status" -ne 1 ]; then
        exit "$status"
    fi
fi
