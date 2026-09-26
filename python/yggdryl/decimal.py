"""Exact decimal field factories."""

from __future__ import annotations

from decimal import Decimal
from typing import TYPE_CHECKING, Literal, SupportsIndex, TypeAlias, cast, overload

from ._native import DataType, Field
from ._common import MetadataInput, new_field
from ._typing import TypedField

if TYPE_CHECKING:
    Decimal32Field: TypeAlias = TypedField[Literal["decimal32"], Decimal]
    Decimal64Field: TypeAlias = TypedField[Literal["decimal64"], Decimal]
    Decimal128Field: TypeAlias = TypedField[Literal["decimal128"], Decimal]
    Decimal256Field: TypeAlias = TypedField[Literal["decimal256"], Decimal]
    #: The fixed `decimal` leaf: thirty-eight digits at scale eighteen.
    DecimalField: TypeAlias = TypedField[Literal["decimal"], Decimal]
    #: The fixed `bigdecimal` leaf: seventy-six digits at scale eighteen.
    BigDecimalField: TypeAlias = TypedField[Literal["bigdecimal"], Decimal]
    #: What `decimal(name, precision, scale)` answers: the narrowest width.
    DecimalWidthField: TypeAlias = (
        Decimal32Field | Decimal64Field | Decimal128Field | Decimal256Field
    )
else:
    Decimal32Field = Decimal64Field = Decimal128Field = Decimal256Field = Field
    DecimalField = BigDecimalField = DecimalWidthField = Field

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


@overload
def decimal(
    name: str,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> DecimalField: ...


@overload
def decimal(
    name: str,
    precision: DecimalArgument,
    scale: DecimalArgument = 0,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> DecimalWidthField: ...


def decimal(
    name: str,
    precision: DecimalArgument | None = None,
    scale: DecimalArgument | None = None,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> DecimalField | DecimalWidthField:
    """The fixed ``decimal`` leaf, or the narrowest width a stated precision fits.

    Without a precision the field is ``decimal``: thirty-eight digits, eighteen
    of them fractional, ``decimal128(38, 18)`` preapplied. With one it is what
    ``DataType.decimal(precision, scale)`` selects.
    """

    if precision is None:
        if scale is not None:
            raise TypeError("decimal(): a scale needs a precision")
        return cast(
            DecimalField,
            new_field(Field, name, DataType._simple("decimal"), nullable, metadata),
        )
    return cast(
        DecimalWidthField,
        new_field(
            Field,
            name,
            DataType.decimal(precision, 0 if scale is None else scale),
            nullable,
            metadata,
        ),
    )


def bigdecimal(
    name: str,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> BigDecimalField:
    """The fixed ``bigdecimal`` leaf: seventy-six digits, eighteen fractional."""

    return new_field(
        BigDecimalField,
        name,
        DataType._simple("bigdecimal"),
        nullable,
        metadata,
    )


__all__ = [
    "BigDecimalField",
    "Decimal32Field",
    "Decimal64Field",
    "Decimal128Field",
    "Decimal256Field",
    "DecimalArgument",
    "DecimalField",
    "DecimalWidthField",
    "bigdecimal",
    "decimal",
    "decimal32",
    "decimal64",
    "decimal128",
    "decimal256",
]
