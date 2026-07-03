"""Tests for the data-type wrappers (yggdryl.dtype) in the Python binding."""

import pytest

from yggdryl import dtype

# (data type, name, format, byte width, min, max)
INTEGERS = [
    (dtype.Int8, "int8", "c", 1, -(2 ** 7), 2 ** 7 - 1),
    (dtype.Int16, "int16", "s", 2, -(2 ** 15), 2 ** 15 - 1),
    (dtype.Int32, "int32", "i", 4, -(2 ** 31), 2 ** 31 - 1),
    (dtype.Int64, "int64", "l", 8, -(2 ** 63), 2 ** 63 - 1),
    (dtype.UInt8, "uint8", "C", 1, 0, 2 ** 8 - 1),
    (dtype.UInt16, "uint16", "S", 2, 0, 2 ** 16 - 1),
    (dtype.UInt32, "uint32", "I", 4, 0, 2 ** 32 - 1),
    (dtype.UInt64, "uint64", "L", 8, 0, 2 ** 64 - 1),
]

IDS = [case[1] for case in INTEGERS]


@pytest.mark.parametrize("case", INTEGERS, ids=IDS)
def test_data_type_describes_itself(case):
    data_type, name, fmt, width, _, _ = case
    instance = data_type()
    assert instance.name() == name
    assert instance.arrow_format() == fmt
    assert instance.byte_width() == width
    assert instance.bit_width() == width * 8


@pytest.mark.parametrize("case", INTEGERS, ids=IDS)
def test_defaults(case):
    data_type, _, _, _, _, _ = case
    instance = data_type()
    assert instance.default_value() == 0
    assert instance.default_scalar().value() == 0

    optional = instance.optional()
    assert optional.default_value() == 0
    assert optional.default_scalar().is_null() is True  # the null variant


@pytest.mark.parametrize("case", INTEGERS, ids=IDS)
def test_codec_round_trips(case):
    data_type, _, _, width, low, high = case
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
def test_optional_is_a_logical_type_over_union_storage(case):
    data_type, name, _, _, _, _ = case
    optional = data_type().optional()
    assert optional.name() == "optional"
    assert optional.arrow_format() == "+us:0,1"  # sparse, type ids 0 and 1
    assert optional.byte_width() is None
    assert optional.value_type().name() == name

    storage = optional.storage()
    assert storage.name() == "union"
    assert storage.child_count() == 2
    assert storage.mode() == "sparse"

    # The optional's codec is the value type's.
    assert optional.native_from_bytes(optional.native_to_bytes(42)) == 42


def test_binary_type_describes_itself_and_codecs():
    binary = dtype.Binary()
    assert binary.name() == "binary"
    assert binary.arrow_format() == "z"
    assert binary.byte_width() is None
    assert binary.bit_width() is None
    # The codec is the identity: any bytes are a valid binary value.
    assert binary.native_to_bytes(b"\x01\x02") == b"\x01\x02"
    assert binary.native_from_bytes(b"\x01\x02") == b"\x01\x02"
    assert binary.native_from_bytes(b"") == b""
    assert binary.default_value() == b""
    assert binary.default_scalar().value() == b""


def test_optional_binary_type():
    optional = dtype.Binary().optional()
    assert optional.name() == "optional"
    assert optional.value_type().name() == "binary"
    assert optional.storage().name() == "union"
    assert optional.default_value() == b""
    assert optional.default_scalar().is_null() is True
    assert optional.native_from_bytes(optional.native_to_bytes(b"xy")) == b"xy"
    assert dtype.OptionalBinary().arrow_format() == optional.arrow_format()


def test_null_type():
    null = dtype.Null()
    assert null.name() == "null"
    assert null.arrow_format() == "n"
    assert null.byte_width() is None
    assert null.bit_width() is None


def test_union_type_reached_through_optional():
    union = dtype.Int64().optional().storage()
    assert union.name() == "union"
    assert union.arrow_format() == "+us:0,1"
    assert union.byte_width() is None
    assert union.child_count() == 2
    assert union.mode() == "sparse"
