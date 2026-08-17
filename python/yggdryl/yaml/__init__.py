"""Byte-first YAML encoding backed by the Yggdryl Rust codec."""

from __future__ import annotations

from collections.abc import Iterable, Iterator
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
    """Encode one value directly to UTF-8 YAML bytes."""

    return _codec.dumps(value, format="yaml")


def dump(
    value: object,
    destination: Destination,
) -> None:
    """Encode one value to a path or caller-owned file object."""

    _codec.dump(value, destination, format="yaml")


def loads(
    source: Source,
    *,
    cls: type[_T] | None = None,
    safe: bool = True,
    errors: _ErrorPolicy = "raise",
) -> _T | Any:
    """Decode YAML bytes, content text, a path, or a readable file object."""

    return _codec.loads(
        source,
        format="yaml",
        cls=cls,
        safe=safe,
        errors=errors,
    )


load = loads


def dumps_all(values: Iterable[object]) -> bytes:
    """Encode a YAML document stream."""

    return _codec.dumps_all(values, format="yaml")


def dump_all(
    values: Iterable[object],
    destination: Destination,
) -> None:
    """Write a YAML document stream."""

    _codec.dump_all_stream(values, destination, format="yaml")


def loads_all(
    source: Source,
    *,
    cls: type[_T] | None = None,
    safe: bool = True,
    errors: _ErrorPolicy = "raise",
) -> Iterator[_T | Any]:
    """Iterate all documents in a YAML stream."""

    return _codec.loads_all(
        source,
        format="yaml",
        cls=cls,
        safe=safe,
        errors=errors,
    )


def load_all(
    source: Source,
    *,
    cls: type[_T] | None = None,
    safe: bool = True,
    errors: _ErrorPolicy = "raise",
) -> Iterator[_T | Any]:
    """Lazily iterate YAML documents from a path or stream."""

    return _codec.load_all_stream(
        source,
        format="yaml",
        cls=cls,
        safe=safe,
        errors=errors,
    )

__all__ = [
    "Buffer",
    "Destination",
    "Readable",
    "Source",
    "Writable",
    "dump",
    "dump_all",
    "dumps",
    "dumps_all",
    "load",
    "load_all",
    "loads",
    "loads_all",
]
