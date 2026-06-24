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
