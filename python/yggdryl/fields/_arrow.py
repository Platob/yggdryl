"""Exact native Field projections into plain Python dataclasses."""

from __future__ import annotations

import collections.abc as cabc
import dataclasses as dc
import datetime as dt
import enum
import keyword
import pathlib
import types
import typing
import uuid
from decimal import Decimal
from typing import Any

from .._native import DataType, Field as NativeField, Uri, Url, Urn
from ._classes import _PhysicalUnionValue, _adopt_materialized_schema

_IDENTITY_KEYS = (
    "python.module",
    "python.class",
    "python.qualname",
    "python.kind",
)
_INTEGER_KINDS = frozenset(
    ("int8", "int16", "int32", "int64", "uint8", "uint16", "uint32", "uint64")
)
_FLOAT_KINDS = frozenset(("float16", "float32", "float64"))
_BINARY_KINDS = frozenset(
    ("binary", "fixed_size_binary", "large_binary", "binary_view")
)
_STRING_KINDS = frozenset(("utf8", "large_utf8", "utf8_view"))
_LIST_KINDS = frozenset(
    ("list", "list_view", "fixed_size_list", "large_list", "large_list_view")
)
_DECIMAL_KINDS = frozenset(("decimal32", "decimal64", "decimal128", "decimal256"))
_RESERVED_NAMES = frozenset(
    {
        "field",
        "__dict__",
        "__slots__",
        "__weakref__",
        "__yggdryl_class_schema__",
        "__yggdryl_field_class__",
        "__yggdryl_value_fields__",
    }
)


def _valid_identifier(value: object) -> bool:
    return isinstance(value, str) and value.isidentifier() and not keyword.iskeyword(value)


def _valid_module(value: object) -> bool:
    return isinstance(value, str) and bool(value) and all(
        _valid_identifier(part) for part in value.split(".")
    )


def _select_identity(
    metadata: cabc.Mapping[str, str],
    *,
    name: str | None,
    module: str | None,
    root_name: str | None,
) -> tuple[str, str]:
    if name is not None and not _valid_identifier(name):
        raise TypeError(
            f"name {name!r} must be a valid non-keyword Python identifier"
        )
    if module is not None and not _valid_module(module):
        raise TypeError(f"module {module!r} must be a dotted Python identifier")
    selected_name = name
    if selected_name is None:
        candidate = metadata.get("python.class")
        if candidate is not None:
            if not _valid_identifier(candidate):
                raise TypeError(
                    f"python.class metadata {candidate!r} must be a valid "
                    "non-keyword Python identifier"
                )
            selected_name = candidate
    if selected_name is None:
        if not _valid_identifier(root_name):
            raise TypeError(
                f"field name {root_name!r} must be a valid non-keyword "
                "Python identifier"
            )
        selected_name = typing.cast(str, root_name)
    selected_module = module
    if selected_module is None:
        candidate = metadata.get("python.module")
        if candidate is not None:
            if not _valid_module(candidate):
                raise TypeError(
                    f"python.module metadata {candidate!r} must be a dotted "
                    "Python identifier"
                )
            selected_module = candidate
        else:
            selected_module = "__main__"
    return selected_name, selected_module


def _validate_column_names(fields: tuple[NativeField, ...]) -> None:
    seen: set[str] = set()
    for field in fields:
        name = field.name
        if not _valid_identifier(name):
            raise TypeError(
                f"Arrow column {name!r} must be a valid non-keyword Python identifier"
            )
        if (
            name in _RESERVED_NAMES
            or name.startswith("__")
            or name.startswith("__yggdryl_")
        ):
            raise TypeError(f"Arrow column {name!r} conflicts with the field-class API")
        if name in seen:
            raise TypeError(f"duplicate Arrow column name {name!r}")
        seen.add(name)


def _optional(hint: Any, nullable: bool) -> Any:
    if not nullable or hint is type(None):
        return hint
    return typing.Optional[hint]


def _union_hint(members: list[Any]) -> Any:
    unique: list[Any] = []
    for member in members:
        if member not in unique:
            unique.append(member)
    if not unique:
        return Any
    if len(unique) == 1:
        return unique[0]
    return typing.Union[tuple(unique)]


def _nested_class_name(owner_name: str, path: tuple[str, ...]) -> str:
    # Length-prefixed UTF-8 hex is collision-free and identifier-safe even
    # when Arrow container child names are not valid Python attributes.
    encoded = "_".join(
        f"{len(raw)}x{raw.hex()}" for part in path for raw in (part.encode("utf-8"),)
    )
    return f"{owner_name}_{encoded}_Field"


