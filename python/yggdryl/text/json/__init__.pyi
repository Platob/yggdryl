from __future__ import annotations

from collections.abc import Iterable, Iterator
from typing import Any, Literal, TypeVar, overload

from .._codec import (
    Buffer as Buffer,
    Destination as Destination,
    Readable as Readable,
    Source as Source,
    Writable as Writable,
)

_T = TypeVar("_T")
_ErrorPolicy = Literal["raise", "default"]
_Indent = int | Literal["\t"] | None

def dumps(value: object, *, indent: _Indent = ...) -> bytes: ...
@overload
def dump(
    value: object,
    destination: None = None,
    *,
    utf8: Literal[False] = False,
    indent: _Indent = ...,
) -> bytes: ...
@overload
def dump(
    value: object,
    destination: None = None,
    *,
    utf8: Literal[True],
    indent: _Indent = ...,
) -> str: ...
@overload
def dump(
    value: object,
    destination: Destination,
    *,
    utf8: bool = False,
    indent: _Indent = ...,
) -> None: ...

@overload
def loads(
    source: Source,
    *,
    cls: type[_T],
    field: object | None = None,
    safe: bool = True,
    errors: _ErrorPolicy = "raise",
    max_depth: int | None = None,
    max_input_bytes: int | None = None,
    max_nodes: int | None = None,
    max_documents: int | None = None,
) -> _T: ...
@overload
def loads(
    source: Source,
    *,
    cls: None = None,
    field: object | None = None,
    safe: bool = True,
    errors: _ErrorPolicy = "raise",
    max_depth: int | None = None,
    max_input_bytes: int | None = None,
    max_nodes: int | None = None,
    max_documents: int | None = None,
) -> Any: ...

def dumps_all(
    values: Iterable[object], *, indent: _Indent = ...
) -> bytes: ...
def dump_all(
    values: Iterable[object], destination: Destination, *, indent: _Indent = ...
) -> None: ...

@overload
def loads_all(
    source: Source,
    *,
    cls: type[_T],
    field: object | None = None,
    safe: bool = True,
    errors: _ErrorPolicy = "raise",
    max_depth: int | None = None,
    max_input_bytes: int | None = None,
    max_nodes: int | None = None,
    max_documents: int | None = None,
) -> Iterator[_T]: ...
@overload
def loads_all(
    source: Source,
    *,
    cls: None = None,
    field: object | None = None,
    safe: bool = True,
    errors: _ErrorPolicy = "raise",
    max_depth: int | None = None,
    max_input_bytes: int | None = None,
    max_nodes: int | None = None,
    max_documents: int | None = None,
) -> Iterator[Any]: ...

@overload
def load_all(
    source: Source,
    *,
    cls: type[_T],
    field: object | None = None,
    safe: bool = True,
    errors: _ErrorPolicy = "raise",
    max_depth: int | None = None,
    max_input_bytes: int | None = None,
    max_nodes: int | None = None,
    max_documents: int | None = None,
) -> Iterator[_T]: ...
@overload
def load_all(
    source: Source,
    *,
    cls: None = None,
    field: object | None = None,
    safe: bool = True,
    errors: _ErrorPolicy = "raise",
    max_depth: int | None = None,
    max_input_bytes: int | None = None,
    max_nodes: int | None = None,
    max_documents: int | None = None,
) -> Iterator[Any]: ...

__all__: list[str]
