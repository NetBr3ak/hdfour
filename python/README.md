# Python API

Build the extension in an active virtual environment with `maturin develop`.
The build backend is pinned in [pyproject.toml](../pyproject.toml).

```python
import hdfour

file = hdfour.open("sample.hdf")
for dataset in file.datasets:
    print(dataset.name, dataset.dtype, dataset.shape)
    values = dataset.read()
```

`hdfour.from_bytes(data)` accepts file contents as `bytes`. Both entry points read
metadata first. Dataset payloads are decoded when `read()` is called. A dataset
retains its file, so it remains readable after the file variable is deleted.

`file[name]` selects one dataset. Missing and duplicate names raise `KeyError`.
Use `file.datasets` to select duplicate names by their SD index.

## Metadata

Properties return snapshots. Changing a list, dictionary or attribute array does
not change the file or a later read. File, dataset and dimension properties cannot
be assigned.

| Property | Python value |
|---|---|
| `file.datasets` | List of `Dataset` objects in SD selection order |
| `file.attrs`, `dataset.attrs`, `dimension.attrs` | Dictionary in file order; `char8` is a Latin-1 string, other types are one-dimensional NumPy arrays |
| `file.skipped` | List of `(object, feature)` tuples |
| `file.eos` | `None`, or a dictionary with `grids` and `swaths` |
| `dataset.dtype` | HDF4 type name: `i8`, `u8`, `i16`, `u16`, `i32`, `u32`, `f32`, `f64`, `char8` or `uchar8` |
| `dataset.shape` | List of axis lengths |
| `dataset.dims` | List of `Dimension` objects with `name`, `size`, `unlimited` and `attrs` |
| `dataset.is_coordinate` | Boolean |
| `dataset.storage` | Dictionary with `kind`: `empty`, `contiguous`, `linked`, `chunked` or `external`; chunked storage also has `chunk_shape` |
| `dataset.codec` | `none`, `rle`, `deflate`, or an integer code |

EOS objects keep `name`, `dims`, `fields`, `dimension_maps` and raw `params`.
Dimension and parameter pairs are tuples. A field's `dataset` is an index into
`file.datasets`, or `None`. The [type stub](hdfour/__init__.pyi) lists every key.

## Values and errors

`dataset.read()` returns a C-order NumPy array in native byte order. It does not
apply scale, offset or fill masking. `char8` arrays use `int8`; `uchar8` uses
`uint8`. Decoding releases the Python global interpreter lock. The output buffer
is transferred to NumPy without a copy. Arrays remain valid after the dataset is
deleted, and each read returns a separate buffer.

`FormatError` and `LimitError` inherit from `ValueError`. `UnsupportedError`
inherits from `NotImplementedError` and names the refused feature. File I/O
failures raise `OSError`.

## Tests

In the development virtual environment, run:

```sh
maturin develop && pytest -q python/tests
sh scripts/check.sh
```

The differential tests use pyhdf by SD index, not its name-keyed dictionary. They
visit F01 through F21 and corpus files present in the checkout. When pyhdf cannot
read a dataset, the value comparison is skipped with its error. The type tests
also compare big-endian, little-endian and never-written data with the C-library
golden bytes. Expected refusals are checked as errors, not treated as matches.
