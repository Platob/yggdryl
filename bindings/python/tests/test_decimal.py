"""Tests for the ``yggdryl.decimal`` fixed-width decimals."""

import pickle

import pytest

from yggdryl.decimal import Decimal32, Decimal64, Decimal128, Decimal256

ALL = [Decimal32, Decimal64, Decimal128, Decimal256]
BITS = {Decimal32: 32, Decimal64: 64, Decimal128: 128, Decimal256: 256}


def test_module_surface():
    import yggdryl

    for cls in ALL:
        assert cls.__module__ == "yggdryl.decimal"
        assert hasattr(yggdryl.decimal, cls.__name__)


def test_construct_value_scale_bits():
    for cls in ALL:
        d = cls(12345, 2)  # 123.45
        assert d.mantissa == 12345
        assert d.scale == 2
        assert d.bits == BITS[cls]
        assert abs(d.to_f64() - 123.45) < 1e-9
        assert d.to_i128() == 123  # truncates toward zero
        assert cls(7).scale == 0  # scale defaults to 0


def test_from_f64():
    for cls in ALL:
        assert cls.from_f64(1.5, 1) == cls(15, 1)


def test_rescale_and_overflow():
    d = Decimal64(123, 0)
    assert d.rescale(2) == Decimal64(12300, 2)
    assert d.rescale(2).rescale(0) == d  # exact round-trip

    # Rescaling past the width raises a guided error naming the remedy.
    with pytest.raises(ValueError, match="wider decimal"):
        Decimal32(2_000_000_000, 0).rescale(2)


def test_constructor_range_check_is_guided():
    # A mantissa that does not fit the width raises, not silently truncates.
    with pytest.raises(ValueError, match="wider decimal"):
        Decimal32(2**40, 0)
    with pytest.raises(ValueError, match="wider decimal"):
        Decimal64(2**80, 0)


def test_byte_round_trip_and_length():
    widths = {Decimal32: 5, Decimal64: 9, Decimal128: 17, Decimal256: 33}
    for cls, n in widths.items():
        d = cls(-4200, 2)  # -42.00
        raw = d.serialize_bytes()
        assert len(raw) == n  # mantissa + scale byte
        assert cls.deserialize_bytes(raw) == d

    with pytest.raises(ValueError, match="expected 5"):
        Decimal32.deserialize_bytes(bytes([0, 0, 0]))


def test_value_semantics_and_pickle():
    for cls in ALL:
        a = cls(12345, 2)
        assert a == cls(12345, 2)
        assert a != cls(12345, 3)  # equal iff bytes equal (rule 7): scale matters
        assert hash(a) == hash(cls(12345, 2))
        assert len({cls(12345, 2), cls(12345, 2), cls(1, 0)}) == 2
        assert pickle.loads(pickle.dumps(a)) == a  # __reduce__ round-trip


def test_str_and_repr():
    assert str(Decimal64(123456, 3)) == "123.456"
    assert str(Decimal64(-5, 2)) == "-0.05"
    assert repr(Decimal32(12345, 2)) == "Decimal32(12345, 2)"


def test_cross_width_widen_and_narrow():
    d32 = Decimal32(12345, 2)  # 123.45
    assert d32.to_decimal256() == Decimal256(12345, 2)
    assert Decimal64(999, 1).to_decimal256() == Decimal256(999, 1)
    assert Decimal128(999, 1).to_decimal256() == Decimal256(999, 1)

    # Narrow 256 -> 128 when it fits.
    assert Decimal256(999, 1).try_to_decimal128() == Decimal128(999, 1)
    # Narrow fails (guided) when the mantissa exceeds i128.
    huge = Decimal256(2 * 2**126, 0)  # > i128 max
    with pytest.raises(ValueError, match="wider decimal"):
        huge.try_to_decimal128()


def test_decimal256_beyond_i128():
    # A 256-bit mantissa that no native int type holds round-trips through the string bridge.
    mantissa = 2**200 + 123
    d = Decimal256(mantissa, 3)
    assert d.mantissa == mantissa  # exact, via the decimal-string bridge
    assert d.to_i128() is None  # integer part exceeds i128
    assert Decimal256.deserialize_bytes(d.serialize_bytes()) == d

    # Negative 256-bit magnitudes survive too (two's-complement vs sign-magnitude).
    neg = Decimal256(-(2**200), 0)
    assert neg.mantissa == -(2**200)
    assert Decimal256.deserialize_bytes(neg.serialize_bytes()) == neg


def test_decimal256_range_and_type_errors():
    with pytest.raises(ValueError, match="out of range for decimal256"):
        Decimal256(2**256, 0)  # exceeds 256-bit signed range
    with pytest.raises(TypeError, match="int mantissa"):
        Decimal256("nope", 0)
