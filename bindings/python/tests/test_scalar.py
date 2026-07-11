"""Tests for the yggdryl.scalar Python binding (Arrow primitive scalars).

Scalars are always present (non-nullable) — nullability is modelled separately.
"""

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


def test_value_and_data_type():
    present = I64Scalar(7)
    assert present.value == 7  # always present — a plain value
    assert present.data_type == I64Type()
    assert not hasattr(present, "is_null")  # nullability is not a scalar concern


def test_byte_round_trip():
    # A scalar serialises to just its value's little-endian bytes (no null flag).
    present = U64Scalar(2**63)  # bigint value survives round-trip
    raw = present.serialize_bytes()
    assert len(raw) == 8
    assert raw == (2**63).to_bytes(8, "little")
    assert U64Scalar.deserialize_bytes(raw) == present


def test_64bit_scalars_reject_out_of_range():
    # A value past the 64-bit range is rejected with a guided message (parity with Node).
    with pytest.raises(ValueError, match="out of range for int64"):
        I64Scalar(2**63)  # i64 max is 2**63 - 1
    with pytest.raises(ValueError, match="out of range for uint64"):
        U64Scalar(-1)  # u64 is unsigned
    with pytest.raises(ValueError, match="out of range for uint64"):
        U64Scalar(2**64)


def test_deserialize_errors_are_guided():
    # The only decode failure is value bytes that don't fit the data type's width.
    for bad in (b"", bytes([2]), bytes([0, 0, 0])):
        with pytest.raises(ValueError):
            I64Scalar.deserialize_bytes(bad)


def test_float_value_semantics_are_bitwise():
    assert F64Scalar(0.0) != F64Scalar(-0.0)  # distinct bits
    assert F64Scalar(math.nan) == F64Scalar(math.nan)  # same bit pattern


def test_value_semantics_and_pickle():
    a = I64Scalar(5)
    assert a == I64Scalar(5)
    assert a != I64Scalar(6)
    assert hash(a) == hash(I64Scalar(5))
    assert len({I64Scalar(5), I64Scalar(5), I64Scalar(6)}) == 2
    assert pickle.loads(pickle.dumps(a)) == a


def test_boolean_scalar():
    assert BooleanScalar(True).value is True
    assert BooleanScalar(False).value is False


def test_repr():
    assert repr(I64Scalar(7)) == "I64Scalar(7)"


def test_all_primitive_scalars_are_exposed():
    for name in ALL_NAMES:
        assert hasattr(scalar, name), name


def test_default_scalar():
    from yggdryl.scalar import BooleanScalar, F64Scalar

    assert I64Scalar.default_scalar() == I64Scalar(0)
    assert I64Scalar.default_scalar().value == 0
    assert F64Scalar.default_scalar() == F64Scalar(0.0)
    assert BooleanScalar.default_scalar().value is False


def test_f32_scalar_marshals_over_f64():
    from yggdryl.scalar import F32Scalar

    s = F32Scalar(1.5)
    assert s.value == 1.5  # f32 stored, exposed over native float
    assert F32Scalar.deserialize_bytes(s.serialize_bytes()) == s
    assert F32Scalar.default_scalar().value == 0.0
