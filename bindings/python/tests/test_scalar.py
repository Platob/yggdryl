"""Tests for the scalar wrappers (yggdryl.scalar) in the Python binding."""

import pytest

from yggdryl import core, scalar

# (scalar, optional scalar, name, min, max)
INTEGERS = [
    (scalar.Int8Scalar, scalar.OptionalInt8Scalar, "int8", -(2 ** 7), 2 ** 7 - 1),
    (scalar.Int16Scalar, scalar.OptionalInt16Scalar, "int16", -(2 ** 15), 2 ** 15 - 1),
    (scalar.Int32Scalar, scalar.OptionalInt32Scalar, "int32", -(2 ** 31), 2 ** 31 - 1),
    (scalar.Int64Scalar, scalar.OptionalInt64Scalar, "int64", -(2 ** 63), 2 ** 63 - 1),
    (scalar.UInt8Scalar, scalar.OptionalUInt8Scalar, "uint8", 0, 2 ** 8 - 1),
    (scalar.UInt16Scalar, scalar.OptionalUInt16Scalar, "uint16", 0, 2 ** 16 - 1),
    (scalar.UInt32Scalar, scalar.OptionalUInt32Scalar, "uint32", 0, 2 ** 32 - 1),
    (scalar.UInt64Scalar, scalar.OptionalUInt64Scalar, "uint64", 0, 2 ** 64 - 1),
]

IDS = [case[2] for case in INTEGERS]


@pytest.mark.parametrize("case", INTEGERS, ids=IDS)
def test_scalar_holds_a_value_or_null(case):
    scalar_class, _, name, low, high = case
    answer = scalar_class(42)
    assert answer.is_null() is False
    assert answer.value() == 42
    assert answer.data_type().name() == name
    assert scalar_class(low).value() == low
    assert scalar_class(high).value() == high

    missing = scalar_class.null()
    assert missing.is_null() is True
    assert missing.value() is None


@pytest.mark.parametrize("case", INTEGERS, ids=IDS)
def test_accessors_convert_exactly(case):
    scalar_class, _, _, _, high = case
    answer = scalar_class(42)
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
        scalar_class.null().as_i64()
    # The extreme converts only where it is exactly representable.
    if high <= 2 ** 7 - 1:
        assert scalar_class(high).as_i8() == high
    else:
        with pytest.raises(ValueError, match="not exactly representable"):
            scalar_class(high).as_i8()


@pytest.mark.parametrize("case", INTEGERS, ids=IDS)
def test_optional_scalar_redirects_to_the_inner_scalar(case):
    _, optional, name, _, _ = case
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

    missing = optional.null()
    assert missing.is_null() is True
    assert missing.value() is None
    assert missing.scalar() is None
    with pytest.raises(ValueError, match="is null"):
        missing.as_i64()


def test_float_access_is_exact_or_raises():
    # 2**53 is the last contiguous integer in f64; 2**53 + 1 rounds.
    assert scalar.Int64Scalar(2 ** 53).as_f64() == float(2 ** 53)
    with pytest.raises(ValueError, match="not exactly representable"):
        scalar.Int64Scalar(2 ** 53 + 1).as_f64()
    with pytest.raises(ValueError, match="not exactly representable"):
        scalar.UInt64Scalar(2 ** 64 - 1).as_f64()
    # Sign changes never pass, and the error names the offending value.
    with pytest.raises(ValueError, match="-1 is not exactly representable"):
        scalar.Int8Scalar(-1).as_u64()


def test_binary_scalar_reads_bytes_and_io():
    blob = scalar.BinaryScalar(b"\x01\x02\x03")
    assert blob.is_null() is False
    assert blob.value() == b"\x01\x02\x03"
    assert blob.as_bytes() == b"\x01\x02\x03"
    assert blob.data_type().name() == "binary"
    # UTF-8 bytes convert to str; anything else raises naming the shape — and
    # an explicit core charset decodes instead.
    assert scalar.BinaryScalar(b"hi").as_str() == "hi"
    assert scalar.BinaryScalar(b"hi").as_str("utf8") == "hi"
    assert scalar.BinaryScalar(b"\xe9").as_str("latin1") == "é"
    with pytest.raises(ValueError, match="non-UTF-8"):
        scalar.BinaryScalar(b"\xff").as_str()
    with pytest.raises(ValueError, match="unknown charset"):
        scalar.BinaryScalar(b"hi").as_str("ascii")
    with pytest.raises(ValueError, match="no i64 conversion"):
        blob.as_i64()

    # The value doubles as a core positioned-IO ByteBuffer.
    io = blob.to_io()
    assert io.byte_size() == 3
    assert io.to_bytes() == b"\x01\x02\x03"
    assert io.pread_byte_one(1, core.Whence.Start) == 2

    # ... or as a full-window ByteBufferSlice for window-relative reads.
    window = blob.to_io_slice()
    assert window.byte_size() == 3
    assert window.pread_byte_one(1, core.Whence.Start) == 2
    assert window.pread_i8(2, core.Whence.Start) == 3

    # The empty value and null are distinct states.
    assert scalar.BinaryScalar(b"").is_null() is False
    missing = scalar.BinaryScalar.null()
    assert missing.is_null() is True
    assert missing.value() is None
    assert missing.to_io() is None
    with pytest.raises(ValueError, match="is null"):
        missing.as_bytes()


def test_optional_binary_redirects_to_the_inner_scalar():
    some = scalar.OptionalBinaryScalar(b"hi")
    assert some.is_null() is False
    assert some.value() == b"hi"
    assert some.scalar().value() == b"hi"
    assert some.as_bytes() == b"hi"
    assert some.as_str() == "hi"

    opt_type = some.data_type()
    assert opt_type.name() == "optional"
    assert opt_type.value_type().name() == "binary"
    assert opt_type.storage().name() == "union"

    missing = scalar.OptionalBinaryScalar.null()
    assert missing.is_null() is True
    assert missing.scalar() is None
    with pytest.raises(ValueError, match="is null"):
        missing.as_bytes()


def test_null_scalar():
    nothing = scalar.NullScalar()
    assert nothing.is_null() is True
    assert nothing.data_type().name() == "null"


def test_int64_serie_holds_a_list():
    numbers = scalar.Int64Serie([1, 2, 3])
    assert numbers.is_null() is False
    assert numbers.is_empty() is False
    assert numbers.len() == 3
    assert numbers.values() == [1, 2, 3]
    assert numbers.get_at(1) == 2
    assert numbers.get_scalar_at(2).value() == 3
    assert numbers.get_scalar_at(3) is None  # out of bounds
    assert numbers.data_type().name() == "list"
    with pytest.raises(ValueError):
        numbers.get_at(3)  # out of bounds

    # The empty serie and null are distinct states.
    empty = scalar.Int64Serie([])
    assert empty.is_null() is False
    assert empty.is_empty() is True
    assert empty.values() == []

    missing = scalar.Int64Serie.null()
    assert missing.is_null() is True
    assert missing.values() is None
    with pytest.raises(ValueError):
        missing.get_at(0)
