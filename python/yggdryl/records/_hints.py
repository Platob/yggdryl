"""Translate Python annotations into native Yggdryl schema values.

This module intentionally owns only Python's annotation semantics. Every
result is constructed as a native :class:`yggdryl.DataType` or
:class:`yggdryl.Field`, and all Arrow validation is performed by the Rust core.
"""

from __future__ import annotations

import collections
import collections.abc as cabc
import dataclasses
import datetime as datetime_module
import decimal
import enum
import fractions
import functools
import importlib
import numbers
import os
import pathlib
import re
import sys
import types
import typing
import uuid
from typing import Any

from .._native import DataType, Field, Uri, Url, Urn

try:  # Python 3.10 gets newer annotation wrappers from typing_extensions.
    _typing_extensions: Any = importlib.import_module("typing_extensions")
except ImportError:  # pragma: no cover - only relevant to minimal installations
    _typing_extensions = None

__all__ = ["datatype_from_pyhint", "field_from_pyhint"]

_NONE_TYPE = type(None)
_MISSING = object()
_MAX_INFERENCE_DEPTH = 64
_FIELD_OPTION_KEYS = frozenset(
    {
        "nullable",
        "metadata",
        "id",
        "dictionary_id",
        "dictionary_is_ordered",
    }
)
_OPTION_KEYS = _FIELD_OPTION_KEYS | {"arrow_type"}
_EXTENSION_METADATA_PREFIX = "ARROW:extension:"
_PARQUET_FIELD_ID = "PARQUET:field_id"
_I32_MIN = -(2**31)
_I32_MAX = 2**31 - 1
_I64_MIN = -(2**63)
_I64_MAX = 2**63 - 1


@dataclasses.dataclass(slots=True)
class _AnnotationOptions:
    """Ephemeral Python options; native Field remains the only schema value."""

    arrow_type: object = _MISSING
    nullable: object = _MISSING
    metadata: dict[str, str] = dataclasses.field(default_factory=dict)
    id: object = _MISSING
    dictionary_id: object = _MISSING
    dictionary_is_ordered: object = _MISSING
    field_keys: set[str] = dataclasses.field(default_factory=set)


def datatype_from_pyhint(hint: object) -> DataType:
    """Return the native Arrow-equivalent datatype for *hint*.

    ``None`` is removed from optional unions because nullability belongs to a
    field, not a datatype. A union containing only ``None`` maps to ``Null``.
    """

    return _datatype_from_pyhint(hint)


def _datatype_from_pyhint(
    hint: object,
    *,
    localns: cabc.Mapping[str, object] | None = None,
    resolved_cache: cabc.MutableMapping[
        type[Any], cabc.Mapping[str, object]
    ]
    | None = None,
) -> DataType:
    """Internal namespace-aware variant used by the records schema cache."""

    return _Inference(localns, resolved_cache).datatype(
        hint,
        path="annotation",
        depth=0,
    )


def field_from_pyhint(
    name: str,
    hint: object,
    metadata: cabc.Mapping[str, str]
    | cabc.Iterable[tuple[str, str]]
    | None = None,
) -> Field:
    """Return a native field inferred from *name* and *hint*.

    Only an explicit ``None`` annotation, optional union, or ``Literal`` that
    contains ``None`` makes the field nullable. Defaults are deliberately not
    inspected. Metadata embedded in ``Annotated`` is applied first and the
    explicit *metadata* argument wins on duplicate keys.
    """

    return _field_from_pyhint(name, hint, metadata=metadata)


def _field_from_pyhint(
    name: str,
    hint: object,
    metadata: cabc.Mapping[str, str]
    | cabc.Iterable[tuple[str, str]]
    | None = None,
    *,
    localns: cabc.Mapping[str, object] | None = None,
    resolved_cache: cabc.MutableMapping[
        type[Any], cabc.Mapping[str, object]
    ]
    | None = None,
) -> Field:
    """Internal namespace-aware variant used by the records schema cache."""

    if not isinstance(name, str):
        raise TypeError("field name must be str")
    return _Inference(localns, resolved_cache).field(
        name,
        hint,
        metadata=metadata,
        path=name or "field",
        depth=0,
    )


