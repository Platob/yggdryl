"""Tests for the yggdryl.field Python binding (Arrow primitive fields)."""

import pickle

import pytest

from yggdryl import field
from yggdryl.dtype import I64Type
from yggdryl.field import BooleanField, F64Field, I32Field, I64Field

ALL_NAMES = [
    "I8Field",
    "I16Field",
    "I32Field",
    "I64Field",
    "U8Field",
    "U16Field",
    "U32Field",
    "U64Field",
    "F32Field",
    "F64Field",
    "BooleanField",
]


def test_name_nullable_and_data_type():
    f = I64Field("id", False)
    assert f.name == "id"
    assert f.nullable is False
    assert f.data_type == I64Type()
    assert f.data_type.name == "int64"

    # nullable defaults to False.
    assert I32Field("count").nullable is False
    assert BooleanField("flag", True).nullable is True


def test_byte_round_trip_and_errors():
    f = I64Field("mesure_€", True)  # non-ASCII name
    assert I64Field.deserialize_bytes(f.serialize_bytes()) == f
    assert f.serialize_bytes()[0] == 1  # nullable flag first

    with pytest.raises(ValueError, match="nullable flag"):
        I64Field.deserialize_bytes(b"")


def test_value_semantics_and_pickle():
    a = I64Field("a", True)
    assert a == I64Field("a", True)
    assert a != I64Field("a", False)
    assert a != I64Field("b", True)
    assert hash(a) == hash(I64Field("a", True))
    assert len({a, I64Field("a", True), I64Field("a", False)}) == 2
    assert pickle.loads(pickle.dumps(a)) == a


def test_repr():
    assert repr(I64Field("id", True)) == "I64Field(name=\"id\", nullable=true)"


def test_headers_round_trips_and_is_identity_bearing():
    f = I64Field("ts", True).with_headers({b"unit": b"ms", b"\xff": b"bin"})
    assert f.headers == {b"unit": b"ms", b"\xff": b"bin"}
    # No headers by default.
    assert I64Field("ts", True).headers is None
    # Byte round-trip carries the headers (pickle too).
    assert I64Field.deserialize_bytes(f.serialize_bytes()) == f
    assert pickle.loads(pickle.dumps(f)) == f
    # Metadata is part of the field's identity.
    assert f != I64Field("ts", True)


def test_all_primitive_fields_are_exposed():
    for name in ALL_NAMES:
        assert hasattr(field, name), name
