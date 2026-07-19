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
