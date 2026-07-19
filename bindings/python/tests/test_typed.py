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
from yggdryl.typed import ByteSerie, Field, Serie


def test_module_surface():
    assert Serie.__module__ == "yggdryl.typed"
    assert ByteSerie.__module__ == "yggdryl.typed"
    assert Field.__module__ == "yggdryl.typed"
    assert hasattr(yggdryl.typed, "Serie")
    assert hasattr(yggdryl.typed, "ByteSerie")
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


# -------------------------------------------------------------------------------------
# docs/typed.md — "Fixed-point decimals"
# -------------------------------------------------------------------------------------


def test_doc_decimal_money():
    # Money as Decimal128 scale 2: the stored value is the unscaled integer.
    col = Serie.from_values([12345, 5, -5], DataTypeId.Decimal128).with_precision_scale(10, 2)
    assert col.get(0) == 12345  # raw unscaled value
    assert col.to_decimal_string(0) == "123.45"  # scale-aware string
    assert col.to_decimal_string(1) == "0.05"
    assert col.to_decimal_string(2) == "-0.05"
    assert col.field().precision() == 10 and col.field().scale() == 2


# -------------------------------------------------------------------------------------
# DataTypeId — the four decimal variants
# -------------------------------------------------------------------------------------


def test_datatype_id_decimal_variants():
    assert int(DataTypeId.Decimal32) == 0x0300
    assert int(DataTypeId.Decimal64) == 0x0301
    assert int(DataTypeId.Decimal128) == 0x0302
    assert int(DataTypeId.Decimal256) == 0x0303
    assert DataTypeId.Decimal128.name() == "decimal128"
    assert DataTypeId.from_name("decimal256") == DataTypeId.Decimal256
    assert DataTypeId.Decimal32.byte_size() == 4
    assert DataTypeId.Decimal256.byte_size() == 32
    assert DataTypeId.Decimal128.is_signed()


# -------------------------------------------------------------------------------------
# Decimals — all four widths
# -------------------------------------------------------------------------------------


def test_decimal_all_widths():
    for dt in (
        DataTypeId.Decimal32,
        DataTypeId.Decimal64,
        DataTypeId.Decimal128,
        DataTypeId.Decimal256,
    ):
        col = Serie.from_values([100, 250], dt).with_precision_scale(5, 2)
        assert col.dtype() == dt
        assert col.get(0) == 100  # raw unscaled value crosses as int
        assert col.to_decimal_string(0) == "1.00"
        assert col.to_decimal_string(1) == "2.50"
        assert col.field().precision() == 5 and col.field().scale() == 2


def test_decimal_str_dtype():
    col = Serie.from_values([100], "decimal32").with_precision_scale(5, 2)
    assert col.dtype() == DataTypeId.Decimal32
    assert col.to_decimal_string(0) == "1.00"


def test_decimal_default_precision_scale():
    # Before with_precision_scale: scale defaults to 0, precision to the width's max.
    col = Serie.from_values([1, 2], DataTypeId.Decimal64)
    assert col.decimal_scale() == 0
    assert col.decimal_precision() == 18  # Decimal64 max precision
    assert col.to_decimal_string(0) == "1"  # scale 0 -> the integer itself


# -------------------------------------------------------------------------------------
# Decimal256 — the 256-bit width (native + beyond i128)
# -------------------------------------------------------------------------------------


def test_decimal256_fits_i128():
    col = Serie.from_values([42, -42], DataTypeId.Decimal256)
    assert col.dtype() == DataTypeId.Decimal256
    assert col.get(0) == 42
    assert col.get(1) == -42
    assert col.to_decimal_string(0) == "42"


def test_decimal256_beyond_i128():
    # A value larger than i128 (2**127 - 1) round-trips through the 32 two's-complement bytes.
    big = 2**200 + 123
    col = Serie.from_values([big, -big], DataTypeId.Decimal256)
    assert col.get(0) == big  # arbitrary-precision Python int
    assert col.get(1) == -big
    assert col.to_decimal_string(0) == str(big)  # scale 0
    assert col.to_decimal_string(1) == str(-big)
    assert col.to_list() == [big, -big]
    # And it still carries precision/scale like any decimal.
    scaled = col.with_precision_scale(76, 5)
    assert scaled.field().precision() == 76 and scaled.field().scale() == 5


def test_decimal256_out_of_range_raises():
    # Beyond 256 bits (2**255 - 1 max) -> guided ValueError.
    with pytest.raises(ValueError):
        Serie.from_values([2**300], DataTypeId.Decimal256)


# -------------------------------------------------------------------------------------
# Decimals — a nullable column
# -------------------------------------------------------------------------------------