class _Inference:
    """One bounded inference traversal with cycle detection."""

    def __init__(
        self,
        localns: cabc.Mapping[str, object] | None = None,
        resolved_cache: cabc.MutableMapping[
            type[Any], cabc.Mapping[str, object]
        ]
        | None = None,
    ) -> None:
        self._active_classes: set[int] = set()
        self._localns = dict(localns or ())
        self._resolved_cache = resolved_cache

    def datatype(self, hint: object, *, path: str, depth: int) -> DataType:
        self._check_depth(path, depth)
        hint, annotation_extras = _unwrap_annotation(hint)
        field_keys = _recognized_field_option_keys(annotation_extras)
        if field_keys:
            keys = ", ".join(sorted(field_keys))
            raise TypeError(
                f"Annotated option(s) {keys} at {path} apply to a Field; "
                "use Field.from_pyhint or a record annotation"
            )
        options = _annotation_options(annotation_extras, path)
        if options.arrow_type is not _MISSING:
            return _datatype_from_override(options.arrow_type, path)

        if isinstance(hint, DataType):
            return hint
        if isinstance(hint, Field):
            return hint.data_type
        if hint is None or hint is _NONE_TYPE:
            return _native_datatype("null")
        if hint is Any or hint is object or hint in _no_value_hints():
            return _native_datatype("null")

        alias_value = _expanded_type_alias(hint)
        if alias_value is not _MISSING:
            return self.datatype(
                alias_value,
                path=path,
                depth=depth + 1,
            )

        new_type_base = getattr(hint, "__supertype__", None)
        if new_type_base is not None:
            return self.datatype(new_type_base, path=path, depth=depth + 1)

        if isinstance(hint, typing.TypeVar):
            constraints = hint.__constraints__
            if constraints:
                return self._alternatives(
                    constraints,
                    path=path,
                    depth=depth + 1,
                )
            if hint.__bound__ is not None:
                return self.datatype(hint.__bound__, path=path, depth=depth + 1)
            return _native_datatype("null")

        if isinstance(hint, typing.ForwardRef):
            return self.datatype(
                _resolve_direct_forward(hint.__forward_arg__, path),
                path=path,
                depth=depth + 1,
            )
        if isinstance(hint, str):
            return self.datatype(
                _resolve_direct_forward(hint, path),
                path=path,
                depth=depth + 1,
            )

        origin = typing.get_origin(hint)
        arguments = typing.get_args(hint)

        if origin is typing.Literal:
            literal_hints = [type(value) for value in arguments if value is not None]
            if not literal_hints:
                return _native_datatype("null")
            return self._alternatives(
                literal_hints,
                path=path,
                depth=depth + 1,
                collapse_equivalent=True,
            )

        if origin in _union_origins():
            members = tuple(member for member in arguments if not _is_none_hint(member))
            if not members:
                return _native_datatype("null")
            if len(members) == 1:
                return self.datatype(
                    members[0], path=path, depth=depth + 1
                )
            return self._alternatives(members, path=path, depth=depth + 1)

        if origin is not None:
            generic = self._generic_datatype(
                hint,
                origin,
                arguments,
                path=path,
                depth=depth + 1,
            )
            if generic is not None:
                return generic

        if isinstance(hint, type):
            scalar = self._class_datatype(hint, path=path, depth=depth + 1)
            if scalar is not None:
                return scalar

        raise TypeError(f"unsupported Python type hint at {path}: {_display_hint(hint)}")

    def field(
        self,
        name: str,
        hint: object,
        *,
        metadata: cabc.Mapping[str, str]
        | cabc.Iterable[tuple[str, str]]
        | None = None,
        path: str,
        depth: int,
    ) -> Field:
        self._check_depth(path, depth)
        identity = _class_identity_metadata(hint)
        base, annotation_extras = _unwrap_annotation(hint)
        options = _annotation_options(annotation_extras, path)
        promoted_member = (
            _single_physical_wrapper_member(base)
            if options.arrow_type is _MISSING
            else _MISSING
        )
        overlay = dict(options.metadata)
        if metadata is not None:
            overlay.update(_string_metadata(metadata, path))
        member_nullable = (
            _explicit_nullable(promoted_member, path)
            if promoted_member is not _MISSING
            else _MISSING
        )
        if options.nullable is not _MISSING:
            nullable = typing.cast(bool, options.nullable)
        elif member_nullable is not _MISSING:
            nullable = typing.cast(bool, member_nullable)
        else:
            nullable = _allows_none(base)

        imported: Field | None = None
        explicit_extension = False
        if options.arrow_type is _MISSING:
            if promoted_member is not _MISSING:
                imported = self.field(
                    name,
                    promoted_member,
                    path=path,
                    depth=depth + 1,
                )
                data_type = imported.data_type
                explicit_extension = any(
                    key.startswith(_EXTENSION_METADATA_PREFIX)
                    for key in imported.metadata
                )
            else:
                data_type = self.datatype(base, path=path, depth=depth + 1)
        else:
            self._validate_logical_graph(base, path=path, depth=depth + 1)
            data_type, imported, explicit_extension = _field_type_from_override(
                name,
                options.arrow_type,
                nullable,
                path,
            )

        imported_metadata = dict(identity)
        if imported is not None:
            imported_metadata.update(imported.metadata.items())
        if options.id is not _MISSING:
            imported_metadata.pop(_PARQUET_FIELD_ID, None)
        if explicit_extension:
            _validate_extension_metadata(imported_metadata, overlay, path)
        imported_metadata.update(overlay)
        result = Field(
            name,
            data_type,
            nullable=nullable,
            metadata=imported_metadata or None,
        )

        if options.id is not _MISSING:
            option_id = typing.cast(int, options.id)
            if _PARQUET_FIELD_ID in overlay:
                if result.parquet_field_id != option_id:
                    raise TypeError(
                        f"conflicting Annotated id and {_PARQUET_FIELD_ID} "
                        f"metadata at {path}"
                    )
            else:
                result.set_parquet_field_id(option_id)

        dictionary_id = options.dictionary_id
        dictionary_is_ordered = options.dictionary_is_ordered
        if dictionary_id is _MISSING and imported is not None:
            imported_id = imported.dictionary_id
            imported_ordered = imported.dictionary_is_ordered
            if imported_id is not None and imported_ordered is not None:
                dictionary_id = imported_id
                dictionary_is_ordered = imported_ordered
        if dictionary_id is not _MISSING:
            try:
                result.set_dictionary_options(
                    typing.cast(int, dictionary_id),
                    typing.cast(bool, dictionary_is_ordered),
                )
            except (TypeError, ValueError, OverflowError) as error:
                raise TypeError(
                    f"invalid Annotated dictionary options at {path}: {error}"
                ) from error
        return result

    def _generic_datatype(
        self,
        hint: object,
        origin: object,
        arguments: tuple[object, ...],
        *,
        path: str,
        depth: int,
    ) -> DataType | None:
        if origin in _binary_origins():
            return _native_datatype("binary")
        if origin in _string_origins():
            return _native_datatype("utf8")
        if origin in _mapping_origins() or (
            isinstance(origin, type) and issubclass(origin, cabc.Mapping)
        ):
            mapping_key_hint: object
            mapping_value_hint: object
            if isinstance(origin, type) and issubclass(origin, collections.Counter):
                mapping_key_hint = arguments[0] if arguments else Any
                mapping_value_hint = int
            else:
                mapping_arguments = arguments + (Any, Any)
                mapping_key_hint = mapping_arguments[0]
                mapping_value_hint = mapping_arguments[1]
            return self._map(
                mapping_key_hint,
                mapping_value_hint,
                path=path,
                depth=depth,
            )
        if origin in _items_view_origins():
            item_arguments = arguments + (Any, Any)
            item_key_hint = item_arguments[0]
            item_value_hint = item_arguments[1]
            item = _struct_datatype(
                [
                    self.field(
                        "_1",
                        item_key_hint,
                        path=f"{path}[].key",
                        depth=depth + 1,
                    ),
                    self.field(
                        "_2",
                        item_value_hint,
                        path=f"{path}[].value",
                        depth=depth + 1,
                    ),
                ]
            )
            item_field = Field("item", item, nullable=False)
            return DataType._list("list", item_field)
        if origin is tuple:
            if hint is typing.Tuple:
                return self._list(Any, path=path, depth=depth)
            if len(arguments) == 2 and arguments[1] is Ellipsis:
                return self._list(arguments[0], path=path, depth=depth)
            fields = [
                self.field(
                    f"_{index}",
                    member,
                    path=f"{path}[{index - 1}]",
                    depth=depth + 1,
                )
                for index, member in enumerate(arguments, start=1)
            ]
            return _struct_datatype(fields)
        if origin in _sequence_origins():
            item_hint = arguments[0] if arguments else Any
            return self._list(item_hint, path=path, depth=depth)
        if origin is type:
            return _native_datatype("utf8")
        if origin in _callable_origins():
            return _native_datatype("null")

        if isinstance(origin, type):
            if _is_struct_class(origin):
                bindings = dict(zip(getattr(origin, "__parameters__", ()), arguments))
                return self._struct_class(
                    origin,
                    bindings=bindings,
                    path=path,
                    depth=depth,
                )
            return self._class_datatype(origin, path=path, depth=depth)
        return None

    def _class_datatype(
        self,
        hint: type[Any],
        *,
        path: str,
        depth: int,
    ) -> DataType | None:
        direct = _DIRECT_CLASS_TYPES.get(hint)
        if direct is not None:
            if direct == "complex":
                return _complex_datatype("float64")
            if direct == "range":
                return self._list(int, path=path, depth=depth)
            return _native_datatype(direct)

        if hint in (Uri, Url, Urn):
            return _native_datatype("utf8")
        if issubclass(hint, enum.Enum):
            values = [type(member.value) for member in hint]
            return (
                self._alternatives(
                    values,
                    path=path,
                    depth=depth,
                    collapse_equivalent=True,
                )
                if values
                else _native_datatype("utf8")
            )
        if _is_declared_struct_class(hint):
            return self._struct_class(hint, bindings={}, path=path, depth=depth)
        numpy_type = _numpy_datatype(hint)
        if numpy_type is not None:
            return numpy_type
        if issubclass(hint, datetime_module.datetime):
            return _native_datatype('timestamp(microsecond,"UTC")')
        if issubclass(hint, datetime_module.date):
            return _native_datatype("date32")
        if issubclass(hint, datetime_module.time):
            return _native_datatype("time64(microsecond)")
        if issubclass(hint, datetime_module.timedelta):
            return _native_datatype("duration(microsecond)")
        if issubclass(hint, decimal.Decimal):
            return _native_datatype("decimal128(38,18)")
        if issubclass(hint, uuid.UUID):
            return _native_datatype("utf8")
        if issubclass(hint, pathlib.PurePath) or issubclass(hint, os.PathLike):
            return _native_datatype("utf8")
        if issubclass(hint, re.Pattern):
            return _native_datatype("utf8")
        if issubclass(hint, str):
            return _native_datatype("utf8")
        if issubclass(hint, (bytes, bytearray)):
            return _native_datatype("binary")
        if issubclass(hint, numbers.Integral):
            return _native_datatype("int64")
        if issubclass(hint, numbers.Real) or issubclass(hint, fractions.Fraction):
            return _native_datatype("float64")
        if issubclass(hint, collections.Counter):
            return self._map(Any, int, path=path, depth=depth)
        if issubclass(hint, cabc.Mapping):
            return self._map(Any, Any, path=path, depth=depth)
        if issubclass(hint, tuple) and not _is_named_tuple(hint):
            return self._list(Any, path=path, depth=depth)
        if issubclass(hint, _sequence_classes()):
            return self._list(Any, path=path, depth=depth)
        if _is_struct_class(hint):
            return self._struct_class(hint, bindings={}, path=path, depth=depth)
        # Arbitrary leaf classes have a stable string representation but no
        # richer lossless Arrow primitive. Utf8 is the conservative boundary.
        return _native_datatype("utf8")

    def _list(self, item_hint: object, *, path: str, depth: int) -> DataType:
        item = self.field(
            "item",
            item_hint,
            path=f"{path}[]",
            depth=depth + 1,
        )
        return DataType._list("list", item)

    def _map(
        self,
        key_hint: object,
        value_hint: object,
        *,
        path: str,
        depth: int,
    ) -> DataType:
        key = self.field(
            "key",
            key_hint,
            path=f"{path}.key",
            depth=depth + 1,
        )
        if key.nullable:
            raise TypeError(f"nullable map key is not representable in Arrow at {path}")
        value = self.field(
            "value",
            value_hint,
            path=f"{path}.value",
            depth=depth + 1,
        )
        entries = Field(
            "entries",
            DataType.from_fields((key, value)),
            nullable=False,
        )
        return DataType._map(entries, False)

    def _alternatives(
        self,
        hints: cabc.Iterable[object],
        *,
        path: str,
        depth: int,
        collapse_equivalent: bool = False,
    ) -> DataType:
        unique: list[tuple[object, Field]] = []
        for hint in hints:
            if _is_none_hint(hint):
                continue
            inferred = self.field(
                "member",
                hint,
                path=path,
                depth=depth + 1,
            )
            data_type = inferred.data_type
            if not collapse_equivalent or not any(
                existing.data_type == data_type for _, existing in unique
            ):
                unique.append((hint, inferred))

        if not unique:
            return _native_datatype("null")
        if len(unique) == 1:
            return unique[0][1].data_type
        if len(unique) > 128:
            raise TypeError(f"union at {path} exceeds Arrow's 128-member limit")

        used_names: dict[str, int] = {}
        fields: list[Field] = []
        for hint, inferred in unique:
            base_name = _union_member_name(hint)
            suffix = used_names.get(base_name, 0)
            used_names[base_name] = suffix + 1
            name = base_name if suffix == 0 else f"{base_name}_{suffix + 1}"
            fields.append(_renamed_field(inferred, name))
        return DataType.variant(fields)

    def _struct_class(
        self,
        cls: type[Any],
        *,
        bindings: cabc.Mapping[object, object],
        path: str,
        depth: int,
    ) -> DataType:
        marker = id(cls)
        if marker in self._active_classes:
            raise TypeError(
                f"recursive Python annotation at {path}: "
                f"{cls.__module__}.{cls.__qualname__}"
            )
        self._active_classes.add(marker)
        try:
            resolved_bindings = _inherited_typevar_bindings(cls, bindings)
            self_hints = [getattr(typing, "Self", None)]
            if _typing_extensions is not None:
                self_hints.append(getattr(_typing_extensions, "Self", None))
            for self_hint in self_hints:
                if self_hint is not None:
                    resolved_bindings[self_hint] = cls
            bindings = resolved_bindings
            cached_annotations = (
                self._resolved_cache.get(cls)
                if self._resolved_cache is not None
                else None
            )
            if cached_annotations is None:
                annotations = _resolved_annotations(cls, path, self._localns)
                if self._resolved_cache is not None:
                    self._resolved_cache[cls] = types.MappingProxyType(
                        dict(annotations)
                    )
            else:
                annotations = dict(cached_annotations)
            fields: list[Field] = []
            if dataclasses.is_dataclass(cls):
                dataclass_fields = dataclasses.fields(cls)
                materialized = {field.name for field in dataclass_fields}
                for name, annotation in annotations.items():
                    if name not in materialized:
                        _reject_non_materialized_options(
                            annotation, f"{path}.{name}"
                        )
                for dataclass_field in dataclass_fields:
                    child_hint = annotations.get(dataclass_field.name, dataclass_field.type)
                    child_hint = _bind_typevars(child_hint, bindings)
                    child_metadata = {
                        key: value
                        for key, value in dataclass_field.metadata.items()
                        if isinstance(key, str) and isinstance(value, str)
                    }
                    fields.append(
                        self.field(
                            dataclass_field.name,
                            child_hint,
                            metadata=child_metadata,
                            path=f"{path}.{dataclass_field.name}",
                            depth=depth + 1,
                        )
                    )
            else:
                names = getattr(cls, "_fields", annotations.keys())
                for name in names:
                    if name not in annotations:
                        continue
                    if _is_classvar_annotation(annotations[name]):
                        _reject_non_materialized_options(
                            annotations[name], f"{path}.{name}"
                        )
                        continue
                    child_hint = _bind_typevars(annotations[name], bindings)
                    fields.append(
                        self.field(
                            name,
                            child_hint,
                            path=f"{path}.{name}",
                            depth=depth + 1,
                        )
                    )
            return _struct_datatype(fields)
        finally:
            self._active_classes.remove(marker)

    def _validate_logical_graph(
        self,
        hint: object,
        *,
        path: str,
        depth: int,
    ) -> None:
        """Bound a shadowed logical hint without materializing physical options."""

        self._check_depth(path, depth)
        hint, _ = _unwrap_annotation(hint)
        if hint is None or hint in (Any, object, _NONE_TYPE, Ellipsis):
            return
        new_type_base = getattr(hint, "__supertype__", None)
        if new_type_base is not None:
            self._validate_logical_graph(
                new_type_base, path=path, depth=depth + 1
            )
            return
        if isinstance(hint, typing.TypeVar):
            members = hint.__constraints__ or (
                (hint.__bound__,) if hint.__bound__ is not None else ()
            )
            for member in members:
                self._validate_logical_graph(
                    member, path=path, depth=depth + 1
                )
            return
        if isinstance(hint, typing.ForwardRef):
            value = hint.__forward_arg__
            resolved = self._localns.get(value, _MISSING)
            if resolved is _MISSING:
                resolved = _resolve_direct_forward(value, path)
            self._validate_logical_graph(
                resolved, path=path, depth=depth + 1
            )
            return
        if isinstance(hint, str):
            resolved = self._localns.get(hint, _MISSING)
            if resolved is _MISSING:
                resolved = _resolve_direct_forward(hint, path)
            self._validate_logical_graph(
                resolved, path=path, depth=depth + 1
            )
            return

        origin = typing.get_origin(hint)
        arguments = typing.get_args(hint)
        candidate = origin or hint
        if isinstance(candidate, type) and _is_struct_class(candidate):
            marker = id(candidate)
            if marker in self._active_classes:
                raise TypeError(
                    f"recursive Python annotation at {path}: "
                    f"{candidate.__module__}.{candidate.__qualname__}"
                )
            self._active_classes.add(marker)
            try:
                bindings = dict(
                    zip(getattr(candidate, "__parameters__", ()), arguments)
                )
                annotations = _resolved_annotations(
                    candidate, path, self._localns
                )
                if dataclasses.is_dataclass(candidate):
                    materialized = {
                        field.name for field in dataclasses.fields(candidate)
                    }
                    for name, annotation in annotations.items():
                        if name not in materialized:
                            _reject_non_materialized_options(
                                annotation, f"{path}.{name}"
                            )
                for name, annotation in annotations.items():
                    if _is_classvar_annotation(annotation):
                        _reject_non_materialized_options(
                            annotation, f"{path}.{name}"
                        )
                        continue
                    self._validate_logical_graph(
                        _bind_typevars(annotation, bindings),
                        path=f"{path}.{name}",
                        depth=depth + 1,
                    )
            finally:
                self._active_classes.remove(marker)
            return
        for index, argument in enumerate(arguments):
            if argument is Ellipsis or not _is_hint_like(argument):
                continue
            self._validate_logical_graph(
                argument,
                path=f"{path}[{index}]",
                depth=depth + 1,
            )

    @staticmethod
    def _check_depth(path: str, depth: int) -> None:
        if depth > _MAX_INFERENCE_DEPTH:
            raise TypeError(
                f"Python annotation depth exceeds {_MAX_INFERENCE_DEPTH} at {path}"
            )


