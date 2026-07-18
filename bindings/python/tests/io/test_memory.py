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
import pickle

import pytest

import yggdryl.memory
from yggdryl.compression import Gzip, Zstd
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
