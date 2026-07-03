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


def test_byte_buffer_current_is_measured_from_the_start_without_a_cursor():
    buf = core.ByteBuffer.from_bytes(bytes([10, 20, 30, 40]))
    # A bare buffer keeps no cursor, so Current == Start.
    assert buf.pread_byte_one(1, core.Whence.Current) == 20
    assert buf.pread_byte_one(1, core.Whence.Start) == 20


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


def test_byte_buffer_capacity_and_resize():
    buf = core.ByteBuffer.from_bytes(b"\x01\x02\x03")
    assert buf.byte_capacity() >= 3
    assert buf.resize_byte_capacity(64) >= 64
    assert buf.resize_bit_capacity(1024) >= 1024
    assert buf.byte_size() == 3  # capacity never changes the size

    buf.resize_bytes(5)
    assert buf.to_bytes() == b"\x01\x02\x03\x00\x00"
    buf.resize_bytes(1)
    assert buf.to_bytes() == b"\x01"

    # ByteBuffer bit resizes round up to whole bytes.
    buf.resize_bits(9)
    assert buf.byte_size() == 2
    assert buf.bit_size() == 16


def test_bit_buffer_capacity_and_exact_bit_resize():
    buf = core.BitBuffer.from_bytes(b"\xff\xff")
    assert buf.byte_capacity() >= 2
    assert buf.bit_capacity() >= 16
    assert buf.resize_byte_capacity(32) >= 32

    buf.resize_bytes(1)  # sets bit_size to 8
    assert buf.bit_size() == 8

    buf.resize_bits(3)  # exact — and truncation zeroes padding
    assert buf.bit_size() == 3
    assert buf.byte_size() == 1
    assert buf.to_bytes() == bytes([0b1110_0000])


def test_byte_buffer_cursor_advances_over_a_copy():
    buf = core.ByteBuffer.from_bytes(bytes([10, 20, 30, 40]))
    cursor = buf.cursor()
    assert cursor.pread_byte_array(0, core.Whence.Current, 2) == bytes([10, 20])
    assert cursor.tell() == 2
    assert cursor.pread_byte_array(0, core.Whence.Current, 2) == bytes([30, 40])
    assert cursor.tell() == 4
    # The cursor holds a copy: writing through it leaves the original buffer intact.
    cursor.seek(0, core.Whence.Start)
    cursor.pwrite_byte_one(0, core.Whence.Current, 99)
    assert cursor.to_bytes() == bytes([99, 20, 30, 40])
    assert buf.to_bytes() == bytes([10, 20, 30, 40])


def test_byte_buffer_cursor_from_bytes():
    cursor = core.ByteBufferCursor.from_bytes(bytes([1, 2, 3]))
    assert cursor.byte_size() == 3
    assert cursor.seek(1, core.Whence.Start) == 1
    assert cursor.pread_byte_one(0, core.Whence.Current) == 2


def test_byte_buffer_slice_bounds_a_window():
    buf = core.ByteBuffer.from_bytes(bytes([10, 20, 30, 40, 50]))
    sliced = buf.slice(1, 4)
    assert sliced.byte_size() == 3
    assert sliced.start() == 1
    assert sliced.end() == 4
    assert sliced.pread_byte_array(0, core.Whence.Start, 3) == bytes([20, 30, 40])
    # Access outside the window raises.
    with pytest.raises(ValueError):
        sliced.pread_byte_array(0, core.Whence.Start, 4)
    # Writes stay in-window and reach the slice's copy of the buffer.
    sliced.pwrite_byte_one(0, core.Whence.Start, 99)
    assert sliced.to_bytes() == bytes([10, 99, 30, 40, 50])


def test_byte_buffer_slice_from_bytes():
    sliced = core.ByteBufferSlice.from_bytes(bytes([1, 2, 3, 4]), 1, 3)
    assert sliced.byte_size() == 2
    assert sliced.pread_byte_array(0, core.Whence.Start, 2) == bytes([2, 3])


def test_bit_buffer_exposes_cursor_and_slice():
    cursor = core.BitBuffer.from_bytes(bytes([0xFF])).cursor()
    assert cursor.bit_size() == 8
    assert cursor.pread_bit_one(0, core.Whence.Current) is True

    sliced = core.BitBuffer.from_bytes(bytes([1, 2, 3])).slice(1, 2)
    assert sliced.byte_size() == 1
    assert sliced.pread_byte_one(0, core.Whence.Start) == 2  # window byte 1 of the inner


def test_primitive_helpers_round_trip_little_endian():
    buf = core.ByteBuffer()
    buf.pwrite_i64(0, core.Whence.Start, -2)
    buf.pwrite_u16(8, core.Whence.Start, 0xBEEF)
    buf.pwrite_f64(10, core.Whence.Start, 1.5)
    assert buf.pread_i64(0, core.Whence.Start) == -2
    assert buf.pread_u16(8, core.Whence.Start) == 0xBEEF
    assert buf.pread_f64(10, core.Whence.Start) == 1.5
    # Little-endian: the low byte comes first.
    buf.pwrite_u32(0, core.Whence.Start, 1)
    assert buf.pread_byte_one(0, core.Whence.Start) == 1
