# Resource limits

The reader checks each file against a set of limits while it reads metadata and
arrays. A file over a limit gives
`Error::LimitExceeded { limit, value, max }`. Its message reads
`<limit> limit exceeded: <value> > <max>`. Python raises `LimitError`, a
subclass of `ValueError`. JavaScript throws an `Error` with the same message.

## Defaults

`Limits::default()` in `crates/hdfour/src/limits.rs` sets these values. Rust
code can pass other values to `File::from_source`. `open` and
`File::from_bytes` use the defaults, and so does the Python binding.

| Field | Default | What it bounds | `limit` in the error |
|---|---|---|---|
| `max_descriptors` | 4,194,304 | Data descriptors in the file, and block slots of one linked element | `descriptors` |
| `max_dd_blocks` | 65,536 | Data descriptor blocks in the chain | `dd_blocks` |
| `max_group_depth` | 64 | Nesting of Vgroups, and nesting of groups in HDF-EOS2 `StructMetadata` | `group_depth` |
| `max_rank` | 32 | Dimensions of a dataset, a chunked element or an HDF-EOS2 field | `rank` |
| `max_array_bytes` | 2,147,483,648 (2 GiB) | Bytes of one decoded array, and of one element or chunk table read into memory | `array_bytes` |
| `max_attribute_bytes` | 67,108,864 (64 MiB) | Stored bytes of one attribute | `attribute_bytes` |
| `max_metadata_bytes` | 67,108,864 (64 MiB) | Metadata the reader builds during `open`, such as names, attribute values and `StructMetadata` text | `metadata_bytes` |
| `max_chunks` | 1,048,576 | Chunks in one dataset's chunk grid, and records in its chunk table | `chunks` |
| `max_chunk_header_bytes` | 256 | Body of a chunked element's header | `chunk_header_bytes` |

`max_chunk_header_bytes` cannot go above 256. The HDF4 C library refuses larger
chunk headers, and the reader applies the same cap.

## Fixed checks

These checks have no setting:

| Check | Result |
|---|---|
| An allocation larger than `isize::MAX` bytes | `LimitExceeded` with `limit` `allocation_bytes` |
| An offset or length past the end of the file, or past 2^31 - 1 | `Malformed` |
| DEFLATE output more than 1032 times its input | `Malformed` |
| RLE output more than 65 times its input | `Malformed` |

## Browser limits

The WebAssembly binding (`crates/hdfour-wasm/src/lib.rs`) uses tighter limits
for `File.fromBytes`:

| Limit | Value | `limit` in the error |
|---|---|---|
| File size | 268,435,456 bytes (256 MiB) | `file bytes` |
| `max_array_bytes` | 268,435,456 (256 MiB) | `array_bytes` |
| `max_attribute_bytes` | 1,048,576 (1 MiB) | `attribute_bytes` |
| `max_metadata_bytes` | 16,777,216 (16 MiB) | `metadata_bytes` |

The other limits keep their defaults.
