"""The zstd content coding over bytes, decoded and encoded by the Rust core.

The wire format matches the standard library's, so either side reads the
other's output; what differs is the engine underneath.
"""

from __future__ import annotations

from ._native import zstd_dumps, zstd_loads

__all__ = ["dumps", "loads"]

loads = zstd_loads
dumps = zstd_dumps
