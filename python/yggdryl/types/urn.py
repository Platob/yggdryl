"""URN field factory."""

from __future__ import annotations

from typing import TYPE_CHECKING, Literal, TypeAlias

from .._native import DataType, Field
from ._common import MetadataInput, new_field
from ._typing import TypedField

if TYPE_CHECKING:
    UrnField: TypeAlias = TypedField[Literal["urn"], str]
else:
    UrnField = Field


_URN = DataType("urn")


def urn(
    name: str,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> UrnField:
    """Create a field of validated, canonical resource names."""

    return new_field(UrnField, name, _URN, nullable, metadata)


__all__ = ["UrnField", "urn"]
