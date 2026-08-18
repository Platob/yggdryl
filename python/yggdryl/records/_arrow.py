"""Arrow-backed record class materialization and bounded row adapters."""

from __future__ import annotations

import collections.abc as cabc
import copy
import dataclasses as dc
import datetime as dt
import enum
import functools
import keyword
import pathlib
import types
import typing
import uuid
from decimal import Decimal
from typing import Any

from .._native import DataType, Field as SchemaField, Uri, Url, Urn
from ._records import (
    _PhysicalUnionValue,
    Record,
    _ScopeToken,
    _adopt_materialized_schema,
    _check_options,
    _convert,
    _ensure_schema,
    _export,
    _from_dict,
    _has_default,
    _install_methods,
    _project_record_values,
)

_IDENTITY_KEYS = (
    "python.module",
    "python.class",
    "python.qualname",
    "python.kind",
)
_EXTENSION_KEYS = (
    "ARROW:extension:name",
    "ARROW:extension:metadata",
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
_RESERVED_NAMES = frozenset(dir(Record)) | frozenset(
    {
        "__dict__",
        "__slots__",
        "__weakref__",
        "__yggdryl_field__",
        "__yggdryl_fields__",
        "__yggdryl_record__",
        "__yggdryl_schema__",
        "__yggdryl_value_fields__",
    }
)
_MISSING = object()


class _OutputShapeError(ValueError):
    """A fully path-qualified Arrow output validation failure."""


@functools.cache
def _pyarrow() -> Any:
    try:
        import pyarrow as pa  # type: ignore[import-untyped]
    except ImportError as error:  # pragma: no cover - declared dependency
        raise RuntimeError("Arrow record interoperability requires pyarrow") from error
    return pa


def _decode_metadata(
    metadata: cabc.Mapping[bytes, bytes] | None, *, context: str
) -> dict[str, str]:
    if not metadata:
        return {}
    decoded: dict[str, str] = {}
    for raw_key, raw_value in metadata.items():
        try:
            key = bytes(raw_key).decode("utf-8")
            value = bytes(raw_value).decode("utf-8")
        except UnicodeDecodeError as error:
            raise TypeError(f"{context} metadata must contain UTF-8 keys and values") from error
        decoded[key] = value
    return decoded


def _valid_identifier(value: object) -> bool:
    return isinstance(value, str) and value.isidentifier() and not keyword.iskeyword(value)


def _valid_module(value: object) -> bool:
    return isinstance(value, str) and bool(value) and all(
        _valid_identifier(part) for part in value.split(".")
    )


def _select_identity(
    metadata: cabc.Mapping[str, str],
    *,
    class_name: str | None,
    module: str | None,
    root_name: str | None,
) -> tuple[str, str]:
    if class_name is not None and not _valid_identifier(class_name):
        raise TypeError("class_name must be a valid non-keyword Python identifier")
    if module is not None and not _valid_module(module):
        raise TypeError("module must be a dotted Python identifier")
    selected_name = class_name
    if selected_name is None:
        candidate = metadata.get("python.class")
        selected_name = candidate if _valid_identifier(candidate) else None
    if selected_name is None and _valid_identifier(root_name):
        selected_name = typing.cast(str, root_name)
    if selected_name is None:
        selected_name = "ArrowRecord"
    selected_module = module
    if selected_module is None:
        candidate = metadata.get("python.module")
        selected_module = candidate if _valid_module(candidate) else "__main__"
    return selected_name, typing.cast(str, selected_module)


def _validate_column_names(fields: tuple[SchemaField, ...]) -> None:
    seen: set[str] = set()
    for field in fields:
        name = field.name
        if not _valid_identifier(name):
            raise TypeError(
                f"Arrow column {name!r} must be a valid non-keyword Python identifier"
            )
        if name in _RESERVED_NAMES or name.startswith("__yggdryl_"):
            raise TypeError(f"Arrow column {name!r} conflicts with a reserved record API")
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
    return f"{owner_name}_{encoded}_Record"


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
    if kind == "duration":
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
        nested_root = SchemaField(
            nested_name,
            data_type,
            nullable=False,
        )
        if materialize_schema:
            return _materialize_record(
                nested_root,
                fields,
                class_name=nested_name,
                module=module,
                schema_metadata=None,
                preserve_root=False,
            )
        return _materialize_lazy_record_hint(
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
    field: SchemaField,
    *,
    module: str,
    owner_name: str,
    path: tuple[str, ...],
    materialize_schema: bool = True,
) -> Any:
    hint = _hint_from_datatype(
        field.data_type,
        module=module,
        owner_name=owner_name,
        path=path,
        materialize_schema=materialize_schema,
    )
    return _optional(hint, field.nullable)


def _materialize_lazy_record_hint(
    fields: tuple[SchemaField, ...],
    *,
    class_name: str,
    module: str,
) -> type[Record]:
    """Build a cached Record hint without touching PyArrow or an Arrow schema.

    The native schema remains lazy until an operation actually needs one. This
    keeps ``default_pyhint`` a pure Python-type projection while the inherited
    Record methods can still compile and cache their normal schema on demand.
    """

    _validate_column_names(fields)
    hints = {
        field.name: _hint_from_field(
            field,
            module=module,
            owner_name=class_name,
            path=(field.name,),
            materialize_schema=False,
        )
        for field in fields
    }
    generated = dc.make_dataclass(
        class_name,
        [(field.name, hints[field.name]) for field in fields],
        bases=(Record,),
        namespace={"__module__": module, "__yggdryl_record__": True},
        slots=True,
    )
    generated.__module__ = module
    generated.__qualname__ = class_name
    return typing.cast(type[Record], generated)


def _materialize_record(
    imported_root: SchemaField,
    fields: tuple[SchemaField, ...],
    *,
    class_name: str,
    module: str,
    schema_metadata: cabc.Mapping[str, str] | None,
    preserve_root: bool,
) -> type[Record]:
    _validate_column_names(fields)
    hints = {
        field.name: _hint_from_field(
            field,
            module=module,
            owner_name=class_name,
            path=(field.name,),
        )
        for field in fields
    }
    generated = dc.make_dataclass(
        class_name,
        [(field.name, hints[field.name]) for field in fields],
        bases=(Record,),
        namespace={"__module__": module},
        slots=True,
    )
    # Python 3.14's make_dataclass assigns its caller module after applying
    # namespace; restore the selected durable identity explicitly.
    generated.__module__ = module
    generated.__qualname__ = class_name
    _adopt_record_schema(
        generated,
        imported_root,
        fields,
        hints,
        class_name=class_name,
        module=module,
        schema_metadata=schema_metadata,
        preserve_root=preserve_root,
    )
    _install_methods(generated, None, _ScopeToken())
    return typing.cast(type[Record], generated)


def _adopt_record_schema(
    generated: type[Record],
    imported_root: SchemaField,
    fields: tuple[SchemaField, ...],
    hints: cabc.Mapping[str, Any],
    *,
    class_name: str,
    module: str,
    schema_metadata: cabc.Mapping[str, str] | None,
    preserve_root: bool,
) -> None:
    """Attach one exact native/Arrow schema to an existing Record class."""

    pa = _pyarrow()
    metadata = dict(
        schema_metadata
        if schema_metadata is not None
        else imported_root.metadata.items() if preserve_root else ()
    )
    for key in _IDENTITY_KEYS:
        metadata.pop(key, None)
    metadata.update(
        {
            "python.module": module,
            "python.class": class_name,
            "python.qualname": class_name,
            "python.kind": "record",
        }
    )
    if preserve_root:
        root = copy.copy(imported_root)
        # Item access on a schema node reaches a child; metadata goes through
        # the view.
        for key in _IDENTITY_KEYS:
            if key in root.metadata:
                del root.metadata[key]
        root.metadata.update(metadata)
    else:
        root = SchemaField(
            class_name,
            DataType.from_fields(fields),
            nullable=False,
            metadata=metadata,
        )
    arrow_field = root.to_arrow()
    # The one root projection already contains every exact native child.
    # A registered extension may wrap a Struct Field, but an Arrow Schema is
    # itself a Struct layout and cannot import an ExtensionType as its root.
    # Keep the exact extension Field for ``into_arrow_field`` while record
    # batches use its identical storage Struct and retain the root metadata at
    # Schema scope.
    storage_type = getattr(arrow_field.type, "storage_type", None)
    schema_type = storage_type if storage_type is not None else arrow_field.type
    public_metadata = dict(arrow_field.metadata or ())
    extension_metadata: dict[bytes, bytes] = {}
    for key in _EXTENSION_KEYS:
        if key in root.metadata:
            extension_metadata[key.encode()] = root.metadata[key].encode()
    public_metadata.update(extension_metadata)
    arrow_schema = pa.schema(schema_type, metadata=public_metadata or None)
    transport_root: SchemaField | None = None
    if storage_type is not None:
        transport_root = copy.copy(root)
        for key in _EXTENSION_KEYS:
            if key in transport_root.metadata:
                del transport_root.metadata[key]
    _adopt_materialized_schema(
        generated,
        root,
        fields,
        hints,
        arrow_field,
        arrow_schema,
        transport_root=transport_root,
        transport_metadata=extension_metadata,
    )


def record_from_arrow_field(
    cls: type[Any],
    value: object,
    *,
    class_name: str | None = None,
    module: str | None = None,
) -> type[Record]:
    if cls is not Record:
        raise TypeError("from_arrow_field must be called on Record")
    imported = SchemaField.from_arrow(value)
    metadata = dict(imported.metadata.items())
    selected_name, selected_module = _select_identity(
        metadata,
        class_name=class_name,
        module=module,
        root_name=imported.name,
    )
    fields = (
        tuple(imported.data_type)
        if imported.data_type.id == "struct"
        else (imported,)
    )
    return _materialize_record(
        imported,
        fields,
        class_name=selected_name,
        module=selected_module,
        schema_metadata=metadata if imported.data_type.id == "struct" else None,
        preserve_root=imported.data_type.id == "struct",
    )


def record_from_arrow_schema(
    cls: type[Any],
    value: object,
    *,
    class_name: str | None = None,
    module: str | None = None,
) -> type[Record]:
    pa = _pyarrow()
    if cls is not Record:
        raise TypeError("from_arrow_schema must be called on Record")
    if not isinstance(value, pa.Schema):
        raise TypeError("from_arrow_schema expects a pyarrow.Schema")
    # Keep the public factory's explicit UTF-8 error contract. The native
    # whole-Schema import below remains the sole parser for reserved transport
    # metadata and the sole Arrow-to-Yggdryl field conversion.
    _decode_metadata(value.metadata, context="Arrow Schema")
    # Import the complete Schema once so Yggdryl's core Arrow module owns validation,
    # restoration, and removal of its reserved dictionary-ID sidecar. Field
    # dictionary IDs cannot be recovered by importing each PyArrow Field in
    # isolation because Arrow C Schema dictionary IDs are transport-local.
    imported_root = SchemaField._record_root_from_arrow_schema(
        value, "ArrowRecord"
    )
    metadata = dict(imported_root.metadata.items())
    selected_name, selected_module = _select_identity(
        metadata,
        class_name=class_name,
        module=module,
        root_name=None,
    )
    fields = tuple(imported_root.data_type)
    return _materialize_record(
        imported_root,
        fields,
        class_name=selected_name,
        module=selected_module,
        schema_metadata=metadata,
        preserve_root=False,
    )


def _semantic_metadata(metadata: cabc.Mapping[bytes, bytes] | None) -> dict[bytes, bytes]:
    if not metadata:
        return {}
    return {
        bytes(key): bytes(value)
        for key, value in metadata.items()
        if bytes(key).startswith(b"ARROW:extension:")
    }


def _extension_semantics(data_type: Any) -> dict[bytes, bytes]:
    storage_type = getattr(data_type, "storage_type", None)
    if storage_type is None:
        return {}
    name = getattr(data_type, "extension_name", None)
    serialized = getattr(data_type, "__arrow_ext_serialize__", None)
    semantics: dict[bytes, bytes] = {}
    if isinstance(name, str):
        semantics[b"ARROW:extension:name"] = name.encode("utf-8")
    if callable(serialized):
        semantics[b"ARROW:extension:metadata"] = bytes(serialized())
    return semantics


def _field_semantics(field: Any) -> dict[bytes, bytes]:
    semantics = _semantic_metadata(field.metadata)
    semantics.update(_extension_semantics(field.type))
    return semantics


def _storage_type(data_type: Any) -> Any:
    return getattr(data_type, "storage_type", data_type)


def _data_type_layout_equal(expected: Any, actual: Any) -> bool:
    expected = _storage_type(expected)
    actual = _storage_type(actual)
    pa = _pyarrow()
    if pa.types.is_dictionary(expected) or pa.types.is_dictionary(actual):
        return (
            pa.types.is_dictionary(expected)
            and pa.types.is_dictionary(actual)
            and expected.ordered == actual.ordered
            and _data_type_layout_equal(expected.index_type, actual.index_type)
            and _data_type_layout_equal(expected.value_type, actual.value_type)
        )
    expected_children = getattr(expected, "num_fields", 0)
    actual_children = getattr(actual, "num_fields", 0)
    if not expected_children and not actual_children:
        return bool(expected.equals(actual))
    if expected.id != actual.id or expected_children != actual_children:
        return False
    for attribute in ("list_size", "mode", "type_codes", "keys_sorted"):
        if getattr(expected, attribute, None) != getattr(actual, attribute, None):
            return False
    return all(
        _field_layout_equal(expected.field(index), actual.field(index))
        for index in range(expected_children)
    )


def _field_layout_equal(expected: Any, actual: Any) -> bool:
    if expected.name != actual.name or expected.nullable != actual.nullable:
        return False
    if _field_semantics(expected) != _field_semantics(actual):
        return False
    return _data_type_layout_equal(expected.type, actual.type)


def _schema_layout_equal(expected: Any, actual: Any) -> bool:
    if len(expected) != len(actual):
        return False
    if _semantic_metadata(expected.metadata) != _semantic_metadata(actual.metadata):
        return False
    return all(
        _field_layout_equal(expected[index], actual[index])
        for index in range(len(expected))
    )


def _validate_schema(cls: type[Any], actual: Any, validate_schema: bool) -> None:
    if type(validate_schema) is not bool:
        raise TypeError("validate_schema must be bool")
    if not validate_schema:
        return
    expected = _ensure_schema(cls).arrow_schema
    if not _schema_layout_equal(expected, actual):
        raise TypeError(
            f"incompatible Arrow schema for {cls.__name__}: "
            f"expected {expected}, got {actual}"
        )


def _check_import_options(safe: bool, errors: str, validate_schema: bool) -> None:
    _check_options(safe, errors)
    if type(validate_schema) is not bool:
        raise TypeError("validate_schema must be bool")


def _cast_import_batch(
    cls: type[Any], batch: Any, *, safe: bool, validate_schema: bool
) -> tuple[Any, bool]:
    """Reconcile structural columns, retaining lazy scalar error localization."""

    if safe and not validate_schema:
        expected = _ensure_schema(cls).arrow_schema
        positional = len(expected) == len(batch.schema) and all(
            expected[index].name == batch.schema[index].name
            for index in range(len(expected))
        )
        if positional:
            # Preserve the established row-lazy Record error/default policy.
            # Physical value casts happen on first cell access below, while
            # native batch casting owns every structural add/drop/reorder.
            return batch, True
        try:
            return (
                _ensure_schema(cls).root.cast_arrow_batch(batch, safe=False),
                False,
            )
        except Exception as error:
            raise TypeError(
                f"cannot safely cast Arrow batch for {cls.__name__} ({error})"
            ) from error
    return batch, False


class _TypePlan(typing.NamedTuple):
    type_id: str
    arrow_type: Any
    children: tuple[_TypePlan, ...]
    map_as_pairs: bool


class _PreparedBatchTypes(typing.NamedTuple):
    """Recursive conversion plans shared by one fixed-schema Arrow source."""

    target_types: tuple[Any | None, ...]
    plans: tuple[_TypePlan, ...]


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


def _prepare_batch_types(
    schema: Any, source_schema: Any, *, cast_to_schema: bool
) -> _PreparedBatchTypes:
    target_types = tuple(
        schema.arrow_schema.field(index).type
        if cast_to_schema
        and not source_schema.field(index).type.equals(
            schema.arrow_schema.field(index).type
        )
        else None
        for index in range(len(schema.fields))
    )
    plans = tuple(
        _prepare_type_plan(
            field.data_type,
            target_types[index]
            if target_types[index] is not None
            else source_schema.field(index).type,
        )
        for index, field in enumerate(schema.fields)
    )
    return _PreparedBatchTypes(target_types, plans)


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


def _field_arrow_scalar(
    field: SchemaField, value: Any, *, safe: bool
) -> Any:
    """Central scalar cast hook shared by record import and export."""

    return field.arrow_scalar(value, safe=safe)


class _ArrowRow(cabc.Mapping[str, Any]):
    __slots__ = (
        "_batch",
        "_cache",
        "_fields",
        "_index",
        "_plans",
        "_positions",
        "_record_path",
        "_target_types",
    )

    def __init__(
        self,
        batch: Any,
        fields: tuple[SchemaField, ...],
        plans: tuple[_TypePlan, ...],
        target_types: tuple[Any | None, ...],
        positions: cabc.Mapping[str, int],
        index: int,
        record_path: str,
    ) -> None:
        self._batch = batch
        self._cache = [_MISSING] * len(fields)
        self._fields = fields
        self._plans = plans
        self._positions = positions
        self._index = index
        self._record_path = record_path
        self._target_types = target_types

    def __len__(self) -> int:
        return len(self._fields)

    def __iter__(self) -> typing.Iterator[str]:
        return (field.name for field in self._fields)

    def __contains__(self, name: object) -> bool:
        return isinstance(name, str) and name in self._positions

    def __getitem__(self, name: str) -> Any:
        position = self._positions[name]
        cached = self._cache[position]
        if cached is not _MISSING:
            return cached
        scalar = self._batch.column(position)[self._index]
        target_type = self._target_types[position]
        if target_type is not None:
            try:
                # This target was projected and cached once for the source
                # boundary. Calling Field.arrow_scalar here would reproject a
                # complete C Field for every mismatched cell.
                scalar = scalar.cast(target_type, safe=True)
            except Exception as error:
                raise TypeError(
                    f"{self._record_path}.{name}: cannot safely cast Arrow value "
                    f"({error})"
                ) from error
        converted = _arrow_scalar_value(
            scalar,
            self._plans[position],
            path=f"{self._record_path}.{name}",
        )
        self._cache[position] = converted
        return converted


class _SelectedRow(cabc.Mapping[str, Any]):
    __slots__ = ("_name_set", "_names", "_row")

    def __init__(
        self,
        row: _ArrowRow,
        names: tuple[str, ...],
        name_set: frozenset[str],
    ) -> None:
        self._row = row
        self._names = names
        self._name_set = name_set

    def __len__(self) -> int:
        return len(self._names)

    def __iter__(self) -> typing.Iterator[str]:
        return iter(self._names)

    def __contains__(self, name: object) -> bool:
        return isinstance(name, str) and name in self._name_set

    def __getitem__(self, name: str) -> Any:
        if name not in self._name_set:
            raise KeyError(name)
        return self._row[name]


def _validate_record_importable(cls: type[Any]) -> None:
    schema = _ensure_schema(cls)
    stored_names = {field.name for field in schema.value_fields}
    required_initvars = [
        field.name
        for field in schema.constructor_fields
        if field.name not in stored_names and not _has_default(field)
    ]
    if required_initvars:
        raise TypeError(
            f"{cls.__name__}: Arrow rows cannot reconstruct required InitVar "
            f"fields {required_initvars!r}; declare defaults or use from_dicts"
        )


def _record_from_arrow_row(
    cls: type[Any],
    schema: Any,
    row: _ArrowRow,
    *,
    constructor_names: tuple[str, ...],
    constructor_name_set: frozenset[str],
    safe: bool,
    errors: str,
    path: str,
) -> Any:
    instance = _from_dict(
        cls,
        _SelectedRow(row, constructor_names, constructor_name_set),
        safe=safe,
        errors=errors,
        path=path,
    )
    for field in schema.value_fields:
        if field.init:
            continue
        try:
            converted = (
                _convert(
                    row[field.name],
                    schema.hints.get(field.name, field.type),
                    cls,
                    f"{path}.{field.name}",
                    errors,
                )
                if safe
                else row[field.name]
            )
        except (TypeError, ValueError):
            if errors == "default" and _has_default(field):
                continue
            raise
        object.__setattr__(instance, field.name, converted)
    return instance


def _batch_rows(
    cls: type[Any],
    batch: Any,
    *,
    safe: bool,
    errors: str,
    start_index: int,
    cast_to_schema: bool,
    prepared_types: _PreparedBatchTypes | None = None,
) -> typing.Iterator[Any]:
    schema = _ensure_schema(cls)
    if cast_to_schema:
        if batch.num_columns != len(schema.fields):
            raise TypeError(
                f"{cls.__name__}[{start_index}:]: expected {len(schema.fields)} "
                f"Arrow columns, got {batch.num_columns}"
            )
    positions = {field.name: index for index, field in enumerate(schema.fields)}
    constructor_names = tuple(
        field.name
        for field in schema.constructor_fields
        if field.name in positions
    )
    constructor_name_set = frozenset(constructor_names)
    if prepared_types is None:
        prepared_types = _prepare_batch_types(
            schema, batch.schema, cast_to_schema=cast_to_schema
        )
    target_types, plans = prepared_types
    for row_index in range(batch.num_rows):
        global_index = start_index + row_index
        path = f"{cls.__name__}[{global_index}]"
        yield _record_from_arrow_row(
            cls,
            schema,
            _ArrowRow(
                batch,
                schema.fields,
                plans,
                target_types,
                positions,
                row_index,
                path,
            ),
            constructor_names=constructor_names,
            constructor_name_set=constructor_name_set,
            safe=safe,
            errors=errors,
            path=path,
        )


def records_from_arrow_record_batch(
    cls: type[Any],
    batch: object,
    *,
    safe: bool = True,
    errors: str = "raise",
    validate_schema: bool = True,
) -> typing.Iterator[Any]:
    pa = _pyarrow()
    _check_import_options(safe, errors, validate_schema)
    _validate_record_importable(cls)
    if not isinstance(batch, pa.RecordBatch):
        raise TypeError("from_arrow_record_batch expects a pyarrow.RecordBatch")
    _validate_schema(cls, batch.schema, validate_schema)
    batch, cast_to_schema = _cast_import_batch(
        cls, batch, safe=safe, validate_schema=validate_schema
    )
    return _batch_rows(
        cls,
        batch,
        safe=safe,
        errors=errors,
        start_index=0,
        cast_to_schema=cast_to_schema,
    )


def records_from_arrow_record_batch_reader(
    cls: type[Any],
    reader: object,
    *,
    safe: bool = True,
    errors: str = "raise",
    validate_schema: bool = True,
) -> typing.Iterator[Any]:
    pa = _pyarrow()
    _check_import_options(safe, errors, validate_schema)
    _validate_record_importable(cls)
    if not isinstance(reader, pa.RecordBatchReader):
        raise TypeError(
            "from_arrow_record_batch_reader expects a pyarrow.RecordBatchReader"
        )
    _validate_schema(cls, reader.schema, validate_schema)

    def rows() -> typing.Iterator[Any]:
        offset = 0
        prepared_types: _PreparedBatchTypes | None = None
        prepared_cast_to_schema = False
        for batch_index, batch in enumerate(reader):
            if not isinstance(batch, pa.RecordBatch):
                raise TypeError(f"Arrow reader item {batch_index} is not a RecordBatch")
            batch, cast_to_schema = _cast_import_batch(
                cls, batch, safe=safe, validate_schema=validate_schema
            )
            if prepared_types is None:
                native_schema = _ensure_schema(cls)
                prepared_cast_to_schema = cast_to_schema
                prepared_types = _prepare_batch_types(
                    native_schema,
                    batch.schema,
                    cast_to_schema=cast_to_schema,
                )
            yield from _batch_rows(
                cls,
                batch,
                safe=safe,
                errors=errors,
                start_index=offset,
                cast_to_schema=prepared_cast_to_schema,
                prepared_types=prepared_types,
            )
            offset += batch.num_rows

    return rows()


def records_from_arrow_table(
    cls: type[Any],
    table: object,
    *,
    safe: bool = True,
    errors: str = "raise",
    validate_schema: bool = True,
) -> typing.Iterator[Any]:
    pa = _pyarrow()
    _check_import_options(safe, errors, validate_schema)
    _validate_record_importable(cls)
    if not isinstance(table, pa.Table):
        raise TypeError("from_arrow_table expects a pyarrow.Table")
    _validate_schema(cls, table.schema, validate_schema)

    def rows() -> typing.Iterator[Any]:
        offset = 0
        prepared_types: _PreparedBatchTypes | None = None
        prepared_cast_to_schema = False
        for batch in table.to_batches():
            batch, cast_to_schema = _cast_import_batch(
                cls, batch, safe=safe, validate_schema=validate_schema
            )
            if prepared_types is None:
                native_schema = _ensure_schema(cls)
                prepared_cast_to_schema = cast_to_schema
                prepared_types = _prepare_batch_types(
                    native_schema,
                    batch.schema,
                    cast_to_schema=cast_to_schema,
                )
            yield from _batch_rows(
                cls,
                batch,
                safe=safe,
                errors=errors,
                start_index=offset,
                cast_to_schema=prepared_cast_to_schema,
                prepared_types=prepared_types,
            )
            offset += batch.num_rows

    return rows()


def records_from_arrow(
    cls: type[Any],
    source: object,
    *,
    safe: bool = True,
    errors: str = "raise",
    validate_schema: bool = True,
) -> typing.Iterator[Any]:
    pa = _pyarrow()
    _check_import_options(safe, errors, validate_schema)
    _validate_record_importable(cls)
    if isinstance(source, pa.RecordBatch):
        return records_from_arrow_record_batch(
            cls, source, safe=safe, errors=errors, validate_schema=validate_schema
        )
    if isinstance(source, pa.Table):
        return records_from_arrow_table(
            cls, source, safe=safe, errors=errors, validate_schema=validate_schema
        )
    if isinstance(source, pa.RecordBatchReader):
        return records_from_arrow_record_batch_reader(
            cls, source, safe=safe, errors=errors, validate_schema=validate_schema
        )
    if callable(getattr(source, "__arrow_c_stream__", None)):
        reader = pa.RecordBatchReader.from_stream(source)
        return records_from_arrow_record_batch_reader(
            cls, reader, safe=safe, errors=errors, validate_schema=validate_schema
        )
    if not isinstance(source, cabc.Iterable) or isinstance(
        source, (str, bytes, bytearray, memoryview, cabc.Mapping)
    ):
        raise TypeError(
            "from_arrow expects a RecordBatch, Table, RecordBatchReader, "
            "Arrow C stream exporter, or iterable of RecordBatch values"
        )

    def rows() -> typing.Iterator[Any]:
        offset = 0
        for batch_index, batch in enumerate(source):
            if not isinstance(batch, pa.RecordBatch):
                raise TypeError(
                    f"Arrow batch iterable item {batch_index} is not a RecordBatch"
                )
            _validate_schema(cls, batch.schema, validate_schema)
            batch, cast_to_schema = _cast_import_batch(
                cls, batch, safe=safe, validate_schema=validate_schema
            )
            yield from _batch_rows(
                cls,
                batch,
                safe=safe,
                errors=errors,
                start_index=offset,
                cast_to_schema=cast_to_schema,
            )
            offset += batch.num_rows

    return rows()


def _lower_arrow_shape(value: Any) -> Any:
    pa = _pyarrow()
    month_day_nano = getattr(pa, "MonthDayNano", None)
    if month_day_nano is not None and isinstance(value, month_day_nano):
        return value
    if isinstance(value, pa.Scalar):
        return value
    if isinstance(value, enum.Enum):
        return _lower_arrow_shape(value.value)
    if isinstance(value, complex):
        return {"real": float(value.real), "imag": float(value.imag)}
    if isinstance(value, (Uri, Url, Urn)):
        return str(value)
    if isinstance(value, (uuid.UUID, pathlib.PurePath)):
        return value.as_posix() if isinstance(value, pathlib.PurePath) else str(value)
    if isinstance(value, (bytearray, memoryview)):
        return bytes(value)
    if dc.is_dataclass(value) and not isinstance(value, type):
        return {
            field.name: _lower_arrow_shape(getattr(value, field.name))
            for field in dc.fields(value)
        }
    if isinstance(value, cabc.Mapping):
        return {
            _lower_arrow_shape(key): _lower_arrow_shape(item)
            for key, item in value.items()
        }
    if isinstance(value, (list, tuple, set, frozenset)):
        return [_lower_arrow_shape(item) for item in value]
    return value


def _reject_unsupported_output(data_type: Any, *, path: str) -> None:
    pa = _pyarrow()
    if pa.types.is_union(data_type):
        raise TypeError(
            f"{path}: PyArrow cannot construct union arrays from Python record values"
        )
    if pa.types.is_dictionary(data_type):
        _reject_unsupported_output(data_type.value_type, path=path)
        return
    storage_type = getattr(data_type, "storage_type", None)
    if storage_type is not None:
        _reject_unsupported_output(storage_type, path=path)
        return
    for index in range(getattr(data_type, "num_fields", 0)):
        child = data_type.field(index)
        _reject_unsupported_output(child.type, path=f"{path}.{child.name}")


def _validate_output_shape(value: Any, field: Any, *, path: str) -> None:
    pa = _pyarrow()
    if value is None or (
        isinstance(value, pa.Scalar) and not value.is_valid
    ):
        if not field.nullable:
            raise _OutputShapeError(f"{path}: field is not nullable")
        return
    if isinstance(value, pa.Scalar):
        if not _needs_recursive_output_validation(field.type):
            return
        value = _scalar_as_py_for_validation(value)
    _validate_nonnull_output_shape(value, field.type, path=path)


def _scalar_as_py_for_validation(value: Any) -> Any:
    return value.as_py()


def _needs_recursive_output_validation(data_type: Any) -> bool:
    pa = _pyarrow()
    if pa.types.is_struct(data_type) or pa.types.is_map(data_type):
        return True
    if _is_list_type(data_type):
        return True
    if pa.types.is_run_end_encoded(data_type) or pa.types.is_dictionary(
        data_type
    ):
        return _needs_recursive_output_validation(data_type.value_type)
    storage_type = getattr(data_type, "storage_type", None)
    return storage_type is not None and _needs_recursive_output_validation(
        storage_type
    )


def _validate_nonnull_output_shape(
    value: Any, data_type: Any, *, path: str
) -> None:
    pa = _pyarrow()
    if pa.types.is_map(data_type):
        pairs = value.items() if isinstance(value, cabc.Mapping) else value
        previous: Any = None
        has_previous = False
        hashable_keys: set[Any] = set()
        unhashable_keys: list[Any] = []
        for index, pair in enumerate(pairs):
            try:
                key, item = pair
            except (TypeError, ValueError) as error:
                raise TypeError(f"{path}[{index}]: expected a map key/value pair") from error
            try:
                duplicate = key in hashable_keys
                hashable_keys.add(key)
            except TypeError:
                duplicate = any(
                    _map_keys_equal(key, candidate)
                    for candidate in unhashable_keys
                )
                unhashable_keys.append(key)
            if duplicate:
                raise ValueError(f"{path}: duplicate Arrow map key {key!r}")
            if (
                data_type.keys_sorted
                and has_previous
                and not getattr(data_type.key_type, "num_fields", 0)
            ):
                try:
                    unsorted = key < previous
                except TypeError as error:
                    raise TypeError(
                        f"{path}: keys_sorted map keys must be mutually orderable"
                    ) from error
                if unsorted:
                    raise ValueError(
                        f"{path}: map keys must be sorted for keys_sorted Arrow type"
                    )
            previous = key
            has_previous = True
            _validate_output_shape(
                key,
                data_type.key_field,
                path=f"{path}.keys[{index}]",
            )
            _validate_output_shape(
                item,
                data_type.item_field,
                path=f"{path}[{key!r}]",
            )
        return
    if pa.types.is_struct(data_type) and isinstance(value, cabc.Mapping):
        for field in data_type:
            if field.name in value:
                _validate_output_shape(
                    value[field.name], field, path=f"{path}.{field.name}"
                )
        return
    if _is_list_type(data_type):
        item_field = data_type.field(0)
        for index, item in enumerate(value):
            _validate_output_shape(
                item, item_field, path=f"{path}[{index}]"
            )
        return
    if pa.types.is_run_end_encoded(data_type):
        _validate_nonnull_output_shape(
            value, data_type.value_type, path=path
        )
        return
    if pa.types.is_dictionary(data_type):
        _validate_nonnull_output_shape(
            value, data_type.value_type, path=path
        )
        return
    storage_type = getattr(data_type, "storage_type", None)
    if storage_type is not None:
        _validate_nonnull_output_shape(value, storage_type, path=path)


def _coerce_arrow_shape(value: Any, data_type: Any) -> Any:
    if value is None:
        return None
    pa = _pyarrow()
    if isinstance(value, pa.Scalar):
        return value
    if pa.types.is_map(data_type):
        pairs = value.items() if isinstance(value, cabc.Mapping) else value
        return [
            (
                _coerce_arrow_shape(key, data_type.key_type),
                _coerce_arrow_shape(item, data_type.item_type),
            )
            for key, item in pairs
        ]
    if pa.types.is_struct(data_type) and isinstance(value, cabc.Mapping):
        return {
            field.name: _coerce_arrow_shape(value.get(field.name), field.type)
            for field in data_type
        }
    if _is_list_type(data_type):
        item_type = data_type.field(0).type
        return [_coerce_arrow_shape(item, item_type) for item in value]
    if pa.types.is_run_end_encoded(data_type):
        return _coerce_arrow_shape(value, data_type.value_type)
    if pa.types.is_dictionary(data_type):
        return _coerce_arrow_shape(value, data_type.value_type)
    storage_type = getattr(data_type, "storage_type", None)
    if storage_type is not None:
        return _coerce_arrow_shape(value, storage_type)
    return value


def _append_record_columns(
    cls: type[Any],
    schema: Any,
    columns: list[list[Any]],
    value: Any,
    *,
    safe: bool,
    row_index: int,
) -> None:
    pa = _pyarrow()
    path = f"{cls.__name__}[{row_index}]"
    if type(value) is not cls:
        raise TypeError(
            f"{path}: expected exact {cls.__name__} instance, "
            f"got {type(value).__name__}"
        )
    try:
        _, context_cache, owner, projected = _project_record_values(
            value,
            safe=safe,
            resolved_cache=schema.nested_hints if safe else None,
            conversion_owner=cls,
        )
        for index, (name, converted, hint, physical_field) in enumerate(projected):
            target_type = schema.arrow_schema.field(index).type
            month_day_nano = getattr(pa, "MonthDayNano", None)
            if (
                month_day_nano is not None
                and isinstance(converted, month_day_nano)
                and target_type == pa.month_day_nano_interval()
            ):
                lowered = converted
            else:
                lowered = (
                    _lower_arrow_shape(
                        _export(
                            converted,
                            context_cache,
                            owner,
                            hint,
                            physical_field,
                        )
                    )
                    if safe
                    else _lower_arrow_shape(converted)
                )
            lowered = _coerce_arrow_shape(lowered, target_type)
            if isinstance(lowered, pa.Scalar) and not lowered.is_valid:
                _validate_output_shape(
                    lowered,
                    schema.arrow_schema.field(index),
                    path=f"{path}.{name}",
                )
            if (
                isinstance(lowered, pa.Scalar)
                and not lowered.type.equals(target_type)
            ):
                try:
                    lowered = _field_arrow_scalar(
                        schema.fields[index], lowered, safe=True
                    )
                    if not lowered.type.equals(target_type):
                        raise TypeError(
                            f"cast returned {lowered.type}, expected {target_type}"
                        )
                except Exception as error:
                    raise TypeError(
                        f"{name}: cannot safely cast Arrow scalar ({error})"
                    ) from error
            _validate_output_shape(
                lowered,
                schema.arrow_schema.field(index),
                path=f"{path}.{name}",
            )
            columns[index].append(lowered)
    except _OutputShapeError:
        raise
    except OverflowError as error:
        raise OverflowError(f"{path}: {error}") from error
    except ValueError as error:
        raise ValueError(f"{path}: {error}") from error
    except TypeError as error:
        raise TypeError(f"{path}: {error}") from error


def _record_columns(
    cls: type[Any],
    schema: Any,
    values: typing.Iterator[Any],
    *,
    safe: bool,
    start_index: int,
    limit: int | None,
) -> tuple[list[list[Any]], int, bool]:
    columns: list[list[Any]] = [[] for _ in schema.fields]
    count = 0
    while limit is None or count < limit:
        try:
            value = next(values)
        except StopIteration:
            return columns, count, True
        _append_record_columns(
            cls,
            schema,
            columns,
            value,
            safe=safe,
            row_index=start_index + count,
        )
        count += 1
    return columns, count, False


def _columns_into_batch(
    cls: type[Any],
    schema: Any,
    columns: list[list[Any]],
    row_count: int,
    *,
    start_index: int,
) -> Any:
    pa = _pyarrow()
    arrow_schema = schema.arrow_transport_schema
    if not columns and row_count:
        # Arrow's empty from_arrays constructor has no row-count argument.
        # Select an empty projection from a temporary counted column instead.
        counted = pa.record_batch(
            [pa.nulls(row_count)], names=["__yggdryl_row_count__"]
        )
        return counted.select([]).replace_schema_metadata(arrow_schema.metadata)
    arrays = []
    for column_index, column in enumerate(columns):
        field = arrow_schema.field(column_index)
        native_field = schema.fields[column_index]
        try:
            arrays.append(_values_into_arrow_array(column, field.type))
        except Exception as error:
            # PyArrow's bulk builder rejects safely castable Scalar values with
            # a different physical type. Scan only after that fast path fails,
            # normalize the explicit Scalar cells, then retry the same bulk
            # builder. Record `safe=False` skips annotation recursion; Arrow
            # physical casts remain safe in both modes.
            if any(isinstance(value, pa.Scalar) for value in column):
                normalized: list[Any] = []
                for offset, value in enumerate(column):
                    if not isinstance(value, pa.Scalar):
                        normalized.append(value)
                        continue
                    if value.type.equals(field.type):
                        normalized.append(value.as_py())
                        continue
                    try:
                        value = _field_arrow_scalar(
                            native_field, value, safe=True
                        ).as_py()
                    except Exception as item_error:
                        raise TypeError(
                            f"{cls.__name__}[{start_index + offset}].{field.name}: "
                            f"cannot construct Arrow value ({item_error})"
                        ) from item_error
                    normalized.append(value)
                try:
                    arrays.append(
                        _values_into_arrow_array(normalized, field.type)
                    )
                except Exception as normalized_error:
                    error = normalized_error
                    column = normalized
                else:
                    continue
            for offset, value in enumerate(column):
                try:
                    _field_arrow_scalar(native_field, value, safe=True)
                except Exception as item_error:
                    # Some physical layouts do not expose PyArrow Scalar
                    # constructors even though their specialized array builder
                    # supports a one-cell array (notably run-end encoding).
                    try:
                        _values_into_arrow_array([value], field.type)
                    except Exception:
                        pass
                    else:
                        continue
                    raise TypeError(
                        f"{cls.__name__}[{start_index + offset}].{field.name}: "
                        f"cannot construct Arrow value ({item_error})"
                    ) from item_error
            raise TypeError(
                f"{cls.__name__}.{field.name}: cannot construct Arrow column ({error})"
            ) from error
    return pa.RecordBatch.from_arrays(arrays, schema=arrow_schema)


def _values_into_arrow_array(values: list[Any], data_type: Any) -> Any:
    pa = _pyarrow()
    storage_type = getattr(data_type, "storage_type", None)
    if storage_type is not None and _contains_run_end_encoded(storage_type):
        storage = _values_into_arrow_array(values, storage_type)
        return pa.ExtensionArray.from_storage(data_type, storage)
    if pa.types.is_run_end_encoded(data_type):
        capacity = _run_end_type_capacity(data_type.run_end_type)
        if len(values) > capacity:
            raise OverflowError(
                f"{data_type.run_end_type} run ends can encode at most "
                f"{capacity} values, got {len(values)}"
            )
        run_ends: list[int] = []
        run_values: list[Any] = []
        previous: Any = None
        has_previous = False
        for index, value in enumerate(values):
            same = False
            if has_previous:
                try:
                    same = bool(value == previous)
                except (TypeError, ValueError):
                    same = False
            if not same:
                if has_previous:
                    run_ends.append(index)
                run_values.append(value)
                previous = value
                has_previous = True
        if has_previous:
            run_ends.append(len(values))
        encoded_ends = pa.array(run_ends, type=data_type.run_end_type)
        encoded_values = _values_into_arrow_array(run_values, data_type.value_type)
        return pa.RunEndEncodedArray.from_arrays(
            encoded_ends, encoded_values, type=data_type
        )
    if pa.types.is_struct(data_type) and _contains_run_end_encoded(data_type):
        children = [
            _values_into_arrow_array(
                [
                    None if value is None else value.get(field.name)
                    for value in values
                ],
                field.type,
            )
            for field in data_type
        ]
        mask = _null_mask(values)
        return pa.StructArray.from_arrays(
            children, mask=mask, type=data_type
        )
    if _is_list_type(data_type) and _contains_run_end_encoded(data_type):
        child_type = data_type.field(0).type
        flattened: list[Any] = []
        offsets = [0]
        sizes: list[int] = []
        for value in values:
            items = [] if value is None else list(value)
            flattened.extend(items)
            sizes.append(len(items))
            offsets.append(len(flattened))
        child = _values_into_arrow_array(flattened, child_type)
        mask = _null_mask(values)
        if pa.types.is_fixed_size_list(data_type):
            width = data_type.list_size
            padded: list[Any] = []
            for value in values:
                items = [None] * width if value is None else list(value)
                if len(items) != width:
                    raise ValueError(
                        f"expected {width} values for fixed-size Arrow list"
                    )
                padded.extend(items)
            child = _values_into_arrow_array(padded, child_type)
            return pa.FixedSizeListArray.from_arrays(
                child, type=data_type, mask=mask
            )
        if pa.types.is_large_list(data_type):
            return pa.LargeListArray.from_arrays(
                pa.array(offsets, type=pa.int64()), child, type=data_type, mask=mask
            )
        if getattr(pa.types, "is_list_view", lambda _: False)(data_type):
            return pa.ListViewArray.from_arrays(
                pa.array(offsets[:-1], type=pa.int32()),
                pa.array(sizes, type=pa.int32()),
                child,
                type=data_type,
                mask=mask,
            )
        if getattr(pa.types, "is_large_list_view", lambda _: False)(data_type):
            return pa.LargeListViewArray.from_arrays(
                pa.array(offsets[:-1], type=pa.int64()),
                pa.array(sizes, type=pa.int64()),
                child,
                type=data_type,
                mask=mask,
            )
        return pa.ListArray.from_arrays(
            pa.array(offsets, type=pa.int32()), child, type=data_type, mask=mask
        )
    if pa.types.is_map(data_type) and _contains_run_end_encoded(data_type):
        offsets = [0]
        keys: list[Any] = []
        map_items: list[Any] = []
        for value in values:
            pairs = [] if value is None else value
            for key, item in pairs:
                keys.append(key)
                map_items.append(item)
            offsets.append(len(keys))
        return pa.MapArray.from_arrays(
            pa.array(offsets, type=pa.int32()),
            _values_into_arrow_array(keys, data_type.key_type),
            _values_into_arrow_array(map_items, data_type.item_type),
            type=data_type,
            mask=_null_mask(values),
        )
    if pa.types.is_dictionary(data_type):
        dictionary_values: list[Any] = []
        indices: list[int | None] = []
        for value in values:
            if value is None:
                indices.append(None)
                continue
            found = next(
                (
                    index
                    for index, candidate in enumerate(dictionary_values)
                    if _map_keys_equal(value, candidate)
                ),
                None,
            )
            if found is None:
                found = len(dictionary_values)
                dictionary_values.append(value)
            indices.append(found)
        return pa.DictionaryArray.from_arrays(
            pa.array(indices, type=data_type.index_type),
            _values_into_arrow_array(dictionary_values, data_type.value_type),
            ordered=data_type.ordered,
            safe=True,
        )
    return pa.array(values, type=data_type)


def _null_mask(values: list[Any]) -> Any:
    pa = _pyarrow()
    nulls = [value is None for value in values]
    return pa.array(nulls, type=pa.bool_()) if any(nulls) else None


def _is_list_type(data_type: Any) -> bool:
    pa = _pyarrow()
    return any(
        predicate(data_type)
        for name in (
            "is_list",
            "is_large_list",
            "is_fixed_size_list",
            "is_list_view",
            "is_large_list_view",
        )
        for predicate in (getattr(pa.types, name, None),)
        if predicate is not None
    )


def _contains_run_end_encoded(data_type: Any) -> bool:
    pa = _pyarrow()
    if pa.types.is_run_end_encoded(data_type):
        return True
    if pa.types.is_dictionary(data_type):
        return _contains_run_end_encoded(data_type.value_type)
    return any(
        _contains_run_end_encoded(data_type.field(index).type)
        for index in range(getattr(data_type, "num_fields", 0))
    )


def _run_end_type_capacity(data_type: Any) -> int:
    bit_width = typing.cast(int | None, getattr(data_type, "bit_width", None))
    if bit_width not in (16, 32, 64):  # pragma: no cover - PyArrow validates this
        raise TypeError(f"unsupported Arrow run-end type {data_type}")
    return (1 << (bit_width - 1)) - 1


def _run_end_capacity(data_type: Any) -> int | None:
    pa = _pyarrow()
    storage_type = getattr(data_type, "storage_type", None)
    if storage_type is not None:
        return _run_end_capacity(storage_type)
    capacities: list[int] = []
    if pa.types.is_run_end_encoded(data_type):
        capacities.append(_run_end_type_capacity(data_type.run_end_type))
        nested = _run_end_capacity(data_type.value_type)
        if nested is not None:
            capacities.append(nested)
    elif pa.types.is_dictionary(data_type):
        nested = _run_end_capacity(data_type.value_type)
        if nested is not None:
            capacities.append(nested)
    else:
        for index in range(getattr(data_type, "num_fields", 0)):
            nested = _run_end_capacity(data_type.field(index).type)
            if nested is not None:
                capacities.append(nested)
    return min(capacities) if capacities else None


def _schema_run_end_capacity(schema: Any) -> int | None:
    capacities = [
        capacity
        for field in schema.arrow_schema
        for capacity in (_run_end_capacity(field.type),)
        if capacity is not None
    ]
    return min(capacities) if capacities else None


def _prepare_output(cls: type[Any], safe: bool) -> Any:
    if type(safe) is not bool:
        raise TypeError("safe must be bool")
    schema = _ensure_schema(cls)
    for field in schema.arrow_schema:
        _reject_unsupported_output(field.type, path=f"{cls.__name__}.{field.name}")
    return schema


def records_into_arrow_record_batch(
    cls: type[Any], values: cabc.Iterable[Any], *, safe: bool = True
) -> Any:
    schema = _prepare_output(cls, safe)
    capacity = _schema_run_end_capacity(schema)
    columns, row_count, exhausted = _record_columns(
        cls,
        schema,
        iter(values),
        safe=safe,
        start_index=0,
        limit=None if capacity is None else capacity + 1,
    )
    if capacity is not None and not exhausted and row_count > capacity:
        raise ValueError(
            f"{cls.__name__}: a single Arrow RecordBatch can contain at most "
            f"{capacity} rows for its narrowest run-end encoded column; use "
            "into_arrow_record_batches or into_arrow_table"
        )
    return _columns_into_batch(
        cls, schema, columns, row_count, start_index=0
    )


def _positive_batch_size(batch_size: int) -> int:
    if isinstance(batch_size, bool) or not isinstance(batch_size, int):
        raise TypeError("batch_size must be an integer")
    if batch_size <= 0:
        raise ValueError("batch_size must be positive")
    return batch_size


def records_into_arrow_record_batches(
    cls: type[Any],
    values: cabc.Iterable[Any],
    *,
    batch_size: int = 65_536,
    safe: bool = True,
) -> typing.Iterator[Any]:
    batch_size = _positive_batch_size(batch_size)
    schema = _prepare_output(cls, safe)
    capacity = _schema_run_end_capacity(schema)
    if capacity is not None:
        batch_size = min(batch_size, capacity)
    iterator = iter(values)

    def batches() -> typing.Iterator[Any]:
        offset = 0
        while True:
            columns, row_count, exhausted = _record_columns(
                cls,
                schema,
                iterator,
                safe=safe,
                start_index=offset,
                limit=batch_size,
            )
            if not row_count:
                return
            yield _columns_into_batch(
                cls,
                schema,
                columns,
                row_count,
                start_index=offset,
            )
            offset += row_count
            if exhausted:
                return

    return batches()


def records_into_arrow_table(
    cls: type[Any], values: cabc.Iterable[Any], *, safe: bool = True
) -> Any:
    pa = _pyarrow()
    schema = _prepare_output(cls, safe)
    capacity = _schema_run_end_capacity(schema)
    batch_size = min(65_536, capacity) if capacity is not None else 65_536
    iterator = iter(values)
    batches: list[Any] = []
    offset = 0
    while True:
        columns, row_count, exhausted = _record_columns(
            cls,
            schema,
            iterator,
            safe=safe,
            start_index=offset,
            limit=batch_size,
        )
        if row_count:
            batches.append(
                _columns_into_batch(
                    cls,
                    schema,
                    columns,
                    row_count,
                    start_index=offset,
                )
            )
            offset += row_count
        if exhausted or not row_count:
            break
    return pa.Table.from_batches(batches, schema=schema.arrow_transport_schema)


def records_into_arrow_record_batch_reader(
    cls: type[Any],
    values: cabc.Iterable[Any],
    *,
    batch_size: int = 65_536,
    safe: bool = True,
) -> Any:
    pa = _pyarrow()
    batch_size = _positive_batch_size(batch_size)
    schema = _prepare_output(cls, safe)
    capacity = _schema_run_end_capacity(schema)
    if capacity is not None:
        batch_size = min(batch_size, capacity)
    iterator = iter(values)

    def batches() -> typing.Iterator[Any]:
        offset = 0
        while True:
            columns, row_count, exhausted = _record_columns(
                cls,
                schema,
                iterator,
                safe=safe,
                start_index=offset,
                limit=batch_size,
            )
            if not row_count:
                return
            yield _columns_into_batch(
                cls,
                schema,
                columns,
                row_count,
                start_index=offset,
            )
            offset += row_count
            if exhausted:
                return

    return pa.RecordBatchReader.from_batches(
        schema.arrow_transport_schema, batches()
    )


__all__ = [
    "record_from_arrow_field",
    "record_from_arrow_schema",
    "records_from_arrow",
    "records_from_arrow_record_batch",
    "records_from_arrow_record_batch_reader",
    "records_from_arrow_table",
    "records_into_arrow_record_batch",
    "records_into_arrow_record_batches",
    "records_into_arrow_record_batch_reader",
    "records_into_arrow_table",
]
