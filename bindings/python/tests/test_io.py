"""Tests for the yggdryl.io Python binding.

IO is split ``std::io.Cursor``-style: a ``ByteBuffer`` is pure storage and a
``ByteCursor`` (from ``ByteBuffer.byte_cursor()``) does the advancing reads/writes.
"""

import pickle

import pytest

from yggdryl import compression, io
from yggdryl.buffer import I32Buffer
from yggdryl.io import (
    ByteBuffer,
    I32Cursor,
    I96Cursor,
    I128Cursor,
    I256Cursor,
    I256Slice,
    Whence,
)


def test_reads_and_writes_advance():
    cur = ByteBuffer().byte_cursor()
    assert cur.pwrite_byte_array(b"hello world", Whence.Start) == 11
    assert cur.tell() == 11  # the write advanced the cursor
    cur.seek(0)
    assert cur.pread_byte_array(5, Whence.Current) == b"hello"
    assert cur.tell() == 5  # the read advanced it too
    assert cur.pread_byte_array(6, Whence.Current) == b" world"
    # byte_size is the *remaining* bytes (0 at the end); the total is the cursor's bytes.
    assert cur.byte_size() == 0
    assert len(cur.as_bytes()) == 11


def test_seek_and_tell_across_origins():
    cur = ByteBuffer(bytes(10)).byte_cursor()
    assert cur.seek(3, Whence.Start) == 3
    assert cur.seek(2, Whence.Current) == 5
    assert cur.seek(-1, Whence.End) == 9
    assert cur.tell() == 9
    cur.set_position(2)
    assert cur.position() == 2


def test_write_is_copy_on_write_leaving_buffer_intact():
    buf = ByteBuffer(b"abcdef")
    cur = buf.byte_cursor()
    cur.pwrite_byte_array(b"XYZ", Whence.Start)
    assert buf.as_bytes() == b"abcdef"  # source untouched
    assert cur.as_bytes() == b"XYZdef"


def test_typed_single_and_array():
    cur = ByteBuffer().byte_cursor()
    assert cur.pwrite_u8_array([0x0A, 0x14, 0x1E, 0x28], Whence.Start) == 4
    cur.seek(1, Whence.Start)
    assert cur.pwrite_one(99, Whence.Current) == 1
    cur.seek(1, Whence.Start)
    assert cur.pread_one(Whence.Current) == 99
    assert cur.pread_u8_array(4, Whence.Start) == [0x0A, 0x63, 0x1E, 0x28]


def test_typed_primitive_round_trip():
    cur = ByteBuffer().byte_cursor()
    cur.pwrite_i64_array([1, 2, 3, -4], Whence.Start)
    cur.seek(0)
    assert cur.pread_i64_array(4, Whence.Current) == [1, 2, 3, -4]


def test_write_past_end_zero_fills():
    cur = ByteBuffer().byte_cursor()
    cur.seek(3, Whence.Start)
    cur.pwrite_byte_array(b"xy", Whence.Current)
    assert cur.as_bytes() == b"\x00\x00\x00xy"


def test_pread_into_fills_reusable_buffer():
    cur = ByteBuffer(b"abcdefgh").byte_cursor()
    scratch = bytearray(4)
    n = cur.pread_into(scratch, Whence.Current)
    assert n == 4
    assert bytes(scratch) == b"abcd"


def test_negative_seek_raises():
    with pytest.raises(ValueError):
        ByteBuffer().byte_cursor().seek(-1, Whence.Start)


def test_value_semantics_and_serialize():
    a = ByteBuffer(b"data")
    b = ByteBuffer(b"data")
    assert a == b
    assert hash(a) == hash(b)
    assert len({a, b, ByteBuffer(b"other")}) == 2
    assert a.serialize_bytes() == b"data"
    assert ByteBuffer.deserialize_bytes(a.serialize_bytes()) == a


def test_pickle_round_trip():
    buf = ByteBuffer(b"payload")
    restored = pickle.loads(pickle.dumps(buf))
    assert restored == buf
    assert repr(restored) == "ByteBuffer(byte_size=7)"


def test_whence_enum():
    assert Whence.Start == 0
    assert Whence.Current == 1
    assert Whence.End == 2
    assert {Whence.Start, Whence.End} == {Whence.Start, Whence.End}


def test_gzip_streams_between_cursors():
    gzip = compression.Gzip(6)
    original = b"stream me " * 500

    source = ByteBuffer(original).byte_cursor()
    packed = ByteBuffer().byte_cursor()
    written = gzip.compress_stream(source, packed)
    # byte_size is the remaining bytes; the total written is the cursor's bytes.
    assert written == len(packed.as_bytes())
    assert len(packed.as_bytes()) < len(original)

    packed.seek(0)
    restored = ByteBuffer().byte_cursor()
    out = gzip.decompress_stream(packed, restored)
    assert out == len(original)
    assert restored.as_bytes() == original


def test_streaming_matches_one_shot():
    gzip = compression.Gzip(9)
    data = b"identical output " * 100
    source = ByteBuffer(data).byte_cursor()
    sink = ByteBuffer().byte_cursor()
    gzip.compress_stream(source, sink)
    assert sink.as_bytes() == gzip.encode_byte_array(data)


def test_bit_seek_and_tell_are_byte_aligned():
    cur = ByteBuffer(bytes(10)).byte_cursor()
    assert cur.bit_tell() == 0
    assert cur.bit_seek(16, Whence.Start) == 16  # byte 2
    assert cur.tell() == 2
    assert cur.bit_tell() == 16
    assert cur.bit_seek(0, Whence.End) == 80  # 10 bytes * 8


