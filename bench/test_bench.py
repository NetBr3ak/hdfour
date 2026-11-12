"""Contract tests for benchmark validation and measurement separation."""

import importlib.util
import tempfile
import unittest
import weakref
from pathlib import Path
from unittest.mock import patch

import numpy as np


def load_reader():
    specification = importlib.util.spec_from_file_location(
        "bench_reader", Path(__file__).with_name("read.py")
    )
    module = importlib.util.module_from_spec(specification)
    specification.loader.exec_module(module)
    return module


class BenchmarkContracts(unittest.TestCase):
    def test_previous_array_is_released_before_the_next_read(self):
        reader = load_reader()

        def arrays(implementation, path):
            values = np.zeros((2, 2), dtype=np.int16)
            reference = weakref.ref(values)
            yield "first", values
            del values
            self.assertIsNone(reference())
            yield "second", np.ones((2, 2), dtype=np.int16)

        with (
            patch.object(reader, "read_arrays", arrays),
            patch("sys.argv", ["read.py", "--impl", "pyhdf", "--file", "unused.hdf"]),
        ):
            reader.main()

    def test_bit_exact_comparison_rejects_changed_value(self):
        reader = load_reader()
        with tempfile.TemporaryDirectory() as directory:
            golden = Path(directory) / "0.bin"
            golden.write_bytes(b"\x00\x80\xff\x7f")
            reader.check_array(np.array([-32768, 32767], dtype=np.int16), golden)
            with self.assertRaises(ValueError):
                reader.check_array(np.array([-32768, 32766], dtype=np.int16), golden)

    def test_bit_exact_comparison_preserves_float_bits(self):
        reader = load_reader()
        with tempfile.TemporaryDirectory() as directory:
            golden = Path(directory) / "0.bin"
            golden.write_bytes(bytes.fromhex("000000800100c07f"))
            values = np.array([0x80000000, 0x7FC00001], dtype=np.uint32).view(
                np.float32
            )
            reader.check_array(values, golden)
            with self.assertRaises(ValueError):
                reader.check_array(np.array([0.0, np.nan], dtype=np.float32), golden)

    def test_generated_golden_includes_shape_and_type(self):
        reader = load_reader()
        with tempfile.TemporaryDirectory() as directory:
            golden = Path(directory)
            (golden / "manifest.json").write_text(
                '[{"name":"field","dtype":"<i2","shape":[1,2]}]'
            )
            (golden / "0.bin").write_bytes(b"\x01\x00\x02\x00")
            reader.check_dataset("field", np.array([[1, 2]], dtype=np.int16), golden, 0)
            with self.assertRaises(ValueError):
                reader.check_dataset(
                    "field", np.array([1, 2], dtype=np.int16), golden, 0
                )
            with self.assertRaises(ValueError):
                reader.check_dataset(
                    "field", np.array([[1, 2]], dtype=np.uint16), golden, 0
                )


if __name__ == "__main__":
    unittest.main()
