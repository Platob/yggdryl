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


def test_parse_accepts_comma_separators():
    assert converter.parse("1,000,000", "i64") == 1_000_000
    assert converter.parse("1,234.5", "f64") == 1234.5


def test_parse_out_of_range_shows_value():
    with pytest.raises(ValueError, match="out of range"):
        converter.parse("99999999999", "i32")


def test_convert_numeric_scalars():
    assert converter.convert(300, "i32", "u8") == 44  # 300 & 0xFF (C-style `as`)
    assert converter.convert(3, "i32", "f32") == 3.0
    assert converter.convert(-1, "i32", "i64") == -1
    assert isinstance(converter.convert(5, "i32", "f64"), float)


def test_convert_rejects_out_of_range_input():
    # The from-dtype extraction is strict: the value must fit that dtype (parity
    # with Node's checked extraction).
    with pytest.raises((ValueError, OverflowError)):
        converter.convert(300, "i8", "i16")  # 300 does not fit i8


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


def test_convert_bytes_cast_round_trips():
    wide = converter.convert_bytes((7).to_bytes(4, "little", signed=True), "cast", "i32", "i64")
    assert wide == (7).to_bytes(8, "little", signed=True)
    # invert casts the i64 bytes back to i32.
    assert converter.invert_bytes(wide, "cast", "i32", "i64") == (7).to_bytes(
        4, "little", signed=True
    )


def test_convert_bytes_string_and_invert():
    # "overall" string convert: UTF-8 text bytes -> i32 little-endian bytes.
    le = converter.convert_bytes(b"42", "string", "i32")
    assert le == (42).to_bytes(4, "little", signed=True)
    # invert string: i32 bytes -> decimal text bytes.
    assert converter.invert_bytes(le, "string", "i32") == b"42"


def test_convert_bytes_bytes_and_utf8():
    payload = (258).to_bytes(4, "little", signed=True)
    assert converter.convert_bytes(payload, "bytes", "i32") == payload
    assert converter.invert_bytes(payload, "bytes", "i32") == payload
    assert converter.convert_bytes(b"c", "utf8") == b"c"
    with pytest.raises(ValueError, match="UTF-8"):
        converter.convert_bytes(b"\xff", "utf8")


def test_convert_bytes_is_guided():
    with pytest.raises(ValueError, match="unknown converter"):
        converter.convert_bytes(b"", "nope", "i32")
    with pytest.raises(ValueError, match="needs a to dtype"):
        converter.convert_bytes((7).to_bytes(4, "little"), "cast", "i32")
    with pytest.raises(ValueError, match="needs a dtype"):
        converter.convert_bytes(b"42", "string")
