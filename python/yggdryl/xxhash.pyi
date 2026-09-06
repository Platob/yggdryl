import pyarrow  # type: ignore[import-untyped]

from ._native import (
    Digest as Digest,
    Digester as Digester,
    FieldLike,
    Xxh3 as Xxh3,
    Xxh128 as Xxh128,
    Xxh32 as Xxh32,
    Xxh64 as Xxh64,
    xxh3 as xxh3,
    xxh128 as xxh128,
    xxh32 as xxh32,
    xxh64 as xxh64,
)

SECRET_MINIMUM_LENGTH: int

def digest(data: str | bytes | bytearray | memoryview, algorithm: str) -> Digest: ...
def row_digests(
    batch: pyarrow.RecordBatch, algorithm: str = "xxh3-64"
) -> pyarrow.Array: ...
def column_digests(
    array: pyarrow.Array, field: FieldLike, algorithm: str = "xxh3-64"
) -> pyarrow.Array: ...
def is_secretable(algorithm: str) -> bool: ...
def is_seedable(algorithm: str) -> bool: ...
def width(algorithm: str) -> int: ...
def bits(algorithm: str) -> int: ...

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
