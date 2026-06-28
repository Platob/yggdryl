"""Tests for the yggdryl Scalar (a single atomic value). Build first with
``maturin develop``, then ``pytest``."""

import pickle

from yggdryl import DataType, DateTime, Scalar


def test_infer_and_read_primitives():
    assert str(Scalar(42).data_type) == "int64"
    assert Scalar(42).value == 42
    assert str(Scalar(1.5).data_type) == "float64"
    assert Scalar(True).value is True
    assert Scalar("hi").value == "hi"
    assert Scalar(b"xy").value == b"xy"


def test_explicit_dtype():
    assert str(Scalar(5, "int32").data_type) == "int32"
    assert str(Scalar(5, DataType.int(32, True)).data_type) == "int32"
    assert str(Scalar(1.5, "float32").data_type) == "float32"


def test_typed_null():
    n = Scalar.null("int64")
    assert n.is_null
    assert n.value is None
    assert str(n.data_type) == "int64"


def test_canonical_string_round_trip():
    assert Scalar(42).to_str() == "42::int64"
    assert Scalar.from_str("42::int64") == Scalar(42)


def test_bytes_and_pickle_round_trip():
    s = Scalar.from_str("1700000000::timestamp[s]")
    assert Scalar.from_bytes(s.to_bytes()) == s
    assert pickle.loads(pickle.dumps(s)) == s


def test_accessors():
    assert Scalar(7, "int32").as_int() == 7
    assert Scalar(1.5).as_float() == 1.5
    assert Scalar("x").as_str() == "x"
    assert Scalar(True).as_bool() is True
    assert Scalar(7).as_float() is None


def test_temporal_value_is_a_datetime():
    s = Scalar.from_str("1700000000::timestamp[s]")
    assert isinstance(s.value, DateTime)


def test_decimal_value_renders_scaled():
    assert Scalar.from_str("12345::decimal128[7, 2]").value == "123.45"


def test_hash_and_eq_contract():
    # -0.0 == 0.0 and hash equal, NaN == NaN — a Scalar can key a set.
    assert Scalar(0.0) == Scalar(-0.0)
    assert hash(Scalar(0.0)) == hash(Scalar(-0.0))
    assert len({Scalar(1), Scalar(1), Scalar("a")}) == 2


def test_component_map_round_trip():
    s = Scalar(99, "int32")
    mapping = s.to_mapping()
    assert mapping["type"] == "int32"
    assert Scalar.from_mapping(mapping) == s


def test_scalar_arithmetic():
    import pytest

    a, b = Scalar(6), Scalar(4)
    assert (a + b).value == 10
    assert (a - b).value == 2
    assert (a * b).value == 24
    assert (a / b).value == 1
    assert (-a).value == -6
    # mixed int + float promotes to float
    mixed = a + Scalar(1.5)
    assert str(mixed.data_type) == "float64" and mixed.value == 7.5
    # division by zero and an undefined combination raise
    with pytest.raises(ValueError):
        a / Scalar(0)
    with pytest.raises(ValueError):
        Scalar("x") + a
