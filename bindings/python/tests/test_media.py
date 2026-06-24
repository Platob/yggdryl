"""Tests for the yggdryl Python extension's MediaType.

Run after building the module, e.g. ``maturin develop`` then ``pytest``.
"""

import pytest

import yggdryl


def test_mime_parse_and_components():
    m = yggdryl.MediaType("application/json")
    assert m.mime == "application/json"
    assert m.type == "application"
    assert m.subtype == "json"
    assert str(m) == "application/json"
    assert m.is_known
    # Parameters are dropped; case is normalised.
    assert yggdryl.MediaType("Text/HTML; charset=utf-8").subtype == "html"


def test_from_extension():
    assert yggdryl.MediaType.from_extension("parquet").mime == "application/vnd.apache.parquet"
    assert yggdryl.MediaType.from_extension(".GZ").mime == "application/gzip"
    assert yggdryl.MediaType.from_extension("png").subtype == "png"
    assert yggdryl.MediaType.from_extension("nope") is None


def test_from_magic():
    assert yggdryl.MediaType.from_magic(b"PAR1\x15\x04").mime == "application/vnd.apache.parquet"
    assert yggdryl.MediaType.from_magic(b"ARROW1\x00\x00").mime == "application/vnd.apache.arrow.file"
    assert yggdryl.MediaType.from_magic(b"PK\x03\x04\x14").subtype == "zip"
    assert yggdryl.MediaType.from_magic(b"\x1f\x8b\x08\x00").mime == "application/gzip"
    assert yggdryl.MediaType.from_magic(b"\x89PNG\r\n\x1a\n").subtype == "png"
    assert yggdryl.MediaType.from_magic(b"not magic") is None


def test_unknown_other():
    m = yggdryl.MediaType("application/x-custom")
    assert not m.is_known
    assert m.subtype == "x-custom"
    assert m.extension is None
    assert m.extensions == []


def test_invalid_mime_raises():
    with pytest.raises(ValueError):
        yggdryl.MediaType("notamime")
    assert yggdryl.MediaType("notamime", safe=False).mime == "notamime"


def test_to_from_mapping():
    m = yggdryl.MediaType("image/svg+xml")
    assert m.to_mapping() == {"type": "image", "subtype": "svg+xml"}
    assert yggdryl.MediaType.from_mapping({"type": "text", "subtype": "csv"}).mime == "text/csv"


def test_extensions_and_equality():
    jpeg = yggdryl.MediaType.from_extension("jpg")
    assert jpeg.extensions == ["jpg", "jpeg"]
    assert jpeg.extension == "jpg"
    assert jpeg == yggdryl.MediaType("image/jpeg")
    assert hash(jpeg) == hash(yggdryl.MediaType("image/jpeg"))


def test_from_path():
    assert yggdryl.MediaType.from_path("/data/sales.parquet").mime == "application/vnd.apache.parquet"
    assert yggdryl.MediaType.from_path("archive.tar.gz").mime == "application/gzip"
    assert yggdryl.MediaType.from_path("/usr/bin/env") is None


def test_uri_url_media_type():
    assert yggdryl.Uri("https://h/a/file.json").media_type().mime == "application/json"
    assert yggdryl.Url("https://h/data/sales.parquet").media_type().subtype == "vnd.apache.parquet"
    assert yggdryl.Uri("file:/dump/archive.tar.gz").media_type().mime == "application/gzip"
    assert yggdryl.Uri("https://h/page").media_type() is None
