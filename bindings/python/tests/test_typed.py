"""Tests for the ``yggdryl.typed`` typed-column surface.

Mirrors ``crates/yggdryl-core/src/typed`` on the Python surface: a ``Serie`` (a typed
column built ``from_values`` / ``from_options``, with the null-aware ``get`` / ``to_list`` /
``is_null`` / ``is_valid`` / ``null_count``, the raw ``values``, the vectorized reductions
``sum`` / ``min`` / ``max`` / ``mean``, ``with_name`` / ``field`` / ``dtype`` / ``filter``)
and its ``Field`` (``name`` / ``dtype`` / ``nullable`` / ``headers``). The ``docs/typed.md``
Python examples are reproduced verbatim, then the edge cases (empty / all-null / out-of-range,
the wide 128-bit types, float NaN reductions, and the non-reducible bool column).
"""

import math

import pytest

import yggdryl.typed
from yggdryl.datatype_id import DataTypeId
from yggdryl.headers import Headers
from yggdryl.typed import Field, Serie


def test_module_surface():
    assert Serie.__module__ == "yggdryl.typed"
    assert Field.__module__ == "yggdryl.typed"
    assert hasattr(yggdryl.typed, "Serie")
    assert hasattr(yggdryl.typed, "Field")


# -------------------------------------------------------------------------------------
# docs/typed.md — "Build a column and reduce it"
# -------------------------------------------------------------------------------------


def test_doc_build_and_reduce():
    col = Serie.from_values([4, 8, 15, 16, 23, 42], DataTypeId.I64)
    assert col.len() == 6
    assert len(col) == 6  # __len__
    assert col.get(0) == 4
    assert col.to_list() == [4, 8, 15, 16, 23, 42]
    assert col.sum() == 108  # vectorized reduction over the data buffer
    assert col.min() == 4 and col.max() == 42
    assert col.mean() == 18.0


# -------------------------------------------------------------------------------------
# docs/typed.md — "Nulls — a nullable column"
# -------------------------------------------------------------------------------------


def test_doc_nulls():
    col = Serie.from_options([1, None, 3, None, 5], DataTypeId.I32)
    assert col.len() == 5
    assert col.null_count() == 2
    assert col.get(0) == 1
    assert col.get(1) is None  # the null
    assert col.is_null(1) and col.is_valid(0)
    assert col.to_list() == [1, None, 3, None, 5]


# -------------------------------------------------------------------------------------
# docs/typed.md — "A column's Field — its metadata"
# -------------------------------------------------------------------------------------


def test_doc_field():
    field = Field("price", DataTypeId.I64, nullable=True)
    assert field.name() == "price"
    assert field.dtype() == DataTypeId.I64
    assert field.nullable()

    col = Serie.from_values([1, 2, 3], DataTypeId.I64).with_name("id")
    assert col.field().name() == "id"
    assert col.field().nullable() is False  # no nulls -> non-nullable


# -------------------------------------------------------------------------------------
# Field — extra surface
# -------------------------------------------------------------------------------------


def test_field_defaults_and_none_name():
    f = Field(dtype=DataTypeId.F64)
    assert f.name() is None
    assert f.dtype() == DataTypeId.F64
    assert f.nullable() is False  # default non-nullable


def test_field_requires_dtype():
    with pytest.raises(TypeError):
        Field("x")


def test_field_str_dtype():
    f = Field("age", "u32", nullable=True)
    assert f.dtype() == DataTypeId.U32
    assert f.nullable()


def test_field_headers_and_value_semantics():
    f = Field("price", DataTypeId.I64, nullable=True)
    headers = f.headers()
    assert isinstance(headers, Headers)
    assert headers.name() == "price"
    assert headers.type_id() == DataTypeId.I64
    assert headers.nullable() is True

    assert f == Field("price", DataTypeId.I64, nullable=True)
    assert f != Field("price", DataTypeId.I64, nullable=False)
    # Immutable value -> hashable, usable as a set / dict key.
    assert hash(f) == hash(Field("price", DataTypeId.I64, nullable=True))
    assert {f, Field("price", DataTypeId.I64, nullable=True)} == {f}
    assert "price" in repr(f)


# -------------------------------------------------------------------------------------
# Serie — dtype / field / repr / str dtype
# -------------------------------------------------------------------------------------


def test_dtype_and_repr():
    col = Serie.from_values([1, 2, 3], DataTypeId.I16)
    assert col.dtype() == DataTypeId.I16
    r = repr(col)
    assert "Serie(" in r and "i16" in r and "len=3" in r


def test_from_values_str_dtype():
    col = Serie.from_values([1, 2, 3, 4], "i64")
    assert col.dtype() == DataTypeId.I64
    assert col.sum() == 10