def _unwrap_annotation(hint: object) -> tuple[object, tuple[object, ...]]:
    extras: list[object] = []
    transparent = _transparent_origins()
    expanded_aliases: set[int] = set()
    inside_alias = False
    while True:
        alias_value = _expanded_type_alias(hint)
        if alias_value is not _MISSING:
            alias = typing.get_origin(hint) or hint
            marker = id(alias)
            if marker in expanded_aliases:
                raise TypeError(f"recursive Python type alias: {_display_hint(alias)}")
            expanded_aliases.add(marker)
            hint = alias_value
            inside_alias = True
            continue
        if isinstance(hint, dataclasses.InitVar):
            hint = hint.type
            continue
        origin = typing.get_origin(hint)
        arguments = typing.get_args(hint)
        if origin in _annotated_origins():
            hint = arguments[0]
            if inside_alias:
                extras[:0] = arguments[1:]
            else:
                extras.extend(arguments[1:])
            continue
        if origin in transparent and arguments:
            hint = arguments[0]
            continue
        return hint, tuple(extras)


def _allows_none(hint: object) -> bool:
    hint, _ = _unwrap_annotation(hint)
    if _is_none_hint(hint):
        return True
    alias_value = _expanded_type_alias(hint)
    if alias_value is not _MISSING:
        return _allows_none(alias_value)
    supertype = getattr(hint, "__supertype__", None)
    if supertype is not None:
        return _allows_none(supertype)
    if isinstance(hint, typing.TypeVar):
        if hint.__constraints__:
            return any(_allows_none(member) for member in hint.__constraints__)
        return hint.__bound__ is not None and _allows_none(hint.__bound__)
    origin = typing.get_origin(hint)
    arguments = typing.get_args(hint)
    if origin in _union_origins():
        return any(_allows_none(member) for member in arguments)
    if origin is typing.Literal:
        return any(value is None for value in arguments)
    return False


