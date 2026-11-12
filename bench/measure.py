"""Measure on idle Linux or macOS, after every implementation passes its oracle."""

import argparse
import datetime
import hashlib
import importlib.metadata
import importlib.util
import json
import os
import platform
import re
import shlex
import statistics
import subprocess
import sys
import tempfile
from collections.abc import Callable, Iterator
from contextlib import contextmanager
from functools import partial
from pathlib import Path


def command(
    implementation: str, path: Path, threads: str, python: str = sys.executable
) -> list[str]:
    return [
        python,
        "-I",
        "bench/read.py",
        "--impl",
        implementation,
        "--file",
        str(path),
        "--threads",
        threads,
    ]


def installed_build(manifest: Path, revision: str) -> dict:
    identity = json.loads(manifest.read_text())
    if identity.get("revision") != revision:
        raise ValueError("installed build revision differs from --revision")
    if identity.get("features") not in ([], ["parallel"]):
        raise ValueError("build manifest has missing or unsupported features")
    specification = importlib.util.find_spec("hdfour._hdfour")
    if specification is None or specification.origin is None:
        raise ValueError("installed extension was not found")
    digest = hashlib.sha256(Path(specification.origin).read_bytes()).hexdigest()
    if identity.get("extension_sha256") != digest:
        raise ValueError("installed extension hash differs from the build manifest")
    return identity


def inspect_build(python: Path, manifest: Path, revision: str) -> dict:
    return json.loads(
        subprocess.check_output(
            [
                str(python),
                "-I",
                "bench/measure.py",
                "--inspect-build",
                "--build-manifest",
                str(manifest),
                "--revision",
                revision,
            ],
            text=True,
        )
    )


def check_build(manifest: Path, expected: dict, python: Path | None) -> None:
    current = (
        installed_build(manifest, expected["revision"])
        if python is None
        else inspect_build(python, manifest, expected["revision"])
    )
    if current != expected:
        raise ValueError(
            "installed build hash or features differ from the verified build"
        )


@contextmanager
def checked_build(check_identity: Callable[[], None] | None) -> Iterator[None]:
    if check_identity is not None:
        check_identity()
    yield
    if check_identity is not None:
        check_identity()


def reference_libraries() -> dict:
    from osgeo import gdal
    from pyhdf.HDF import getlibversion

    build = dict(
        line.split("=", 1)
        for line in gdal.VersionInfo("BUILD_INFO").splitlines()
        if "=" in line
    )
    return {
        "pyhdf": {
            "hdf4": list(getlibversion()[:3]),
            "zlib": "not exposed by pyhdf",
        },
        "gdal": {
            "hdf4": build.get("HDF4_VERSION", "not exposed by GDAL"),
            "zlib": {
                key: value for key, value in build.items() if "ZLIB" in key.upper()
            }
            or "not exposed by GDAL",
        },
    }


def measure(
    arguments: list[str],
    temporary: Path,
    check_identity: Callable[[], None] | None = None,
) -> dict:
    speed = temporary / "speed.json"
    with checked_build(check_identity):
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
        if platform.system() == "Darwin":
            with checked_build(check_identity):
                result = subprocess.run(
                    ["/usr/bin/time", "-l", *arguments],
                    check=True,
                    text=True,
                    stderr=subprocess.PIPE,
                )
            match = re.search(
                r"(?m)^\s*(\d+)\s+maximum resident set size\s*$", result.stderr
            )
            if match is None:
                raise ValueError("macOS time did not report peak RSS")
            rss.append(int(match.group(1)) / 1024)
        else:
            output = temporary / "rss.txt"
            with checked_build(check_identity):
                subprocess.run(
                    ["/usr/bin/time", "-f", "%M", "-o", str(output), *arguments],
                    check=True,
                )
            rss.append(int(output.read_text().strip()))
    return {
        "hyperfine": json.loads(speed.read_text())["results"][0],
        "peak_rss_kib": rss,
        "median_peak_rss_kib": statistics.median(rss),
    }


def hardware() -> dict:
    if platform.system() == "Darwin":
        cpu = subprocess.check_output(
            ["sysctl", "-n", "machdep.cpu.brand_string"], text=True
        ).strip()
        memory = subprocess.check_output(
            ["sysctl", "-n", "hw.memsize"], text=True
        ).strip()
        if not cpu or not memory.isdecimal() or int(memory) <= 0:
            raise ValueError(
                "macOS hardware requires a CPU model and positive memory size"
            )
        return {
            "cpu_model": cpu,
            "physical_memory_bytes": int(memory),
        }
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
    parser.add_argument("--idle-machine", action="store_true")
    parser.add_argument("--build", type=Path)
    parser.add_argument("--r01", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--build-manifest", type=Path, required=True)
    parser.add_argument("--serial-python", type=Path)
    parser.add_argument("--serial-manifest", type=Path)
    parser.add_argument("--inspect-build", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args()
    identity = installed_build(args.build_manifest, args.revision)
    if args.inspect_build:
        print(json.dumps(identity))
        return
    if platform.system() not in ("Linux", "Darwin"):
        parser.error("measurement supports Linux and macOS")
    if not args.idle_machine or any(
        value is None for value in (args.build, args.r01, args.output)
    ):
        parser.error("measurement requires --idle-machine, --build, --r01 and --output")
    if identity["features"] != ["parallel"]:
        parser.error("the primary interpreter requires the parallel build")
    builds = {"all": identity}
    if bool(args.serial_python) != bool(args.serial_manifest):
        parser.error("--serial-python and --serial-manifest are required together")
    if args.serial_python:
        builds["one"] = inspect_build(
            args.serial_python, args.serial_manifest, args.revision
        )
        if builds["one"]["features"]:
            parser.error("the serial interpreter must use a build without parallel")
    elif platform.system() == "Darwin":
        parser.error("macOS requires --serial-python and --serial-manifest")
    else:
        builds["one"] = identity
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
    commands = {
        (implementation, threads, path): command(
            implementation,
            path,
            threads,
            str(args.serial_python)
            if implementation == "hdfour" and threads == "one" and args.serial_python
            else sys.executable,
        )
        for path, _ in files
        for implementation, threads in variants
    }
    for path, golden in files:
        for implementation, threads in variants:
            subprocess.run(
                [*commands[implementation, threads, path], "--check", str(golden)],
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
                            commands[implementation, threads, path],
                            Path(directory),
                            partial(
                                check_build,
                                args.serial_manifest
                                if threads == "one" and args.serial_python
                                else args.build_manifest,
                                builds[threads],
                                args.serial_python if threads == "one" else None,
                            ),
                        ),
                    }
                )
    document = {
        "date": datetime.date.today().isoformat(),
        "platform": platform.platform(),
        "architecture": platform.machine(),
        "hardware": machine,
        "cpu_affinity": sorted(os.sched_getaffinity(0))
        if platform.system() == "Linux"
        else None,
        "python": platform.python_version(),
        "versions": versions,
        "builds": builds,
        "reference_libraries": reference_libraries(),
        "hyperfine": subprocess.check_output(
            ["hyperfine", "--version"], text=True
        ).strip(),
        "method": {
            "warmups": 3,
            "speed_runs": 20,
            "rss_runs": 5,
            "idle_machine": True,
            "one_thread": "serial build"
            if args.serial_python
            else "Linux CPU affinity",
            "rss_tool": "macOS time -l (bytes converted to KiB)"
            if platform.system() == "Darwin"
            else "GNU time -f %M (KiB)",
        },
        "results": results,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(document, indent=2) + "\n")


if __name__ == "__main__":
    main()
