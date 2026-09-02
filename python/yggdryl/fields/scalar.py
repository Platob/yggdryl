"""Null and Boolean field factories."""

from __future__ import annotations

from typing import TYPE_CHECKING, Literal, TypeAlias

from .._native import Field
from ._common import MetadataInput, new_field, simple_dtype
from ._typing import TypedField

if TYPE_CHECKING:
    NullField: TypeAlias = TypedField[Literal["null"], None]
    BooleanField: TypeAlias = TypedField[Literal["boolean"], bool]
else:
    NullField = Field
    BooleanField = Field

_NULL = simple_dtype("null")
_BOOLEAN = simple_dtype("boolean")


def null(
    name: str,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> NullField:
    """Create a native null field."""

    return new_field(NullField, name, _NULL, nullable, metadata)


def boolean(
    name: str,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> BooleanField:
    """Create a native Boolean field."""

    return new_field(BooleanField, name, _BOOLEAN, nullable, metadata)


__all__ = ["BooleanField", "NullField", "boolean", "null"]
