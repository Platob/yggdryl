"""Tests for the yggdryl Python extension's MimeType and MediaType.

Run after building the module, e.g. ``maturin develop`` then ``pytest``.
"""

import pytest

import yggdryl


def test_mime_parse_and_components():
    m = yggdryl.MimeType("application/json")
    assert m.mime == "application/json"
    assert m.type == "application"
    assert m.subtype == "json"
    assert str(m) == "application/json"
    assert m.is_known
    # Parameters are dropped; case is normalised.
    assert yggdryl.MimeType("Text/HTML; charset=utf-8").subtype == "html"


def test_mime_from_extension():
    assert yggdryl.MimeType.from_extension("parquet").mime == "application/vnd.apache.parquet"
    assert yggdryl.MimeType.from_extension(".GZ").mime == "application/gzip"
    assert yggdryl.MimeType.from_extension("png").subtype == "png"
    assert yggdryl.MimeType.from_extension("nope") is None


def test_mime_from_magic():
    assert yggdryl.MimeType.from_magic(b"PAR1\x15\x04").mime == "application/vnd.apache.parquet"
    assert yggdryl.MimeType.from_magic(b"ARROW1\x00\x00").mime == "application/vnd.apache.arrow.file"
    assert yggdryl.MimeType.from_magic(b"PK\x03\x04\x14").subtype == "zip"
    assert yggdryl.MimeType.from_magic(b"\x1f\x8b\x08\x00").mime == "application/gzip"
    assert yggdryl.MimeType.from_magic(b"\x89PNG\r\n\x1a\n").subtype == "png"
    assert yggdryl.MimeType.from_magic(b"not magic") is None


def test_mime_unknown_other():
    m = yggdryl.MimeType("application/x-custom")
    assert not m.is_known
    assert m.subtype == "x-custom"
    assert m.extension is None
    assert m.extensions == []


def test_mime_invalid_raises():
    with pytest.raises(ValueError):
        yggdryl.MimeType("notamime")
    assert yggdryl.MimeType("notamime", safe=False).mime == "notamime"


def test_mime_to_from_mapping_and_equality():
    m = yggdryl.MimeType("image/svg+xml")
    assert m.to_mapping() == {"type": "image", "subtype": "svg+xml"}
    assert yggdryl.MimeType.from_mapping({"type": "text", "subtype": "csv"}).mime == "text/csv"
    jpeg = yggdryl.MimeType.from_extension("jpg")
    assert jpeg.extensions == ["jpg", "jpeg"]
    assert jpeg == yggdryl.MimeType("image/jpeg")
    assert hash(jpeg) == hash(yggdryl.MimeType("image/jpeg"))


def test_registry_add_and_remove():
    assert yggdryl.MimeType.from_extension("ygg") is None
    try:
        yggdryl.MimeType.register("application/x-yggdryl", ["ygg"], [b"YGG1"])
        m = yggdryl.MimeType.from_extension("ygg")
        assert m.mime == "application/x-yggdryl"
        assert m.extensions == ["ygg"]
        assert yggdryl.MimeType.from_magic(b"YGG1\x00").mime == "application/x-yggdryl"
        assert yggdryl.MimeType.unregister("application/x-yggdryl")
        assert yggdryl.MimeType.from_extension("ygg") is None
        assert not yggdryl.MimeType.unregister("application/x-yggdryl")
    finally:
        yggdryl.MimeType.unregister("application/x-yggdryl")


def test_media_type_is_ordered_stack():
    stack = yggdryl.MediaType.from_path("data.csv.gz")
    types = stack.types
    assert [t.mime for t in types] == ["text/csv", "application/gzip"]
    assert stack.first.mime == "text/csv"
    assert stack.last.mime == "application/gzip"
    assert len(stack) == 2
    assert stack[0] == yggdryl.MimeType("text/csv")
    assert str(stack) == "csv.gz"


def test_media_type_explicit_construction():
    stack = yggdryl.MediaType([yggdryl.MimeType("text/csv"), yggdryl.MimeType("application/gzip")])
    assert stack == yggdryl.MediaType.from_path("x.csv.gz")
    # Empty stacks are falsy.
    assert not yggdryl.MediaType.from_path("/usr/bin/env")
    assert len(yggdryl.MediaType([])) == 0


def test_convenient_from_constructors():
    # MimeType: single outermost type from a path.
    assert yggdryl.MimeType.from_path("data.csv.gz").mime == "application/gzip"
    assert yggdryl.MimeType.from_path("notes") is None
    # MediaType: from one or many extensions, and from a mapping.
    assert [t.mime for t in yggdryl.MediaType.from_extension("json").types] == ["application/json"]
    assert [t.mime for t in yggdryl.MediaType.from_extensions(["csv", "nope", "gz"]).types] == [
        "text/csv",
        "application/gzip",
    ]
    assert yggdryl.MediaType.from_mapping({"path": "report.csv.gz"}) == yggdryl.MediaType.from_path("report.csv.gz")


def test_default_octet_stream_fallback():
    assert yggdryl.MimeType.default().mime == "application/octet-stream"
    assert [t.mime for t in yggdryl.MediaType.default().types] == ["application/octet-stream"]
    # Conventional fallback for failed inference.
    assert (yggdryl.MimeType.from_path("notes") or yggdryl.MimeType.default()).mime == "application/octet-stream"


def test_uri_url_media_type():
    uri = yggdryl.Uri("https://h/a/file.json")
    assert [t.mime for t in uri.media_type().types] == ["application/json"]
    url = yggdryl.Url("https://h/dump/archive.tar.gz")
    assert [t.mime for t in url.media_type().types] == ["application/x-tar", "application/gzip"]
    # Single outermost type via mime_type(); From a Uri/Url struct.
    assert url.mime_type().mime == "application/gzip"
    assert yggdryl.Uri("https://h/page").media_type() is None
    assert yggdryl.Uri("https://h/page").mime_type() is None