def test_decimal_nullable():
    col = Serie.from_options([12345, None, -5], DataTypeId.Decimal128).with_precision_scale(10, 2)
    assert col.len() == 3
    assert col.null_count() == 1
    assert col.get(0) == 12345
    assert col.get(1) is None
    assert col.is_null(1) and col.is_valid(0)
    assert col.to_decimal_string(0) == "123.45"
    assert col.to_decimal_string(1) is None  # the null
    assert col.to_decimal_string(2) == "-0.05"
    assert col.to_list() == [12345, None, -5]
    assert col.field().nullable() is True
    assert col.field().precision() == 10 and col.field().scale() == 2


def test_decimal256_nullable():
    big = 2**200
    col = Serie.from_options([big, None], DataTypeId.Decimal256)
    assert col.null_count() == 1
    assert col.get(0) == big
    assert col.get(1) is None


# -------------------------------------------------------------------------------------
# Decimals — with_name carries precision/scale, non-decimal rejection, no reduction
# -------------------------------------------------------------------------------------


def test_decimal_with_name_preserves_precision_scale():
    col = (
        Serie.from_values([12345], DataTypeId.Decimal128)
        .with_precision_scale(10, 2)
        .with_name("price")
    )
    assert col.field().name() == "price"
    assert col.field().precision() == 10 and col.field().scale() == 2
    assert col.to_decimal_string(0) == "123.45"


def test_decimal_no_reduction():
    col = Serie.from_values([1, 2, 3], DataTypeId.Decimal128)
    for reduce in ("sum", "min", "max", "mean"):
        with pytest.raises(TypeError):
            getattr(col, reduce)()


def test_decimal_methods_reject_non_decimal():
    col = Serie.from_values([1, 2, 3], DataTypeId.I64)
    with pytest.raises(TypeError):
        col.to_decimal_string(0)
    with pytest.raises(TypeError):
        col.decimal_precision()
    with pytest.raises(TypeError):
        col.decimal_scale()
    with pytest.raises(TypeError):
        col.with_precision_scale(10, 2)


def test_field_precision_scale_none_for_non_decimal():
    f = Field("x", DataTypeId.I64)
    assert f.precision() is None
    assert f.scale() is None


# =====================================================================================
# ByteSerie — the variable-length + fixed-size byte columns
# =====================================================================================


def test_bytes_module_surface():
    assert ByteSerie.__module__ == "yggdryl.typed"


# -------------------------------------------------------------------------------------
# A variable-length `binary` column
# -------------------------------------------------------------------------------------


def test_binary_column():
    col = ByteSerie.from_values([b"a", b"bb", b"", b"ccc"], DataTypeId.Binary)
    assert col.len() == 4
    assert len(col) == 4  # __len__
    assert bool(col) is True  # __bool__
    assert col.dtype() == DataTypeId.Binary
    assert col.width() is None  # variable-length -> no fixed width
    assert col.get(0) == b"a"
    assert col.get(2) == b""  # an empty element is not a null
    assert col.get(3) == b"ccc"
    assert col.get(4) is None  # out of range
    assert col.to_list() == [b"a", b"bb", b"", b"ccc"]
    assert col.values() == [b"a", b"bb", b"", b"ccc"]
    assert col.null_count() == 0
    field = col.field()
    assert field.dtype() == DataTypeId.Binary
    assert field.byte_width() is None  # variable-length carries no width


def test_binary_str_dtype():
    col = ByteSerie.from_values([b"x", b"y"], "binary")
    assert col.dtype() == DataTypeId.Binary
    assert col.to_list() == [b"x", b"y"]


# -------------------------------------------------------------------------------------
# A variable-length `utf8` column — multibyte round-trip
# -------------------------------------------------------------------------------------


def test_utf8_column_multibyte():
    col = ByteSerie.from_values(["héllo", "世界", "", "ok"], DataTypeId.Utf8)
    assert col.dtype() == DataTypeId.Utf8
    assert col.get(0) == "héllo"
    assert col.get(1) == "世界"  # multibyte survives the byte round-trip
    assert col.get(2) == ""
    assert col.to_list() == ["héllo", "世界", "", "ok"]
    assert col.values() == ["héllo", "世界", "", "ok"]
    assert col.width() is None


def test_utf8_str_dtype():
    col = ByteSerie.from_values(["a", "béta"], "utf8")
    assert col.dtype() == DataTypeId.Utf8
    assert col.to_list() == ["a", "béta"]


# -------------------------------------------------------------------------------------
# A nullable column via from_options
# -------------------------------------------------------------------------------------


