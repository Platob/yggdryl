"""Tests for the `null` type across the dtype / field / scalar bindings.

The null type is how yggdryl represents "null" now that scalars are always present.
"""

import pickle

from yggdryl.dtype import NullType
from yggdryl.field import NullField
from yggdryl.scalar import NullScalar


def test_null_type():
    dt = NullType()
    assert dt.name == "null"
    assert dt.byte_width == 0  # a null value is zero bytes
    assert dt.primitive_tag is None  # sui generis — not a primitive
    assert dt.serialize_bytes() == b""
    assert NullType.deserialize_bytes(b"") == dt
    assert dt == NullType()
    assert hash(dt) == hash(NullType())
    assert pickle.loads(pickle.dumps(dt)) == dt
    assert repr(dt) == "NullType()"


def test_null_field():
    f = NullField("maybe", True)
    assert f.name == "maybe"
    assert f.nullable is True
    assert f.data_type == NullType()
    assert NullField.deserialize_bytes(f.serialize_bytes()) == f
    assert pickle.loads(pickle.dumps(f)) == f


def test_null_scalar():
    s = NullScalar()
    assert s.value is None  # its value is the null value
    assert s.data_type == NullType()
    assert s.serialize_bytes() == b""
    assert NullScalar.deserialize_bytes(b"") == s
    assert s == NullScalar()
    assert hash(s) == hash(NullScalar())
    assert pickle.loads(pickle.dumps(s)) == s
    assert repr(s) == "NullScalar()"
    assert NullScalar.default_scalar() == s  # the default null scalar is the null value
