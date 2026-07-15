"""Tests for the ``yggdryl.types`` variable-length value layer: ``Utf8Scalar`` / ``Utf8Serie``
(strings) and ``BinaryScalar`` / ``BinarySerie`` (byte strings), over ``yggdryl_core::io::var``.

A UTF-8 value crosses as ``str``; a binary value as ``bytes``. A ``Scalar`` is an immutable value
(hashable, pickles); a ``Serie`` is a mutable column (unhashable) whose per-element ``set`` may
rewrite trailing offsets.
"""

import copy
import pickle

import pytest

import yggdryl
from yggdryl.types import (
    BinaryScalar,
    BinarySerie,
    DataType,
    Utf8Scalar,
    Utf8Serie,
)


def test_module_surface():
    for cls in (Utf8Scalar, Utf8Serie, BinaryScalar, BinarySerie):
        assert cls.__module__ == "yggdryl.types"
        assert hasattr(yggdryl.types, cls.__name__)


# ---------------------------------------------------------------------------------------
# Scalar
# ---------------------------------------------------------------------------------------


def test_utf8_scalar():
    s = Utf8Scalar("héllo")
    assert s.value == "héllo" and not s.is_null and s.type_name == "utf8"
    assert s.data_type == DataType.utf8()
    for null in (Utf8Scalar(), Utf8Scalar(None), Utf8Scalar.null()):
        assert null.is_null and null.value is None
    assert s == Utf8Scalar("héllo") and hash(s) == hash(Utf8Scalar("héllo"))
    assert s != Utf8Scalar("other") and s != Utf8Scalar.null()
    assert {s: 1, Utf8Scalar("héllo"): 2}[Utf8Scalar("héllo")] == 2  # dict key


def test_binary_scalar():
    raw = bytes([0xFF, 0x00, 0x41])
    b = BinaryScalar(raw)
    assert b.value == raw and b.type_name == "binary"
    assert b.data_type == DataType.binary()
    assert b == BinaryScalar(raw) and b != BinaryScalar(b"other")


@pytest.mark.parametrize(
    "cls, value",
    [(Utf8Scalar, "hi"), (Utf8Scalar, ""), (BinaryScalar, b"\x00\xff"), (BinaryScalar, b"")],
)
def test_scalar_codec_pickle_copy(cls, value):
    original = cls(value)
    assert cls.deserialize_bytes(original.serialize_bytes()) == original
    assert cls.deserialize_bytes(cls.null().serialize_bytes()) == cls.null()
    assert pickle.loads(pickle.dumps(original)) == original
    assert copy.deepcopy(original) == original


def test_scalar_field():
    field = Utf8Scalar("x").field("name", nullable=False)
    assert field.name == "name" and field.type_name == "utf8" and field.nullable is False


def test_invalid_utf8_is_rejected():
    # A binary blob of invalid UTF-8, deserialized as a Utf8Scalar, is a guided error.
    bad = BinaryScalar(bytes([0xFF, 0xFE])).serialize_bytes()
    with pytest.raises(ValueError):
        Utf8Scalar.deserialize_bytes(bad)


# ---------------------------------------------------------------------------------------
# Serie
# ---------------------------------------------------------------------------------------


def test_utf8_serie():
    col = Utf8Serie(["a", None, "cd"])
    assert len(col) == 3 and col.null_count == 1 and col.has_nulls
    assert col.to_options() == ["a", None, "cd"] and list(col) == ["a", None, "cd"]
    assert col[0] == "a" and col[-1] == "cd" and col.get(1) is None
    with pytest.raises(IndexError):
        col[3]
    assert col.get_scalar(0) == Utf8Scalar("a")
    assert col.get_scalar(1) == Utf8Scalar.null()
    assert Utf8Serie().is_empty() and not Utf8Serie()


def test_serie_mutation_rewrites_offsets():
    col = Utf8Serie(["a", "bb", "ccc"])
    col.set(1, "longer")  # grows -> trailing offsets shift
    col.set(2, None)  # -> null, slot shrinks
    col.push("z")
    assert col.to_options() == ["a", "longer", None, "z"]
    with pytest.raises(ValueError):
        col.set(99, "x")


def test_binary_serie():
    col = BinarySerie([b"\x01", None, b"\xff\xfe"])
    assert col.to_options() == [b"\x01", None, b"\xff\xfe"]
    assert col.data_len == 3  # 1 + 0 + 2 value bytes
    assert col.data_type == DataType.binary()


@pytest.mark.parametrize(
    "cls, values",
    [
        (Utf8Serie, ["a", None, "cd", ""]),
        (BinarySerie, [b"\x01", None, b"\xff\xfe", b""]),
    ],
)
def test_serie_codec_and_pickle(cls, values):
    col = cls(values)
    assert cls.deserialize_bytes(col.serialize_bytes()) == col
    assert pickle.loads(pickle.dumps(col)) == col
    # Clearing the last null still round-trips byte-equal (canonical identity).
    col.set(1, values[0])
    assert cls.deserialize_bytes(col.serialize_bytes()) == col


def test_serie_field_infers_nullability():
    assert Utf8Serie(["a", None]).to_field("c").nullable is True
    assert Utf8Serie(["a", "b"]).to_field("c").nullable is False
    assert Utf8Serie(["a"]).field("c", nullable=True).nullable is True


def test_serie_is_unhashable_and_copy_is_independent():
    with pytest.raises(TypeError):
        hash(Utf8Serie(["a"]))
    original = Utf8Serie(["a", "b"])
    dup = original.copy()
    dup.push("c")
    assert len(original) == 2 and len(dup) == 3
