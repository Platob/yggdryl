"""Tests for the ``yggdryl.decimal`` columnar types: ``D*Scalar`` (a decimal value carrying its
column ``(precision, scale)``) and ``D*Serie`` (a nullable decimal column), over
``yggdryl_core::io::fixed``'s ``DecimalScalar`` / ``DecimalSerie``. Values cross as decimal strings.
"""

import copy
import pickle

import pytest

import yggdryl
from yggdryl.decimal import (
    D32Scalar,
    D32Serie,
    D64Scalar,
    D64Serie,
    D128Scalar,
    D128Serie,
    D256Scalar,
    D256Serie,
)


def test_module_surface():
    for cls in (D32Scalar, D32Serie, D256Scalar, D256Serie):
        assert cls.__module__ == "yggdryl.decimal"
        assert hasattr(yggdryl.decimal, cls.__name__)


# ---------------------------------------------------------------------------------------
# Scalar
# ---------------------------------------------------------------------------------------


def test_scalar_infers_or_pins_precision_scale():
    inferred = D128Scalar("123.45")
    assert inferred.value == "123.45" and inferred.precision == 5 and inferred.scale == 2

    pinned = D128Scalar("123.45", 20, 2)
    assert pinned.value == "123.45" and pinned.precision == 20 and pinned.scale == 2

    for null in (D128Scalar(), D128Scalar(None), D128Scalar.null(10, 2)):
        assert null.is_null and null.value is None


def test_scalar_value_identity_across_scale():
    # 2.5 and 2.50 are the same value (different scale) -> equal and hash-equal.
    assert D128Scalar("2.5", 5, 1) == D128Scalar("2.50", 5, 2)
    assert hash(D128Scalar("2.5", 5, 1)) == hash(D128Scalar("2.50", 5, 2))
    assert D128Scalar("2.5", 5, 1) != D128Scalar("2.75", 5, 2)
    assert {D128Scalar("2.5", 5, 1): "x"}[D128Scalar("2.50", 5, 2)] == "x"  # dict key


def test_scalar_precision_overflow_is_guided():
    with pytest.raises(ValueError):
        D128Scalar("1.234", 5, 2)  # needs scale 3, does not fit scale 2


@pytest.mark.parametrize(
    "cls", [D32Scalar, D64Scalar, D128Scalar, D256Scalar]
)
def test_scalar_codec_pickle_copy(cls):
    s = cls("12.34", 10, 2)
    assert cls.deserialize_bytes(s.serialize_bytes()) == s
    assert cls.deserialize_bytes(cls.null(10, 2).serialize_bytes()) == cls.null(10, 2)
    assert pickle.loads(pickle.dumps(s)) == s
    assert copy.deepcopy(s) == s


# ---------------------------------------------------------------------------------------
# Serie
# ---------------------------------------------------------------------------------------


def test_serie_construction_and_access():
    col = D128Serie(20, 2, ["123.45", None, "6"])
    assert len(col) == 3 and col.null_count == 1 and col.has_nulls
    assert col.precision == 20 and col.scale == 2
    assert col.get(0) == "123.45" and col.get(1) is None
    assert col.to_options() == ["123.45", None, "6.00"]  # re-expressed at scale 2
    assert list(col) == ["123.45", None, "6.00"]
    assert col[0] == "123.45" and col[-1] == "6.00"
    with pytest.raises(IndexError):
        col[3]

    dense = D128Serie.from_values(10, 2, ["1", "2"])
    assert dense.null_count == 0 and dense.to_options() == ["1.00", "2.00"]
    assert D128Serie(10, 2).is_empty()


def test_serie_mutation_and_fit():
    col = D64Serie(10, 2, ["1.00", None])
    col.push("3")
    col.set(1, "2.50")
    assert col.to_options() == ["1.00", "2.50", "3.00"]
    assert col.get_scalar(0) == D64Scalar("1.00", 10, 2)
    with pytest.raises(ValueError):
        col.set(0, "1.234")  # does not fit scale 2
    with pytest.raises(ValueError):
        col.set(99, "0")  # out of range


def test_serie_codec_pickle_and_mutability():
    col = D128Serie(20, 2, ["123.45", None, "6"])
    col.set(1, "0.01")  # clears the last null -> still round-trips byte-equal
    assert D128Serie.deserialize_bytes(col.serialize_bytes()) == col
    assert pickle.loads(pickle.dumps(col)) == col
    assert copy.deepcopy(col) == col
    with pytest.raises(TypeError):
        hash(col)  # mutable -> unhashable


def test_serie_copy_is_independent():
    original = D256Serie(10, 0, ["1", "2"])
    dup = original.copy()
    dup.push("3")
    assert len(original) == 2 and len(dup) == 3
