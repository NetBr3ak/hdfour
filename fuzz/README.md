# Fuzz harness

Run these commands on a Linux build runner, not on the development computer.
The toolchain is pinned to nightly-2026-10-28. Install cargo-fuzz 0.13.2
(released 2026-06-09) with `cargo install cargo-fuzz --version 0.13.2 --locked`.

```sh
cargo +1.99.1 test --manifest-path fuzz/Cargo.toml --lib --test contracts
sh fuzz/run.sh 120
```

The default duration is 1800 seconds per target for the release check. A
120-second harness check does not satisfy the release's 30-minute requirement.
All targets build before the first run. A failing target does not prevent the
remaining targets from running; the script still exits nonzero.

| Target | Input |
| --- | --- |
| `open_read_all` | HDF4 bytes, followed through metadata and dataset reads. |
| `codec_deflate` | Two-byte little-endian output length, then a zlib stream. |
| `codec_rle` | Two-byte little-endian output length, then HDF4 RLE records. |
| `odl` | UTF-8 text, parsed as ODL and mapped to EOS objects. |

`seed.py` copies the committed F fixtures and ODL examples into a local corpus.
It also extracts compressed streams from fixture descriptors. Corpus growth,
build output and crash artifacts are ignored. Codec output is at most 65535
bytes. File parsing and dataset reads receive tighter `Limits`: arrays at most
1 MiB, attributes at most 256 KiB, 4096 descriptors, 128 DD blocks, rank 8,
group depth 16 and 1024 chunks.
The process RSS ceiling is 1024 MiB, and one input has a 10-second deadline.
These are failure detectors, not guarantees about untrusted files.

For a failure, keep the original artifact and minimise it on the same toolchain:

```sh
cd fuzz
cargo fuzz tmin open_read_all artifacts/open_read_all/crash-HASH
```

Replace the target and artifact path as needed. Put the minimised input and a
deterministic test in `crates/hdfour/tests/regress/`. Report the stack location
and reproduction command. Do not repair the core in this batch. Timeouts and
RSS failures also need triage; do not increase the limits to make them pass.
