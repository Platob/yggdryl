"""Shared byte-oriented helpers for the public JSON, TOML, and YAML modules."""

from __future__ import annotations

import dataclasses
import io
import os
from collections.abc import Callable, Iterable, Iterator, Mapping
from typing import Any, Literal, Protocol, TypeAlias, TypeVar, cast

from . import _native

Format = Literal["json", "json_lines", "toml", "yaml"]
CodecFormat: TypeAlias = Literal[
    "json",
    "json_lines",
    "json-lines",
    "jsonl",
    "ndjson",
    "toml",
    ".toml",
    "application/toml",
    "yaml",
    "yml",
]
ErrorPolicy = Literal["raise", "default"]
Buffer: TypeAlias = bytes | bytearray | memoryview


class BinaryReadable(Protocol):
    """Minimal binary reader accepted by the native codec adapter."""

    def read(self, size: int = -1, /) -> Buffer: ...


class TextReadable(Protocol):
    """Minimal text reader accepted by the native codec adapter."""

    def read(self, size: int = -1, /) -> str: ...


class BinaryWritable(Protocol):
    """Minimal binary writer accepted by the native codec adapter."""

    def write(self, value: bytes, /) -> int | None: ...


class TextWritable(Protocol):
    """Minimal text writer accepted by the native codec adapter."""

    def write(self, value: str, /) -> int | None: ...


Readable: TypeAlias = BinaryReadable | TextReadable
Writable: TypeAlias = BinaryWritable | TextWritable
Source: TypeAlias = (
    str
    | Buffer
    | os.PathLike[str]
    | os.PathLike[bytes]
    | Readable
)
Destination: TypeAlias = (
    str | os.PathLike[str] | os.PathLike[bytes] | Writable
)
_T = TypeVar("_T")
_MAX_STREAM_BYTES = 64 * 1024 * 1024
_MAX_STREAM_DOCUMENTS = 1_024
_DEFAULT_INDENT = object()


def _indent_code(indent: object) -> int:
    """Normalize one language-level indent request for core Formatting."""

    if indent is _DEFAULT_INDENT:
        return -2
    if indent is None:
        return -1
    if indent == "\t":
        return -3
    if isinstance(indent, bool) or not isinstance(indent, int):
        raise TypeError("indent must be None, a non-negative int, or '\\t'")
    if not 0 <= indent <= 255:
        raise ValueError("indent must be between 0 and 255")
    return indent


def _limit_values(
    max_depth: int | None,
    max_input_bytes: int | None,
    max_nodes: int | None,
    max_documents: int | None,
) -> tuple[int | None, int | None, int | None, int | None]:
    values = {
        "max_depth": max_depth,
        "max_input_bytes": max_input_bytes,
        "max_nodes": max_nodes,
        "max_documents": max_documents,
    }
    for name, value in values.items():
        if value is not None and (isinstance(value, bool) or not isinstance(value, int)):
            raise TypeError(f"{name} must be int or None")
        if value is not None and value < 0:
            raise ValueError(f"{name} must be non-negative")
    return max_depth, max_input_bytes, max_nodes, max_documents


def _normalize_format(format: str) -> Format:
    if not isinstance(format, str):
        raise TypeError("format must be 'json', 'json_lines', 'toml', or 'yaml'")
    try:
        return cast(Format, _native._codec_normalize_format(format))
    except ValueError as error:
        raise ValueError(
            "format must be 'json', 'json_lines', 'toml', or 'yaml'"
        ) from error


def _format_from_path(name: str, *, destination: bool = False) -> Format:
    """Delegate suffix matching to core while retaining public diagnostics."""

    try:
        return cast(Format, _native._codec_infer_path(name))
    except ValueError as error:
        context = "destination" if destination else "path"
        raise ValueError(
            f"cannot infer format from {context} {name!r}; pass format explicitly"
        ) from error


def infer_format(source: Source, format: str | None = None) -> Format:
    """Infer a codec from an explicit name, declared path, or document prefix."""

    if format is not None:
        return _normalize_format(format)

    name: object | None = None
    if isinstance(source, os.PathLike):
        name = os.fspath(source)
    elif hasattr(source, "name"):
        name = getattr(source, "name")

    if isinstance(name, bytes):
        name = os.fsdecode(name)
    if isinstance(name, str):
        return _format_from_path(name)

    if isinstance(source, str):
        return _normalize_format(_native._codec_infer_text(source))
    if isinstance(source, (bytes, bytearray, memoryview)):
        return _normalize_format(_native._codec_infer(source))
    raise ValueError("cannot infer stream format; pass format explicitly")


