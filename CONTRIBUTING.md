# Contributing

The reader is under development. No packages have been published.

Each feature needs a fixture made through the HDF4 C library, a golden file
containing its reference output, a specification reference and a changelog entry.
A claim that a feature is supported also needs the golden comparison and fuzz
evidence required by the build specification.

Write a failing test before changing the implementation. Keep existing tests.
If a test appears wrong, explain which requirement contradicts it before changing
it. Keep changes focused. Use clear names and checked arithmetic for file values.
Do not add handwritten unsafe code or dependencies outside the approved pins.
The core and bindings support Rust 1.85. Development tests need Rust 1.88 or
later because the pinned proptest version requires it.

Use Python 3.10 or later, Node.js 22 or later and a Rust toolchain with rustfmt,
Clippy and the `wasm32-unknown-unknown` target. In a virtual environment, run
`python -m pip install --group test` to install the pinned development tools
(with pip 25.1 or later). Install `wasm-bindgen-cli` 0.2.130 with
`cargo install wasm-bindgen-cli --version 0.2.130 --locked`.

Run `sh scripts/check.sh` and `sh scripts/check-bindings.sh` before submitting a
change. The first checks formatting, Clippy, core tests, documentation, Ruff
and handwritten unsafe code. The second builds and installs a Python wheel,
runs the Python tests, builds WebAssembly and runs the JavaScript tests against
the generated module. It changes only the active Python environment.
GitHub Actions runs both commands on Linux and macOS.

Use the pull request checklist. Report security issues through GitHub private
vulnerability reporting, as described in [SECURITY.md](SECURITY.md).
Follow the [Code of Conduct](CODE_OF_CONDUCT.md).

Contributions use `MIT OR Apache-2.0`, with third-party notices retained.
There is no contributor licence agreement (CLA).
