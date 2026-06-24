"""Tests for the yggdryl Python extension.

Run after building the module, e.g. ``maturin develop`` then ``pytest``.
"""

import pytest

import yggdryl


def sample() -> "yggdryl.Tree":
    tree = yggdryl.Tree()
    tree.insert("roots/urdr", 1.0)
    tree.insert("roots/verdandi", 2.0)
    tree.insert("roots/skuld", 3.0)
    return tree


def test_insert_and_get():
    tree = sample()
    assert tree.get("roots/urdr") == 1.0
    assert tree.get("roots/missing") is None


def test_insert_returns_previous():
    tree = yggdryl.Tree()
    assert tree.insert("a", 1.0) is None
    assert tree.insert("a", 2.0) == 1.0


def test_count_sum_depth():
    tree = sample()
    assert tree.count() == 4
    assert tree.sum() == 6.0
    assert tree.depth() == 2


def test_dunders():
    tree = sample()
    assert len(tree) == 4
    assert "roots/urdr" in tree
    assert "nope" not in tree
    assert repr(tree).startswith("Tree(")


def test_leaves_sorted():
    assert sample().leaves() == [
        ("roots/skuld", 3.0),
        ("roots/urdr", 1.0),
        ("roots/verdandi", 2.0),
    ]


def test_remove():
    tree = sample()
    assert tree.remove("roots/urdr") == 1.0
    assert tree.get("roots/urdr") is None
    with pytest.raises(KeyError):
        tree.remove("roots/urdr")


def test_empty_path_raises():
    tree = yggdryl.Tree()
    with pytest.raises(ValueError):
        tree.insert("", 1.0)


def test_arrow_ipc_round_trip():
    tree = sample()
    data = tree.to_arrow_ipc()
    assert isinstance(data, bytes) and len(data) > 0
    restored = yggdryl.Tree.from_arrow_ipc(data)
    assert restored.leaves() == tree.leaves()


def test_arrow_ipc_readable_by_pyarrow():
    pa = pytest.importorskip("pyarrow")
    batch = pa.ipc.open_stream(sample().to_arrow_ipc()).read_all()
    assert batch.num_rows == 3
    assert batch.schema.names == ["path", "value"]


def test_version():
    assert isinstance(yggdryl.__version__, str)
