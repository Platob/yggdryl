from ._native import (
    Digest as Digest,
    Xxh3_64 as Xxh3_64,
    Xxh3_128 as Xxh3_128,
    Xxh32 as Xxh32,
    Xxh64 as Xxh64,
    xxh3_64 as xxh3_64,
    xxh3_128 as xxh3_128,
    xxh32 as xxh32,
    xxh64 as xxh64,
)

SECRET_MINIMUM_LENGTH: int

def digest(data: str | bytes | bytearray | memoryview, algorithm: str) -> Digest: ...

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
