# Python API

The `hdfour` package needs Python 3.10 or later and NumPy. Build it from source
as described in the [README](../README.md#install). The type stub is
[python/hdfour/__init__.pyi](../python/hdfour/__init__.pyi).

## Example

```python
import hdfour

file = hdfour.open("fixtures/F01.hdf")
for dataset in file.datasets:
    print(dataset.name, dataset.dtype, dataset.shape)

values = file["i16"].read()  # NumPy array of shape (3, 5)
print(file.skipped)  # []
print(file.eos)  # None
```

## Functions

| Function | Returns |
|---|---|
| `hdfour.open(path)` | A `File`. `path` is a `str` or a path-like object. |
| `hdfour.from_bytes(data)` | A `File` read from `bytes`. |

Both functions read all metadata. Neither function reads array values.

## `File`

| Member | Value |
|---|---|
| `datasets` | List of `Dataset` objects in SD selection order |
| `attrs` | File attributes as a dict in file order |
| `skipped` | List of `(object, feature)` tuples: objects left out and the feature name |
| `eos` | `None`, or a dict with `grids` and `swaths` |
| `file[name]` | The `Dataset` with that name. Raises `KeyError` if no dataset or more than one has the name. |

Two datasets can share a name. Select them by position in `file.datasets`.

## `Dataset`

| Member | Value |
|---|---|
| `name` | `str` |
| `dtype` | `"i8"`, `"u8"`, `"i16"`, `"u16"`, `"i32"`, `"u32"`, `"f32"`, `"f64"`, `"char8"` or `"uchar8"` |
| `shape` | List of axis lengths |
| `dims` | List of `Dimension` objects |
| `attrs` | Dataset attributes as a dict in file order |
| `is_coordinate` | `True` for a coordinate variable |
| `storage` | Dict with `kind`: `"empty"`, `"contiguous"`, `"linked"`, `"chunked"` or `"external"`. Chunked storage also has `chunk_shape`. |
| `codec` | `"none"`, `"rle"`, `"deflate"`, or the integer compression code |
| `read()` | The whole array as a NumPy array |
| `read_window(start, count)` | A NumPy array with shape `count`. Pass one zero-based start and one count per axis. A zero count gives an empty array. |

Both methods return raw values in C order and native byte order. It does not apply
scale or offset and does not mask fill values; data that was never written reads
as the fill value. `char8` gives `int8` and `uchar8` gives `uint8`.
The binding releases the global interpreter lock while it decodes.

## `Dimension`

| Member | Value |
|---|---|
| `name` | `str` |
| `size` | `int` |
| `unlimited` | `bool` |
| `attrs` | Dimension attributes as a dict in file order |

## Attributes

An attribute dict maps each name to its value. A `char8` value is a `str`
decoded as Latin-1. Other types are one-dimensional NumPy arrays.

## HDF-EOS2 metadata

`file.eos["grids"]` and `file.eos["swaths"]` are lists of dicts with these keys:

| Key | Value |
|---|---|
| `name` | `str` |
| `dims` | List of `(name, size)` tuples |
| `fields` | List of dicts with `name`, `dims`, `geolocation` and `dataset` |
| `dimension_maps` | List of dicts with `geo_dim`, `data_dim`, `offset` and `increment` |
| `params` | List of `(name, raw value)` tuples |

A field's `dataset` is an index into `file.datasets`, or `None`.

## Exceptions

| Exception | Base class | Cause |
|---|---|---|
| `hdfour.FormatError` | `ValueError` | Not an HDF4 file, or a broken structure |
| `hdfour.UnsupportedError` | `NotImplementedError` | A refused feature; the message names it |
| `hdfour.LimitError` | `ValueError` | A resource limit, see [limits.md](limits.md) |
| `OSError` | | The file cannot be opened or read |
| `KeyError` | | `file[name]` finds no dataset or more than one |

The feature names are listed in [format-coverage.md](format-coverage.md).
