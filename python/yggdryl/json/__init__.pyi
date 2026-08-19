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

def dumps(value: object) -> bytes: ...
def dump(value: object, destination: Destination) -> None: ...

@overload
def loads(
    source: Source,
    *,
    cls: type[_T],
    safe: bool = True,
    errors: _ErrorPolicy = "raise",
) -> _T: ...
@overload
def loads(
    source: Source,
    *,
    cls: None = None,
    safe: bool = True,
    errors: _ErrorPolicy = "raise",
) -> Any: ...

load = loads

def dumps_all(values: Iterable[object]) -> bytes: ...
def dump_all(values: Iterable[object], destination: Destination) -> None: ...

@overload
def loads_all(
    source: Source,
    *,
    cls: type[_T],
    safe: bool = True,
    errors: _ErrorPolicy = "raise",
) -> Iterator[_T]: ...
@overload
def loads_all(
    source: Source,
    *,
    cls: None = None,
    safe: bool = True,
    errors: _ErrorPolicy = "raise",
) -> Iterator[Any]: ...

@overload
def load_all(
    source: Source,
    *,
    cls: type[_T],
    safe: bool = True,
    errors: _ErrorPolicy = "raise",
) -> Iterator[_T]: ...
@overload
def load_all(
    source: Source,
    *,
    cls: None = None,
    safe: bool = True,
    errors: _ErrorPolicy = "raise",
) -> Iterator[Any]: ...

__all__: list[str]
