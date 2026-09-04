"""Shared construction helpers for datatype-specific field factories."""

from __future__ import annotations

from collections.abc import Iterable, Mapping
from typing import TypeAlias, TypeVar, cast

from .._native import DataType, Field

MetadataInput: TypeAlias = (
    Mapping[str, str] | Iterable[tuple[str, str]] | None
)
# The native inference boundary also accepts PyArrow datatypes and Python
# annotations such as ``int`` or ``list[str]``. ``object`` is intentional:
# Python's runtime typing objects have no useful common static protocol, and
# the native constructor remains the single source of validation.
DataTypeInput: TypeAlias = object

_FieldT = TypeVar("_FieldT", bound=Field)


def simple_dtype(expression: str) -> DataType:
    """Build one module-level singleton through the native variant bridge."""

    return DataType._simple(expression)


def new_field(
    expected: type[_FieldT],
    name: str,
    value: DataType,
    nullable: bool,
    metadata: MetadataInput,
) -> _FieldT:
    # ``expected`` is the static phantom view. At runtime every alias is the
    # same native Field class, so no schema object is copied or wrapped.
    return cast(_FieldT, Field(name, value, nullable=nullable, metadata=metadata))


__all__ = [
    "DataTypeInput",
    "MetadataInput",
    "new_field",
    "simple_dtype",
]
