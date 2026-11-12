"""Build benchmark wheels from verified, committed sources and record their identity."""

import argparse
import hashlib
import json
import re
import subprocess
import sys
import tarfile
import tempfile
import zipfile
from pathlib import Path


def wheel_identity(wheel: Path, revision: str, features: list[str]) -> dict:
    with zipfile.ZipFile(wheel) as archive:
        extensions = [
            name
            for name in archive.namelist()
            if name.startswith("hdfour/_hdfour") and name.endswith((".so", ".pyd"))
        ]
        if len(extensions) != 1:
            raise ValueError("wheel must contain exactly one hdfour extension")
        digest = hashlib.sha256(archive.read(extensions[0])).hexdigest()
    return {
        "revision": revision,
        "features": features,
        "extension_sha256": digest,
    }


def enabled_features(root: Path) -> list[str]:
    tree = subprocess.check_output(
        [
            "cargo",
            "tree",
            "--locked",
            "-p",
            "hdfour-py",
            "--edges",
            "normal",
            "--prefix",
            "none",
            "--format",
            "{p}|{f}",
        ],
        cwd=root,
        text=True,
    )
    rows = [line for line in tree.splitlines() if line.startswith("hdfour v")]
    if len(rows) != 1:
        raise ValueError("could not identify the core dependency features")
    features = rows[0].rsplit("|", 1)[1].strip()
    return sorted(features.split(",")) if features else []


def build_wheel(options: argparse.Namespace) -> None:
    output = options.output.resolve()
    with (
        tarfile.open(options.source_archive) as archive,
        tempfile.TemporaryDirectory(prefix="hdfour-wheel-") as directory,
    ):
        if archive.pax_headers.get("comment") != options.revision:
            raise ValueError("source archive revision differs from --revision")
        source = Path(directory)
        archive.extractall(source, filter="data")
        output.mkdir(parents=True, exist_ok=False)
        if options.threads == "one":
            binding = source / "crates/hdfour-py/Cargo.toml"
            original = binding.read_text()
            dependency = 'hdfour = { workspace = true, features = ["parallel"] }'
            if original.count(dependency) != 1:
                raise ValueError(
                    "binding dependency changed; review the serial build recipe"
                )
            binding.write_text(
                original.replace(dependency, "hdfour = { workspace = true }")
            )
        features = enabled_features(source)
        expected = [] if options.threads == "one" else ["parallel"]
        if features != expected:
            raise ValueError("resolved core features differ from the requested build")
        subprocess.run(
            [
                "maturin",
                "build",
                "--release",
                "--locked",
                "--out",
                str(output),
                "--interpreter",
                sys.executable,
            ],
            cwd=source,
            check=True,
        )
    wheels = list(output.glob("*.whl"))
    if len(wheels) != 1:
        raise ValueError("build must produce exactly one wheel")
    identity = wheel_identity(wheels[0], options.revision, features)
    (output / "build.json").write_text(json.dumps(identity, indent=2) + "\n")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--export-source", type=Path)
    mode.add_argument("--source-archive", type=Path)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--threads", choices=("one", "all"), default="all")
    parser.add_argument("--output", type=Path)
    options = parser.parse_args()
    if not re.fullmatch(r"[0-9a-f]{40}", options.revision):
        parser.error("--revision requires a full git revision")
    if options.export_source:
        root = Path(__file__).resolve().parent.parent
        revision = subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=root, text=True
        ).strip()
        if options.revision != revision:
            raise ValueError("source revision must be the full current git revision")
        with options.export_source.open("xb") as output:
            subprocess.run(
                ["git", "archive", "--format=tar", "HEAD"],
                cwd=root,
                stdout=output,
                check=True,
            )
        return
    if options.output is None:
        parser.error("building requires --output (a new directory)")
    build_wheel(options)


if __name__ == "__main__":
    main()
