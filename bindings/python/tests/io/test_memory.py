"""Tests for the ``yggdryl.memory`` ``Heap`` / ``Mmap`` sources and ``Whence`` seek anchor.

Mirrors ``crates/yggdryl-core/tests/memory_heap.rs`` on the Python surface: construction,
size/capacity, the positioned ``pread_*`` / ``pwrite_*`` primitives and typed accessors
(including UTF-8 text, the bulk ``i32``/``i64`` arrays, and repeated fills), the cursor
stream, seeks from every anchor, bounded slices, the source metadata (``headers`` /
``mode`` / ``kind``), the byte codec + pickle, and the value dunders
(``bytes()`` / ``==`` / ``copy`` / unhashability). The ``Mmap`` section drives the same
surface over a real file: open/create dispatch (str path or ``Uri``), persistence with
exact truncation, read-only mappings, capacity over a file, and the ``close()`` /
context-manager lifecycle.
"""

import copy
import gc
import pickle

import pytest

import yggdryl.memory
from yggdryl.headers import Headers
from yggdryl.io import IOKind, IOMode
from yggdryl.memory import Cursor, Heap, Mmap, Slice, Whence
from yggdryl.uri import Uri


def test_module_surface():
    for cls in (Heap, Whence, Cursor, Slice, Mmap):
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
# Mmap: constructors + generic open dispatch
# -------------------------------------------------------------------------------------


def test_mmap_has_no_plain_constructor():
    # The explicit lifecycle verbs open/open_readonly/create are the only entry points.
    with pytest.raises(TypeError):
        Mmap()


def test_mmap_generic_open_dispatch_str_and_uri(tmp_path):
    p = tmp_path / "dispatch.bin"
    m = Mmap.create(str(p))  # str -> create_path
    assert m.pwrite_utf8(0, "hi") == 2
    m.close()

    by_str = Mmap.open(str(p))  # str -> open_path
    assert by_str.pread_utf8(0, 2) == "hi"
    by_str.close()

    uri = Uri.from_path(str(p))
    by_uri = Mmap.open(uri)  # Uri -> open_uri
    assert by_uri.pread_utf8(0, 2) == "hi"
    by_uri.close()

    ro = Mmap.open_readonly(uri)  # Uri -> open_uri_readonly
    assert ro.mode == IOMode.Read
    assert ro.pread_utf8(0, 2) == "hi"
    ro.close()

    made = Mmap.create(Uri.from_path(str(tmp_path / "made.bin")))  # Uri -> create_uri
    assert made.is_empty()
    made.close()


def test_mmap_open_dispatch_rejects_other_types_guided(tmp_path):
    with pytest.raises(TypeError, match="expected a str filesystem path"):
        Mmap.open(123)
    with pytest.raises(TypeError, match="str\\(path\\)"):
        Mmap.create(tmp_path / "x.bin")  # a pathlib.Path must be passed as str(path)


def test_mmap_open_missing_path_is_guided(tmp_path):
    missing = str(tmp_path / "missing.bin")
    with pytest.raises(ValueError, match="check that the path exists"):
        Mmap.open(missing)
    with pytest.raises(ValueError, match="check that the path exists"):
        Mmap.open_readonly(missing)


# -------------------------------------------------------------------------------------
# Mmap: write/read round-trips (typed + bulk + utf8 + cursor stream)
# -------------------------------------------------------------------------------------


def test_mmap_typed_positioned_round_trip(tmp_path):
    m = Mmap.create(str(tmp_path / "typed.bin"))
    m.pwrite_byte(0, 0xAB)
    m.pwrite_i32(1, -42)
    m.pwrite_i64(5, 1 << 40)
    assert m.pread_byte(0) == 0xAB
    assert m.pread_i32(1) == -42
    assert m.pread_i64(5) == 1 << 40
    assert m.byte_size() == 13
    m.pwrite_bit(104, True)  # bit 0 of byte 13 -- grows the file by one byte
    assert m.pread_bit(104)
    assert not m.pread_bit(105)
    assert m.byte_size() == 14
    with pytest.raises(ValueError, match="unexpected end of data"):
        m.pread_i64(m.byte_size())
    m.close()


