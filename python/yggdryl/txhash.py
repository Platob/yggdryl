"""An instant coupled with an xxHash digest, in one sortable value.

A :class:`TxHash` is a unix count followed by a digest: the instant first,
big-endian, so ``bytes(value)`` sorts by time and then by content; the digest
after it, at its algorithm's exact width. The instant is always UTC and
counted in microseconds unless a resolution is named, and the digest is what
:mod:`yggdryl.xxhash` answers for the same bytes - this module defines no
second hash, only the coupling.

Every ``unix`` argument reads the same way: an ``int`` is the count already,
and a ``datetime``, a ``date``, timestamp text, or a native ``Scalar`` is read
through the core's one instant intake. A zoned ``datetime`` already counts from
the epoch, and a naive one is read as if it were UTC.
"""

from __future__ import annotations

from ._native import (
    TxHash,
    TxHasher,
    txh3,
    txh32,
    txh64,
    txh128,
    txhash_column_txhashes,
    txhash_compose,
    txhash_decompose,
    txhash_digest,
    txhash_dtype,
    txhash_restate_unix,
    txhash_row_txhashes,
    txhash_unix_array,
    txhash_unix_now,
    txhash_unix_of,
    txhash_width,
)

#: The resolution a unix count carries when a caller names none.
DEFAULT_UNIT: str = "us"

#: The bytes the instant takes at the front of every value.
UNIX_WIDTH: int = 8

#: Couple a microsecond instant with a complete value's digest.
digest = txhash_digest

#: Couple every row's digest with the instant beside it.
row_txhashes = txhash_row_txhashes

#: Couple every cell's digest with the instant beside it.
column_txhashes = txhash_column_txhashes

#: Couple an instant column with a digest column already computed.
compose = txhash_compose

#: Split a coupled column into its UTC timestamp column and its digest column.
decompose = txhash_decompose

#: Read a timestamp, date, or integer column as unix counts.
unix_array = txhash_unix_array

#: Read the system clock as a unix count.
unix_now = txhash_unix_now

#: Read any instant as a unix count.
unix_of = txhash_unix_of

#: Restate a count of one resolution as a count of another.
restate_unix = txhash_restate_unix

#: The width of a value coupling an instant with an algorithm's digest.
width = txhash_width

#: The datatype a column of coupled values is stored under.
dtype = txhash_dtype

__all__ = [
    "DEFAULT_UNIT",
    "UNIX_WIDTH",
    "TxHash",
    "TxHasher",
    "column_txhashes",
    "compose",
    "decompose",
    "digest",
    "dtype",
    "restate_unix",
    "row_txhashes",
    "txh3",
    "txh128",
    "txh32",
    "txh64",
    "unix_array",
    "unix_now",
    "unix_of",
    "width",
]
