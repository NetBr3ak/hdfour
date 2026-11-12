"""Check metadata, ownership and concurrent reads through the public API."""

from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import pytest

import hdfour

FIXTURES = Path(__file__).resolve().parents[2] / "fixtures"


def test_open_path_string_and_bytes_match():
    path = FIXTURES / "F01.hdf"
    files = [
        hdfour.open(path),
        hdfour.open(str(path)),
        hdfour.from_bytes(path.read_bytes()),
    ]
    assert all(isinstance(file, hdfour.File) for file in files)
    for file in files:
        assert file["i16"].read().tobytes() == files[0]["i16"].read().tobytes()
        assert file.eos is None
        assert file.skipped == []


@pytest.mark.parametrize(
    "fixture,kind",
    [
        ("F01", "contiguous"),
        ("F05", "linked"),
        ("F07", "chunked"),
        ("F12", "external"),
        ("F13", "empty"),
    ],
)
def test_storage_metadata(fixture, kind):
    storage = hdfour.open(FIXTURES / f"{fixture}.hdf").datasets[0].storage
    assert storage["kind"] == kind
    if kind == "chunked":
        assert storage["chunk_shape"] == [2, 3]


@pytest.mark.parametrize(
    "fixture,codec",
    [("F01", "none"), ("F06", "deflate"), ("F09", "rle"), ("F10", 3), ("F11", 2)],
)
def test_codec_metadata(fixture, codec):
    assert hdfour.open(FIXTURES / f"{fixture}.hdf").datasets[0].codec == codec


def test_metadata_is_read_only():
    dataset = hdfour.open(FIXTURES / "F01.hdf")["i16"]
    with pytest.raises(AttributeError):
        dataset.name = "changed"
    shape = dataset.shape
    shape.clear()
    assert dataset.shape == [3, 5]


def test_shared_dataset_reads_on_threads():
    dataset = hdfour.open(FIXTURES / "F08.hdf").datasets[0]
    expected = dataset.read().tobytes()
    with ThreadPoolExecutor(max_workers=2) as executor:
        results = list(executor.map(lambda _: dataset.read().tobytes(), range(4)))
    assert results == [expected] * 4
