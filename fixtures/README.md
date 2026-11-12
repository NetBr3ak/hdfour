# Reference fixtures

These files define the reference cases for the reader. Values come from the HDF4
C library, not from a second implementation of the file format. They do not by
themselves establish support for a satellite product.

## Rebuild on Linux x86-64

Run from the repository root. The host needs a C compiler, curl and SHA-256
utilities. The locked environment supplies Python. Nothing needs root access.

```sh
sh fixtures/build.sh
export PATH="$HOME/.local/share/hdfour-fixtures/env/bin:$PATH"
sh fixtures/build.sh && python -I fixtures/dump.py --check
```

`build.sh` downloads micromamba into the home directory, checks its SHA-256,
creates the environment from `conda-lock.txt`, or verifies an existing environment
against that lock. It compiles both C programs, regenerates F01 to F21, refreshes
the golden files, checks them, runs the fixture tests and writes `SHA256SUMS`.
The explicit lock records the package URLs, versions, builds and SHA-256 hashes.
It is for Linux x86-64 only. Environment files and compiler outputs are not data
that belongs in the repository.

The reference Python script always uses this locked environment. Its isolated
mode does not depend on a user Python path or site packages.

To check existing data without rebuilding:

```sh
python -I fixtures/dump.py --check
python -I fixtures/test_fixtures.py
(cd fixtures && .build/sdsdump --check F01.hdf golden/F01)
sha256sum -c fixtures/SHA256SUMS
```

For the anonymous real corpus, keep downloads and their goldens in the ignored
cache:

```sh
python -I corpus/fetch.py
python -I fixtures/dump.py --corpus
python -I fixtures/dump.py --check --corpus
```

The environment includes conda-forge HDF4 4.2.15, pyhdf 0.11.7 and HDF-EOS2 2.20.
HDF-EOS2 is available on conda-forge, so F15, F16 and F18 use its grid and swath
writer APIs. They are not hand-built approximations of its Vgroup layout.
After the EOS writer closes a file, the generator removes terminal metadata NUL
padding through the SD API. F18 then splits the text into three consecutive
attributes, including cuts inside tokens. Concatenation must precede parsing.

## Cases

Each HDF file is smaller than 65,536 bytes. Most arrays have shape 3 by 5.
Integer patterns use different high and low bytes; floating-point patterns use
exact binary fractions. The generator uses no clock or random input.

| File | Case | Reader expectation |
|---|---|---|
| F01 | All ten number types, big-endian | Values |
| F02 | All ten number types, little-endian | Values |
| F03 | All ten attribute types on file, dataset and dimension; raw fill and scale | Values |
| F04 | Shared named dimensions, two coordinate variables, DimVal0.0 and DimVal0.1 | Values |
| F05 | Unlimited first dimension, five single-record appends | Values |
| F06 | DEFLATE-6 and plain storage, deliberately duplicate dataset names | Values by index |
| F07 | Shape 5 by 7, chunks 2 by 3, partial edge chunks | Values |
| F08 | The F07 layout with DEFLATE-6 | Values |
| F09 | RLE | Values |
| F10 | Skipping Huffman | `unsupported:skipping-huffman` |
| F11 | NBIT | `unsupported:nbit` |
| F12 | External data in the companion F12.raw | `unsupported:external-element` |
| F13 | Ten never-written types, custom fill, and an empty unlimited dataset | Fill or an empty array |
| F14 | Legacy DFSDadddata layout, no CDF0.0 | `skipped:legacy-sd-layout` |
| F15 | EOS grid with one field | Values and the hand-written EOS expectation |
| F16 | EOS swath, regular map, 1D Vdata, merged fields and indexed map | Values plus four skipped objects |
| F17 | A Var0.0 Vgroup with a v4 attribute record | Values |
| F18 | The F15 grid with StructMetadata.0, .1 and .2 | The same values and EOS object as F15 |
| F19 | Sparse chunks: one stored chunk, 29 missing values | Chunk-header fill |
| F20 | Never-written compressed dataset with custom fill | Custom fill |
| F21 | SD_NOFILL partial write, then a deliberately shortened element | `malformed` |

## Golden file contract

`expect.json` is maintained by hand. Missing dataset overrides mean `values`.
Overrides use decimal SDselect indices, not names. They are `unsupported:<feature>`,
`skipped:<feature>` or `malformed`. Each file also has its exact `skipped` list and,
where applicable, its expected EOS object. No program in this toolchain derives
that EOS expectation. Raw EOS parameter values retain quotes and tuple syntax,
without the whitespace around the value. Parameters are the entries immediately
inside the grid or swath group, in source order, including its name and sizes.
The representation is the same for F15, F16, F18 and the cached R01 case.

For each committed input:

- `golden/ID.json` records the input filename and SHA-256, file attributes,
  datasets in SDselect order, the expected skipped objects and the EOS object.
- Each dataset has its index, name, numeric HDF dtype with DFNT_LITEND masked,
  shape, ordered dimensions, ordered attributes, `is_coordinate` and `expect`.
