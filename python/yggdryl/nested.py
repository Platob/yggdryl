"""Nested, encoded, and collection field factories.

The serie layouts have their own module: :mod:`yggdryl.serie` holds
``serie``, ``large_serie``, ``serie_view``, ``large_serie_view`` and
``fixed_size_serie``.
"""

from __future__ import annotations

from collections.abc import Iterable, Mapping
from typing import TYPE_CHECKING, Any, Literal, TypeAlias, TypeVar, cast

from ._native import DataType, Field
from ._common import (
    DataTypeInput,
    MetadataInput,
    new_field,
)
from ._typing import TypedField

_KeyT = TypeVar("_KeyT")
_ValueT = TypeVar("_ValueT")

if TYPE_CHECKING:
    StructField: TypeAlias = TypedField[Literal["struct"], object]
    UnionField: TypeAlias = TypedField[Literal["union"], object]
    DenseUnionField: TypeAlias = UnionField
    DictionaryField: TypeAlias = TypedField[Literal["dictionary"], _ValueT]
    MapField: TypeAlias = TypedField[
        Literal["map"], Mapping[_KeyT, _ValueT]
    ]
    RunEndEncodedField: TypeAlias = TypedField[
        Literal["run_end_encoded"], _ValueT | None
    ]
else:
    StructField = UnionField = Field
    DenseUnionField = Field
    DictionaryField = MapField = RunEndEncodedField = Field


def struct(
    name: str,
    fields: Iterable[Field],
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> StructField:
    value = DataType.from_fields(fields)
    return new_field(StructField, name, value, nullable, metadata)


def union(
    name: str,
    fields: Iterable[tuple[int, Field]],
    mode: str = "sparse",
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> UnionField:
    value = DataType._union(fields, mode)
    return new_field(UnionField, name, value, nullable, metadata)


def dense_union(
    name: str,
    members: Iterable[Field],
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> DenseUnionField:
    """Create the canonical dense Union with sequential native type IDs."""

    value = DataType.variant(members)
    return new_field(DenseUnionField, name, value, nullable, metadata)


def dictionary(
    name: str,
    key: DataTypeInput,
    value: DataTypeInput,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> DictionaryField[Any]:
    nested = DataType._dictionary(key, value)
    return new_field(DictionaryField, name, nested, nullable, metadata)


def map(
    name: str,
    entries: Field,
    *,
    keys_sorted: bool = False,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> MapField[Any, Any]:
    value = DataType._map(entries, keys_sorted)
    return new_field(MapField, name, value, nullable, metadata)


def map_of(
    name: str,
    key: DataTypeInput,
    value: DataTypeInput,
    *,
    keys_sorted: bool = False,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> MapField[Any, Any]:
    entries = struct(
        "entries",
        [
            Field("key", key, nullable=False),
            Field("value", value, nullable=True),
        ],
        nullable=False,
    )
    return map(
        name,
        cast(Field, entries),
        keys_sorted=keys_sorted,
        nullable=nullable,
        metadata=metadata,
    )


def run_end_encoded(
    name: str,
    run_ends: Field,
    values: TypedField[Any, _ValueT] | Field,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> RunEndEncodedField[_ValueT]:
    value = DataType._run_end_encoded(run_ends, values)
    return new_field(RunEndEncodedField, name, value, nullable, metadata)


__all__ = [
    "DenseUnionField",
    "DictionaryField",
    "MapField",
    "RunEndEncodedField",
    "StructField",
    "UnionField",
    "dense_union",
    "dictionary",
    "map",
    "map_of",
    "run_end_encoded",
    "struct",
    "union",
]
