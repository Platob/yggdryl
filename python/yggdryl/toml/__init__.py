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


def dumps(
    value: object,
    *,
    indent: int | Literal["\t"] | None | object = _codec._DEFAULT_INDENT,
) -> bytes:
    """Encode one value directly to UTF-8 TOML bytes."""

    return _codec.dumps(value, format="toml", indent=indent)


def dump(
    value: object,
    destination: Destination | None = None,
    *,
    utf8: bool = False,
    indent: int | Literal["\t"] | None | object = _codec._DEFAULT_INDENT,
) -> bytes | str | None:
    """Return bytes/UTF-8, or write directly when given a destination."""

    return _codec.dump(
        value, destination, format="toml", utf8=utf8, indent=indent
    )


def loads(
    source: Source,
    *,
    cls: type[_T] | None = None,
    field: object | None = None,
    safe: bool = True,
    errors: _ErrorPolicy = "raise",
    placeholders: Mapping[str, Any] | None = None,
    environment: bool = False,
    max_depth: int | None = None,
    max_input_bytes: int | None = None,
    max_nodes: int | None = None,
    max_documents: int | None = None,
) -> _T | Any:
    """Decode TOML bytes, string content, a PathLike, or a readable object.

    A ``str`` is always document content; use ``pathlib.Path`` or another
    ``os.PathLike`` to declare a source location.

    ``cls=Value`` returns the exact core value without natural-type lowering.

    `placeholders` turns Jinja-style `{{ NAME }}` substitution on, resolving
    from the mapping it is given; `environment` additionally consults the
    process environment, which is never read unless asked for. Both default to
    off, and the mapping wins over the environment.
    """

    return _codec.loads(
        source,
        format="toml",
        cls=cls,
        field=field,
        safe=safe,
        errors=errors,
        placeholders=placeholders,
        environment=environment,
        max_depth=max_depth,
        max_input_bytes=max_input_bytes,
        max_nodes=max_nodes,
        max_documents=max_documents,
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