def _is_none_hint(hint: object) -> bool:
    hint, _ = _unwrap_annotation(hint)
    if hint is None or hint is _NONE_TYPE:
        return True
    supertype = getattr(hint, "__supertype__", None)
    if supertype is not None:
        return _is_none_hint(supertype)
    if isinstance(hint, typing.TypeVar):
        if hint.__constraints__:
            return all(_is_none_hint(member) for member in hint.__constraints__)
        return hint.__bound__ is not None and _is_none_hint(hint.__bound__)
    return (
        typing.get_origin(hint) is typing.Literal
        and bool(typing.get_args(hint))
        and all(value is None for value in typing.get_args(hint))
    )


def _is_hint_like(value: object) -> bool:
    return (
        value is None
        or isinstance(value, (type, str, typing.ForwardRef, typing.TypeVar))
        or typing.get_origin(value) is not None
        or getattr(value, "__supertype__", None) is not None
    )


def _semantic_alternatives(hint: object) -> tuple[object, ...]:
    hint, _ = _unwrap_annotation(hint)
    supertype = getattr(hint, "__supertype__", None)
    if supertype is not None:
        return _semantic_alternatives(supertype)
    if isinstance(hint, typing.TypeVar):
        if hint.__constraints__:
            return tuple(hint.__constraints__)
        if hint.__bound__ is not None:
            return _semantic_alternatives(hint.__bound__)
        return (hint,)
    if typing.get_origin(hint) in _union_origins():
        return typing.get_args(hint)
    return (hint,)


