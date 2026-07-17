"""Tests for the ``yggdryl.io`` ``Headers`` map.

Mirrors ``crates/yggdryl-core/src/io/headers.rs`` on the Python surface: construction,
case-insensitive multi-value access, append vs insert-replace, the raw ``*_bytes``
accessors, the HTTP text form and the binary byte codec, the ``dict`` protocol
(``len`` / ``in`` / ``h[name]`` / ``del`` / iteration), and the value semantics
(``==`` / copy / pickle / unhashability — ``Headers`` is mutable like ``dict``).
"""

import copy
import pickle

import pytest

import yggdryl.io
from yggdryl.headers import Headers
from yggdryl.mediatype import MediaType
from yggdryl.mimetype import MimeType


def test_module_surface():
    assert Headers.__module__ == "yggdryl.headers"
    assert hasattr(yggdryl.headers, "Headers")


# -------------------------------------------------------------------------------------
# Construction
# -------------------------------------------------------------------------------------


def test_construct_empty_and_with_capacity():
    h = Headers()
    assert len(h) == 0
    assert not h  # __bool__

    pre = Headers.with_capacity(8)
    assert len(pre) == 0
    assert pre == h  # capacity is not part of the value


def test_parse_http():
    h = Headers.parse_http(b"Host: example.com\r\nAccept: */*\r\n")
    assert h.get("host") == "example.com"
    assert h.get("accept") == "*/*"
    # Lenient: a blank line stops parsing, a colon-less line is skipped.
    partial = Headers.parse_http(b"A: 1\r\nno-colon-line\r\n\r\nB: 2\r\n")
    assert partial.get("a") == "1"
    assert partial.get("b") is None


# -------------------------------------------------------------------------------------
# Case-insensitive get + multi-value append vs insert-replace
# -------------------------------------------------------------------------------------


def test_get_is_case_insensitive():
    h = Headers()
    h.insert("Content-Type", "text/plain")
    assert h.get("content-type") == "text/plain"
    assert h.get("CONTENT-TYPE") == "text/plain"
    assert h.get("absent") is None
    assert h.contains("Content-Type")
    assert h.contains("content-TYPE")
    assert not h.contains("absent")


def test_append_keeps_and_insert_replaces():
    h = Headers()
    h.append("Set-Cookie", "a=1")
    h.append("Set-Cookie", "b=2")
    assert h.get_all("set-cookie") == ["a=1", "b=2"]
    assert h.get("set-cookie") == "a=1"  # first value
    assert len(h) == 2  # a repeated name counts once per occurrence

    h.insert("Set-Cookie", "only=1")  # replace semantics: removes both, appends one
    assert h.get_all("set-cookie") == ["only=1"]
    assert len(h) == 1


def test_bytes_accessors_and_non_utf8_values():
    h = Headers()
    h.append_bytes(b"X-Bin", b"\xff\xfe")
    assert h.get_bytes(b"x-bin") == b"\xff\xfe"
    assert h.get_all_bytes(b"X-BIN") == [b"\xff\xfe"]
    assert h.get("x-bin") is None  # not valid UTF-8 -> the str accessor misses
    assert h.get_all("x-bin") == []

    h.insert_bytes(b"X-Bin", b"\x01")
    assert h.get_bytes(b"x-bin") == b"\x01"
    assert len(h) == 1


def test_items_and_keys_and_iteration():
    h = Headers()
    h.append("A", "1")
    h.append("b", "2")
    h.append("A", "3")
    assert h.items() == [(b"A", b"1"), (b"b", b"2"), (b"A", b"3")]
    assert h.keys() == ["A", "b", "A"]  # insertion order, one per occurrence
    assert list(h) == ["A", "b", "A"]  # __iter__ mirrors keys()

    # Non-UTF-8 names are skipped by the str-side keys()/iteration.
    h.append_bytes(b"\xff", b"v")
    assert h.keys() == ["A", "b", "A"]
    assert len(h) == 4  # ...but the entry itself is present


# -------------------------------------------------------------------------------------
# remove / clear / with_ builder / merge_with
# -------------------------------------------------------------------------------------


def test_remove_returns_count_and_clear():
    h = Headers()
    h.append("A", "1")
    h.append("a", "2")
    h.append("B", "3")
    assert h.remove("A") == 2  # case-insensitive, every occurrence
    assert h.remove("A") == 0
    assert len(h) == 1
    h.clear()
    assert len(h) == 0
    assert not h