def prepare_source(
    source: Source, format: str | None = None
) -> tuple[Format, Source]:
    """Resolve a format while retaining every byte read from anonymous I/O."""

    if format is not None:
        return _normalize_format(format), source

    name: object | None = None
    if isinstance(source, os.PathLike):
        name = os.fspath(source)
    elif hasattr(source, "name"):
        name = getattr(source, "name")
    if isinstance(name, bytes):
        name = os.fsdecode(name)
    if isinstance(name, str):
        return infer_format(source), source

    if isinstance(source, str):
        return _normalize_format(_native._codec_infer_text(source)), source
    if isinstance(source, (bytes, bytearray, memoryview)):
        return _normalize_format(_native._codec_infer(source)), source

    reader = getattr(source, "read", None)
    if reader is None:
        raise TypeError(
            "source must be bytes-like, str, PathLike, or a readable file object"
        )
    content = _read_bounded(reader)
    return _normalize_format(_native._codec_infer(content)), content


def _source_needs_content_inference(source: Source) -> bool:
    """Return whether generic decoding must retain and inspect source content."""

    if isinstance(source, os.PathLike):
        return False
    if isinstance(source, str):
        return True
    if isinstance(source, (bytes, bytearray, memoryview)):
        return True
    name = getattr(source, "name", None)
    return not isinstance(name, (str, bytes))


def _decode_inferred(
    source: Source,
    *,
    cls: type[_T] | None = None,
    field: object | None = None,
    safe: bool = True,
    errors: ErrorPolicy = "raise",
    max_depth: int | None = None,
    max_input_bytes: int | None = None,
    max_nodes: int | None = None,
    max_documents: int | None = None,
) -> _T | Any:
    """Infer and decode retained content through one native parse."""

    _check_decode_options(cls, safe, errors)
    native_value = cls is _native.Value
    limits = _limit_values(max_depth, max_input_bytes, max_nodes, max_documents)
    if isinstance(source, os.PathLike):
        with open(source, "rb") as stream:
            decoded = _native._codec_decode_inferred(
                _read_bounded(stream.read, max_input_bytes),
                _decode_field(field),
                native_value,
                *limits,
            )
    elif isinstance(source, str):
        decoded = _native._codec_decode_inferred_text(
            source, _decode_field(field), native_value, *limits
        )
    elif isinstance(source, (bytes, bytearray, memoryview)):
        decoded = _native._codec_decode_inferred(
            source, _decode_field(field), native_value, *limits
        )
    else:
        reader = getattr(source, "read", None)
        if reader is None:
            raise TypeError(
                "source must be bytes-like, str, PathLike, or a readable file object"
            )
        decoded = _native._codec_decode_inferred(
            _read_bounded(reader, max_input_bytes),
            _decode_field(field),
            native_value,
            *limits,
        )
    return _materialize_decoded(decoded, cls, safe=safe, errors=errors)


def infer_destination_format(
    destination: Destination, format: str | None = None
) -> Format:
    """Infer an output codec from an explicit name or destination suffix."""

    if format is not None:
        return _normalize_format(format)
    name = (
        os.fspath(destination)
        if isinstance(destination, (str, os.PathLike))
        else getattr(destination, "name", None)
    )
    if isinstance(name, bytes):
        name = os.fsdecode(name)
    if not isinstance(name, str):
        raise ValueError("cannot infer destination format; pass format explicitly")
    return _format_from_path(name, destination=True)


def read_bytes(source: Source) -> bytes:
    """Read declared paths and readers; encode every string as content."""

    if isinstance(source, bytes):
        return _check_input_size(source)
    if isinstance(source, bytearray):
        return _check_input_size(bytes(source))
    if isinstance(source, memoryview):
        return _check_input_size(source.tobytes())
    if isinstance(source, os.PathLike):
        with open(source, "rb") as stream:
            return _read_bounded(stream.read)
    if isinstance(source, str):
        return _check_input_size(source.encode("utf-8"))
    reader = getattr(source, "read", None)
    if reader is None:
        raise TypeError(
            "source must be bytes-like, str, PathLike, or a readable file object"
        )
    return _read_bounded(reader)