def test_unaligned_bit_seek_raises_with_guidance():
    cur = ByteBuffer(bytes(4)).byte_cursor()
    with pytest.raises(ValueError) as excinfo:
        cur.bit_seek(17, Whence.Start)
    message = str(excinfo.value)
    assert "17" in message
    assert "byte-aligned" in message or "multiple of 8" in message


def test_byte_cursor_default_accessors():
    cur = ByteBuffer().byte_cursor()
    assert cur.default_value() == 0
    assert cur.default_byte_array(3) == b"\x00\x00\x00"


def test_typed_cursor_round_trip_and_units():
    cur = I32Buffer([10, 20, 30, 40]).cursor()
    assert cur.tell() == 0
    assert cur.pread_one(Whence.Start) == 10
    assert cur.tell() == 1  # one i32 in
    assert cur.byte_tell() == 4  # four bytes in
    assert cur.seek(2, Whence.Start) == 2
    assert cur.pread_one(Whence.Current) == 30
    assert cur.seek(-1, Whence.End) == 3
    assert cur.pread_one(Whence.Current) == 40
    # size/byte_size are the *remaining* counts (0 at the end).
    assert cur.size() == 0
    assert cur.byte_size() == 0
    cur.seek(0, Whence.Start)
    assert cur.size() == 4  # 4 i32 remaining from the start
    assert cur.byte_size() == 16


def test_typed_cursor_write_past_end_fills_with_default():
    cur = I32Cursor.with_capacity(8)
    cur.pwrite_one(1, Whence.Start)
    cur.seek(3, Whence.Start)  # skip two i32 values
    cur.pwrite_one(9, Whence.Current)
    cur.seek(0, Whence.Start)
    assert cur.size() == 4  # 4 i32 total, from the start
    assert cur.pread_array(4, Whence.Start) == [1, 0, 0, 9]
    assert cur.capacity() >= 8


def test_typed_cursor_negative_seek_raises():
    with pytest.raises(ValueError):
        I32Buffer([1, 2]).cursor().seek(-1, Whence.Start)


def test_typed_cursor_is_copy_on_write():
    buf = I32Buffer([1, 2, 3])
    cur = buf.cursor()
    cur.pwrite_array([9, 9], Whence.Start)
    assert cur.pread_array(3, Whence.Start) == [9, 9, 3]
    assert buf.to_list() == [1, 2, 3]  # source untouched


def test_wide_int_cursors_round_trip_as_python_ints():
    # i96
    c96 = I96Cursor.with_capacity(3)
    c96.pwrite_array([-(2**95), 0, 2**95 - 1], Whence.Start)
    c96.seek(0)
    assert c96.pread_array(3, Whence.Start) == [-(2**95), 0, 2**95 - 1]
    assert len(c96.as_bytes()) == 36  # 12 bytes each

    # i128
    c128 = I128Cursor.with_capacity(2)
    c128.pwrite_one(-(2**127), Whence.Start)
    c128.pwrite_one(2**127 - 1, Whence.Current)
    c128.seek(0)
    assert c128.pread_array(2, Whence.Start) == [-(2**127), 2**127 - 1]

    # i256 — values far beyond i128 round-trip.
    big = 2**200 + 12345
    c256 = I256Cursor.with_capacity(2)
    c256.pwrite_array([big, -big], Whence.Start)
    c256.seek(0)
    assert c256.pread_array(2, Whence.Start) == [big, -big]
    assert len(c256.as_bytes()) == 64  # 32 bytes each


def test_wide_int_cursor_out_of_range_raises():
    c96 = I96Cursor.with_capacity(1)
    with pytest.raises((OverflowError, ValueError)):
        c96.pwrite_one(2**95, Whence.Start)  # one past i96 MAX


def test_byte_slice_is_a_bounded_window():
    buf = ByteBuffer(b"hello world")
    sl = buf.byte_slice(6, 5)  # "world"
    assert sl.slice_offset() == 6
    assert sl.slice_len() == 5
    assert sl.pread_byte_array(100) == b"world"  # clamped to the window
    assert sl.byte_size() == 0
    # Writes are clamped and copy-on-write.
    sl.seek(0)
    assert sl.pwrite_byte_array(b"EARTHLING") == 5
    assert sl.as_bytes() == b"EARTH"
    assert buf.as_bytes() == b"hello world"  # source intact


def test_typed_slice_over_buffer():
    sl = I32Buffer([10, 20, 30, 40, 50]).slice(1, 3)  # [20, 30, 40]
    assert sl.slice_offset() == 4  # byte offset of element 1
    assert sl.slice_len() == 12
    assert sl.size() == 3
    assert sl.pread_array(100) == [20, 30, 40]  # clamped
    sl.seek(-1, Whence.End)
    assert sl.pread_one(Whence.Current) == 40


def test_wide_slice_round_trips():
    big = 2**200 + 7
    sl = I256Slice.from_bytes(big.to_bytes(32, "little", signed=True) * 3, 0, 96)
    assert sl.slice_len() == 96
    assert sl.pread_array(3) == [big, big, big]


def test_io_module_surface():
    for name in (
        "ByteBuffer",
        "ByteCursor",
        "Whence",
        "I8Cursor",
        "U8Cursor",
        "I16Cursor",
        "U16Cursor",
        "I32Cursor",
        "U32Cursor",
        "I64Cursor",
        "U64Cursor",
        "F32Cursor",
        "F64Cursor",
        "I96Cursor",
        "I128Cursor",
        "I256Cursor",
        "ByteSlice",
        "I8Slice",
        "U8Slice",
        "I32Slice",
        "F64Slice",
        "I96Slice",
        "I128Slice",
        "I256Slice",
    ):
        assert hasattr(io, name)
