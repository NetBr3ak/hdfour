"""A measurement cannot retain another extension's verified build identity."""

import hashlib
import importlib.util
import json
import subprocess
import sys
import tempfile
import types
import unittest
from pathlib import Path
from unittest.mock import patch


class MeasurementIdentity(unittest.TestCase):
    def test_parent_refuses_a_result_from_a_replaced_extension(self):
        specification = importlib.util.spec_from_file_location(
            "bench_measure", Path(__file__).with_name("measure.py")
        )
        measure = importlib.util.module_from_spec(specification)
        specification.loader.exec_module(measure)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in ("chunked.hdf", "contiguous.hdf", "R01.hdf"):
                (root / name).write_bytes(b"input")
            arguments = [
                "measure.py",
                "--idle-machine",
                "--build",
                str(root),
                "--r01",
                str(root / "R01.hdf"),
                "--output",
                str(root / "result.json"),
                "--revision",
                "a" * 40,
                "--build-manifest",
                str(root / "build.json"),
            ]
            identity = {
                "revision": "a" * 40,
                "features": ["parallel"],
                "extension_sha256": hashlib.sha256(b"verified extension").hexdigest(),
            }
            extension = root / "_hdfour.so"
            extension.write_bytes(b"verified extension")
            (root / "build.json").write_text(json.dumps(identity))

            timed_runs = []

            def run(arguments, **kwargs):
                if arguments[0] == "hyperfine":
                    timed_runs.append(arguments)
                    Path(arguments[6]).write_text('{"results":[{}]}')
                    extension.write_bytes(b"replaced extension")
                elif arguments[0] == "/usr/bin/time":
                    Path(arguments[4]).write_text("1024\n")
                return subprocess.CompletedProcess(arguments, 0)

            with (
                patch.object(sys, "argv", arguments),
                patch.object(
                    measure.importlib.util,
                    "find_spec",
                    return_value=types.SimpleNamespace(origin=str(extension)),
                ),
                patch.object(measure.platform, "system", return_value="Linux"),
                patch.object(
                    measure.importlib.metadata, "version", return_value="0.11.7"
                ),
                patch.object(measure, "hardware", return_value={}),
                patch.object(measure, "reference_libraries", return_value={}),
                patch.object(measure.subprocess, "run", side_effect=run),
                patch.object(
                    measure.subprocess, "check_output", return_value="hyperfine"
                ),
                self.assertRaisesRegex(ValueError, "hash"),
            ):
                measure.main()
            self.assertEqual(len(timed_runs), 1)
            self.assertFalse((root / "result.json").exists())

    def test_each_timed_command_has_untimed_identity_checks(self):
        specification = importlib.util.spec_from_file_location(
            "bench_measure", Path(__file__).with_name("measure.py")
        )
        measure = importlib.util.module_from_spec(specification)
        specification.loader.exec_module(measure)
        events = []
        with tempfile.TemporaryDirectory() as directory:
            temporary = Path(directory)

            def run(arguments, **kwargs):
                events.append("run")
                if arguments[0] == "hyperfine":
                    (temporary / "speed.json").write_text('{"results":[{}]}')
                else:
                    (temporary / "rss.txt").write_text("1024\n")
                return subprocess.CompletedProcess(arguments, 0)

            with (
                patch.object(measure.platform, "system", return_value="Linux"),
                patch.object(measure.subprocess, "run", side_effect=run),
            ):
                measure.measure(
                    ["python", "read.py"], temporary, lambda: events.append("check")
                )
        self.assertEqual(events, ["check", "run", "check"] * 6)

    def test_failed_precheck_does_not_start_a_timed_command(self):
        specification = importlib.util.spec_from_file_location(
            "bench_measure", Path(__file__).with_name("measure.py")
        )
        measure = importlib.util.module_from_spec(specification)
        specification.loader.exec_module(measure)

        def changed_build():
            raise ValueError("extension hash changed")

        with (
            tempfile.TemporaryDirectory() as directory,
            patch.object(measure.subprocess, "run") as run,
            self.assertRaisesRegex(ValueError, "hash"),
        ):
            measure.measure(["python", "read.py"], Path(directory), changed_build)
        run.assert_not_called()


if __name__ == "__main__":
    unittest.main()
