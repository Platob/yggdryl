"""Tests for the ``yggdryl.memory`` in-heap ``Heap`` source and ``Whence`` seek anchor.

Mirrors ``crates/yggdryl-core/tests/memory_heap.rs`` on the Python surface: construction,
size/capacity, the positioned ``pread_*`` / ``pwrite_*`` primitives and typed accessors,
the cursor stream, seeks from every anchor, bounded slices, and the value dunders
(``bytes()`` / ``==`` / ``copy`` / unhashability).
"""

import copy

import pytest

import yggdryl.memory
from yggdryl.memory import Cursor, Heap, Slice, Whence
from yggdryl.uri import Uri


def test_module_surface():
    for cls in (Heap, Whence, Cursor, Slice):
        assert cls.__module__ == "yggdryl.memory"
        assert hasattr(yggdryl.memory, cls.__name__)


# -------------------------------------------------------------------------------------
# Construction + size + capacity
# -------------------------------------------------------------------------------------


def test_construct_empty_and_from_bytes():
    empty = Heap()
    assert empty.byte_size() == 0
    assert empty.is_empty()
    assert len(empty) == 0
    assert not empty  # __bool__

    h = Heap(b"abcd")
    assert h.byte_size() == 4
    assert h.bit_size() == 32
    assert len(h) == 4
    assert not h.is_empty()
    assert h  # __bool__

    # bytearray is accepted as well as bytes.
    assert Heap(bytearray(b"xy")).byte_size() == 2


def test_with_capacity_and_reserve():
    h = Heap.with_capacity(64)
    assert h.is_empty()
    assert h.capacity() >= 64

    cap = h.capacity()
    assert h.pwrite_byte_array(0, b"\x01\x02\x03\x04") == 4
    assert h.byte_size() == 4
    assert h.capacity() == cap  # write within capacity keeps the allocation

    h.reserve(1000)
    assert h.capacity() >= 1004
    assert h.byte_size() == 4  # reserve grows capacity, not size


# -------------------------------------------------------------------------------------
# Positioned byte-array primitives
# -------------------------------------------------------------------------------------


def test_pread_byte_array_short_and_empty():
    h = Heap(b"hello")
    assert h.pread_byte_array(2, 8) == b"llo"  # only 3 remain from offset 2
    assert h.pread_byte_array(2, 2) == b"ll"  # exact
    assert h.pread_byte_array(5, 4) == b""  # at the end
    assert h.pread_byte_array(99, 4) == b""  # past the end


def test_pwrite_byte_array_grows_and_zero_fills():
    h = Heap(b"abc")
    assert h.pwrite_byte_array(5, b"Z") == 1
    assert h.to_bytes() == b"abc\x00\x00Z"  # gap zero-filled
    assert h.pwrite_byte_array(0, b"XY") == 2  # overwrite in place
    assert h.to_bytes() == b"XYc\x00\x00Z"
    assert h.pwrite_byte_array(0, b"") == 0  # empty write is a no-op


# -------------------------------------------------------------------------------------
# Typed positioned accessors: byte / bit / i32 / i64
# -------------------------------------------------------------------------------------


def test_typed_byte_roundtrip_and_eof():
    h = Heap()
    h.pwrite_byte(3, 0xAB)  # grows to 4, zero-filling 0..3
    assert h.to_bytes() == b"\x00\x00\x00\xab"
    assert h.pread_byte(3) == 0xAB
    assert h.pread_byte(0) == 0
    with pytest.raises(ValueError, match="unexpected end of data"):
        h.pread_byte(4)


def test_typed_bit_lsb_first():
    h = Heap(bytes([0b0000_0101, 0b1000_0000]))
    assert h.pread_bit(0)  # byte 0, bit 0
    assert not h.pread_bit(1)
    assert h.pread_bit(2)
    assert h.pread_bit(15)  # byte 1, bit 7 (MSB)
    assert not h.pread_bit(8)
    with pytest.raises(ValueError):
        h.pread_bit(16)  # past the end


