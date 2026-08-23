"""Inferred structured-text I/O over the shared native Value codec."""

from __future__ import annotations

import os
from collections.abc import Iterable, Iterator
from typing import Any, Literal, TypeVar

from . import _codec

_T = TypeVar("_T")
_ErrorPolicy = Literal["raise", "default"]
_Indent = int | Literal["\t"] | None


def _named_format(value: object, *, output: bool) -> str | None:
    name: object | None
    if isinstance(value, os.PathLike) or (output and isinstance(value, str)):
        name = os.fspath(value)
    else:
        name = getattr(value, "name", None)
    if isinstance(name, bytes):
        name = os.fsdecode(name)
    if not isinstance(name, str):
        return None
    try:
        if output:
            return _codec.infer_destination_format(value)  # type: ignore[arg-type]
        return _codec.infer_format(value)  # type: ignore[arg-type]
    except ValueError:
        return None


def _selected_format(value: object | None, format: str | None, *, output: bool) -> str | None:
    explicit = _codec._normalize_format(format) if format is not None else None
    inferred = _named_format(value, output=output) if value is not None else None
    if explicit is not None and inferred is not None and explicit != inferred:
        direction = "destination" if output else "source"
        raise ValueError(
            f"explicit format {explicit!r} contradicts {direction} format {inferred!r}"
        )
    return explicit or inferred


def from_io(
    source: _codec.Source,
    *,
    format: str | None = None,
    cls: type[_T] | None = None,
    field: object | None = None,
    safe: bool = True,
    errors: _ErrorPolicy = "raise",
    max_depth: int | None = None,
    max_input_bytes: int | None = None,
    max_nodes: int | None = None,
    max_documents: int | None = None,
) -> _T | Any | Iterator[_T | Any]:
    """Decode content, a declared path, or a reader with format inference."""

    selected = _selected_format(source, format, output=False)
    options = dict(
        cls=cls,
        field=field,
        safe=safe,
        errors=errors,
        max_depth=max_depth,
        max_input_bytes=max_input_bytes,
        max_nodes=max_nodes,
        max_documents=max_documents,
    )
    if selected is None:
        return _codec._decode_inferred(source, **options)
    if selected == "json_lines":
        return _codec.loads_all(source, format=selected, **options)
    return _codec.loads(source, format=selected, **options)


def from_stream(
    source: _codec.Source,
    *,
    format: str | None = None,
    cls: type[_T] | None = None,
    field: object | None = None,
    safe: bool = True,
    errors: _ErrorPolicy = "raise",
    max_depth: int | None = None,
    max_input_bytes: int | None = None,
    max_nodes: int | None = None,
    max_documents: int | None = None,
) -> _T | Any | Iterator[_T | Any]:
    """Decode a stream; multi-document formats stay lazy."""

    selected = _selected_format(source, format, output=False)
    if selected is None:
        return from_io(
            source,
            cls=cls,
            field=field,
            safe=safe,
            errors=errors,
            max_depth=max_depth,
            max_input_bytes=max_input_bytes,
            max_nodes=max_nodes,
            max_documents=max_documents,
        )
    options = dict(
        cls=cls,
        field=field,
        safe=safe,
        errors=errors,
        max_depth=max_depth,
        max_input_bytes=max_input_bytes,
        max_nodes=max_nodes,
        max_documents=max_documents,
    )
    if selected in ("json_lines", "yaml"):
        return _codec.load_all_stream(source, format=selected, **options)
    return _codec.loads(source, format=selected, **options)


def into_io(
    value: object,
    destination: _codec.Destination | None = None,
    *,
    format: str | None = None,
    utf8: bool = False,
    indent: _Indent | object = _codec._DEFAULT_INDENT,
) -> bytes | str | None:
    """Encode to returned bytes/UTF-8 or directly to a destination."""

    selected = _selected_format(destination, format, output=True) or "json"
    if selected == "json_lines":
        if isinstance(value, (str, bytes, bytearray, memoryview)) or not isinstance(
            value, Iterable
        ):
            raise TypeError("json_lines encoding requires an iterable of values")
        if destination is None:
            encoded = _codec.dumps_all(value, format=selected, indent=indent)
            return encoded.decode("utf-8") if utf8 else encoded
        _codec.dump_all_stream(value, destination, format=selected, indent=indent)
        return None
    return _codec.dump(
        value,
        destination,
        format=selected,
        utf8=utf8,
        indent=indent,
    )


def into_stream(
    value: object,
    destination: _codec.Destination,
    *,
    format: str | None = None,
    indent: _Indent | object = _codec._DEFAULT_INDENT,
) -> None:
    """Encode directly to a caller-owned writer or declared path."""

    into_io(value, destination, format=format, indent=indent)


__all__ = ["from_io", "from_stream", "into_io", "into_stream"]