def _hint_from_datatype(
    data_type: DataType,
    *,
    module: str,
    owner_name: str,
    path: tuple[str, ...],
    materialize_schema: bool = True,
) -> Any:
    kind = data_type.id
    if kind == "null":
        return type(None)
    if kind == "boolean":
        return bool
    if kind in _INTEGER_KINDS:
        return int
    if kind in _FLOAT_KINDS:
        return float
    if kind in _DECIMAL_KINDS:
        return Decimal
    if kind == "timestamp":
        return dt.datetime
    if kind in ("date32", "date64"):
        return dt.date
    if kind in ("time32", "time64"):
        return dt.time
    if kind in ("duration32", "duration64"):
        return dt.timedelta
    if kind == "interval":
        return Any
    if kind in _BINARY_KINDS:
        return bytes
    if kind in _STRING_KINDS:
        return str
    if kind in _LIST_KINDS:
        child = data_type[0]
        item = _hint_from_field(
            child,
            module=module,
            owner_name=owner_name,
            path=(*path, child.name),
            materialize_schema=materialize_schema,
        )
        return types.GenericAlias(list, item)
    if kind == "struct":
        nested_name = _nested_class_name(owner_name, path)
        fields = tuple(data_type)
        if not materialize_schema:
            try:
                _validate_column_names(fields)
            except TypeError:
                hints = {
                    field.name: _hint_from_field(
                        field,
                        module=module,
                        owner_name=nested_name,
                        path=(*path, field.name),
                        materialize_schema=False,
                    )
                    for field in fields
                }
                fallback = typing.TypedDict(nested_name, hints)  # type: ignore[misc]
                fallback.__module__ = module
                fallback.__qualname__ = nested_name
                return fallback
        nested_root = NativeField(
            nested_name,
            data_type,
            nullable=False,
        )
        if materialize_schema:
            return _materialize_dataclass(
                nested_root,
                fields,
                class_name=nested_name,
                module=module,
            )
        return _materialize_lazy_dataclass_hint(
            fields,
            class_name=nested_name,
            module=module,
        )
    if kind == "union":
        return _union_hint(
            [
                _hint_from_field(
                    child,
                    module=module,
                    owner_name=owner_name,
                    path=(*path, child.name),
                    materialize_schema=materialize_schema,
                )
                for child in data_type
            ]
        )
    if kind == "map":
        entries = data_type[0].data_type
        key = _hint_from_field(
            entries[0],
            module=module,
            owner_name=owner_name,
            path=(*path, "key"),
            materialize_schema=materialize_schema,
        )
        item = _hint_from_field(
            entries[1],
            module=module,
            owner_name=owner_name,
            path=(*path, "value"),
            materialize_schema=materialize_schema,
        )
        if entries[0].data_type.is_nested:
            pair = types.GenericAlias(tuple, (key, item))
            return types.GenericAlias(list, pair)
        return types.GenericAlias(dict, (key, item))
    if kind == "run_end_encoded":
        return _hint_from_datatype(
            data_type[1].data_type,
            module=module,
            owner_name=owner_name,
            path=(*path, "values"),
            materialize_schema=materialize_schema,
        )
    if kind == "dictionary":
        return _hint_from_datatype(
            data_type._dictionary_value_type(),
            module=module,
            owner_name=owner_name,
            path=(*path, "dictionary"),
            materialize_schema=materialize_schema,
        )
    return Any


def _hint_from_field(
    field: NativeField,
    *,
    module: str,
    owner_name: str,
    path: tuple[str, ...],
    materialize_schema: bool = True,
) -> Any:
    if materialize_schema and field.data_type.id == "struct":
        nested_name = _nested_class_name(owner_name, path)
        # A nested non-null Struct can retain its exact source Field. For a
        # nullable member, Optional carries the parent's nullability while the
        # class itself owns the same present-value shape as every dataclass.
        root = field
        if field.nullable:
            root = NativeField(
                field.name,
                field.data_type,
                nullable=False,
                metadata=dict(field.metadata.items()),
            )
        hint = _materialize_dataclass(
            root,
            tuple(field.data_type),
            class_name=nested_name,
            module=module,
        )
        return _optional(hint, field.nullable)
    hint = _hint_from_datatype(
        field.data_type,
        module=module,
        owner_name=owner_name,
        path=path,
        materialize_schema=materialize_schema,
    )
    return _optional(hint, field.nullable)


