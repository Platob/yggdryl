"""Variant, UUID, and geospatial field factories."""

from __future__ import annotations

from typing import TYPE_CHECKING, Literal, TypeAlias

from .._native import DataType, Field
from ._common import MetadataInput, new_field
from ._typing import TypedField

if TYPE_CHECKING:
    VariantField: TypeAlias = TypedField[Literal["variant"], object]
    UuidField: TypeAlias = TypedField[
        Literal["uuid", "uuidv4", "uuidv7", "uuidv8"], str
    ]
    GeometryField: TypeAlias = TypedField[Literal["geometry"], bytes]
    GeographyField: TypeAlias = TypedField[Literal["geography"], bytes]
else:
    VariantField = UuidField = GeometryField = GeographyField = Field


_UUID = DataType("uuid")
_UUIDV4 = DataType("uuidv4")
_UUIDV7 = DataType("uuidv7")
_UUIDV8 = DataType("uuidv8")


def variant(
    name: str,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> VariantField:
    """Create a self-describing semi-structured Variant field.

    The parenthesis disambiguates on ``DataType.variant``: this factory calls
    it bare, so the field carries the Variant datatype, never the dense-union
    sugar that :func:`dense_union` builds from members.
    """

    value = DataType.variant()
    return new_field(VariantField, name, value, nullable, metadata)


def uuid(
    name: str,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> UuidField:
    """Create a field of one 128-bit universally unique identifier.

    Storage is the sixteen bytes; every value reads back as the 36-character
    lowercase hyphenated spelling. This leaf admits every RFC 9562 version;
    the three below each admit one.
    """

    return new_field(UuidField, name, _UUID, nullable, metadata)


def uuidv4(
    name: str,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> UuidField:
    """A field of RFC 9562 version 4 identifiers: random ones.

    A value of another version is refused, which is the whole difference
    between this and :func:`uuid`.
    """

    return new_field(UuidField, name, _UUIDV4, nullable, metadata)


def uuidv7(
    name: str,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> UuidField:
    """A field of RFC 9562 version 7 identifiers: time-ordered ones."""

    return new_field(UuidField, name, _UUIDV7, nullable, metadata)


def uuidv8(
    name: str,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> UuidField:
    """A field of RFC 9562 version 8 identifiers: custom ones."""

    return new_field(UuidField, name, _UUIDV8, nullable, metadata)


def geometry(
    name: str,
    crs: str | None = None,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> GeometryField:
    """Create a planar geometry field carrying Well-Known Binary."""

    value = DataType.geometry(crs)
    return new_field(GeometryField, name, value, nullable, metadata)


def geography(
    name: str,
    crs: str | None = None,
    algorithm: str | None = None,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> GeographyField:
    """Create a geography field: features on a sphere or spheroid."""

    value = DataType.geography(crs, algorithm)
    return new_field(GeographyField, name, value, nullable, metadata)


__all__ = [
    "GeographyField",
    "GeometryField",
    "UuidField",
    "VariantField",
    "geography",
    "geometry",
    "uuid",
    "uuidv4",
    "uuidv7",
    "uuidv8",
    "variant",
]
