from __future__ import annotations

from collections.abc import Mapping
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
    placeholders: Mapping[str, Any] | None = None,
    environment: bool = False,
) -> _T: ...
@overload
def loads(
    source: Source,
    *,
    cls: None = None,
    safe: bool = True,
    errors: _ErrorPolicy = "raise",
    placeholders: Mapping[str, Any] | None = None,
    environment: bool = False,
) -> Any: ...

load = loads

__all__: list[str]
