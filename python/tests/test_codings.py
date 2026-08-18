"""The byte codings agree with the standard library's wire formats."""

from __future__ import annotations

import gzip as std_gzip
import importlib
import sys
import zlib as std_zlib

import pytest

import yggdryl
from yggdryl import gzip, zlib, zstd

PAYLOAD = b'{"id": 1, "venue": "XNAS"}\n' * 512


class TestCodings:
    def test_gzip_round_trips_and_reads_the_standard_library(self) -> None:
        assert gzip.loads(gzip.dumps(PAYLOAD)) == PAYLOAD
        assert gzip.loads(std_gzip.compress(PAYLOAD)) == PAYLOAD
        assert std_gzip.decompress(gzip.dumps(PAYLOAD, level=9)) == PAYLOAD

    def test_zlib_round_trips_and_reads_the_standard_library(self) -> None:
        assert zlib.loads(zlib.dumps(PAYLOAD)) == PAYLOAD
        assert zlib.loads(std_zlib.compress(PAYLOAD)) == PAYLOAD
        assert std_zlib.decompress(zlib.dumps(PAYLOAD, level=1)) == PAYLOAD

    def test_zstd_round_trips(self) -> None:
        assert zstd.loads(zstd.dumps(PAYLOAD)) == PAYLOAD
        assert zstd.loads(zstd.dumps(PAYLOAD, level=9)) == PAYLOAD


class TestRawDeflate:
    """Raw DEFLATE is its own pair, because it is its own wire format."""

    def test_the_raw_pair_round_trips(self) -> None:
        assert zlib.loads_raw(zlib.dumps_raw(PAYLOAD)) == PAYLOAD
        assert zlib.loads_raw(zlib.dumps_raw(PAYLOAD, level=1)) == PAYLOAD
        assert zlib.loads_raw(zlib.dumps_raw(PAYLOAD, 9)) == PAYLOAD

    def test_the_raw_pair_is_the_standard_library_negative_window_spelling(
        self,
    ) -> None:
        raw = zlib.dumps_raw(PAYLOAD)

        # ``-MAX_WBITS`` is how the standard library spells "no header, no
        # checksum", so a zip member and an Avro ``deflate`` block written here
        # are readable there and back.
        assert std_zlib.decompress(raw, -std_zlib.MAX_WBITS) == PAYLOAD

        encoder = std_zlib.compressobj(wbits=-std_zlib.MAX_WBITS)
        assert zlib.loads_raw(encoder.compress(PAYLOAD) + encoder.flush()) == PAYLOAD

    def test_a_raw_stream_carries_neither_the_header_nor_the_checksum(self) -> None:
        raw = zlib.dumps_raw(PAYLOAD)
        framed = zlib.dumps(PAYLOAD)

        # The RFC 1950 wrapper is two leading bytes and a four-byte Adler-32,
        # and the raw stream is the same DEFLATE with both removed - which is
        # the whole reason it cannot be a flag on the framed pair.
        assert len(framed) == len(raw) + 6
        assert framed[2:-4] == raw

    def test_raw_and_framed_reject_each_other_bytes(self) -> None:
        # Two framings are two formats: reading one as the other has to fail
        # rather than return a plausible prefix of the wrong answer.
        with pytest.raises(ValueError):
            zlib.loads(zlib.dumps_raw(PAYLOAD))
        with pytest.raises(ValueError):
            zlib.loads_raw(zlib.dumps(PAYLOAD))

    def test_a_gzip_value_is_not_a_raw_deflate_value(self) -> None:
        with pytest.raises(ValueError):
            zlib.loads_raw(gzip.dumps(PAYLOAD))


class TestModuleReach:
    """The submodules carry the standard library's names, and its spelling."""

    def test_a_bare_import_reaches_all_three_by_attribute(self) -> None:
        module = importlib.import_module("yggdryl")

        # ``import yggdryl`` alone has to be enough: a caller who never wrote
        # ``from yggdryl import gzip`` still reaches ``yggdryl.gzip``.
        assert module.gzip.loads(module.gzip.dumps(PAYLOAD)) == PAYLOAD
        assert module.zlib.loads(module.zlib.dumps(PAYLOAD)) == PAYLOAD
        assert module.zstd.loads(module.zstd.dumps(PAYLOAD)) == PAYLOAD

    def test_the_from_import_names_the_same_three_modules(self) -> None:
        assert yggdryl.gzip is gzip
        assert yggdryl.zlib is zlib
        assert yggdryl.zstd is zstd
        assert sys.modules["yggdryl.gzip"] is gzip
        assert sys.modules["yggdryl.zlib"] is zlib
        assert sys.modules["yggdryl.zstd"] is zstd

    def test_they_are_exported_rather_than_merely_imported(self) -> None:
        assert {"gzip", "zlib", "zstd"} <= set(yggdryl.__all__)
        assert set(zlib.__all__) == {"dumps", "dumps_raw", "loads", "loads_raw"}
        assert set(gzip.__all__) == {"dumps", "loads"}

    def test_the_shared_name_is_not_the_standard_library_module(self) -> None:
        # The name is deliberate and the engine underneath is not: a test that
        # passed because it reached ``gzip`` from the standard library would be
        # testing nothing at all.
        assert gzip is not std_gzip
        assert zlib is not std_zlib
        assert not hasattr(gzip, "compress")
        assert not hasattr(zlib, "decompressobj")
