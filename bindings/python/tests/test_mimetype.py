"""Tests for the ``yggdryl.mimetype`` ``MimeType`` and ``MimeCatalog``.

Mirrors ``crates/yggdryl-core/src/mimetype.rs`` on the Python surface: construction and
parsing, the default-catalog resolution front doors (``from_extension`` / ``from_name`` /
``from_magic`` / ``guess``), the accessors (``essence`` / ``type`` / ``subtype`` /
``extensions`` / ``magic``), the ``MimeCatalog`` registry, and the value-type dunders
(``==`` / ``hash`` / ``str`` / pickle / the essence-only byte codec / copy).
"""

import copy
import pickle

import pytest

import yggdryl.mimetype
from yggdryl.mimetype import MimeCatalog, MimeType


def test_module_surface():
    for cls in (MimeType, MimeCatalog):
        assert cls.__module__ == "yggdryl.mimetype"
        assert hasattr(yggdryl.mimetype, cls.__name__)


# -------------------------------------------------------------------------------------
# Construction + parsing
# -------------------------------------------------------------------------------------


def test_parse_drops_parameters_and_lowercases():
    m = MimeType.parse("Application/JSON; charset=utf-8")
    assert m.essence == "application/json"
    assert m.type == "application"
    assert m.subtype == "json"
    assert m.extensions == []
    assert m.magic == []


def test_parse_bad_essence_is_valueerror():
    with pytest.raises(ValueError, match="type/subtype"):
        MimeType.parse("notamime")


def test_construct_with_extensions_and_magic():
    m = MimeType("Image/PNG", [".PNG", "apng"], [b"\x89PNG"])
    assert m.essence == "image/png"  # lowercased
    assert m.extensions == ["png", "apng"]  # lowercased, leading dot stripped
    assert m.magic == [b"\x89PNG"]
    assert m.has_extension("PNG")  # case-insensitive, dot ignored
    assert m.has_extension(".png")
    assert not m.has_extension("gif")
    assert m.matches_magic(b"\x89PNG\r\n more")
    assert not m.matches_magic(b"GIF89a")


def test_octet_stream():
    m = MimeType.octet_stream()
    assert m.essence == "application/octet-stream"
    assert m.is_octet_stream()
    assert not MimeType.parse("text/plain").is_octet_stream()


# -------------------------------------------------------------------------------------
# Default-catalog resolution front doors
# -------------------------------------------------------------------------------------


def test_from_extension_name_magic():
    assert MimeType.from_extension("png").essence == "image/png"
    assert MimeType.from_extension("nope") is None
    assert MimeType.from_name("report.pdf").essence == "application/pdf"
    assert MimeType.from_name("no-extension") is None
    assert MimeType.from_magic(b"\x89PNG\r\n\x1a\n...").essence == "image/png"
    assert MimeType.from_magic(b"not-magic") is None


def test_guess_prefers_magic_then_name_then_octet_stream():
    # Magic wins even when the name says otherwise.
    assert MimeType.guess("mystery.txt", b"%PDF-1.7").essence == "application/pdf"
    # No magic -> the name's extension.
    assert MimeType.guess("data.json", b"").essence == "application/json"
    # Neither -> the octet-stream fallback (never None).
    assert MimeType.guess("mystery", b"nope").is_octet_stream()


# -------------------------------------------------------------------------------------
# Value semantics: eq / hash / str / repr / copy / pickle / byte codec
# -------------------------------------------------------------------------------------


def test_equality_and_hash_include_extensions():
    parsed = MimeType.parse("image/png")  # no extensions
    catalog = MimeType.from_extension("png")  # has extensions + magic
    assert parsed.essence == catalog.essence
    # Equality is over the full value (essence + extensions + magic), so these differ.
    assert parsed != catalog
    # Same value -> equal and equal hash; usable as set/dict keys.
    again = MimeType.from_extension("png")
    assert catalog == again
    assert hash(catalog) == hash(again)
    assert {catalog, again} == {catalog}


def test_str_and_repr():
    m = MimeType.parse("text/html")
    assert str(m) == "text/html"
    assert repr(m) == 'MimeType("text/html")'


def test_copy_and_stdlib_copy():
    m = MimeType.from_extension("png")
    assert m.copy() == m
    assert copy.copy(m) == m
    assert copy.deepcopy(m) == m


