"""Compare raw metadata and values with pyhdf, by dataset index."""

import json
from pathlib import Path

import numpy as np
from pyhdf.SD import SD, SDC, HDF4Error
import pytest

import hdfour

ROOT = Path(__file__).resolve().parents[2]
EXPECT = json.loads((ROOT / "fixtures/expect.json").read_text())
DTYPES = {
    3: "u1",
    4: "i1",
    5: "f4",
    6: "f8",
    20: "i1",
    21: "u1",
    22: "i2",
    23: "u2",
    24: "i4",
    25: "u4",
}


def input_files():
    files = [
        (f"F{index:02}", ROOT / f"fixtures/F{index:02}.hdf") for index in range(1, 22)
    ]
    files.extend((path.stem, path) for path in sorted((ROOT / "corpus").glob("R*.hdf")))
    manifest = json.loads((ROOT / "corpus/manifest.json").read_text())
    for entry in manifest:
        path = ROOT / "corpus/cache" / entry["filename"]
        if not entry["committed"] and path.is_file():
            files.append((entry["id"], path))
    files.extend(
        (path.stem, path)
        for path in sorted((ROOT / "corpus/local").glob("*"))
        if path.is_file()
    )
    return files


FILES = input_files()


def dataset_cases():
    cases = []
    for identifier, path in FILES:
        handle = SD(str(path), SDC.READ)
        try:
            for index in range(handle.info()[0]):
                expectation = (
                    EXPECT.get(identifier, {})
                    .get("datasets", {})
                    .get(str(index), "values")
                )
                cases.append(
                    pytest.param(path, index, expectation, id=f"{identifier}-{index}")
                )
        finally:
            handle.end()
    return cases


def compare_attrs(actual, owner):
    attributes = [owner.attr(index) for index in range(len(owner.attributes()))]
    assert list(actual) == [attribute.info()[0] for attribute in attributes]
    for attribute in attributes:
        name, dtype, _ = attribute.info()
        value = attribute.get()
        if dtype & ~0x4000 == SDC.CHAR8:
            assert actual[name] == value
            continue
        expected = np.asarray(value, dtype=DTYPES[dtype & ~0x4000]).reshape(-1)
        assert actual[name].ndim == 1
        assert actual[name].dtype == expected.dtype
        assert actual[name].tobytes() == expected.tobytes()


@pytest.mark.parametrize("identifier,path", FILES, ids=[item[0] for item in FILES])
def test_diff_file(identifier, path):
    actual = hdfour.open(path)
    expected = EXPECT.get(identifier, {})
    if "skipped" in expected:
        assert actual.skipped == [
            (item["object"], item["feature"]) for item in expected["skipped"]
        ]
    if "eos" in expected:
        assert json.loads(json.dumps(actual.eos)) == expected["eos"]
    handle = SD(str(path), SDC.READ)
    try:
        skipped = sum(
            value.startswith("skipped:")
            for value in expected.get("datasets", {}).values()
        )
        assert len(actual.datasets) == handle.info()[0] - skipped
        compare_attrs(actual.attrs, handle)
    finally:
        handle.end()


@pytest.mark.parametrize("path,index,expectation", dataset_cases())
def test_diff_dataset(path, index, expectation):
    actual_file = hdfour.open(path)
    if expectation.startswith("skipped:"):
        assert not actual_file.datasets
        assert expectation.split(":", 1)[1] in [
            feature for _, feature in actual_file.skipped
        ]
        return
    actual = actual_file.datasets[index]
    handle = SD(str(path), SDC.READ)
    reference = handle.select(index)
    try:
        name, rank, shape, dtype, _ = reference.info()
        shape = [shape] if isinstance(shape, int) else list(shape)
        assert actual.name == name
        assert actual.shape == shape
        assert actual.is_coordinate == bool(reference.iscoordvar())
        compare_attrs(actual.attrs, reference)
        assert len(actual.dims) == rank
        for axis, dimension in enumerate(actual.dims):
            expected_dimension = reference.dim(axis)
            dim_name, size, _, _ = expected_dimension.info()
            assert (dimension.name, dimension.size, dimension.unlimited) == (
                dim_name,
                shape[axis],
                size == 0,
            )
            compare_attrs(dimension.attrs, expected_dimension)
        if expectation.startswith("unsupported:"):
            with pytest.raises(
                hdfour.UnsupportedError, match=expectation.split(":", 1)[1]
            ):
                actual.read()
            return
        if expectation == "malformed":
            with pytest.raises(hdfour.FormatError):
                actual.read()
            return
        values = actual.read()
        assert values.dtype == np.dtype(DTYPES[dtype & ~0x4000])
        try:
            expected_values = reference.get()
        except (HDF4Error, ValueError) as error:
            # pyhdf 0.11.7 has no reader for some number types and fails on an empty
            # dataset; test_dtypes.py compares those with the C library's golden bytes.
            # Any other pyhdf failure is unexplained and must fail here.
            if "cannot currently deal with the SDS data type" in str(error):
                pytest.skip(f"pyhdf cannot read this number type: {error}")
            if values.size == 0:
                return
            raise
        if dtype & ~0x4000 == SDC.CHAR8:
            expected_values = np.asarray(expected_values).view(np.int8)
        else:
            expected_values = np.asarray(expected_values, dtype=values.dtype)
        assert values.shape == expected_values.shape
        assert values.flags.c_contiguous
        assert values.dtype.isnative
        assert values.tobytes() == expected_values.tobytes()
    finally:
        reference.endaccess()
        handle.end()