def test_typed_bit_write_grows_and_sets():
    h = Heap()
    h.pwrite_bit(10, True)  # byte 1, bit 2 — grows to 2 bytes
    assert h.to_bytes() == bytes([0b0000_0000, 0b0000_0100])
    assert h.pread_bit(10)
    h.pwrite_bit(10, False)  # clear it back
    assert h.to_bytes() == bytes([0, 0])
    h.pwrite_bit(1, True)  # preserve neighbours in the same byte
    h.pwrite_bit(3, True)
    assert h.to_bytes()[0] == 0b0000_1010


def test_typed_i32_i64_little_endian_and_eof():
    h = Heap()
    h.pwrite_i32(0, -42)
    h.pwrite_i64(4, 1234567890123)
    assert h.to_bytes()[:4] == (-42).to_bytes(4, "little", signed=True)
    assert h.pread_i32(0) == -42
    assert h.pread_i64(4) == 1234567890123

    small = Heap(b"abc")  # only 3 bytes
    with pytest.raises(ValueError):
        small.pread_i32(0)
    with pytest.raises(ValueError):
        small.pread_i64(0)


# -------------------------------------------------------------------------------------
# Cursor stream
# -------------------------------------------------------------------------------------


def test_cursor_read_write_advances():
    h = Heap()
    assert h.write(b"hello") == 5
    assert h.position == 5
    h.rewind()
    assert h.position == 0
    assert h.read(5) == b"hello"
    assert h.position == 5
    assert h.read(5) == b""  # at the end


def test_cursor_typed_roundtrip_and_eof_leaves_cursor():
    h = Heap()
    h.write_byte(0x7F)
    h.write_i32(-7)
    h.write_i64(1 << 40)
    assert h.position == 1 + 4 + 8
    h.rewind()
    assert h.read_byte() == 0x7F
    assert h.read_i32() == -7
    assert h.read_i64() == 1 << 40
    pos = h.position
    with pytest.raises(ValueError):
        h.read_byte()  # past the end
    assert h.position == pos  # a failed read must not advance


def test_cursor_read_to_end():
    h = Heap(b"hello world")
    h.seek(Whence.Start, 6)
    assert h.read_to_end() == b"world"
    assert h.position == 11
    assert h.read_to_end() == b""


def test_set_position_and_read():
    h = Heap(b"hello world")
    h.set_position(6)
    assert h.position == 6
    assert h.read(5) == b"world"


# -------------------------------------------------------------------------------------
# Seek / Whence
# -------------------------------------------------------------------------------------


def test_seek_from_all_anchors():
    h = Heap(b"hello world")
    assert h.seek(Whence.Start, 6) == 6
    assert h.seek(Whence.Current, -1) == 5
    assert h.seek(Whence.End, -5) == 6
    assert h.seek(Whence.End, 10) == 21  # past the end is allowed
    with pytest.raises(ValueError, match="invalid seek"):
        h.seek(Whence.Start, -1)  # before the start is not


def test_write_past_seeked_end_zero_fills():
    h = Heap()
    h.seek(Whence.Start, 4)
    h.write(b"Z")
    assert h.to_bytes() == b"\x00\x00\x00\x00Z"


def test_whence_members_and_equality():
    assert Whence.Start == Whence.Start
    assert Whence.Start != Whence.End
    assert {Whence.Start, Whence.Current, Whence.End}  # hashable enum members


# -------------------------------------------------------------------------------------
# Slice
# -------------------------------------------------------------------------------------


def test_slice_window_and_bounds():
    h = Heap(b"hello world")
    world = h.slice(6, 5)
    assert isinstance(world, Heap)
    assert world.byte_size() == 5
    assert world.to_bytes() == b"world"
    assert world.slice(0, 2).to_bytes() == b"wo"  # re-sliceable from its own 0
    with pytest.raises(ValueError, match="runs past the end"):
        h.slice(6, 6)  # 6 + 6 > 11