def test_binary_nullable():
    col = ByteSerie.from_options([b"a", None, b"ccc"], DataTypeId.Binary)
    assert col.len() == 3
    assert col.null_count() == 1
    assert col.get(0) == b"a"
    assert col.get(1) is None  # the null
    assert col.is_null(1) and col.is_valid(0)
    assert not col.is_valid(1)
    assert col.to_list() == [b"a", None, b"ccc"]


def test_utf8_nullable():
    col = ByteSerie.from_options(["x", None, "z"], DataTypeId.Utf8)
    assert col.null_count() == 1
    assert col.get(1) is None
    assert col.to_list() == ["x", None, "z"]


# -------------------------------------------------------------------------------------
# A fixed_binary column — zero-pad + truncation at the fixed width
# -------------------------------------------------------------------------------------


def test_fixed_binary_width():
    # width=4: "ab" zero-pads to 4 bytes, "abcdef" truncates to the first 4.
    col = ByteSerie.from_values([b"ab", b"abcd", b"abcdef"], DataTypeId.FixedBinary, width=4)
    assert col.dtype() == DataTypeId.FixedBinary
    assert col.width() == 4
    assert col.get(0) == b"ab\x00\x00"  # zero-padded to the width
    assert col.get(1) == b"abcd"  # exact fit
    assert col.get(2) == b"abcd"  # truncated to the width
    assert col.len() == 3
    field = col.field()
    assert field.dtype() == DataTypeId.FixedBinary
    assert field.byte_width() == 4


def test_fixed_binary_str_dtype():
    col = ByteSerie.from_values([b"ab"], "fixed_binary", width=2)
    assert col.dtype() == DataTypeId.FixedBinary
    assert col.width() == 2
    assert col.get(0) == b"ab"


# -------------------------------------------------------------------------------------
# A fixed_utf8 nullable column
# -------------------------------------------------------------------------------------


def test_fixed_utf8_nullable():
    col = ByteSerie.from_options(["ab", None, "cd"], DataTypeId.FixedUtf8, width=2)
    assert col.dtype() == DataTypeId.FixedUtf8
    assert col.width() == 2
    assert col.len() == 3
    assert col.null_count() == 1
    assert col.get(0) == "ab"
    assert col.get(1) is None  # the null
    assert col.get(2) == "cd"
    assert col.is_null(1) and col.is_valid(0)
    assert col.field().byte_width() == 2
    assert col.field().nullable() is True


# -------------------------------------------------------------------------------------
# with_name — a fresh column sharing the bytes, preserving width
# -------------------------------------------------------------------------------------


def test_bytes_with_name():
    col = ByteSerie.from_values([b"a", b"bb"], DataTypeId.Binary).with_name("blob")
    assert col.field().name() == "blob"
    assert col.to_list() == [b"a", b"bb"]  # bytes preserved


def test_fixed_bytes_with_name_preserves_width():
    col = ByteSerie.from_values(["ab", "cd"], DataTypeId.FixedUtf8, width=2).with_name("code")
    assert col.field().name() == "code"
    assert col.width() == 2  # width carried over
    assert col.field().byte_width() == 2
    assert col.to_list() == ["ab", "cd"]


# -------------------------------------------------------------------------------------
# repr
# -------------------------------------------------------------------------------------


def test_bytes_repr():
    col = ByteSerie.from_values([b"a", b"bb"], DataTypeId.Binary)
    r = repr(col)
    assert "ByteSerie(" in r and "binary" in r and "len=2" in r

    fixed = ByteSerie.from_values([b"ab"], DataTypeId.FixedBinary, width=4).with_name("k")
    fr = repr(fixed)
    assert "fixed_binary" in fr and "width=4" in fr and 'name="k"' in fr


# -------------------------------------------------------------------------------------
# Empty column
# -------------------------------------------------------------------------------------


def test_bytes_empty():
    col = ByteSerie.from_values([], DataTypeId.Binary)
    assert col.len() == 0
    assert col.is_empty()
    assert not col  # __bool__
    assert col.to_list() == []
    assert col.values() == []
    assert col.get(0) is None
    assert col.null_count() == 0


# -------------------------------------------------------------------------------------
# Guided errors — width missing / width given / non-byte dtype
# -------------------------------------------------------------------------------------


def test_fixed_requires_width():
    with pytest.raises(ValueError):
        ByteSerie.from_values([b"ab"], DataTypeId.FixedBinary)
    with pytest.raises(ValueError):
        ByteSerie.from_options([b"ab", None], DataTypeId.FixedUtf8)


