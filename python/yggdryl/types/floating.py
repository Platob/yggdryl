"""Floating-point field factories."""

from __future__ import annotations

from typing import TYPE_CHECKING, Literal, TypeAlias

from .._native import Field
from ._common import MetadataInput, new_field, simple_dtype
from ._typing import TypedField

if TYPE_CHECKING:
    Float16Field: TypeAlias = TypedField[Literal["float16"], float]
    Float32Field: TypeAlias = TypedField[Literal["float32"], float]
    Float64Field: TypeAlias = TypedField[Literal["float64"], float]
else:
    Float16Field = Float32Field = Float64Field = Field

_FLOAT16 = simple_dtype("float16")
_FLOAT32 = simple_dtype("float32")
_FLOAT64 = simple_dtype("float64")


def float16(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> Float16Field:
    return new_field(Float16Field, name, _FLOAT16, nullable, metadata)


def float32(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> Float32Field:
    return new_field(Float32Field, name, _FLOAT32, nullable, metadata)


def float64(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> Float64Field:
    return new_field(Float64Field, name, _FLOAT64, nullable, metadata)


__all__ = [
    "Float16Field",
    "Float32Field",
    "Float64Field",
    "float16",
    "float32",
    "float64",
]
