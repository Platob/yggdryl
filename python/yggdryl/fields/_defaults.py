"""Cached Python projections of native canonical schema defaults."""

from __future__ import annotations

import dataclasses as dc
import functools
import hashlib
import itertools
import types
import typing
from typing import Any

from .._native import DataType, Field as NativeField
from ._arrow import (
    _adopt_dataclass_schema,
    _arrow_scalar_value,
    _hint_from_datatype,
    _hint_from_field,
    _prepare_type_plan,
    _validate_column_names,
)
from ._classes import _convert


class _DataTypeLayoutKey:
    """Hashable native layout identity that ignores Field metadata recursively."""

    __slots__ = ("data_type",)

    def __init__(self, data_type: DataType) -> None:
        self.data_type = data_type

    def __hash__(self) -> int:
        # Equal native layouts necessarily share their root kind and child
        # count. This hash is intentionally coarse because the binding exposes
        # no recursively metadata-free DataType hash; exact structural
        # equality remains delegated to the core below. Same-arity collisions
        # are therefore harmless and are covered by the cache regressions.
        return hash((self.data_type.id, len(self.data_type)))

    def __eq__(self, other: object) -> bool:
        if not isinstance(other, _DataTypeLayoutKey):
            return NotImplemented
        return self.data_type.equals(other.data_type, with_metadata=False)


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


def _metadata_free_data_type(data_type: DataType) -> DataType:
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

    return DataType.from_dict(strip(data_type.into_dict()))


@functools.cache
def _datatype_hint_cached(layout: _DataTypeLayoutKey) -> Any:
    data_type = _metadata_free_data_type(layout.data_type)
    return _hint_from_datatype(
        data_type,
        module=__name__,
        owner_name=_cache_name("Default", layout),
        path=("value",),
        materialize_schema=False,
    )


def _datatype_hint(data_type: DataType) -> Any:
    return _datatype_hint_cached(_DataTypeLayoutKey(data_type))


@functools.cache
def _field_hint_cached(layout: _DataTypeLayoutKey, nullable: bool) -> Any:
    # Name and metadata intentionally do not participate in Python hint
    # identity. The physical Field remains authoritative for value conversion.
    data_type = _metadata_free_data_type(layout.data_type)
    field = NativeField("value", data_type, nullable=nullable)
    return _hint_from_field(
        field,
        module=__name__,
        owner_name=_cache_name("Default", layout, nullable),
        path=("value",),
        materialize_schema=False,
    )


def _field_hint(data_type: DataType, nullable: bool) -> Any:
    return _field_hint_cached(_DataTypeLayoutKey(data_type), nullable)


@functools.cache
def _conversion_owner(data_type: DataType, nullable: bool) -> type[Any]:
    """Own nested hint resolution without compiling a parallel value schema."""

    hint = _field_hint(data_type, nullable)
    name = _cache_name(
        "DefaultOwner", _DataTypeLayoutKey(data_type), nullable
    )
    generated = dc.make_dataclass(
        name,
        [("value", hint)],
        namespace={"__module__": __name__},
        slots=True,
    )
    generated.__module__ = __name__
    generated.__qualname__ = name
    return generated


def _struct_fields(field: NativeField) -> typing.Iterator[NativeField]:
    data_type = field.data_type
    kind = data_type.id
    if kind == "struct":
        children = tuple(data_type)
        try:
            _validate_column_names(children)
        except TypeError:
            pass
        else:
            yield field
        for child in children:
            yield from _struct_fields(child)
    elif kind in ("list", "list_view", "fixed_size_list", "large_list", "large_list_view"):
        yield from _struct_fields(data_type[0])
    elif kind == "union":
        for child in data_type:
            yield from _struct_fields(child)
    elif kind == "map":
        entries = data_type[0].data_type
        for child in entries:
            yield from _struct_fields(child)
    elif kind == "dictionary":
        yield from _struct_fields(
            NativeField(
                "dictionary",
                data_type._dictionary_value_type(),
                nullable=False,
            )
        )
    elif kind == "run_end_encoded":
        yield from _struct_fields(data_type[1])


def _dataclass_classes(hint: Any) -> typing.Iterator[type[Any]]:
    origin = typing.get_origin(hint)
    if origin in (typing.Union, types.UnionType):
        for member in typing.get_args(hint):
            if member is not type(None):
                yield from _dataclass_classes(member)
        return
    if isinstance(hint, type) and dc.is_dataclass(hint):
        yield hint
        for field in dc.fields(hint):
            yield from _dataclass_classes(field.type)
        return
    if typing.is_typeddict(hint):
        for member in hint.__annotations__.values():
            yield from _dataclass_classes(member)
        return
    for member in typing.get_args(hint):
        yield from _dataclass_classes(member)


