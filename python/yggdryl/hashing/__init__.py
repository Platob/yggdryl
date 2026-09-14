"""Byte/value digests and the digests coupled with an instant.

:mod:`~yggdryl.hashing.xxhash` answers XXH32, XXH64, XXH3-64, and XXH3-128
over bytes, values, handles, and Arrow data; :mod:`~yggdryl.hashing.txhash`
lays a UTC instant in front of one of those digests, in one sortable value.
"""

from . import txhash, xxhash

__all__ = [
    "txhash",
    "xxhash",
]
