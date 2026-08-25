"""Native ``Scalar`` and its dataclass projection decorator."""

from __future__ import annotations

import dataclasses as dc
import sys
from typing import Any, Callable, TypeVar, overload

from ._native import Scalar
from .fields._classes import _capture_context, _decorate_field_class

if sys.version_info >= (3, 11):
    from typing import dataclass_transform
else:  # pragma: no cover - executed on Python 3.10
    from typing_extensions import dataclass_transform

_T = TypeVar("_T")


@overload
def scalar(cls: type[_T], /, **options: Any) -> type[_T]: ...


@overload
def scalar(
    cls: None = None, /, **options: Any
) -> Callable[[type[_T]], type[_T]]: ...


@dataclass_transform(field_specifiers=(dc.field, dc.Field))
def scalar(
    cls: type[_T] | None = None, /, **options: Any
) -> type[_T] | Callable[[type[_T]], type[_T]]:
    """Create a dataclass with one cached native ``field()`` staticmethod."""

    def decorate(candidate: type[_T]) -> type[_T]:
        localns, token = _capture_context()
        return _decorate_field_class(candidate, options, localns, token)

    if cls is None:
        return decorate
    localns, token = _capture_context()
    return _decorate_field_class(cls, options, localns, token)


__all__ = ["Scalar", "scalar"]
