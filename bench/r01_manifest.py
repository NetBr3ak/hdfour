"""Add SD metadata beside R01 values dumped by the existing C oracle."""

import argparse
import json
from pathlib import Path

from pyhdf.SD import SD, SDC


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("file", type=Path)
    parser.add_argument("golden", type=Path)
    args = parser.parse_args()
    file = SD(str(args.file), SDC.READ)
    entries = []
    codes = {
        3: "|u1",
        4: "|i1",
        5: "<f4",
        6: "<f8",
        20: "|i1",
        21: "|u1",
        22: "<i2",
        23: "<u2",
        24: "<i4",
        25: "<u4",
    }
    try:
        for index in range(file.info()[0]):
            dataset = file.select(index)
            try:
                name, _, shape, dtype, _ = dataset.info()
                entries.append(
                    {"name": name, "shape": list(shape), "dtype": codes[dtype & 0xFFF]}
                )
            finally:
                dataset.endaccess()
    finally:
        file.end()
    (args.golden / "manifest.json").write_text(json.dumps(entries, indent=2) + "\n")


if __name__ == "__main__":
    main()
