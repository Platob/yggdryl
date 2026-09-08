"""URL field factory."""

from __future__ import annotations

from typing import TYPE_CHECKING, Literal, TypeAlias

from .._native import DataType, Field
from ._common import MetadataInput, new_field
from ._typing import TypedField

if TYPE_CHECKING:
    UrlField: TypeAlias = TypedField[Literal["url"], str]
else:
    UrlField = Field


_URL = DataType("url")


def url(
    name: str,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> UrlField:
    """Create a field of validated, canonical locations."""

    return new_field(UrlField, name, _URL, nullable, metadata)


__all__ = ["UrlField", "url"]
