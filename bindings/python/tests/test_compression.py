"""Tests for the yggdryl.compression Python binding."""

import pickle

import pytest

from yggdryl import compression


def test_gzip_round_trip():
    gzip = compression.Gzip(6)
    original = b"the quick brown fox jumps over the lazy dog" * 16
    compressed = gzip.encode_byte_array(original)
    assert len(compressed) < len(original)
    assert gzip.decode_byte_array(compressed) == original


def test_zstd_round_trip_and_defaults():
    zstd = compression.Zstd()
    assert zstd.level == 3
    assert zstd.name == "zstd"
    lo, hi = compression.Zstd.level_range()
    assert lo <= 3 <= hi

    original = b"the quick brown fox " * 200
    compressed = zstd.encode_byte_array(original)
    assert len(compressed) < len(original)
    assert zstd.decode_byte_array(compressed) == original


def test_zstd_value_and_pickle():
    zstd = compression.Zstd(9)
    assert zstd == compression.Zstd(9)
    assert zstd != compression.Zstd(3)
    assert compression.Zstd.deserialize_bytes(zstd.serialize_bytes()) == zstd
    assert pickle.loads(pickle.dumps(zstd)).level == 9


def test_zstd_streams_between_cursors():
    from yggdryl.io import ByteBuffer

    zstd = compression.Zstd()
    original = b"stream me " * 500
    source = ByteBuffer(original).byte_cursor()
    packed = ByteBuffer().byte_cursor()
    zstd.compress_stream(source, packed)
    packed.seek(0)
    restored = ByteBuffer().byte_cursor()
    zstd.decompress_stream(packed, restored)
    assert restored.as_bytes() == original


def test_gzip_defaults_to_level_six():
    gzip = compression.Gzip()
    assert gzip.level == 6
    assert gzip.name == "gzip"


def test_gzip_rejects_invalid_level():
    with pytest.raises(ValueError):
        compression.Gzip(10)


def test_gzip_rejects_corrupt_stream():
    with pytest.raises(ValueError):
        compression.Gzip().decode_byte_array(b"not a gzip stream")


def test_gzip_round_trips_through_bytes():
    gzip = compression.Gzip(9)
    restored = compression.Gzip.deserialize_bytes(gzip.serialize_bytes())
    assert restored.level == 9


def test_gzip_pickles():
    gzip = compression.Gzip(3)
    restored = pickle.loads(pickle.dumps(gzip))
    assert restored.level == 3
    assert repr(restored) == "Gzip(level=3)"


def test_gzip_equality_and_hashing():
    assert compression.Gzip(6) == compression.Gzip()
    assert compression.Gzip(6) != compression.Gzip(9)
    assert hash(compression.Gzip(6)) == hash(compression.Gzip())
    assert len({compression.Gzip(1), compression.Gzip(1), compression.Gzip(9)}) == 2
