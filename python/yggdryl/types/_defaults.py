"""Cached Python annotations for native canonical schema defaults.

A default *value* is a :class:`yggdryl.Scalar` and is answered as one; this
module owns only the *hint* - the Python annotation a datatype or field
projects - which is a type, not a value, and so has no Scalar to be.
"""

from __future__ import annotations

import functools
import itertools
from typing import Any

from .._native import DataType, Field as NativeField
from ._arrow import _hint_from_datatype, _hint_from_field


class _DataTypeLayoutKey:
    """Hashable native layout identity that ignores Field metadata recursively."""

    __slots__ = ("dtype",)

    def __init__(self, dtype: DataType) -> None:
        self.dtype = dtype

    def __hash__(self) -> int:
        # Equal native layouts necessarily share their root kind and child
        # count. This hash is intentionally coarse because the binding exposes
        # no recursively metadata-free DataType hash; exact structural
        # equality remains delegated to the core below. Same-arity collisions
        # are therefore harmless and are covered by the cache regressions.
        return hash((self.dtype.id, len(self.dtype)))

    def __eq__(self, other: object) -> bool:
        if not isinstance(other, _DataTypeLayoutKey):
            return NotImplemented
        return self.dtype.equals(other.dtype, with_metadata=False)


_LAYOUT_IDS = itertools.count()


@functools.cache
def _layout_id(layout: _DataTypeLayoutKey) -> int:
    """Intern one process-local identifier for each native physical layout."""

    return next(_LAYOUT_IDS)


def _cache_name(
    prefix: str, layout: _DataTypeLayoutKey, nullable: bool = False
) -> str:
    identity = _layout_id(layout)
    suffix = "Nullable" if nullable else "Required"
    return f"{prefix}_{identity:016x}_{suffix}"


def _metadata_free_dtype(dtype: DataType) -> DataType:
    """Clone one exact native layout while removing every child Field's metadata."""

    def strip(value: Any) -> Any:
        if isinstance(value, dict):
            return {
                key: ({} if key == "metadata" else strip(member))
                for key, member in value.items()
            }
        if isinstance(value, list):
            return [strip(member) for member in value]
        return value

    return DataType.from_dict(strip(dtype.into_dict()))


@functools.cache
def _datatype_hint_cached(layout: _DataTypeLayoutKey) -> Any:
    dtype = _metadata_free_dtype(layout.dtype)
    return _hint_from_datatype(
        dtype,
        module=__name__,
        owner_name=_cache_name("Default", layout),
        path=("value",),
        materialize_schema=False,
    )


def _datatype_hint(dtype: DataType) -> Any:
    return _datatype_hint_cached(_DataTypeLayoutKey(dtype))


@functools.cache
def _field_hint_cached(layout: _DataTypeLayoutKey, nullable: bool) -> Any:
    # Name and metadata intentionally do not participate in Python hint
    # identity. The physical Field remains authoritative for value conversion.
    dtype = _metadata_free_dtype(layout.dtype)
    field = NativeField("value", dtype, nullable=nullable)
    return _hint_from_field(
        field,
        module=__name__,
        owner_name=_cache_name("Default", layout, nullable),
        path=("value",),
        materialize_schema=False,
    )


def _field_hint(dtype: DataType, nullable: bool) -> Any:
    return _field_hint_cached(_DataTypeLayoutKey(dtype), nullable)


def _default_pyhint_from_datatype(dtype: DataType) -> Any:
    """Return the cached non-nullable Python hint for a native datatype."""

    return _datatype_hint(dtype)


def _default_pyhint_from_field(field: NativeField) -> Any:
    """Return the cached Python hint while honoring only Field nullability."""

    return _field_hint(field.dtype, field.nullable)


__all__: tuple[str, ...] = ()
