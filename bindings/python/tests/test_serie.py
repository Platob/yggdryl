"""Tests for the yggdryl Python extension's Serie (Arrow-backed column).

Run after building the module, e.g. ``maturin develop`` then ``pytest``.
"""

import copy
import pickle

import pytest

import yggdryl


def test_from_values_infers_type_and_reads():
    s = yggdryl.Serie("n", [1, None, 3])
    assert s.name == "n"
    assert len(s) == 3
    assert s.num_rows == 3
    assert s.null_count == 1
    assert str(s.data_type) == "int64"
    assert s.category == "primitive"
    assert s[0] == 1
    assert s[1] is None
    assert s[-1] == 3            # negative index
    assert s.to_list() == [1, None, 3]
    assert s.is_null(1)
    assert s.is_valid(0)
    with pytest.raises(IndexError):
        _ = s[5]


def test_infers_each_scalar_kind():
    assert str(yggdryl.Serie("b", [True, False]).data_type) == "bool"
    assert str(yggdryl.Serie("f", [1.5, 2.5]).data_type) == "float64"
    assert str(yggdryl.Serie("s", ["a", "b"]).data_type) == "utf8"
    assert yggdryl.Serie("s", ["a", None]).to_list() == ["a", None]
    blob = yggdryl.Serie("raw", [b"xy", None])
    assert blob.to_list()[0] == b"xy"
    # bool is checked before int (Python bool subclasses int), so a bool list with a
    # null stays bool rather than being read as int.
    b = yggdryl.Serie("b", [True, None, False])
    assert str(b.data_type) == "bool"
    assert b.to_list() == [True, None, False]


def test_dtype_argument_casts():
    s = yggdryl.Serie("n", [1, 2, 3], dtype="int32")
    assert str(s.data_type) == "int32"
    # a DataType instance works too
    s2 = yggdryl.Serie("n", [1, 2], dtype=yggdryl.DataType.from_str("float64"))
    assert str(s2.data_type) == "float64"
    assert s2[0] == 1.0
    # all-null needs an explicit dtype
    nulls = yggdryl.Serie("n", [None, None], dtype="int16")
    assert str(nulls.data_type) == "int16"
    assert nulls.null_count == 2
    with pytest.raises(Exception):
        yggdryl.Serie("n", [None, None])


def test_slice_head_resize():
    s = yggdryl.Serie("n", [10, 20, 30, 40])
    assert s.slice(1, 2).to_list() == [20, 30]
    assert s.head(2).to_list() == [10, 20]
    grown = s.resize(6)             # nullable grows with nulls
    assert grown.to_list() == [10, 20, 30, 40, None, None]
    assert s.resize(2).to_list() == [10, 20]


def test_cast_and_categorical():
    s = yggdryl.Serie("n", [1, 2, 3])
    wide = s.cast("float64")
    assert str(wide.data_type) == "float64"
    assert wide[0] == 1.0
    cat = yggdryl.Serie("c", ["a", "b", "a", "a"]).categorical()
    assert cat.is_materialized is False
    assert cat.to_list() == ["a", "b", "a", "a"]
    assert cat.materialize().is_materialized is True


def test_lazy_range_and_index():
    r = yggdryl.Serie.range(5)
    assert r.is_materialized is False
    assert r.to_list() == [0, 1, 2, 3, 4]
    r2 = yggdryl.Serie.range(3, start=10, step=5)
    assert r2.to_list() == [10, 15, 20]
    idx = yggdryl.Serie.index(4)
    assert idx.to_list() == [0, 1, 2, 3]


def test_nested_struct_and_select():
    a = yggdryl.Serie("a", [1, 2])
    b = yggdryl.Serie("b", ["x", "y"])
    rec = yggdryl.Serie.struct("rec", [a, b])
    assert rec.category == "nested"
    assert rec.children()[0].name == "a"
    assert rec.child("b").to_list() == ["x", "y"]
    assert rec.child(0).name == "a"
    assert rec.select("a")[1] == 2
    assert rec.select("missing") is None
    with pytest.raises(Exception):
        rec.select("a.")              # malformed path raises


def test_display_repr():
    s = yggdryl.Serie("n", list(range(100)))
    text = s.display(max_rows=3)
    assert "n: int64" in text
    assert "97 more rows" in text
    assert "Serie('n'" in repr(s)


def test_bytes_roundtrip_and_pickle():
    s = yggdryl.Serie("n", [1, None, 3])
    back = yggdryl.Serie.from_bytes(s.to_bytes())
    assert back.to_list() == [1, None, 3]
    assert bytes(s) == s.to_bytes()
    # pickle / copy round-trip through the IPC bytes (nested too)
    assert pickle.loads(pickle.dumps(s)).to_list() == [1, None, 3]
    rec = yggdryl.Serie.struct("rec", [yggdryl.Serie("a", [1, 2])])
    assert copy.copy(rec).select("a").to_list() == [1, 2]


def test_eq_and_hash():
    a = yggdryl.Serie("n", [1, 2, 3])
    b = yggdryl.Serie("n", [1, 2, 3])
    c = yggdryl.Serie("n", [1, 2, 4])
    assert a == b
    assert a != c
    assert hash(a) == hash(b)
    assert len({a, b}) == 1          # hashable, equal series collapse
