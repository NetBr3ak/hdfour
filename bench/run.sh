#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
mode=smoke
size=small
idle=no
for argument in "$@"; do
    case "$argument" in
        --measure) mode=measure; size=full ;;
        --small) size=small ;;
        --idle-machine) idle=yes ;;
        *) echo 'Usage: sh bench/run.sh [--measure --idle-machine] [--small]' >&2; exit 2 ;;
    esac
done
if [ "$mode" = measure ] && [ "$idle" != yes ]; then
    echo 'Measurements require --idle-machine. A shared runner cannot supply a measurement.' >&2
    exit 2
fi
if [ "$mode" = measure ]; then
    : "${HDFOUR_BENCH_REVISION:?Set the full intended git revision}"
    : "${HDFOUR_BENCH_BUILD_MANIFEST:?Set the parallel wheel build.json path}"
fi
prefix=${HDFOUR_REFERENCE_PREFIX:-"$HOME/.local/share/hdfour-fixtures/env"}
python=${HDFOUR_BENCH_PYTHON:-"$prefix/bin/python"}
build=bench/.build
export LD_LIBRARY_PATH="$prefix/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export OPENBLAS_NUM_THREADS=1 OMP_NUM_THREADS=1
mkdir -p "$build/chunked" "$build/contiguous"
gcc -std=c11 -O2 -Wall -Wextra -Werror -I"$prefix/include" bench/gen.c \
    -L"$prefix/lib" -Wl,-rpath,"$prefix/lib" -lmfhdf -ldf -lz -ljpeg -o "$build/gen"
"$build/gen" "$build" "$size"
"$python" -I bench/test_bench.py
"$python" -I bench/test_measure.py
"$python" -I bench/test_build.py
cargo test --release --manifest-path bench/Cargo.toml --test generated -- --nocapture
for kind in chunked contiguous; do
    "$python" -I bench/read.py --impl pyhdf --file "$build/$kind.hdf" --check "$build/$kind"
done
if [ "$mode" = smoke ]; then
    echo 'PASS benchmark smoke (small files by default; no timings recorded)'
    exit 0
fi
r01=${HDFOUR_BENCH_R01:-corpus/cache/R01.hdf}
test -f "$r01"
mkdir -p "$build/R01"
gcc -std=c11 -O2 -Wall -Wextra -Werror -I"$prefix/include" fixtures/c/sdsdump.c \
    -L"$prefix/lib" -Wl,-rpath,"$prefix/lib" -lmfhdf -ldf -lz -ljpeg -o "$build/sdsdump"
"$build/sdsdump" "$r01" "$build/R01"
"$python" -I bench/r01_manifest.py "$r01" "$build/R01"
system=$(uname -s | tr '[:upper:]' '[:lower:]')
result=${HDFOUR_BENCH_RESULT:-"bench/results/$(date +%F)-$system-$(uname -m).json"}
set -- --idle-machine --build "$build" --r01 "$r01" --output "$result" \
    --revision "$HDFOUR_BENCH_REVISION" --build-manifest "$HDFOUR_BENCH_BUILD_MANIFEST"
if [ -n "${HDFOUR_BENCH_SERIAL_PYTHON:-}" ]; then
    : "${HDFOUR_BENCH_SERIAL_MANIFEST:?Set the serial wheel build.json path}"
    set -- "$@" --serial-python "$HDFOUR_BENCH_SERIAL_PYTHON" \
        --serial-manifest "$HDFOUR_BENCH_SERIAL_MANIFEST"
fi
"$python" -I bench/measure.py "$@"