def _single_physical_wrapper_member(hint: object) -> object:
    hint, _ = _unwrap_annotation(hint)
    supertype = getattr(hint, "__supertype__", None)
    if supertype is not None:
        return supertype
    if isinstance(hint, typing.TypeVar):
        if hint.__bound__ is not None:
            return hint.__bound__
        members = tuple(
            member
            for member in hint.__constraints__
            if not _is_none_hint(member)
        )
        return members[0] if len(members) == 1 else _MISSING
    alternatives = _semantic_alternatives(hint)
    if not any(_is_none_hint(member) for member in alternatives):
        return _MISSING
    members = tuple(
        member for member in alternatives if not _is_none_hint(member)
    )
    return members[0] if len(members) == 1 else _MISSING


def _explicit_nullable(hint: object, path: str) -> object:
    base, extras = _unwrap_annotation(hint)
    options = _annotation_options(extras, path)
    if options.nullable is not _MISSING:
        return options.nullable
    member = _single_physical_wrapper_member(base)
    return (
        _explicit_nullable(member, path)
        if member is not _MISSING
        else _MISSING
    )


def _is_classvar_annotation(hint: object) -> bool:
    if isinstance(hint, dataclasses.InitVar):
        return False
    origin = typing.get_origin(hint)
    if origin is typing.ClassVar:
        return True
    arguments = typing.get_args(hint)
    if origin in _annotated_origins() and arguments:
        return _is_classvar_annotation(arguments[0])
    alias_value = _expanded_type_alias(hint)
    return alias_value is not _MISSING and _is_classvar_annotation(alias_value)


def _annotation_options(
    extras: cabc.Iterable[object], path: str
) -> _AnnotationOptions:
    options = _AnnotationOptions()
    for extra in extras:
        if (
            isinstance(extra, tuple)
            and extra
            and isinstance(extra[0], str)
            and extra[0] in _OPTION_KEYS
            and len(extra) != 2
        ):
            raise TypeError(
                f"Annotated option tuple for {extra[0]!r} at {path} must be "
                "exactly (key, value)"
            )
        if isinstance(extra, Field):
            options.metadata.update(extra.items())
            continue
        if isinstance(extra, cabc.Mapping):
            try:
                pairs = tuple(extra.items())
            except (TypeError, ValueError) as error:
                raise TypeError(
                    f"Annotated options at {path} must contain key/value pairs"
                ) from error
            keys = {key for key, _ in pairs if isinstance(key, str)}
            has_dictionary_id = "dictionary_id" in keys
            has_dictionary_order = "dictionary_is_ordered" in keys
            if has_dictionary_id != has_dictionary_order:
                raise TypeError(
                    "dictionary_id and dictionary_is_ordered must appear together "
                    f"in an Annotated option mapping at {path}"
                )
            for key, value in pairs:
                if isinstance(key, str) and key in _OPTION_KEYS:
                    _set_annotation_option(options, key, value, path)
                elif isinstance(key, str) and isinstance(value, str):
                    options.metadata[key] = value
                else:
                    raise TypeError(
                        f"unknown Annotated options at {path} must map str keys "
                        "to str values for metadata"
                    )
            continue
        if (
            isinstance(extra, tuple)
            and len(extra) == 2
            and isinstance(extra[0], str)
        ):
            key, value = extra
            if key in _OPTION_KEYS:
                _set_annotation_option(options, key, value, path)
            elif isinstance(value, str):
                options.metadata[key] = value

    if (options.dictionary_id is _MISSING) != (
        options.dictionary_is_ordered is _MISSING
    ):
        raise TypeError(
            "dictionary_id and dictionary_is_ordered must both be supplied "
            f"through Annotated options at {path}"
        )
    _validate_final_annotation_options(options, path)
    return options


def _recognized_field_option_keys(
    extras: cabc.Iterable[object],
) -> set[str]:
    keys: set[str] = set()
    for extra in extras:
        if isinstance(extra, cabc.Mapping):
            try:
                candidates = extra.keys()
            except (TypeError, ValueError):
                continue
            keys.update(
                key
                for key in candidates
                if isinstance(key, str) and key in _FIELD_OPTION_KEYS
            )
        elif (
            isinstance(extra, tuple)
            and len(extra) == 2
            and isinstance(extra[0], str)
            and extra[0] in _FIELD_OPTION_KEYS
        ):
            keys.add(extra[0])
    return keys


