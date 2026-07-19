"""Tests for the ``yggdryl.datatype_id`` ``DataTypeId`` enum.

Mirrors ``crates/yggdryl-core/src/datatype_id.rs`` on the Python surface: the wire-stable
numeric values, the ``u16`` id round-trip (``as_u16`` / ``from_u16``), the lowercase token
names (``name`` / ``from_name``), the storage widths (``byte_size`` / ``bit_size``), the
category predicates (``is_integer`` / ``is_signed`` / ``is_float`` / ``is_bool`` /
``is_fixed_width`` / ``is_binary`` / ``is_utf8`` / ``is_variable_length`` / ``is_large``), the
element count,
and the int/index/str/repr dunders.
"""

import operator

import pytest

import yggdryl.datatype_id
from yggdryl.datatype_id import DataTypeId

# Every variant, with its wire-stable value — ids live in per-category bands with reserved gaps.
ALL_TYPES = [
    (DataTypeId.Unknown, 0x0000, "unknown"),
    (DataTypeId.Bool, 0x0010, "bool"),
    (DataTypeId.I8, 0x0100, "i8"),
    (DataTypeId.U8, 0x0101, "u8"),
    (DataTypeId.I16, 0x0102, "i16"),
    (DataTypeId.U16, 0x0103, "u16"),
    (DataTypeId.I32, 0x0104, "i32"),
    (DataTypeId.U32, 0x0105, "u32"),
    (DataTypeId.I64, 0x0106, "i64"),
    (DataTypeId.U64, 0x0107, "u64"),
    (DataTypeId.I128, 0x0108, "i128"),
    (DataTypeId.U128, 0x0109, "u128"),
    (DataTypeId.F32, 0x0201, "f32"),
    (DataTypeId.F64, 0x0202, "f64"),
    (DataTypeId.Decimal32, 0x0300, "decimal32"),
    (DataTypeId.Decimal64, 0x0301, "decimal64"),
    (DataTypeId.Decimal128, 0x0302, "decimal128"),
    (DataTypeId.Decimal256, 0x0303, "decimal256"),
    (DataTypeId.Binary, 0x0500, "binary"),
    (DataTypeId.LargeBinary, 0x0502, "large_binary"),
    (DataTypeId.FixedBinary, 0x0510, "fixed_binary"),
    (DataTypeId.Utf8, 0x0600, "utf8"),
    (DataTypeId.LargeUtf8, 0x0602, "large_utf8"),
    (DataTypeId.FixedUtf8, 0x0610, "fixed_utf8"),
]

# The band-name each type's category() reports.
CATEGORIES = {
    DataTypeId.Unknown: "null",
    DataTypeId.Bool: "boolean",
    DataTypeId.I32: "integer",
    DataTypeId.U128: "integer",
    DataTypeId.F64: "float",
    DataTypeId.Decimal128: "decimal",
    DataTypeId.Binary: "binary",
    DataTypeId.LargeBinary: "binary",
    DataTypeId.FixedBinary: "binary",
    DataTypeId.Utf8: "utf8",
    DataTypeId.LargeUtf8: "utf8",
    DataTypeId.FixedUtf8: "utf8",
}


def test_module_surface():
    assert DataTypeId.__module__ == "yggdryl.datatype_id"
    assert hasattr(yggdryl.datatype_id, "DataTypeId")


def test_wire_stable_values_and_int():
    for dtype, value, _ in ALL_TYPES:
        assert dtype == value  # eq_int
        assert int(dtype) == value
        assert operator.index(dtype) == value
        assert dtype.as_u16() == value
    assert DataTypeId.Unknown == 0  # the default (zero) value


def test_as_u16_from_u16_round_trip():
    for dtype, value, _ in ALL_TYPES:
        assert dtype.as_u16() == value
        assert DataTypeId.from_u16(value) == dtype
    # An unrecognized id degrades to Unknown (total, never raises).
    assert DataTypeId.from_u16(999) == DataTypeId.Unknown
    assert DataTypeId.from_u16(0x0011) == DataTypeId.Unknown  # a reserved gap in the bool band