def _dataclass_members(
    fields: tuple[NativeField, ...], hints: cabc.Mapping[str, Any]
) -> list[tuple[Any, ...]]:
    members: list[tuple[Any, ...]] = []
    for child in fields:
        if child.nullable:
            members.append((child.name, hints[child.name], dc.field(default=None)))
        else:
            members.append((child.name, hints[child.name]))
    return members


def _materialize_lazy_dataclass_hint(
    fields: tuple[NativeField, ...],
    *,
    class_name: str,
    module: str,
) -> type[Any]:
    """Build an exact class hint without importing or projecting through Arrow."""

    _validate_column_names(fields)
    hints = {
        child.name: _hint_from_field(
            child,
            module=module,
            owner_name=class_name,
            path=(child.name,),
            materialize_schema=False,
        )
        for child in fields
    }
    generated = dc.make_dataclass(
        class_name,
        _dataclass_members(fields, hints),
        namespace={"__module__": module},
        kw_only=True,
        slots=True,
    )
    generated.__module__ = module
    generated.__qualname__ = class_name
    root = NativeField(
        class_name,
        DataType.from_fields(fields),
        nullable=False,
        metadata={
            "python.module": module,
            "python.class": class_name,
            "python.qualname": class_name,
            "python.kind": "field",
        },
    )
    _adopt_materialized_schema(generated, root, fields, hints)
    return generated


def _adopt_dataclass_schema(
    generated: type[Any],
    root: NativeField,
    fields: tuple[NativeField, ...],
    hints: cabc.Mapping[str, Any],
) -> None:
    """Attach one exact native Field graph to a generated dataclass."""

    _adopt_materialized_schema(generated, root, fields, hints)


def _materialize_dataclass(
    root: NativeField,
    fields: tuple[NativeField, ...],
    *,
    class_name: str,
    module: str,
) -> type[Any]:
    _validate_column_names(fields)
    hints = {
        child.name: _hint_from_field(
            child,
            module=module,
            owner_name=class_name,
            path=(child.name,),
        )
        for child in fields
    }
    namespace: dict[str, Any] = {"__module__": module}
    description = root.metadata.get("description")
    if description:
        namespace["__doc__"] = description
    generated = dc.make_dataclass(
        class_name,
        _dataclass_members(fields, hints),
        namespace=namespace,
        kw_only=True,
        slots=True,
    )
    generated.__module__ = module
    generated.__qualname__ = class_name
    _adopt_dataclass_schema(generated, root, fields, hints)
    return generated


def dataclass_from_field(
    value: NativeField,
    *,
    name: str | None = None,
    module: str | None = None,
) -> type[Any]:
    """Build a dataclass whose cached ``field()`` is exactly ``value``."""

    if value.nullable or value.data_type.id != "struct":
        raise TypeError("into_dataclass requires a non-nullable Struct Field")
    metadata = dict(value.metadata.items())
    selected_name, selected_module = _select_identity(
        metadata,
        name=name,
        module=module,
        root_name=value.name,
    )
    return _materialize_dataclass(
        value,
        tuple(value.data_type),
        class_name=selected_name,
        module=selected_module,
    )


class _TypePlan(typing.NamedTuple):
    type_id: str
    arrow_type: Any
    children: tuple[_TypePlan, ...]
    map_as_pairs: bool


def _prepare_type_plan(data_type: DataType, arrow_type: Any) -> _TypePlan:
    storage_type = getattr(arrow_type, "storage_type", None)
    if storage_type is not None:
        return _TypePlan(
            "extension",
            arrow_type,
            (_prepare_type_plan(data_type, storage_type),),
            False,
        )
    kind = data_type.id
    if kind == "dictionary":
        return _TypePlan(
            kind,
            arrow_type,
            (
                _prepare_type_plan(
                    data_type._dictionary_value_type(), arrow_type.value_type
                ),
            ),
            False,
        )
    native_children = tuple(data_type)
    children = tuple(
        _prepare_type_plan(child.data_type, arrow_type.field(index).type)
        for index, child in enumerate(native_children)
    )
    map_as_pairs = (
        kind == "map"
        and bool(native_children)
        and native_children[0].data_type[0].data_type.is_nested
    )
    return _TypePlan(kind, arrow_type, children, map_as_pairs)


