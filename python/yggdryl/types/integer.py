"""Signed and unsigned integer field factories."""

from __future__ import annotations

from typing import TYPE_CHECKING, Literal, TypeAlias

from .._native import Field
from ._common import MetadataInput, new_field, simple_dtype
from ._typing import TypedField

if TYPE_CHECKING:
    Int8Field: TypeAlias = TypedField[Literal["int8"], int]
    Int16Field: TypeAlias = TypedField[Literal["int16"], int]
    Int32Field: TypeAlias = TypedField[Literal["int32"], int]
    Int64Field: TypeAlias = TypedField[Literal["int64"], int]
    UInt8Field: TypeAlias = TypedField[Literal["uint8"], int]
    UInt16Field: TypeAlias = TypedField[Literal["uint16"], int]
    UInt32Field: TypeAlias = TypedField[Literal["uint32"], int]
    UInt64Field: TypeAlias = TypedField[Literal["uint64"], int]
else:
    Int8Field = Int16Field = Int32Field = Int64Field = Field
    UInt8Field = UInt16Field = UInt32Field = UInt64Field = Field

_INT8 = simple_dtype("int8")
_INT16 = simple_dtype("int16")
_INT32 = simple_dtype("int32")
_INT64 = simple_dtype("int64")
_UINT8 = simple_dtype("uint8")
_UINT16 = simple_dtype("uint16")
_UINT32 = simple_dtype("uint32")
_UINT64 = simple_dtype("uint64")


def int8(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> Int8Field:
    return new_field(Int8Field, name, _INT8, nullable, metadata)


def int16(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> Int16Field:
    return new_field(Int16Field, name, _INT16, nullable, metadata)


def int32(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> Int32Field:
    return new_field(Int32Field, name, _INT32, nullable, metadata)


def int64(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> Int64Field:
    return new_field(Int64Field, name, _INT64, nullable, metadata)


def uint8(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> UInt8Field:
    return new_field(UInt8Field, name, _UINT8, nullable, metadata)


def uint16(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> UInt16Field:
    return new_field(UInt16Field, name, _UINT16, nullable, metadata)


def uint32(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> UInt32Field:
    return new_field(UInt32Field, name, _UINT32, nullable, metadata)


def uint64(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> UInt64Field:
    return new_field(UInt64Field, name, _UINT64, nullable, metadata)


__all__ = [
    "Int8Field",
    "Int16Field",
    "Int32Field",
    "Int64Field",
    "UInt8Field",
    "UInt16Field",
    "UInt32Field",
    "UInt64Field",
    "int8",
    "int16",
    "int32",
    "int64",
    "uint8",
    "uint16",
    "uint32",
    "uint64",
]
