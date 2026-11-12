"""Independent checks that each reference file exercises its named storage case."""

import importlib.util
import json
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile
import unittest

FIXTURES = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("fixture_dump", FIXTURES / "dump.py")
DUMP = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(DUMP)


def descriptors(name):
    raw = (FIXTURES / f"{name}.hdf").read_bytes()
    result = []
    position = 4
    visited = set()
    while position:
        if position in visited:
            raise ValueError("descriptor loop")
        visited.add(position)
        count, following = struct.unpack_from(">HI", raw, position)
        for index in range(count):
            tag, ref, offset, length = struct.unpack_from(
                ">HHII", raw, position + 6 + 12 * index
            )
            if tag != 1:
                result.append((tag, ref, raw[offset : offset + length]))
        position = following
    return result


def metadata(name):
    return json.loads((FIXTURES / "golden" / f"{name}.json").read_text())


class FixtureTests(unittest.TestCase):
    def test_all_number_types_and_byte_orders(self):
        for name, number_class in (("F01", 1), ("F02", 4)):
            records = [value for tag, _, value in descriptors(name) if tag == 106]
            self.assertEqual(
                {value[1] for value in records}, {3, 4, 5, 6, 20, 21, 22, 23, 24, 25}
            )
            self.assertEqual({value[3] for value in records}, {number_class})
            self.assertEqual(len(metadata(name)["datasets"]), 10)

    def test_attributes_at_three_levels(self):
        record = metadata("F03")
        dataset = record["datasets"][0]
        for values in (
            record["attributes"],
            dataset["attributes"],
            dataset["dimensions"][0]["attributes"],
        ):
            self.assertEqual(
                {value["dtype"] for value in values},
                {3, 4, 5, 6, 20, 21, 22, 23, 24, 25},
            )

    def test_duplicate_names_keep_both_datasets(self):
        datasets = metadata("F06")["datasets"]
        self.assertEqual([value["index"] for value in datasets], [0, 1])
        self.assertEqual(datasets[0]["name"], datasets[1]["name"])
        self.assertEqual(
            (FIXTURES / "golden/F06/0.bin").read_bytes(),
            (FIXTURES / "golden/F06/1.bin").read_bytes(),
        )

    def test_three_metadata_parts_join_exactly(self):
        whole = [
            bytes.fromhex(value["hex"])
            for value in metadata("F15")["attributes"]
            if value["name"].startswith("StructMetadata.")
        ]
        parts = [
            bytes.fromhex(value["hex"])
            for value in metadata("F18")["attributes"]
            if value["name"].startswith("StructMetadata.")
        ]
        self.assertEqual(len(parts), 3)
        self.assertEqual(b"".join(parts), b"".join(whole))

    def test_dimension_versions_and_coordinate_variables(self):
        records = descriptors("F04")
        headers = [value for tag, _, value in records if tag == 1962]
        self.assertTrue(any(b"DimVal0.0" in value for value in headers))
        self.assertTrue(any(b"DimVal0.1" in value for value in headers))
        datasets = metadata("F04")["datasets"]
        self.assertEqual(sum(value["is_coordinate"] for value in datasets), 2)
        self.assertEqual(datasets[0]["dimensions"], datasets[-1]["dimensions"])

    def test_appends_create_linked_storage(self):
        linked = [value for tag, _, value in descriptors("F05") if tag == 0x42BE]
        self.assertEqual(len(linked), 1)
        self.assertEqual(struct.unpack_from(">H", linked[0])[0], 1)
        self.assertEqual(struct.unpack_from(">i", linked[0], 2)[0], 50)
        self.assertEqual(metadata("F05")["datasets"][0]["shape"], [5, 5])

    def test_chunk_shapes_partial_edges_and_sparse_fill(self):
        for name in ("F07", "F08", "F19"):
            chunks = [value for tag, _, value in descriptors(name) if tag == 0x42BE]
            self.assertEqual(struct.unpack_from(">H", chunks[0])[0], 5)
            self.assertEqual(metadata(name)["datasets"][0]["shape"], [5, 7])
        values = struct.unpack("<35h", (FIXTURES / "golden/F19/0.bin").read_bytes())
        self.assertEqual(sum(value == -12345 for value in values), 29)
        self.assertEqual(values[17:20], (0x2134, 0x2235, 0x2336))
        self.assertEqual(values[24:27], (0x2437, 0x2538, 0x2639))

    def test_vgroup_four_and_attributes(self):
        groups = [value for tag, _, value in descriptors("F17") if tag == 1965]
        self.assertIn(
            4, [struct.unpack_from(">H", value, len(value) - 5)[0] for value in groups]
        )
        self.assertTrue(
            any(
                b"vgroup_marker" in value
                for tag, _, value in descriptors("F17")
                if tag == 1962
            )
        )

    def test_short_element_is_deliberately_malformed(self):
        values = [value for tag, _, value in descriptors("F21") if tag == 702]
        self.assertEqual(len(values[0]), 4)
        self.assertEqual(metadata("F21")["datasets"][0]["expect"], "malformed")

    def test_legacy_datasets_are_all_skipped(self):
        for name in ("F14", "R02"):
            datasets = metadata(name)["datasets"]
            self.assertTrue(
                all(value["expect"] == "skipped:legacy-sd-layout" for value in datasets)
            )

    def test_empty_unlimited_dataset_has_no_records(self):
        dataset = metadata("F13")["datasets"][-1]
        self.assertEqual(dataset["name"], "empty_records")
        self.assertEqual(dataset["shape"], [0, 5])
        self.assertTrue(dataset["dimensions"][0]["unlimited"])
        self.assertEqual(
            (FIXTURES / "golden/F13" / f"{dataset['index']}.bin").read_bytes(), b""
        )

    def test_stale_metadata_and_values_are_rejected(self):
        with tempfile.TemporaryDirectory(dir=FIXTURES / ".build") as directory:
            target = Path(directory) / "golden.json"
            DUMP.compare_or_write(target, b"original", False)
            with self.assertRaisesRegex(ValueError, "stale golden"):
                DUMP.compare_or_write(target, b"changed", True)
            target = Path(directory) / "0.bin"
            DUMP.compare_or_write(target, b"\x01\x02", False)
            with self.assertRaisesRegex(ValueError, "stale golden"):
                DUMP.compare_or_write(target, b"\x02\x01", True)

    def test_c_check_rejects_changed_values(self):
        with tempfile.TemporaryDirectory(dir=FIXTURES / ".build") as directory:
            output = Path(directory) / "F01"
            shutil.copytree(FIXTURES / "golden/F01", output)
            command = [
                str(FIXTURES / ".build/sdsdump"),
                "--check",
                str(FIXTURES / "F01.hdf"),
                str(output),
            ]
            subprocess.run(command, check=True, capture_output=True)
            target = output / "2.bin"
            content = bytearray(target.read_bytes())
            content[0], content[1] = content[1], content[0]
            target.write_bytes(content)
            result = subprocess.run(command, capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("stale values", result.stderr)


if __name__ == "__main__":
    unittest.main(verbosity=2)