def test_remove_bytes_reaches_non_utf8_names():
    h = Headers()
    h.append_bytes(b"\xff", b"v")
    h.append("Keep", "k")
    assert len(h) == 2
    assert h.remove_bytes(b"\xff") == 1  # a non-UTF-8 name is removable by bytes
    assert h.remove_bytes(b"\xff") == 0
    assert len(h) == 1
    assert h.remove_bytes(b"keep") == 1  # still case-insensitive
    assert len(h) == 0


def test_with_builder_chains_without_mutating():
    base = Headers()
    built = base.with_("a", "1").with_("b", "2")
    assert built.get("a") == "1"
    assert built.get("b") == "2"
    assert len(base) == 0  # the builder never mutates its receiver


def test_merge_with_overlays_names():
    base = Headers()
    base.append("Keep", "k")
    base.append("Set-Cookie", "old=1")
    base.append("Set-Cookie", "old=2")

    overlay = Headers()
    overlay.append("Set-Cookie", "new=1")
    overlay.append("Extra", "e")

    merged = base.merge_with(overlay)
    assert merged.get("keep") == "k"  # names only base carries are kept
    assert merged.get_all("set-cookie") == ["new=1"]  # other's name replaces all
    assert merged.get("extra") == "e"
    # The receivers are untouched.
    assert base.get_all("set-cookie") == ["old=1", "old=2"]
    assert overlay.get("keep") is None


# -------------------------------------------------------------------------------------
# Typed conveniences
# -------------------------------------------------------------------------------------


def test_content_type_and_content_length():
    h = Headers()
    assert h.content_type() is None
    assert h.content_length() is None
    h.insert("content-type", "application/json")
    h.insert("content-length", " 1024 ")  # trimmed before parsing
    assert h.content_type() == "application/json"
    assert h.content_length() == 1024
    h.insert("content-length", "not-a-number")
    assert h.content_length() is None


# -------------------------------------------------------------------------------------
# HTTP round-trip + byte codec
# -------------------------------------------------------------------------------------


def test_http_round_trip():
    h = Headers()
    h.insert("Host", "example.com")
    h.append("Set-Cookie", "a=1")
    h.append("Set-Cookie", "b=2")
    wire = h.to_http_bytes()
    assert wire == b"Host: example.com\r\nSet-Cookie: a=1\r\nSet-Cookie: b=2\r\n"
    assert Headers.parse_http(wire) == h


def test_byte_codec_round_trip_and_truncated():
    h = Headers()
    h.append("Set-Cookie", "a=1")
    h.append("Set-Cookie", "b=2")
    h.append_bytes(b"X-Bin", b"\r\n:\xff")  # arbitrary bytes survive the binary frame
    data = h.serialize_bytes()
    assert Headers.deserialize_bytes(data) == h
    with pytest.raises(ValueError, match="unexpected end of data"):
        Headers.deserialize_bytes(data[:-1])


# -------------------------------------------------------------------------------------
# dict protocol
# -------------------------------------------------------------------------------------


def test_mapping_dunders():
    h = Headers()
    h["a"] = "1"  # __setitem__ = insert/replace
    assert h["a"] == "1"  # __getitem__ = first value
    assert "A" in h  # __contains__ is case-insensitive
    assert len(h) == 1

    h.append("a", "2")
    assert h["a"] == "1"  # still the first value
    h["a"] = "3"  # replace collapses both entries
    assert h.get_all("a") == ["3"]

    del h["A"]  # __delitem__ removes every occurrence
    assert len(h) == 0
    with pytest.raises(KeyError):
        del h["a"]
    with pytest.raises(KeyError):
        h["absent"]


def test_getitem_present_non_utf8_value_is_valueerror_not_keyerror():
    h = Headers()
    h.append_bytes(b"X-Bin", b"\xff\xfe")
    assert "x-bin" in h  # membership sees the entry...
    # ...so subscription must not raise KeyError (the dict invariant `k in h` =>
    # `h[k]` never KeyErrors); the value simply is not text, and the guided error
    # points at get_bytes.
    with pytest.raises(ValueError, match="not valid UTF-8"):
        h["x-bin"]
    assert h.get_bytes(b"x-bin") == b"\xff\xfe"  # the raw read works


