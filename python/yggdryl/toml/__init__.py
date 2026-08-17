"""Byte-first TOML encoding backed by the Yggdryl Rust codec."""

from __future__ import annotations

from typing import Any, Literal, TypeVar

from .. import _codec

_T = TypeVar("_T")
_ErrorPolicy = Literal["raise", "default"]
Buffer = _codec.Buffer
Destination = _codec.Destination
Readable = _codec.Readable
Source = _codec.Source
Writable = _codec.Writable


def dumps(value: object) -> bytes:
    """Encode one value directly to UTF-8 TOML bytes."""

    return _codec.dumps(value, format="toml")


def dump(value: object, destination: Destination) -> None:
    """Encode one value to a path or caller-owned file object."""

    _codec.dump(value, destination, format="toml")


def loads(
    source: Source,
    *,
    cls: type[_T] | None = None,
    safe: bool = True,
    errors: _ErrorPolicy = "raise",
) -> _T | Any:
    """Decode TOML bytes, content text, a path, or a readable file object."""

    return _codec.loads(
        source,
        format="toml",
        cls=cls,
        safe=safe,
        errors=errors,
    )


load = loads

__all__ = [
    "Buffer",
    "Destination",
    "Readable",
    "Source",
    "Writable",
    "dump",
    "dumps",
    "load",
    "loads",
]
