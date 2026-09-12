"""The byte field factories: one family, Arrow's four layouts.

Every byte column is one datatype, ``DataType.bytes(...)``, and every factory
here is that datatype with a layout picked once. The bound is one number with
one reading per layout - the exact width on ``fixed_size_binary``, the
maximum stored bytes everywhere else - so ``fixed`` and ``max`` are the two
names it is spelled by. Bytes are never padded: a fixed value is exactly its
width.
"""

from __future__ import annotations

from typing import TYPE_CHECKING, Literal, TypeAlias

from .._native import DataType, Field
from ._common import MetadataInput, new_field, simple_dtype
from ._typing import TypedField

if TYPE_CHECKING:
    BytesField: TypeAlias = TypedField[
        Literal["binary", "fixed_size_binary", "large_binary", "binary_view"],
        bytes,
    ]
else:
    BytesField = Field

_BINARY = simple_dtype("binary")
_LARGE_BINARY = simple_dtype("large_binary")
_BINARY_VIEW = simple_dtype("binary_view")


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

    ``layout`` is ``binary``, ``fixed_size_binary``, ``large_binary``, or
    ``binary_view``. ``fixed`` is the exact width the fixed layout stores;
    ``max`` the most bytes a value on any other layout may hold.

    Raises:
        TypeError: when both ``fixed`` and ``max`` are given.
        ValueError: when the layout or bound is not one the core accepts - the
            fixed layout with no width, or a bound of zero.
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


__all__ = [
    "BytesField",
    "binary",
    "binary_view",
    "bytes",
    "fixed_size_binary",
    "large_binary",
]
