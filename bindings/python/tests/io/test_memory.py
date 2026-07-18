"""Tests for the ``yggdryl.memory`` ``Heap`` source and ``Whence`` seek anchor.

Mirrors ``crates/yggdryl-core/tests/memory_heap.rs`` on the Python surface: construction,
size/capacity, the positioned ``pread_*`` / ``pwrite_*`` primitives and typed accessors
(including UTF-8 text, the bulk ``i32``/``i64`` arrays, and repeated fills), the cursor
stream, seeks from every anchor, bounded slices, the source metadata (``headers`` /
``mode`` / ``kind`` and the ``is_file`` / ``is_dir`` / ``exists`` predicates), the byte
codec + pickle, the value dunders (``bytes()`` / ``==`` / ``copy`` / unhashability), and
the leaf graph surface (``name`` / ``parent`` / the empty ``ls`` stream / ``children`` /
the guided ``rm`` family — ``IOBase`` is the central access path, and the in-memory
sources are leaves of the IO graph). The on-disk sources moved to ``yggdryl.local`` (see
``tests/io/test_local.py``).
"""

import copy
import io
import pickle

import pytest

import yggdryl.memory
from yggdryl.compression import Gzip, Zstd
from yggdryl.dtype import DataTypeId
from yggdryl.headers import Headers
from yggdryl.io import IOKind, IOMode
from yggdryl.mediatype import MediaType
from yggdryl.mimetype import MimeType
from yggdryl.memory import Cursor, Heap, NoChildren, Slice, Whence
from yggdryl.uri import Uri


def test_module_surface():
    for cls in (Heap, Whence, Cursor, Slice, NoChildren):
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
# Heap address (uri)
# -------------------------------------------------------------------------------------


def test_heap_uri_is_always_the_synthetic_mem_heap():
    # A heap stores no address: every heap reports the stable synthetic mem://heap.
    h = Heap(b"x")
    assert isinstance(h.uri, Uri)
    assert str(h.uri) == "mem://heap"
    assert h.uri.scheme == "mem"
    assert h.uri.host == "heap"
    assert str(Heap().uri) == "mem://heap"
    # There is deliberately no setter (an anonymous in-memory buffer has no other identity).
    assert not hasattr(h, "set_uri")
    assert not hasattr(h, "with_uri")


def test_heap_copy_is_a_plain_clone():
    h = Heap(b"data")
    plain = h.copy()
    assert plain == h
    assert plain.to_bytes() == b"data"
    # copy.copy / copy.deepcopy behave identically.
    assert copy.copy(h) == h
    assert copy.deepcopy(h) == h


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
    h = Heap(b"payload")
    cur = Cursor.over(h)
    assert str(cur.uri) == "mem://heap"  # delegates to the wrapped source's address
    inner = cur.inner()
    assert isinstance(inner, Heap)
    assert inner.to_bytes() == b"payload"
    assert "position=0" in repr(cur)


# -------------------------------------------------------------------------------------
# Slice class directly
# -------------------------------------------------------------------------------------


def test_slice_over_matches_constructor():
    h = Heap(b"hello world")
    win = Slice.over(h, 6, 5)
    assert isinstance(win, Slice)
    assert win.byte_size() == 5
    assert win.offset == 6
    assert win.to_bytes() == b"world"
    win.pwrite_byte_array(0, b"W")  # over() wraps an independent copy
    assert h.to_bytes() == b"hello world"
    with pytest.raises(ValueError, match="runs past the end"):
        Slice.over(h, 6, 6)  # same bounds check as the constructor


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
    h = Heap(b"hello world")
    win = Slice(h, 0, 5)
    assert str(win.uri) == "mem://heap"  # delegates to the wrapped source's address
    inner = win.inner()
    assert isinstance(inner, Heap)
    assert inner.to_bytes() == b"hello world"  # inner() is the whole source
    assert "offset=0" in repr(win)


# -------------------------------------------------------------------------------------
# Heap metadata: headers / mode / kind
# -------------------------------------------------------------------------------------


def test_heap_headers_getter_returns_a_copy():
    h = Heap(b"x")
    assert isinstance(h.headers, Headers)
    assert len(h.headers) == 0

    grabbed = h.headers
    grabbed.insert("a", "1")  # mutating the returned copy...
    assert len(h.headers) == 0  # ...does not touch the heap until written back
    h.set_headers(grabbed)
    assert h.headers.get("a") == "1"


def test_heap_with_headers_is_a_copy():
    meta = Headers().with_("Content-Type", "text/plain")
    h = Heap(b"x")
    tagged = h.with_headers(meta)
    assert tagged.headers.content_type() == "text/plain"
    assert len(h.headers) == 0  # the original is untouched
    assert tagged == h  # equality is over the bytes; headers are metadata


def test_heap_mode_default_set_and_with():
    h = Heap(b"x")
    assert h.mode == IOMode.ReadWrite  # in-memory default
    h.set_mode(IOMode.Read)
    assert h.mode == IOMode.Read

    readonly = Heap(b"x").with_mode(IOMode.Append)
    assert readonly.mode == IOMode.Append
    assert Heap(b"x").mode == IOMode.ReadWrite  # with_mode never mutated a fresh heap


def test_heap_kind_is_heap():
    assert Heap().kind == IOKind.Heap
    assert Heap(b"x").kind == IOKind.Heap


def test_cursor_and_slice_metadata_delegate():
    meta = Headers().with_("a", "1")
    h = Heap(b"hello world").with_headers(meta).with_mode(IOMode.Read)

    cur = Cursor.over(h)
    assert cur.headers.get("a") == "1"
    assert cur.mode == IOMode.Read
    assert cur.kind == IOKind.Heap

    win = Slice(h, 0, 5)
    assert win.headers.get("a") == "1"
    assert win.mode == IOMode.Read
    assert win.kind == IOKind.Heap


# -------------------------------------------------------------------------------------
# UTF-8 text: positioned + cursor-style
# -------------------------------------------------------------------------------------


def test_pread_pwrite_utf8_round_trip_and_invalid():
    h = Heap()
    assert h.pwrite_utf8(0, "héllo") == 6  # é is 2 bytes
    assert h.pread_utf8(0, 6) == "héllo"
    assert h.pread_utf8(0, 100) == "héllo"  # clamped near the end
    with pytest.raises(ValueError, match="invalid UTF-8"):
        h.pread_utf8(0, 2)  # cuts the 2-byte é in half