def _reject_non_materialized_options(hint: object, path: str) -> None:
    active: set[int] = set()

    def scan(current: object, current_path: str, depth: int) -> None:
        if depth > _MAX_INFERENCE_DEPTH:
            raise TypeError(
                f"Python annotation depth exceeds {_MAX_INFERENCE_DEPTH} "
                f"at {current_path}"
            )
        marker = id(current)
        if marker in active:
            return
        active.add(marker)
        try:
            base, extras = _unwrap_annotation(current)
            options = _annotation_options(extras, current_path)
            keys = set(options.field_keys)
            if options.arrow_type is not _MISSING:
                keys.add("arrow_type")
            if options.metadata:
                keys.add("metadata")
            if keys:
                rendered = ", ".join(sorted(keys))
                raise TypeError(
                    f"Annotated option(s) {rendered} at {current_path} "
                    "require a materialized record Field; InitVar and "
                    "ClassVar values are not schema fields"
                )

            new_type_base = getattr(base, "__supertype__", None)
            if new_type_base is not None:
                scan(new_type_base, current_path, depth + 1)
                return
            if isinstance(base, typing.TypeVar):
                members = base.__constraints__ or (
                    (base.__bound__,) if base.__bound__ is not None else ()
                )
                for index, member in enumerate(members):
                    scan(member, f"{current_path}[{index}]", depth + 1)
                return

            for index, argument in enumerate(typing.get_args(base)):
                if argument is Ellipsis or not _is_hint_like(argument):
                    continue
                scan(argument, f"{current_path}[{index}]", depth + 1)
        finally:
            active.remove(marker)

    scan(hint, path, 0)


def _set_annotation_option(
    options: _AnnotationOptions,
    key: str,
    value: object,
    path: str,
) -> None:
    if key == "arrow_type":
        options.arrow_type = value
        return
    options.field_keys.add(key)
    if key == "metadata":
        if isinstance(value, (str, bytes, bytearray, memoryview)):
            raise TypeError(f"metadata option at {path} must contain key/value pairs")
        options.metadata.update(_string_metadata(typing.cast(Any, value), path))
    elif key == "nullable":
        options.nullable = value
    elif key == "id":
        options.id = value
    elif key == "dictionary_id":
        options.dictionary_id = value
    elif key == "dictionary_is_ordered":
        options.dictionary_is_ordered = value


def _validate_final_annotation_options(
    options: _AnnotationOptions,
    path: str,
) -> None:
    if options.nullable is not _MISSING and type(options.nullable) is not bool:
        raise TypeError(f"nullable option at {path} must be bool")
    if options.id is not _MISSING:
        options.id = _exact_integer(
            options.id, "id", path, _I32_MIN, _I32_MAX
        )
    if options.dictionary_id is not _MISSING:
        options.dictionary_id = _exact_integer(
            options.dictionary_id,
            "dictionary_id",
            path,
            _I64_MIN,
            _I64_MAX,
        )
    if (
        options.dictionary_is_ordered is not _MISSING
        and type(options.dictionary_is_ordered) is not bool
    ):
        raise TypeError(
            f"dictionary_is_ordered option at {path} must be bool"
        )


def _exact_integer(
    value: object,
    key: str,
    path: str,
    minimum: int,
    maximum: int,
) -> int:
    if type(value) is not int:
        raise TypeError(f"{key} option at {path} must be int")
    integer = value
    if integer < minimum or integer > maximum:
        raise ValueError(
            f"{key} option at {path} must be in {minimum}..={maximum}"
        )
    return integer


def _datatype_from_override(value: object, path: str) -> DataType:
    pa = _pyarrow_for_override(path)
    if not isinstance(value, pa.DataType):
        raise TypeError(
            f"arrow_type option at {path} must be a pyarrow.DataType"
        )
    if _is_pyarrow_extension_type(pa, value):
        raise TypeError(
            f"arrow_type ExtensionType at {path} carries Field metadata; use "
            "Field.from_pyhint or a record annotation to preserve its identity"
        )
    return DataType.from_arrow(value)


def _field_type_from_override(
    name: str,
    value: object,
    nullable: bool,
    path: str,
) -> tuple[DataType, Field | None, bool]:
    pa = _pyarrow_for_override(path)
    if not isinstance(value, pa.DataType):
        raise TypeError(
            f"arrow_type option at {path} must be a pyarrow.DataType"
        )
    is_extension = _is_pyarrow_extension_type(pa, value)
    try:
        imported = Field.from_arrow(pa.field(name, value, nullable=nullable))
    except Exception as error:
        if is_extension and "utf-8" in str(error).lower():
            raise TypeError(
                f"ExtensionType arrow_type at {path} requires UTF-8 "
                "serialized extension metadata"
            ) from error
        raise
    return imported.data_type, imported, is_extension


def _is_pyarrow_extension_type(pa: Any, value: object) -> bool:
    base = getattr(pa, "BaseExtensionType", pa.ExtensionType)
    return isinstance(value, base)


def _validate_extension_metadata(
    imported: cabc.Mapping[str, str],
    overlay: cabc.Mapping[str, str],
    path: str,
) -> None:
    for key, value in overlay.items():
        if not key.startswith(_EXTENSION_METADATA_PREFIX):
            continue
        if imported.get(key) != value:
            raise TypeError(
                f"metadata at {path} conflicts with explicit ExtensionType "
                f"identity key {key!r}"
            )


def _renamed_field(field: Field, name: str) -> Field:
    renamed = Field(
        name,
        field.data_type,
        nullable=field.nullable,
        metadata=dict(field.metadata.items()) or None,
    )
    if field.dictionary_id is not None and field.dictionary_is_ordered is not None:
        renamed.set_dictionary_options(
            field.dictionary_id,
            field.dictionary_is_ordered,
        )
    return renamed


@functools.cache
def _pyarrow_module() -> Any:
    return importlib.import_module("pyarrow")


def _pyarrow_for_override(path: str) -> Any:
    try:
        return _pyarrow_module()
    except ImportError as error:
        raise TypeError(
            f"pyarrow is required for the explicit arrow_type option at {path}"
        ) from error


def _string_metadata(
    values: cabc.Mapping[str, str] | cabc.Iterable[tuple[str, str]], path: str
) -> dict[str, str]:
    try:
        pairs = values.items() if isinstance(values, cabc.Mapping) else values
        metadata = dict(pairs)
    except (TypeError, ValueError) as error:
        raise TypeError(f"metadata at {path} must contain key/value pairs") from error
    for key, value in metadata.items():
        if not isinstance(key, str) or not isinstance(value, str):
            raise TypeError(f"metadata at {path} must map str keys to str values")
    return metadata


