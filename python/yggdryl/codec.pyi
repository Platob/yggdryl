from __future__ import annotations

from collections.abc import Iterator
from typing import Any, Literal, TypeVar, overload

from ._codec import Destination, Source
from ._native import Scalar

_T = TypeVar("_T")
_ErrorPolicy = Literal["raise", "default"]
_Indent = int | Literal["\t"] | None

@overload
def from_io(
    source: Source,
    *,
    format: str | None = None,
    cls: type[_T],
    field: object | None = None,
    safe: bool = True,
    errors: _ErrorPolicy = "raise",
    max_depth: int | None = None,
    max_input_bytes: int | None = None,
    max_nodes: int | None = None,
    max_documents: int | None = None,
) -> _T | Iterator[_T]: ...
@overload
def from_io(
    source: Source,
    *,
    format: str | None = None,
    cls: None = None,
    field: object | None = None,
    safe: bool = True,
    errors: _ErrorPolicy = "raise",
    max_depth: int | None = None,
    max_input_bytes: int | None = None,
    max_nodes: int | None = None,
    max_documents: int | None = None,
) -> Any | Iterator[Any]: ...

@overload
def from_stream(
    source: Source,
    *,
    format: str | None = None,
    cls: type[_T],
    field: object | None = None,
    safe: bool = True,
    errors: _ErrorPolicy = "raise",
    max_depth: int | None = None,
    max_input_bytes: int | None = None,
    max_nodes: int | None = None,
    max_documents: int | None = None,
) -> _T | Iterator[_T]: ...
@overload
def from_stream(
    source: Source,
    *,
    format: str | None = None,
    cls: None = None,
    field: object | None = None,
    safe: bool = True,
    errors: _ErrorPolicy = "raise",
    max_depth: int | None = None,
    max_input_bytes: int | None = None,
    max_nodes: int | None = None,
    max_documents: int | None = None,
) -> Any | Iterator[Any]: ...

@overload
def into_io(
    value: object,
    destination: None = None,
    *,
    format: str | None = None,
    utf8: Literal[False] = False,
    indent: _Indent = ...,
) -> bytes: ...
@overload
def into_io(
    value: object,
    destination: None = None,
    *,
    format: str | None = None,
    utf8: Literal[True],
    indent: _Indent = ...,
) -> str: ...
@overload
def into_io(
    value: object,
    destination: Destination,
    *,
    format: str | None = None,
    utf8: bool = False,
    indent: _Indent = ...,
) -> None: ...

def into_stream(
    value: object,
    destination: Destination,
    *,
    format: str | None = None,
    indent: _Indent = ...,
) -> None: ...

__all__: list[str]