def test_unknown_dtype_raises():
    with pytest.raises(ValueError):
        Serie.from_values([1, 2, 3], DataTypeId.Unknown)
    with pytest.raises(ValueError):
        Serie.from_values([1, 2, 3], "nope")


# -------------------------------------------------------------------------------------
# Edges: empty column
# -------------------------------------------------------------------------------------


def test_empty_serie():
    col = Serie.from_values([], DataTypeId.I64)
    assert col.len() == 0
    assert col.is_empty()
    assert not col  # __bool__
    assert col.to_list() == []
    assert col.values() == []
    assert col.get(0) is None
    assert col.null_count() == 0
    # An empty reduction: sum is the zero accumulator, min/max/mean are None.
    assert col.sum() == 0
    assert col.min() is None
    assert col.max() is None
    assert col.mean() is None


# -------------------------------------------------------------------------------------
# Edges: all-null + out-of-range + raw values
# -------------------------------------------------------------------------------------


def test_all_null():
    col = Serie.from_options([None, None, None], DataTypeId.I64)
    assert col.len() == 3
    assert col.null_count() == 3
    assert col.to_list() == [None, None, None]
    assert col.get(0) is None
    assert col.is_null(0) and not col.is_valid(0)


def test_get_out_of_range_is_none():
    col = Serie.from_values([1, 2, 3], DataTypeId.I32)
    assert col.get(3) is None
    assert col.get(100) is None
    assert col.is_valid(100) is False


def test_values_ignores_validity():
    # A null slot stores its default (0), which the raw `values` surfaces.
    col = Serie.from_options([10, None, 30], DataTypeId.I32)
    assert col.values() == [10, 0, 30]
    assert col.to_list() == [10, None, 30]


# -------------------------------------------------------------------------------------
# Edges: the wide 128-bit types
# -------------------------------------------------------------------------------------


def test_u128_wide():
    big = 2**120
    col = Serie.from_values([big, big + 1], DataTypeId.U128)
    assert col.dtype() == DataTypeId.U128
    assert col.get(0) == big
    assert col.sum() == big + big + 1
    assert col.max() == big + 1


def test_i128_wide():
    neg = -(2**120)
    col = Serie.from_values([neg, 0, -neg], DataTypeId.I128)
    assert col.get(0) == neg
    assert col.min() == neg
    assert col.max() == -neg
    assert col.sum() == 0


# -------------------------------------------------------------------------------------
# Edges: floats + NaN-safe min/max
# -------------------------------------------------------------------------------------


def test_float_nan_min_max():
    col = Serie.from_values([1.0, float("nan"), 3.0, 2.0], DataTypeId.F64)
    # min / max ignore NaN.
    assert col.min() == 1.0
    assert col.max() == 3.0
    # The NaN still round-trips through the buffer.
    values = col.to_list()
    assert values[0] == 1.0 and math.isnan(values[1])


def test_float32_column():
    col = Serie.from_values([0.5, 1.5, 2.0], DataTypeId.F32)
    assert col.dtype() == DataTypeId.F32
    assert col.sum() == 4.0
    assert col.mean() == pytest.approx(4.0 / 3.0)


# -------------------------------------------------------------------------------------
# Edges: a bool column (and that a bool reduction raises)
# -------------------------------------------------------------------------------------


def test_bool_column():
    col = Serie.from_options([True, None, False], DataTypeId.Bool)
    assert col.dtype() == DataTypeId.Bool
    assert col.len() == 3
    assert col.get(0) is True
    assert col.get(1) is None
    assert col.get(2) is False
    assert col.is_null(1)
    assert col.to_list() == [True, None, False]


def test_bool_reduction_raises():
    col = Serie.from_values([True, False, True], DataTypeId.Bool)
    for reduce in ("sum", "min", "max", "mean"):
        with pytest.raises(TypeError):
            getattr(col, reduce)()


# -------------------------------------------------------------------------------------
# filter — by a bool list and by a bool Serie
# -------------------------------------------------------------------------------------


def test_filter_by_list():
    col = Serie.from_values([1, 2, 3, 4], DataTypeId.I64)
    kept = col.filter([True, False, True, False])
    assert kept.to_list() == [1, 3]
    assert kept.dtype() == DataTypeId.I64


def test_filter_by_bool_serie():
    col = Serie.from_values([10, 20, 30, 40], DataTypeId.I32)
    mask = Serie.from_values([False, True, True, False], DataTypeId.Bool)
    assert col.filter(mask).to_list() == [20, 30]


def test_filter_preserves_nulls():
    col = Serie.from_options([1, None, 3, None], DataTypeId.I64)
    kept = col.filter([True, True, False, True])
    assert kept.to_list() == [1, None, None]
    assert kept.null_count() == 2
