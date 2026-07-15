"""Tests for the ``yggdryl.types`` null value layer: ``NullScalar`` (the one null value) and
``NullSerie`` (a run of nulls stored as just its length), over ``yggdryl_core::io::fixed``."""

import pickle

import pytest

import yggdryl
from yggdryl.types import DataType, NullScalar, NullSerie


def test_module_surface():
    for cls in (NullScalar, NullSerie):
        assert cls.__module__ == "yggdryl.types"
        assert hasattr(yggdryl.types, cls.__name__)


def test_null_scalar():
    s = NullScalar()
    assert s.is_null and not s.is_valid() and s.value is None
    assert s.type_name == "null" and s.data_type == DataType.null()
    assert s == NullScalar() and s == NullScalar.null()
    assert hash(s) == hash(NullScalar())
    assert {s: "x"}[NullScalar()] == "x"  # hashable dict key
    assert s.serialize_bytes() == b""
    assert NullScalar.deserialize_bytes(s.serialize_bytes()) == s
    assert pickle.loads(pickle.dumps(s)) == s
    assert repr(s) == "NullScalar()"
    assert s.field("n").type_name == "null" and s.field("n").nullable is True
    assert s.to_serie() == NullSerie(1)


def test_null_serie():
    col = NullSerie(3)
    assert len(col) == 3 and col.null_count == 3 and col.has_nulls
    col.push()
    col.extend(2)
    assert len(col) == 6
    assert col[0] is None and col[-1] is None
    assert list(col) == [None] * 6
    with pytest.raises(IndexError):
        col[6]
    assert col.get_scalar(0) == NullScalar()
    with pytest.raises(IndexError):
        col.get_scalar(99)

    assert NullSerie().is_empty() and not NullSerie()
    assert col.data_type == DataType.null()
    assert col.to_field("x").nullable is True and col.to_field("x").type_name == "null"


def test_null_serie_equality_codec_and_mutability():
    assert NullSerie(2) == NullSerie(2)
    assert NullSerie() != NullSerie(1)

    col = NullSerie(4)
    assert NullSerie.deserialize_bytes(col.serialize_bytes()) == col
    assert pickle.loads(pickle.dumps(col)) == col

    with pytest.raises(TypeError):
        hash(col)  # mutable -> unhashable

    dup = col.copy()
    dup.push()
    assert len(col) == 4 and len(dup) == 5
