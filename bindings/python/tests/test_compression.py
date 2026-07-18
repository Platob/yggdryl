"""Tests for the ``yggdryl.compression`` codecs and the ``codec_for`` resolver.

Mirrors ``crates/yggdryl-core/src/compression.rs`` on the Python surface: the four native
codecs (``Gzip`` / ``Zlib`` / ``Zstd`` / ``Lzma``) — construction with an optional ``level``,
the ``essence`` / ``name`` properties, and the ``compress`` / ``decompress`` byte round-trip
(shrinking a compressible payload, handling empty input, and raising a guided ``ValueError``
on a corrupt stream) — plus the module-level ``codec_for`` resolver from a mime essence string
or a ``yggdryl.mimetype.MimeType``.
"""

import pytest

import yggdryl.compression
from yggdryl.compression import Gzip, Lzma, Zlib, Zstd, codec_for
from yggdryl.mimetype import MimeType

# (class, essence, short name) for each native codec.
CODECS = [
    (Gzip, "application/gzip", "gzip"),
    (Zlib, "application/zlib", "zlib"),
    (Zstd, "application/zstd", "zstd"),
    (Lzma, "application/x-xz", "xz"),
]
CLASSES = [row[0] for row in CODECS]


def test_module_surface():
    for cls in CLASSES:
        assert cls.__module__ == "yggdryl.compression"
        assert hasattr(yggdryl.compression, cls.__name__)
    assert hasattr(yggdryl.compression, "codec_for")


# -------------------------------------------------------------------------------------
# essence / name / repr
# -------------------------------------------------------------------------------------


@pytest.mark.parametrize("cls,essence,name", CODECS)
def test_essence_name_and_repr(cls, essence, name):
    codec = cls()
    assert codec.essence == essence
    assert codec.name == name
    assert cls.__name__ in repr(codec)
    assert essence in repr(codec)


# -------------------------------------------------------------------------------------
# Round-trip + shrink + empty input
# -------------------------------------------------------------------------------------


@pytest.mark.parametrize("cls,essence,name", CODECS)
def test_round_trip_and_shrinks(cls, essence, name):
    codec = cls()
    payload = b"yggdryl compresses bytes " * 4096  # highly compressible
    packed = codec.compress(payload)
    assert isinstance(packed, bytes)
    assert len(packed) < len(payload)  # a repetitive payload shrinks
    assert codec.decompress(packed) == payload


@pytest.mark.parametrize("cls", CLASSES)
def test_empty_input_round_trips(cls):
    codec = cls()
    packed = codec.compress(b"")
    assert isinstance(packed, bytes)
    assert codec.decompress(packed) == b""


@pytest.mark.parametrize("cls", CLASSES)
def test_bytearray_input_is_accepted(cls):
    codec = cls()
    packed = codec.compress(bytearray(b"payload " * 100))
    assert codec.decompress(packed) == b"payload " * 100


# -------------------------------------------------------------------------------------
# level kwarg
# -------------------------------------------------------------------------------------


def test_level_kwarg_round_trips_across_levels():
    payload = b"the quick brown fox jumps over the lazy dog " * 1000
    # gzip / zlib / xz share the 0..9 level range; zstd uses 1..22. Decompression never
    # needs the level, so a default codec reads any level's stream back.
    for fast, small in [(Gzip(level=1), Gzip(level=9)), (Zlib(level=1), Zlib(level=9)),
                        (Lzma(level=0), Lzma(level=9)), (Zstd(level=1), Zstd(level=19))]:
        assert type(fast)().decompress(fast.compress(payload)) == payload
        assert type(small)().decompress(small.compress(payload)) == payload


# -------------------------------------------------------------------------------------
# Corrupt input -> guided ValueError
# -------------------------------------------------------------------------------------


@pytest.mark.parametrize("cls", CLASSES)
def test_corrupt_input_is_valueerror(cls):
    with pytest.raises(ValueError, match="cannot decompress"):
        cls().decompress(b"not a valid compressed stream at all, just plain text")


# -------------------------------------------------------------------------------------
# codec_for: from a mime essence string
# -------------------------------------------------------------------------------------


def test_codec_for_from_essence():
    assert codec_for("application/gzip").name == "gzip"
    assert codec_for("application/zlib").name == "zlib"
    assert codec_for("application/zstd").name == "zstd"
    assert codec_for("application/x-xz").name == "xz"
    assert codec_for("application/x-lzma").name == "xz"  # the lzma-alone essence maps to Lzma
    # A non-compression (or unknown) essence resolves to None.
    assert codec_for("application/json") is None
    assert codec_for("text/plain") is None
    assert codec_for("image/png") is None


def test_codec_for_essence_round_trips():
    codec = codec_for("application/zstd")
    payload = b"resolve me by essence " * 500
    assert codec.decompress(codec.compress(payload)) == payload


# -------------------------------------------------------------------------------------
# codec_for: from a MimeType
# -------------------------------------------------------------------------------------


def test_codec_for_from_mimetype():
    gz = MimeType.from_extension("gz")
    assert gz.is_compression()
    codec = codec_for(gz)
    assert codec.name == "gzip"
    payload = b"resolve me by MimeType " * 500
    assert codec.decompress(codec.compress(payload)) == payload

    # A non-compression MimeType resolves to None.
    assert codec_for(MimeType.from_extension("json")) is None
    assert codec_for(MimeType.octet_stream()) is None


def test_codec_for_rejects_other_types():
    with pytest.raises(TypeError):
        codec_for(123)
    with pytest.raises(TypeError):
        codec_for(None)


# -------------------------------------------------------------------------------------
# codec_for: cached module-level singletons (resolve shared instances once)
# -------------------------------------------------------------------------------------


def test_default_codecs_are_cached_singletons():
    # Each default codec resolves to one shared process-wide instance, never a new object
    # per call (the "resolve shared instances once" rule).
    for essence in ("application/gzip", "application/zlib", "application/zstd", "application/x-xz"):
        assert codec_for(essence) is codec_for(essence)

    # A source's compression() accessor shares the very same singleton as the module resolver.
    from yggdryl.headers import Headers
    from yggdryl.memory import Heap

    h = Heap()
    h.set_headers(Headers().with_("Content-Type", "application/gzip"))
    assert h.compression() is codec_for("application/gzip")
