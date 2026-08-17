"""Shared byte-oriented helpers for the public JSON, TOML, and YAML modules."""

from __future__ import annotations

import dataclasses
import io
import os
import re
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
    """Infer a codec from an explicit name, path suffix, or document prefix."""

    if format is not None:
        return _normalize_format(format)

    name: object | None = None
    if isinstance(source, os.PathLike):
        name = os.fspath(source)
    elif isinstance(source, str) and _is_file(source):
        name = source
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
    elif isinstance(source, str) and _is_file(source):
        name = source
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
        return not _is_file(source)
    if isinstance(source, (bytes, bytearray, memoryview)):
        return True
    name = getattr(source, "name", None)
    return not isinstance(name, (str, bytes))


def _decode_inferred(
    source: Source,
    *,
    cls: type[_T] | None = None,
    safe: bool = True,
    errors: ErrorPolicy = "raise",
) -> _T | Any:
    """Infer and decode retained content through one native parse."""

    _check_decode_options(cls, safe, errors)
    if isinstance(source, str):
        decoded = _native._codec_decode_inferred_text(source)
    elif isinstance(source, (bytes, bytearray, memoryview)):
        decoded = _native._codec_decode_inferred(source)
    else:
        reader = getattr(source, "read", None)
        if reader is None:
            raise TypeError(
                "source must be bytes-like, str, PathLike, or a readable file object"
            )
        decoded = _native._codec_decode_inferred(_read_bounded(reader))
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
    """Read bytes without treating a non-existent string path as a path."""

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
        if _is_file(source):
            with open(source, "rb") as stream:
                return _read_bounded(stream.read)
        return _check_input_size(source.encode("utf-8"))
    reader = getattr(source, "read", None)
    if reader is None:
        raise TypeError(
            "source must be bytes-like, str, PathLike, or a readable file object"
        )
    return _read_bounded(reader)


def _read_bounded(reader: Any) -> bytes:
    chunks: list[bytes] = []
    total = 0
    while True:
        try:
            value = reader(_MAX_STREAM_BYTES - total + 1)
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
        if total > _MAX_STREAM_BYTES:
            raise ValueError(f"codec input exceeds {_MAX_STREAM_BYTES} bytes")
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


def dumps(value: object, *, format: str) -> bytes:
    return _native._codec_encode(value, format)


def dump(value: object, destination: Destination, *, format: str) -> None:
    """Encode directly to a path or caller-owned text/binary writer."""

    if isinstance(destination, (str, os.PathLike)):
        _native._codec_encode_path(value, destination, format)
        return
    if getattr(destination, "write", None) is None:
        raise TypeError("destination must be a path or writable file object")
    _native._codec_encode_writer(value, destination, format)


def loads(
    source: Source,
    *,
    format: str,
    cls: type[_T] | None = None,
    safe: bool = True,
    errors: ErrorPolicy = "raise",
) -> _T | Any:
    _check_decode_options(cls, safe, errors)
    decoded = _decode_source(source, format)
    return _materialize_decoded(decoded, cls, safe=safe, errors=errors)


def _decode_source(source: Source, format: str) -> Any:
    if isinstance(source, (bytes, bytearray, memoryview)):
        return _native._codec_decode(source, format)
    if isinstance(source, os.PathLike):
        with open(source, "rb") as stream:
            return _native._codec_decode_reader(stream, format)
    if isinstance(source, str):
        if _is_file(source):
            with open(source, "rb") as stream:
                return _native._codec_decode_reader(stream, format)
        return _native._codec_decode_text(source, format)
    if getattr(source, "read", None) is None:
        raise TypeError(
            "source must be bytes-like, str, PathLike, or a readable file object"
        )
    return _native._codec_decode_reader(source, format)


def dumps_all(
    values: Iterable[object],
    *,
    format: str,
) -> bytes:
    selected = _normalize_format(format)
    if selected == "toml":
        raise ValueError("TOML supports exactly one document; use dumps()")
    return _native._codec_encode_all(values, selected)


def loads_all(
    source: Source,
    *,
    format: str,
    cls: type[_T] | None = None,
    safe: bool = True,
    errors: ErrorPolicy = "raise",
) -> Iterator[_T | Any]:
    selected = _normalize_format(format)
    if selected == "toml":
        raise ValueError("TOML supports exactly one document; use loads()")
    _check_decode_options(cls, safe, errors)
    decoded = _decode_all_source(source, selected)

    def materialize() -> Iterator[_T | Any]:
        for value in decoded:
            yield _materialize_decoded(value, cls, safe=safe, errors=errors)

    return materialize()


