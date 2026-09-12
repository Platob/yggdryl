"""The string field factories: one layout over one charset.

``utf8``, ``large_utf8``, ``utf8_view``, ``ascii`` and ``ascii(n)`` are the
spellings a string in a charset this crate already names answers, so those live
in :mod:`yggdryl.types.binary` and :mod:`yggdryl.types.ascii`. These five build
the string a charset the crate has no other name for: the same five layouts -
variable, fixed-width, view, large, large view - over any registered charset.

A bound is one number read two ways. Under ``fixed_string`` it is exactly that
many bytes, padded; under every other layout it is at most that many. The
layout is what says which, which is why one keyword serves both.

One datatype has one name, so a string another spelling already covers answers
that spelling rather than a second way to write it::

    >>> from yggdryl import types
    >>> types.string("value", "utf-8").dtype.id
    'utf8'
    >>> types.string("value", "windows-1252").dtype.id
    'string'
"""

from __future__ import annotations

from typing import TYPE_CHECKING, Literal, TypeAlias, cast

from .._native import DataType, Field
from ._common import MetadataInput, new_field
from ._typing import TypedField

if TYPE_CHECKING:
    StringField: TypeAlias = TypedField[Literal["string"], str]
    FixedStringField: TypeAlias = TypedField[Literal["fixed_string"], str]
    StringViewField: TypeAlias = TypedField[Literal["string_view"], str]
    LargeStringField: TypeAlias = TypedField[Literal["large_string"], str]
    LargeStringViewField: TypeAlias = TypedField[Literal["large_string_view"], str]
else:
    StringField = FixedStringField = StringViewField = LargeStringField = (
        LargeStringViewField
    ) = Field


def string(
    name: str,
    charset: str = "utf-8",
    *,
    max_bytes: int | None = None,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> StringField:
    """Variable-width text in ``charset``, at most ``max_bytes`` bytes.

    Raises:
        ValueError: when the charset is not registered, or the bound is zero.
    """

    return cast(
        StringField,
        new_field(
            Field,
            name,
            DataType.string(charset, "string", max_bytes),
            nullable,
            metadata,
        ),
    )


def fixed_string(
    name: str,
    charset: str,
    width: int,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> FixedStringField:
    """Text in ``charset`` stored as exactly ``width`` bytes, padded.

    Raises:
        ValueError: when the charset is not registered, or the width is zero.
    """

    return cast(
        FixedStringField,
        new_field(
            Field,
            name,
            DataType.string(charset, "fixed_string", width),
            nullable,
            metadata,
        ),
    )


def string_view(
    name: str,
    charset: str = "utf-8",
    *,
    max_bytes: int | None = None,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> StringViewField:
    """Text in ``charset`` over Arrow's view layout."""

    return cast(
        StringViewField,
        new_field(
            Field,
            name,
            DataType.string(charset, "string_view", max_bytes),
            nullable,
            metadata,
        ),
    )


def large_string(
    name: str,
    charset: str = "utf-8",
    *,
    max_bytes: int | None = None,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> LargeStringField:
    """Text in ``charset`` over 64-bit offsets."""

    return cast(
        LargeStringField,
        new_field(
            Field,
            name,
            DataType.string(charset, "large_string", max_bytes),
            nullable,
            metadata,
        ),
    )


def large_string_view(
    name: str,
    charset: str = "utf-8",
    *,
    max_bytes: int | None = None,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> LargeStringViewField:
    """Text in ``charset`` over the view layout, declared large."""

    return cast(
        LargeStringViewField,
        new_field(
            Field,
            name,
            DataType.string(charset, "large_string_view", max_bytes),
            nullable,
            metadata,
        ),
    )


__all__ = [
    "FixedStringField",
    "LargeStringField",
    "LargeStringViewField",
    "StringField",
    "StringViewField",
    "fixed_string",
    "large_string",
    "large_string_view",
    "string",
    "string_view",
]
