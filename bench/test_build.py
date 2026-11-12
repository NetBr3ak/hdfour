"""Source and wheel provenance contracts without a compiler or installed binding."""

import hashlib
import importlib.util
import io
import json
import sys
import tarfile
import tempfile
import unittest
import zipfile
from pathlib import Path
from unittest.mock import patch


def load_build():
    specification = importlib.util.spec_from_file_location(
        "bench_build", Path(__file__).with_name("build.py")
    )
    module = importlib.util.module_from_spec(specification)
    specification.loader.exec_module(module)
    return module


class SourceIdentity(unittest.TestCase):
    def test_wheel_manifest_hashes_the_built_extension(self):
        build = load_build()
        with tempfile.TemporaryDirectory() as directory:
            wheel = Path(directory) / "reader.whl"
            with zipfile.ZipFile(wheel, "w") as archive:
                archive.writestr("hdfour/_hdfour.abi3.so", b"built extension")
                archive.writestr("hdfour/__init__.py", b"package wrapper")
            identity = build.wheel_identity(wheel, "a" * 40, ["parallel"])
            self.assertEqual(identity["revision"], "a" * 40)
            self.assertEqual(identity["features"], ["parallel"])
            self.assertEqual(
                identity["extension_sha256"],
                hashlib.sha256(b"built extension").hexdigest(),
            )

    def test_build_uses_archive_and_records_the_built_extension(self):
        build = load_build()
        revision = "a" * 40
        dependency = 'hdfour = { workspace = true, features = ["parallel"] }'
        for threads, features in (("all", ["parallel"]), ("one", [])):
            with (
                self.subTest(threads=threads),
                tempfile.TemporaryDirectory() as directory,
            ):
                root = Path(directory)
                source = root / "source.tar"
                output = root / "wheel"
                with tarfile.open(
                    source, "w", pax_headers={"comment": revision}
                ) as archive:
                    entry = tarfile.TarInfo("crates/hdfour-py/Cargo.toml")
                    entry.size = len(dependency)
                    archive.addfile(entry, io.BytesIO(dependency.encode()))

                def compile_wheel(arguments, **kwargs):
                    self.assertEqual(arguments[0], "maturin")
                    binding = Path(kwargs["cwd"]) / "crates/hdfour-py/Cargo.toml"
                    expected = (
                        dependency
                        if threads == "all"
                        else "hdfour = { workspace = true }"
                    )
                    self.assertEqual(binding.read_text(), expected)
                    with zipfile.ZipFile(output / "reader.whl", "w") as wheel:
                        wheel.writestr("hdfour/_hdfour.abi3.so", b"built extension")

                with (
                    patch.object(
                        sys,
                        "argv",
                        [
                            "build.py",
                            "--source-archive",
                            str(source),
                            "--revision",
                            revision,
                            "--threads",
                            threads,
                            "--output",
                            str(output),
                        ],
                    ),
                    patch.object(build, "enabled_features", return_value=features),
                    patch.object(build.subprocess, "run", side_effect=compile_wheel),
                ):
                    build.main()
                self.assertEqual(
                    (output / "build.json").read_text(),
                    json.dumps(
                        {
                            "revision": revision,
                            "features": features,
                            "extension_sha256": hashlib.sha256(
                                b"built extension"
                            ).hexdigest(),
                        },
                        indent=2,
                    )
                    + "\n",
                )

    def test_archive_revision_mismatch_stops_before_compilation(self):
        build = load_build()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source.tar"
            with tarfile.open(
                source, "w", pax_headers={"comment": "a" * 40}
            ) as archive:
                archive.addfile(tarfile.TarInfo("Cargo.toml"))
            with (
                patch.object(
                    sys,
                    "argv",
                    [
                        "build.py",
                        "--source-archive",
                        str(source),
                        "--revision",
                        "b" * 40,
                        "--output",
                        str(root / "wheel"),
                    ],
                ),
                patch.object(build.subprocess, "run") as compile_wheel,
                self.assertRaisesRegex(ValueError, "revision"),
            ):
                build.main()
            compile_wheel.assert_not_called()


if __name__ == "__main__":
    unittest.main()
