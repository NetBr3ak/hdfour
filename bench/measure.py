"""Measure only on an idle Linux machine, after every implementation passes its oracle."""

import argparse
import datetime
import hashlib
import importlib.metadata
import json
import os
import platform
import shlex
import statistics
import subprocess
import sys
import tempfile
from pathlib import Path


def command(implementation: str, path: Path, threads: str) -> list[str]:
    return [
        sys.executable,
        "-I",
        "bench/read.py",
        "--impl",
        implementation,
        "--file",
        str(path),
        "--threads",
        threads,
    ]


def measure(arguments: list[str], temporary: Path) -> dict:
    speed = temporary / "speed.json"
    subprocess.run(
        [
            "hyperfine",
            "--warmup",
            "3",
            "--runs",
            "20",
            "--export-json",
            str(speed),
            shlex.join(arguments),
        ],
        check=True,
    )
    rss = []
    for _ in range(5):
        output = temporary / "rss.txt"
        subprocess.run(
            ["/usr/bin/time", "-f", "%M", "-o", str(output), *arguments], check=True
        )
        rss.append(int(output.read_text().strip()))
    return {
        "hyperfine": json.loads(speed.read_text())["results"][0],
        "peak_rss_kib": rss,
        "median_peak_rss_kib": statistics.median(rss),
    }


def hardware() -> dict:
    for line in Path("/proc/cpuinfo").read_text().splitlines():
        key, separator, value = line.partition(":")
        if separator and key.strip() == "model name" and value.strip():
            return {
                "cpu_model": value.strip(),
                "physical_memory_bytes": os.sysconf("SC_PHYS_PAGES")
                * os.sysconf("SC_PAGE_SIZE"),
            }
    raise RuntimeError("CPU model is missing; record the machine before measuring")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--idle-machine", action="store_true", required=True)
    parser.add_argument("--build", type=Path, required=True)
    parser.add_argument("--r01", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if platform.system() != "Linux":
        parser.error("this RSS and affinity harness requires Linux")
    if args.output.exists():
        parser.error("output already exists; choose a new result path")
    versions = {
        name: importlib.metadata.version(name) for name in ("hdfour", "pyhdf", "GDAL")
    }
    if versions["pyhdf"] != "0.11.7":
        parser.error("the comparison requires pyhdf 0.11.7")
    machine = hardware()
    files = [
        (args.build / "chunked.hdf", args.build / "chunked"),
        (args.build / "contiguous.hdf", args.build / "contiguous"),
        (args.r01, args.build / "R01"),
    ]
    variants = [("hdfour", "one"), ("hdfour", "all"), ("pyhdf", "all"), ("gdal", "all")]
    for path, golden in files:
        for implementation, threads in variants:
            subprocess.run(
                [*command(implementation, path, threads), "--check", str(golden)],
                check=True,
            )
    results = []
    with tempfile.TemporaryDirectory(prefix="hdfour-measure-") as directory:
        for path, _ in files:
            for implementation, threads in variants:
                results.append(
                    {
                        "file": path.name,
                        "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
                        "implementation": implementation,
                        "threads": threads,
                        **measure(
                            command(implementation, path, threads), Path(directory)
                        ),
                    }
                )
    document = {
        "date": datetime.date.today().isoformat(),
        "platform": platform.platform(),
        "architecture": platform.machine(),
        "hardware": machine,
        "cpu_affinity": sorted(os.sched_getaffinity(0)),
        "python": platform.python_version(),
        "versions": versions,
        "hyperfine": subprocess.check_output(
            ["hyperfine", "--version"], text=True
        ).strip(),
        "method": {"warmups": 3, "speed_runs": 20, "rss_runs": 5, "idle_machine": True},
        "results": results,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(document, indent=2) + "\n")


if __name__ == "__main__":
    main()
