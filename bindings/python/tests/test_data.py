"""Tests for the data-model wrappers in the yggdryl Python binding."""

import pytest

from yggdryl import data

# (data type, field, scalar, optional scalar, name, format, byte width, min, max)
INTEGERS = [
    (data.Int8, data.Int8Field, data.Int8Scalar, data.OptionalInt8Scalar,
     "int8", "c", 1, -(2 ** 7), 2 ** 7 - 1),
    (data.Int16, data.Int16Field, data.Int16Scalar, data.OptionalInt16Scalar,
     "int16", "s", 2, -(2 ** 15), 2 ** 15 - 1),
    (data.Int32, data.Int32Field, data.Int32Scalar, data.OptionalInt32Scalar,
     "int32", "i", 4, -(2 ** 31), 2 ** 31 - 1),
    (data.Int64, data.Int64Field, data.Int64Scalar, data.OptionalInt64Scalar,
     "int64", "l", 8, -(2 ** 63), 2 ** 63 - 1),
    (data.UInt8, data.UInt8Field, data.UInt8Scalar, data.OptionalUInt8Scalar,
     "uint8", "C", 1, 0, 2 ** 8 - 1),
    (data.UInt16, data.UInt16Field, data.UInt16Scalar, data.OptionalUInt16Scalar,
     "uint16", "S", 2, 0, 2 ** 16 - 1),
    (data.UInt32, data.UInt32Field, data.UInt32Scalar, data.OptionalUInt32Scalar,
     "uint32", "I", 4, 0, 2 ** 32 - 1),
    (data.UInt64, data.UInt64Field, data.UInt64Scalar, data.OptionalUInt64Scalar,
     "uint64", "L", 8, 0, 2 ** 64 - 1),
]

IDS = [case[4] for case in INTEGERS]


@pytest.mark.parametrize("case", INTEGERS, ids=IDS)
def test_data_type_describes_itself(case):
    data_type, _, _, _, name, fmt, width, _, _ = case
    instance = data_type()
    assert instance.name() == name
    assert instance.arrow_format() == fmt
    assert instance.byte_width() == width
    assert instance.bit_width() == width * 8


@pytest.mark.parametrize("case", INTEGERS, ids=IDS)
def test_codec_round_trips(case):
    data_type, _, _, _, _, _, width, low, high = case
    instance = data_type()
    for value in (low, 0, 42, high):
        encoded = instance.native_to_bytes(value)
        assert len(encoded) == width
        assert instance.native_from_bytes(encoded) == value
    # Little-endian: the low byte comes first.
    assert instance.native_to_bytes(1)[0] == 1
    with pytest.raises(ValueError):
        instance.native_from_bytes(b"\x00" * (width + 1))


@pytest.mark.parametrize("case", INTEGERS, ids=IDS)
def test_field_pairs_a_name_with_the_type(case):
    _, field, _, _, name, _, _, _, _ = case
    column = field("id", False)
    assert column.name() == "id"
    assert column.data_type().name() == name
    assert column.is_nullable() is False
    assert field("maybe").is_nullable() is True  # nullable by default


@pytest.mark.parametrize("case", INTEGERS, ids=IDS)
def test_scalar_holds_a_value_or_null(case):
    _, _, scalar, _, name, _, _, low, high = case
    answer = scalar(42)
    assert answer.is_null() is False
    assert answer.value() == 42
    assert answer.data_type().name() == name
    assert scalar(low).value() == low
    assert scalar(high).value() == high

    missing = scalar.null()
    assert missing.is_null() is True
    assert missing.value() is None


@pytest.mark.parametrize("case", INTEGERS, ids=IDS)
def test_accessors_convert_exactly(case):
    _, _, scalar, _, _, _, _, _, high = case
    answer = scalar(42)
    # A small value converts to every numeric target.
    for accessor in ("as_i8", "as_i16", "as_i32", "as_i64",
                     "as_u8", "as_u16", "as_u32", "as_u64"):
        assert getattr(answer, accessor)() == 42
    assert answer.as_f32() == 42.0
    assert answer.as_f64() == 42.0
    # An integer is never a bool or a str.
    assert answer.as_bool() is None
    assert answer.as_str() is None
    # Null answers None everywhere.
    assert scalar.null().as_i64() is None
    # The extreme converts only where it is exactly representable.
    assert scalar(high).as_i8() == (high if high <= 2 ** 7 - 1 else None)


@pytest.mark.parametrize("case", INTEGERS, ids=IDS)
def test_optional_scalar_redirects_to_the_inner_scalar(case):
    data_type, _, _, optional, name, _, _, _, _ = case
    answer = optional(42)
    assert answer.is_null() is False
    assert answer.value() == 42
    assert answer.scalar().value() == 42
    assert answer.as_i64() == 42

    union = answer.data_type()
    assert union.name() == "union"
    assert union.arrow_format() == "+us:0,1"
    assert union.child_count() == 2
    assert union.mode() == "sparse"
    assert union.byte_width() is None

    missing = optional.null()
    assert missing.is_null() is True
    assert missing.value() is None
    assert missing.scalar() is None
    assert missing.as_i64() is None

    # The union reached through the data type is the same shape.
    assert data_type().optional().arrow_format() == union.arrow_format()


def test_float_access_is_exact_or_none():
    # 2**53 is the last contiguous integer in f64; 2**53 + 1 rounds.
    assert data.Int64Scalar(2 ** 53).as_f64() == float(2 ** 53)
    assert data.Int64Scalar(2 ** 53 + 1).as_f64() is None
    assert data.UInt64Scalar(2 ** 64 - 1).as_f64() is None
    # Sign changes never pass.
    assert data.Int8Scalar(-1).as_u64() is None


def test_union_field():
    union = data.Int64().optional()
    field = data.UnionField("value", union)
    assert field.name() == "value"
    assert field.is_nullable() is True
    assert field.data_type().arrow_format() == "+us:0,1"


def test_null_family():
    null = data.Null()
    assert null.name() == "null"
    assert null.arrow_format() == "n"
    assert null.byte_width() is None
    assert null.bit_width() is None

    gap = data.NullField("gap")
    assert (gap.name(), gap.data_type().name(), gap.is_nullable()) == ("gap", "null", True)

    nothing = data.NullScalar()
    assert nothing.is_null() is True
    assert nothing.data_type().name() == "null"
