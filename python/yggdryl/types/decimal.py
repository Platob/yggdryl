"""Exact decimal field factories."""

from __future__ import annotations

from decimal import Decimal
from typing import TYPE_CHECKING, Literal, SupportsIndex, TypeAlias, cast

from .._native import DataType, Field
from ._common import MetadataInput, new_field
from ._typing import TypedField

if TYPE_CHECKING:
    Decimal32Field: TypeAlias = TypedField[Literal["decimal32"], Decimal]
    Decimal64Field: TypeAlias = TypedField[Literal["decimal64"], Decimal]
    Decimal128Field: TypeAlias = TypedField[Literal["decimal128"], Decimal]
    Decimal256Field: TypeAlias = TypedField[Literal["decimal256"], Decimal]
    DecimalField: TypeAlias = Decimal128Field | Decimal256Field
else:
    Decimal32Field = Decimal64Field = Decimal128Field = Decimal256Field = Field
    DecimalField = Field

DecimalArgument: TypeAlias = SupportsIndex | str


def decimal32(
    name: str,
    precision: DecimalArgument,
    scale: DecimalArgument = 0,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> Decimal32Field:
    return new_field(
        Decimal32Field,
        name,
        DataType._decimal("decimal32", precision, scale),
        nullable,
        metadata,
    )


def decimal64(
    name: str,
    precision: DecimalArgument,
    scale: DecimalArgument = 0,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> Decimal64Field:
    return new_field(
        Decimal64Field,
        name,
        DataType._decimal("decimal64", precision, scale),
        nullable,
        metadata,
    )


def decimal128(
    name: str,
    precision: DecimalArgument,
    scale: DecimalArgument = 0,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> Decimal128Field:
    return new_field(
        Decimal128Field,
        name,
        DataType._decimal("decimal128", precision, scale),
        nullable,
        metadata,
    )


def decimal256(
    name: str,
    precision: DecimalArgument,
    scale: DecimalArgument = 0,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> Decimal256Field:
    return new_field(
        Decimal256Field,
        name,
        DataType._decimal("decimal256", precision, scale),
        nullable,
        metadata,
    )


def decimal(
    name: str,
    precision: DecimalArgument,
    scale: DecimalArgument = 0,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> DecimalField:
    """Select Decimal128 or Decimal256 through ``DataType.decimal``."""

    return cast(
        DecimalField,
        new_field(
            Field,
            name,
            DataType.decimal(precision, scale),
            nullable,
            metadata,
        ),
    )


__all__ = [
    "Decimal32Field",
    "Decimal64Field",
    "Decimal128Field",
    "Decimal256Field",
    "DecimalArgument",
    "DecimalField",
    "decimal",
    "decimal32",
    "decimal64",
    "decimal128",
    "decimal256",
]
