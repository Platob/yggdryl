"""Native ``Serie`` and the serie field factories.

A ``Serie`` is many values, as a schema-free run or as the Arrow buffers of
one field; ``SerieReader`` is the same over a stream: one record ``Serie``
per batch, each cast by one plan; ``ChunkedSerie`` is many ``Serie`` columns
under one field, held apart - a ``pyarrow.ChunkedArray``, or a
``pyarrow.Table`` of one batch per chunk.

A serie datatype is that column as a value: one item field repeated, in one
of five layouts - ``serie`` and ``large_serie`` cut by 32- or 64-bit
offsets, ``serie_view`` and ``large_serie_view`` viewed by offsets and
sizes, and ``fixed_size_serie`` holding exactly ``length`` items per row.
Every factory here is one of them, and the column a serie datatype holds
comes back as the leaf class named for its layout: ``SerieSerie``,
``LargeSerieSerie``, ``SerieViewSerie``, ``LargeSerieViewSerie``,
``FixedSizeSerieSerie``. The earlier spellings ``list``, ``large_list``,
``list_view``, ``large_list_view`` and ``fixed_size_list`` are still read
wherever a datatype is parsed, and name the same five layouts.
"""

from __future__ import annotations

from typing import TYPE_CHECKING, Any, Literal, TypeAlias, TypeVar

from ._native import (
    ChunkedSerie,
    DataType,
    Field,
    FixedSizeSerieSerie,
    LargeSerieSerie,
    LargeSerieViewSerie,
    MapSerie,
    Serie,
    SerieReader,
    SerieSerie,
    SerieViewSerie,
    StructSerie,
)
from ._common import MetadataInput, new_field
from ._typing import TypedField

_ItemT = TypeVar("_ItemT")

if TYPE_CHECKING:
    SerieField: TypeAlias = TypedField[Literal["serie"], list[_ItemT]]
    SerieViewField: TypeAlias = TypedField[Literal["serie_view"], list[_ItemT]]
    FixedSizeSerieField: TypeAlias = TypedField[
        Literal["fixed_size_serie"], list[_ItemT | None]
    ]
    LargeSerieField: TypeAlias = TypedField[Literal["large_serie"], list[_ItemT]]
    LargeSerieViewField: TypeAlias = TypedField[
        Literal["large_serie_view"], list[_ItemT]
    ]
else:
    SerieField = SerieViewField = FixedSizeSerieField = Field
    LargeSerieField = LargeSerieViewField = Field


def serie(
    name: str,
    item: TypedField[Any, _ItemT] | Field,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> SerieField[_ItemT]:
    """A serie of ``item``, cut by 32-bit offsets: Arrow's ``list``."""

    value = DataType._serie("serie", item)
    return new_field(SerieField, name, value, nullable, metadata)


def serie_view(
    name: str,
    item: TypedField[Any, _ItemT] | Field,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> SerieViewField[_ItemT]:
    """A serie of ``item`` viewed by 32-bit offsets and sizes: Arrow's ``list_view``."""

    value = DataType._serie("serie_view", item)
    return new_field(SerieViewField, name, value, nullable, metadata)


def fixed_size_serie(
    name: str,
    item: TypedField[Any, _ItemT] | Field,
    length: int,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> FixedSizeSerieField[_ItemT]:
    """A serie of exactly ``length`` ``item`` values per row: Arrow's fixed-size ``list``.

    Raises:
        ValueError: when ``length`` is not a width the core accepts.
    """

    value = DataType._serie("fixed_size_serie", item, length)
    return new_field(FixedSizeSerieField, name, value, nullable, metadata)


def large_serie(
    name: str,
    item: TypedField[Any, _ItemT] | Field,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> LargeSerieField[_ItemT]:
    """A serie of ``item``, cut by 64-bit offsets: Arrow's ``large_list``."""

    value = DataType._serie("large_serie", item)
    return new_field(LargeSerieField, name, value, nullable, metadata)


def large_serie_view(
    name: str,
    item: TypedField[Any, _ItemT] | Field,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> LargeSerieViewField[_ItemT]:
    """A serie of ``item`` viewed by 64-bit offsets and sizes: Arrow's ``large_list_view``."""

    value = DataType._serie("large_serie_view", item)
    return new_field(LargeSerieViewField, name, value, nullable, metadata)


__all__ = [
    "ChunkedSerie",
    "FixedSizeSerieField",
    "FixedSizeSerieSerie",
    "LargeSerieField",
    "LargeSerieSerie",
    "LargeSerieViewField",
    "LargeSerieViewSerie",
    "MapSerie",
    "Serie",
    "SerieField",
    "SerieReader",
    "SerieSerie",
    "SerieViewField",
    "SerieViewSerie",
    "StructSerie",
    "fixed_size_serie",
    "large_serie",
    "large_serie_view",
    "serie",
    "serie_view",
]