def test_heap_cursor_style_utf8_advances_by_bytes():
    h = Heap()
    assert h.write_utf8("héllo") == 6
    assert h.position == 6
    h.rewind()
    assert h.read_utf8(6) == "héllo"
    assert h.position == 6
    h.set_position(1)
    with pytest.raises(ValueError, match="invalid UTF-8"):
        h.read_utf8(1)  # lands inside é
    assert h.position == 1  # a failed read leaves the cursor put


def test_cursor_class_utf8():
    cur = Cursor()
    assert cur.write_utf8("héllo") == 6
    cur.rewind()
    assert cur.read_utf8(6) == "héllo"
    # Positioned UTF-8 reaches any offset without moving the cursor.
    assert cur.pread_utf8(1, 2) == "é"
    assert cur.position == 6
    assert cur.pwrite_utf8(0, "H") == 1
    assert cur.pread_utf8(0, 1) == "H"


def test_slice_pread_utf8_within_window():
    h = Heap()
    h.pwrite_utf8(0, "hello world")
    win = Slice(h, 6, 5)
    assert win.pread_utf8(0, 5) == "world"
    assert win.pread_utf8(0, 100) == "world"  # clamped to the window


# -------------------------------------------------------------------------------------
# Bulk typed arrays (i32 / i64)
# -------------------------------------------------------------------------------------


def test_bulk_i32_array_round_trip_across_chunks():
    values = list(range(-500, 500))  # 1000 elements crosses the 256-element chunk
    h = Heap()
    h.pwrite_i32_array(0, values)
    assert h.byte_size() == 4000
    assert h.pread_i32_array(0, 1000) == values
    assert h.pread_i32(0) == -500  # little-endian, element-addressable
    with pytest.raises(ValueError, match="unexpected end of data"):
        h.pread_i32_array(0, 1001)  # one element too many


def test_bulk_i64_array_round_trip_across_chunks():
    values = [(1 << 40) + i for i in range(1000)]
    h = Heap()
    h.pwrite_i64_array(8, values)  # offset 8 zero-fills the gap
    assert h.byte_size() == 8 + 8000
    assert h.pread_i64_array(8, 1000) == values
    assert h.pread_i64(0) == 0  # the zero-filled gap
    with pytest.raises(ValueError, match="unexpected end of data"):
        h.pread_i64_array(8, 1001)


def test_bulk_read_hostile_count_fails_fast_without_allocating():
    tiny = Heap(b"tiny")
    # The bounds are checked BEFORE the result list is allocated, so a hostile count
    # raises immediately instead of attempting a multi-GiB allocation.
    with pytest.raises(ValueError, match="unexpected end of data"):
        tiny.pread_i32_array(0, 2_000_000_000)
    with pytest.raises(ValueError, match="unexpected end of data"):
        tiny.pread_i64_array(0, 2_000_000_000)
    with pytest.raises(ValueError, match="unexpected end of data"):
        tiny.pread_i32_array(99, 1)  # offset past the end: 0 bytes available


# -------------------------------------------------------------------------------------
# Repeated-value fills
# -------------------------------------------------------------------------------------


def test_pwrite_byte_repeat():
    h = Heap()
    h.pwrite_byte_repeat(2, 0xAB, 5)
    assert h.to_bytes() == b"\x00\x00" + b"\xab" * 5
    h.pwrite_byte_repeat(0, 0x00, 0)  # zero count is a no-op
    assert h.byte_size() == 7


def test_pwrite_i32_and_i64_repeat_cross_chunks():
    h = Heap()
    h.pwrite_i32_repeat(0, -1, 1000)  # crosses the 256-element stack chunk
    assert h.byte_size() == 4000
    assert h.pread_i32_array(0, 1000) == [-1] * 1000

    wide = Heap()
    wide.pwrite_i64_repeat(0, 1 << 40, 1000)
    assert wide.byte_size() == 8000
    assert wide.pread_i64_array(0, 1000) == [1 << 40] * 1000


# -------------------------------------------------------------------------------------
# Heap byte codec + pickle
# -------------------------------------------------------------------------------------


def test_heap_serialize_deserialize_round_trip():
    h = Heap(b"payload")
    data = h.serialize_bytes()
    assert data == b"payload"  # the value form is the stored bytes
    back = Heap.deserialize_bytes(data)
    assert isinstance(back, Heap)
    assert back == h
    assert Heap.deserialize_bytes(b"") == Heap()


def test_heap_pickle_round_trip_is_content_only():
    h = Heap(b"payload").with_mode(IOMode.Read)
    h.set_position(3)
    back = pickle.loads(pickle.dumps(h))
    assert back == h  # equality is over the stored bytes only
    assert back.to_bytes() == b"payload"
    assert back.position == 0  # the cursor is transient
    assert back.mode == IOMode.ReadWrite  # metadata is not serialized
    assert str(back.uri) == "mem://heap"  # every heap reports the synthetic address


# -------------------------------------------------------------------------------------
# Capacity family: checked reserves, ensure, shrink, spare
# -------------------------------------------------------------------------------------


def test_capacity_family_checked_and_scaling():
    h = Heap.with_capacity(64)
    assert h.spare_capacity() >= 64
    h.pwrite_byte_array(0, b"\x00" * 16)
    assert h.spare_capacity() == h.capacity() - 16

    h.reserve_exact(100)
    assert h.capacity() >= 116

    # Checked reserves: a hostile size raises the guided error, never aborts the process.
    h.try_reserve(1024)
    h.try_reserve_exact(2048)
    with pytest.raises(ValueError, match="reserve less"):
        h.try_reserve(2**63)
    with pytest.raises(ValueError, match="reserve less"):
        h.try_ensure_capacity(2**63)
    # Still fully usable afterwards.
    h.pwrite_utf8(0, "alive")
    assert h.pread_utf8(0, 5) == "alive"

    # ensure_capacity targets a total and never shrinks.
    h.ensure_capacity(8192)
    assert h.capacity() >= 8192
    cap = h.capacity()
    h.ensure_capacity(16)
    assert h.capacity() == cap

    # shrink releases spare room (contents untouched).
    h.shrink_to(64)
    h.shrink_to_fit()
    assert h.capacity() <= cap
    assert h.pread_utf8(0, 5) == "alive"


