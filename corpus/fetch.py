#!/usr/bin/env python3
"""Fetch pinned public samples into the ignored cache and verify their hashes."""

import argparse
import hashlib
import json
from pathlib import Path
from typing import TypedDict
from urllib.request import urlopen

ROOT = Path(__file__).resolve().parent


class Sample(TypedDict):
    id: str
    filename: str
    url: str
    sha256: str
    bytes: int
    committed: bool


def fetch(entry: Sample) -> None:
    destination = ROOT / "cache" / f"{entry['id']}.hdf"
    if entry["committed"]:
        destination = ROOT / f"{entry['id']}.hdf"
    if destination.exists():
        content = destination.read_bytes()
    else:
        with urlopen(entry["url"]) as response:
            content = response.read(entry["bytes"] + 1)
    if len(content) != entry["bytes"]:
        raise ValueError(f"wrong length: {entry['id']}")
    if hashlib.sha256(content).hexdigest() != entry["sha256"]:
        raise ValueError(f"wrong SHA-256: {entry['id']}")
    if not destination.exists():
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(content)
    print(f"OK {entry['id']} {entry['sha256']}")


def main() -> None:
    entries: list[Sample] = json.loads((ROOT / "manifest.json").read_text())
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "ids", nargs="*", help="sample IDs; default: all anonymous public samples"
    )
    args = parser.parse_args()
    available = {entry["id"] for entry in entries}
    unknown = set(args.ids) - available
    if unknown:
        parser.error(f"unknown or local-only samples: {', '.join(sorted(unknown))}")
    for entry in entries:
        if not args.ids or entry["id"] in args.ids:
            fetch(entry)


if __name__ == "__main__":
    main()
