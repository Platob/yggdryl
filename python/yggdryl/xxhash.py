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
    Digester,
    Xxh3,
    Xxh128,
    Xxh32,
    Xxh64,
    xxh3,
    xxh128,
    xxh32,
    xxh64,
    xxhash_bits,
    xxhash_column_digests,
    xxhash_digest,
    xxhash_is_secretable,
    xxhash_is_seedable,
    xxhash_row_digests,
    xxhash_secret_minimum_length,
    xxhash_width,
)

#: The shortest custom secret XXH3 accepts, in bytes.
SECRET_MINIMUM_LENGTH: int = xxhash_secret_minimum_length()

digest = xxhash_digest

#: Digest every row of one Arrow batch, framed as an ordered sequence.
row_digests = xxhash_row_digests

#: Digest every cell of one Arrow column, with no row framing around it.
column_digests = xxhash_column_digests

#: Whether an algorithm accepts a custom secret.
is_secretable = xxhash_is_secretable

#: Whether an algorithm accepts a seed.
is_seedable = xxhash_is_seedable

#: An algorithm's digest width, in bytes.
width = xxhash_width

#: An algorithm's digest width, in bits.
bits = xxhash_bits

__all__ = [
    "SECRET_MINIMUM_LENGTH",
    "Digest",
    "Digester",
    "Xxh3",
    "Xxh128",
    "Xxh32",
    "Xxh64",
    "bits",
    "column_digests",
    "digest",
    "is_secretable",
    "is_seedable",
    "row_digests",
    "width",
    "xxh3",
    "xxh128",
    "xxh32",
    "xxh64",
]
