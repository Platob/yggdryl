"""Binary and UTF-8 field factories."""

from __future__ import annotations

from typing import TYPE_CHECKING, Literal, TypeAlias

from .._native import DataType, Field
from ._common import MetadataInput, new_field, simple_dtype
from ._typing import TypedField

if TYPE_CHECKING:
    BinaryField: TypeAlias = TypedField[Literal["binary"], bytes]
    FixedSizeBinaryField: TypeAlias = TypedField[
        Literal["fixed_size_binary"], bytes
    ]
    LargeBinaryField: TypeAlias = TypedField[Literal["large_binary"], bytes]
    BinaryViewField: TypeAlias = TypedField[Literal["binary_view"], bytes]
    Utf8Field: TypeAlias = TypedField[Literal["utf8"], str]
    LargeUtf8Field: TypeAlias = TypedField[Literal["large_utf8"], str]
    Utf8ViewField: TypeAlias = TypedField[Literal["utf8_view"], str]
else:
    BinaryField = FixedSizeBinaryField = LargeBinaryField = BinaryViewField = Field
    Utf8Field = LargeUtf8Field = Utf8ViewField = Field

_BINARY = simple_dtype("binary")
_LARGE_BINARY = simple_dtype("large_binary")
_BINARY_VIEW = simple_dtype("binary_view")
_UTF8 = simple_dtype("utf8")
_LARGE_UTF8 = simple_dtype("large_utf8")
_UTF8_VIEW = simple_dtype("utf8_view")


def binary(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> BinaryField:
    return new_field(BinaryField, name, _BINARY, nullable, metadata)


def fixed_size_binary(
    name: str,
    byte_width: int,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> FixedSizeBinaryField:
    return new_field(
        FixedSizeBinaryField,
        name,
        DataType._fixed_size_binary(byte_width),
        nullable,
        metadata,
    )


def large_binary(
    name: str,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> LargeBinaryField:
    return new_field(LargeBinaryField, name, _LARGE_BINARY, nullable, metadata)


def binary_view(
    name: str,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> BinaryViewField:
    return new_field(BinaryViewField, name, _BINARY_VIEW, nullable, metadata)


def utf8(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> Utf8Field:
    return new_field(Utf8Field, name, _UTF8, nullable, metadata)


def large_utf8(
    name: str,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> LargeUtf8Field:
    return new_field(LargeUtf8Field, name, _LARGE_UTF8, nullable, metadata)


def utf8_view(
    name: str,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> Utf8ViewField:
    return new_field(Utf8ViewField, name, _UTF8_VIEW, nullable, metadata)


__all__ = [
    "BinaryField",
    "BinaryViewField",
    "FixedSizeBinaryField",
    "LargeBinaryField",
    "LargeUtf8Field",
    "Utf8Field",
    "Utf8ViewField",
    "binary",
    "binary_view",
    "fixed_size_binary",
    "large_binary",
    "large_utf8",
    "utf8",
    "utf8_view",
]
