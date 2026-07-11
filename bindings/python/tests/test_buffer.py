"""Tests for the yggdryl.buffer Python binding (typed native-type buffers)."""

import math
import pickle

import pytest

from yggdryl import buffer
from yggdryl.buffer import (
    BooleanBuffer,
    F64Buffer,
    I32Buffer,
    I64Buffer,
    U8Buffer,
    U64Buffer,
)
from yggdryl.dtype import I64Type
from yggdryl.io import Whence


def test_buffer_field_and_headers():
    # A buffer hands out the matching typed field, carrying its headers.
    annotated = I64Buffer([1, 2, 3]).with_headers({b"unit": b"ms"})
    assert annotated.headers == {b"unit": b"ms"}

    field = annotated.field("ts", True)
    assert field.name == "ts"
    assert field.nullable is True
    assert field.data_type == I64Type()
    assert field.headers == {b"unit": b"ms"}

    # No headers by default; field() defaults nullable to False.
    plain = I64Buffer([1, 2, 3])
    assert plain.headers is None
    assert plain.field("ts").nullable is False
    assert plain.field("ts").headers is None

    # The boolean buffer hands out a BooleanField.
    bfield = BooleanBuffer([True, False]).field("flag", True)
    assert bfield.name == "flag"

    # Metadata is an annotation — it does not change byte identity.
    assert I64Buffer([1, 2, 3]) == annotated
    assert I64Buffer([1, 2, 3]).serialize_bytes() == annotated.serialize_bytes()


def test_numeric_construct_and_access():
    buf = I64Buffer([10, 20, 30])
    assert len(buf) == 3
    assert buf.len() == 3
    assert not buf.is_empty()
    assert buf.get(1) == 20
    assert buf.get(3) is None
    assert buf.to_list() == [10, 20, 30]
    assert I64Buffer().is_empty()


def test_numeric_serialize_round_trip_and_validation():
    buf = I32Buffer([1, -2, 3])
    data = buf.serialize_bytes()
    assert len(data) == 12
    assert I32Buffer.deserialize_bytes(data) == buf

    # little-endian layout
    assert U8Buffer([1, 2, 3]).as_bytes() == b"\x01\x02\x03"
    assert I32Buffer([0x01020304]).as_bytes() == b"\x04\x03\x02\x01"

    # a non-multiple length raises with actionable guidance
    with pytest.raises(ValueError, match="multiple of 4"):
        I32Buffer.deserialize_bytes(bytes(6))


def test_numeric_value_semantics_and_pickle():
    a = I64Buffer([1, 2, 3])
    b = I64Buffer([1, 2, 3])
    assert a == b
    assert hash(a) == hash(b)
    assert len({a, b, I64Buffer([9])}) == 2
    assert pickle.loads(pickle.dumps(a)) == a


def test_unsigned_64_bit_round_trip():
    big = 2**63 + 7
    buf = U64Buffer([big, 0, 1])
    assert buf.get(0) == big
    assert U64Buffer.deserialize_bytes(buf.serialize_bytes()) == buf


def test_float_equality_is_bitwise():
    nan1 = F64Buffer([math.nan])
    nan2 = F64Buffer([math.nan])
    assert nan1 == nan2  # same bit-pattern, unlike IEEE ==
    # +0.0 and -0.0 differ in bytes, so the buffers differ
    assert F64Buffer([0.0]) != F64Buffer([-0.0])


def test_bridges_to_positioned_io():
    buf = I64Buffer([7, 8, 9])
    cursor = buf.byte_cursor()
    assert cursor.pread_i64_array(3, Whence.Start) == [7, 8, 9]
    assert I64Buffer.from_byte_buffer(buf.to_byte_buffer()) == buf


def test_boolean_buffer():
    buf = BooleanBuffer([True, False, True, True, False])
    assert len(buf) == 5
    assert buf.get(0) is True
    assert buf.get(1) is False
    assert buf.get(5) is None
    assert buf.count_set_bits() == 3
    assert buf.to_list() == [True, False, True, True, False]

    # trailing bits are canonicalised: 0xFF over 3 bits is only the low three
    packed = BooleanBuffer.from_bytes(b"\xff", 3)
    assert packed.count_set_bits() == 3
    assert packed == BooleanBuffer([True, True, True])

    # serialize round-trip + pickle
    assert BooleanBuffer.deserialize_bytes(buf.serialize_bytes()) == buf
    assert pickle.loads(pickle.dumps(buf)) == buf

    with pytest.raises(ValueError):
        BooleanBuffer.from_bytes(b"\x00\x00", 3)


def test_all_native_buffer_types_are_exposed():
    for name in (
        "I8Buffer",
        "I16Buffer",
        "I32Buffer",
        "I64Buffer",
        "U8Buffer",
        "U16Buffer",
        "U32Buffer",
        "U64Buffer",
        "F32Buffer",
        "F64Buffer",
        "BooleanBuffer",
    ):
        assert hasattr(buffer, name)
