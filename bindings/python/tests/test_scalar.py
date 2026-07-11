"""Tests for the yggdryl.scalar Python binding (Arrow primitive scalars)."""

import math
import pickle

import pytest

from yggdryl import scalar
from yggdryl.dtype import I64Type
from yggdryl.scalar import (
    BooleanScalar,
    F64Scalar,
    I64Scalar,
    U64Scalar,
)

ALL_NAMES = [
    "I8Scalar",
    "I16Scalar",
    "I32Scalar",
    "I64Scalar",
    "U8Scalar",
    "U16Scalar",
    "U32Scalar",
    "U64Scalar",
    "F32Scalar",
    "F64Scalar",
    "BooleanScalar",
]


def test_present_and_null():
    present = I64Scalar(7)
    assert present.value == 7
    assert present.is_null is False
    assert present.data_type == I64Type()

    null = I64Scalar(None)
    assert null.value is None
    assert null.is_null is True
    # The no-arg constructor and the null() factory also build a null.
    assert I64Scalar().is_null is True
    assert I64Scalar.null().is_null is True


def test_byte_round_trip_present_and_null():
    present = U64Scalar(2**63)  # bigint value survives round-trip
    assert U64Scalar.deserialize_bytes(present.serialize_bytes()) == present
    assert present.serialize_bytes()[0] == 1  # present flag

    null = I64Scalar.null()
    assert null.serialize_bytes() == bytes([0])
    assert I64Scalar.deserialize_bytes(bytes([0])) == null


def test_deserialize_errors_are_guided():
    with pytest.raises(ValueError, match="null flag"):
        I64Scalar.deserialize_bytes(b"")
    with pytest.raises(ValueError, match="expected 0"):
        I64Scalar.deserialize_bytes(bytes([2]))


def test_float_value_semantics_are_bitwise():
    assert F64Scalar(0.0) != F64Scalar(-0.0)  # distinct bits
    assert F64Scalar(1.0) != F64Scalar.null()
    assert F64Scalar(math.nan) == F64Scalar(math.nan)  # same bit pattern


def test_value_semantics_and_pickle():
    a = I64Scalar(5)
    assert a == I64Scalar(5)
    assert a != I64Scalar(6)
    assert hash(a) == hash(I64Scalar(5))
    assert I64Scalar.null() == I64Scalar.null()
    assert len({I64Scalar(5), I64Scalar(5), I64Scalar.null()}) == 2
    assert pickle.loads(pickle.dumps(a)) == a
    assert pickle.loads(pickle.dumps(I64Scalar.null())) == I64Scalar.null()


def test_boolean_scalar():
    assert BooleanScalar(True).value is True
    assert BooleanScalar(False).value is False
    assert BooleanScalar.null().is_null is True


def test_repr():
    assert repr(I64Scalar(7)) == "I64Scalar(7)"
    assert repr(I64Scalar.null()) == "I64Scalar(null)"


def test_all_primitive_scalars_are_exposed():
    for name in ALL_NAMES:
        assert hasattr(scalar, name), name
