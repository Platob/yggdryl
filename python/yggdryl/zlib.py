"""The zlib content coding over bytes, decoded and encoded by the Rust core.

The wire format matches the standard library's, so either side reads the
other's output; what differs is the engine underneath.
"""

from __future__ import annotations

from ._native import zlib_dumps, zlib_loads

__all__ = ["dumps", "loads"]

loads = zlib_loads
dumps = zlib_dumps