def _decode_all_source(source: Source, format: str) -> list[Any]:
    if isinstance(source, (bytes, bytearray, memoryview)):
        return _native._codec_decode_all(source, format)
    if isinstance(source, os.PathLike):
        with open(source, "rb") as stream:
            return _native._codec_decode_all_reader(stream, format)
    if isinstance(source, str):
        if _is_file(source):
            with open(source, "rb") as stream:
                return _native._codec_decode_all_reader(stream, format)
        return _native._codec_decode_all_text(source, format)
    if getattr(source, "read", None) is None:
        raise TypeError(
            "source must be bytes-like, str, PathLike, or a readable file object"
        )
    return _native._codec_decode_all_reader(source, format)


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
    if dataclasses.is_dataclass(cls):
        if not isinstance(decoded, Mapping):
            raise TypeError(
                f"decoded data for {cls.__module__}.{cls.__qualname__} must be a mapping"
            )
        from .records import from_dict

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
    safe: bool = True,
    errors: ErrorPolicy = "raise",
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
            safe=safe,
            errors=errors,
        )

    def values() -> Iterator[_T | Any]:
        if isinstance(source, (str, os.PathLike)):
            with open(source, "rb") as stream:
                yield from _decode_framed(
                    stream, selected, cls=cls, safe=safe, errors=errors
                )
        else:
            yield from _decode_framed(
                source, selected, cls=cls, safe=safe, errors=errors
            )

    return values()


def dump_all_stream(
    values: Iterable[object], destination: Destination, *, format: str
) -> None:
    """Encode and write one stream item at a time."""

    selected = _normalize_format(format)
    if selected == "toml":
        raise ValueError("TOML supports exactly one document; use dump()")

    if isinstance(destination, (str, os.PathLike)):
        with open(destination, "wb") as stream:
            _native._codec_encode_all_writer(values, stream, selected)
        return
    if getattr(destination, "write", None) is None:
        raise TypeError("destination must be a path or writable file object")
    _native._codec_encode_all_writer(values, destination, selected)


def _is_content_source(source: object) -> bool:
    return isinstance(source, (bytes, bytearray, memoryview)) or (
        isinstance(source, str) and not _is_file(source)
    )


def _decode_framed(
    stream: object,
    format: Format,
    *,
    cls: type[_T] | None,
    safe: bool,
    errors: ErrorPolicy,
) -> Iterator[_T | Any]:
    _check_decode_options(cls, safe, errors)
    reader = (
        getattr(stream, "read", None)
        if format == "yaml"
        else getattr(stream, "readline", None)
    )
    if format == "yaml" and not callable(reader):
        reader = getattr(stream, "readline", None)
    if not callable(reader):
        decoded_values = _native._codec_decode_all(
            read_bytes(cast(Source, stream)), format
        )
        for value in decoded_values:
            yield _materialize_decoded(value, cls, safe=safe, errors=errors)
        return
    chunks: Iterable[tuple[int, bytes]]
    if format == "yaml":
        chunks = _yaml_documents(reader)
    else:
        chunks = _json_lines(reader)
    for index, (source_offset, chunk) in enumerate(chunks, 1):
        if not chunk:
            break
        parse_format = "yaml" if format == "yaml" else "json"
        try:
            decoded_value = _native._codec_decode(chunk, parse_format)
            yield _materialize_decoded(
                decoded_value, cls, safe=safe, errors=errors
            )
        except (TypeError, ValueError, OverflowError) as error:
            label = (
                "YAML document"
                if format == "yaml"
                else "JSON Lines document"
            )
            message = _offset_stream_error(str(error), source_offset)
            raise type(error)(f"{label} {index}: {message}") from error


_BYTE_POSITION = re.compile(r"\bat byte (\d+)")


def _offset_stream_error(message: str, source_offset: int) -> str:
    """Map a framed parser byte position back to the original stream."""

    match = _BYTE_POSITION.search(message)
    if match is None:
        return message
    local_offset = int(match.group(1))
    return (
        message[: match.start()]
        + f"at byte {source_offset + local_offset} (document byte {local_offset})"
        + message[match.end() :]
    )


def _yaml_documents(reader: Any) -> Iterator[tuple[int, bytes]]:
    current = bytearray()
    current_start = 0
    has_data = False
    document_started = False
    explicit_start = False
    total = 0
    documents = 0
    for line in _yaml_lines(reader):
        line_start = total
        total += len(line)
        start_marker = _is_yaml_start(line)
        end_marker = _is_yaml_end(line)
        if start_marker and (has_data or explicit_start):
            documents += 1
            if documents > _MAX_STREAM_DOCUMENTS:
                raise ValueError(
                    f"YAML stream exceeds {_MAX_STREAM_DOCUMENTS} documents"
                )
            # Parsing the frame at artificial EOF changes empty clip/keep
            # block scalars (`|`, `>`, `|+`, `>+`). Supply the same document
            # boundary context as the following source marker without adding
            # those synthetic bytes to source offsets.
            current.extend(b"...")
            framed = bytes(current)
            del current[-3:]
            yield current_start, framed
            current.clear()
            current_start = line_start
            has_data = False
            document_started = False
            explicit_start = False
        current.extend(line)
        stripped = line.strip(b" \t\r\n")
        if start_marker or stripped.startswith(b"%"):
            document_started = True
        if start_marker:
            explicit_start = True
        if (
            stripped
            and not stripped.startswith((b"#", b"%"))
            and not start_marker
            and not end_marker
        ):
            has_data = True
            document_started = True
        if end_marker and document_started:
            documents += 1
            if documents > _MAX_STREAM_DOCUMENTS:
                raise ValueError(
                    f"YAML stream exceeds {_MAX_STREAM_DOCUMENTS} documents"
                )
            yield current_start, bytes(current)
            current.clear()
            current_start = total
            has_data = False
            document_started = False
            explicit_start = False
    if current and document_started:
        documents += 1
        if documents > _MAX_STREAM_DOCUMENTS:
            raise ValueError(
                f"YAML stream exceeds {_MAX_STREAM_DOCUMENTS} documents"
            )
        yield current_start, bytes(current)


