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
The minimum supported Rust version is 1.85.

Run `sh scripts/check.sh` on the development runner before submitting a change.
It checks formatting, Clippy, tests, documentation, Ruff and handwritten unsafe
code. The local development policy reserves builds and tests for that runner.

Use the pull request checklist. Report security issues through GitHub private
vulnerability reporting, as described in [SECURITY.md](SECURITY.md).
Follow the [Code of Conduct](CODE_OF_CONDUCT.md).

Contributions use `MIT OR Apache-2.0`, with third-party notices retained.
There is no contributor licence agreement (CLA).
