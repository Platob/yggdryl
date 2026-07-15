"""Tests for numeric casting on the fixed value types: the per-target ``to_<type>`` methods on
each numeric ``Scalar`` / ``Serie`` (over ``yggdryl_core::io::Converter``), plus the universal
UTF-8 / binary bridges (``to_utf8`` / ``to_binary`` and their reverse on ``Utf8Scalar`` /
``BinaryScalar``).
"""

import pytest

from yggdryl.types import (
    BinaryScalar,
    F64Scalar,
    I32Scalar,
    I32Serie,
    U8Scalar,
    Utf8Scalar,
)


def test_numeric_scalar_cast():
    assert I32Scalar(300).to_i64().value == "300"  # widen (i64 crosses as a string)
    assert I32Scalar(65).to_u8().value == 65
    assert I32Scalar(300).to_f64().value == 300.0  # precision-lossy path
    assert U8Scalar(255).to_i32().value == 255


def test_out_of_range_cast_is_guided():
    with pytest.raises(ValueError, match="out of range"):
        I32Scalar(300).to_u8()  # 300 > u8::MAX
    with pytest.raises(ValueError, match="out of range"):
        I32Scalar(-1).to_u8()


def test_null_casts_to_null():
    assert I32Scalar().to_i64().is_null
    assert I32Scalar().to_f64().is_null
    assert I32Scalar().to_utf8().is_null


def test_serie_cast_preserves_nulls():
    wide = I32Serie([1, None, 3]).to_i64()
    assert wide.to_options() == ["1", None, "3"]
    assert wide.data_type.name == "i64"
    floats = I32Serie([1, None, 3]).to_f64()
    assert floats.to_options() == [1.0, None, 3.0]
    with pytest.raises(ValueError):
        I32Serie([1, 300]).to_u8()  # 300 out of range


def test_utf8_and_binary_bridges():
    # any -> utf8 -> any
    assert I32Scalar(42).to_utf8().value == "42"
    assert Utf8Scalar("42").to_i32().value == 42
    assert I32Scalar(-7).to_utf8().to_i32().value == -7
    with pytest.raises(ValueError, match="parse"):
        Utf8Scalar("nope").to_i32()

    # any -> binary -> any (canonical little-endian bytes)
    b = I32Scalar(-7).to_binary()
    assert b.type_name == "binary" and len(b.value) == 4
    assert BinaryScalar(b.value).to_i32().value == -7
    with pytest.raises(ValueError, match="bytes"):
        BinaryScalar(b"\x01").to_i32()  # width mismatch (1 != 4)


def test_float_bridges():
    assert F64Scalar(1.5).to_utf8().value == "1.5"
    assert Utf8Scalar("1.5").to_f64().value == 1.5
