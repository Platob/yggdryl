"""ByteCursor auto-resize, capacity reduction, and negative-seek edge cases.

Mirrors the core ``tests/io_edge_cases.rs``.
"""

import pytest

from yggdryl.io import ByteBuffer, Whence


def test_write_past_end_auto_grows():
    cursor = ByteBuffer(b"ab").byte_cursor()
    cursor.seek(5, Whence.Start)  # past the 2-byte end
    cursor.pwrite_byte_array(b"XY", Whence.Current)
    assert cursor.as_bytes() == b"ab\x00\x00\x00XY"


def test_append_grows_capacity():
    cursor = ByteBuffer.with_byte_capacity(2).byte_cursor()
    cursor.pwrite_byte_array(bytes(1000), Whence.Start)
    assert cursor.byte_capacity() >= 1000


def test_set_byte_capacity_reserves_above():
    cursor = ByteBuffer(b"abc").byte_cursor()
    assert cursor.set_byte_capacity(128) >= 128
    assert cursor.as_bytes() == b"abc"  # content untouched when growing


def test_set_byte_capacity_reduces_below_length():
    cursor = ByteBuffer(b"abcdefgh").byte_cursor()
    cursor.seek(0, Whence.End)  # position at 8
    cursor.set_byte_capacity(3)  # below length -> reduce the inner buffer
    assert cursor.as_bytes() == b"abc"
    assert cursor.position() == 3  # clamped to the new end


def test_set_byte_capacity_leaves_source_intact():
    buf = ByteBuffer(b"shared")
    cursor = buf.byte_cursor()
    cursor.set_byte_capacity(2)  # copy-on-write reduce
    assert cursor.as_bytes() == b"sh"
    assert buf.as_bytes() == b"shared"


def test_set_bit_capacity_rounds_up():
    cursor = ByteBuffer().byte_cursor()
    assert cursor.set_bit_capacity(17) >= 3  # 17 bits -> 3 bytes


def test_negative_seek_before_start_raises():
    cursor = ByteBuffer(b"abc").byte_cursor()
    with pytest.raises(ValueError, match="before the start"):
        cursor.seek(-1, Whence.Start)


def test_negative_seek_from_end_resolves():
    cursor = ByteBuffer(b"0123456789").byte_cursor()
    assert cursor.seek(-3, Whence.End) == 7
    assert cursor.pread_byte_array(3, Whence.Current) == b"789"