def test_headers_is_unhashable_like_dict():
    h = Headers()
    with pytest.raises(TypeError):
        {h}  # noqa: B018 - mutable map must be unhashable
    with pytest.raises(TypeError):
        hash(h)


# -------------------------------------------------------------------------------------
# Value semantics: eq / copy / pickle / repr
# -------------------------------------------------------------------------------------


def test_equality():
    a = Headers().with_("A", "1")
    b = Headers().with_("A", "1")
    assert a == b
    assert a != Headers().with_("A", "2")
    assert a != Headers().with_("a", "1")  # equality is exact bytes (order + case)


def test_copy_and_stdlib_copy_module():
    base = Headers().with_("a", "1")
    dup = base.copy()
    assert dup == base
    dup.insert("a", "changed")  # mutating the copy leaves the original untouched
    assert base.get("a") == "1"

    assert copy.copy(base) == base
    assert copy.deepcopy(base) == base
    indep = copy.copy(base)
    indep.insert("a", "x")
    assert base.get("a") == "1"


def test_pickle_round_trip():
    h = Headers()
    h.append("Set-Cookie", "a=1")
    h.append("Set-Cookie", "b=2")
    h.append_bytes(b"X-Bin", b"\xff")
    assert pickle.loads(pickle.dumps(h)) == h


def test_repr():
    h = Headers().with_("Host", "example.com")
    assert repr(h) == 'Headers({"Host": "example.com"})'


# -------------------------------------------------------------------------------------
# Header-name constants + Content-Encoding / set_content_type
# -------------------------------------------------------------------------------------


def test_header_name_constants():
    assert Headers.CONTENT_TYPE == "Content-Type"
    assert Headers.CONTENT_LENGTH == "Content-Length"
    assert Headers.CONTENT_ENCODING == "Content-Encoding"
    assert Headers.HOST == "Host"
    assert Headers.LAST_MODIFIED == "Last-Modified"
    assert Headers.MTIME == "X-Mtime-Us"
    # The constants are usable as keys and match case-insensitively.
    h = Headers()
    h.insert(Headers.CONTENT_TYPE, "text/plain")
    assert h.get("content-type") == "text/plain"


def test_content_type_and_encoding_setters():
    h = Headers()
    assert h.content_encoding() is None
    h.set_content_type("application/json")
    h.set_content_encoding("gzip")
    assert h.content_type() == "application/json"
    assert h.content_encoding() == "gzip"


# -------------------------------------------------------------------------------------
# mime_type / media_type interpretation of Content-Type / Content-Encoding
# -------------------------------------------------------------------------------------


def test_mime_type_none_until_content_type_set():
    h = Headers()
    assert h.mime_type() is None
    assert h.media_type() is None
    h.set_content_type("application/json; charset=utf-8")
    mime = h.mime_type()
    assert isinstance(mime, MimeType)
    assert mime.essence == "application/json"  # parameters dropped, primary of a list
    media = h.media_type()
    assert isinstance(media, MediaType)
    assert media.essences() == ["application/json"]


def test_media_type_extends_with_content_encoding():
    h = Headers()
    h.set_content_type("application/x-tar")
    h.set_content_encoding("gzip")
    assert h.media_type().essences() == ["application/x-tar", "application/gzip"]
    # The primary mime type is just the Content-Type half.
    assert h.mime_type().essence == "application/x-tar"


def test_set_mime_type_and_set_media_type():
    h = Headers()
    h.set_mime_type(MimeType.parse("application/pdf"))
    assert h.content_type() == "application/pdf"
    h.set_media_type(MediaType.parse("application/x-tar, application/gzip"))
    assert h.content_type() == "application/x-tar, application/gzip"
    # Round-trips back through media_type().
    assert h.media_type().essences() == ["application/x-tar", "application/gzip"]


# -------------------------------------------------------------------------------------
# mtime (epoch microseconds)
# -------------------------------------------------------------------------------------


def test_mtime_round_trip_and_absent():
    h = Headers()
    assert h.mtime() is None
    h.set_mtime(1_600_000_000_000_000)
    assert h.mtime() == 1_600_000_000_000_000
    assert h.get(Headers.MTIME) == "1600000000000000"
    # Signed — before 1970 is negative.
    h.set_mtime(-1_234)
    assert h.mtime() == -1_234
    # A non-integer value reads back as None.
    h.insert(Headers.MTIME, "not-a-number")
    assert h.mtime() is None
