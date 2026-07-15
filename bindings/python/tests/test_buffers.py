"""Tests for the ``yggdryl.types`` typed buffers: ``I8Buffer`` … ``F64Buffer`` (a contiguous,
non-nullable values store over ``yggdryl_core::io::fixed``'s ``Buffer<T>``)."""

import copy

import pytest

import yggdryl
from yggdryl.types import DataType, F64Buffer, I32Buffer, U8Buffer, U256Buffer


def test_module_surface():
    for cls in (I32Buffer, U256Buffer, F64Buffer):
        assert cls.__module__ == "yggdryl.types"
        assert hasattr(yggdryl.types, cls.__name__)


def test_construction_and_access():
    b = I32Buffer([1, 2, 3])
    assert b.count == 3 and len(b) == 3 and bool(b)
    assert b.get(0) == 1 and b[2] == 3 and b[-1] == 3
    assert b.get(99) is None  # out of range -> None (not an error)
    assert b.to_values() == [1, 2, 3] and list(b) == [1, 2, 3]
    assert not I32Buffer()

    with pytest.raises(IndexError):
        b[3]


def test_mutation():
    b = I32Buffer([1, 2, 3])
    b.push(4)
    b.set(1, 20)
    assert b.to_values() == [1, 20, 3, 4]
    with pytest.raises(IndexError):
        b.set(99, 0)


def test_byte_codec_and_bytes_protocol():
    b = I32Buffer([1, 2, 3])
    assert I32Buffer.from_bytes(b.to_bytes()) == b
    assert bytes(b) == b.to_bytes()
    assert len(b.to_bytes()) == 12  # 3 * 4 bytes


def test_equality_copy_and_descriptor():
    a = I32Buffer([1, 2, 3])
    assert a == I32Buffer([1, 2, 3]) and a != I32Buffer([1, 2])
    dup = a.copy()
    dup.push(4)
    assert len(a) == 3 and len(dup) == 4
    assert a.data_type == DataType.i32()
    assert a.field("c", nullable=False).type_name == "i32"


def test_across_flavors():
    assert U8Buffer([1, 255]).to_values() == [1, 255]
    assert F64Buffer([1.5, -2.5]).to_values() == [1.5, -2.5]
    # wide 256-bit values cross as little-endian bytes
    w = U256Buffer([(5).to_bytes(32, "little"), (9).to_bytes(32, "little")])
    assert w.count == 2 and int.from_bytes(w[1], "little") == 9
    assert U256Buffer.from_bytes(w.to_bytes()) == w


def test_deepcopy():
    b = I32Buffer([1, 2, 3])
    assert copy.deepcopy(b) == b