# -------------------------------------------------------------------------------------
# Predicates: is_file / is_dir / exists
# -------------------------------------------------------------------------------------


def test_heap_predicates_a_live_heap_always_exists():
    h = Heap(b"x")
    assert h.exists() is True  # a live in-memory buffer always exists...
    assert h.is_file() is False  # ...although it is neither file...
    assert h.is_dir() is False  # ...nor directory
    assert Heap().exists() is True  # even when empty


def test_cursor_and_slice_predicates_derive_from_kind():
    h = Heap(b"hello world")
    cur = Cursor.over(h)
    assert cur.is_file() is False
    assert cur.is_dir() is False
    assert cur.exists() is True  # the wrapper forwards the live heap's own notion
    win = Slice(h, 0, 5)
    assert win.is_file() is False
    assert win.is_dir() is False
    assert win.exists() is True


# -------------------------------------------------------------------------------------
# Leaf graph surface: name / parent / ls / children / rm family
# -------------------------------------------------------------------------------------


def test_heap_is_a_leaf_of_the_io_graph():
    h = Heap()
    assert h.name == ""  # mem://heap has no path segment to name
    assert h.parent() is None
    assert h.children() == []  # the collected convenience is the empty list


def test_heap_ls_streams_an_empty_iterator_not_a_list():
    h = Heap(b"x")
    entries = h.ls()
    assert isinstance(entries, NoChildren)
    assert not isinstance(entries, list)  # a stream, never a pre-collected tree
    assert iter(entries) is entries  # the Python iterator protocol
    assert list(entries) == []  # ...that yields nothing: a heap is a leaf
    with pytest.raises(StopIteration):
        next(entries)  # exhausted forever
    assert list(h.ls(recursive=True)) == []  # the subtree of a leaf is empty too
    assert repr(h.ls()) == "NoChildren(<empty>)"


def test_heap_rm_family_is_a_guided_refusal():
    h = Heap(b"x")
    with pytest.raises(ValueError, match="removable backing"):
        h.rm()  # nothing on disk backs a heap...
    with pytest.raises(ValueError, match="LocalIO"):
        h.rm()  # ...and the fix names a filesystem node instead
    with pytest.raises(ValueError, match="rmfile needs a removable backing"):
        h.rmfile()
    with pytest.raises(ValueError, match="rmdir needs a removable backing"):
        h.rmdir()


def test_cursor_and_slice_are_leaves_too():
    h = Heap(b"hello world")

    cur = Cursor.over(h)
    assert cur.name == ""
    assert cur.parent() is None
    entries = cur.ls()
    assert iter(entries) is entries and list(entries) == []
    assert cur.children() == []
    with pytest.raises(ValueError, match="removable backing"):
        cur.rm()

    win = Slice(h, 0, 5)
    assert win.name == ""
    assert win.parent() is None
    walk = win.ls(recursive=True)
    assert iter(walk) is walk and list(walk) == []
    assert win.children() == []
    with pytest.raises(ValueError, match="removable backing"):
        win.rmdir()


# -------------------------------------------------------------------------------------
# Heap graph addressing: join / the "/" operator / parent (compose over the URI)
# -------------------------------------------------------------------------------------


def test_heap_join_composes_addresses_over_an_independent_buffer():
    # The uniform graph join/parent work over an in-memory heap as pure address algebra:
    # the child is an independent buffer, but its address composes through the URI.
    root = Heap()
    assert str(root.uri) == "mem://heap"
    assert root.parent() is None  # the mem://heap root has no parent

    child = root.join("logs/app.bin")
    assert isinstance(child, Heap)
    assert str(child.uri) == "mem://heap/logs/app.bin"
    assert child.name == "app.bin"

    # The child is a real, independent buffer — writing/reading it never touches the root.
    assert child.pwrite_utf8(0, "entry") == 5
    assert child.pread_utf8(0, 5) == "entry"
    assert child.byte_size() == 5
    assert root.byte_size() == 0  # addresses compose; bytes do not

    # parent() navigates back up the URI — the exact inverse of join.
    assert str(child.parent().uri) == "mem://heap/logs"
    assert str(child.parent().parent().uri) == "mem://heap"
    assert child.parent().parent().parent() is None


def test_heap_truediv_operator_matches_join():
    root = Heap(b"seed")  # the "/" operator is the operator spelling of join
    assert str((root / "logs/app.bin").uri) == str(root.join("logs/app.bin").uri)
    assert (root / "a/b/c").name == "c"
    # The addressed child is still an independent, empty buffer (the seed stays put).
    assert (root / "app.bin").byte_size() == 0
    assert root.to_bytes() == b"seed"


def test_heap_join_percent_encodes_a_spaced_segment():
    # A spaced segment is percent-encoded in the composed address...
    assert str(Heap().join("my dir/f").uri) == "mem://heap/my%20dir/f"
    # ...while name percent-decodes the retained leaf segment.
    assert Heap().join("my dir/f").name == "f"
    assert str(Heap().join("my dir").parent().uri) == "mem://heap"


def test_heap_parents_lists_ancestor_addresses_nearest_first():
    node = Heap().join("a/b/c")
    parents = node.parents()
    assert isinstance(parents, list)  # a bounded ancestor walk collected as a list
    assert all(isinstance(p, Heap) for p in parents)
    # Nearest first, up to the mem://heap root — the repeated parent() chain.
    assert [str(p.uri) for p in parents] == ["mem://heap/a/b", "mem://heap/a", "mem://heap"]
    # A bare root has no ancestors.
    assert Heap().parents() == []


# -------------------------------------------------------------------------------------
# Media type: declared headers, else the address, else the octet-stream fallback
# -------------------------------------------------------------------------------------


def test_heap_mime_type_octet_stream_fallback():
    # No headers and an anonymous mem://heap address (no extension) -> octet-stream.
    heap = Heap()
    mime = heap.mime_type()
    assert isinstance(mime, MimeType)
    assert mime.is_octet_stream()
    media = heap.media_type()
    assert isinstance(media, MediaType)
    assert media.essences() == ["application/octet-stream"]