def test_variable_rejects_width():
    with pytest.raises(ValueError):
        ByteSerie.from_values([b"ab"], DataTypeId.Binary, width=4)
    with pytest.raises(ValueError):
        ByteSerie.from_options(["ab", None], DataTypeId.Utf8, width=2)


def test_non_byte_dtype_rejected():
    for dt in (DataTypeId.I64, DataTypeId.Bool, DataTypeId.Decimal128, DataTypeId.Unknown):
        with pytest.raises(ValueError):
            ByteSerie.from_values([b"ab"], dt)


# =====================================================================================
# Serie — the extended numeric reductions (std / var / median / count_ge) and the
# universal aggregations (count / valid_count / n_unique / first_value / last_value)
# =====================================================================================


def test_std_var_median():
    col = Serie.from_values([2, 4, 4, 4, 5, 5, 7, 9], DataTypeId.I64)
    # mean is 5.0; population variance = 32 / 8 = 4.0, std = sqrt(4) = 2.0.
    assert col.var() == 4.0
    assert col.std() == 2.0
    # An even count -> the average of the two middle order statistics (4 and 5).
    assert col.median() == 4.5


def test_count_ge():
    col = Serie.from_values([2, 4, 4, 4, 5, 5, 7, 9], DataTypeId.I64)
    assert col.count_ge(5) == 4  # 5, 5, 7, 9 are >= 5
    assert col.count_ge(4) == 7  # everything but the leading 2
    assert col.count_ge(2) == 8  # all
    assert col.count_ge(10) == 0  # none reach 10


def test_count_ge_float():
    col = Serie.from_values([0.5, 1.5, 2.0, 2.5], DataTypeId.F64)
    assert col.count_ge(2.0) == 2  # 2.0 and 2.5


def test_std_var_median_empty_is_none():
    col = Serie.from_values([], DataTypeId.I64)
    assert col.std() is None
    assert col.var() is None
    assert col.median() is None


def test_universal_aggregations_numeric():
    col = Serie.from_values([2, 4, 4, 4, 5, 5, 7, 9], DataTypeId.I64)
    assert col.count() == 8  # total, nulls included
    assert col.valid_count() == 8  # no nulls here
    assert col.n_unique() == 5  # distinct values {2, 4, 5, 7, 9}
    assert col.first_value() == 2
    assert col.last_value() == 9


def test_universal_aggregations_nullable():
    col = Serie.from_options([1, None, 3, None, 5, 5], DataTypeId.I64)
    assert col.count() == 6  # total, nulls included
    assert col.valid_count() == 4  # nulls excluded (1, 3, 5, 5)
    assert col.n_unique() == 3  # distinct non-null {1, 3, 5}
    assert col.first_value() == 1
    assert col.last_value() == 5


def test_first_last_value_null_slot():
    # first_value / last_value are null-aware: a null at the edge reads as None.
    col = Serie.from_options([None, 2, 3, None], DataTypeId.I64)
    assert col.first_value() is None  # index 0 is null
    assert col.last_value() is None  # last index is null
    empty = Serie.from_values([], DataTypeId.I64)
    assert empty.first_value() is None and empty.last_value() is None


def test_n_unique_float():
    # A float column has no core `n_unique` (f64 is not Eq/Hash) -> distinct bit patterns.
    col = Serie.from_values([1.0, 2.0, 2.0, 3.0], DataTypeId.F64)
    assert col.n_unique() == 3
    assert col.count() == 4
    assert col.first_value() == 1.0
    assert col.last_value() == 3.0


def test_extended_reductions_reject_bool():
    col = Serie.from_values([True, False, True], DataTypeId.Bool)
    for reduce in ("std", "var", "median"):
        with pytest.raises(TypeError):
            getattr(col, reduce)()
    with pytest.raises(TypeError):
        col.count_ge(1)
    # The universal aggregations still work on a bool column (no TypeError).
    assert col.count() == 3
    assert col.valid_count() == 3
    assert col.n_unique() == 2  # {True, False}
    assert col.first_value() is True
    assert col.last_value() is True


def test_extended_reductions_reject_decimal():
    col = Serie.from_values([1, 2, 3], DataTypeId.Decimal128)
    for reduce in ("std", "var", "median"):
        with pytest.raises(TypeError):
            getattr(col, reduce)()
    with pytest.raises(TypeError):
        col.count_ge(1)
    # Universal aggregations remain available on a decimal column.
    assert col.count() == 3
    assert col.valid_count() == 3
    assert col.n_unique() == 3


# =====================================================================================
# ByteSerie — the universal aggregations (count / valid_count / n_unique / first_value /
# last_value / min_value / max_value)
# =====================================================================================


