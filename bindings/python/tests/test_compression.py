"""Tests for yggdryl.Compression — naming, parsing and round-trips.

Run after building the module, e.g. ``maturin develop`` then ``pytest``.
"""

import pytest

import yggdryl


def test_parses_names_and_extensions():
    assert yggdryl.Compression("gzip").name == "gzip"
    assert yggdryl.Compression.from_str("GZ").name == "gzip"
    assert yggdryl.Compression("zst").name == "zstd"
    assert yggdryl.Compression(" snappy ").name == "snappy"
    assert yggdryl.Compression("store").name == "none"

    assert yggdryl.Compression("gzip").extension == "gz"
    assert yggdryl.Compression("none").extension is None
    assert yggdryl.Compression.from_extension(".zst").name == "zstd"
    assert yggdryl.Compression.from_extension("txt") is None

    with pytest.raises(ValueError):
        yggdryl.Compression("lzo")


def test_none_is_identity():
    codec = yggdryl.Compression("none")
    assert codec.is_available is True
    payload = b"the quick brown fox"
    assert codec.compress(payload) == payload
    assert codec.decompress(payload) == payload


@pytest.mark.parametrize("name", ["gzip", "zstd", "snappy"])
def test_round_trips_each_codec(name):
    codec = yggdryl.Compression(name)
    assert codec.is_available is True  # the wheel enables all three backends
    payload = bytes((i % 251) for i in range(4096))
    packed = codec.compress(payload)
    assert codec.decompress(packed) == payload


def test_equality_and_repr():
    assert yggdryl.Compression("gzip") == yggdryl.Compression("gz")
    assert yggdryl.Compression("gzip") != yggdryl.Compression("zstd")
    assert repr(yggdryl.Compression("zstd")) == "Compression('zstd')"
    assert str(yggdryl.Compression("zstd")) == "zstd"


def test_from_mime():
    gzip = yggdryl.Compression.from_mime(yggdryl.MimeType("application/gzip"))
    assert gzip.name == "gzip"
    assert yggdryl.Compression.from_mime(yggdryl.MimeType("application/json")) is None


@pytest.mark.parametrize("kind", ["bytesio", "localpath"])
def test_io_compress_decompress(tmp_path, kind):
    payload = bytes((i % 251) for i in range(2048))

    def make(data):
        if kind == "bytesio":
            return yggdryl.BytesIO(data)
        path = str(tmp_path / "data.bin")
        yggdryl.LocalPath(path).write(data)
        return yggdryl.LocalPath(path)

    # Compress, then decompress passing the codec explicitly.
    packed = make(payload).compress("zstd")
    assert isinstance(packed, yggdryl.BytesIO)
    assert packed.decompress("zstd").getvalue() == payload


def test_io_decompress_infers_codec_from_extension(tmp_path):
    payload = b"inferred from the .gz extension"
    packed = yggdryl.BytesIO(payload).compress("gzip").getvalue()
    path = str(tmp_path / "data.txt.gz")
    yggdryl.LocalPath(path).write(packed)
    # No codec given -> inferred as gzip from the `.gz` suffix.
    assert yggdryl.LocalPath(path).decompress().getvalue() == payload


def test_io_decompress_infers_codec_from_magic_bytes():
    # An in-memory buffer has no extension, so the codec is sniffed from the
    # gzip magic bytes.
    packed = yggdryl.BytesIO(b"sniffed from magic").compress("gzip").getvalue()
    assert yggdryl.BytesIO(packed).decompress().getvalue() == b"sniffed from magic"