def _read_bounded(reader: Any, maximum: int | None = None) -> bytes:
    maximum = _MAX_STREAM_BYTES if maximum is None else maximum
    chunks: list[bytes] = []
    total = 0
    while True:
        try:
            value = reader(maximum - total + 1)
        except TypeError as error:
            raise TypeError("file read() must accept a finite size argument") from error
        if isinstance(value, str):
            chunk = value.encode("utf-8")
        elif isinstance(value, bytes):
            chunk = value
        elif isinstance(value, (bytearray, memoryview)):
            chunk = bytes(value)
        else:
            raise TypeError("file read() must return str or bytes-like data")
        if not chunk:
            break
        total += len(chunk)
        if total > maximum:
            raise ValueError(f"codec input exceeds {maximum} bytes")
        chunks.append(chunk)
    return b"".join(chunks)


def _check_input_size(value: bytes) -> bytes:
    if len(value) > _MAX_STREAM_BYTES:
        raise ValueError(f"codec input exceeds {_MAX_STREAM_BYTES} bytes")
    return value


def write_bytes(destination: Destination, data: bytes) -> None:
    """Write encoded bytes to a path or caller-owned binary/text stream."""

    if isinstance(destination, (str, os.PathLike)):
        with open(destination, "wb") as stream:
            _write_all(stream.write, data)
        return
    writer = getattr(destination, "write", None)
    if writer is None:
        raise TypeError("destination must be a path or writable file object")
    if isinstance(destination, io.TextIOBase):
        _write_all(writer, data.decode("utf-8"))
        return
    try:
        _write_all(writer, data)
    except _InitialWriteTypeError:
        _write_all(writer, data.decode("utf-8"))


class _InitialWriteTypeError(TypeError):
    """Signal that a stream rejected bytes before accepting any data."""


def _write_all(writer: Any, data: bytes | str) -> None:
    offset = 0
    while offset < len(data):
        try:
            written = writer(data[offset:])
        except TypeError as error:
            if offset == 0:
                raise _InitialWriteTypeError from error
            raise
        if written is None:
            return
        if not isinstance(written, int) or isinstance(written, bool):
            raise TypeError("file write() must return an integer or None")
        if written <= 0 or written > len(data) - offset:
            raise OSError("file write() returned an invalid byte/character count")
        offset += written


def dumps(
    value: object,
    *,
    format: str,
    indent: int | str | None | object = _DEFAULT_INDENT,
) -> bytes:
    return _native._codec_encode(value, format, _indent_code(indent))


def dump(
    value: object,
    destination: Destination | None = None,
    *,
    format: str,
    utf8: bool = False,
    indent: int | str | None | object = _DEFAULT_INDENT,
) -> bytes | str | None:
    """Return encoded content, or write it when a destination is supplied."""

    if destination is None:
        encoded = dumps(value, format=format, indent=indent)
        return encoded.decode("utf-8") if utf8 else encoded

    if isinstance(destination, (str, os.PathLike)):
        _native._codec_encode_path(value, destination, format, _indent_code(indent))
        return None
    if getattr(destination, "write", None) is None:
        raise TypeError("destination must be a path or writable file object")
    _native._codec_encode_writer(value, destination, format, _indent_code(indent))
    return None


def loads(
    source: Source,
    *,
    format: str,
    cls: type[_T] | None = None,
    field: object | None = None,
    safe: bool = True,
    errors: ErrorPolicy = "raise",
    placeholders: Mapping[str, Any] | None = None,
    environment: bool = False,
    max_depth: int | None = None,
    max_input_bytes: int | None = None,
    max_nodes: int | None = None,
    max_documents: int | None = None,
) -> _T | Any:
    _check_decode_options(cls, safe, errors)
    decoded = _decode_source(
        source,
        format,
        placeholders=placeholders,
        environment=environment,
        field=_decode_field(field),
        native_value=cls is _native.Value,
        limits=_limit_values(max_depth, max_input_bytes, max_nodes, max_documents),
    )
    return _materialize_decoded(decoded, cls, safe=safe, errors=errors)