def _class_identity_metadata(hint: object) -> dict[str, str]:
    original = hint
    while True:
        origin = typing.get_origin(original)
        arguments = typing.get_args(original)
        if origin in _annotated_origins() and arguments:
            original = arguments[0]
            continue
        if origin in _transparent_origins() and arguments:
            original = arguments[0]
            continue
        break

    alias_target = typing.get_origin(original) or original
    if type(alias_target).__name__ == "TypeAliasType":
        target = alias_target
        kind = "type_alias"
    else:
        hint, _ = _unwrap_annotation(original)
        if hint is Any or hint is object or hint is None or hint is _NONE_TYPE:
            return {}
        non_none = (
            [
                member
                for member in typing.get_args(hint)
                if not _is_none_hint(member)
            ]
            if typing.get_origin(hint) in _union_origins()
            else []
        )
        if len(non_none) == 1:
            return _class_identity_metadata(non_none[0])

        if getattr(hint, "__supertype__", None) is not None:
            target = hint
            kind = "newtype"
        else:
            target = typing.get_origin(hint) or hint
            if not isinstance(target, type) or target.__module__ == "builtins":
                return {}
            if target.__dict__.get("__yggdryl_record__", False):
                kind = "record"
            elif dataclasses.is_dataclass(target):
                kind = "dataclass"
            elif _is_typed_dict(target):
                kind = "typed_dict"
            elif _is_named_tuple(target):
                kind = "named_tuple"
            elif issubclass(target, enum.Enum):
                kind = "enum"
            else:
                kind = "class"

    if target is Any or target is object or target is None or target is _NONE_TYPE:
        return {}
    module = getattr(target, "__module__", None)
    name = getattr(target, "__name__", None)
    qualname = getattr(target, "__qualname__", name)
    if not isinstance(module, str) or not isinstance(name, str):
        return {}
    return {
        "python.module": module,
        "python.class": name,
        "python.qualname": qualname if isinstance(qualname, str) else name,
        "python.kind": kind,
    }


def _resolved_annotations(
    cls: type[Any],
    path: str,
    external_localns: cabc.Mapping[str, object] | None = None,
) -> dict[str, object]:
    module = sys.modules.get(cls.__module__)
    globalns = vars(module) if module is not None else {}
    localns: dict[str, object] = dict(external_localns or ())
    for base in reversed(cls.__mro__):
        localns.update(vars(base))
        localns[base.__name__] = base
    localns[cls.__name__] = cls
    try:
        return typing.get_type_hints(
            cls,
            globalns=globalns,
            localns=localns,
            include_extras=True,
        )
    except (NameError, TypeError) as error:
        raise TypeError(
            f"cannot resolve Python annotations for "
            f"{cls.__module__}.{cls.__qualname__} at {path}: {error}"
        ) from error


def _bind_typevars(
    hint: object, bindings: cabc.Mapping[object, object]
) -> object:
    try:
        if hint in bindings:
            return bindings[hint]
    except TypeError:
        pass
    if isinstance(hint, typing.TypeVar):
        return bindings.get(hint, hint)
    origin = typing.get_origin(hint)
    arguments = typing.get_args(hint)
    if origin is None or not arguments:
        return hint
    if origin in _annotated_origins():
        rebound_base = _bind_typevars(arguments[0], bindings)
        if rebound_base is arguments[0]:
            return hint
        annotated_arguments = (rebound_base, *arguments[1:])
        return typing.Annotated[annotated_arguments]
    rebound = tuple(_bind_typevars(argument, bindings) for argument in arguments)
    if rebound == arguments:
        return hint
    copier = getattr(hint, "copy_with", None)
    if copier is not None:
        try:
            return copier(rebound)
        except (AssertionError, TypeError, ValueError):
            pass
    if origin in _union_origins():
        result = rebound[0]
        for member in rebound[1:]:
            result = result | member  # type: ignore[operator]
        return result
    try:
        return origin[rebound[0] if len(rebound) == 1 else rebound]
    except (TypeError, AttributeError):
        return hint


def _inherited_typevar_bindings(
    cls: type[Any], initial: cabc.Mapping[object, object]
) -> dict[object, object]:
    resolved = dict(initial)
    visited: set[type[Any]] = set()

    def visit(current: type[Any], current_bindings: cabc.Mapping[object, object]) -> None:
        if current in visited:
            return
        visited.add(current)
        for base_hint in current.__dict__.get("__orig_bases__", ()):
            origin = typing.get_origin(base_hint) or base_hint
            if not isinstance(origin, type):
                continue
            arguments = tuple(
                _bind_typevars(argument, current_bindings)
                for argument in typing.get_args(base_hint)
            )
            parameters = getattr(origin, "__parameters__", ())
            base_bindings = dict(zip(parameters, arguments))
            for parameter, argument in base_bindings.items():
                resolved.setdefault(parameter, argument)
            visit(origin, base_bindings)

    visit(cls, initial)
    return resolved


def _expanded_type_alias(hint: object) -> object:
    origin = typing.get_origin(hint)
    alias = origin if type(origin).__name__ == "TypeAliasType" else hint
    if type(alias).__name__ != "TypeAliasType":
        return _MISSING
    value = getattr(alias, "__value__", None)
    parameters = getattr(alias, "__type_params__", ())
    arguments = typing.get_args(hint) if alias is origin else ()
    if parameters and arguments:
        value = _bind_typevars(value, dict(zip(parameters, arguments)))
    return value


def _struct_datatype(fields: cabc.Iterable[Field]) -> DataType:
    return DataType.from_fields(fields)


def _complex_datatype(component: str) -> DataType:
    return _struct_datatype(
        [
            Field("real", _native_datatype(component), nullable=False),
            Field("imag", _native_datatype(component), nullable=False),
        ]
    )


def _numpy_datatype(hint: type[Any]) -> DataType | None:
    if not hint.__module__.startswith("numpy"):
        return None
    name = hint.__name__
    mapping = {
        "bool_": "boolean",
        "int8": "int8",
        "int16": "int16",
        "int32": "int32",
        "int64": "int64",
        "uint8": "uint8",
        "uint16": "uint16",
        "uint32": "uint32",
        "uint64": "uint64",
        "float16": "float16",
        "float32": "float32",
        "float64": "float64",
        "str_": "utf8",
        "bytes_": "binary",
        "void": "binary",
        "datetime64": "timestamp(nanosecond)",
        "timedelta64": "duration(nanosecond)",
    }
    if name == "complex64":
        return _complex_datatype("float32")
    if name == "complex128":
        return _complex_datatype("float64")
    value = mapping.get(name)
    return _native_datatype(value) if value is not None else None