def test_category_bands():
    for dtype, expected in CATEGORIES.items():
        assert dtype.category() == expected
    assert DataTypeId.I64.is_numeric() and DataTypeId.F64.is_numeric()
    assert DataTypeId.Decimal32.is_numeric()
    assert not DataTypeId.Bool.is_numeric() and not DataTypeId.Utf8.is_numeric()
    assert DataTypeId.FixedBinary.is_byte_like() and DataTypeId.FixedBinary.is_fixed_size()
    assert not DataTypeId.Binary.is_fixed_size()
    # The reserved bands answer their predicates with no member yet.
    assert not DataTypeId.I64.is_temporal() and not DataTypeId.I64.is_nested()


def test_names_and_from_name():
    for dtype, _, name in ALL_TYPES:
        assert dtype.name() == name
        assert DataTypeId.from_name(name) == dtype
    # from_name is case-insensitive and trims whitespace.
    assert DataTypeId.from_name("I32") == DataTypeId.I32
    assert DataTypeId.from_name("  f64 ") == DataTypeId.F64
    assert DataTypeId.from_name("unknown") == DataTypeId.Unknown
    # An unknown token is None (mirrors the core Option).
    assert DataTypeId.from_name("nope") is None


def test_byte_and_bit_sizes():
    widths = {
        DataTypeId.Unknown: 0,
        DataTypeId.Bool: 1,
        DataTypeId.I8: 1,
        DataTypeId.U8: 1,
        DataTypeId.I16: 2,
        DataTypeId.U16: 2,
        DataTypeId.I32: 4,
        DataTypeId.U32: 4,
        DataTypeId.F32: 4,
        DataTypeId.I64: 8,
        DataTypeId.U64: 8,
        DataTypeId.F64: 8,
        DataTypeId.I128: 16,
        DataTypeId.U128: 16,
        DataTypeId.Decimal32: 4,
        DataTypeId.Decimal64: 8,
        DataTypeId.Decimal128: 16,
        DataTypeId.Decimal256: 32,
        # The byte columns are not fixed-width — no id-derivable element width.
        DataTypeId.Binary: 0,
        DataTypeId.Utf8: 0,
        DataTypeId.FixedBinary: 0,
        DataTypeId.FixedUtf8: 0,
    }
    for dtype, byte_size in widths.items():
        assert dtype.byte_size() == byte_size
    # bit_size: bool is 1 bit, every other fixed type is byte_size * 8, Unknown is 0.
    assert DataTypeId.Bool.bit_size() == 1
    assert DataTypeId.I32.bit_size() == 32
    assert DataTypeId.F64.bit_size() == 64
    assert DataTypeId.Unknown.bit_size() == 0


def test_category_predicates():
    assert DataTypeId.I32.is_integer()
    assert DataTypeId.U64.is_integer()
    assert not DataTypeId.Bool.is_integer()  # bool is NOT counted as an integer
    assert not DataTypeId.F32.is_integer()
    assert not DataTypeId.Unknown.is_integer()

    assert DataTypeId.I32.is_signed()
    assert DataTypeId.F64.is_signed()
    assert not DataTypeId.U32.is_signed()
    assert not DataTypeId.Bool.is_signed()

    assert DataTypeId.F32.is_float()
    assert DataTypeId.F64.is_float()
    assert not DataTypeId.I32.is_float()

    assert DataTypeId.Bool.is_bool()
    assert not DataTypeId.I8.is_bool()

    assert DataTypeId.I32.is_fixed_width()
    assert DataTypeId.Bool.is_fixed_width()
    assert not DataTypeId.Unknown.is_fixed_width()


def test_element_count():
    assert DataTypeId.I32.element_count(20) == 5
    assert DataTypeId.I64.element_count(20) == 2  # 20 // 8, whole elements only
    assert DataTypeId.Bool.element_count(3) == 3
    assert DataTypeId.Unknown.element_count(100) == 0  # raw bytes have no element count


def test_str_and_repr():
    assert str(DataTypeId.I32) == "i32"
    assert str(DataTypeId.Unknown) == "unknown"
    assert [str(d) for d, _, _ in ALL_TYPES] == [name for _, _, name in ALL_TYPES]
    assert repr(DataTypeId.I32) == "DataTypeId.I32"
    assert repr(DataTypeId.F64) == "DataTypeId.F64"