def test_pickle_round_trips_extensions_and_magic():
    m = MimeType.from_extension("png")  # carries extensions + magic
    restored = pickle.loads(pickle.dumps(m))
    assert restored == m
    assert restored.extensions == m.extensions
    assert restored.magic == m.magic


def test_byte_codec_is_essence_only():
    m = MimeType.parse("application/json")
    assert m.serialize_bytes() == b"application/json"
    assert MimeType.deserialize_bytes(m.serialize_bytes()) == m
    # The byte form drops extensions/magic (essence is the value identity over the wire).
    rich = MimeType.from_extension("png")
    thin = MimeType.deserialize_bytes(rich.serialize_bytes())
    assert thin.essence == rich.essence
    assert thin.extensions == []
    with pytest.raises(ValueError, match="type/subtype"):
        MimeType.deserialize_bytes(b"not-a-mime")


# -------------------------------------------------------------------------------------
# MimeCatalog registry
# -------------------------------------------------------------------------------------


def test_empty_and_defaults_catalog():
    empty = MimeCatalog()
    assert len(empty) == 0
    assert not empty  # __bool__

    defaults = MimeCatalog.defaults()
    assert len(defaults) == len(defaults.types()) > 0
    assert defaults
    assert defaults.from_extension("png").essence == "image/png"
    assert defaults.from_name("a.json").essence == "application/json"
    assert defaults.from_mime("text/plain").essence == "text/plain"
    assert defaults.from_magic(b"PK\x03\x04...").essence == "application/zip"
    assert defaults.from_extension("no-such-ext") is None


def test_register_mutates_and_with_is_non_mutating():
    catalog = MimeCatalog()
    foo = MimeType("text/x-foo", ["foo"], [b"FOO"])
    catalog.register(foo)
    assert len(catalog) == 1
    assert catalog.from_extension("foo") == foo
    assert catalog.from_magic(b"FOObar") == foo

    # with_ leaves the receiver untouched.
    bar = MimeType("text/x-bar", ["bar"])
    built = catalog.with_(bar)
    assert built.from_extension("bar") == bar
    assert catalog.from_extension("bar") is None  # receiver unchanged
    assert len(catalog) == 1


def test_register_overrides_same_essence():
    catalog = MimeCatalog()
    catalog.register(MimeType("text/x-foo", ["foo"]))
    catalog.register(MimeType("text/x-foo", ["bar"]))  # same essence, later wins
    assert len(catalog) == 1
    assert catalog.from_extension("bar").essence == "text/x-foo"
    assert catalog.from_extension("foo") is None


def test_catalog_copy_and_repr():
    catalog = MimeCatalog.defaults()
    assert len(copy.copy(catalog)) == len(catalog)
    assert repr(MimeCatalog()) == "MimeCatalog(<0 types>)"


# -------------------------------------------------------------------------------------
# names / extension / is_compression / from_alias
# -------------------------------------------------------------------------------------


def test_names_and_primary_extension_accessors():
    gz = MimeType.from_extension("gz")  # a built-in with names + extensions
    assert "gzip" in gz.names
    assert gz.extension == "gz"  # the primary (first) extension
    # A parsed essence has neither names nor extensions.
    parsed = MimeType.parse("application/json")
    assert parsed.names == []
    assert parsed.extension is None


def test_construct_with_names():
    m = MimeType("Application/X-Foo", ["foo", "fo"], [b"FOO"], names=["Foo", "FOOBAR"])
    assert m.essence == "application/x-foo"
    assert m.names == ["foo", "foobar"]  # lowercased
    assert m.extensions == ["foo", "fo"]
    assert m.extension == "foo"
    assert m.magic == [b"FOO"]


def test_is_compression():
    assert MimeType.from_extension("gz").is_compression()
    assert MimeType.from_extension("zst").is_compression()
    assert MimeType.from_extension("xz").is_compression()
    assert not MimeType.from_extension("json").is_compression()
    assert not MimeType.octet_stream().is_compression()


def test_from_alias():
    assert MimeType.from_alias("gzip").essence == "application/gzip"
    assert MimeType.from_alias("zstd").essence == "application/zstd"
    assert MimeType.from_alias("json").essence == "application/json"
    assert MimeType.from_alias("no-such-alias") is None


def test_pickle_round_trips_names():
    m = MimeType.from_extension("gz")  # carries names + extensions + magic
    restored = pickle.loads(pickle.dumps(m))
    assert restored == m
    assert restored.names == m.names
    assert restored.extensions == m.extensions