def test_heap_mime_type_from_headers_wins():
    heap = Heap()
    heap.set_headers(Headers().with_("Content-Type", "application/json"))
    assert heap.mime_type().essence == "application/json"
    assert heap.media_type().essences() == ["application/json"]


def test_heap_media_type_headers_with_encoding():
    heap = Heap()
    headers = Headers()
    headers.set_content_type("application/x-tar")
    headers.set_content_encoding("gzip")
    heap.set_headers(headers)
    assert heap.mime_type().essence == "application/x-tar"
    assert heap.media_type().essences() == ["application/x-tar", "application/gzip"]


def test_heap_mime_type_inferred_from_address():
    # An addressed heap (mem://heap/report.pdf) infers from the URI's file name.
    node = Heap().join("report.pdf")
    assert node.mime_type().essence == "application/pdf"


def test_ensure_content_type_memoizes():
    heap = Heap()
    assert heap.headers.content_type() is None
    resolved = heap.ensure_content_type()
    assert resolved.is_octet_stream()  # inferred, and now stored
    assert heap.headers.content_type() == "application/octet-stream"

    # When Content-Type is already set, ensure_content_type returns it untouched.
    declared = Heap()
    declared.set_headers(Headers().with_("Content-Type", "application/json"))
    assert declared.ensure_content_type().essence == "application/json"
    assert declared.headers.content_type() == "application/json"


def test_cursor_and_slice_delegate_media_type():
    heap = Heap(b"hello world")
    heap.set_headers(Headers().with_("Content-Type", "text/plain"))
    cur = heap.cursor()
    assert cur.mime_type().essence == "text/plain"
    assert cur.media_type().essences() == ["text/plain"]
    assert cur.ensure_content_type().essence == "text/plain"

    sl = heap.window(0, 5)
    assert sl.mime_type().essence == "text/plain"
    assert sl.media_type().essences() == ["text/plain"]
    # A bare window over an anonymous heap falls back to octet-stream.
    assert Heap(b"hello").window(0, 5).mime_type().is_octet_stream()


# -------------------------------------------------------------------------------------
# Magic inference: infer_mime_type / infer_media_type (positioned reads, cursor untouched)
# -------------------------------------------------------------------------------------

PNG_HEAD = b"\x89PNG\r\n\x1a\n" + b"\x00" * 64


def test_infer_mime_type_from_magic_does_not_move_cursor():
    h = Heap(PNG_HEAD)
    h.set_position(3)  # a cursor mid-stream
    inferred = h.infer_mime_type()
    assert isinstance(inferred, MimeType)
    assert inferred.essence == "image/png"  # magic wins over the octet-stream fallback
    assert h.position == 3  # a positioned head read never moved the cursor

    # No magic and no address -> the declared/octet-stream fallback.
    assert Heap(b"just some plain text").infer_mime_type().is_octet_stream()


def test_cursor_infer_mime_type_does_not_move_cursor():
    cur = Cursor(PNG_HEAD)
    assert cur.read(4) == b"\x89PNG"  # advance the cursor first
    assert cur.position == 4
    assert cur.infer_mime_type().essence == "image/png"
    assert cur.position == 4  # infer read the head positioned, leaving the cursor put


def test_infer_media_type_peels_compression_layers():
    # A gzip stream whose payload has PNG magic: recursive inference peels the gzip layer.
    inner = PNG_HEAD
    packed = Gzip().compress(inner)
    media = Heap(packed).infer_media_type()
    assert isinstance(media, MediaType)
    assert media.essences()[0] == "application/gzip"  # outer magic (0x1f 0x8b)
    assert "image/png" in media.essences()  # the peeled inner type


# -------------------------------------------------------------------------------------
# Compression: compression() / decompress() (inferred) + compress_with / decompress_with
# -------------------------------------------------------------------------------------


def test_decompress_heap_addressed_by_content_type():
    payload = b"decompress me from a zstd heap " * 100
    packed = Zstd().compress(payload)
    h = Heap(packed)
    h.set_headers(Headers().with_("Content-Type", "application/zstd"))
    # compression() resolves the codec from the declared media type...
    codec = h.compression()
    assert codec is not None
    assert codec.name == "zstd"
    # ...and decompress() uses it, returning the original bytes.
    assert h.decompress() == payload


def test_decompress_gzip_heap_by_content_type():
    payload = b"gzip payload " * 200
    h = Heap(Gzip().compress(payload))
    h.set_headers(Headers().with_("Content-Type", "application/gzip"))
    assert h.compression().name == "gzip"
    assert h.decompress() == payload


def test_compression_is_none_and_decompress_raises_for_non_compression():
    h = Heap(b"plain bytes, not a compression")  # octet-stream -> not a codec
    assert h.compression() is None
    with pytest.raises(ValueError, match="not a supported compression"):
        h.decompress()


def test_compress_with_and_decompress_with_explicit_codec():
    payload = b"explicit codec round trip " * 100
    src = Heap(payload)
    packed = src.compress_with(Gzip())
    assert isinstance(packed, bytes)
    assert len(packed) < len(payload)
    # A heap holding the packed bytes, decompressed with a matching codec, restores the input.
    assert Heap(packed).decompress_with(Gzip()) == payload


def test_compress_with_rejects_a_non_codec():
    with pytest.raises(TypeError):
        Heap(b"data").compress_with("not a codec")


def test_cursor_and_slice_compression_delegate():
    payload = b"delegated compression " * 100
    packed = Zstd().compress(payload)
    # A cursor/window over a zstd-addressed heap decompresses through the wrapped source.
    heap = Heap(packed)
    heap.set_headers(Headers().with_("Content-Type", "application/zstd"))
    assert heap.cursor().decompress() == payload
    assert heap.window(0, len(packed)).decompress() == payload
    # The explicit-codec path works over the views too.
    assert Cursor(payload).compress_with(Zstd())  # non-empty compressed bytes
    assert Heap(packed).cursor().decompress_with(Zstd()) == payload


# -------------------------------------------------------------------------------------
# truncate + content_length
# -------------------------------------------------------------------------------------


def test_heap_truncate_shrinks_and_extends():
    h = Heap(b"hello world")
    h.truncate(5)
    assert h.to_bytes() == b"hello"  # tail dropped
    h.truncate(8)
    assert h.to_bytes() == b"hello\x00\x00\x00"  # extended, zero-filled
    assert h.byte_size() == 8


