"""Keep the published stub and differential failure handling honest."""

import ast
from pathlib import Path
from types import SimpleNamespace

import numpy as np
from pyhdf.SD import HDF4Error
import pytest

import hdfour
import test_diff

STUB = Path(hdfour.__file__).with_name("__init__.pyi")


def test_stub_helper_names_are_private():
    tree = ast.parse(STUB.read_text())
    names = []
    for node in tree.body:
        if isinstance(node, (ast.ClassDef, ast.FunctionDef)):
            names.append(node.name)
        if isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name):
            names.append(node.target.id)
    assert [
        name for name in names if not name.startswith("_") and not hasattr(hdfour, name)
    ] == []


def test_stub_getitem_is_positional_only():
    tree = ast.parse(STUB.read_text())
    file = next(
        node
        for node in tree.body
        if isinstance(node, ast.ClassDef) and node.name == "File"
    )
    method = next(
        node
        for node in file.body
        if isinstance(node, ast.FunctionDef) and node.name == "__getitem__"
    )
    assert [argument.arg for argument in method.args.posonlyargs] == ["self", "name"]
    with pytest.raises(TypeError):
        hdfour.open(test_diff.ROOT / "fixtures/F01.hdf").__getitem__(name="i16")


def reference_failure(monkeypatch, shape, error):
    def fail():
        raise error

    dimension = SimpleNamespace(info=lambda: ("row", 0, 0, 0))
    reference = SimpleNamespace(
        info=lambda: ("empty", 1, [0], 22, 0),
        iscoordvar=lambda: False,
        dim=lambda _: dimension,
        get=fail,
        endaccess=lambda: None,
    )
    dataset = SimpleNamespace(
        name="empty",
        shape=[0],
        is_coordinate=False,
        attrs={},
        dims=[SimpleNamespace(name="row", size=0, unlimited=True, attrs={})],
        read=lambda: np.empty(shape, dtype=np.int16),
    )
    monkeypatch.setattr(test_diff, "compare_attrs", lambda *_: None)
    monkeypatch.setattr(
        test_diff,
        "SD",
        lambda *_: SimpleNamespace(select=lambda _: reference, end=lambda: None),
    )
    monkeypatch.setattr(hdfour, "open", lambda _: SimpleNamespace(datasets=[dataset]))


def test_diff_rejects_wrong_shape_before_reference_failure(monkeypatch):
    reference_failure(monkeypatch, (0, 3), ValueError("unexpected failure"))
    with pytest.raises(AssertionError):
        test_diff.test_diff_dataset(Path("empty.hdf"), 0, "values")


def test_diff_reraises_unrelated_empty_reference_failure(monkeypatch):
    reference_failure(monkeypatch, (0,), HDF4Error("unexpected failure"))
    with pytest.raises(HDF4Error, match="unexpected failure"):
        test_diff.test_diff_dataset(Path("empty.hdf"), 0, "values")


def test_diff_reports_known_empty_reference_failure_as_skip(monkeypatch):
    reference_failure(monkeypatch, (0,), ValueError("SDreaddata failure"))
    with pytest.raises(pytest.skip.Exception, match="empty dataset"):
        test_diff.test_diff_dataset(Path("empty.hdf"), 0, "values")