def test_mmap_pread_pwrite_byte_array_and_gap_zero_fill(tmp_path):
    m = Mmap.create(str(tmp_path / "bytes.bin"))
    assert m.pwrite_byte_array(0, b"abc") == 3
    assert m.pwrite_byte_array(5, b"Z") == 1  # past the end: the gap is zero-filled
    assert m.pread_byte_array(0, 99) == b"abc\x00\x00Z"  # clamped to what remains
    assert m.pread_byte_array(6, 4) == b""  # at the end
    m.close()


def test_mmap_bulk_arrays_repeats_and_utf8(tmp_path):
    m = Mmap.create(str(tmp_path / "bulk.bin"))
    values = list(range(-500, 500))  # 1000 elements crosses the 256-element chunk
    m.pwrite_i32_array(0, values)
    assert m.pread_i32_array(0, 1000) == values
    wide = [(1 << 40) + i for i in range(300)]
    m.pwrite_i64_array(4000, wide)
    assert m.pread_i64_array(4000, 300) == wide

    m.pwrite_byte_repeat(6400, 0xAB, 5)
    assert m.pread_byte_array(6400, 5) == b"\xab" * 5
    m.pwrite_i32_repeat(6405, -1, 10)
    assert m.pread_i32_array(6405, 10) == [-1] * 10
    m.pwrite_i64_repeat(6445, 7, 4)
    assert m.pread_i64_array(6445, 4) == [7] * 4

    assert m.pwrite_utf8(0, "héllo") == 6  # é is 2 bytes
    assert m.pread_utf8(0, 6) == "héllo"
    with pytest.raises(ValueError, match="invalid UTF-8"):
        m.pread_utf8(0, 2)  # cuts the 2-byte é in half

    # The bulk-read bounds are checked before the result list is allocated.
    with pytest.raises(ValueError, match="unexpected end of data"):
        m.pread_i32_array(0, 2_000_000_000)
    with pytest.raises(ValueError, match="unexpected end of data"):
        m.pread_i64_array(0, 2_000_000_000)
    m.close()


def test_mmap_cursor_stream(tmp_path):
    m = Mmap.create(str(tmp_path / "stream.bin"))
    assert m.position == 0
    assert m.write(b"hello") == 5
    assert m.write_utf8(" wörld") == 7
    m.write_byte(0x21)
    m.write_i32(-7)
    m.write_i64(1 << 40)
    assert m.position == 5 + 7 + 1 + 4 + 8

    m.rewind()
    assert m.position == 0
    assert m.read(5) == b"hello"
    assert m.read_utf8(7) == " wörld"
    assert m.read_byte() == 0x21
    assert m.read_i32() == -7
    assert m.read_i64() == 1 << 40

    assert m.seek(Whence.Start, 6) == 6
    assert m.seek(Whence.Current, -1) == 5
    assert m.seek(Whence.End, -5) == m.byte_size() - 5
    with pytest.raises(ValueError, match="invalid seek"):
        m.seek(Whence.Start, -1)

    payload = (
        "hello wörld".encode()
        + b"\x21"
        + (-7).to_bytes(4, "little", signed=True)
        + (1 << 40).to_bytes(8, "little")
    )
    m.rewind()
    assert m.read_to_end() == payload
    assert m.position == m.byte_size()
    assert m.read(5) == b""  # at the end

    m.set_position(6)
    assert m.read(5) == payload[6:11]
    pos = m.position
    m.set_position(m.byte_size())
    with pytest.raises(ValueError):
        m.read_i32()  # past the end
    assert m.position == m.byte_size()  # a failed read must not advance
    m.set_position(pos)
    m.close()


def test_mmap_auto_grow_appends_and_size_dunders(tmp_path):
    m = Mmap.create(str(tmp_path / "grow.bin"))
    assert m.byte_size() == 0
    assert m.is_empty()
    assert len(m) == 0
    assert not m  # __bool__ over an empty file
    for i in range(100):
        m.pwrite_i64(i * 8, i)  # every append past the end auto-grows the file
    assert m.byte_size() == 800
    assert len(m) == 800
    assert m.bit_size() == 6400
    assert not m.is_empty()
    assert m
    assert m.pread_i64_array(0, 100) == list(range(100))
    m.pwrite_byte(1000, 0xFF)  # far past the end: the gap arrives zero-filled
    assert m.byte_size() == 1001
    assert m.pread_byte(900) == 0
    m.close()


