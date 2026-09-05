"""Version field factory."""

from __future__ import annotations

from typing import TYPE_CHECKING, Literal, TypeAlias

from .._native import DataType, Field
from ._common import MetadataInput, new_field
from ._typing import TypedField

if TYPE_CHECKING:
    VersionField: TypeAlias = TypedField[Literal["version"], str]
else:
    VersionField = Field


_VERSION = DataType("version")


def version(
    name: str,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> VersionField:
    """Create a field of canonical, numerically ordered versions."""

    return new_field(VersionField, name, _VERSION, nullable, metadata)


__all__ = ["VersionField", "version"]