def test_bytes_min_max_value_utf8():
    col = ByteSerie.from_values(["banana", "apple", "cherry"], DataTypeId.Utf8)
    assert col.min_value() == "apple"  # lexicographic min
    assert col.max_value() == "cherry"  # lexicographic max


def test_bytes_min_max_value_binary():
    col = ByteSerie.from_values([b"banana", b"apple", b"cherry"], DataTypeId.Binary)
    assert col.min_value() == b"apple"
    assert col.max_value() == b"cherry"


def test_bytes_universal_aggregations():
    col = ByteSerie.from_values(["a", "b", "a", "c"], DataTypeId.Utf8)
    assert col.count() == 4
    assert col.valid_count() == 4
    assert col.n_unique() == 3  # {"a", "b", "c"} — the duplicate "a" counts once
    assert col.first_value() == "a"
    assert col.last_value() == "c"


def test_bytes_universal_aggregations_nullable():
    col = ByteSerie.from_options(
        ["banana", None, "apple", None, "cherry"], DataTypeId.Utf8
    )
    assert col.count() == 5  # nulls included
    assert col.valid_count() == 3  # nulls excluded
    assert col.n_unique() == 3  # distinct non-null
    assert col.min_value() == "apple"  # nulls excluded from the ordering
    assert col.max_value() == "cherry"
    assert col.first_value() == "banana"
    assert col.last_value() == "cherry"


def test_bytes_first_last_value_null_slot():
    col = ByteSerie.from_options([None, "z"], DataTypeId.Utf8)
    assert col.first_value() is None  # index 0 is null
    assert col.last_value() == "z"


def test_bytes_min_max_value_empty_is_none():
    col = ByteSerie.from_values([], DataTypeId.Utf8)
    assert col.min_value() is None
    assert col.max_value() is None
    assert col.first_value() is None
    assert col.last_value() is None
    assert col.n_unique() == 0


# =====================================================================================
# Serie — in-place mutation: set / set_checked / set_null / slice / set_range /
# set_range_serie
# =====================================================================================


def test_set_replaces_element():
    col = Serie.from_values([1, 2, 3, 4], DataTypeId.I64)
    col.set(1, 20)
    assert col.to_list() == [1, 20, 3, 4]
    assert col.get(1) == 20


def test_set_checked_replaces_element():
    col = Serie.from_values([1, 2, 3], DataTypeId.I32)
    col.set_checked(0, 99)  # caller guarantees index < len
    assert col.to_list() == [99, 2, 3]


def test_set_revalidates_null_slot():
    # set on a previously-null slot marks it valid again.
    col = Serie.from_options([1, None, 3], DataTypeId.I64)
    assert col.is_null(1) and col.null_count() == 1
    col.set(1, 22)
    assert col.is_valid(1)
    assert col.get(1) == 22
    assert col.null_count() == 0
    assert col.to_list() == [1, 22, 3]


def test_set_null_nulls_element():
    col = Serie.from_values([10, 20, 30], DataTypeId.I64)
    assert col.null_count() == 0
    col.set_null(1)
    assert col.is_null(1)
    assert col.get(1) is None
    assert col.null_count() == 1
    assert col.to_list() == [10, None, 30]


def test_set_out_of_range_raises():
    col = Serie.from_values([1, 2, 3], DataTypeId.I64)
    with pytest.raises(ValueError):
        col.set(3, 9)
    with pytest.raises(ValueError):
        col.set(100, 9)
    with pytest.raises(ValueError):
        col.set_null(3)


def test_set_wrong_type_raises():
    # The per-variant conversion is the runtime type check.
    col = Serie.from_values([1, 2, 3], DataTypeId.I64)
    with pytest.raises((TypeError, ValueError)):
        col.set(0, "not a number")


def test_set_decimal_unscaled_int():
    col = Serie.from_values([100, 200], DataTypeId.Decimal128).with_precision_scale(5, 2)
    col.set(0, 12345)  # the unscaled integer, like from_values
    assert col.get(0) == 12345
    assert col.to_decimal_string(0) == "123.45"


def test_set_decimal256_beyond_i128():
    big = 2**200 + 7
    col = Serie.from_values([1, 2], DataTypeId.Decimal256)
    col.set(1, big)
    assert col.get(1) == big


def test_slice_sub_column():
    col = Serie.from_values([1, 2, 3, 4, 5], DataTypeId.I64)
    sub = col.slice(1, 3)
    assert sub.to_list() == [2, 3, 4]
    assert sub.dtype() == DataTypeId.I64
    # The original is untouched.
    assert col.to_list() == [1, 2, 3, 4, 5]