def test_heap_content_length_falls_back_to_byte_size_then_prefers_header():
    h = Heap(b"abcde")
    assert h.content_length() == 5  # no header — falls back to byte_size
    h.set_headers(Headers().with_("Content-Length", "999"))
    assert h.content_length() == 999  # now served from the cached header


def test_cursor_and_slice_truncate_is_the_guided_view_refusal():
    h = Heap(b"hello world")
    cur = Cursor.over(h)
    assert cur.content_length() == 11
    with pytest.raises(ValueError, match="cannot be resized"):
        cur.truncate(3)  # a view has no resizable backing of its own
    win = Slice(h, 0, 5)
    assert win.content_length() == 5
    with pytest.raises(ValueError, match="cannot be resized"):
        win.truncate(2)


# -------------------------------------------------------------------------------------
# Bulk unsigned + floating arrays (u16 / u32 / u64 / f32 / f64) + repeats
# -------------------------------------------------------------------------------------


def test_bulk_unsigned_arrays_round_trip():
    h = Heap()
    h.pwrite_u16_array(0, [0, 1, 2, 65535])
    assert h.pread_u16_array(0, 4) == [0, 1, 2, 65535]

    h.pwrite_u32_array(8, [0, 2**31, 2**32 - 1])
    assert h.pread_u32_array(8, 3) == [0, 2**31, 2**32 - 1]

    h.pwrite_u64_array(20, [0, 2**63, 2**64 - 1])
    assert h.pread_u64_array(20, 3) == [0, 2**63, 2**64 - 1]


def test_bulk_float_arrays_round_trip():
    h = Heap()
    h.pwrite_f32_array(0, [1.5, -2.25, 3.0])  # exactly representable in f32
    assert h.pread_f32_array(0, 3) == [1.5, -2.25, 3.0]

    wide = Heap()
    wide.pwrite_f64_array(0, [1.5, -2.5, 1e300])
    assert wide.pread_f64_array(0, 3) == [1.5, -2.5, 1e300]


def test_bulk_unsigned_and_float_repeats_cross_chunks():
    h = Heap()
    h.pwrite_u16_repeat(0, 7, 1000)  # crosses the 256-element stack chunk
    assert h.pread_u16_array(0, 1000) == [7] * 1000
    h2 = Heap()
    h2.pwrite_u32_repeat(0, 2**32 - 1, 500)
    assert h2.pread_u32_array(0, 500) == [2**32 - 1] * 500
    h3 = Heap()
    h3.pwrite_u64_repeat(0, 2**64 - 1, 300)
    assert h3.pread_u64_array(0, 300) == [2**64 - 1] * 300
    h4 = Heap()
    h4.pwrite_f32_repeat(0, 1.25, 300)
    assert h4.pread_f32_array(0, 300) == [1.25] * 300
    h5 = Heap()
    h5.pwrite_f64_repeat(0, 2.5, 300)
    assert h5.pread_f64_array(0, 300) == [2.5] * 300


def test_bulk_unsigned_read_hostile_count_fails_fast():
    tiny = Heap(b"tiny")
    for reader in ("pread_u16_array", "pread_u32_array", "pread_u64_array",
                   "pread_f32_array", "pread_f64_array"):
        with pytest.raises(ValueError, match="unexpected end of data"):
            getattr(tiny, reader)(0, 2_000_000_000)


# -------------------------------------------------------------------------------------
# Cross-source copy: copy_from / pwrite_from
# -------------------------------------------------------------------------------------


def test_heap_copy_from_overwrites_and_truncates():
    dst = Heap(b"old and longer data")
    src = Heap(b"new")
    assert dst.copy_from(src) == 3
    assert dst.to_bytes() == b"new"  # overwritten and truncated to match


def test_heap_pwrite_from_positioned_slice():
    dst = Heap(b"..........")  # 10 dots
    src = Heap(b"ABCDEF")
    assert dst.pwrite_from(2, src, 1, 3) == 3  # src[1:4] = "BCD" at offset 2
    assert dst.to_bytes() == b"..BCD....."
    # A length past the end of src is short (transfers only what remains).
    assert dst.pwrite_from(0, src, 4, 100) == 2  # src[4:] = "EF"
    assert dst.to_bytes() == b"EFBCD....."


# -------------------------------------------------------------------------------------
# readline / readlines
# -------------------------------------------------------------------------------------


def test_heap_readline_and_readlines():
    h = Heap(b"first\nsecond")
    assert h.readline() == "first"  # the trailing \n is stripped
    assert h.readline() == "second"  # last line, no terminator
    assert h.readline() == ""  # now at EOF (returns "" without advancing)
    h2 = Heap(b"a\n\nb\n")
    assert h2.readlines() == ["a", "", "b"]  # blank line kept, terminators stripped


def test_heap_readline_strips_crlf_and_honors_quoted_newlines():
    # A CRLF terminator is stripped whole (not just the \n).
    crlf = Heap(b"first\r\nsecond")
    assert crlf.readline() == "first"
    assert crlf.readline() == "second"
    # CSV-aware: a \n inside a double-quoted field does not end the record.
    csv = Heap(b'a,"x\ny",b\nnext')
    assert csv.readline() == 'a,"x\ny",b'
    assert csv.readline() == "next"


def test_cursor_readline_and_readlines():
    cur = Cursor(b"one\ntwo\n")
    assert cur.readline() == "one"  # the trailing \n is stripped
    assert cur.readlines() == ["two"]  # continues from the cursor


# -------------------------------------------------------------------------------------
# In-place (de)compression
# -------------------------------------------------------------------------------------


def test_heap_compress_in_place_explicit_codec_round_trip():
    payload = b"compress me in place " * 50
    h = Heap(payload)
    h.compress_in_place(Gzip())  # explicit codec
    assert h.to_bytes() != payload
    assert len(h) < len(payload)
    assert h.headers.content_type() == "application/gzip"  # Content-Type synced to the codec
    h.decompress_in_place()  # codec inferred from the media type
    assert h.to_bytes() == payload


def test_heap_compress_in_place_default_codec_from_media_type():
    payload = b"gzip via the declared media type " * 30
    h = Heap(payload)
    h.set_headers(Headers().with_("Content-Type", "application/gzip"))
    h.compress_in_place()  # None -> the codec of the heap's own media type (gzip)
    assert h.to_bytes() != payload
    h.decompress_in_place()
    assert h.to_bytes() == payload


