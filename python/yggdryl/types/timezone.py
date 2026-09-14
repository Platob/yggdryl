"""Timezone field factory."""

from __future__ import annotations

from typing import TYPE_CHECKING, Literal, TypeAlias

from .._native import DataType, Field
from ._common import MetadataInput, new_field
from ._typing import TypedField

if TYPE_CHECKING:
    TimezoneField: TypeAlias = TypedField[Literal["timezone"], str]
else:
    TimezoneField = Field


_TIMEZONE = DataType("timezone")


def timezone(
    name: str,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> TimezoneField:
    """Create a field of canonical time zone names."""

    return new_field(TimezoneField, name, _TIMEZONE, nullable, metadata)


__all__ = ["TimezoneField", "timezone"]
