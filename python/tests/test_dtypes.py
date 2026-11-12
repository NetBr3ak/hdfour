"""Check all scoped types against the C-library byte goldens."""

import gc
import json
from pathlib import Path

import numpy as np
import pytest

import hdfour

FIXTURES = Path(__file__).resolve().parents[2] / "fixtures"
TYPES = {
    "i8": "i1",
    "u8": "u1",
    "i16": "i2",
    "u16": "u2",
    "i32": "i4",
    "u32": "u4",
    "f32": "f4",
    "f64": "f8",
    "char8": "i1",
    "uchar8": "u1",
}


@pytest.mark.parametrize("fixture", ["F01", "F02", "F13"])
def test_all_dtypes_and_fill_values(fixture):
    file = hdfour.open(FIXTURES / f"{fixture}.hdf")
    metadata = json.loads((FIXTURES / "golden" / f"{fixture}.json").read_text())
    for index, (dataset, golden) in enumerate(
        zip(file.datasets, metadata["datasets"], strict=True)
    ):
        values = dataset.read()
        assert values.dtype == np.dtype(TYPES[dataset.dtype])
        assert values.dtype.isnative
        assert values.shape == tuple(golden["shape"])
        assert values.flags.c_contiguous
        assert values.base is not None
        assert not values.flags.owndata
        expected = np.frombuffer(
            (FIXTURES / "golden" / fixture / f"{index}.bin").read_bytes(),
            dtype=values.dtype.newbyteorder("<"),
        )
        assert values.tobytes() == expected.astype(values.dtype).tobytes()


def test_attributes_keep_types_order_and_raw_values():
    file = hdfour.open(FIXTURES / "F03.hdf")
    for attrs in [file.attrs, file.datasets[0].attrs, file.datasets[0].dims[0].attrs]:
        assert list(attrs)[: len(TYPES)] == list(TYPES)
        assert attrs["char8"] == "ABC"
        for name, dtype in TYPES.items():
            if name == "char8":
                continue
            assert attrs[name].dtype == np.dtype(dtype)
            assert attrs[name].ndim == 1
        attrs["i16"][0] = 0
    assert file.attrs["i16"][0] != 0
    assert file.datasets[0].attrs["i16"][0] != 0


def test_char8_attributes_decode_latin1():
    data = (FIXTURES / "F03.hdf").read_bytes().replace(b"ABC", b"A\x80\xff")
    file = hdfour.from_bytes(data)
    for attrs in [file.attrs, file.datasets[0].attrs, file.datasets[0].dims[0].attrs]:
        assert attrs["char8"] == "A\x80\xff"


def test_char8_arrays_keep_signed_high_bytes():
    data = (FIXTURES / "F01.hdf").read_bytes()
    pattern = b"ABCDEFGHIJKLMNO"
    assert data.count(pattern) == 1
    raw = bytes(range(128, 143))
    values = hdfour.from_bytes(data.replace(pattern, raw))["char8"].read()
    assert values.dtype == np.dtype("i1")
    assert values.tobytes() == raw
    assert values[0, 0] == -128


def test_dataset_and_array_own_their_lifetimes():
    dataset = hdfour.from_bytes((FIXTURES / "F01.hdf").read_bytes())["i16"]
    gc.collect()
    values = dataset.read()
    expected = values.copy()
    del dataset
    gc.collect()
    assert values.tobytes() == expected.tobytes()
    values[0, 0] = 0
    assert values[0, 0] == 0


def test_reads_have_independent_buffers():
    dataset = hdfour.open(FIXTURES / "F01.hdf")["i16"]
    first, second = dataset.read(), dataset.read()
    assert not np.shares_memory(first, second)
    first[0, 0] = 0
    assert second[0, 0] != 0