def test_heap_compress_in_place_without_a_codec_is_guided():
    with pytest.raises(ValueError, match="no codec"):
        Heap(b"plain octet-stream bytes").compress_in_place()  # nothing to infer


# -------------------------------------------------------------------------------------
# rm family: the new exist_ok argument
# -------------------------------------------------------------------------------------


def test_heap_rm_family_accepts_exist_ok_and_still_refuses():
    h = Heap(b"x")
    # A heap has no removable backing, so it refuses regardless of exist_ok, but the
    # argument is accepted (mirrors the changed core signature).
    with pytest.raises(ValueError, match="removable backing"):
        h.rm(exist_ok=True)
    with pytest.raises(ValueError, match="removable backing"):
        h.rm(exist_ok=False)
    with pytest.raises(ValueError, match="removable backing"):
        h.rmfile(exist_ok=False)
    with pytest.raises(ValueError, match="removable backing"):
        h.rmdir(exist_ok=True)


# -------------------------------------------------------------------------------------
# Context managers (Heap / Cursor / Slice)
# -------------------------------------------------------------------------------------


def test_heap_context_manager_returns_self_and_propagates():
    with Heap(b"data") as h:
        assert isinstance(h, Heap)
        assert h.to_bytes() == b"data"
    with pytest.raises(RuntimeError, match="boom"):  # __exit__ propagates exceptions
        with Heap(b"x"):
            raise RuntimeError("boom")


def test_cursor_and_slice_context_managers():
    with Cursor(b"abc") as c:
        assert isinstance(c, Cursor)
        assert c.read(3) == b"abc"
    h = Heap(b"hello world")
    with Slice(h, 0, 5) as w:
        assert isinstance(w, Slice)
        assert w.to_bytes() == b"hello"


# -------------------------------------------------------------------------------------
# from_io: type-inferring constructor over a Python file-like object
# -------------------------------------------------------------------------------------


def test_heap_from_io_bytesio_transfers_position():
    buf = io.BytesIO(b"hello world")
    assert buf.read(6) == b"hello "  # partially consume it
    assert buf.tell() == 6
    h = Heap.from_io(buf)
    assert h.to_bytes() == b"hello world"  # the full contents are copied in
    assert h.position == 6  # ...and the consumed position transfers
    assert h.read_to_end() == b"world"


def test_heap_from_io_stringio_encodes_utf8():
    h = Heap.from_io(io.StringIO("héllo"))
    assert h.to_bytes() == "héllo".encode("utf-8")


def test_heap_from_io_read_only_object():
    class Reader:
        def __init__(self, data):
            self._data = data

        def read(self):
            return self._data

    h = Heap.from_io(Reader(b"raw reader bytes"))  # no getvalue()/tell() -> read(), pos 0
    assert h.to_bytes() == b"raw reader bytes"
    assert h.position == 0


def test_heap_from_io_rejects_a_non_file_like():
    with pytest.raises(TypeError):
        Heap.from_io(123)


def test_cursor_from_io_positions_at_tell():
    buf = io.BytesIO(b"abcdef")
    buf.read(2)
    cur = Cursor.from_io(buf)
    assert isinstance(cur, Cursor)
    assert cur.position == 2  # the cursor starts where the BytesIO was
    assert cur.read(4) == b"cdef"


# -------------------------------------------------------------------------------------
# Line iteration (Heap / Cursor behave like a file object)
# -------------------------------------------------------------------------------------


def test_heap_line_iteration():
    h = Heap(b"a\nb\nc")
    assert iter(h) is h  # the iterator protocol
    assert list(h) == ["a", "b", "c"]  # lines from the cursor, terminators stripped
    assert [line for line in Heap(b"x\ny\n")] == ["x", "y"]
    # A blank line is yielded (as ""); only EOF (a readline that consumes nothing) stops.
    assert list(Heap(b"a\n\nb")) == ["a", "", "b"]


def test_cursor_line_iteration():
    cur = Cursor(b"one\ntwo\n")
    assert list(cur) == ["one", "two"]


# -------------------------------------------------------------------------------------
# __getitem__ (int AND slice) on the buffer types
# -------------------------------------------------------------------------------------


def test_heap_getitem_int_and_slice():
    h = Heap(b"hello world")
    assert h[0] == ord("h")  # int index -> the byte as an int
    assert h[-1] == ord("d")  # negative index wraps
    assert h[0:5] == b"hello"  # slice -> bytes
    assert h[6:] == b"world"
    assert h[::-1] == b"dlrow olleh"  # step (delegated to bytes' own __getitem__)
    with pytest.raises(IndexError):
        h[100]


def test_cursor_and_slice_getitem():
    cur = Cursor(b"abcdef")
    assert cur[0] == ord("a")
    assert cur[1:4] == b"bcd"
    assert cur.position == 0  # indexing never moves the cursor

    win = Slice(Heap(b"hello world"), 6, 5)  # the "world" window
    assert win[0] == ord("w")  # indices are within the window
    assert win[-1] == ord("d")
    assert win[:] == b"world"
    with pytest.raises(IndexError):
        win[5]  # past the window's end


# -------------------------------------------------------------------------------------
# All native numeric widths — scalars, bulk arrays + repeats, cursor read/write
# -------------------------------------------------------------------------------------

# (rust_type, min, max) sample edge values for each scalar width.
_SCALAR_CASES = [
    ("i8", -128, 127),
    ("u8", 0, 255),
    ("i16", -(2**15), 2**15 - 1),
    ("u16", 0, 2**16 - 1),
    ("u32", 0, 2**32 - 1),
    ("u64", 0, 2**64 - 1),
    ("i128", -(2**127), 2**127 - 1),
    ("u128", 0, 2**128 - 1),
]


def test_heap_scalar_pread_pwrite_all_widths():
    h = Heap()
    for t, lo, hi in _SCALAR_CASES:
        pw = getattr(h, f"pwrite_{t}")
        pr = getattr(h, f"pread_{t}")
        pw(0, lo)
        assert pr(0) == lo, t
        pw(0, hi)
        assert pr(0) == hi, t
    # i128 / u128 map to Python int with full precision.
    h.pwrite_i128(0, -(2**100))
    assert h.pread_i128(0) == -(2**100)
    h.pwrite_u128(0, 2**120 + 7)
    assert h.pread_u128(0) == 2**120 + 7


