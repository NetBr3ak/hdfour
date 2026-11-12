"""Read all datasets through one implementation; optionally check C golden bytes."""

import argparse
import json
import os
import platform
from pathlib import Path

import numpy as np


def check_array(values: np.ndarray, golden: Path) -> None:
    little = values.astype(values.dtype.newbyteorder("<"), copy=False)
    if little.tobytes(order="C") != golden.read_bytes():
        raise ValueError(f"values differ from SDreaddata: {golden.name}")


def check_dataset(name, values: np.ndarray, golden: Path, index: int) -> None:
    entries = json.loads((golden / "manifest.json").read_text())
    expected = entries[index]
    if name is not None and name != expected["name"]:
        raise ValueError(f"dataset {index}: name differs")
    if list(values.shape) != expected["shape"]:
        raise ValueError(f"dataset {index}: shape differs")
    if values.dtype.newbyteorder("<") != np.dtype(expected["dtype"]):
        raise ValueError(f"dataset {index}: dtype differs")
    check_array(values, golden / f"{index}.bin")


def read_arrays(implementation: str, path: Path):
    if implementation == "hdfour":
        import hdfour

        file = hdfour.open(str(path))
        for dataset in file.datasets:
            yield dataset.name, dataset.read()
        return
    if implementation == "pyhdf":
        from pyhdf.SD import SD, SDC

        file = SD(str(path), SDC.READ)
        try:
            for index in range(file.info()[0]):
                dataset = file.select(index)
                try:
                    yield dataset.info()[0], dataset[:]
                finally:
                    dataset.endaccess()
        finally:
            file.end()
        return
    from osgeo import gdal

    gdal.UseExceptions()
    with gdal.Open(str(path)) as file:
        subdatasets = file.GetSubDatasets()
        if not subdatasets:
            yield None, file.ReadAsArray()
            return
        for identifier, _ in subdatasets:
            with gdal.Open(identifier) as dataset:
                yield None, dataset.ReadAsArray()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--impl", choices=("hdfour", "pyhdf", "gdal"), required=True)
    parser.add_argument("--file", type=Path, required=True)
    parser.add_argument("--check", type=Path, metavar="GOLDEN_DIRECTORY")
    parser.add_argument("--threads", choices=("one", "all"), default="all")
    args = parser.parse_args()
    if args.threads == "one" and platform.system() == "Linux":
        # available_parallelism observes the calling process's Linux CPU affinity.
        cpus = os.sched_getaffinity(0)
        os.sched_setaffinity(0, {min(cpus)})
    # On macOS, measure.py verifies and selects a separate non-parallel build.
    count = 0
    for name, values in read_arrays(args.impl, args.file):
        if args.check:
            check_dataset(name, values, args.check, count)
        count += 1
        del values
    if count == 0:
        raise ValueError("no datasets were read")
    if args.check:
        expected = json.loads((args.check / "manifest.json").read_text())
        if count != len(expected):
            raise ValueError("dataset count differs from SDreaddata")
        print(f"PASS {args.impl}: {count} datasets match SDreaddata")


if __name__ == "__main__":
    main()
