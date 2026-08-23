"""Byte-first JSON encoding backed by the Yggdryl Rust codec."""

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


def dumps(
    value: object,
    *,
    indent: int | Literal["\t"] | None | object = _codec._DEFAULT_INDENT,
) -> bytes:
    """Encode one value directly to UTF-8 JSON bytes."""

    return _codec.dumps(value, format="json", indent=indent)


def dump(
    value: object,
    destination: Destination | None = None,
    *,
    utf8: bool = False,
    indent: int | Literal["\t"] | None | object = _codec._DEFAULT_INDENT,
) -> bytes | str | None:
    """Return bytes/UTF-8, or write directly when given a destination."""

    return _codec.dump(
        value, destination, format="json", utf8=utf8, indent=indent
    )


def loads(
    source: Source,
    *,
    cls: type[_T] | None = None,
    field: object | None = None,
    safe: bool = True,
    errors: _ErrorPolicy = "raise",
    max_depth: int | None = None,
    max_input_bytes: int | None = None,
    max_nodes: int | None = None,
    max_documents: int | None = None,
) -> _T | Any:
    """Decode JSON bytes, string content, a PathLike, or a readable object.

    A ``str`` is always document content; use ``pathlib.Path`` or another
    ``os.PathLike`` to declare a source location.

    ``cls=Value`` returns the exact core value without natural-type lowering.

    JSON is a data interchange format, so `{{ }}` placeholder substitution is
    a YAML and TOML feature only - a configuration document that wants
    templating is better written as one of those.
    """

    return _codec.loads(
        source,
        format="json",
        cls=cls,
        field=field,
        safe=safe,
        errors=errors,
        max_depth=max_depth,
        max_input_bytes=max_input_bytes,
        max_nodes=max_nodes,
        max_documents=max_documents,
    )


load = loads


def dumps_all(
    values: Iterable[object],
    *,
    indent: int | Literal["\t"] | None | object = _codec._DEFAULT_INDENT,
) -> bytes:
    """Encode values as newline-delimited JSON bytes."""

    return _codec.dumps_all(values, format="json_lines", indent=indent)


def dump_all(
    values: Iterable[object],
    destination: Destination,
    *,
    indent: int | Literal["\t"] | None | object = _codec._DEFAULT_INDENT,
) -> None:
    """Write values as newline-delimited JSON."""

    _codec.dump_all_stream(
        values, destination, format="json_lines", indent=indent
    )


def loads_all(
    source: Source,
    *,
    cls: type[_T] | None = None,
    field: object | None = None,
    safe: bool = True,
    errors: _ErrorPolicy = "raise",
    max_depth: int | None = None,
    max_input_bytes: int | None = None,
    max_nodes: int | None = None,
    max_documents: int | None = None,
) -> Iterator[_T | Any]:
    """Iterate JSON Lines; ``cls=Value`` yields exact core values."""

    return _codec.loads_all(
        source,
        format="json_lines",
        cls=cls,
        field=field,
        safe=safe,
        errors=errors,
        max_depth=max_depth,
        max_input_bytes=max_input_bytes,
        max_nodes=max_nodes,
        max_documents=max_documents,
    )


def load_all(
    source: Source,
    *,
    cls: type[_T] | None = None,
    field: object | None = None,
    safe: bool = True,
    errors: _ErrorPolicy = "raise",
    max_depth: int | None = None,
    max_input_bytes: int | None = None,
    max_nodes: int | None = None,
    max_documents: int | None = None,
) -> Iterator[_T | Any]:
    """Iterate JSON Lines lazily; ``cls=Value`` yields exact core values."""

    return _codec.load_all_stream(
        source,
        format="json_lines",
        cls=cls,
        field=field,
        safe=safe,
        errors=errors,
        max_depth=max_depth,
        max_input_bytes=max_input_bytes,
        max_nodes=max_nodes,
        max_documents=max_documents,
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
