"""The string field factories: one family, eighteen real leaves.

Every string is one datatype, ``DataType.string(...)``, and every factory
here is that datatype with a leaf picked once. A leaf is a shape in a
charset: six shapes - plain, large, view, large view, fixed and sized - in
each of UTF-8, US-ASCII and windows-1252, the three charsets a string
datatype exists for. Two shapes per charset *are* a number - ``fixed_*`` is
an exact width padded with trailing NUL and ``sized_*`` a maximum - so
``fixed`` and ``max`` are the two names that number is spelled by, and the
other four stand alone and refuse one. :func:`string` takes the whole
declaration: a charset-free layout, the charset, and the number.

The ten registered codes are not strings: a currency is an identity over
ISO 4217 that stores as the text it is, so its factory lives in
:mod:`yggdryl.codes`.
"""

from __future__ import annotations

from typing import TYPE_CHECKING, Literal, TypeAlias

from ._native import DataType, Field
from ._common import MetadataInput, new_field, simple_dtype
from ._typing import TypedField

if TYPE_CHECKING:
    StringField: TypeAlias = TypedField[
        Literal[
            "utf8",
            "large_utf8",
            "utf8_view",
            "large_utf8_view",
            "fixed_utf8",
            "sized_utf8",
            "ascii",
            "large_ascii",
            "ascii_view",
            "large_ascii_view",
            "fixed_ascii",
            "sized_ascii",
            "cp1252",
            "large_cp1252",
            "cp1252_view",
            "large_cp1252_view",
            "fixed_cp1252",
            "sized_cp1252",
        ],
        str,
    ]
else:
    StringField = Field

_UTF8 = simple_dtype("utf8")
_LARGE_UTF8 = simple_dtype("large_utf8")
_UTF8_VIEW = simple_dtype("utf8_view")
_LARGE_UTF8_VIEW = simple_dtype("large_utf8_view")
_ASCII = simple_dtype("ascii")
_LARGE_ASCII = simple_dtype("large_ascii")
_ASCII_VIEW = simple_dtype("ascii_view")
_LARGE_ASCII_VIEW = simple_dtype("large_ascii_view")
_CP1252 = simple_dtype("cp1252")
_LARGE_CP1252 = simple_dtype("large_cp1252")
_CP1252_VIEW = simple_dtype("cp1252_view")
_LARGE_CP1252_VIEW = simple_dtype("large_cp1252_view")


def _bound(fixed: int | None, max: int | None) -> int | None:
    """The one number a declaration carries, under the name its leaf reads."""

    if fixed is not None and max is not None:
        raise TypeError("a string declares either a fixed width or a maximum, not both")
    return fixed if fixed is not None else max


