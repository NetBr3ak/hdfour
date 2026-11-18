"""Rectangular reads retain shape, dtype, and owned values."""

from pathlib import Path

import numpy as np
import pytest

import hdfour

FIXTURES = Path(__file__).resolve().parents[2] / "fixtures"


@pytest.mark.parametrize("name", ["i16", "f32", "char8"])
def test_window_matches_slice(name):
    dataset = hdfour.open(FIXTURES / "F01.hdf")[name]
    expected = dataset.read()[1:3, 1:4]
    actual = dataset.read_window([1, 1], [2, 3])
    np.testing.assert_array_equal(actual, expected)
    assert actual.shape == (2, 3)
    assert actual.dtype == expected.dtype
    actual.flat[0] = 0
    np.testing.assert_array_equal(dataset.read_window([1, 1], [2, 3]), expected)


def test_window_rejects_invalid_coordinates():
    dataset = hdfour.open(FIXTURES / "F01.hdf")["i16"]
    with pytest.raises(ValueError):
        dataset.read_window([1], [1])
    with pytest.raises(ValueError):
        dataset.read_window([2, 4], [2, 2])
    with pytest.raises(OverflowError):
        dataset.read_window([-1, 0], [1, 1])
