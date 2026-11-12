"""Build identity and macOS benchmark contracts, runnable on Linux."""

import hashlib
import importlib.util
import json
import subprocess
import tempfile
import types
import unittest
from pathlib import Path
from unittest.mock import patch


def load_measure():
    specification = importlib.util.spec_from_file_location(
        "bench_measure", Path(__file__).with_name("measure.py")
    )
    module = importlib.util.module_from_spec(specification)
    specification.loader.exec_module(module)
    return module


class BuildIdentity(unittest.TestCase):
    def test_installed_extension_must_match_manifest_and_revision(self):
        measure = load_measure()
        with tempfile.TemporaryDirectory() as directory:
            extension = Path(directory) / "_hdfour.so"
            extension.write_bytes(b"installed extension")
            manifest = Path(directory) / "build.json"
            identity = {
                "revision": "a" * 40,
                "features": ["parallel"],
                "extension_sha256": hashlib.sha256(extension.read_bytes()).hexdigest(),
            }
            manifest.write_text(json.dumps(identity))
            with patch.object(
                measure.importlib.util,
                "find_spec",
                return_value=types.SimpleNamespace(origin=str(extension)),
            ):
                self.assertEqual(measure.installed_build(manifest, "a" * 40), identity)
                with self.assertRaisesRegex(ValueError, "revision"):
                    measure.installed_build(manifest, "b" * 40)
                extension.write_bytes(b"another equally correct build")
                with self.assertRaisesRegex(ValueError, "hash"):
                    measure.installed_build(manifest, "a" * 40)

    def test_missing_features_are_rejected(self):
        measure = load_measure()
        with tempfile.TemporaryDirectory() as directory:
            manifest = Path(directory) / "build.json"
            manifest.write_text(json.dumps({"revision": "a" * 40}))
            with self.assertRaisesRegex(ValueError, "features"):
                measure.installed_build(manifest, "a" * 40)


class MacMeasurement(unittest.TestCase):
    def test_macos_rejects_empty_cpu_or_nonpositive_memory(self):
        measure = load_measure()
        for cpu, memory in (
            ("  \n", "8589934592"),
            ("ARM CPU", "0"),
            ("ARM CPU", "-1"),
        ):
            with (
                self.subTest(cpu=cpu, memory=memory),
                patch.object(measure.platform, "system", return_value="Darwin"),
                patch.object(
                    measure.subprocess, "check_output", side_effect=[cpu, memory]
                ),
                self.assertRaisesRegex(ValueError, "hardware"),
            ):
                measure.hardware()

    def test_hardware_uses_sysctl_without_recording_host_identity(self):
        measure = load_measure()
        values = {"machdep.cpu.brand_string": "ARM CPU", "hw.memsize": "8589934592"}
        with (
            patch.object(measure.platform, "system", return_value="Darwin"),
            patch.object(
                measure.subprocess,
                "check_output",
                side_effect=lambda arguments, **kwargs: values[arguments[-1]],
            ),
        ):
            self.assertEqual(
                measure.hardware(),
                {"cpu_model": "ARM CPU", "physical_memory_bytes": 8589934592},
            )

    def test_macos_rss_bytes_are_converted_to_kib(self):
        measure = load_measure()
        calls = []
        with tempfile.TemporaryDirectory() as directory:
            temporary = Path(directory)

            def run(arguments, **kwargs):
                calls.append(arguments)
                if arguments[0] == "hyperfine":
                    (temporary / "speed.json").write_text('{"results":[{"mean":1}]}')
                return subprocess.CompletedProcess(
                    arguments,
                    0,
                    stderr="read diagnostic\n  2097152  maximum resident set size\n",
                )

            with (
                patch.object(measure.platform, "system", return_value="Darwin"),
                patch.object(measure.subprocess, "run", side_effect=run),
            ):
                result = measure.measure(["python", "read.py"], temporary)
            self.assertEqual(result["peak_rss_kib"], [2048] * 5)
            self.assertTrue(all(call[1] == "-l" for call in calls[1:]))

    def test_one_thread_command_uses_the_serial_interpreter(self):
        measure = load_measure()
        self.assertEqual(
            measure.command("hdfour", Path("data.hdf"), "one", "/serial/python")[0],
            "/serial/python",
        )


class AssemblyInspection(unittest.TestCase):
    def test_elf_and_macho_symbols_stop_before_other_functions(self):
        for symbol, end in (
            ("_ZN6hdfour2nt13swap_in_place17h123E:", ".Lfunc_end0:"),
            ("__ZN6hdfour2nt13swap_in_place17h123E:", "Lfunc_end0:"),
        ):
            with self.subTest(symbol=symbol):
                result = subprocess.run(
                    ["awk", "-f", str(Path(__file__).with_name("simd.awk"))],
                    input=f"{symbol}\n rev16 v0.16b, v0.16b\n{end}\nother:\n ret\n",
                    text=True,
                    capture_output=True,
                    check=True,
                )
                self.assertIn("rev16", result.stdout)
                self.assertNotIn("other:", result.stdout)

    def test_unknown_argument_fails_without_invoking_cargo(self):
        result = subprocess.run(
            ["sh", str(Path(__file__).with_name("simd.sh")), "--unknown"],
            text=True,
            capture_output=True,
        )
        self.assertEqual(result.returncode, 2)
        self.assertIn("Usage:", result.stderr)


if __name__ == "__main__":
    unittest.main()
