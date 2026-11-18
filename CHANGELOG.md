# Changelog

This changelog follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Releases will use [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-11-18

### Fixed

- The repository check script runs from any working directory and keeps the
  Cargo lockfile unchanged.

- Unsupported SD datasets no longer prevent an HDF-EOS2 file from opening.
  Their EOS fields remain unlinked, and the datasets appear in `skipped()`.

### Added

- Rectangular dataset reads in Rust (`read_window`), Python (`read_window`) and
  JavaScript (`readWindow`). Uncompressed plain arrays read just the selected
  bytes. Chunked arrays decode only the chunks that intersect the selection.

- Linux and macOS CI for core checks, Python wheel tests and WebAssembly tests.
- A binding check script and pinned Python development tools.

- Rust `File::byte_len` and `File::read_into` for caller-allocated output buffers.
- Rust core: `open`, `File::from_bytes` and `File::from_source` read all metadata;
  `File::read` reads one whole dataset as raw values in C order.
- HDF4 container: header check, data descriptor blocks, linked, compressed and
  chunked special elements.
- SD layout with a `CDF0.0` Vgroup: datasets, dimensions (unlimited too),
  coordinate variables, and attributes on the file, datasets and dimensions.
- Number types i8, u8, i16, u16, i32, u32, f32, f64, char8 and uchar8, in both
  byte orders.
- Codecs none, DEFLATE and RLE.
- Fill values for datasets and chunks that were never written.
- HDF-EOS2: `StructMetadata` parts joined in order, an ODL parser, grids and
  swaths with fields linked to datasets, dimension maps and raw parameters.
- Refusals by feature name: `Error::Unsupported` on read, `File::skipped()` for
  objects left out at open.
- Resource limits (`Limits`) for descriptors, DD blocks, group depth, rank, array
  bytes, attribute bytes, metadata bytes, chunks and chunk header bytes.
- Python binding (`hdfour`) with NumPy arrays, a type stub and its own exception
  classes.
- WebAssembly binding with typed arrays, and a one-page browser inspector in
  `demo/`.
- Fixtures F01 to F21 written through the HDF4 C library, with golden files;
  corpus files R02 and R03 from the HDF4 test suite.
- Golden, property, Python, WebAssembly and fuzz tests; a benchmark harness.
- Contribution guidance, reporting policies and licences.