def _arrow_scalar_value(
    scalar: Any,
    plan: _TypePlan,
    *,
    path: str,
    preserve_union_branch: bool = False,
) -> Any:
    kind = plan.type_id
    if not scalar.is_valid:
        if kind == "union" and preserve_union_branch:
            try:
                index = tuple(plan.arrow_type.type_codes).index(scalar.type_code)
            except ValueError as error:  # pragma: no cover - Arrow validates type codes
                raise TypeError(
                    f"{path}: unknown Arrow union type code {scalar.type_code}"
                ) from error
            return _PhysicalUnionValue(index, None)
        return None
    if kind == "extension":
        storage = getattr(scalar, "value", None)
        if storage is not None and hasattr(storage, "is_valid"):
            return _arrow_scalar_value(
                storage,
                plan.children[0],
                path=path,
                preserve_union_branch=preserve_union_branch,
            )
        return scalar.as_py()
    if kind in _LIST_KINDS:
        return [
            _arrow_scalar_value(
                item,
                plan.children[0],
                path=f"{path}[{index}]",
                preserve_union_branch=preserve_union_branch,
            )
            for index, item in enumerate(scalar.values)
        ]
    if kind == "struct":
        return {
            field.name: _arrow_scalar_value(
                scalar[index],
                plan.children[index],
                path=f"{path}.{field.name}",
                preserve_union_branch=preserve_union_branch,
            )
            for index, field in enumerate(plan.arrow_type)
        }
    if kind == "map":
        entries_plan = plan.children[0]
        key_plan, value_plan = entries_plan.children
        normalized: dict[Any, Any] = {}
        normalized_pairs: list[tuple[Any, Any]] = []
        previous: Any = None
        has_previous = False
        for index, entry in enumerate(scalar.values):
            key = _arrow_scalar_value(
                entry[0],
                key_plan,
                path=f"{path}.keys[{index}]",
                preserve_union_branch=preserve_union_branch,
            )
            if plan.map_as_pairs:
                duplicate = any(
                    _map_keys_equal(key, previous_key)
                    for previous_key, _ in normalized_pairs
                )
            else:
                try:
                    duplicate = key in normalized
                except TypeError as error:
                    raise TypeError(
                        f"{path}.keys[{index}]: Arrow map key is not hashable"
                    ) from error
            if duplicate:
                raise ValueError(f"{path}: duplicate Arrow map key {key!r}")
            if plan.arrow_type.keys_sorted and has_previous and not plan.map_as_pairs:
                try:
                    unsorted = key < previous
                except TypeError as error:
                    raise TypeError(
                        f"{path}: keys_sorted map keys must be mutually orderable"
                    ) from error
                if unsorted:
                    raise ValueError(
                        f"{path}: Arrow map keys are not sorted for keys_sorted type"
                    )
            previous = key
            has_previous = True
            item = _arrow_scalar_value(
                entry[1],
                value_plan,
                path=f"{path}[{key!r}]",
                preserve_union_branch=preserve_union_branch,
            )
            if plan.map_as_pairs:
                normalized_pairs.append((key, item))
            else:
                normalized[key] = item
        return normalized_pairs if plan.map_as_pairs else normalized
    if kind == "union":
        try:
            index = tuple(plan.arrow_type.type_codes).index(scalar.type_code)
        except ValueError as error:  # pragma: no cover - Arrow validates type codes
            raise TypeError(f"{path}: unknown Arrow union type code {scalar.type_code}") from error
        converted = _arrow_scalar_value(
            scalar.value,
            plan.children[index],
            path=path,
            preserve_union_branch=preserve_union_branch,
        )
        return (
            _PhysicalUnionValue(index, converted)
            if preserve_union_branch
            else converted
        )
    if kind == "dictionary":
        decoded = getattr(scalar, "value", None)
        if decoded is not None and hasattr(decoded, "is_valid"):
            return _arrow_scalar_value(
                decoded,
                plan.children[0],
                path=path,
                preserve_union_branch=preserve_union_branch,
            )
        return scalar.as_py()
    if kind == "run_end_encoded":
        decoded = getattr(scalar, "value", None)
        if decoded is not None and hasattr(decoded, "is_valid"):
            return _arrow_scalar_value(
                decoded,
                plan.children[1],
                path=path,
                preserve_union_branch=preserve_union_branch,
            )
        return scalar.as_py()
    return scalar.as_py()


def _map_keys_equal(left: Any, right: Any) -> bool:
    try:
        return bool(left == right)
    except (TypeError, ValueError):
        return False


__all__: tuple[str, ...] = ()