- A dimension has its name, current size, unlimited flag and ordered attributes.
- An attribute has its name, dtype, element count and exact little-endian bytes as
  lowercase `hex`. Character attributes are bytes, not normalized text.
- `golden/ID/INDEX.bin` is the native `SDreaddata` output, converted to
  little-endian on a big-endian host. There is no scale or offset conversion.
- `golden/ID/status.json` records every C read result and requested byte count.
  The same list appears as `c_reads` in the metadata JSON. A failed C read has no
  `.bin` file. In this locked build, F21 fails and R03 has no SZIP decoder.
  An empty array has zero bytes and requires no C read selection.

The Python walker selects datasets and attributes by index. F06 tests duplicate
names. pyhdf cannot read little-endian SDS values, so it supplies metadata only.
The C program supplies all value bytes, including F02. Legacy synthetic coordinate
variables appear in C and Python output, but their `expect` values mark them as
absent from the reader's dataset list. The skipped object list names only the
stored NDG objects, not those synthetic coordinate variables.

`dump.py --check` creates fresh C output in a temporary directory and compares
metadata, bytes, read status and file inventories. It never refreshes a golden.
A refresh removes obsolete numbered binary files after a read changes from
success to failure. It refuses to remove an unrelated artifact.

## Append storage: confirmed

The recorded command is:

```sh
hdp dumpsds -h fixtures/F05.hdf
```

Its complete output is [F05-hdp.txt](F05-hdp.txt). It reports dataset `records`,
index 0, type int16, NDG reference 2, no compression, rank 2, unlimited dimension
currently 5, and second dimension 5. `hdp` does not label linked storage in this
header view. The independent descriptor test finds SD tag `0x42be`, reference 3,
special code 1 and logical length 50 bytes. Thus these five appends do create a
linked element. Its header declares block length 640 bytes, 128 block references
per link table and first link reference 1. This resolves the append question in
specification section 2 for this library and this fixture, not for every writer.

## F21: the C behavior and the deliberate truncation

An SD_NOFILL write of two int16 values into a declared 3 by 5 dataset does not
produce a short element in HDF4 4.2.15. The library allocates 30 bytes. The measured
full read succeeds and returns `0x1234`, `0x5678` and thirteen zeros. That alone
cannot exercise the short-element rule in specification section 2.1.

The generator therefore closes the SD interface and calls the HDF4 C API
`Htrunc` to reduce the stored SD element from 30 bytes to 4 bytes. This is an
explicit fixture adjustment, not normal SD_NOFILL behavior. `SDreaddata` on the
result fails; the golden records `read: fail` with 30 requested bytes and no
value file. The reader expectation remains `malformed`, as specified for an
element shorter than its array.

References: HDF4 `hfile.c` lines 1777 to 1819 for `Htrunc`; `putget.c` lines 843
to 886 for missing-storage reads; DS section 7.8.2 for unwritten SD elements.
The source revision used for format reading is recorded in [the corpus
manifest](../corpus/manifest.json). The installed reference library is the
version pinned in `conda-lock.txt`, rather than that source revision.

## Tool versions

The micromamba binary is 2.10.0-0, released 2026-10-26. The locked conda-forge
builds are HDF4 4.2.15 `h2a13503_7` (uploaded 2023-09-25), pyhdf 0.11.7
`np2py312hfc9d514_2` (uploaded 2026-09-14) and HDF-EOS2 2.20
`heef44b4_1005` (uploaded 2026-08-28). Package upload dates are not upstream
release dates. All these builds predate 2026-10-29. Sources:
[micromamba release](https://github.com/mamba-org/micromamba-releases/releases/tag/2.10.0-0),
[HDF4 package](https://api.anaconda.org/package/conda-forge/hdf4),
[pyhdf package](https://api.anaconda.org/package/conda-forge/pyhdf),
[HDF-EOS2 package](https://api.anaconda.org/package/conda-forge/hdfeos2).

Upstream releases are HDF4 4.2.15 on 2020-02-27, pyhdf 0.11.7 on 2026-07-11,
Python 3.12.16 on 2026-10-25 and NumPy 2.5.4 on 2026-10-10. HDF-EOS2's NASA
source archive documents version 2.20 on 2017-10-05; its exact release date has
not been confirmed. All named tool versions have dated evidence older than
14 days at fixture generation. Transitive package release dates were not audited.
Sources: [HDF4 announcement](https://www.hdfgroup.org/2020/02/27/release-of-hdf-4-2-1-5-newsletter-171/),
[pyhdf release](https://github.com/fhs/pyhdf/releases/tag/v0.11.7),
[Python release](https://www.python.org/downloads/release/python-31216/),
[NumPy releases](https://numpy.org/news/) and
[NASA source archive](https://git.earthdata.nasa.gov/rest/git-lfs/storage/DAS/hdfeos/cb0f900d2732ab01e51284d6c9e90d0e852d61bba9bce3b43af0430ab5414903).
