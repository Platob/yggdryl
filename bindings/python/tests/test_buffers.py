"""Tests for the ByteBuffer / BitBuffer wrappers in the yggdryl Python binding."""

import pytest

from yggdryl import core


def test_byte_buffer_round_trips():
    buf = core.ByteBuffer()
    buf.pwrite_byte_array(0, core.Whence.Start, b"\x01\x02\x03")
    assert buf.byte_size() == 3
    assert buf.bit_size() == 24
    assert buf.pread_byte_one(1, core.Whence.Start) == 2
    assert buf.to_bytes() == b"\x01\x02\x03"


def test_byte_buffer_bit_access_is_msb_first():
    buf = core.ByteBuffer.from_bytes(bytes([0b1010_0000]))
    assert buf.pread_bit_one(0, core.Whence.Start) is True
    assert buf.pread_bit_one(1, core.Whence.Start) is False


def test_byte_buffer_seek_tracks_the_cursor():
    buf = core.ByteBuffer.from_bytes(bytes([10, 20, 30, 40]))
    assert buf.seek(2, core.Whence.Start) == 2
    assert buf.tell() == 2
    assert buf.pread_byte_one(1, core.Whence.Current) == 40


def test_bit_buffer_tracks_an_exact_bit_length():
    buf = core.BitBuffer()
    buf.pwrite_bit_array(0, core.Whence.Start, [True, False, True])
    assert buf.bit_size() == 3
    assert buf.byte_size() == 1
    assert buf.pread_bit_array(0, core.Whence.Start, 3) == [True, False, True]


def test_out_of_bounds_read_raises_value_error():
    buf = core.ByteBuffer.from_bytes(b"\x01\x02")
    with pytest.raises(ValueError):
        buf.pread_byte_array(0, core.Whence.Start, 3)


def test_capacity_and_resize():
    buf = core.ByteBuffer.from_bytes(b"\x01\x02\x03")
    assert buf.byte_capacity() >= 3
    assert buf.resize_byte_capacity(64) >= 64
    assert buf.byte_size() == 3  # capacity never changes the size
    assert buf.bit_capacity() >= 64 * 8

    buf.resize_bytes(5)
    assert buf.to_bytes() == b"\x01\x02\x03\x00\x00"
    buf.resize_bytes(1)
    assert buf.to_bytes() == b"\x01"

    bits = core.BitBuffer()
    bits.resize_bits(3)  # exact bit resize
    assert bits.bit_size() == 3
    assert bits.byte_size() == 1


def test_stream_copy_between_buffers():
    source = core.ByteBuffer.from_bytes(b"\x01\x02\x03\x04")
    sink = core.ByteBuffer()
    source.pread_io(1, core.Whence.Start, 3, sink, 0, core.Whence.Start)
    assert sink.to_bytes() == b"\x02\x03\x04"

    appended = core.ByteBuffer.from_bytes(b"\x09")
    appended.pwrite_io(0, core.Whence.End, source, 0, core.Whence.Start, 2)
    assert appended.to_bytes() == b"\x09\x01\x02"
