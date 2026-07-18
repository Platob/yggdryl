"""Tests for the ``yggdryl.datatype_id`` ``DataTypeId`` enum.

Mirrors ``crates/yggdryl-core/src/datatype_id.rs`` on the Python surface: the wire-stable
numeric values, the ``u16`` id round-trip (``as_u16`` / ``from_u16``), the lowercase token
names (``name`` / ``from_name``), the storage widths (``byte_size`` / ``bit_size``), the
category predicates (``is_integer`` / ``is_signed`` / ``is_float`` / ``is_bool`` /
``is_fixed_width``), the element count, and the int/index/str/repr dunders.
"""

import operator

import pytest

import yggdryl.datatype_id
from yggdryl.datatype_id import DataTypeId

# Every variant in id order, with its wire-stable value.
ALL_TYPES = [
    (DataTypeId.Unknown, 0, "unknown"),
    (DataTypeId.Bool, 1, "bool"),
    (DataTypeId.I8, 2, "i8"),
    (DataTypeId.U8, 3, "u8"),
    (DataTypeId.I16, 4, "i16"),
    (DataTypeId.U16, 5, "u16"),
    (DataTypeId.I32, 6, "i32"),
    (DataTypeId.U32, 7, "u32"),
    (DataTypeId.I64, 8, "i64"),
    (DataTypeId.U64, 9, "u64"),
    (DataTypeId.I128, 10, "i128"),
    (DataTypeId.U128, 11, "u128"),
    (DataTypeId.F32, 12, "f32"),
    (DataTypeId.F64, 13, "f64"),
    (DataTypeId.Decimal32, 14, "decimal32"),
    (DataTypeId.Decimal64, 15, "decimal64"),
    (DataTypeId.Decimal128, 16, "decimal128"),
    (DataTypeId.Decimal256, 17, "decimal256"),
]


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
    assert DataTypeId.from_u16(18) == DataTypeId.Unknown  # one past Decimal256 (17)


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
    assert lookup[DataTypeId.from_u16(8)] == "wide"  # equal values hash equal