def test_heap_scalar_float_widths():
    h = Heap()
    h.pwrite_f32(0, 1.5)
    assert h.pread_f32(0) == pytest.approx(1.5)
    h.pwrite_f64(8, 2.25)
    assert h.pread_f64(8) == 2.25


def test_heap_scalar_pread_past_end_raises():
    with pytest.raises(ValueError):
        Heap().pread_i128(0)  # empty -> EOF


def test_heap_bulk_arrays_and_repeats():
    # element width in the fail-fast bounds check: 1 (i8), 2 (i16), 16 (i128/u128)
    h = Heap()
    h.pwrite_i8_array(0, [-1, 2, -3])
    assert h.pread_i8_array(0, 3) == [-1, 2, -3]
    h.pwrite_i16_array(0, [1000, -2000, 3000])
    assert h.pread_i16_array(0, 3) == [1000, -2000, 3000]
    h.pwrite_i128_array(0, [2**100, -(2**99)])
    assert h.pread_i128_array(0, 2) == [2**100, -(2**99)]
    h.pwrite_u128_array(0, [2**127, 1])
    assert h.pread_u128_array(0, 2) == [2**127, 1]
    # repeats never materialize the full array
    h2 = Heap()
    h2.pwrite_i8_repeat(0, 7, 5)
    assert h2.pread_i8_array(0, 5) == [7, 7, 7, 7, 7]
    h2.pwrite_i16_repeat(0, -9, 3)
    assert h2.pread_i16_array(0, 3) == [-9, -9, -9]
    h2.pwrite_i128_repeat(0, 2**100, 2)
    assert h2.pread_i128_array(0, 2) == [2**100, 2**100]
    h2.pwrite_u128_repeat(0, 2**120, 2)
    assert h2.pread_u128_array(0, 2) == [2**120, 2**120]


def test_heap_bulk_array_fail_fast_bounds():
    h = Heap(b"\x01\x02\x03")  # 3 bytes
    with pytest.raises(ValueError):
        h.pread_i16_array(0, 100)  # 200 bytes wanted, fails before allocating
    with pytest.raises(ValueError):
        h.pread_i128_array(0, 1)  # 16 bytes wanted, only 3 available


def test_heap_cursor_typed_read_write_all_widths():
    c = Heap()
    c.write_i8(-9)
    c.write_u8(200)
    c.write_i16(-300)
    c.write_u16(60000)
    c.write_u32(4_000_000_000)
    c.write_u64(10**19)
    c.write_i128(-(2**100))
    c.write_u128(2**120)
    c.write_f32(3.5)
    c.write_f64(6.5)
    c.rewind()
    assert c.read_i8() == -9
    assert c.read_u8() == 200
    assert c.read_i16() == -300
    assert c.read_u16() == 60000
    assert c.read_u32() == 4_000_000_000
    assert c.read_u64() == 10**19
    assert c.read_i128() == -(2**100)
    assert c.read_u128() == 2**120
    assert c.read_f32() == pytest.approx(3.5)
    assert c.read_f64() == 6.5


def test_cursor_scalar_and_cursor_typed_all_widths():
    cur = Cursor()
    # cursor stream read/write
    cur.write_i8(-1)
    cur.write_u128(2**100)
    cur.rewind()
    assert cur.read_i8() == -1
    assert cur.read_u128() == 2**100
    # positioned scalar accessors on the same cursor
    assert cur.pread_i8(0) == -1
    cur.pwrite_u16(0, 40000)
    assert cur.pread_u16(0) == 40000
    cur.pwrite_f64(0, 1.25)
    assert cur.pread_f64(0) == 1.25


# -------------------------------------------------------------------------------------
# move_into (Heap) + the module-level yggdryl.open
# -------------------------------------------------------------------------------------


def test_heap_move_into_empties_source():
    src = Heap(b"relocate me")
    dst = Heap()
    n = src.move_into(dst)
    assert n == 11
    assert bytes(dst) == b"relocate me"
    assert src.byte_size() == 0  # the source is emptied


def test_open_bytes_and_bytearray_wrap_a_heap():
    import yggdryl

    h = yggdryl.open(b"hello bytes")
    assert isinstance(h, Heap)
    assert bytes(h) == b"hello bytes"
    ba = yggdryl.open(bytearray(b"mutable"))
    assert isinstance(ba, Heap)
    assert bytes(ba) == b"mutable"


def test_open_mem_uri_and_str_wrap_a_heap():
    import yggdryl

    assert isinstance(yggdryl.open("mem://heap/data"), Heap)
    assert isinstance(yggdryl.open(Uri.parse("mem://heap/data")), Heap)
    assert yggdryl.open("mem://heap/data").uri.scheme == "mem"


def test_open_rejects_unknown_scheme_and_type():
    import yggdryl

    with pytest.raises(ValueError):
        yggdryl.open("http://example.com/x")  # unsupported scheme
    with pytest.raises(TypeError):
        yggdryl.open(1234)  # not an address / bytes / PathLike


# -------------------------------------------------------------------------------------
# Element type (dtype) + transforms: dtype / element_count / resize_dtype / mask_filter
# -------------------------------------------------------------------------------------


def test_heap_dtype_get_set_and_element_count():
    h = Heap()
    assert h.dtype() == DataTypeId.Unknown  # no declared element type by default
    assert h.element_count() == 0  # Unknown has no element count
    h.pwrite_i64_array(0, [1, 2, 3])
    h.set_dtype(DataTypeId.I64)
    assert h.dtype() == DataTypeId.I64
    assert h.element_count() == 3  # 24 bytes / 8
    # The declared type is stored in the headers (Elem-Type-Id).
    assert h.headers.elem_type_id() == DataTypeId.I64
    # Unknown clears it.
    h.set_dtype(DataTypeId.Unknown)
    assert h.dtype() == DataTypeId.Unknown


