"""Nested, encoded, and collection field factories."""

from __future__ import annotations

import builtins
from collections.abc import Iterable, Mapping
from typing import TYPE_CHECKING, Any, Literal, TypeAlias, TypeVar, cast

from .._native import DataType, Field
from ._common import (
    DataTypeInput,
    MetadataInput,
    new_field,
)
from ._typing import TypedField

if TYPE_CHECKING:
    from ..records import Record

_ItemT = TypeVar("_ItemT")
_KeyT = TypeVar("_KeyT")
_ValueT = TypeVar("_ValueT")

if TYPE_CHECKING:
    ListField: TypeAlias = TypedField[
        Literal["list"], builtins.list[_ItemT]
    ]
    ListViewField: TypeAlias = TypedField[
        Literal["list_view"], builtins.list[_ItemT]
    ]
    FixedSizeListField: TypeAlias = TypedField[
        Literal["fixed_size_list"], builtins.list[_ItemT | None]
    ]
    LargeListField: TypeAlias = TypedField[
        Literal["large_list"], builtins.list[_ItemT]
    ]
    LargeListViewField: TypeAlias = TypedField[
        Literal["large_list_view"], builtins.list[_ItemT]
    ]
    StructField: TypeAlias = TypedField[
        Literal["struct"], Record | Mapping[str, object]
    ]
    UnionField: TypeAlias = TypedField[Literal["union"], object]
    VariantField: TypeAlias = UnionField
    DictionaryField: TypeAlias = TypedField[Literal["dictionary"], _ValueT]
    MapField: TypeAlias = TypedField[
        Literal["map"], Mapping[_KeyT, _ValueT]
    ]
    RunEndEncodedField: TypeAlias = TypedField[
        Literal["run_end_encoded"], _ValueT | None
    ]
else:
    ListField = ListViewField = FixedSizeListField = Field
    LargeListField = LargeListViewField = StructField = UnionField = Field
    VariantField = Field
    DictionaryField = MapField = RunEndEncodedField = Field


def list(
    name: str,
    item: TypedField[Any, _ItemT] | Field,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> ListField[_ItemT]:
    value = DataType._list("list", item)
    return new_field(ListField, name, value, nullable, metadata)


def list_view(
    name: str,
    item: TypedField[Any, _ItemT] | Field,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> ListViewField[_ItemT]:
    value = DataType._list("list_view", item)
    return new_field(ListViewField, name, value, nullable, metadata)


def fixed_size_list(
    name: str,
    item: TypedField[Any, _ItemT] | Field,
    length: int,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> FixedSizeListField[_ItemT]:
    value = DataType._list("fixed_size_list", item, length)
    return new_field(FixedSizeListField, name, value, nullable, metadata)


def large_list(
    name: str,
    item: TypedField[Any, _ItemT] | Field,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> LargeListField[_ItemT]:
    value = DataType._list("large_list", item)
    return new_field(LargeListField, name, value, nullable, metadata)


def large_list_view(
    name: str,
    item: TypedField[Any, _ItemT] | Field,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> LargeListViewField[_ItemT]:
    value = DataType._list("large_list_view", item)
    return new_field(LargeListViewField, name, value, nullable, metadata)


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


def variant(
    name: str,
    members: Iterable[Field],
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> VariantField:
    """Create the canonical dense Union with sequential native type IDs."""

    value = DataType.variant(members)
    return new_field(VariantField, name, value, nullable, metadata)


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
    "DictionaryField",
    "FixedSizeListField",
    "LargeListField",
    "LargeListViewField",
    "ListField",
    "ListViewField",
    "MapField",
    "RunEndEncodedField",
    "StructField",
    "UnionField",
    "VariantField",
    "dictionary",
    "fixed_size_list",
    "large_list",
    "large_list_view",
    "list",
    "list_view",
    "map",
    "map_of",
    "run_end_encoded",
    "struct",
    "union",
    "variant",
]
