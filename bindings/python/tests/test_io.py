"""Tests for the ``yggdryl.io`` byte-I/O family: ``Bytes`` and ``Whence``."""

import io as _stdio

import pytest

import yggdryl
from yggdryl.io import Bytes, Whence


def test_module_surface():
    for cls in (Bytes, Whence):
        assert cls.__module__ == "yggdryl.io"
        assert hasattr(yggdryl.io, cls.__name__)


def test_whence_values_match_posix():
    # Same integer meaning as the stdlib SEEK_* constants.
    assert int(Whence.Start) == _stdio.SEEK_SET == 0
    assert int(Whence.Current) == _stdio.SEEK_CUR == 1
    assert int(Whence.End) == _stdio.SEEK_END == 2


def test_construct_len_and_content():
    assert len(Bytes()) == 0
    b = Bytes(b"hello world")
    assert len(b) == 11
    assert b.to_bytes() == b"hello world"
    assert bytes(b) == b"hello world"  # __bytes__
    assert b.position == 0


def test_positioned_pread_pwrite():
    b = Bytes(b"hello world")
    assert b.pread(6, 5) == b"world"
    assert b.pread(6, 100) == b"world"  # short near the end
    assert b.pread(11, 5) == b""  # at the end
    assert b.position == 0  # positioned ops never move the cursor

    assert b.pwrite(6, b"earth") == 5
    assert b.to_bytes() == b"hello earth"
    # Writing past the end grows and zero-fills the gap.
    b2 = Bytes(b"abc")
    assert b2.pwrite(5, b"Z") == 1
    assert b2.to_bytes() == b"abc\x00\x00Z"


def test_cursor_read_write_and_seek():
    b = Bytes()
    assert b.write(b"hello") == 5
    assert b.write(b" world") == 6
    assert b.position == 11
    assert b.to_bytes() == b"hello world"

    assert b.seek(Whence.Start, 6) == 6
    assert b.read(5) == b"world"
    assert b.position == 11
    assert b.seek(Whence.End, -5) == 6
    assert b.read_to_end() == b"world"

    b.rewind()
    assert b.position == 0


def test_read_exact_and_errors():
    b = Bytes(b"hello")
    assert b.pread_exact(1, 3) == b"ell"
    with pytest.raises(ValueError, match="end of data"):
        b.pread_exact(3, 5)  # only 2 remain

    b.seek(Whence.Start, 3)
    with pytest.raises(ValueError):
        b.read_exact(5)  # only 2 remain
    assert b.position == 3  # cursor unchanged on error
    assert b.read_exact(2) == b"lo"
    assert b.position == 5


def test_seek_edges():
    b = Bytes(b"hello")
    with pytest.raises(ValueError, match="before the start"):
        b.seek(Whence.Start, -1)
    # Seeking past the end is allowed; a read there is empty and a write fills the gap.
    assert b.seek(Whence.End, 3) == 8
    assert b.read(4) == b""
    assert b.write(b"Z") == 1
    assert b.to_bytes() == b"hello\x00\x00\x00Z"


def test_slice_is_zero_copy_with_copy_on_write():
    parent = Bytes(b"hello world")
    window = parent.slice(6, 5)
    assert window.to_bytes() == b"world"
    assert len(window) == 5

    # Writing to the slice copies-on-write; the parent is untouched.
    window.pwrite(0, b"WORLD")
    assert window.to_bytes() == b"WORLD"
    assert parent.to_bytes() == b"hello world"

    # Out-of-bounds slice is a guided error.
    with pytest.raises(ValueError, match="past the end"):
        parent.slice(6, 6)


def test_copy_and_equality():
    a = Bytes(b"hello")
    dup = a.copy()
    assert dup == a
    dup.pwrite(0, b"HELLO")
    assert dup.to_bytes() == b"HELLO"
    assert a.to_bytes() == b"hello"  # copy is independent

    # Equality is by content; the cursor is not part of the value.
    other = Bytes(b"hello")
    other.seek(Whence.Start, 3)
    assert a == other
    assert a != Bytes(b"world")


def test_bytes_is_mutable_hence_unhashable():
    # Like bytearray, a mutable buffer is not hashable.
    with pytest.raises(TypeError):
        hash(Bytes(b"x"))


def test_bytes_indexing_and_bool():
    import copy

    b = Bytes(b"hello world")
    assert b[0] == ord("h") and b[-1] == ord("d")  # int index, negative index
    assert b[0:5] == b"hello"  # slice -> bytes
    assert b[6:] == b"world"
    assert b[::-1] == b"dlrow olleh"  # negative step
    assert b[::2] == b"hlowrd"

    with pytest.raises(IndexError):
        _ = b[100]
    with pytest.raises(TypeError):
        _ = b["x"]

    assert bool(b) is True and bool(Bytes()) is False  # __bool__

    # copy / deepcopy are independent (copy-on-write).
    dup = copy.copy(b)
    dup.pwrite(0, b"HELLO")
    assert dup.to_bytes() == b"HELLO world"
    assert b.to_bytes() == b"hello world"
    assert copy.deepcopy(b) == b