def _yaml_lines(reader: Any) -> Iterator[bytes]:
    """Yield YAML lines for LF, CRLF, and lone-CR byte streams."""

    pending = bytearray()
    line_start = 0
    search_start = 0
    total = 0
    while True:
        carriage = pending.find(b"\r", search_start)
        newline = pending.find(b"\n", search_start)
        boundary = (
            newline
            if carriage < 0
            else carriage
            if newline < 0
            else min(carriage, newline)
        )
        if boundary >= 0:
            boundary += 1
            yield bytes(pending[line_start:boundary])
            line_start = boundary
            search_start = boundary
            continue

        if line_start:
            del pending[:line_start]
            line_start = 0
        search_start = len(pending)

        capacity = min(8 * 1024, _MAX_STREAM_BYTES - total + 1)
        try:
            value = reader(max(1, capacity))
        except TypeError as error:
            raise TypeError(
                "file read() must accept a finite size argument"
            ) from error
        chunk = _line_bytes(value)
        returned = len(value) if isinstance(value, str) else len(chunk)
        if returned > capacity:
            unit = "characters" if isinstance(value, str) else "data"
            raise OSError(
                f"file read() returned more {unit} than requested"
            )
        if not chunk:
            if pending:
                yield bytes(pending)
            return
        total += len(chunk)
        if total > _MAX_STREAM_BYTES:
            raise ValueError(
                f"YAML stream exceeds {_MAX_STREAM_BYTES} bytes at byte {total}"
            )
        pending.extend(chunk)


def _json_lines(reader: Any) -> Iterator[tuple[int, bytes]]:
    total = 0
    documents = 0
    while True:
        line_start = total
        line = _readline_bounded(reader, _MAX_STREAM_BYTES - total)
        if not line:
            return
        total += len(line)
        if total > _MAX_STREAM_BYTES:
            raise ValueError(
                f"JSON Lines stream exceeds {_MAX_STREAM_BYTES} bytes at byte {total}"
            )
        if not _is_json_whitespace(line):
            documents += 1
            if documents > _MAX_STREAM_DOCUMENTS:
                raise ValueError(
                    f"JSON Lines stream exceeds {_MAX_STREAM_DOCUMENTS} documents"
                )
            yield line_start, line


def _is_json_whitespace(value: bytes) -> bool:
    """Recognize only the four whitespace bytes permitted by JSON."""

    for byte in value:
        if byte not in (0x09, 0x0A, 0x0D, 0x20):
            return False
    return True


def _line_bytes(value: object) -> bytes:
    if not value:
        return b""
    if isinstance(value, str):
        return value.encode("utf-8")
    if isinstance(value, bytes):
        return value
    if isinstance(value, (bytearray, memoryview)):
        return bytes(value)
    raise TypeError("file readline() must return str or bytes-like data")


def _readline_bounded(reader: Any, remaining: int) -> bytes:
    """Read one logical line from readers that may return short fragments."""

    chunks: list[bytes] = []
    total = 0
    while True:
        try:
            value = reader(max(1, remaining - total + 1))
        except TypeError as error:
            raise TypeError(
                "file readline() must accept a finite size argument"
            ) from error
        chunk = _line_bytes(value)
        if not chunk:
            break
        chunks.append(chunk)
        total += len(chunk)
        if total > remaining or chunk.endswith(b"\n"):
            break
    return b"".join(chunks)


def _is_yaml_start(line: bytes) -> bool:
    return _is_yaml_marker(line, b"---")


def _is_yaml_end(line: bytes) -> bool:
    return _is_yaml_marker(line, b"...")


def _is_yaml_marker(line: bytes, marker: bytes) -> bool:
    if line[:1] in (b" ", b"\t"):
        return False
    stripped = line.rstrip(b"\r\n")
    if not stripped.startswith(marker):
        return False
    suffix = stripped[len(marker) :]
    return not suffix or suffix[:1] in (b" ", b"\t")


def _is_file(value: str) -> bool:
    try:
        return os.path.isfile(value)
    except (OSError, ValueError):
        return False


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
