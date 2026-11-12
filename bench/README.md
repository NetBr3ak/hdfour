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

Use an idle Linux machine with an installed hdfour wheel, pyhdf 0.11.7,
conda-forge GDAL with `libgdal-hdf4`, hyperfine and GNU time. Use
`HDFOUR_BENCH_PYTHON` to select the Python environment containing all three
readers. Fetch R01 with `python3 -I corpus/fetch.py R01`, or supply its location
through `HDFOUR_BENCH_R01`.

```sh
sh bench/run.sh --measure --idle-machine
```

`--idle-machine` asserts that the machine is idle. Do not use it on a shared,
busy runner. The harness does not acquire exclusive use itself. This command
requires the B7 Python binding; the smoke check does not substitute its Rust
process for a Python timing. Every reader must first match dataset count,
shape, type and C golden bytes. Names are also checked where the API exposes
them. R01 bytes come from `fixtures/c/sdsdump.c`.

Speed uses `python -I bench/read.py --impl X --file F`, three warm-ups and
20 hyperfine runs. Memory uses GNU time's peak RSS, five runs and their median.
Each read discards its result before the next dataset. NumPy copies and golden
comparisons happen only during verification, outside the timed commands.
The hdfour variants use one allowed CPU or all allowed CPUs. Linux process
affinity controls `available_parallelism`; the core has no thread-count setting.
External OpenMP and OpenBLAS pools use one thread in every variant.

Results go to `results/<date>-linux-<architecture>.json`. Override the path with
`HDFOUR_BENCH_RESULT`. Existing results are not overwritten. The JSON includes
input hashes, tool versions, platform, CPU model, physical memory, CPU affinity,
method, individual speed
and RSS samples. It contains no hostname. Do not publish a speed or memory claim
without its machine and method. No measurement result is bundled here.

## Assembly inspection

```sh
sh bench/simd.sh
```

This emits release assembly and prints `nt::swap_in_place`. Inspect that function
before drawing a SIMD conclusion. It does not test ARM64, and an instruction in
another function is not evidence for the bulk byte swap.