def _resolve_direct_forward(value: str, path: str) -> object:
    normalized = value.strip()
    direct: dict[str, object] = {
        "None": _NONE_TYPE,
        "NoneType": _NONE_TYPE,
        "Any": Any,
        "typing.Any": Any,
        "bool": bool,
        "int": int,
        "float": float,
        "complex": complex,
        "str": str,
        "bytes": bytes,
        "bytearray": bytearray,
        "memoryview": memoryview,
        "datetime": datetime_module.datetime,
        "datetime.datetime": datetime_module.datetime,
        "datetime.date": datetime_module.date,
        "datetime.time": datetime_module.time,
        "datetime.timedelta": datetime_module.timedelta,
        "Decimal": decimal.Decimal,
        "decimal.Decimal": decimal.Decimal,
        "UUID": uuid.UUID,
        "uuid.UUID": uuid.UUID,
        "Path": pathlib.Path,
        "pathlib.Path": pathlib.Path,
        "Uri": Uri,
        "Url": Url,
        "Urn": Urn,
    }
    if normalized in direct:
        return direct[normalized]
    raise TypeError(f"unresolved forward annotation at {path}: {value!r}")


def _union_member_name(hint: object) -> str:
    hint, _ = _unwrap_annotation(hint)
    target = typing.get_origin(hint) or hint
    name = getattr(target, "__name__", None)
    if not isinstance(name, str):
        name = str(target).rsplit(".", maxsplit=1)[-1]
    cleaned = "".join(character if character.isalnum() else "_" for character in name)
    return cleaned.strip("_") or "member"


def _display_hint(hint: object) -> str:
    module = getattr(hint, "__module__", None)
    qualname = getattr(hint, "__qualname__", None)
    if isinstance(module, str) and isinstance(qualname, str):
        return f"{module}.{qualname}"
    return repr(hint)


def _is_struct_class(value: object) -> bool:
    return isinstance(value, type) and (
        _is_declared_struct_class(value)
        or bool(getattr(value, "__annotations__", {}))
    )


def _is_declared_struct_class(value: object) -> bool:
    return isinstance(value, type) and (
        dataclasses.is_dataclass(value)
        or _is_typed_dict(value)
        or _is_named_tuple(value)
    )


def _is_typed_dict(value: object) -> bool:
    checker = getattr(typing, "is_typeddict", None)
    if checker is not None and checker(value):
        return True
    # typing_extensions.TypedDict on older supported Python versions is not
    # always recognized by typing.is_typeddict.
    return (
        isinstance(value, type)
        and hasattr(value, "__required_keys__")
        and hasattr(value, "__optional_keys__")
        and hasattr(value, "__total__")
        and isinstance(getattr(value, "__annotations__", None), dict)
    )


def _is_named_tuple(value: object) -> bool:
    return (
        isinstance(value, type)
        and issubclass(value, tuple)
        and isinstance(getattr(value, "_fields", None), tuple)
    )


@functools.cache
def _no_value_hints() -> tuple[object, ...]:
    return tuple(
        value
        for value in (
            getattr(typing, "Never", None),
            typing.NoReturn,
        )
        if value is not None
    )


@functools.cache
def _union_origins() -> tuple[object, ...]:
    return (typing.Union, types.UnionType)


@functools.cache
def _transparent_origins() -> tuple[object, ...]:
    values = [
        typing.ClassVar,
        typing.Final,
        getattr(typing, "Required", None),
        getattr(typing, "NotRequired", None),
        getattr(typing, "ReadOnly", None),
    ]
    if _typing_extensions is not None:
        values.extend(
            [
                getattr(_typing_extensions, "Required", None),
                getattr(_typing_extensions, "NotRequired", None),
                getattr(_typing_extensions, "ReadOnly", None),
            ]
        )
    return tuple(value for value in values if value is not None)


@functools.cache
def _annotated_origins() -> tuple[object, ...]:
    values = [typing.Annotated]
    if _typing_extensions is not None:
        values.append(_typing_extensions.Annotated)
    return tuple(values)


@functools.cache
def _mapping_origins() -> frozenset[object]:
    return frozenset(
        {
            dict,
            collections.defaultdict,
            collections.OrderedDict,
            collections.Counter,
            cabc.Mapping,
            cabc.MutableMapping,
        }
    )


@functools.cache
def _sequence_origins() -> frozenset[object]:
    return frozenset(
        {
            list,
            set,
            frozenset,
            collections.deque,
            cabc.Collection,
            cabc.Container,
            cabc.Iterable,
            cabc.Iterator,
            cabc.Reversible,
            cabc.Sequence,
            cabc.MutableSequence,
            cabc.Set,
            cabc.MutableSet,
            cabc.AsyncIterable,
            cabc.AsyncIterator,
            cabc.Generator,
            cabc.AsyncGenerator,
            cabc.KeysView,
            cabc.ValuesView,
        }
    )


@functools.cache
def _items_view_origins() -> frozenset[object]:
    return frozenset({cabc.ItemsView})


@functools.cache
def _sequence_classes() -> tuple[type[Any], ...]:
    return (
        list,
        set,
        frozenset,
        range,
        collections.deque,
        cabc.Collection,
        cabc.Iterable,
        cabc.Iterator,
        cabc.Reversible,
        cabc.Sequence,
        cabc.Set,
        cabc.AsyncIterable,
        cabc.AsyncIterator,
        cabc.Generator,
        cabc.AsyncGenerator,
        cabc.KeysView,
        cabc.ValuesView,
        cabc.ItemsView,
    )


@functools.cache
def _binary_origins() -> frozenset[object]:
    return frozenset({bytes, bytearray, memoryview, cabc.ByteString})


@functools.cache
def _string_origins() -> frozenset[object]:
    return frozenset({str, re.Pattern})


@functools.cache
def _callable_origins() -> frozenset[object]:
    return frozenset({cabc.Callable})


@functools.cache
def _native_datatype(value: str) -> DataType:
    return DataType.from_str(value)


_DIRECT_CLASS_TYPES: dict[type[Any], str] = {
    _NONE_TYPE: "null",
    bool: "boolean",
    int: "int64",
    float: "float64",
    complex: "complex",
    str: "utf8",
    bytes: "binary",
    bytearray: "binary",
    memoryview: "binary",
    datetime_module.datetime: "timestamp(microsecond,\"UTC\")",
    datetime_module.date: "date32",
    datetime_module.time: "time64(microsecond)",
    datetime_module.timedelta: "duration(microsecond)",
    decimal.Decimal: "decimal128(38,18)",
    uuid.UUID: "utf8",
    pathlib.Path: "utf8",
    pathlib.PurePath: "utf8",
    os.PathLike: "utf8",
    range: "range",
    type(Ellipsis): "null",
    type(NotImplemented): "null",
}
