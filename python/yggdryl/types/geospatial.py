"""Variant, GUID, and geospatial field factories."""

from __future__ import annotations

from typing import TYPE_CHECKING, Literal, TypeAlias

from .._native import DataType, Field
from ._common import MetadataInput, new_field
from ._typing import TypedField

if TYPE_CHECKING:
    VariantField: TypeAlias = TypedField[Literal["variant"], object]
    GuidField: TypeAlias = TypedField[Literal["guid"], str]
    GeometryField: TypeAlias = TypedField[Literal["geometry"], bytes]
    GeographyField: TypeAlias = TypedField[Literal["geography"], bytes]
else:
    VariantField = GuidField = GeometryField = GeographyField = Field


_GUID = DataType("guid")


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


def guid(
    name: str,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> GuidField:
    """Create a field of one 128-bit universally unique identifier.

    Storage is the sixteen bytes; every value reads back as the 36-character
    lowercase hyphenated spelling.
    """

    return new_field(GuidField, name, _GUID, nullable, metadata)


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
    "VariantField",
    "geography",
    "geometry",
    "variant",
]
