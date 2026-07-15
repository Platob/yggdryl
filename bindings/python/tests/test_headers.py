"""Tests for ``yggdryl.io.Headers`` — the centralized, case-insensitive, ordered, multi-value
string map that backs both HTTP headers and a ``Field``'s metadata."""

import copy
import pickle

import pytest

import yggdryl
from yggdryl.io import Headers


def test_module_surface():
    assert Headers.__module__ == "yggdryl.io"
    assert hasattr(yggdryl.io, "Headers")


def test_dict_like_and_insert_replaces():
    h = Headers()
    assert len(h) == 0 and h.get("x") is None and h.get("x", "d") == "d"
    h["a"] = "1"
    h.insert("b", "2")
    assert len(h) == 2
    assert h["a"] == "1" and "b" in h and "c" not in h
    assert h.keys() == ["a", "b"] and h.values() == ["1", "2"]
    assert h.items() == [("a", "1"), ("b", "2")]
    assert h.to_dict() == {"a": "1", "b": "2"}

    with pytest.raises(KeyError):
        _ = h["missing"]
    # insert replaces (single-value); remove returns the count removed.
    h.insert("a", "9")
    assert h["a"] == "9" and len(h) == 2
    assert h.remove("a") == 1 and h.remove("a") == 0
    del h["b"]
    assert len(h) == 0
    with pytest.raises(KeyError):
        del h["gone"]


def test_case_insensitive_and_insertion_ordered():
    h = Headers({"Content-Type": "application/json"})
    assert "content-type" in h and "CONTENT-TYPE" in h  # case-insensitive
    assert h.get("content-type") == "application/json"

    # Equality is order-significant (unlike a plain dict/set): insertion order matters.
    a = Headers()
    a.append("x", "1")
    a.append("y", "2")
    b = Headers()
    b.append("y", "2")
    b.append("x", "1")
    assert a != b
    assert a == Headers({"x": "1", "y": "2"})


def test_append_is_multi_value():
    h = Headers()
    h.append("Set-Cookie", "a=1")
    h.append("set-cookie", "b=2")  # case-insensitive name, kept as a second value
    assert h.get_all("Set-Cookie") == ["a=1", "b=2"]
    assert h.get("set-cookie") == "a=1"  # first value
    assert len(h) == 2

    # insert collapses the multi-value back to one.
    h.insert("SET-COOKIE", "c=3")
    assert h.get_all("set-cookie") == ["c=3"] and len(h) == 1


def test_with_entry_is_non_mutating():
    base = Headers({"a": "1"})
    extended = base.with_entry("z", "9")
    assert extended.items() == [("a", "1"), ("z", "9")]
    assert base.items() == [("a", "1")]  # base untouched


def test_constructor_dict_roundtrip_and_clear():
    original = {"unit": "°C", "source": "sensor-3"}
    h = Headers(original)
    assert h.to_dict() == original
    h.clear()
    assert len(h) == 0 and not h

    with pytest.raises(ValueError, match="Headers or a dict"):
        Headers(123)


def test_iter_and_bool():
    h = Headers({"z": "1", "a": "2", "m": "3"})
    assert list(h) == ["z", "a", "m"]  # __iter__ yields keys in insertion order
    assert {k: h[k] for k in h} == {"a": "2", "m": "3", "z": "1"}
    assert bool(h) is True
    assert bool(Headers()) is False  # __bool__


def test_http_conveniences():
    h = Headers()
    h.insert(Headers.CONTENT_TYPE, "text/html; charset=utf-8")
    h.insert(Headers.CONTENT_LENGTH, "2048")
    assert h.content_type == "text/html; charset=utf-8"
    assert h.content_length == 2048

    h.append("Accept", "text/html")
    h.append("Accept", "application/json")
    wire = h.to_http_bytes()
    assert b"Accept: text/html\r\n" in wire and b"Accept: application/json\r\n" in wire

    parsed = Headers.parse_http(b"Host: example.com\r\nAccept: */*\r\n\r\nignored: body")
    assert parsed.get("host") == "example.com"
    assert parsed.get("accept") == "*/*"
    assert "ignored" not in parsed  # stops at the blank line


def test_byte_codec_round_trips_multi_value():
    h = Headers()
    h.append("Set-Cookie", "a=1")
    h.append("Set-Cookie", "b=2")
    h.insert("Host", "example.com")
    blob = h.serialize_bytes()
    assert Headers.deserialize_bytes(blob) == h  # multi-value + order preserved

    with pytest.raises(ValueError):
        Headers.deserialize_bytes(b"\x01\x00\x00\x00")  # truncated frame


def test_is_mutable_hence_unhashable():
    # Like dict/bytearray, a mutable map is not hashable.
    with pytest.raises(TypeError):
        hash(Headers({"a": "1"}))


def test_pickle_and_copy():
    h = Headers()
    h.append("Set-Cookie", "a=1")
    h.append("Set-Cookie", "b=2")
    assert pickle.loads(pickle.dumps(h)) == h  # lossless (multi-value survives)

    dup = copy.copy(h)
    dup.insert("X", "1")
    assert "X" not in h  # copy is independent
    assert copy.deepcopy(h) == h

    explicit = h.copy()
    assert explicit == h and explicit is not h