def test_hashable_and_frozen():
    assert {DataTypeId.I32, DataTypeId.I32, DataTypeId.F64} == {DataTypeId.I32, DataTypeId.F64}
    lookup = {DataTypeId.I64: "wide"}
    assert lookup[DataTypeId.from_u16(0x0106)] == "wide"  # equal values hash equal


# -------------------------------------------------------------------------------------
# The four byte columns — Binary / Utf8 / FixedBinary / FixedUtf8
# -------------------------------------------------------------------------------------


def test_byte_variants():
    byte_types = [
        (DataTypeId.Binary, 0x0500, "binary"),
        (DataTypeId.LargeBinary, 0x0502, "large_binary"),
        (DataTypeId.Utf8, 0x0600, "utf8"),
        (DataTypeId.LargeUtf8, 0x0602, "large_utf8"),
        (DataTypeId.FixedBinary, 0x0510, "fixed_binary"),
        (DataTypeId.FixedUtf8, 0x0610, "fixed_utf8"),
    ]
    for dtype, value, name in byte_types:
        assert int(dtype) == value
        assert dtype.name() == name
        assert DataTypeId.from_name(name) == dtype
        assert DataTypeId.from_u16(value) == dtype
        # Not fixed-width — no id-derivable element width (a fixed-size width is field metadata).
        assert dtype.byte_size() == 0
        assert dtype.is_fixed_width() is False
        # Byte columns are neither integer nor float nor bool.
        assert not dtype.is_integer()
        assert not dtype.is_float()
        assert not dtype.is_bool()


def test_byte_category_predicates():
    # is_binary: Binary / LargeBinary / FixedBinary.
    assert DataTypeId.Binary.is_binary()
    assert DataTypeId.LargeBinary.is_binary()
    assert DataTypeId.FixedBinary.is_binary()
    assert not DataTypeId.Utf8.is_binary()
    assert not DataTypeId.I32.is_binary()

    # is_utf8: Utf8 / LargeUtf8 / FixedUtf8.
    assert DataTypeId.Utf8.is_utf8()
    assert DataTypeId.LargeUtf8.is_utf8()
    assert DataTypeId.FixedUtf8.is_utf8()
    assert not DataTypeId.Binary.is_utf8()
    assert not DataTypeId.I32.is_utf8()

    # is_variable_length: the offsets + data layouts (Binary / Utf8 / LargeBinary / LargeUtf8).
    assert DataTypeId.Binary.is_variable_length()
    assert DataTypeId.Utf8.is_variable_length()
    assert DataTypeId.LargeBinary.is_variable_length()
    assert DataTypeId.LargeUtf8.is_variable_length()
    assert not DataTypeId.FixedBinary.is_variable_length()
    assert not DataTypeId.FixedUtf8.is_variable_length()
    assert not DataTypeId.I32.is_variable_length()


def test_large_byte_variants():
    # The i64-offset large columns — ids, names, and the u16 / name round-trips.
    for dtype, value, name in [
        (DataTypeId.LargeBinary, 0x0502, "large_binary"),
        (DataTypeId.LargeUtf8, 0x0602, "large_utf8"),
    ]:
        assert int(dtype) == value
        assert dtype.name() == name
        assert DataTypeId.from_u16(value) == dtype
        assert DataTypeId.from_name(name) == dtype
        # Variable-length + large, but not a fixed-size stride.
        assert dtype.is_variable_length()
        assert dtype.is_large()
        assert dtype.is_fixed_size() is False
        # Not fixed-width — no id-derivable element width.
        assert dtype.byte_size() == 0
        assert dtype.is_fixed_width() is False

    # is_binary / is_utf8 split the two large columns.
    assert DataTypeId.LargeBinary.is_binary() and not DataTypeId.LargeBinary.is_utf8()
    assert DataTypeId.LargeUtf8.is_utf8() and not DataTypeId.LargeUtf8.is_binary()

    # is_large is false for the i32 / fixed byte columns and for a numeric type.
    for dtype in (
        DataTypeId.Binary,
        DataTypeId.Utf8,
        DataTypeId.FixedBinary,
        DataTypeId.FixedUtf8,
        DataTypeId.I32,
    ):
        assert dtype.is_large() is False
