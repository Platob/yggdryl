"""Fixed-width ASCII field factories."""

from __future__ import annotations

from typing import TYPE_CHECKING, Literal, TypeAlias, cast

from .._native import DataType, Field
from ._common import MetadataInput, new_field, simple_dtype
from ._typing import TypedField

if TYPE_CHECKING:
    Ascii32Field: TypeAlias = TypedField[Literal["ascii32"], str]
    Ascii64Field: TypeAlias = TypedField[Literal["ascii64"], str]
    Ascii128Field: TypeAlias = TypedField[Literal["ascii128"], str]
    AsciiField: TypeAlias = Ascii32Field | Ascii64Field | Ascii128Field
else:
    Ascii32Field = Ascii64Field = Ascii128Field = AsciiField = Field

_ASCII32 = simple_dtype("ascii32")
_ASCII64 = simple_dtype("ascii64")
_ASCII128 = simple_dtype("ascii128")


def ascii32(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> Ascii32Field:
    return new_field(Ascii32Field, name, _ASCII32, nullable, metadata)


def ascii64(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> Ascii64Field:
    return new_field(Ascii64Field, name, _ASCII64, nullable, metadata)


def ascii128(
    name: str, *, nullable: bool = True, metadata: MetadataInput = None
) -> Ascii128Field:
    return new_field(Ascii128Field, name, _ASCII128, nullable, metadata)


def ascii(
    name: str,
    width: int,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> AsciiField:
    """Select the ASCII width holding ``width`` bytes through ``DataType.ascii``."""

    return cast(
        AsciiField,
        new_field(Field, name, DataType.ascii(width), nullable, metadata),
    )


__all__ = [
    "Ascii32Field",
    "Ascii64Field",
    "Ascii128Field",
    "AsciiField",
    "ascii",
    "ascii32",
    "ascii64",
    "ascii128",
]
