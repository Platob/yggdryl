"""Tests for the yggdryl.converter Python binding (dtype-keyed conversion)."""

import pytest

from yggdryl import converter


def test_cast_widens_bytes():
    data = (7).to_bytes(4, "little", signed=True)
    assert converter.cast(data, "i32", "i64") == (7).to_bytes(8, "little", signed=True)


def test_cast_narrows_bytes():
    data = (258).to_bytes(4, "little", signed=True)
    assert converter.cast(data, "i32", "u8") == bytes([2])  # 258 & 0xFF


def test_cast_unknown_dtype_is_guided():
    with pytest.raises(ValueError, match="i8, i16"):
        converter.cast(bytes(4), "i32", "i128")


def test_parse_flexible_integer_formats():
    assert converter.parse("42", "i32") == 42
    assert converter.parse("+42", "i32") == 42
    assert converter.parse("  -7 ", "i32") == -7
    assert converter.parse("0x2A", "i32") == 42
    assert converter.parse("0b101010", "u8") == 42
    assert converter.parse("0o52", "u8") == 42
    assert converter.parse("1_000_000", "i64") == 1_000_000


def test_parse_floats():
    assert converter.parse("1.5e3", "f64") == 1500.0
    assert converter.parse("-0.25", "f32") == -0.25


def test_parse_returns_native_type():
    assert isinstance(converter.parse("5", "i32"), int)
    assert isinstance(converter.parse("5", "f64"), float)


def test_parse_failure_is_guided():
    with pytest.raises(ValueError, match="0x-hex"):
        converter.parse("twelve", "i32")
    with pytest.raises(ValueError):
        converter.parse("-1", "u8")  # out of range for u8


def test_format_round_trips():
    assert converter.format(42, "i32") == "42"
    assert converter.format(-7, "i16") == "-7"
    assert converter.parse(converter.format(-123, "i64"), "i64") == -123


def test_utf8_round_trip_and_validation():
    assert converter.utf8_encode("café") == "café".encode()
    assert converter.utf8_decode("café".encode()) == "café"
    with pytest.raises(ValueError, match="UTF-8"):
        converter.utf8_decode(b"\xff")
