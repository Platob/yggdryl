"""Tests for the yggdryl.dtype Python binding (Arrow primitive data types)."""

import pickle

import pytest

from yggdryl import dtype
from yggdryl.dtype import (
    BooleanType,
    F32Type,
    F64Type,
    I8Type,
    I32Type,
    I64Type,
    U64Type,
)

ALL_NAMES = [
    "I8Type",
    "I16Type",
    "I32Type",
    "I64Type",
    "U8Type",
    "U16Type",
    "U32Type",
    "U64Type",
    "F32Type",
    "F64Type",
    "BooleanType",
]


def test_names_and_widths():
    assert I8Type().name == "int8"
    assert I8Type().byte_width == 1
    assert I64Type().byte_width == 8
    assert F32Type().byte_width == 4
    assert I64Type().primitive_tag == "i64"
    assert U64Type().primitive_tag == "u64"


def test_boolean_is_bit_packed():
    dt = BooleanType()
    assert dt.name == "boolean"
    assert dt.byte_width is None  # bit-packed
    assert dt.primitive_tag is None  # outside the core numeric tags


def test_byte_round_trip_and_error():
    dt = I32Type()
    assert dt.serialize_bytes() == b""
    assert I32Type.deserialize_bytes(dt.serialize_bytes()) == dt
    with pytest.raises(ValueError, match="carries no parameters"):
        I32Type.deserialize_bytes(b"\x01")


def test_value_semantics_and_pickle():
    a, b = I64Type(), I64Type()
    assert a == b
    assert hash(a) == hash(b)
    assert I64Type() != F64Type()
    # Markers deduplicate in a set.
    assert len({I64Type(), I64Type(), F64Type()}) == 2
    # Pickle round-trips.
    assert pickle.loads(pickle.dumps(a)) == a
    assert pickle.loads(pickle.dumps(BooleanType())) == BooleanType()


def test_repr():
    assert repr(I64Type()) == "I64Type()"


def test_all_primitive_types_are_exposed():
    for name in ALL_NAMES:
        assert hasattr(dtype, name), name