# -------------------------------------------------------------------------------------
# Mmap: persistence (truncate-on-close) + reopen
# -------------------------------------------------------------------------------------


def test_mmap_persists_with_exact_logical_length(tmp_path):
    p = tmp_path / "persist.bin"
    m = Mmap.create(str(p))
    m.pwrite_utf8(0, "hello mapped world")
    assert m.byte_size() == 18
    assert m.capacity() >= 4096  # page-backed capacity padding while open
    del m  # drop unmaps and truncates the padding away
    gc.collect()

    assert p.stat().st_size == 18  # the on-disk file keeps only the logical length
    back = Mmap.open(str(p))
    assert back.byte_size() == 18
    assert back.pread_utf8(0, 5) == "hello"
    assert back.pread_utf8(6, 6) == "mapped"
    back.close()


def test_mmap_create_keeps_existing_contents(tmp_path):
    p = tmp_path / "keep.bin"
    first = Mmap.create(str(p))
    first.pwrite_utf8(0, "keep me")
    first.close()

    again = Mmap.create(str(p))  # create never truncates an existing file on open
    assert again.pread_utf8(0, 7) == "keep me"
    again.close()


# -------------------------------------------------------------------------------------
# Mmap: read-only mappings
# -------------------------------------------------------------------------------------


def test_mmap_open_readonly_reads_work_writes_are_guided(tmp_path):
    p = tmp_path / "ro.bin"
    m = Mmap.create(str(p))
    m.pwrite_i32(0, 99)
    m.close()

    ro = Mmap.open_readonly(str(p))
    assert ro.mode == IOMode.Read
    assert ro.pread_i32(0) == 99
    assert ro.pread_byte_array(0, 4) == (99).to_bytes(4, "little")
    assert ro.pwrite_byte_array(0, b"XX") == 0  # the write primitives write nothing
    assert ro.pread_i32(0) == 99
    with pytest.raises(ValueError, match="read-only"):
        ro.pwrite_i32(0, 1)  # the full/typed writes name the fix
    with pytest.raises(ValueError, match="read-only"):
        ro.try_reserve(64)
    with pytest.raises(ValueError, match="read-only"):
        ro.try_reserve_exact(64)
    ro.close()


# -------------------------------------------------------------------------------------
# Mmap: capacity family over a file
# -------------------------------------------------------------------------------------


def test_mmap_capacity_family_over_a_file(tmp_path):
    m = Mmap.create(str(tmp_path / "cap.bin"))
    m.pwrite_byte_array(0, b"\x01" * 16)
    assert m.capacity() >= 4096  # never below one page while mapped
    m.try_reserve(8192)
    assert m.capacity() >= 16 + 8192
    assert m.spare_capacity() == m.capacity() - 16
    m.try_reserve_exact(32)  # already satisfied
    m.reserve(1)
    m.reserve_exact(1)
    m.ensure_capacity(10_000)
    m.try_ensure_capacity(10_000)
    assert m.capacity() >= 10_000

    # An overflowing request raises the guided capacity error before touching the mapping.
    with pytest.raises(ValueError, match="reserve less"):
        m.try_reserve(2**64 - 1)
    assert m.pread_byte_array(0, 4) == b"\x01" * 4  # still fully usable

    m.shrink_to(64)
    m.shrink_to_fit()  # remaps to exactly the logical length
    assert m.capacity() == 16
    assert m.byte_size() == 16
    assert m.spare_capacity() == 0
    assert m.pread_byte_array(0, 16) == b"\x01" * 16  # contents survive the remaps
    m.close()


# -------------------------------------------------------------------------------------
# Mmap: metadata (path / uri / kind / mode / headers) + flush + repr
# -------------------------------------------------------------------------------------