def string(
    name: str,
    *,
    layout: str = "string",
    charset: str | None = None,
    fixed: int | None = None,
    max: int | None = None,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> StringField:
    """A string in one leaf, bounded by ``fixed`` or ``max``.

    ``layout`` takes any spelling of a leaf - its canonical name (``utf8``,
    ``sized_cp1252``, ...) or a charset-free one (``string``,
    ``fixed_string``, ``string_view``, ``large_string``,
    ``large_string_view``, ``varchar``, ...). Only a charset-free spelling
    takes a ``charset``, which restates the leaf in that charset's family.
    ``fixed`` is the exact width a fixed leaf stores; ``max`` the most bytes
    a sized leaf holds, which the plain shape is the shorter spelling of.
    No other leaf takes a number.

    Raises:
        TypeError: when both ``fixed`` and ``max`` are given.
        ValueError: when the layout, charset, or bound is not one the core
            accepts - a charset beside a charset-named spelling, a charset
            with no string datatype, a leaf that is its number with none
            stated, a number beside a leaf that carries none, or a bound of
            zero.
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


def large_utf8_view(
    name: str,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> StringField:
    """The UTF-8 view layout over 64-bit offsets.

    Arrow has one view width, so this crosses an Arrow boundary as
    ``utf8_view`` with the leaf written in the ``yggdryl.string`` document.
    """

    return new_field(StringField, name, _LARGE_UTF8_VIEW, nullable, metadata)


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


def sized_utf8(
    name: str,
    max: int,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> StringField:
    """At most ``max`` bytes of UTF-8 per value, stored as plain ``utf8``.

    Arrow has no bounded string, so the maximum rides the ``yggdryl.string``
    document while the storage is the ``utf8`` the values fill.

    Raises:
        ValueError: when ``max`` is not at least one byte.
    """

    return new_field(
        StringField,
        name,
        DataType.string("sized_utf8", None, max),
        nullable,
        metadata,
    )


def ascii(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> StringField:
    """Variable-width US-ASCII: any length, every byte below ``0x80``, none NUL."""

    return new_field(StringField, name, _ASCII, nullable, metadata)


def large_ascii(
    name: str,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> StringField:
    """Unbounded US-ASCII with 64-bit offsets, over Arrow's ``large_utf8``."""

    return new_field(StringField, name, _LARGE_ASCII, nullable, metadata)


def ascii_view(
    name: str,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> StringField:
    """Unbounded US-ASCII in the view layout, over Arrow's ``utf8_view``."""

    return new_field(StringField, name, _ASCII_VIEW, nullable, metadata)


def large_ascii_view(
    name: str,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> StringField:
    """The US-ASCII view layout over 64-bit offsets, over Arrow's ``utf8_view``."""

    return new_field(StringField, name, _LARGE_ASCII_VIEW, nullable, metadata)


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


def sized_ascii(
    name: str,
    max: int,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> StringField:
    """At most ``max`` bytes of US-ASCII per value, stored as plain ``ascii``.

    Raises:
        ValueError: when ``max`` is not at least one byte.
    """

    return new_field(
        StringField,
        name,
        DataType.string("sized_ascii", None, max),
        nullable,
        metadata,
    )


def cp1252(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> StringField:
    """Variable-width windows-1252, over Arrow's ``binary``.

    Its bytes are not UTF-8, so every windows-1252 leaf rides binary
    storage with the charset written in the ``yggdryl.string`` document.
    """

    return new_field(StringField, name, _CP1252, nullable, metadata)


def large_cp1252(
    name: str,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> StringField:
    """Unbounded windows-1252 with 64-bit offsets, over Arrow's ``large_binary``."""

    return new_field(StringField, name, _LARGE_CP1252, nullable, metadata)


def cp1252_view(
    name: str,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> StringField:
    """Unbounded windows-1252 in the view layout, over Arrow's ``binary_view``."""

    return new_field(StringField, name, _CP1252_VIEW, nullable, metadata)


def large_cp1252_view(
    name: str,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> StringField:
    """The windows-1252 view layout over 64-bit offsets, over Arrow's ``binary_view``."""

    return new_field(StringField, name, _LARGE_CP1252_VIEW, nullable, metadata)


def fixed_cp1252(
    name: str,
    width: int,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> StringField:
    """Windows-1252 padded with trailing NUL to exactly ``width`` bytes.

    Raises:
        ValueError: when ``width`` is not at least one byte.
    """

    return new_field(
        StringField,
        name,
        DataType.string("fixed_cp1252", None, width),
        nullable,
        metadata,
    )


def sized_cp1252(
    name: str,
    max: int,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> StringField:
    """At most ``max`` bytes of windows-1252 per value, stored as plain ``cp1252``.

    Raises:
        ValueError: when ``max`` is not at least one byte.
    """

    return new_field(
        StringField,
        name,
        DataType.string("sized_cp1252", None, max),
        nullable,
        metadata,
    )


__all__ = [
    "StringField",
    "ascii",
    "ascii_view",
    "cp1252",
    "cp1252_view",
    "fixed_ascii",
    "fixed_cp1252",
    "fixed_utf8",
    "large_ascii",
    "large_ascii_view",
    "large_cp1252",
    "large_cp1252_view",
    "large_utf8",
    "large_utf8_view",
    "sized_ascii",
    "sized_cp1252",
    "sized_utf8",
    "string",
    "utf8",
    "utf8_view",
]
