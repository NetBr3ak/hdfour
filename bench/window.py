"""Compare warm rectangular reads with pyhdf's SDreaddata-backed reads.

Example: python bench/window.py --file bench/.build/chunked.hdf \
    --dataset chunked_0 --start 100 100 --count 256 256 --measure
Use sh bench/run.sh for a small smoke fixture, or generate full fixtures with
bench/gen.c as described in bench/README.md.
"""

import argparse
import importlib.metadata
import json
import statistics
import time
from pathlib import Path


def selection(shape, start, count):
    if len(start) != len(shape) or len(count) != len(shape):
        raise ValueError("start and count must have one value per dimension")
    if any(
        s < 0 or n <= 0 or s + n > size
        for size, s, n in zip(shape, start, count, strict=True)
    ):
        raise ValueError("the window must be non-empty and inside the dataset")
    return tuple(slice(s, s + n) for s, n in zip(start, count, strict=True))


def compare(actual, expected, label):
    if actual.shape != expected.shape or actual.dtype != expected.dtype:
        raise ValueError(f"{label}: shape or dtype differs from pyhdf")
    if actual.tobytes() != expected.tobytes():
        raise ValueError(f"{label}: values differ from pyhdf")


def measure(read, warmup, repeat):
    for _ in range(warmup):
        read()
    samples = []
    for _ in range(repeat):
        start = time.perf_counter_ns()
        result = read()
        elapsed = time.perf_counter_ns() - start
        del result
        samples.append(elapsed / 1_000_000)
    return {"median_ms": statistics.median(samples), "samples_ms": samples}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--file", type=Path, required=True)
    parser.add_argument("--dataset", required=True)
    parser.add_argument("--start", type=int, nargs="+", required=True)
    parser.add_argument("--count", type=int, nargs="+", required=True)
    parser.add_argument("--warmup", type=int, default=2)
    parser.add_argument("--repeat", type=int, default=7)
    parser.add_argument(
        "--baseline-only", action="store_true", help="allow a wheel without read_window"
    )
    parser.add_argument(
        "--measure", action="store_true", help="record timings after checking data"
    )
    args = parser.parse_args()
    if args.warmup < 1 or args.repeat < 1:
        parser.error("warmup and repeat must be positive")

    import hdfour
    from pyhdf.SD import SD, SDC

    file = hdfour.open(args.file)
    dataset = file[args.dataset]
    slices = selection(dataset.shape, args.start, args.count)
    reference_file = SD(str(args.file), SDC.READ)
    try:
        reference = reference_file.select(args.dataset)
        try:
            readers = {
                "hdfour_full_slice": lambda: dataset.read()[slices],
                "pyhdf_window": lambda: reference.get(
                    start=args.start, count=args.count
                ),
            }
            if not args.baseline_only:
                readers["hdfour_window"] = lambda: dataset.read_window(
                    args.start, args.count
                )
            expected = readers["pyhdf_window"]()
            for name, read in readers.items():
                compare(read(), expected, name)
            result = {
                "file": str(args.file),
                "file_bytes": args.file.stat().st_size,
                "dataset": args.dataset,
                "shape": dataset.shape,
                "dtype": dataset.dtype,
                "storage": dataset.storage,
                "start": args.start,
                "count": args.count,
                "hdfour_version": importlib.metadata.version("hdfour"),
                "pyhdf_version": importlib.metadata.version("pyhdf"),
                "correctness": "all results match pyhdf window",
            }
            if args.measure:
                result["timing"] = {
                    "unit": "milliseconds",
                    "cache": "warm",
                    "warmup_per_case": args.warmup,
                    "repetitions_per_case": args.repeat,
                    "cases": {
                        name: measure(read, args.warmup, args.repeat)
                        for name, read in readers.items()
                    },
                }
            print(json.dumps(result, indent=2))
        finally:
            reference.endaccess()
    finally:
        reference_file.end()


if __name__ == "__main__":
    main()