def _decode_source(
    source: Source,
    format: str,
    *,
    placeholders: Mapping[str, Any] | None = None,
    environment: bool = False,
    field: object | None = None,
    native_value: bool = False,
    limits: tuple[int | None, int | None, int | None, int | None] = (
        None,
        None,
        None,
        None,
    ),
) -> Any:
    # Both switches travel to the core untouched: `placeholders=None` with
    # `environment=False` is the plain parse, and nothing here decides for the
    # caller which of the two they meant.
    filling = (placeholders, environment)
    if isinstance(source, (bytes, bytearray, memoryview)):
        return _native._codec_decode(
            source, format, *filling, field, native_value, *limits
        )
    if isinstance(source, os.PathLike):
        with open(source, "rb") as stream:
            return _native._codec_decode_reader(
                stream, format, *filling, field, native_value, *limits
            )
    if isinstance(source, str):
        return _native._codec_decode_text(
            source, format, *filling, field, native_value, *limits
        )
    if getattr(source, "read", None) is None:
        raise TypeError(
            "source must be bytes-like, str, PathLike, or a readable file object"
        )
    return _native._codec_decode_reader(
        source, format, *filling, field, native_value, *limits
    )


def dumps_all(
    values: Iterable[object],
    *,
    format: str,
    indent: int | str | None | object = _DEFAULT_INDENT,
) -> bytes:
    selected = _normalize_format(format)
    if selected == "toml":
        raise ValueError("TOML supports exactly one document; use dumps()")
    return _native._codec_encode_all(values, selected, _indent_code(indent))


def loads_all(
    source: Source,
    *,
    format: str,
    cls: type[_T] | None = None,
    field: object | None = None,
    safe: bool = True,
    errors: ErrorPolicy = "raise",
    max_depth: int | None = None,
    max_input_bytes: int | None = None,
    max_nodes: int | None = None,
    max_documents: int | None = None,
) -> Iterator[_T | Any]:
    selected = _normalize_format(format)
    if selected == "toml":
        raise ValueError("TOML supports exactly one document; use loads()")
    _check_decode_options(cls, safe, errors)
    decoded = _decode_all_source(
        source,
        selected,
        _decode_field(field),
        native_value=cls is _native.Value,
        limits=_limit_values(max_depth, max_input_bytes, max_nodes, max_documents),
    )

    def materialize() -> Iterator[_T | Any]:
        for value in decoded:
            yield _materialize_decoded(value, cls, safe=safe, errors=errors)

    return materialize()


def _decode_all_source(
    source: Source,
    format: str,
    field: object | None,
    *,
    native_value: bool = False,
    limits: tuple[int | None, int | None, int | None, int | None] = (
        None,
        None,
        None,
        None,
    ),
) -> list[Any]:
    if isinstance(source, (bytes, bytearray, memoryview)):
        return _native._codec_decode_all(source, format, field, native_value, *limits)
    if isinstance(source, os.PathLike):
        with open(source, "rb") as stream:
            return _native._codec_decode_all_reader(
                stream, format, field, native_value, *limits
            )
    if isinstance(source, str):
        return _native._codec_decode_all_text(
            source, format, field, native_value, *limits
        )
    if getattr(source, "read", None) is None:
        raise TypeError(
            "source must be bytes-like, str, PathLike, or a readable file object"
        )
    return _native._codec_decode_all_reader(
        source, format, field, native_value, *limits
    )


def _decode_field(field: object | None) -> object | None:
    """Keep core field parsing explicit; ``cls`` retains wrapper casting."""

    return field


def _materialize_decoded(
    decoded: Any,
    cls: type[_T] | None,
    *,
    safe: bool,
    errors: ErrorPolicy,
) -> _T | Any:
    """Apply the one shared decoded-value-to-target conversion policy."""

    if cls is None:
        return decoded
    if cls is _native.Value:
        # The core already built this exact value. Returning it directly keeps
        # narrow floats, integers, temporal layouts, and geospatial bytes from
        # taking a lossy round trip through Python's natural types.
        return cast(_T, decoded)
    if dataclasses.is_dataclass(cls):
        if not isinstance(decoded, Mapping):
            raise TypeError(
                f"decoded data for {cls.__module__}.{cls.__qualname__} must be a mapping"
            )
        from .fields._classes import from_dict

        return from_dict(cls, decoded, safe=safe, errors=errors)
    return cast(Callable[[Any], _T], cls)(decoded)