# -------------------------------------------------------------------------------------
# Value semantics + dunders
# -------------------------------------------------------------------------------------


def test_bytes_and_to_bytes():
    h = Heap(b"payload")
    assert bytes(h) == b"payload"
    assert h.to_bytes() == b"payload"
    assert bytes(Heap()) == b""


def test_equality_ignores_cursor():
    a = Heap(b"same")
    b = Heap(b"same")
    a.set_position(3)  # different cursor
    assert a == b  # equality is over the bytes, not the cursor
    assert Heap(b"same") != Heap(b"diff")


def test_copy_and_stdlib_copy_module():
    base = Heap(b"data")
    dup = base.copy()
    assert dup == base
    dup.pwrite_byte(0, ord("X"))  # mutating the copy leaves the original untouched
    assert base.to_bytes() == b"data"
    assert dup.to_bytes() == b"Xata"

    assert copy.copy(base) == base
    assert copy.deepcopy(base) == base
    indep = copy.copy(base)
    indep.pwrite_byte(0, ord("Y"))
    assert base.to_bytes() == b"data"


def test_repr():
    assert repr(Heap(b"abc")) == "Heap(<3 bytes>)"
    assert repr(Heap()) == "Heap(<0 bytes>)"


def test_heap_is_unhashable_like_bytearray():
    with pytest.raises(TypeError):
        {Heap(b"x")}  # noqa: B018 - mutable buffer must be unhashable
    with pytest.raises(TypeError):
        hash(Heap(b"x"))


# -------------------------------------------------------------------------------------
# Heap address (uri) + copy(uri=...)
# -------------------------------------------------------------------------------------


def test_heap_uri_default_and_set():
    h = Heap(b"x")
    assert isinstance(h.uri, Uri)
    assert h.uri == Uri.parse("")  # empty/opaque by default

    addr = Uri.parse("mem://buf/1")
    h.set_uri(addr)
    assert h.uri == addr
    assert h.uri.host == "buf"


def test_heap_with_uri_is_a_copy():
    h = Heap(b"x")
    addr = Uri.parse("mem://scratch/a")
    named = h.with_uri(addr)
    assert named.uri == addr
    assert h.uri == Uri.parse("")  # original address untouched
    assert named == h  # equality is over the bytes; the address is metadata


def test_heap_copy_with_uri_override():
    h = Heap(b"data").with_uri(Uri.parse("mem://a/1"))
    plain = h.copy()  # no-arg copy keeps the address
    assert plain.uri == Uri.parse("mem://a/1")
    assert plain.to_bytes() == b"data"

    readdressed = h.copy(uri=Uri.parse("mem://b/2"))
    assert readdressed.uri == Uri.parse("mem://b/2")
    assert readdressed.to_bytes() == b"data"
    assert h.uri == Uri.parse("mem://a/1")  # original untouched

    # copy.copy / copy.deepcopy stay plain clones (keep the address).
    assert copy.copy(h).uri == Uri.parse("mem://a/1")
    assert copy.deepcopy(h).uri == Uri.parse("mem://a/1")


# -------------------------------------------------------------------------------------
# Heap.cursor() / Heap.window() view builders
# -------------------------------------------------------------------------------------


def test_heap_cursor_round_trip():
    h = Heap()
    cur = h.cursor()
    assert isinstance(cur, Cursor)
    cur.write_i32(-7)
    cur.write_i64(1 << 40)
    cur.rewind()
    assert cur.read_i32() == -7
    assert cur.read_i64() == 1 << 40
    # The cursor works over an independent copy — the original heap is untouched.
    assert h.byte_size() == 0


def test_heap_window_view_and_bounds():
    h = Heap(b"hello world")
    win = h.window(6, 5)
    assert isinstance(win, Slice)
    assert win.byte_size() == 5
    assert win.offset == 6
    assert win.to_bytes() == b"world"
    with pytest.raises(ValueError, match="runs past the end"):
        h.window(6, 6)  # 6 + 6 > 11


