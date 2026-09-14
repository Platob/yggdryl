"""MimeType field factory."""

from __future__ import annotations

from typing import TYPE_CHECKING, Literal, TypeAlias

from .._native import DataType, Field
from ._common import MetadataInput, new_field
from ._typing import TypedField

if TYPE_CHECKING:
    MimeTypeField: TypeAlias = TypedField[Literal["mimetype"], str]
else:
    MimeTypeField = Field


_MIMETYPE = DataType("mimetype")


def mimetype(
    name: str,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> MimeTypeField:
    """Create a field of validated, canonical MIME types."""

    return new_field(MimeTypeField, name, _MIMETYPE, nullable, metadata)


__all__ = ["MimeTypeField", "mimetype"]
