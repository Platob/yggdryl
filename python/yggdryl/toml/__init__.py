"""Byte-first TOML encoding backed by the Yggdryl Rust codec."""

from __future__ import annotations

from collections.abc import Mapping
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
    placeholders: Mapping[str, Any] | None = None,
    environment: bool = False,
) -> _T | Any:
    """Decode TOML bytes, content text, a path, or a readable file object.

    `placeholders` turns Jinja-style `{{ NAME }}` substitution on, resolving
    from the mapping it is given; `environment` additionally consults the
    process environment, which is never read unless asked for. Both default to
    off, and the mapping wins over the environment.
    """

    return _codec.loads(
        source,
        format="toml",
        cls=cls,
        safe=safe,
        errors=errors,
        placeholders=placeholders,
        environment=environment,
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