def test_slice_clamps_never_errors():
    col = Serie.from_values([1, 2, 3], DataTypeId.I32)
    assert col.slice(1, 100).to_list() == [2, 3]  # over-long len clamps
    assert col.slice(10, 5).to_list() == []  # out-of-range start -> empty


def test_slice_preserves_nulls():
    col = Serie.from_options([1, None, 3, None, 5], DataTypeId.I64)
    sub = col.slice(1, 3)
    assert sub.to_list() == [None, 3, None]
    assert sub.null_count() == 2


def test_set_range_from_list():
    col = Serie.from_values([0, 0, 0, 0, 0], DataTypeId.I64)
    col.set_range(1, [10, 20, 30])
    assert col.to_list() == [0, 10, 20, 30, 0]


def test_set_range_checked_from_list():
    col = Serie.from_values([1, 1, 1, 1], DataTypeId.I32)
    col.set_range_checked(2, [7, 8])  # caller guarantees the window fits
    assert col.to_list() == [1, 1, 7, 8]


def test_set_range_out_of_range_raises():
    col = Serie.from_values([1, 2, 3], DataTypeId.I64)
    with pytest.raises(ValueError):
        col.set_range(2, [10, 20])  # 2 + 2 > 3


def test_set_range_serie_from_another_serie():
    col = Serie.from_values([0, 0, 0, 0], DataTypeId.I64)
    other = Serie.from_values([7, 8], DataTypeId.I64)
    col.set_range_serie(1, other)
    assert col.to_list() == [0, 7, 8, 0]


def test_set_range_serie_carries_nulls():
    col = Serie.from_values([1, 2, 3, 4], DataTypeId.I64)
    other = Serie.from_options([None, 9], DataTypeId.I64)
    col.set_range_serie(2, other)
    assert col.is_null(2)
    assert col.get(3) == 9
    assert col.to_list() == [1, 2, None, 9]


def test_set_range_serie_dtype_mismatch_raises():
    col = Serie.from_values([1, 2, 3], DataTypeId.I64)
    other = Serie.from_values([1, 2], DataTypeId.I32)
    with pytest.raises(ValueError):
        col.set_range_serie(0, other)


# =====================================================================================
# ByteSerie — in-place mutation: slice / set / set_checked
# =====================================================================================


def test_bytes_slice_utf8():
    col = ByteSerie.from_values(["a", "bb", "ccc", "dddd"], DataTypeId.Utf8)
    sub = col.slice(1, 2)
    assert sub.to_list() == ["bb", "ccc"]
    assert sub.dtype() == DataTypeId.Utf8
    assert col.to_list() == ["a", "bb", "ccc", "dddd"]  # original untouched


def test_bytes_slice_fixed_binary_preserves_width():
    col = ByteSerie.from_values([b"aa", b"bb", b"cc", b"dd"], DataTypeId.FixedBinary, width=2)
    sub = col.slice(1, 2)
    assert sub.to_list() == [b"bb", b"cc"]
    assert sub.width() == 2  # width carried over
    assert sub.dtype() == DataTypeId.FixedBinary


def test_bytes_slice_clamps():
    col = ByteSerie.from_values(["x", "y", "z"], DataTypeId.Utf8)
    assert col.slice(1, 100).to_list() == ["y", "z"]
    assert col.slice(10, 5).to_list() == []


def test_fixed_binary_set_zero_pad_and_truncate():
    col = ByteSerie.from_values([b"aa", b"bb", b"cc"], DataTypeId.FixedBinary, width=4)
    col.set(0, b"z")  # zero-pads to the width
    assert col.get(0) == b"z\x00\x00\x00"
    col.set(1, b"abcdef")  # truncates to the width
    assert col.get(1) == b"abcd"
    assert col.get(2) == b"cc\x00\x00"  # untouched


def test_fixed_utf8_set():
    col = ByteSerie.from_values(["ab", "cd"], DataTypeId.FixedUtf8, width=2)
    col.set(1, "zz")
    assert col.get(1) == "zz"


def test_fixed_binary_set_revalidates_null():
    col = ByteSerie.from_options([b"aa", None, b"cc"], DataTypeId.FixedBinary, width=2)
    assert col.is_null(1) and col.null_count() == 1
    col.set(1, b"xy")
    assert col.is_valid(1)
    assert col.get(1) == b"xy"
    assert col.null_count() == 0


def test_fixed_binary_set_out_of_range_raises():
    col = ByteSerie.from_values([b"aa"], DataTypeId.FixedBinary, width=2)
    with pytest.raises(ValueError):
        col.set(1, b"bb")


