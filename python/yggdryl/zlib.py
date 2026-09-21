"""The zlib content coding over bytes, decoded and encoded by the Rust core.

The wire format matches the standard library's, so either side reads the
other's output; what differs is the engine underneath.

The raw pair is the same DEFLATE stream without the RFC 1950 header and
checksum - what a zip member and an Avro ``deflate`` block carry, and what
``zlib.decompress(data, -zlib.MAX_WBITS)`` reads. It is a separate pair rather
than a flag because the two framings are two wire formats: bytes written by one
cannot be read by the other.
"""

from __future__ import annotations

from ._native import zlib_dumps, zlib_dumps_raw, zlib_loads, zlib_loads_raw

__all__ = ["dumps", "dumps_raw", "loads", "loads_raw"]

loads = zlib_loads
dumps = zlib_dumps
loads_raw = zlib_loads_raw
dumps_raw = zlib_dumps_raw
