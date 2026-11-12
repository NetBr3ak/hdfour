"""Source exports must contain committed blobs, not hidden working-tree edits."""

import importlib.util
import io
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


def load_build():
    specification = importlib.util.spec_from_file_location(
        "bench_build", Path(__file__).with_name("build.py")
    )
    module = importlib.util.module_from_spec(specification)
    specification.loader.exec_module(module)
    return module


class CommittedArchive(unittest.TestCase):
    def test_export_uses_committed_blobs_when_status_hides_a_worktree_edit(self):
        build = load_build()
        revision = "a" * 40
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "Cargo.toml").write_bytes(b"uncommitted edit")
            destination = root / "source.tar"

            def git_output(arguments, **kwargs):
                if arguments[1] == "rev-parse":
                    return revision + "\n"
                if arguments[1] == "status":
                    return ""
                if arguments[1] == "ls-files":
                    return "Cargo.toml\0"
                raise AssertionError(arguments)

            def git_archive(arguments, **kwargs):
                self.assertEqual(arguments, ["git", "archive", "--format=tar", "HEAD"])
                with tarfile.open(
                    fileobj=kwargs["stdout"],
                    mode="w",
                    pax_headers={"comment": revision},
                ) as archive:
                    entry = tarfile.TarInfo("Cargo.toml")
                    entry.size = len(b"committed blob")
                    archive.addfile(entry, io.BytesIO(b"committed blob"))

            with (
                patch.object(build, "__file__", str(root / "bench/build.py")),
                patch.object(build.subprocess, "check_output", side_effect=git_output),
                patch.object(
                    build.subprocess, "run", side_effect=git_archive
                ) as export,
                patch.object(
                    sys,
                    "argv",
                    [
                        "build.py",
                        "--export-source",
                        str(destination),
                        "--revision",
                        revision,
                    ],
                ),
            ):
                build.main()
            with tarfile.open(destination) as archive:
                self.assertEqual(archive.pax_headers["comment"], revision)
                self.assertEqual(
                    archive.extractfile("Cargo.toml").read(), b"committed blob"
                )
            export.assert_called_once()


if __name__ == "__main__":
    unittest.main()
