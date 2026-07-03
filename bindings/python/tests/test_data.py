"""Tests for the data-model wrappers in the yggdryl Python binding."""

import pytest

from yggdryl import core, data

# (data type, field, scalar, optional scalar, name, format, byte width, min, max)
INTEGERS = [
    (data.Int8Type, data.Int8Field, data.Int8, data.OptionalInt8,
     "int8", "c", 1, -(2 ** 7), 2 ** 7 - 1),
    (data.Int16Type, data.Int16Field, data.Int16, data.OptionalInt16,
     "int16", "s", 2, -(2 ** 15), 2 ** 15 - 1),
    (data.Int32Type, data.Int32Field, data.Int32, data.OptionalInt32,
     "int32", "i", 4, -(2 ** 31), 2 ** 31 - 1),
    (data.Int64Type, data.Int64Field, data.Int64, data.OptionalInt64,
     "int64", "l", 8, -(2 ** 63), 2 ** 63 - 1),
    (data.UInt8Type, data.UInt8Field, data.UInt8, data.OptionalUInt8,
     "uint8", "C", 1, 0, 2 ** 8 - 1),
    (data.UInt16Type, data.UInt16Field, data.UInt16, data.OptionalUInt16,
     "uint16", "S", 2, 0, 2 ** 16 - 1),
    (data.UInt32Type, data.UInt32Field, data.UInt32, data.OptionalUInt32,
     "uint32", "I", 4, 0, 2 ** 32 - 1),
    (data.UInt64Type, data.UInt64Field, data.UInt64, data.OptionalUInt64,
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
def test_defaults(case):
    data_type, _, _, _, _, _, _, _, _ = case
    instance = data_type()
    assert instance.default_value() == 0
    assert instance.default_scalar().value() == 0

    optional = instance.optional()
    assert optional.default_value() == 0
    assert optional.default_scalar().is_null() is True  # the null variant


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
    # An integer is never a bool, a str or bytes: an actionable ValueError.
    with pytest.raises(ValueError, match="no bool conversion"):
        answer.as_bool()
    with pytest.raises(ValueError, match="no str conversion"):
        answer.as_str()
    with pytest.raises(ValueError, match="no bytes conversion"):
        answer.as_bytes()
    # A null scalar holds no value: every accessor raises.
    with pytest.raises(ValueError, match="is null"):
        scalar.null().as_i64()
    # The extreme converts only where it is exactly representable.
    if high <= 2 ** 7 - 1:
        assert scalar(high).as_i8() == high
    else:
        with pytest.raises(ValueError, match="not exactly representable"):
            scalar(high).as_i8()


@pytest.mark.parametrize("case", INTEGERS, ids=IDS)
def test_optional_scalar_redirects_to_the_inner_scalar(case):
    data_type, _, _, optional, name, _, _, _, _ = case
    answer = optional(42)
    assert answer.is_null() is False
    assert answer.value() == 42
    assert answer.scalar().value() == 42
    assert answer.as_i64() == 42

    # The data type is the logical optional over union storage.
    opt_type = answer.data_type()
    assert opt_type.name() == "optional"
    assert opt_type.arrow_format() == "+us:0,1"
    assert opt_type.byte_width() is None
    assert opt_type.value_type().name() == name
    storage = opt_type.storage()
    assert storage.name() == "union"
    assert storage.child_count() == 2
    assert storage.mode() == "sparse"

    missing = optional.null()
    assert missing.is_null() is True
    assert missing.value() is None
    assert missing.scalar() is None
    with pytest.raises(ValueError, match="is null"):
        missing.as_i64()

    # The optional reached through the value type is the same shape, and its
    # codec is the value type's.
    reached = data_type().optional()
    assert reached.arrow_format() == opt_type.arrow_format()
    assert reached.native_from_bytes(reached.native_to_bytes(42)) == 42


def test_float_access_is_exact_or_raises():
    # 2**53 is the last contiguous integer in f64; 2**53 + 1 rounds.
    assert data.Int64(2 ** 53).as_f64() == float(2 ** 53)
    with pytest.raises(ValueError, match="not exactly representable"):
        data.Int64(2 ** 53 + 1).as_f64()
    with pytest.raises(ValueError, match="not exactly representable"):
        data.UInt64(2 ** 64 - 1).as_f64()
    # Sign changes never pass, and the error names the offending value.
    with pytest.raises(ValueError, match="-1 is not exactly representable"):
        data.Int8(-1).as_u64()


def test_binary_type_describes_itself_and_codecs():
    binary = data.BinaryType()
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


def test_binary_field():
    payload = data.BinaryField("payload")
    assert payload.name() == "payload"
    assert payload.is_nullable() is True
    assert payload.data_type().name() == "binary"
    assert data.BinaryField("id", False).is_nullable() is False


def test_binary_scalar_reads_bytes_and_io():
    blob = data.Binary(b"\x01\x02\x03")
    assert blob.is_null() is False
    assert blob.value() == b"\x01\x02\x03"
    assert blob.as_bytes() == b"\x01\x02\x03"
    # UTF-8 bytes convert to str; anything else raises naming the shape.
    assert data.Binary(b"hi").as_str() == "hi"
    with pytest.raises(ValueError, match="non-UTF-8"):
        data.Binary(b"\xff").as_str()
    with pytest.raises(ValueError, match="no i64 conversion"):
        blob.as_i64()

    # The value doubles as a core positioned-IO ByteBuffer.
    io = blob.to_io()
    assert io.byte_size() == 3
    assert io.to_bytes() == b"\x01\x02\x03"
    assert io.pread_byte_one(1, core.Whence.Start) == 2

    # The empty value and null are distinct states.
    assert data.Binary(b"").is_null() is False
    missing = data.Binary.null()
    assert missing.is_null() is True
    assert missing.value() is None
    assert missing.to_io() is None
    with pytest.raises(ValueError, match="is null"):
        missing.as_bytes()


def test_optional_binary_redirects_to_the_inner_scalar():
    some = data.OptionalBinary(b"hi")
    assert some.is_null() is False
    assert some.value() == b"hi"
    assert some.scalar().value() == b"hi"
    assert some.as_bytes() == b"hi"
    assert some.as_str() == "hi"

    opt_type = some.data_type()
    assert opt_type.name() == "optional"
    assert opt_type.value_type().name() == "binary"
    assert opt_type.storage().name() == "union"
    assert opt_type.default_value() == b""
    assert opt_type.native_from_bytes(opt_type.native_to_bytes(b"xy")) == b"xy"

    missing = data.OptionalBinary.null()
    assert missing.is_null() is True
    assert missing.scalar() is None
    with pytest.raises(ValueError, match="is null"):
        missing.as_bytes()

    # The optional reached through the value type is the same shape.
    assert data.BinaryType().optional().arrow_format() == opt_type.arrow_format()
    assert data.OptionalBinaryField("payload").data_type().name() == "optional"


def test_optional_field():
    score = data.OptionalInt64Field("score")
    assert score.name() == "score"
    assert score.is_nullable() is True
    assert score.data_type().name() == "optional"
    assert score.data_type().value_type().name() == "int64"


def test_union_field():
    union = data.Int64Type().optional().storage()
    field = data.UnionField("value", union)
    assert field.name() == "value"
    assert field.is_nullable() is True
    assert field.data_type().arrow_format() == "+us:0,1"


def test_null_family():
    null = data.NullType()
    assert null.name() == "null"
    assert null.arrow_format() == "n"
    assert null.byte_width() is None
    assert null.bit_width() is None

    gap = data.NullField("gap")
    assert (gap.name(), gap.data_type().name(), gap.is_nullable()) == ("gap", "null", True)

    nothing = data.Null()
    assert nothing.is_null() is True
    assert nothing.data_type().name() == "null"
