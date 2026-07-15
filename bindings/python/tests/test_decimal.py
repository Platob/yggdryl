"""Tests for the ``yggdryl.decimal`` fixed-width scaled decimals (``D32``/``D64``/``D128``/``D256``).

These mirror the Rust ``io::fixed::decimal`` value-type suite method-for-method: construction,
checked arithmetic, true numeric ordering, value identity (``2.5 == 2.50``), conversions, and the
byte codec / pickle round-trip.
"""

import copy
import decimal
import pickle

import pytest

import yggdryl
from yggdryl.decimal import D32, D64, D128, D256
from yggdryl.types import DataType

ALL = [D32, D64, D128, D256]
BITS = {D32: 32, D64: 64, D128: 128, D256: 256}
MAX_PRECISION = {D32: 9, D64: 18, D128: 38, D256: 76}


def test_module_surface():
    for cls in ALL:
        assert cls.__module__ == "yggdryl.decimal"
        assert hasattr(yggdryl.decimal, cls.__name__)


def test_construct_value_scale_bits():
    for cls in ALL:
        d = cls(12345, 2)  # 123.45
        assert d.coefficient == 12345
        assert d.scale == 2
        assert d.precision == 5
        assert d.bits == BITS[cls]
        assert d.max_precision == MAX_PRECISION[cls]
        assert str(d) == "123.45"
        assert abs(d.to_float() - 123.45) < 1e-9
        assert cls(7).scale == 0  # scale defaults to 0


def test_from_string_and_float():
    for cls in ALL:
        assert cls.from_string("-0.005") == cls(-5, 3)
        assert str(cls.from_string("123.45")) == "123.45"
        assert cls.from_float(1.5, 1) == cls(15, 1)
    with pytest.raises(ValueError):
        D128.from_string("1.2.3")
    with pytest.raises(ValueError, match="non-finite"):
        D128.from_float(float("nan"), 2)


def test_arithmetic_operators_align_scales():
    a = D128(12345, 2)  # 123.45
    b = D128(617, 2)  #     6.17
    assert str(a + b) == "129.62"
    assert str(a - b) == "117.28"
    # Mixed scales align to the larger scale.
    assert str(D64(25, 1) + D64(25, 2)) == "2.75"
    # Multiply sums the scales; unary minus and abs.
    assert str(D64(25, 1) * D64(20, 1)) == "5.00"
    assert str(-a) == "-123.45"
    assert str(abs(D128(-5, 1))) == "0.5"
    # Remainder aligns scales; division takes an explicit result scale.
    assert str(D64(75, 1) % D64(20, 1)) == "1.5"
    assert str(D128(1, 0).div(D128(3, 0), 4)) == "0.3333"


def test_checked_overflow_is_guided():
    with pytest.raises(ValueError, match="overflow"):
        _ = D128(2**126, 0) + D128(2**126, 0)
    # A coefficient that does not fit the width raises on construction.
    with pytest.raises(ValueError, match="wider decimal"):
        D32(3_000_000_000, 0)


def test_identity_is_by_value_across_scales():
    a, b = D128(25, 1), D128(250, 2)  # 2.5 == 2.50
    assert a == b
    assert hash(a) == hash(b)
    assert len({a, b}) == 1  # usable as dict/set keys
    assert a.serialize_bytes() == b.serialize_bytes()
    # Ordering is true numeric order.
    assert D64(25, 1) < D64(275, 2)
    assert sorted([D64(275, 2), D64(25, 1), D64(-1, 0)])[0] == D64(-1, 0)


def test_conversions_and_rescale():
    assert D128(12300, 2).to_int() == 123  # 123.00 is integral
    with pytest.raises(ValueError, match="not an exact integer"):
        D128(12345, 2).to_int()
    assert str(D64(12345, 2).rescale(4)) == "123.4500"
    with pytest.raises(ValueError, match="drop non-zero"):
        D64(12345, 2).rescale(1)
    assert str(D64(12345, 2).round_to_scale(1)) == "123.5"
    assert str(D64(12345, 2).trunc_to_scale(1)) == "123.4"
    assert str(D64(12345, 2).trunc()) == "123"
    assert D64(250, 2).normalized() == D64(25, 1)


def test_cast_between_widths():
    wide = D32(12345, 2).to_d128()
    assert isinstance(wide, D128)
    assert str(wide) == "123.45"
    # Narrowing an out-of-range value raises.
    with pytest.raises(ValueError, match="does not fit"):
        D128(2**100, 0).to_d32()


def test_d256_carries_a_wide_coefficient():
    # A coefficient beyond i128 marshals through the decimal string.
    big = 10**60
    d = D256(big, 5)
    assert d.coefficient == big
    assert d.to_d256().coefficient == big  # identity cast


def test_byte_codec_and_pickle_round_trip():
    for cls in ALL:
        original = cls(-123456789, 4)
        restored = cls.deserialize_bytes(original.serialize_bytes())
        assert restored == original
        assert pickle.loads(pickle.dumps(original)) == original


def test_copy_is_a_snapshot():
    d = D128(12345, 2)
    assert copy.copy(d) == d
    assert copy.deepcopy(d) == d
    assert d.copy() == d


def test_predicates_and_repr():
    assert D128(0, 0).is_zero()
    assert D128(-1, 0).is_negative()
    assert D128(1, 2).is_positive()
    assert repr(D128(12345, 2)) == 'D128("123.45")'  # the value in decimal form


def test_native_python_coercion():
    d = D128(12345, 2)  # 123.45
    assert int(D128(19, 1)) == 1  # int() truncates toward zero (1.9 -> 1)
    assert int(D128(-19, 1)) == -1
    assert float(d) == 123.45
    # A d256 integer beyond 64 bits truncates through the digit string.
    assert int(D256(10**40, 0)) == 10**40


def test_decimal_module_interop():
    d = D128(12345, 2)
    native = d.to_decimal()
    assert isinstance(native, decimal.Decimal)
    assert native == decimal.Decimal("123.45")
    # Round-trips through decimal.Decimal, including scientific notation and a wide d256.
    assert D128.from_decimal(decimal.Decimal("1.5E+3")) == D128(1500, 0)
    assert D128.from_decimal(native) == d
    big = D256(10**60, 5)
    assert D256.from_decimal(big.to_decimal()) == big


def test_datatype_knows_decimals():
    for name, width in [("d32", 4), ("d64", 8), ("d128", 16), ("d256", 32)]:
        dt = DataType.by_name(name)
        assert (dt.name, dt.byte_width, dt.category) == (name, width, "decimal")
        assert dt.is_decimal() and dt.is_numeric() and dt.is_signed()
        assert not dt.is_integer() and not dt.is_floating()
    assert DataType.d128().name == "d128"
    field = DataType.d128().field("amount")
    assert field.is_decimal() and field.type_name == "d128"