def _check_decode_options(
    cls: type[Any] | None,
    safe: bool,
    errors: ErrorPolicy,
) -> None:
    if cls is not None and not isinstance(cls, type):
        raise TypeError("cls must be a type")
    if type(safe) is not bool:
        raise TypeError("safe must be bool")
    if errors not in ("raise", "default"):
        raise ValueError("errors must be 'raise' or 'default'")


def load_all_stream(
    source: Source,
    *,
    format: str,
    cls: type[_T] | None = None,
    field: object | None = None,
    safe: bool = True,
    errors: ErrorPolicy = "raise",
    max_depth: int | None = None,
    max_input_bytes: int | None = None,
    max_nodes: int | None = None,
    max_documents: int | None = None,
) -> Iterator[_T | Any]:
    """Lazily frame and decode documents from a path or readable stream."""

    selected = _normalize_format(format)
    if selected == "toml":
        raise ValueError("TOML supports exactly one document; use loads()")
    if _is_content_source(source):
        return loads_all(
            source,
            format=selected,
            cls=cls,
            field=field,
            safe=safe,
            errors=errors,
            max_depth=max_depth,
            max_input_bytes=max_input_bytes,
            max_nodes=max_nodes,
            max_documents=max_documents,
        )

    def values() -> Iterator[_T | Any]:
        if isinstance(source, (str, os.PathLike)):
            with open(source, "rb") as stream:
                yield from _decode_stream(
                    stream,
                    selected,
                    cls=cls,
                    field=field,
                    safe=safe,
                    errors=errors,
                    max_depth=max_depth,
                    max_input_bytes=max_input_bytes,
                    max_nodes=max_nodes,
                    max_documents=max_documents,
                )
        else:
            yield from _decode_stream(
                source,
                selected,
                cls=cls,
                field=field,
                safe=safe,
                errors=errors,
                max_depth=max_depth,
                max_input_bytes=max_input_bytes,
                max_nodes=max_nodes,
                max_documents=max_documents,
            )

    return values()


def dump_all_stream(
    values: Iterable[object],
    destination: Destination,
    *,
    format: str,
    indent: int | str | None | object = _DEFAULT_INDENT,
) -> None:
    """Encode and write one stream item at a time."""

    selected = _normalize_format(format)
    if selected == "toml":
        raise ValueError("TOML supports exactly one document; use dump()")

    if isinstance(destination, (str, os.PathLike)):
        with open(destination, "wb") as stream:
            _native._codec_encode_all_writer(
                values, stream, selected, _indent_code(indent)
            )
        return
    if getattr(destination, "write", None) is None:
        raise TypeError("destination must be a path or writable file object")
    _native._codec_encode_all_writer(
        values, destination, selected, _indent_code(indent)
    )


def _is_content_source(source: object) -> bool:
    return isinstance(source, (str, bytes, bytearray, memoryview))


def _decode_stream(
    stream: object,
    format: Format,
    *,
    cls: type[_T] | None,
    field: object | None,
    safe: bool,
    errors: ErrorPolicy,
    max_depth: int | None,
    max_input_bytes: int | None,
    max_nodes: int | None,
    max_documents: int | None,
) -> Iterator[_T | Any]:
    """Adapt Python reads and targets around the core's lazy decoder."""

    _check_decode_options(cls, safe, errors)
    # The native iterator still owns cumulative accounting across chunks and
    # documents; these are its explicit Python stream defaults.
    limits = _limit_values(
        max_depth,
        _MAX_STREAM_BYTES if max_input_bytes is None else max_input_bytes,
        max_nodes,
        _MAX_STREAM_DOCUMENTS if max_documents is None else max_documents,
    )
    for decoded_value in _native._codec_decode_iter(
        stream,
        format,
        *limits,
        _decode_field(field),
        cls is _native.Value,
    ):
        yield _materialize_decoded(
            decoded_value, cls, safe=safe, errors=errors
        )


__all__ = [
    "BinaryReadable",
    "BinaryWritable",
    "Buffer",
    "CodecFormat",
    "Destination",
    "Readable",
    "Source",
    "TextReadable",
    "TextWritable",
    "Writable",
    "Format",
    "ErrorPolicy",
    "dump",
    "dumps",
    "dumps_all",
    "infer_format",
    "infer_destination_format",
    "load_all_stream",
    "loads",
    "loads_all",
    "prepare_source",
    "read_bytes",
    "write_bytes",
    "dump_all_stream",
]
