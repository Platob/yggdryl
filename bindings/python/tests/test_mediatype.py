"""Tests for the ``yggdryl.mediatype`` ``MediaType`` — an ordered list of ``MimeType``s.

Mirrors ``crates/yggdryl-core/src/mediatype.rs`` on the Python surface: construction from a
mime list, a single type, or a file's extensions; the accessors (``primary`` / ``types`` /
``essences`` / ``contains`` / ``len``); ``push``; and the value-type dunders (``==`` / ``hash``
/ ``str`` / pickle / the comma-joined byte codec / copy).
"""

import copy
import pickle

import pytest

import yggdryl.mediatype
from yggdryl.mediatype import MediaType
from yggdryl.mimetype import MimeType


def test_module_surface():
    assert MediaType.__module__ == "yggdryl.mediatype"
    assert hasattr(yggdryl.mediatype, "MediaType")


# -------------------------------------------------------------------------------------
# Construction
# -------------------------------------------------------------------------------------


def test_empty_and_from_types():
    empty = MediaType()
    assert len(empty) == 0
    assert not empty  # __bool__
    assert empty.primary() is None
    assert empty.essences() == []

    m = MediaType([MimeType.parse("application/json"), MimeType.parse("text/html")])
    assert len(m) == 2
    assert m.essences() == ["application/json", "text/html"]


def test_parse_comma_list_dropping_params():
    m = MediaType.parse("application/json; q=0.9, text/html")
    assert m.essences() == ["application/json", "text/html"]
    assert m.primary().essence == "application/json"
    # Empty items are skipped.
    assert MediaType.parse("application/json,, ,text/html").essences() == [
        "application/json",
        "text/html",
    ]
    with pytest.raises(ValueError, match="type/subtype"):
        MediaType.parse("application/json, notamime")


def test_of_single_type():
    m = MediaType.of(MimeType.parse("application/pdf"))
    assert len(m) == 1
    assert m.primary().essence == "application/pdf"


def test_from_extensions_builds_the_layer_stack():
    tgz = MediaType.from_extensions(["tar", "gz"])
    assert tgz.essences() == ["application/x-tar", "application/gzip"]
    # Unknown extensions are skipped.
    assert MediaType.from_extensions(["json", "nope"]).essences() == ["application/json"]


# -------------------------------------------------------------------------------------
# Accessors + push
# -------------------------------------------------------------------------------------


def test_primary_types_contains():
    m = MediaType.parse("application/x-tar, application/gzip")
    assert m.primary().essence == "application/x-tar"
    assert [t.essence for t in m.types()] == ["application/x-tar", "application/gzip"]
    assert m.contains("application/gzip")
    assert m.contains("APPLICATION/GZIP")  # case-insensitive
    assert not m.contains("text/plain")


def test_push_appends():
    m = MediaType.of(MimeType.parse("application/x-tar"))
    m.push(MimeType.parse("application/gzip"))
    assert m.essences() == ["application/x-tar", "application/gzip"]


# -------------------------------------------------------------------------------------
# Value semantics: eq / hash / str / repr / copy / pickle / byte codec
# -------------------------------------------------------------------------------------


def test_equality_and_hash():
    a = MediaType.parse("application/json, text/html")
    b = MediaType.parse("application/json, text/html")
    assert a == b
    assert hash(a) == hash(b)
    assert {a, b} == {a}
    assert a != MediaType.parse("application/json")


def test_str_and_repr():
    m = MediaType.parse("application/x-tar, application/gzip")
    assert str(m) == "application/x-tar, application/gzip"
    assert repr(m) == 'MediaType("application/x-tar, application/gzip")'


def test_copy_and_stdlib_copy():
    m = MediaType.from_extensions(["tar", "gz"])
    assert m.copy() == m
    assert copy.copy(m) == m
    assert copy.deepcopy(m) == m


def test_pickle_round_trips_faithfully():
    # from_extensions carries MimeTypes with extensions — pickle preserves them.
    m = MediaType.from_extensions(["tar", "gz"])
    restored = pickle.loads(pickle.dumps(m))
    assert restored == m
    assert [t.extensions for t in restored.types()] == [t.extensions for t in m.types()]


def test_byte_codec_round_trip():
    m = MediaType.parse("application/x-tar, application/gzip")
    assert m.serialize_bytes() == b"application/x-tar, application/gzip"
    assert MediaType.deserialize_bytes(m.serialize_bytes()) == m
    with pytest.raises(ValueError, match="type/subtype"):
        MediaType.deserialize_bytes(b"application/json, notamime")
