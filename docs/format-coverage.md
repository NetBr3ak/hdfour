# Format coverage

This table lists each HDF4 feature, what the reader does with it and the test
input that checks it. F files are in [fixtures/](../fixtures/README.md); R02
and R03 are in [corpus/](../corpus/README.md). The Rust golden tests compare
every readable value with the bytes that the HDF4 C library's `SDreaddata`
returns.

"Read" means the reader returns the values. "Refused on read" means `read`
returns `Error::Unsupported` with the named feature. "Skipped" means `open`
succeeds and `File::skipped()` lists the object with the named feature.

## Read

| Feature | Test input |
|---|---|
| Header and data descriptor blocks | every fixture |
| Ten number types, big-endian | F01 |
| Ten number types, little-endian | F02 |
| Attributes of every type on the file, datasets and dimensions | F03 |
| Shared dimensions, coordinate variables, DimVal0.0 and DimVal0.1 | F04 |
| Unlimited first dimension and linked storage | F05 |
| DEFLATE and plain storage, duplicate dataset names | F06 |
| Chunked storage with partial edge chunks | F07 |
| Chunked storage with DEFLATE | F08 |
| RLE | F09 |
| Datasets never written: fill value, or an empty array | F13 |
| Vgroup with a version 4 attribute record | F17 |
| Chunks never written: the chunk header's fill value | F19 |
| Compressed dataset never written: fill value | F20 |
| HDF-EOS2 grid | F15 |
| HDF-EOS2 grid with `StructMetadata` split into three parts | F18 |
| HDF-EOS2 swath with a dimension map | F16 |

## Refused on read

| Feature | Feature name | Test input |
|---|---|---|
| SZIP compression (code 5) | `szip` | R03 |
| NBIT compression (code 2) | `nbit` | F11 |
| Skipping Huffman compression (code 3) | `skipping-huffman` | F10 |
| JPEG compression (code 7) | `jpeg` | unit test in `crates/hdfour/src/codec.rs` |
| IMCOMP compression (code 12) | `imcomp` | unit test in `crates/hdfour/src/codec.rs` |
| Any other compression code | `codec` | unit test in `crates/hdfour/src/codec.rs` |
| Data in an external file | `external-element` | F12 |
| `_FillValue` of another type than the dataset | `fill-type-mismatch` | F03, edited in `crates/hdfour/tests/golden.rs` |

## Skipped

| Feature | Feature name | Test input |
|---|---|---|
| Unknown number type of a dataset or attribute | `number-type` | F03, edited in `crates/hdfour/tests/golden.rs` |
| Ragged array | `ragged-array` | F03, edited in `crates/hdfour/tests/golden.rs` |
| Vdata stored without interlace | `vdata-interlace` | built in `crates/hdfour/tests/sq7.rs` |
| Legacy SD layout without `CDF0.0` | `legacy-sd-layout` | F14, R02 |
| HDF-EOS2 point | `eos-point` | `fixtures/odl/point.odl` |
| HDF-EOS2 field stored as Vdata | `vdata-field` | F16 |
| HDF-EOS2 merged field | `merged-field` | F16 |
| HDF-EOS2 index dimension map | `index-dimension-map` | F16 |

## Errors

| Case | Error | Test input |
|---|---|---|
| Stored element shorter than its array | `Malformed` | F21 |
| Chunk table record with tag 0 | `Malformed` | F07, edited in `crates/hdfour/tests/golden.rs` |
| Not an HDF4 header | `NotHdf4` | Python test `python/tests/test_errors.py` |
| A resource limit is exceeded | `LimitExceeded` | see [limits.md](limits.md) |
