"""XXH32, XXH64, XXH3-64, and XXH3-128 over bytes, values, and handles.

The one-shot functions answer a plain ``int`` at the algorithm's native width
and accept ``bytes``, ``bytearray``, ``memoryview``, any other buffer, or a
``str`` encoded as UTF-8. :class:`Digest` is the answer that carries its
algorithm with it, which is what keeps ``xxh64`` and ``xxh3-64`` - both 64 bits
wide - from being confused for one another.

xxHash is not a cryptographic hash: a digest detects accidental change, never
an adversary who chooses the input. It is also not Iceberg's ``bucket``
transform, which the specification pins to murmur3 x86_32.
"""

from __future__ import annotations

from ._native import (
    Digest,
    Xxh3_64,
    Xxh3_128,
    Xxh32,
    Xxh64,
    xxh3_64,
    xxh3_128,
    xxh32,
    xxh64,
    xxhash_digest,
    xxhash_secret_minimum_length,
)

#: The shortest custom secret XXH3 accepts, in bytes.
SECRET_MINIMUM_LENGTH: int = xxhash_secret_minimum_length()

digest = xxhash_digest

__all__ = [
    "SECRET_MINIMUM_LENGTH",
    "Digest",
    "Xxh3_64",
    "Xxh3_128",
    "Xxh32",
    "Xxh64",
    "digest",
    "xxh3_64",
    "xxh3_128",
    "xxh32",
    "xxh64",
]
