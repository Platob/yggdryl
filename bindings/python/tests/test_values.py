"""Tests for the ``yggdryl.types`` fixed-width value layer: the per-primitive ``Scalar`` (one
nullable value) and ``Serie`` (one nullable column) wrappers over ``yggdryl_core::io::fixed``.

Value marshaling depends on the element width: small ints (``u8``…``u32``, ``i8``…``i32``) cross as
``int``; wide ints (``u64``/``i64``/``u128``/``i128``) as a decimal ``str``; the 96/256-bit ints as
little-endian ``bytes``; and the floats as ``float``.
"""

import copy
import pickle

import pytest

import yggdryl
from yggdryl.types import (
    DataType,
    F16Scalar,
    F16Serie,
    F64Scalar,
    F64Serie,
    I8Scalar,
    I32Scalar,
    I32Serie,
    I256Scalar,
    I256Serie,
    U8Scalar,
    U64Scalar,
    U64Serie,
    U256Scalar,
    U256Serie,
)

# A representative scalar per marshaling flavor: (class, present value, its type name).
SCALARS = [
    (I32Scalar, -5, "i32"),
    (U8Scalar, 255, "u8"),
    (U64Scalar, "18446744073709551615", "u64"),
    (I256Scalar, (12345).to_bytes(32, "little"), "i256"),
    (F16Scalar, 1.5, "f16"),
    (F64Scalar, -2.25, "f64"),
]


def test_module_surface():
    for cls in (I32Scalar, I32Serie, U256Scalar, U256Serie, F64Scalar, F64Serie):
        assert cls.__module__ == "yggdryl.types"
        assert hasattr(yggdryl.types, cls.__name__)


# ---------------------------------------------------------------------------------------
# Scalar
# ---------------------------------------------------------------------------------------


@pytest.mark.parametrize("cls, value, name", SCALARS)
def test_scalar_present_and_null(cls, value, name):
    present = cls(value)
    assert present.value == value
    assert not present.is_null
    assert present.type_name == name
    assert present.data_type == DataType.by_name(name)

    for null in (cls(), cls(None), cls.null()):
        assert null.is_null
        assert null.value is None


@pytest.mark.parametrize("cls, value, name", SCALARS)
def test_scalar_equality_hash_and_codec(cls, value, name):
    a, b = cls(value), cls(value)
    assert a == b and hash(a) == hash(b)
    assert a != cls.null()
    assert cls.null() == cls.null()
    # Usable as a dict/set key (immutable value type).
    assert {a: 1, b: 2}[cls(value)] == 2
    # Byte codec round-trips (value and null).
    assert cls.deserialize_bytes(a.serialize_bytes()) == a
    assert cls.deserialize_bytes(cls.null().serialize_bytes()) == cls.null()


@pytest.mark.parametrize("cls, value, name", SCALARS)
def test_scalar_pickle_and_copy(cls, value, name):
    original = cls(value)
    assert pickle.loads(pickle.dumps(original)) == original
    assert copy.copy(original) == original
    assert copy.deepcopy(original) == original


def test_scalar_field_and_to_serie():
    scalar = I32Scalar(7)
    field = scalar.field("x", nullable=False)
    assert field.name == "x" and field.type_name == "i32" and field.nullable is False

    column = scalar.to_serie()
    assert len(column) == 1 and column.get(0) == 7
    assert column == I32Serie([7])


def test_scalar_repr():
    assert repr(I32Scalar(7)) == "I32Scalar(7)"
    assert repr(I32Scalar()) == "I32Scalar(null)"


def test_scalar_small_int_range_is_checked():
    with pytest.raises((OverflowError, ValueError)):
        U8Scalar(256)
    with pytest.raises((OverflowError, ValueError)):
        I8Scalar(200)


def test_scalar_wide_int_accepts_str_or_int():
    assert U64Scalar(42).value == "42"  # an int input is coerced to its decimal string
    assert U64Scalar("42").value == "42"
    with pytest.raises(ValueError, match="u64"):
        U64Scalar("not-a-number")


def test_scalar_wide_bytes_width_is_checked():
    assert U256Scalar(bytes(32)).value == bytes(32)
    with pytest.raises(ValueError, match="little-endian bytes"):
        U256Scalar(bytes(8))


# ---------------------------------------------------------------------------------------
# Serie
# ---------------------------------------------------------------------------------------


def test_serie_construction_and_access():
    col = I32Serie([1, None, 3])
    assert len(col) == 3
    assert col.null_count == 1 and col.has_nulls
    assert col.to_options() == [1, None, 3]
    assert list(col) == [1, None, 3]
    assert col.get(0) == 1 and col.get(1) is None
    assert col[0] == 1 and col[-1] == 3  # indexing, negatives allowed
    with pytest.raises(IndexError):
        col[3]

    empty = I32Serie()
    assert len(empty) == 0 and not empty and empty.is_empty()

    dense = I32Serie.from_values([1, 2, 3])
    assert dense.null_count == 0 and not dense.has_nulls


def test_serie_mutation():
    col = I32Serie([1, None, 3])
    col.push(4)
    col.push(None)
    assert col.to_options() == [1, None, 3, 4, None]
    col.set(1, 20)
    assert col.to_options() == [1, 20, 3, 4, None]
    with pytest.raises(ValueError):
        col.set(99, 0)


def test_serie_scalar_interop():
    col = I32Serie([1, None, 3])
    assert col.get_scalar(0) == I32Scalar(1)
    assert col.get_scalar(1) == I32Scalar.null()
    assert col.get_scalar(99) == I32Scalar.null()  # out of range -> null
    assert I32Serie.from_values([7]).as_scalar() == I32Scalar(7)
    assert I32Serie([1, 2]).as_scalar() is None
    assert I32Serie.from_scalar(I32Scalar(9)) == I32Serie([9])


def test_serie_field_infers_nullability():
    with_nulls = I32Serie([1, None])
    dense = I32Serie([1, 2])
    assert with_nulls.to_field("c").nullable is True
    assert dense.to_field("c").nullable is False
    assert dense.field("c", nullable=True).nullable is True
    assert with_nulls.data_type == DataType.i32()


def test_serie_codec_and_pickle():
    col = I32Serie([1, None, 3])
    col.set(1, 2)  # clears the last null -> still round-trips byte-equal (canonical identity)
    assert I32Serie.deserialize_bytes(col.serialize_bytes()) == col
    assert pickle.loads(pickle.dumps(col)) == col
    assert copy.deepcopy(col) == col


def test_serie_is_unhashable_because_mutable():
    with pytest.raises(TypeError):
        hash(I32Serie([1, 2]))


def test_serie_copy_is_independent():
    original = I32Serie([1, 2, 3])
    dup = original.copy()
    dup.push(4)
    assert len(original) == 3 and len(dup) == 4


@pytest.mark.parametrize(
    "cls, values",
    [
        (I32Serie, [1, None, -3]),
        (U64Serie, ["0", None, "18446744073709551615"]),
        (I256Serie, [(1).to_bytes(32, "little"), None, (2).to_bytes(32, "little")]),
        (F64Serie, [1.5, None, -2.25]),
    ],
)
def test_serie_round_trip_across_flavors(cls, values):
    col = cls(values)
    assert col.to_options() == values
    assert cls.deserialize_bytes(col.serialize_bytes()) == col
    assert pickle.loads(pickle.dumps(col)) == col