def test_heap_resize_dtype_copy_leaves_source_untouched():
    src = Heap()
    src.pwrite_i64_array(0, [1, -2, 3])
    src.set_dtype(DataTypeId.I64)
    narrowed = src.resize_dtype(DataTypeId.I32)  # 24 -> 12 byte copy
    assert isinstance(narrowed, Heap)
    assert narrowed.byte_size() == 12
    assert narrowed.dtype() == DataTypeId.I32
    assert narrowed.pread_i32_array(0, 3) == [1, -2, 3]
    assert src.byte_size() == 24  # the source is untouched
    assert src.dtype() == DataTypeId.I64


def test_heap_resize_dtype_in_place_widens_and_returns_count():
    h = Heap()
    h.pwrite_i32_array(0, [1, 2, 3, 4])
    h.set_dtype(DataTypeId.I32)
    n = h.resize_dtype_in_place(DataTypeId.I64)  # widen 16 -> 32 bytes
    assert n == 4
    assert h.dtype() == DataTypeId.I64
    assert h.pread_i64_array(0, 4) == [1, 2, 3, 4]


def test_heap_resize_dtype_without_a_type_is_guided():
    h = Heap(b"\x00\x00\x00\x00")  # no declared element type
    with pytest.raises(ValueError, match="element type"):
        h.resize_dtype(DataTypeId.I32)


def test_heap_mask_filter_copy_selects_set_bits():
    src = Heap()
    src.pwrite_i32_array(0, [10, 20, 30, 40])
    src.set_dtype(DataTypeId.I32)
    mask = Heap(bytes([0b1010]))  # bits 1 and 3 set (LSB-first) -> keep elements 1, 3
    kept = src.mask_filter(mask)
    assert isinstance(kept, Heap)
    assert kept.pread_i32_array(0, 2) == [20, 40]
    assert src.pread_i32_array(0, 4) == [10, 20, 30, 40]  # the source is untouched


def test_heap_mask_filter_in_place_compacts_and_returns_count():
    h = Heap()
    h.pwrite_i32_array(0, [10, 20, 30, 40])
    h.set_dtype(DataTypeId.I32)
    mask = Heap(bytes([0b0101]))  # bits 0 and 2 set -> keep elements 0, 2
    kept = h.mask_filter_in_place(mask)
    assert kept == 2
    assert h.pread_i32_array(0, 2) == [10, 30]
    assert h.byte_size() == 8  # truncated to the kept length


def test_heap_mask_filter_without_a_type_is_guided():
    h = Heap(b"\x01\x02\x03\x04")  # no declared element type to select over
    with pytest.raises(ValueError, match="element type"):
        h.mask_filter(Heap(bytes([0b1])))


# -------------------------------------------------------------------------------------
# Vectorized aggregations (sum / min / max / mean / std / first / last / count_ge)
# -------------------------------------------------------------------------------------


def test_heap_integer_aggregations():
    h = Heap()
    h.pwrite_i64_array(0, [4, 8, 15, 16, 23, 42])
    assert h.sum_i64(0, 6) == 108
    assert h.min_i64(0, 6) == 4
    assert h.max_i64(0, 6) == 42
    assert h.mean_i64(0, 6) == 18.0
    assert h.first_i64(0, 6) == 4
    assert h.last_i64(0, 6) == 42
    assert h.count_ge_i64(0, 6, 16) == 3  # 16, 23, 42
    assert h.std_i64(0, 6) == pytest.approx(12.3153, rel=1e-4)


def test_heap_aggregations_all_representative_types():
    for t, values in [
        ("i32", [-5, 0, 5, 10]),
        ("i64", [1 << 40, (1 << 40) + 1]),
        ("u32", [1, 2, 3, 4]),
        ("u64", [10, 20, 30]),
        ("f32", [1.0, 2.0, 3.0, 4.0]),
        ("f64", [1.5, 2.5, 3.5]),
    ]:
        h = Heap()
        getattr(h, f"pwrite_{t}_array")(0, values)
        n = len(values)
        assert getattr(h, f"sum_{t}")(0, n) == sum(values), t
        assert getattr(h, f"min_{t}")(0, n) == min(values), t
        assert getattr(h, f"max_{t}")(0, n) == max(values), t
        assert getattr(h, f"mean_{t}")(0, n) == pytest.approx(sum(values) / n), t
        assert getattr(h, f"first_{t}")(0, n) == pytest.approx(values[0]), t
        assert getattr(h, f"last_{t}")(0, n) == pytest.approx(values[-1]), t
        assert getattr(h, f"std_{t}")(0, n) >= 0.0, t
        assert getattr(h, f"count_ge_{t}")(0, n, min(values)) == n, t


def test_heap_aggregations_empty_is_none():
    h = Heap()  # no data
    assert h.min_i32(0, 0) is None
    assert h.max_i32(0, 0) is None
    assert h.mean_i32(0, 0) is None
    assert h.std_i32(0, 0) is None
    assert h.first_i32(0, 0) is None
    assert h.last_i32(0, 0) is None
    assert h.sum_i32(0, 0) == 0  # an empty sum is the zero of the accumulator
    assert h.count_ge_i32(0, 0, 0) == 0


def test_heap_float_min_max_ignore_nan():
    h = Heap()
    h.pwrite_f64_array(0, [1.0, float("nan"), 3.0, 2.0])
    assert h.min_f64(0, 4) == 1.0  # NaN is ignored order-independently
    assert h.max_f64(0, 4) == 3.0


# -------------------------------------------------------------------------------------
# Headers: storage element type + resource name conveniences
# -------------------------------------------------------------------------------------


def test_headers_elem_type_id_round_trip():
    hdr = Headers()
    assert hdr.elem_type_id() == DataTypeId.Unknown  # nothing declared
    assert hdr.elem_byte_size() == 0
    assert hdr.elem_bit_size() == 0
    hdr.set_elem_type_id(DataTypeId.I64)
    assert hdr.elem_type_id() == DataTypeId.I64
    assert hdr.elem_byte_size() == 8
    assert hdr.elem_bit_size() == 64
    # Unknown removes the header.
    hdr.set_elem_type_id(DataTypeId.Unknown)
    assert hdr.elem_type_id() == DataTypeId.Unknown
    assert not hdr.contains(Headers.ELEM_TYPE_ID)


def test_headers_name_round_trip():
    hdr = Headers()
    assert hdr.name() is None
    hdr.set_name("column_1")
    assert hdr.name() == "column_1"
    assert hdr.get(Headers.NAME) == "column_1"
