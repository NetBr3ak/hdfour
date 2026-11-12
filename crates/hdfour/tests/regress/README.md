# Fuzz regressions

Store a minimised crashing or hanging input here with a deterministic Rust test
that reproduces the failure. Use `main.rs` as the integration-test entry point;
Cargo discovers `tests/regress/main.rs` without changing the crate manifest.
Name the original fuzz target and its pinned toolchain in the report. Keep the
test failing until the core owner fixes the cause. Do not add an ignored test,
raise a timeout or change core code in the harness batch.
