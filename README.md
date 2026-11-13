# hdfour

Status: work in progress. Nothing is published to crates.io, PyPI or npm.

hdfour reads HDF4 and HDF-EOS2 files without the HDF4 C library. The goal for
version 0.1: return every value of every dataset in scope bit-exactly as the HDF4
C library's `SDreaddata` returns it, or an error that names the unsupported feature.
This is a development target. [Tests](#tests) lists what the tests check today.

hdfour is an independent project. It is not affiliated with, endorsed by or supported by The HDF Group or NASA. HDF is a trade mark of The HDF Group.

The reader is read-only. It returns raw stored values: it does not apply scale
or offset and does not mask fill values. Data that was never written reads as
the fill value.

## Install

All three bindings build from source. You need Rust 1.85 or later.

### Rust

Add the core crate as a path dependency:

```toml
[dependencies]
hdfour = { path = "path/to/hdfour/crates/hdfour" }
```

The `parallel` feature is on by default. It reads the chunks of one dataset on
several threads. The WebAssembly binding turns it off.

### Python

You need Python 3.10 or later. In a virtual environment at the repository root:

```sh
pip install maturin==1.16.0 numpy
maturin develop
```

### WebAssembly

You need the `wasm32-unknown-unknown` Rust target and
`wasm-bindgen-cli` 0.2.129. From the repository root:

```sh
sh demo/build.sh
```

The script writes the module and its JavaScript glue to `demo/pkg/`.

## Examples

The Rust and Python examples read the committed fixture `fixtures/F01.hdf`.

Rust:

```rust
fn main() -> hdfour::Result<()> {
    let file = hdfour::open("fixtures/F01.hdf")?;
    let dataset = file.dataset("i16")?;
    let array = file.read(dataset)?;
    println!("{} {:?}", dataset.name, array.shape); // i16 [3, 5]
    Ok(())
}
```

To print all metadata of a file: `cargo run -p hdfour --example ls -- fixtures/F01.hdf`.

Python:

```python
import hdfour

file = hdfour.open("fixtures/F01.hdf")
dataset = file["i16"]
print(dataset.shape)  # [3, 5]
values = dataset.read()  # NumPy array, C order, native byte order
```

More in [docs/python.md](docs/python.md).

JavaScript, with the module from `sh demo/build.sh`:

```js
import init, { File } from './pkg/hdfour_wasm.js';

await init();
const file = File.fromBytes(bytes); // bytes: a Uint8Array with the file contents
try {
  const datasets = file.datasets();
  const values = file.read(0); // a typed array
} finally {
  file.free();
}
```

## Scope

The reader handles:

- the HDF4 header, data descriptor blocks and special elements;
- the SD layout with a `CDF0.0` Vgroup: datasets, dimensions (unlimited too),
  coordinate variables and attributes on the file, datasets and dimensions;
- the types i8, u8, i16, u16, i32, u32, f32, f64, char8 and uchar8, big-endian
  and little-endian;
- contiguous, linked and chunked storage;
- the codecs none, DEFLATE and RLE;
- datasets and chunks that were never written: they read as the fill value;
- HDF-EOS2 `StructMetadata`: grids and swaths, their fields linked to datasets,
  dimension maps and raw parameters.

Some features stop `read`. It returns an `Unsupported` error
with one of these feature names: `szip`, `nbit`, `skipping-huffman`, `jpeg`,
`imcomp`, `codec` (any other compression code), `external-element` and
`fill-type-mismatch`. A chunked dataset whose chunk table uses an unsupported
layout or type gives `vdata-interlace` or `number-type`.

Other features do not stop `open`. The reader leaves the object out and lists it
in `skipped()` with one of these names: `number-type`, `ragged-array`,
`vdata-interlace`, `legacy-sd-layout`, `eos-point`, `vdata-field`, `merged-field`
and `index-dimension-map`.

A file with a broken structure gives a `Malformed` error. So does an offset or
length past the end of the file or past 2^31 - 1. The parsing modules deny
indexing, unchecked arithmetic, `unwrap` and `panic` through Clippy lints.

There is no API for writing, raster images, annotations, Vdata tables or HDF5.
The reader does not compute projections or coordinates.

The table in [docs/format-coverage.md](docs/format-coverage.md) maps each feature
to the fixture that tests it.

## Limits

Every file is read under resource limits. A file over a limit gives a
`LimitExceeded` error. The Rust `Limits` defaults:

| Limit | Default |
|---|---|
| `max_descriptors` | 4,194,304 |
| `max_dd_blocks` | 65,536 |
| `max_group_depth` | 64 |
| `max_rank` | 32 |
| `max_array_bytes` | 2,147,483,648 (2 GiB) |
| `max_attribute_bytes` | 67,108,864 (64 MiB) |
| `max_metadata_bytes` | 67,108,864 (64 MiB) |
| `max_chunks` | 1,048,576 |
| `max_chunk_header_bytes` | 256 |

Rust code can set other limits with `File::from_source`. The browser binding uses
tighter limits: a file of at most 268,435,456 bytes (256 MiB), arrays of at most
256 MiB, attributes of at most 1 MiB and metadata of at most 16 MiB.

[docs/limits.md](docs/limits.md) explains what each limit bounds.

## Demo

The `demo/` folder holds a one-page browser inspector. It reads a file in the
browser, lists its datasets and previews a two-dimensional dataset in grey scale.
The page has no upload path.

1. Build the module: `sh demo/build.sh`.
2. Serve the repository root with any static HTTP server.
3. Open `/demo/` and pick an HDF4 file, or drop one on the page.

There is no public address for the demo. See [demo/README.md](demo/README.md).

## Benchmarks

The harness in `bench/` compares hdfour with pyhdf 0.11.7 and with GDAL built
with `libgdal-hdf4`. Speed comes from hyperfine: 3 warm-ups and 20 runs of one
read script per reader. Memory is the median peak RSS of 5 runs. Each reader must
first match the C library's golden bytes.

The latest measured run: 2026-11-13, revision `026a6b9`, hdfour 0.1.0, pyhdf
0.11.7 (HDF4 4.2.15), GDAL 3.13.2, hyperfine 2.0.0, Python 3.12.16, Linux x86_64,
Intel Core i5-8350U, 16 GB of memory, an otherwise idle machine. Each time is the
median of 20 runs; each memory figure is the median peak RSS of 5 runs (GNU time,
KiB divided by 1024).

| File | Reader | Median time | Median peak RSS |
|---|---|---:|---:|
| `chunked.hdf`: four 4800 by 4800 int16 datasets, DEFLATE-6 chunks | hdfour, one thread | 0.374 s | 73.2 MiB |
| | hdfour, all threads | 0.294 s | 75.2 MiB |
| | pyhdf | 0.320 s | 78.0 MiB |
| | GDAL | 0.459 s | 160.3 MiB |
| `contiguous.hdf`: one 8192 by 8192 float32 dataset | hdfour, one thread | 0.326 s | 284.7 MiB |
| | hdfour, all threads | 0.323 s | 284.7 MiB |
| | pyhdf | 0.240 s | 285.4 MiB |
| | GDAL | 0.394 s | 579.7 MiB |
| R01: a 282,525-byte MODIS file (see [corpus/README.md](corpus/README.md)) | hdfour, one thread | 0.097 s | 28.9 MiB |
| | hdfour, all threads | 0.097 s | 28.9 MiB |
| | pyhdf | 0.102 s | 29.6 MiB |
| | GDAL | 0.145 s | 67.3 MiB |

In this run hdfour with all threads reads the chunked file in 8 percent less time
than pyhdf; with one thread it takes 1.17 times as long. On the contiguous file
hdfour takes 1.36 times as long as pyhdf. On R01 the two are level. The `parallel`
feature shortened the chunked read by 21 percent. hdfour's peak memory is close to
pyhdf's and less than half of GDAL's. The first run, at revision `3708d41`, measured
0.662 s on the chunked file and 0.370 s on the contiguous file for hdfour with one
thread; the fill of the output and the byte swap changed after it. These figures
describe one machine and three files. The method is in
[bench/README.md](bench/README.md).

## Tests

`sh scripts/check.sh` runs `cargo fmt --check`, Clippy with warnings as errors,
`cargo test` for the workspace without the Python crate, `cargo doc` with
warnings as errors, and Ruff. It also checks that each parsing module still names
the `clippy::indexing_slicing` lint and that no crate has handwritten `unsafe`
code.

The Rust golden tests open the fixtures F01 to F21 and the committed corpus files
R02 and R03. They compare metadata, refusals and skipped objects with the
expected results, and every readable value with the C library's output, byte for
byte. A property test checks that random bytes never cause a panic.

The fixtures were written through the HDF4 C library 4.2.15 and HDF-EOS2 2.20
from conda-forge, by `fixtures/c/gen.c`. The golden values are the bytes that
`SDreaddata` returns (`fixtures/c/sdsdump.c`); the metadata comes from pyhdf
(`fixtures/dump.py`). `sh fixtures/build.sh` rebuilds them on Linux x86-64 from
a locked environment. See [fixtures/README.md](fixtures/README.md) and
[corpus/README.md](corpus/README.md).

Other tests:

- Python: `pip install pytest==9.0.2 pyhdf==0.11.7`, then
  `maturin develop && pytest -q python/tests`. This includes a comparison with
  pyhdf.
- WebAssembly: `node demo/smoke.mjs` after `sh demo/build.sh`.
- Fuzzing: four cargo-fuzz targets, `open_read_all`, `codec_deflate`,
  `codec_rle` and `odl`. They need the pinned nightly toolchain and cargo-fuzz.
  Run them with `sh fuzz/run.sh`. See [fuzz/README.md](fuzz/README.md).

## Credits

The format layouts follow the HDF4 C sources. R02 and R03 are test files from the
HDF4 source tree. [NOTICE](NOTICE) carries their licence. The reader depends on
the Rust crates miniz_oxide and bytemuck. The bindings use PyO3, rust-numpy,
wasm-bindgen and js-sys.

## Licence

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or
[MIT](LICENSE-MIT), at your option. See also [NOTICE](NOTICE).
