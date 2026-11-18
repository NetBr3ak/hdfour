#!/usr/bin/env python3
"""Check selected SDreaddata bytes and preserve the read-window reference."""

from __future__ import annotations

import argparse
import ctypes
import hashlib
import itertools
import json
import math
import os
from pathlib import Path
import sys

FIXTURES = Path(__file__).resolve().parent
CASES = (
    "F01",
    "F02",
    "F04",
    "F05",
    "F06",
    "F07",
    "F08",
    "F09",
    "F13",
    "F16",
    "F19",
    "F20",
)
WIDTHS = {3: 1, 4: 1, 5: 4, 6: 8, 20: 1, 21: 1, 22: 2, 23: 2, 24: 4, 25: 4}


class Hdf4:
    def __init__(self) -> None:
        library_directory = Path(sys.prefix) / "lib"
        # The locked libmfhdf leaves its base HDF symbols for the caller to resolve.
        self.base_library = ctypes.CDLL(
            str(library_directory / "libdf.so"), mode=ctypes.RTLD_GLOBAL
        )
        self.library = ctypes.CDLL(str(library_directory / "libmfhdf.so"))
        coordinate = ctypes.POINTER(ctypes.c_int32)
        self.library.SDstart.argtypes = [ctypes.c_char_p, ctypes.c_int32]
        self.library.SDstart.restype = ctypes.c_int32
        self.library.SDselect.argtypes = [ctypes.c_int32, ctypes.c_int32]
        self.library.SDselect.restype = ctypes.c_int32
        self.library.SDreaddata.argtypes = [
            ctypes.c_int32,
            coordinate,
            coordinate,
            coordinate,
            ctypes.c_void_p,
        ]
        self.library.SDreaddata.restype = ctypes.c_int
        self.library.SDendaccess.argtypes = [ctypes.c_int32]
        self.library.SDendaccess.restype = ctypes.c_int
        self.library.SDend.argtypes = [ctypes.c_int32]
        self.library.SDend.restype = ctypes.c_int

    @staticmethod
    def check(status: int, operation: str) -> int:
        if status < 0:
            raise RuntimeError(f"HDF4 C {operation} failed")
        return status

    def read(
        self, dataset: int, start: list[int], count: list[int], width: int
    ) -> bytes:
        coordinates = ctypes.c_int32 * len(start)
        length = math.prod(count) * width
        output = ctypes.create_string_buffer(length)
        self.check(
            self.library.SDreaddata(
                dataset,
                coordinates(*start),
                coordinates(*([1] * len(start))),
                coordinates(*count),
                output,
            ),
            "SDreaddata",
        )
        native = output.raw
        if sys.byteorder == "little":
            return native
        return b"".join(
            native[index : index + width][::-1] for index in range(0, length, width)
        )


def selections(shape: list[int]) -> list[tuple[list[int], list[int]]]:
    if 0 in shape:
        return [([0] * len(shape), shape)]
    interior = [min(1, size - 1) for size in shape]
    return [
        ([0] * len(shape), [min(2, size) for size in shape]),
        (interior, [max(1, size - 2) for size in shape]),
        ([size - 1 for size in shape], [1] * len(shape)),
        ([0] * len(shape), [1, *shape[1:]]),
        ([shape[0], *([0] * (len(shape) - 1))], [0, *shape[1:]]),
    ]


def select_bytes(
    raw: bytes, shape: list[int], start: list[int], count: list[int], width: int
) -> bytes:
    expected_length = math.prod(shape) * width
    if len(raw) != expected_length:
        raise ValueError(
            f"whole-array C golden length: {len(raw)} != {expected_length}"
        )
    selected = bytearray()
    for coordinate in itertools.product(*(range(size) for size in count)):
        linear = 0
        for size, first, relative in zip(shape, start, coordinate, strict=True):
            linear = linear * size + first + relative
        selected.extend(raw[linear * width : (linear + 1) * width])
    return bytes(selected)


def reference(fixtures: Path) -> dict:
    hdf = Hdf4()
    sources = {}
    windows = []
    for name in CASES:
        metadata = json.loads((fixtures / "golden" / f"{name}.json").read_text())
        path = fixtures / metadata["file"]
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        if digest != metadata["sha256"]:
            raise ValueError(f"fixture hash differs from C golden: {name}")
        sources[metadata["file"]] = digest
        handle = hdf.check(hdf.library.SDstart(os.fsencode(path), 1), "SDstart")
        try:
            for info in metadata["datasets"]:
                if info["expect"] != "values":
                    continue
                index = info["index"]
                shape = info["shape"]
                raw = (fixtures / "golden" / name / f"{index}.bin").read_bytes()
                dataset = hdf.check(hdf.library.SDselect(handle, index), "SDselect")
                try:
                    for start, count in selections(shape):
                        expected = select_bytes(
                            raw, shape, start, count, WIDTHS[info["dtype"]]
                        )
                        if 0 in count:
                            actual = expected
                            method = "empty window extension: no SDreaddata selection"
                        else:
                            actual = hdf.read(
                                dataset, start, count, WIDTHS[info["dtype"]]
                            )
                            method = "direct HDF4 C SDreaddata selection"
                        if actual != expected:
                            raise ValueError(
                                f"C selection differs from whole-array C golden: {name}/{index} {start=} {count=}"
                            )
                        windows.append(
                            {
                                "file": metadata["file"],
                                "dataset": index,
                                "dtype": info["dtype"],
                                "start": start,
                                "count": count,
                                "reference": method,
                                "hex": actual.hex(),
                            }
                        )
                finally:
                    hdf.check(hdf.library.SDendaccess(dataset), "SDendaccess")
        finally:
            hdf.check(hdf.library.SDend(handle), "SDend")
    return {
        "reference": "Direct HDF4 4.2.15 libmfhdf.so from fixtures/conda-lock.txt; little-endian C-order bytes",
        "sources": sources,
        "windows": windows,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fixtures", type=Path, default=FIXTURES)
    parser.add_argument("--output", type=Path)
    parser.add_argument(
        "--write",
        action="store_true",
        help="write instead of checking the immutable reference",
    )
    args = parser.parse_args()
    output = args.output or args.fixtures / "window-reference.json"
    result = reference(args.fixtures)
    header = json.dumps(
        {"reference": result["reference"], "sources": result["sources"]}, indent=2
    ).removesuffix("\n}")
    records = ",\n".join("    " + json.dumps(window) for window in result["windows"])
    rendered = f'{header},\n  "windows": [\n{records}\n  ]\n}}\n'
    if args.write:
        output.write_text(rendered)
    elif output.read_text() != rendered:
        raise ValueError(f"window reference differs: {output}")
    counts = {}
    for window in result["windows"]:
        method = window["reference"]
        counts[method] = counts.get(method, 0) + 1
    print(
        json.dumps(
            {"windows": len(result["windows"]), "methods": counts}, sort_keys=True
        )
    )


if __name__ == "__main__":
    main()
