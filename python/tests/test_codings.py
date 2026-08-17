"""The byte codings agree with the standard library's wire formats."""

from __future__ import annotations

import gzip as std_gzip
import zlib as std_zlib

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
