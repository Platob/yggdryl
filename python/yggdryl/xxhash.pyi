from ._native import (
    Digest as Digest,
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

__all__ = [
    "SECRET_MINIMUM_LENGTH",
    "Digest",
    "Xxh3",
    "Xxh128",
    "Xxh32",
    "Xxh64",
    "digest",
    "xxh3",
    "xxh128",
    "xxh32",
    "xxh64",
]
