"""Check public exceptions and selection without changing core limits."""

from pathlib import Path
import struct

from pyhdf.SD import SD, SDC
import pytest

import hdfour

FIXTURES = Path(__file__).resolve().parents[2] / "fixtures"


@pytest.mark.parametrize("data", [b"", b"not HDF4", b"\x89HDF\r\n\x1a\n"])
def test_not_hdf4_is_format_error(data):
    assert issubclass(hdfour.FormatError, ValueError)
    with pytest.raises(hdfour.FormatError, match="not an HDF4"):
        hdfour.from_bytes(data)


def test_truncated_hdf4_is_format_error():
    with pytest.raises(hdfour.FormatError, match="malformed"):
        hdfour.from_bytes(b"\x0e\x03\x13\x01")


def test_missing_file_is_oserror(tmp_path):
    with pytest.raises(OSError):
        hdfour.open(tmp_path / "missing.hdf")


@pytest.mark.parametrize(
    "fixture,feature",
    [("F10", "skipping-huffman"), ("F11", "nbit"), ("F12", "external-element")],
)
def test_unsupported_read_is_notimplementederror(fixture, feature):
    assert issubclass(hdfour.UnsupportedError, NotImplementedError)
    dataset = hdfour.open(FIXTURES / f"{fixture}.hdf").datasets[0]
    with pytest.raises(hdfour.UnsupportedError, match=feature):
        dataset.read()


def test_short_storage_is_format_error():
    with pytest.raises(hdfour.FormatError, match="malformed"):
        hdfour.open(FIXTURES / "F21.hdf").datasets[0].read()


def test_descriptor_block_limit_is_valueerror():
    assert issubclass(hdfour.LimitError, ValueError)
    count = 65537
    blocks = [
        struct.pack(">HI", 0, 4 + (index + 1) * 6 if index + 1 < count else 0)
        for index in range(count)
    ]
    with pytest.raises(hdfour.LimitError, match="dd_blocks"):
        hdfour.from_bytes(b"\x0e\x03\x13\x01" + b"".join(blocks))


def test_missing_dataset_is_keyerror():
    with pytest.raises(KeyError, match="missing"):
        hdfour.open(FIXTURES / "F01.hdf")["missing"]


def test_duplicate_dataset_is_keyerror(tmp_path):
    path = tmp_path / "duplicates.hdf"
    handle = SD(str(path), SDC.CREATE | SDC.WRITE)
    try:
        for value in [1, 2]:
            dataset = handle.create("duplicate", SDC.INT16, (1,))
            dataset[:] = [value]
            dataset.endaccess()
    finally:
        handle.end()
    file = hdfour.open(path)
    assert [dataset.name for dataset in file.datasets] == ["duplicate", "duplicate"]
    assert [dataset.read().item() for dataset in file.datasets] == [1, 2]
    with pytest.raises(KeyError, match="duplicate"):
        file["duplicate"]
