"""Tests for the type-inference factory (yggdryl.factory) in the Python binding."""

import pytest

from yggdryl import factory


def test_scalar_infers_the_type_from_the_value():
    # int -> int64, bytes -> binary, None -> null, list[int] -> int64 serie.
    answer = factory.scalar(42)
    assert answer.data_type().name() == "int64"
    assert answer.as_i64() == 42

    blob = factory.scalar(b"\x01\x02\x03")
    assert blob.data_type().name() == "binary"
    assert blob.as_bytes() == b"\x01\x02\x03"

    nothing = factory.scalar(None)
    assert nothing.data_type().name() == "null"
    assert nothing.is_null()

    numbers = factory.scalar([1, 2, 3])
    assert numbers.data_type().name() == "list"
    assert numbers.to_pylist() == [1, 2, 3]

    # bytearray also infers to binary.
    assert factory.scalar(bytearray(b"hi")).data_type().name() == "binary"
    # An empty list defaults to the int64 serie.
    assert factory.scalar([]).data_type().name() == "list"


def test_dtype_infers_the_type_from_the_value():
    assert factory.dtype(42).name() == "int64"
    assert factory.dtype(b"x").name() == "binary"
    assert factory.dtype(None).name() == "null"
    assert factory.dtype([1, 2, 3]).name() == "list"


def test_field_infers_the_type_and_keeps_the_name():
    id_field = factory.field("id", 42)
    assert id_field.name() == "id"
    assert id_field.data_type().name() == "int64"
    assert id_field.is_nullable()  # nullable defaults to True

    # nullable is respected.
    payload = factory.field("payload", b"x", nullable=False)
    assert payload.name() == "payload"
    assert payload.data_type().name() == "binary"
    assert not payload.is_nullable()

    scores = factory.field("scores", [1, 2, 3])
    assert scores.data_type().name() == "list"

    missing = factory.field("maybe", None)
    assert missing.data_type().name() == "null"


@pytest.mark.parametrize("value", [1.5, "text", True, {"a": 1}, [1, "x"]])
def test_unsupported_values_raise(value):
    # A float, str, bool, dict, or a non-int list has no matching model type.
    with pytest.raises(ValueError):
        factory.scalar(value)
    with pytest.raises(ValueError):
        factory.dtype(value)


def test_an_int_outside_int64_raises():
    with pytest.raises(ValueError):
        factory.scalar(2 ** 64)
