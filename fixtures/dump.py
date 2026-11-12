#!/usr/bin/env python3
"""Build or check metadata and C-library value goldens by dataset index."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
from typing import TYPE_CHECKING, NotRequired, TypedDict

if TYPE_CHECKING:
    from pyhdf.SD import SD, SDS, SDim

type Json = None | bool | int | float | str | list[Json] | dict[str, Json]


class Attribute(TypedDict):
    name: str
    dtype: int
    count: int
    hex: str


class Dimension(TypedDict):
    name: str
    size: int
    unlimited: bool
    attributes: list[Attribute]


class Dataset(TypedDict):
    index: int
    name: str
    dtype: int
    shape: list[int]
    dimensions: list[Dimension]
    attributes: list[Attribute]
    is_coordinate: bool
    expect: str


class Skipped(TypedDict):
    object: str
    feature: str


class Expectation(TypedDict):
    datasets: NotRequired[dict[str, str]]
    skipped: list[Skipped]
    eos: NotRequired[Json]


class ReadStatus(TypedDict):
    index: int
    read: str
    bytes: int


class Golden(TypedDict):
    file: str
    sha256: str
    attributes: list[Attribute]
    datasets: list[Dataset]
    skipped: list[Skipped]
    eos: Json
    c_reads: NotRequired[list[ReadStatus]]


ROOT = Path(__file__).resolve().parent.parent
FIXTURES = ROOT / "fixtures"
GOLDEN = FIXTURES / "golden"
FORMATS = {
    3: "B",
    4: "b",
    5: "f",
    6: "d",
    20: "b",
    21: "B",
    22: "h",
    23: "H",
    24: "i",
    25: "I",
}


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def attributes(owner: SD | SDS | SDim, count: int) -> list[Attribute]:
    result: list[Attribute] = []
    for index in range(count):
        attribute = owner.attr(index)
        name, dtype, length = attribute.info()
        value = attribute.get()
        dtype &= ~0x4000
        values = value if isinstance(value, (list, tuple)) else [value]
        if dtype == 4:
            raw = value.encode("latin1")
        else:
            raw = struct.pack("<" + FORMATS[dtype] * length, *values)
        require(
            len(raw) == length * struct.calcsize(FORMATS[dtype]),
            f"attribute length: {name}",
        )
        result.append({"name": name, "dtype": dtype, "count": length, "hex": raw.hex()})
    return result


def describe(path: Path, expected: Expectation) -> Golden:
    from pyhdf.SD import SD, SDC

    handle = SD(str(path), SDC.READ)
    try:
        count, attribute_count = handle.info()
        datasets: list[Dataset] = []
        overrides = expected.get("datasets", {})
        for index in range(count):
            dataset = handle.select(index)
            try:
                name, rank, shape, dtype, attr_count = dataset.info()
                if rank == 1:
                    shape = [shape] if isinstance(shape, int) else list(shape)
                else:
                    shape = list(shape)
                dimensions: list[Dimension] = []
                for axis in range(rank):
                    dimension = dataset.dim(axis)
                    dim_name, size, _, dim_attrs = dimension.info()
                    dimensions.append(
                        {
                            "name": dim_name,
                            "size": shape[axis],
                            "unlimited": size == 0,
                            "attributes": attributes(dimension, dim_attrs),
                        }
                    )
                datasets.append(
                    {
                        "index": index,
                        "name": name,
                        "dtype": dtype & ~0x4000,
                        "shape": shape,
                        "dimensions": dimensions,
                        "attributes": attributes(dataset, attr_count),
                        "is_coordinate": bool(dataset.iscoordvar()),
                        "expect": overrides.get(str(index), "values"),
                    }
                )
            finally:
                dataset.endaccess()
        require(
            set(overrides) <= {str(i) for i in range(count)},
            f"unknown dataset index: {path.name}",
        )
        return {
            "file": path.name,
            "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
            "attributes": attributes(handle, attribute_count),
            "datasets": datasets,
            "skipped": expected["skipped"],
            "eos": expected.get("eos"),
        }
    finally:
        handle.end()


def encoded(value: Golden) -> bytes:
    return (json.dumps(value, indent=2, ensure_ascii=True) + "\n").encode()


def compare_or_write(path: Path, content: bytes, check: bool) -> None:
    if not check:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(content)
        return
    require(path.is_file(), f"missing golden: {path.relative_to(ROOT)}")
    require(path.read_bytes() == content, f"stale golden: {path.relative_to(ROOT)}")


def check_values(source: Path, metadata: Golden, target: Path) -> list[ReadStatus]:
    executable = FIXTURES / ".build" / "sdsdump"
    subprocess.run(
        [str(executable), str(source), str(target)], check=True, cwd=source.parent
    )
    status: list[ReadStatus] = json.loads((target / "status.json").read_text())
    require(len(status) == len(metadata["datasets"]), f"C dataset count: {source.name}")
    for dataset, read_status in zip(metadata["datasets"], status, strict=True):
        require(read_status["index"] == dataset["index"], "C index order")
        if dataset["expect"] == "values":
            require(
                read_status["read"] == "ok",
                f"C read failed: {source.name}/{dataset['index']}",
            )
            size = struct.calcsize(FORMATS[dataset["dtype"]])
            for length in dataset["shape"]:
                size *= length
            require(
                (target / f"{dataset['index']}.bin").stat().st_size == size,
                "C value byte count",
            )
    return status


def run(check: bool, corpus: bool) -> None:
    expectations: dict[str, Expectation] = json.loads(
        (FIXTURES / "expect.json").read_text()
    )
    fixture_paths = sorted(FIXTURES.glob("F[0-9][0-9].hdf"))
    require(
        [p.stem for p in fixture_paths] == [f"F{i:02d}" for i in range(1, 22)],
        "test_fixture_inventory: F01-F21 required",
    )
    paths = fixture_paths + sorted((ROOT / "corpus").glob("R0[23].hdf"))
    require(len(paths) == 23, "test_corpus_inventory: committed R02 and R03 required")
    if check:
        names = {
            name for source in paths for name in (source.stem, f"{source.stem}.json")
        }
        require(
            {path.name for path in GOLDEN.iterdir()} == names,
            "stale golden file inventory",
        )
    if corpus:
        for name in ("R01", "R04a", "R04b", "R04c"):
            source = ROOT / "corpus" / "cache" / f"{name}.hdf"
            require(source.is_file(), f"fetch {name} with corpus/fetch.py first")
            paths.append(source)
    with tempfile.TemporaryDirectory(prefix="hdfour-golden-") as directory:
        for source in paths:
            require(source.stem in expectations, f"missing expectation: {source.stem}")
            if source.stem.startswith("F"):
                require(
                    source.stat().st_size < 65536, f"test_fixture_size: {source.name}"
                )
            metadata = describe(source, expectations[source.stem])
            target = Path(directory) / source.stem
            target.mkdir()
            metadata["c_reads"] = check_values(source, metadata, target)
            output = (
                source.parent / "golden" if source.parent.name == "cache" else GOLDEN
            )
            compare_or_write(output / f"{source.stem}.json", encoded(metadata), check)
            destination = output / source.stem
            names = {p.name for p in target.iterdir()}
            if check:
                require(
                    destination.is_dir(), f"missing golden directory: {source.stem}"
                )
                require(
                    {p.name for p in destination.iterdir()} == names,
                    f"stale golden inventory: {source.stem}",
                )
            elif destination.exists():
                for stale in destination.iterdir():
                    if stale.name not in names:
                        require(
                            stale.suffix == ".bin" and stale.stem.isdecimal(),
                            f"unexpected golden artifact: {stale}",
                        )
                        stale.unlink()
            for path in target.iterdir():
                compare_or_write(destination / path.name, path.read_bytes(), check)
    print("PASS test_fixture_inventory test_corpus_inventory test_fixture_size")
    print(
        "PASS test_metadata_by_index test_c_reference_bytes test_expectations test_stale_goldens"
    )
    print(f"PASS {'checked' if check else 'refreshed'} {len(paths)} files")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--check", action="store_true", help="fail instead of refreshing a stale golden"
    )
    parser.add_argument(
        "--corpus",
        action="store_true",
        help="also check cached R01 and R04; keep their goldens in the cache",
    )
    args = parser.parse_args()
    python = Path.home() / ".local/share/hdfour-fixtures/env/bin/python"
    if not python.exists():
        raise SystemExit(
            "Run sh fixtures/build.sh to create the reference environment."
        )
    if python.resolve() != Path(sys.executable).resolve():
        os.execv(
            str(python),
            [str(python), "-I", str(Path(__file__).resolve()), *sys.argv[1:]],
        )
    try:
        run(args.check, args.corpus)
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        raise SystemExit(str(error)) from error


if __name__ == "__main__":
    main()
