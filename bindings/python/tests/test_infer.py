"""Tests for the yggdryl.infer Python binding (runtime buffer type inference)."""

import pytest

from yggdryl.infer import buffer
from yggdryl.buffer import (
    BooleanBuffer,
    F64Buffer,
    I64Buffer,
    U8Buffer,
)


def test_infers_int_sequence_as_i64():
    buf = buffer([10, 20, 30])
    assert isinstance(buf, I64Buffer)
    assert buf == I64Buffer([10, 20, 30])


def test_infers_float_sequence_as_f64():
    buf = buffer([1.5, 2.5])
    assert isinstance(buf, F64Buffer)
    assert buf == F64Buffer([1.5, 2.5])


def test_infers_bool_sequence_as_boolean_before_int():
    buf = buffer([True, False, True])
    assert isinstance(buf, BooleanBuffer)
    assert buf == BooleanBuffer([True, False, True])


def test_infers_bytes_like_as_u8():
    assert buffer(b"\x01\x02\x03") == U8Buffer([1, 2, 3])
    assert buffer(bytearray([4, 5])) == U8Buffer([4, 5])
    assert isinstance(buffer(b"abc"), U8Buffer)


def test_accepts_tuples():
    assert buffer((1, 2, 3)) == I64Buffer([1, 2, 3])


def test_empty_sequence_is_a_guided_error():
    with pytest.raises(ValueError, match="empty sequence"):
        buffer([])


def test_out_of_i64_range_names_explicit_constructor():
    with pytest.raises(ValueError, match="U64Buffer"):
        buffer([2**64])


def test_mixed_sequence_is_rejected():
    with pytest.raises(ValueError):
        buffer([1, 2.5])


def test_unsupported_element_type_is_a_guided_error():
    with pytest.raises(TypeError, match="bool, int, and float"):
        buffer(["a", "b"])


def test_non_sequence_is_a_guided_error():
    with pytest.raises(TypeError):
        buffer(42)


def test_none_becomes_the_type_default():
    # A null element materialises into the type's default value.
    assert buffer([1, None, 3]) == I64Buffer([1, 0, 3])
    assert buffer([1.5, None]) == F64Buffer([1.5, 0.0])
    assert buffer([True, None, False]) == BooleanBuffer([True, False, False])
    # The element type is inferred from the first non-null element, even with leading nulls.
    assert buffer([None, 5, None]) == I64Buffer([0, 5, 0])


def test_all_null_sequence_is_a_guided_error():
    with pytest.raises(ValueError, match="every value is null"):
        buffer([None, None])


def test_float_first_mixed_sequence_is_rejected():
    # An int in a float sequence raises (previously silently coerced with precision loss).
    with pytest.raises(ValueError, match="must be a float"):
        buffer([1.5, 2])
