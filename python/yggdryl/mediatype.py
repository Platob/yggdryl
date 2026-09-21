"""MediaType field factory."""

from __future__ import annotations

from typing import TYPE_CHECKING, Literal, TypeAlias

from ._native import DataType, Field, MediaType
from ._common import MetadataInput, new_field
from ._typing import TypedField

if TYPE_CHECKING:
    MediaTypeField: TypeAlias = TypedField[Literal["mediatype"], str]
else:
    MediaTypeField = Field


_MEDIATYPE = DataType("mediatype")


def mediatype(
    name: str,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> MediaTypeField:
    """Create a field of MIME types with their charset and content codings."""

    return new_field(MediaTypeField, name, _MEDIATYPE, nullable, metadata)


__all__ = ["MediaType", "MediaTypeField", "mediatype"]
