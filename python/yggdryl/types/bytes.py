"""The byte field factories: one family, six real leaves.

Every byte column is one datatype, ``DataType.bytes(...)``, and every factory
here is that datatype with a leaf picked once. Two of the six leaves *are* a
number - ``fixed_binary`` is an exact width and ``sized_binary`` a maximum -
so ``fixed`` and ``max`` are the two names that number is spelled by, and the
other four stand alone and refuse one. Bytes are never padded: a fixed value
is exactly its width.
"""

from __future__ import annotations

from typing import TYPE_CHECKING, Literal, TypeAlias

from .._native import DataType, Field
from ._common import MetadataInput, new_field, simple_dtype
from ._typing import TypedField

if TYPE_CHECKING:
    BytesField: TypeAlias = TypedField[
        Literal[
            "binary",
            "large_binary",
            "binary_view",
            "large_binary_view",
            "fixed_binary",
            "sized_binary",
        ],
        bytes,
    ]
else:
    BytesField = Field

_BINARY = simple_dtype("binary")
_LARGE_BINARY = simple_dtype("large_binary")
_BINARY_VIEW = simple_dtype("binary_view")
_LARGE_BINARY_VIEW = simple_dtype("large_binary_view")


def bytes(
    name: str,
    *,
    layout: str = "binary",
    fixed: int | None = None,
    max: int | None = None,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> BytesField:
    """A byte column in one layout, bounded by ``fixed`` or ``max``.

    ``layout`` names one of the six leaves. ``fixed`` is the exact width
    ``fixed_binary`` stores; ``max`` the most bytes ``sized_binary`` holds,
    which ``binary`` is the shorter spelling of. No other leaf takes a number.

    Raises:
        TypeError: when both ``fixed`` and ``max`` are given.
        ValueError: when the layout or bound is not one the core accepts - a
            leaf that is its number with none stated, a number beside a leaf
            that carries none, or a bound of zero.
    """

    if fixed is not None and max is not None:
        raise TypeError("a byte column declares either a fixed width or a maximum, not both")
    dtype = DataType.bytes(layout, fixed if fixed is not None else max)
    return new_field(BytesField, name, dtype, nullable, metadata)


def binary(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> BytesField:
    """Unbounded bytes with 32-bit offsets: Arrow's ``binary``."""

    return new_field(BytesField, name, _BINARY, nullable, metadata)


def fixed_size_binary(
    name: str,
    byte_width: int,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> BytesField:
    """Exactly ``byte_width`` bytes per value: Arrow's ``fixed_size_binary``.

    Raises:
        ValueError: when ``byte_width`` is not at least one byte.
    """

    return new_field(
        BytesField,
        name,
        DataType.fixed_size_binary(byte_width),
        nullable,
        metadata,
    )


def large_binary(
    name: str,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> BytesField:
    """Unbounded bytes with 64-bit offsets: Arrow's ``large_binary``."""

    return new_field(BytesField, name, _LARGE_BINARY, nullable, metadata)


def binary_view(
    name: str,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> BytesField:
    """Unbounded bytes in the view layout: Arrow's ``binary_view``."""

    return new_field(BytesField, name, _BINARY_VIEW, nullable, metadata)


def large_binary_view(
    name: str,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> BytesField:
    """The view layout over 64-bit offsets.

    Arrow has one view width, so this crosses an Arrow boundary as
    ``binary_view`` with the leaf written in the ``yggdryl.bytes`` document.
    """

    return new_field(BytesField, name, _LARGE_BINARY_VIEW, nullable, metadata)


def sized_binary(
    name: str,
    max: int,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> BytesField:
    """At most ``max`` bytes per value, stored as plain ``binary``.

    Arrow has no bounded binary, so the maximum rides the ``yggdryl.bytes``
    document while the storage is the ``binary`` the values fill.

    Raises:
        ValueError: when ``max`` is not at least one byte.
    """

    return new_field(
        BytesField,
        name,
        DataType.bytes("sized_binary", max),
        nullable,
        metadata,
    )


__all__ = [
    "BytesField",
    "binary",
    "binary_view",
    "bytes",
    "fixed_size_binary",
    "large_binary",
    "large_binary_view",
    "sized_binary",
]