def test_mmap_metadata_and_flush(tmp_path):
    p = tmp_path / "meta.bin"
    m = Mmap.create(str(p))
    assert m.kind == IOKind.File
    assert m.mode == IOMode.ReadWrite
    m.set_mode(IOMode.Read)  # a label only; the physical protection is fixed at open
    assert m.mode == IOMode.Read
    m.set_mode(IOMode.ReadWrite)

    assert m.path == str(p)
    assert isinstance(m.uri, Uri)
    # The uri reports the file path back (from_path rewrites back-slashes).
    assert str(m.uri) == str(p).replace("\\", "/")

    assert isinstance(m.headers, Headers)
    assert len(m.headers) == 0
    grabbed = m.headers
    grabbed.insert("a", "1")  # mutating the returned copy...
    assert len(m.headers) == 0  # ...does not touch the mapping until written back
    m.set_headers(grabbed)
    assert m.headers.get("a") == "1"
    # There is deliberately no with_headers/with_mode: they would need a copy, and a
    # live mapping cannot be copied.
    assert not hasattr(m, "with_headers")
    assert not hasattr(m, "with_mode")

    m.pwrite_utf8(0, "flushed")
    m.flush()  # persists the mapped bytes without closing
    assert m.pread_utf8(0, 7) == "flushed"

    assert repr(m) == f"Mmap({p}, <7 bytes>)"
    m.close()


def test_mmap_is_a_live_resource_not_a_value(tmp_path):
    m = Mmap.create(str(tmp_path / "resource.bin"))
    # A live OS resource is deliberately not a value: no equality/copy/codec/pickle and
    # no monomorphic Cursor/Slice builders (they are Heap-only in the binding).
    for absent in (
        "copy",
        "__copy__",
        "__deepcopy__",
        "serialize_bytes",
        "deserialize_bytes",
        "with_capacity",
        "cursor",
        "window",
        "slice",
    ):
        assert not hasattr(m, absent)
    assert m == m  # default identity equality only
    other = Mmap.open(str(tmp_path / "resource.bin"))
    assert m != other
    with pytest.raises(TypeError):
        pickle.dumps(m)  # a mapping does not pickle
    other.close()
    m.close()


# -------------------------------------------------------------------------------------
# Mmap: close() + context manager
# -------------------------------------------------------------------------------------


def test_mmap_close_is_idempotent_and_guards_every_access(tmp_path):
    p = tmp_path / "close.bin"
    m = Mmap.create(str(p))
    m.pwrite_utf8(0, "abc")
    assert not m.closed
    m.close()
    assert m.closed
    m.close()  # double-close is a no-op
    assert m.closed

    with pytest.raises(ValueError, match="closed"):
        m.byte_size()
    with pytest.raises(ValueError, match="closed"):
        m.pread_byte(0)
    with pytest.raises(ValueError, match="closed"):
        m.pwrite_utf8(0, "x")
    with pytest.raises(ValueError, match="closed"):
        m.read_to_end()
    with pytest.raises(ValueError, match="closed"):
        m.flush()
    with pytest.raises(ValueError, match="closed"):
        m.path
    assert repr(m) == "Mmap(<closed>)"  # repr never raises

    assert p.stat().st_size == 3  # close truncated the file to its logical length
    # The guided error names the fix: reopen.
    reopened = Mmap.open(str(p))
    assert reopened.pread_utf8(0, 3) == "abc"
    reopened.close()


def test_mmap_context_manager_closes_and_persists(tmp_path):
    p = tmp_path / "ctx.bin"
    with Mmap.create(str(p)) as m:
        assert m.write(b"managed") == 7
        assert m.capacity() >= 4096  # padded while open...
    assert m.closed  # __exit__ closed it
    assert p.stat().st_size == 7  # ...but on disk only the logical length remains

    with Mmap.open(str(p)) as back:
        assert back.read_to_end() == b"managed"
    assert back.closed


def test_mmap_context_manager_closes_on_exception(tmp_path):
    p = tmp_path / "ctx_err.bin"
    with pytest.raises(RuntimeError, match="boom"):
        with Mmap.create(str(p)) as m:
            m.write(b"partial")
            raise RuntimeError("boom")  # __exit__ must close and re-raise
    assert m.closed
    assert p.stat().st_size == 7