def test_fixed_binary_set_checked():
    col = ByteSerie.from_values([b"aa", b"bb"], DataTypeId.FixedBinary, width=2)
    col.set_checked(0, b"xy")  # caller guarantees index < len
    assert col.get(0) == b"xy"


def test_variable_binary_set_is_append_only():
    col = ByteSerie.from_values([b"a", b"bb", b"ccc"], DataTypeId.Binary)
    with pytest.raises(ValueError):
        col.set(0, b"x")
    with pytest.raises(ValueError):
        col.set_checked(0, b"x")


def test_variable_utf8_set_is_append_only():
    col = ByteSerie.from_values(["a", "bb"], DataTypeId.Utf8)
    with pytest.raises(ValueError):
        col.set(0, "x")


# =====================================================================================
# Field — metadata accessors / mutators
# =====================================================================================


def test_field_metadata_round_trip():
    f = Field("price", DataTypeId.I64, nullable=True)
    assert f.metadata("unit") is None  # no annotation yet
    f.set_metadata("unit", "usd")
    assert f.metadata("unit") == "usd"  # set / get round-trip
    # with_metadata returns a fresh field, leaving the original untouched.
    g = f.with_metadata("currency", "USD")
    assert g.metadata("currency") == "USD"
    assert g.metadata("unit") == "usd"  # carried over from the clone
    assert f.metadata("currency") is None  # original untouched


def test_field_set_name_and_nullable():
    f = Field("a", DataTypeId.I32, nullable=False)
    f.set_name("b")
    assert f.name() == "b"  # set_name reflected
    f.set_nullable(True)
    assert f.nullable() is True  # set_nullable reflected
    # with_nullable is the clone-with-override front door.
    g = f.with_nullable(False)
    assert g.nullable() is False
    assert f.nullable() is True  # original untouched


# =====================================================================================
# Serie.cast_field — same-dtype metadata reshape + fixed-width dtype change
# =====================================================================================


def test_cast_field_same_dtype_adds_nullability():
    col = Serie.from_values([1, 2, 3], DataTypeId.I64)  # non-nullable
    assert col.field().nullable() is False
    out = col.cast_field(Field("x", DataTypeId.I64, nullable=True))
    assert out.dtype() == DataTypeId.I64
    assert out.field().nullable() is True  # became nullable
    assert out.field().name() == "x"
    assert out.to_list() == [1, 2, 3]  # values intact
    assert out.null_count() == 0  # all valid


def test_cast_field_same_dtype_name_and_metadata():
    col = Serie.from_values([10, 20], DataTypeId.I32)
    target = Field("amount", DataTypeId.I32).with_metadata("unit", "usd")
    out = col.cast_field(target)
    assert out.field().name() == "amount"
    assert out.field().metadata("unit") == "usd"  # annotation carried through
    assert out.to_list() == [10, 20]


def test_cast_field_nullable_with_nulls_to_nonnullable_raises():
    col = Serie.from_options([1, None, 3], DataTypeId.I64)  # a null present
    with pytest.raises(ValueError):
        col.cast_field(Field("x", DataTypeId.I64, nullable=False))


def test_cast_field_widen_i32_to_i64():
    col = Serie.from_values([1, 2, 3], DataTypeId.I32)
    out = col.cast_field(Field("x", DataTypeId.I64))
    assert out.dtype() == DataTypeId.I64  # widened
    assert out.to_list() == [1, 2, 3]  # values preserved
    assert out.field().name() == "x"


def test_cast_field_i64_to_f64():
    col = Serie.from_values([1, 2, 3], DataTypeId.I64)
    out = col.cast_field(Field(dtype=DataTypeId.F64))
    assert out.dtype() == DataTypeId.F64
    assert out.to_list() == [1.0, 2.0, 3.0]  # floats


def test_cast_field_narrowing_saturates():
    col = Serie.from_values([300, -5, 100], DataTypeId.I64)
    out = col.cast_field(Field(dtype=DataTypeId.I8))  # i8 range -128..127
    assert out.dtype() == DataTypeId.I8
    assert out.to_list() == [127, -5, 100]  # 300 saturates to the i8 max


def test_cast_field_dtype_change_preserves_nulls():
    col = Serie.from_options([1, None, 3], DataTypeId.I32)
    out = col.cast_field(Field("x", DataTypeId.I64, nullable=True))
    assert out.dtype() == DataTypeId.I64
    assert out.to_list() == [1, None, 3]  # the null survives the width change
    assert out.null_count() == 1


def test_cast_field_to_byte_dtype_raises():
    col = Serie.from_values([1, 2, 3], DataTypeId.I64)
    with pytest.raises(ValueError):
        col.cast_field(Field(dtype=DataTypeId.Utf8))


