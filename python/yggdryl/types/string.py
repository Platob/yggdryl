"""The string field factories: one family, five layouts, one charset each.

Every string is one datatype, ``DataType.string(...)``, and every factory here
is that datatype with a layout and charset picked once: ``utf8`` is the
default layout in UTF-8, ``ascii`` the same layout in US-ASCII, ``fixed_utf8``
and ``fixed_ascii`` the fixed layout, and :func:`string` takes the whole
declaration. The bound is one number with one reading per layout - the exact
width on the fixed layout, the maximum stored bytes everywhere else - so
``fixed`` and ``max`` are the two names it is spelled by.

The nine registered codes are not strings: a currency is three ASCII bytes
with an identity, so its factory lives in :mod:`yggdryl.types.codes`.
"""

from __future__ import annotations

from typing import TYPE_CHECKING, Literal, TypeAlias

from .._native import DataType, Field
from ._common import MetadataInput, new_field, simple_dtype
from ._typing import TypedField

if TYPE_CHECKING:
    StringField: TypeAlias = TypedField[
        Literal[
            "string",
            "fixed_string",
            "string_view",
            "large_string",
            "large_string_view",
        ],
        str,
    ]
else:
    StringField = Field

_UTF8 = simple_dtype("utf8")
_LARGE_UTF8 = simple_dtype("large_utf8")
_UTF8_VIEW = simple_dtype("utf8_view")
_ASCII = simple_dtype("ascii")


def _bound(fixed: int | None, max: int | None) -> int | None:
    """The one bound a declaration carries, under the name its layout reads."""

    if fixed is not None and max is not None:
        raise TypeError("a string declares either a fixed width or a maximum, not both")
    return fixed if fixed is not None else max


def string(
    name: str,
    *,
    layout: str = "string",
    charset: str = "utf-8",
    fixed: int | None = None,
    max: int | None = None,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> StringField:
    """A string in one layout and charset, bounded by ``fixed`` or ``max``.

    ``layout`` takes any of a layout's three spellings (``string``, ``utf8``,
    ``ascii``; ``fixed_string``; ``string_view``; ``large_string``;
    ``large_string_view``) and ``charset`` any documented charset alias.
    ``fixed`` is the exact width the fixed layout stores; ``max`` the most
    bytes a value on any other layout may hold.

    Raises:
        TypeError: when both ``fixed`` and ``max`` are given.
        ValueError: when the layout, charset, or bound is not one the core
            accepts - a fixed layout with no width, or a bound of zero.
    """

    dtype = DataType.string(layout, charset, _bound(fixed, max))
    return new_field(StringField, name, dtype, nullable, metadata)


def utf8(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> StringField:
    """Unbounded UTF-8 with 32-bit offsets: Arrow's ``utf8``."""

    return new_field(StringField, name, _UTF8, nullable, metadata)


def large_utf8(
    name: str,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> StringField:
    """Unbounded UTF-8 with 64-bit offsets: Arrow's ``large_utf8``."""

    return new_field(StringField, name, _LARGE_UTF8, nullable, metadata)


def utf8_view(
    name: str,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> StringField:
    """Unbounded UTF-8 in the view layout: Arrow's ``utf8_view``."""

    return new_field(StringField, name, _UTF8_VIEW, nullable, metadata)


def ascii(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> StringField:
    """Variable-width US-ASCII: any length, every byte below ``0x80``, none NUL."""

    return new_field(StringField, name, _ASCII, nullable, metadata)


def fixed_utf8(
    name: str,
    width: int,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> StringField:
    """UTF-8 padded with trailing NUL to exactly ``width`` bytes.

    Raises:
        ValueError: when ``width`` is not at least one byte.
    """

    return new_field(StringField, name, DataType.fixed_utf8(width), nullable, metadata)


def fixed_ascii(
    name: str,
    width: int,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> StringField:
    """US-ASCII padded with trailing NUL to exactly ``width`` bytes.

    Raises:
        ValueError: when ``width`` is not at least one byte.
    """

    return new_field(StringField, name, DataType.fixed_ascii(width), nullable, metadata)


__all__ = [
    "StringField",
    "ascii",
    "fixed_ascii",
    "fixed_utf8",
    "large_utf8",
    "string",
    "utf8",
    "utf8_view",
]