def _replace_dataclass_classes(
    hint: Any, replacements: typing.Mapping[type[Any], type[Any]]
) -> Any:
    if isinstance(hint, type) and hint in replacements:
        return replacements[hint]
    if typing.is_typeddict(hint):
        replaced_fields = {
            name: _replace_dataclass_classes(member, replacements)
            for name, member in hint.__annotations__.items()
        }
        fallback = typing.TypedDict(  # type: ignore[misc]
            f"{hint.__name__}_Value",
            replaced_fields,
            total=hint.__total__,
        )
        fallback.__module__ = hint.__module__
        fallback.__qualname__ = f"{hint.__qualname__}_Value"
        return fallback
    arguments = typing.get_args(hint)
    if not arguments:
        return hint
    replaced_arguments = tuple(
        _replace_dataclass_classes(argument, replacements)
        for argument in arguments
    )
    origin = typing.get_origin(hint)
    if origin in (typing.Union, types.UnionType):
        return typing.Union[replaced_arguments]
    if isinstance(origin, type):
        return types.GenericAlias(origin, replaced_arguments)
    return hint


@functools.cache
def _value_hint(field: NativeField, hint: Any) -> Any:
    """Build exact value classes without mutating metadata-free hint classes."""

    native = tuple(_struct_fields(field))
    classes = tuple(_dataclass_classes(hint))
    if len(native) != len(classes):
        raise TypeError(
            "native Struct defaults and generated dataclass hints have different layouts"
        )
    replacements: dict[type[Any], type[Any]] = {}
    identity = hashlib.blake2b(field.into_json().encode(), digest_size=8).hexdigest()
    # Create exact subclasses from children upward. The public cached hint
    # classes remain metadata-free and therefore cannot be poisoned by two
    # same-layout Fields with different names or metadata.
    for index, (native_field, hint_type) in reversed(
        tuple(enumerate(zip(native, classes)))
    ):
        name = f"DefaultValue_{identity}_{index}_Field"
        generated = dc.make_dataclass(
            name,
            [],
            bases=(hint_type,),
            namespace={"__module__": __name__},
            slots=True,
        )
        generated.__module__ = __name__
        generated.__qualname__ = name
        value_type = generated
        replacements[hint_type] = value_type
        hints = {
            child.name: _replace_dataclass_classes(child.type, replacements)
            for child in dc.fields(typing.cast(Any, hint_type))
        }
        # A generated dataclass root is intrinsically present. Its parent
        # Field retains the original nullability in the enclosing exact
        # schema; all other Field state remains intact.
        root = NativeField(
            native_field.name,
            native_field.data_type,
            nullable=False,
            metadata=dict(native_field.metadata.items()),
        )
        _adopt_dataclass_schema(
            value_type,
            root,
            tuple(native_field.data_type),
            hints,
        )
    return _replace_dataclass_classes(hint, replacements)


def _default_pyhint_from_datatype(data_type: DataType) -> Any:
    """Return the cached non-nullable Python hint for a native datatype."""

    return _datatype_hint(data_type)


def _default_pyhint_from_field(field: NativeField) -> Any:
    """Return the cached Python hint while honoring only Field nullability."""

    return _field_hint(field.data_type, field.nullable)


def _convert_default(
    field: NativeField,
    scalar: Any,
    hint: Any,
) -> Any:
    plan = _prepare_type_plan(field.data_type, scalar.type)
    value = _arrow_scalar_value(
        scalar, plan, path="$", preserve_union_branch=True
    )
    # DataType(null) intentionally projects a typed null through its synthetic
    # non-nullable root. Nullable Fields also stop here, after the core alone
    # has decided that their canonical value is null.
    if value is None:
        return None
    hint = _value_hint(field, hint)
    owner = _conversion_owner(field.data_type, field.nullable)
    return _convert(
        value,
        hint,
        owner,
        "$",
        "raise",
        physical_field=field,
    )


def _default_pyvalue_from_datatype(data_type: DataType, scalar: Any) -> Any:
    field = NativeField("value", data_type, nullable=False)
    return _convert_default(field, scalar, _datatype_hint(data_type))


def _default_pyvalue_from_field(field: NativeField, scalar: Any) -> Any:
    return _convert_default(
        field,
        scalar,
        _field_hint(field.data_type, field.nullable),
    )


__all__: tuple[str, ...] = ()