def test_cast_field_bool_or_decimal256_cross_dtype_guard():
    # A cross-dtype cast touching bool (bit-packed) does not convert through the numeric resize.
    col = Serie.from_values([1, 0, 1], DataTypeId.I64)
    with pytest.raises(ValueError):
        col.cast_field(Field(dtype=DataTypeId.Bool))
    # But a same-dtype bool -> bool reshape (adding nullability) still works.
    bools = Serie.from_values([True, False, True], DataTypeId.Bool)
    out = bools.cast_field(Field("flag", DataTypeId.Bool, nullable=True))
    assert out.dtype() == DataTypeId.Bool
    assert out.field().nullable() is True
    assert out.field().name() == "flag"
    assert out.to_list() == [True, False, True]


# =====================================================================================
# Serie.parse / parse_exact — build a column by parsing text; to_strings /
# to_string_options — render a column back to strings
# =====================================================================================


def test_parse_int_flexible():
    # Flexible parsing: thousands separators, leading +, scientific, hex radix.
    col = Serie.parse(["1,000", "+42", "1e3", "0xFF"], DataTypeId.I64)
    assert col.dtype() == DataTypeId.I64
    assert col.to_list() == [1000, 42, 1000, 255]
    assert col.to_strings() == ["1000", "42", "1000", "255"]


def test_parse_float_flexible():
    col = Serie.parse(["1,234.5", "9.99"], DataTypeId.F64)
    assert col.dtype() == DataTypeId.F64
    assert col.to_list() == [1234.5, 9.99]


def test_parse_bool_flexible():
    col = Serie.parse(["YES", "0", "true"], DataTypeId.Bool)
    assert col.dtype() == DataTypeId.Bool
    assert col.to_list() == [True, False, True]


def test_parse_str_dtype():
    col = Serie.parse(["1", "2", "3"], "i32")
    assert col.dtype() == DataTypeId.I32
    assert col.to_list() == [1, 2, 3]


def test_parse_exact_strict_rejects_flexible():
    # parse accepts "1,000"; parse_exact (str::parse, no coercion) rejects it.
    assert Serie.parse(["1,000"], DataTypeId.I64).to_list() == [1000]
    with pytest.raises(ValueError):
        Serie.parse_exact(["1,000"], DataTypeId.I64)


def test_parse_exact_accepts_plain():
    col = Serie.parse_exact(["1000", "42"], DataTypeId.I64)
    assert col.to_list() == [1000, 42]


def test_parse_invalid_value_raises():
    with pytest.raises(ValueError):
        Serie.parse(["not a number"], DataTypeId.I64)


def test_parse_non_fixed_width_raises():
    with pytest.raises(ValueError):
        Serie.parse(["a", "b"], DataTypeId.Utf8)
    with pytest.raises(ValueError):
        Serie.parse(["1"], DataTypeId.Unknown)


def test_parse_decimal256_raises_guided():
    # decimal256's I256 native has no string parse -> guided ValueError naming from_values.
    with pytest.raises(ValueError) as excinfo:
        Serie.parse(["1", "2"], DataTypeId.Decimal256)
    assert "from_values" in str(excinfo.value)
    with pytest.raises(ValueError):
        Serie.parse_exact(["1"], DataTypeId.Decimal256)


def test_to_strings_decimal_unscaled():
    # For a decimal column, to_strings renders the raw unscaled integer (not scaled).
    col = Serie.from_values([12345, 5], DataTypeId.Decimal128).with_precision_scale(10, 2)
    assert col.to_strings() == ["12345", "5"]
    # to_decimal_string is the scale-aware rendering.
    assert col.to_decimal_string(0) == "123.45"


def test_to_strings_decimal256_raises_guided():
    col = Serie.from_values([1, 2], DataTypeId.Decimal256)
    with pytest.raises(ValueError) as excinfo:
        col.to_strings()
    assert "to_decimal_string" in str(excinfo.value)
    with pytest.raises(ValueError):
        col.to_string_options()


def test_to_string_options_nullable():
    col = Serie.from_options([1, None, 3], DataTypeId.I64)
    assert col.to_string_options() == ["1", None, "3"]
    # to_strings ignores validity — the null slot surfaces its stored default.
    assert col.to_strings() == ["1", "0", "3"]


def test_to_strings_round_trip():
    col = Serie.from_values([10, 20, 30], DataTypeId.I32)
    assert col.to_strings() == ["10", "20", "30"]
    assert Serie.parse(col.to_strings(), DataTypeId.I32).to_list() == [10, 20, 30]
