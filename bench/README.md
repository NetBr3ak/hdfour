# Benchmark harness

Run builds and tests on the Linux runner. The default command is a small
correctness smoke check. It records no timings and needs no Python binding.

```sh
sh bench/run.sh
```

It uses the reference environment created by `fixtures/build.sh`. Override its
location with `HDFOUR_REFERENCE_PREFIX`. The smoke check generates four 48 by 48
int16 datasets in 16 by 16 DEFLATE-6 chunks and a contiguous 64 by 64 float32
dataset. It checks the core and pyhdf against bytes read back by `SDreaddata`.
Generated files, golden bytes and executables stay in `.build/`.

## Measurement

The full generator writes four 4800 by 4800 int16 datasets in 480 by 480
DEFLATE-6 chunks, plus a separate contiguous 8192 by 8192 float32 file.
The arrays contain coordinate-dependent values. Their raw payload totals about
452 MB; allow space for both HDF files and golden bytes.

Use idle Linux or macOS with an installed hdfour wheel, pyhdf 0.11.7,
conda-forge GDAL with `libgdal-hdf4`, and hyperfine. Linux uses GNU time;
macOS uses its system time command. Use
`HDFOUR_BENCH_PYTHON` to select the Python environment containing all three
readers. Fetch R01 with `python3 -I corpus/fetch.py R01`, or supply its location
through `HDFOUR_BENCH_R01`.

```sh
export HDFOUR_BENCH_REVISION="$(git rev-parse HEAD)"
export HDFOUR_BENCH_BUILD_MANIFEST=bench/.build/parallel-wheel/build.json
sh bench/run.sh --measure --idle-machine
```

Build the wheel and its manifest with the procedure below first. macOS also
requires `HDFOUR_BENCH_SERIAL_PYTHON` and `HDFOUR_BENCH_SERIAL_MANIFEST`.

`--idle-machine` asserts that the machine is idle. Do not use it on a shared,
busy runner. The harness does not acquire exclusive use itself. This command
requires the B7 Python binding; the smoke check does not substitute its Rust
process for a Python timing. Every reader must first match dataset count,
shape, type and C golden bytes. Names are also checked where the API exposes
them. R01 bytes come from `fixtures/c/sdsdump.c`.

Speed uses `python -I bench/read.py --impl X --file F`, three warm-ups and
20 hyperfine runs. Memory records five peak RSS samples and their median.
GNU time reports KiB; macOS time reports bytes, which the script divides by 1024.
Each read discards its result before the next dataset. NumPy copies and golden
comparisons happen only during verification, outside the timed commands.
On Linux, the hdfour variants use one allowed CPU or all allowed CPUs. Linux
process affinity controls `available_parallelism`. A separate serial build can
also be selected. macOS requires two installations: a build without the core's
`parallel` feature for one thread, and the regular parallel build for all threads.
The core has no thread-count setting. The parent verifies both installed
extension hashes, revisions and features before checking the files. It checks
the selected extension against that verified build immediately before and after
each hyperfine invocation and each peak RSS command. These checks run outside
the timed commands; `read.py` computes no extension hash. A changed identity
aborts the measurement without writing a result.
External OpenMP and OpenBLAS pools use one thread in every variant.

Results go to `results/<date>-<system>-<architecture>.json`. Override the path with
`HDFOUR_BENCH_RESULT`. Existing results are not overwritten. The JSON includes
input hashes, extension hashes, build revisions, core features, tool versions,
platform, CPU model, physical memory, CPU affinity where available, method,
individual speed and RSS samples. It records the HDF4 version from pyhdf and
HDF4/zlib version fields that GDAL exposes. A missing library version is marked
as not exposed, rather than inferred from Python's own zlib. It contains no
hostname. Do not publish a speed or memory claim
without its machine and method. No measurement result is bundled here.

## Build identity

Export the committed source on the Git machine:

```sh
mkdir -p bench/.build
python3 -I bench/build.py --export-source bench/.build/source.tar \
    --revision "$(git rev-parse HEAD)"
```

The exporter uses `git archive HEAD`, so uncommitted edits never enter the
archive, even when Git status hides them. Transfer the archive to the build
machine; Git is not needed there. The builder checks its recorded revision
against `--revision` and extracts it into a temporary directory. It never assigns
a revision to an arbitrary pre-existing wheel.

```sh
python3 -I bench/build.py --source-archive bench/.build/source.tar \
    --revision "$HDFOUR_BENCH_REVISION" --threads all \
    --output bench/.build/parallel-wheel
python3 -I bench/build.py --source-archive bench/.build/source.tar \
    --revision "$HDFOUR_BENCH_REVISION" --threads one \
    --output bench/.build/serial-wheel
```

The serial recipe removes the binding's request for the core `parallel` feature
in the temporary source copy only. Cargo's resolved core features are checked
before compilation. One sidecar, `build.json`, records the revision, resolved
core features and SHA-256 hash of the built extension. The tracked binding remains
unchanged.
Build output directories must not already exist.

Install the parallel wheel in the environment selected by `HDFOUR_BENCH_PYTHON`.
Install the serial wheel in a second environment with NumPy. Set
`HDFOUR_BENCH_SERIAL_PYTHON` to that environment's absolute Python path and
`HDFOUR_BENCH_SERIAL_MANIFEST` to `bench/.build/serial-wheel/build.json`.
The serial environment does not need the rival readers. Preserve both manifests
with the measurement result. They are build records, not signed attestations;
the source exporter and compiler remain trusted.

The equivalent direct measurement command is:

```sh
"$HDFOUR_BENCH_PYTHON" -I bench/measure.py --idle-machine \
    --build bench/.build --r01 corpus/cache/R01.hdf --output bench/results/new.json \
    --revision "$HDFOUR_BENCH_REVISION" \
    --build-manifest "$HDFOUR_BENCH_BUILD_MANIFEST" \
    --serial-python "$HDFOUR_BENCH_SERIAL_PYTHON" \
    --serial-manifest "$HDFOUR_BENCH_SERIAL_MANIFEST"
```

Run the Apple measurement at night with other applications closed. Linux mock
tests cover macOS command construction and unit conversion, not Apple timing
results.

## Assembly inspection

```sh
sh bench/simd.sh
# Cross-inspect only if this Rust target is already installed:
sh bench/simd.sh --target aarch64-unknown-linux-gnu
```

This accepts native x86-64 and ARM64, or an explicit installed target. It emits
fresh release assembly in a temporary target directory, prints only
`nt::swap_in_place`, then removes that directory. The symbol matcher handles ELF
and Mach-O function labels. It never installs a missing target. Inspect that
function before drawing a SIMD conclusion. An instruction in another function
is not evidence for the bulk byte swap.

`fixtures/swap-macho.s` contains one function and the next function's opening
instructions from native ARM64 Mach-O release output. It was generated with
Rust 1.98.0 using `cargo rustc -p hdfour --release -- --emit asm`. The output has
no `Lfunc_end` label. `test_simd.py` checks its `.cfi_endproc` boundary and the
next function label when that directive is removed.