def test_heap_window_clamped_write():
    h = Heap(b"hello world")
    win = h.window(6, 5)  # "world"
    # A write past the window's end is clamped away (fixed-length window).
    assert win.pwrite_byte_array(3, b"ABCDEF") == 2  # only 2 bytes fit (positions 3,4)
    assert win.to_bytes() == b"worAB"


# -------------------------------------------------------------------------------------
# Cursor class directly
# -------------------------------------------------------------------------------------


def test_cursor_construct_and_stream():
    cur = Cursor(b"hello world")
    assert isinstance(cur, Cursor)
    assert len(cur) == 11
    assert cur.byte_size() == 11
    assert cur.bit_size() == 88
    assert cur.position == 0
    assert cur.read(5) == b"hello"
    assert cur.position == 5
    assert cur.seek(Whence.Start, 6) == 6
    assert cur.read_to_end() == b"world"

    empty = Cursor()  # no data -> empty heap
    assert empty.byte_size() == 0
    assert empty.write(b"abc") == 3
    assert bytes(empty) == b"abc"


def test_cursor_over_is_independent_copy():
    h = Heap(b"src")
    cur = Cursor.over(h)
    cur.write_byte(ord("X"))  # position 0
    assert bytes(cur) == b"Xrc"
    assert h.to_bytes() == b"src"  # the source copy is untouched


def test_cursor_typed_and_positioned_and_eof():
    cur = Cursor()
    cur.write_byte(0x7F)
    cur.write_i32(-7)
    assert cur.position == 5
    # Positioned accessors reach any offset without moving the cursor.
    assert cur.pread_byte(0) == 0x7F
    assert cur.pread_i32(1) == -7
    assert cur.position == 5
    cur.pwrite_byte(0, 0x01)
    assert cur.pread_byte(0) == 0x01
    # A failed typed read leaves the cursor put.
    cur.rewind()
    cur.set_position(5)
    with pytest.raises(ValueError):
        cur.read_i32()  # past the end
    assert cur.position == 5


def test_cursor_inner_and_uri_delegate():
    addr = Uri.parse("mem://c/1")
    h = Heap(b"payload").with_uri(addr)
    cur = Cursor.over(h)
    assert cur.uri == addr  # delegates to the wrapped source's address
    inner = cur.inner()
    assert isinstance(inner, Heap)
    assert inner.to_bytes() == b"payload"
    assert "position=0" in repr(cur)


# -------------------------------------------------------------------------------------
# Slice class directly
# -------------------------------------------------------------------------------------


def test_slice_construct_and_read():
    h = Heap(b"hello world")
    win = Slice(h, 6, 5)
    assert isinstance(win, Slice)
    assert len(win) == 5
    assert win.byte_size() == 5
    assert win.offset == 6
    assert win.pread_byte_array(0, 5) == b"world"
    assert win.pread_byte_array(0, 99) == b"world"  # clamped to the window
    assert win.pread_byte(0) == ord("w")
    assert bytes(win) == b"world"
    with pytest.raises(ValueError):
        Slice(h, 6, 6)  # out of bounds


def test_slice_typed_reads_within_window():
    h = Heap()
    h.pwrite_i32(0, 111)
    h.pwrite_i32(4, -222)
    h.pwrite_i64(8, 1 << 40)
    win = Slice(h, 4, 12)  # covers the -222 i32 and the i64
    assert win.pread_i32(0) == -222
    assert win.pread_i64(4) == 1 << 40
    with pytest.raises(ValueError):
        win.pread_i64(8)  # only 4 bytes remain in the window


def test_slice_inner_and_uri_delegate():
    addr = Uri.parse("mem://s/1")
    h = Heap(b"hello world").with_uri(addr)
    win = Slice(h, 0, 5)
    assert win.uri == addr  # delegates to the wrapped source's address
    inner = win.inner()
    assert isinstance(inner, Heap)
    assert inner.to_bytes() == b"hello world"  # inner() is the whole source
    assert "offset=0" in repr(win)
