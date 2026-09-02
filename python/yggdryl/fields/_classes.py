"""Dataclass classes projected onto cached native :class:`yggdryl.Field` values."""

from __future__ import annotations

import collections
import collections.abc as cabc
import ast
import dataclasses as dc
import datetime as dt
import enum
import importlib
import inspect
import itertools
import operator
import os
import pathlib
import re
import sys
import textwrap
import threading
import types
import typing
import uuid
import weakref
from decimal import Decimal
from typing import Any, Callable, Literal, Mapping, TypeVar, get_args, get_origin

from .._native import DataType, Field, Field as NativeField
from .nested import StructField

_T = TypeVar("_T")

_typing_extensions: types.ModuleType | None
try:
    import typing_extensions as _typing_extensions_module
except ImportError:  # pragma: no cover - dependency is installed on Python 3.10
    _typing_extensions = None
else:
    _typing_extensions = _typing_extensions_module

_annotationlib: types.ModuleType | None
try:
    _annotationlib_module = importlib.import_module("annotationlib")
except ImportError:  # pragma: no cover - Python before 3.14
    _annotationlib = None
else:
    _annotationlib = _annotationlib_module

_ErrorPolicy = Literal["raise", "default"]
_NONE_TYPE = type(None)
_MISSING_KEY = object()
_MISSING_FIELD_DESCRIPTOR = object()
_UNION_ORIGINS = (typing.Union, types.UnionType)
_SELF_HINTS = tuple(
    value
    for value in (
        getattr(typing, "Self", None),
        getattr(_typing_extensions, "Self", None) if _typing_extensions else None,
    )
    if value is not None
)
_TRANSPARENT_ORIGINS = tuple(
    value
    for value in (
        typing.Annotated,
        typing.ClassVar,
        typing.Final,
        getattr(typing, "Required", None),
        getattr(typing, "NotRequired", None),
        getattr(typing, "ReadOnly", None),
        getattr(_typing_extensions, "Annotated", None) if _typing_extensions else None,
        getattr(_typing_extensions, "Required", None) if _typing_extensions else None,
        getattr(_typing_extensions, "NotRequired", None) if _typing_extensions else None,
        getattr(_typing_extensions, "ReadOnly", None) if _typing_extensions else None,
    )
    if value is not None
)
_TRUE_STRINGS = frozenset(("true", "1", "yes", "on"))
_FALSE_STRINGS = frozenset(("false", "0", "no", "off"))
_INTEGER = re.compile(r"[+-]?[0-9]+\Z")
_SCHEMA_LOCK = threading.RLock()
_SCOPE_TOKEN_NAME = "__yggdryl_field_invocation_token__"


class _ScopeToken:
    __slots__ = ()


class _UnresolvedAnnotation(TypeError):
    """A forward name that may become resolvable later in the same scope."""


class _UnknownFieldError(TypeError):
    """An unknown input key, which error fallback must never suppress."""

    def __init__(self, message: str, *, matched: bool) -> None:
        super().__init__(message)
        self.matched = matched


class _PhysicalUnionValue:
    """Private Arrow-default value retaining one selected physical branch."""

    __slots__ = ("field_index", "value")

    def __init__(self, field_index: int, value: object) -> None:
        self.field_index = field_index
        self.value = value


class _Schema(typing.NamedTuple):
    owner_id: int
    root: NativeField
    fields: tuple[NativeField, ...]
    value_fields: tuple[dc.Field[Any], ...]
    field_lookup: Mapping[str, NativeField]
    hints: Mapping[str, Any]
    nested_hints: dict[type[Any], Mapping[str, Any]]
    constructor_fields: tuple[dc.Field[Any], ...]
    constructor_names: frozenset[str]


_PENDING_SCHEMAS: weakref.WeakKeyDictionary[type[Any], object] = (
    weakref.WeakKeyDictionary()
)
_BUILDING_SCHEMAS: set[int] = set()
_MODULE_SCOPE_TOKENS: weakref.WeakKeyDictionary[
    types.ModuleType, _ScopeToken
] = weakref.WeakKeyDictionary()


def _unevaluated_annotations(value: type[Any]) -> dict[str, Any]:
    """Read annotations without executing deferred forward references."""

    annotations = value.__dict__.get("__annotations__")
    if isinstance(annotations, dict):
        return dict(annotations)
    if _annotationlib is None:
        return {}
    try:
        return dict(
            _annotationlib.get_annotations(
                value,
                format=_annotationlib.Format.STRING,
            )
        )
    except (TypeError, ValueError):
        return {}


def _capture_context() -> tuple[dict[str, Any], _ScopeToken]:
    frame = inspect.currentframe()
    try:
        caller = frame.f_back.f_back if frame is not None and frame.f_back is not None else None
        if caller is None:
            return {}, _ScopeToken()
        class_frames: list[types.FrameType] = []
        scope_frame = caller
        while (
            "__module__" in scope_frame.f_locals
            and "__qualname__" in scope_frame.f_locals
            and scope_frame.f_back is not None
        ):
            class_frames.append(scope_frame)
            scope_frame = scope_frame.f_back
        scope_locals = scope_frame.f_locals
        module = (
            sys.modules.get(scope_frame.f_globals.get("__name__", ""))
            if scope_frame.f_code.co_name == "<module>"
            else None
        )
        if module is not None:
            with _SCHEMA_LOCK:
                token = _MODULE_SCOPE_TOKENS.get(module)
                if token is None:
                    token = _ScopeToken()
                    _MODULE_SCOPE_TOKENS[module] = token
        else:
            token = scope_locals.get(_SCOPE_TOKEN_NAME)
            if not isinstance(token, _ScopeToken):
                token = _ScopeToken()
                scope_locals[_SCOPE_TOKEN_NAME] = token
        namespace = dict(scope_locals)
        for class_frame in reversed(class_frames):
            namespace.update(class_frame.f_locals)
        namespace.pop(_SCOPE_TOKEN_NAME, None)
        return namespace, token
    finally:
        del frame


def _annotation_names(annotation: Any) -> set[str]:
    if isinstance(annotation, typing.ForwardRef):
        annotation = annotation.__forward_arg__
    if not isinstance(annotation, str):
        return set()
    seen: set[str] = set()
    while True:
        if annotation in seen:
            return set()
        seen.add(annotation)
        try:
            expression = ast.parse(annotation, mode="eval")
        except SyntaxError:
            return set()
        if not (
            isinstance(expression.body, ast.Constant)
            and isinstance(expression.body.value, str)
        ):
            break
        annotation = expression.body.value
    names = {
        node.id
        for node in ast.walk(expression)
        if isinstance(node, ast.Name)
    }
    names.update(_quoted_annotation_names(expression.body, allow_strings=True))
    return names


def _quoted_annotation_names(
    node: ast.AST,
    *,
    allow_strings: bool,
) -> set[str]:
    if isinstance(node, ast.Constant):
        if allow_strings and isinstance(node.value, str):
            return _annotation_names(node.value)
        return set()
    if isinstance(node, ast.Subscript):
        if isinstance(node.value, ast.Attribute):
            terminal = node.value.attr
        elif isinstance(node.value, ast.Name):
            terminal = node.value.id
        else:
            terminal = ""
        items = node.slice.elts if isinstance(node.slice, ast.Tuple) else (node.slice,)
        if terminal == "Literal":
            return set()
        if terminal == "Annotated":
            items = items[:1]
        names: set[str] = set()
        for item in items:
            names.update(_quoted_annotation_names(item, allow_strings=True))
        return names
    child_names: set[str] = set()
    for child in ast.iter_child_nodes(node):
        child_names.update(
            _quoted_annotation_names(child, allow_strings=allow_strings)
        )
    return child_names


def _annotation_dependencies(annotation: Any) -> tuple[Any, ...]:
    dependencies = list(get_args(annotation))
    origin = get_origin(annotation)
    alias = origin if type(origin).__name__ == "TypeAliasType" else annotation
    if type(alias).__name__ == "TypeAliasType":
        dependencies.append(getattr(alias, "__value__", Any))
    if isinstance(annotation, type):
        for base in reversed(annotation.__mro__):
            dependencies.extend(_unevaluated_annotations(base).values())
    return tuple(dependencies)


def _relevant_namespace(
    cls: type[Any], namespace: Mapping[str, Any]
) -> dict[str, Any]:
    """Keep only captured bindings reachable from the class annotations."""

    pending = [
        annotation
        for base in reversed(cls.__mro__)
        for annotation in _unevaluated_annotations(base).values()
    ]
    relevant: dict[str, Any] = {}
    seen_objects: set[int] = set()
    while pending:
        annotation = pending.pop()
        marker = id(annotation)
        if marker in seen_objects:
            continue
        seen_objects.add(marker)
        for name in _annotation_names(annotation):
            if name in relevant or name not in namespace:
                continue
            value = namespace[name]
            relevant[name] = value
            pending.append(value)
        pending.extend(_annotation_dependencies(annotation))
    return relevant


def _pending_namespace(cls: type[Any]) -> dict[str, Any] | None:
    cached = getattr(cls, "__yggdryl_pending_namespace__", None)
    if (
        isinstance(cached, tuple)
        and len(cached) == 2
        and cached[0] == id(cls)
        and isinstance(cached[1], dict)
    ):
        return cached[1]
    return None


def _resolved_hints(cls: type[Any], localns: Mapping[str, Any] | None = None) -> dict[str, Any]:
    module = sys.modules.get(cls.__module__)
    globalns = vars(module) if module is not None else {}
    namespace: dict[str, Any] = dict(localns or ())
    for base in reversed(cls.__mro__):
        namespace.update(vars(base))
        namespace[base.__name__] = base
    namespace[cls.__name__] = cls
    try:
        resolved = typing.get_type_hints(
            cls,
            globalns=globalns,
            localns=namespace,
            include_extras=True,
        )
    except NameError as error:
        raise _UnresolvedAnnotation(
            f"cannot resolve annotations for {cls.__module__}.{cls.__qualname__}: {error}"
        ) from error
    except Exception as error:
        raise TypeError(
            f"cannot resolve annotations for {cls.__module__}.{cls.__qualname__}: {error}"
        ) from error
    return _bind_declared_hints(cls, resolved, {})


def _bind_hint(hint: Any, bindings: Mapping[object, object]) -> Any:
    # Share the inference engine's reconstruction rules for Union, Annotated,
    # inherited aliases, and version-specific typing objects.
    from ._hints import _bind_typevars

    return _bind_typevars(hint, bindings)


def _inherited_bindings(
    cls: type[Any], initial: Mapping[object, object]
) -> dict[object, object]:
    from ._hints import _inherited_typevar_bindings

    return _inherited_typevar_bindings(cls, initial)


def _binding_contexts(
    cls: type[Any], initial: Mapping[object, object]
) -> dict[type[Any], dict[object, object]]:
    from ._hints import _typevar_bindings_by_class

    return _typevar_bindings_by_class(cls, initial)


def _annotation_owner(cls: type[Any], name: str) -> type[Any]:
    for candidate in cls.__mro__:
        if name in _unevaluated_annotations(candidate):
            return candidate
    return cls


def _bind_declared_hints(
    cls: type[Any],
    hints: Mapping[str, Any],
    initial: Mapping[object, object],
) -> dict[str, Any]:
    contexts = _binding_contexts(cls, initial)
    return {
        name: _bind_hint(
            hint,
            contexts.get(_annotation_owner(cls, name), initial),
        )
        for name, hint in hints.items()
    }


def _classes_in_hint(hint: Any) -> tuple[type[Any], ...]:
    classes: list[type[Any]] = []
    pending = [hint]
    seen: set[int] = set()
    while pending:
        current = pending.pop()
        marker = id(current)
        if marker in seen:
            continue
        seen.add(marker)
        origin = get_origin(current)
        candidate = origin or current
        if isinstance(candidate, type):
            classes.append(candidate)
        pending.extend(get_args(current))
        supertype = getattr(current, "__supertype__", None)
        if supertype is not None:
            pending.append(supertype)
        if isinstance(current, TypeVar):
            pending.extend(current.__constraints__)
            if current.__bound__ is not None:
                pending.append(current.__bound__)
        alias = origin if type(origin).__name__ == "TypeAliasType" else current
        if type(alias).__name__ == "TypeAliasType":
            pending.append(getattr(alias, "__value__", Any))
    return tuple(classes)


def _resolved_cache_slice(
    cls: type[Any],
    resolved_hints: Mapping[str, Any] | None,
    source: Mapping[type[Any], Mapping[str, Any]] | None,
) -> dict[type[Any], Mapping[str, Any]]:
    if resolved_hints is None and not source:
        return {}
    available = source or {}
    sliced: dict[type[Any], Mapping[str, Any]] = {}
    pending = [cls]
    while pending:
        current = pending.pop()
        if current in sliced:
            continue
        annotations = (
            resolved_hints
            if current is cls and resolved_hints is not None
            else available.get(current)
        )
        if annotations is None:
            continue
        sliced[current] = annotations
        for annotation in annotations.values():
            for dependency in _classes_in_hint(annotation):
                if dependency in available and dependency not in sliced:
                    pending.append(dependency)
    return sliced


def _field_metadata(field: dc.Field[Any]) -> dict[str, str] | None:
    metadata = {
        key: value
        for key, value in field.metadata.items()
        if isinstance(key, str) and isinstance(value, str)
    }
    return metadata or None


def _clone_native_field(field: NativeField, name: str | None = None) -> NativeField:
    """Clone every native Field property without projecting through Arrow."""

    cloned = NativeField(
        field.name if name is None else name,
        field.dtype,
        nullable=field.nullable,
        metadata=dict(field.metadata.items()) or None,
    )
    if field.dictionary_id is not None and field.dictionary_is_ordered is not None:
        cloned.set_dictionary_options(
            field.dictionary_id,
            field.dictionary_is_ordered,
        )
    return cloned


def _same_hint(left: object, right: object) -> bool:
    if left is right:
        return True
    try:
        return bool(left == right)
    except (TypeError, ValueError):
        return False


def _inherited_native_field(
    cls: type[Any],
    name: str,
    hint: object,
    direct_names: cabc.Set[str] | None = None,
) -> NativeField | None:
    """Reuse an exact, unmodified inherited child from its native base root."""

    if direct_names is None:
        direct_names = _unevaluated_annotations(cls).keys()
    if name in direct_names:
        return None
    for base in cls.__mro__[1:]:
        if base is object or not dc.is_dataclass(base):
            continue
        if name not in {field.name for field in dc.fields(base)}:
            continue
        schema = _ensure_schema(base)
        inherited = schema.field_lookup.get(name)
        if inherited is None:
            return None
        # A generic specialization changes the resolved annotation and must
        # be inferred under the subclass's TypeVar bindings. An unchanged
        # hint means the base's physical Field is strictly more authoritative
        # than Python's coarse int/str/container annotation.
        if not _same_hint(schema.hints.get(name), hint):
            return None
        return _clone_native_field(inherited)
    return None


_DOC_SECTION = re.compile(
    r"^[ \t]*(?:Attributes|Args|Arguments|Parameters)[ \t]*:[ \t]*$"
)
_DOC_ENTRY = re.compile(
    r"^[ \t]+(?P<name>\*{0,2}\w+)[ \t]*(?:\([^)]*\))?[ \t]*:[ \t]*(?P<text>.*)$"
)
_DOC_SPHINX = re.compile(
    r"^[ \t]*:(?:param|parameter|arg|ivar|var|attribute)[ \t]+"
    r"(?:[\w\[\], .]+[ \t]+)?(?P<name>\w+)[ \t]*:[ \t]*(?P<text>.*)$"
)


def _fold_doc(text: str) -> str:
    return " ".join(inspect.cleandoc(text).split())


def _docstring_summary(cls: type[Any]) -> str:
    doc = cls.__dict__.get("__doc__")
    if not isinstance(doc, str) or not doc.strip():
        return ""
    paragraph: list[str] = []
    for line in doc.strip().splitlines():
        if not line.strip():
            break
        paragraph.append(line.strip())
    return " ".join(paragraph)


def _parse_member_docs(doc: str) -> dict[str, str]:
    described: dict[str, str] = {}
    lines = doc.splitlines()
    index = 0
    while index < len(lines):
        line = lines[index]
        index += 1
        sphinx = _DOC_SPHINX.match(line)
        if sphinx:
            described[sphinx["name"]] = sphinx["text"].strip()
            continue
        if not _DOC_SECTION.match(line):
            continue
        current: str | None = None
        while index < len(lines):
            candidate = lines[index]
            if candidate.strip() and not candidate[:1].isspace():
                break
            entry = _DOC_ENTRY.match(candidate)
            if entry:
                current = entry["name"].lstrip("*")
                described[current] = entry["text"].strip()
            elif current and candidate.strip():
                described[current] = (
                    f"{described[current]} {candidate.strip()}".strip()
                )
            elif not candidate.strip():
                current = None
            index += 1
    return {name: _fold_doc(text) for name, text in described.items() if text}


def _attribute_docs(cls: type[Any]) -> dict[str, str]:
    if not _unevaluated_annotations(cls):
        return {}
    try:
        parsed = ast.parse(textwrap.dedent(inspect.getsource(cls))).body[0]
    except (OSError, TypeError, SyntaxError, IndentationError, IndexError):
        return {}
    if not isinstance(parsed, ast.ClassDef) or parsed.name != cls.__name__:
        return {}
    described: dict[str, str] = {}
    for member, following in itertools.pairwise(parsed.body):
        if not isinstance(member, ast.AnnAssign) or not isinstance(member.target, ast.Name):
            continue
        if (
            isinstance(following, ast.Expr)
            and isinstance(following.value, ast.Constant)
            and isinstance(following.value.value, str)
        ):
            described[member.target.id] = _fold_doc(following.value.value)
    return described


def _member_docs(cls: type[Any]) -> dict[str, str]:
    described: dict[str, str] = {}
    for base in reversed(cls.__mro__):
        doc = base.__dict__.get("__doc__")
        if isinstance(doc, str):
            described.update(_parse_member_docs(doc))
        described.update(_attribute_docs(base))
    return described


def _value_fields(cls: type[Any]) -> tuple[dc.Field[Any], ...]:
    cached = getattr(cls, "__yggdryl_scalar_fields__", None)
    if (
        isinstance(cached, tuple)
        and len(cached) == 2
        and cached[0] == id(cls)
        and isinstance(cached[1], tuple)
    ):
        return cached[1]
    with _SCHEMA_LOCK:
        cached = getattr(cls, "__yggdryl_scalar_fields__", None)
        if (
            isinstance(cached, tuple)
            and len(cached) == 2
            and cached[0] == id(cls)
            and isinstance(cached[1], tuple)
        ):
            return cached[1]
        fields = dc.fields(cls)
        setattr(cls, "__yggdryl_scalar_fields__", (id(cls), fields))
        return fields


def _build_schema(
    cls: type[Any],
    localns: Mapping[str, Any] | None,
    resolved_hints: Mapping[str, Any] | None = None,
    resolved_cache: Mapping[type[Any], Mapping[str, Any]] | None = None,
) -> _Schema:
    if not dc.is_dataclass(cls):
        raise TypeError(f"{cls!r} is not a dataclass")

    if resolved_hints is None:
        hints = _resolved_hints(cls, localns)
    else:
        hints = _bind_declared_hints(cls, resolved_hints, {})
    from ._hints import _field_from_pyhint, _reject_non_materialized_options

    value_fields = _value_fields(cls)
    materialized_names = {field.name for field in value_fields}
    for name, hint in hints.items():
        if name not in materialized_names:
            _reject_non_materialized_options(
                hint, f"{cls.__name__}.{name}"
            )
    inference_localns = dict(localns) if localns else None
    nested_hints = _resolved_cache_slice(
        cls,
        hints if resolved_hints is not None else None,
        resolved_cache,
    )
    described = _member_docs(cls)
    direct_names = _unevaluated_annotations(cls).keys()
    built_children: list[NativeField] = []
    for field in value_fields:
        hint = hints.get(field.name, field.type)
        child = _inherited_native_field(
            cls,
            field.name,
            hint,
            direct_names,
        )
        if child is None:
            child = _field_from_pyhint(
                field.name,
                hint,
                metadata=_field_metadata(field),
                localns=inference_localns,
                resolved_cache=nested_hints,
            )
        description = described.get(field.name)
        if description and "description" not in child.metadata:
            child.metadata["description"] = description
        built_children.append(child)
    children = tuple(built_children)

    # Assemble the Struct directly from native children. Arrow remains an
    # explicit projection of this Field and is not constructed as part of
    # ordinary annotation inference.
    dtype = DataType.from_fields(children)
    kind = (
        "field"
        if cls.__dict__.get("__yggdryl_field_class__", False)
        else "dataclass"
    )
    root_metadata = {
        "python.module": cls.__module__,
        "python.class": cls.__name__,
        "python.qualname": cls.__qualname__,
        "python.kind": kind,
    }
    description = _docstring_summary(cls)
    if description:
        root_metadata["description"] = description
    root = NativeField(
        cls.__name__,
        dtype,
        nullable=False,
        metadata=root_metadata,
    )
    # Cached schema projections are read-only values. Freezing prevents a
    # child singleton from diverging from the already assembled root Struct.
    for child in children:
        child._freeze()
    root._freeze()
    constructor_fields = _constructor_fields(cls, value_fields)
    return _Schema(
        id(cls),
        root,
        children,
        value_fields,
        types.MappingProxyType({field.name: field for field in children}),
        types.MappingProxyType(hints),
        nested_hints,
        constructor_fields,
        frozenset(field.name for field in constructor_fields),
    )


def _ensure_schema(
    cls: type[Any],
    localns: Mapping[str, Any] | None = None,
    resolved_hints: Mapping[str, Any] | None = None,
    resolved_cache: Mapping[type[Any], Mapping[str, Any]] | None = None,
) -> _Schema:
    direct = getattr(cls, "__yggdryl_class_schema__", None)
    if isinstance(direct, _Schema) and direct.owner_id == id(cls):
        return direct
    with _SCHEMA_LOCK:
        direct = getattr(cls, "__yggdryl_class_schema__", None)
        if isinstance(direct, _Schema) and direct.owner_id == id(cls):
            return direct
        marker = id(cls)
        if marker in _BUILDING_SCHEMAS:
            raise TypeError(
                "recursive Python annotation for "
                f"{cls.__module__}.{cls.__qualname__}"
            )
        _BUILDING_SCHEMAS.add(marker)
        try:
            namespace = localns if localns is not None else _pending_namespace(cls)
            schema = _build_schema(cls, namespace, resolved_hints, resolved_cache)
            _PENDING_SCHEMAS.pop(cls, None)
            pending = _pending_namespace(cls)
            if pending is not None:
                delattr(cls, "__yggdryl_pending_namespace__")
            setattr(cls, "__yggdryl_class_schema__", schema)
            return schema
        finally:
            _BUILDING_SCHEMAS.remove(marker)


def _decorated_field_owner(cls: type[Any]) -> type[Any] | None:
    """Return the nearest class that directly owns the field decoration."""

    for candidate in cls.__mro__:
        if candidate.__dict__.get("__yggdryl_field_class__", False):
            return candidate
    return None


def _field_accessor(owner: type[Any]) -> Callable[[], StructField]:
    """Build the cached static accessor owned by one decorated dataclass."""

    def field() -> StructField:
        return typing.cast(StructField, _ensure_schema(owner).root)

    field.__name__ = "field"
    field.__qualname__ = f"{owner.__qualname__}.field"
    setattr(field, "__yggdryl_field_accessor__", True)
    return field


def _install_field_staticmethod(cls: type[Any]) -> None:
    """Install the owner-capturing schema accessor on a decorated dataclass."""

    setattr(cls, "field", staticmethod(_field_accessor(cls)))


def _resolved_field_descriptor(cls: type[Any]) -> object:
    """Return the first ``field`` descriptor in one class's MRO."""

    for candidate in cls.__mro__:
        if "field" in candidate.__dict__:
            return typing.cast(object, candidate.__dict__["field"])
    return _MISSING_FIELD_DESCRIPTOR


def _is_installed_field_descriptor(value: object) -> bool:
    """Report whether ``value`` is a static accessor installed by ``@scalar``."""

    return isinstance(value, staticmethod) and bool(
        getattr(value.__func__, "__yggdryl_field_accessor__", False)
    )


def _adopt_materialized_schema(
    cls: type[Any],
    root: NativeField,
    fields: tuple[NativeField, ...],
    hints: Mapping[str, Any],
) -> _Schema:
    """Publish an exact foreign schema without re-inferring its physical types."""

    value_fields = _value_fields(cls)
    value_names = tuple(field.name for field in value_fields)
    native_names = tuple(field.name for field in fields)
    if value_names != native_names:
        raise TypeError(
            "materialized field children do not match dataclass fields: "
            f"{native_names!r} != {value_names!r}"
        )

    nested_hints: dict[type[Any], Mapping[str, Any]] = {}
    for hint in hints.values():
        for candidate in _classes_in_hint(hint):
            if candidate is cls or not dc.is_dataclass(candidate):
                continue
            candidate_schema = _ensure_schema(candidate)
            nested_hints[candidate] = candidate_schema.hints
            nested_hints.update(candidate_schema.nested_hints)

    for field in fields:
        field._freeze()
    root._freeze()
    constructor_fields = _constructor_fields(cls, value_fields)
    schema = _Schema(
        id(cls),
        root,
        fields,
        value_fields,
        types.MappingProxyType({field.name: field for field in fields}),
        types.MappingProxyType(dict(hints)),
        nested_hints,
        constructor_fields,
        frozenset(field.name for field in constructor_fields),
    )
    setattr(cls, "__yggdryl_class_schema__", schema)
    setattr(cls, "__yggdryl_field_class__", True)
    _install_field_staticmethod(cls)
    return schema


def _dataclass_type(value: object) -> type[Any]:
    cls = value if isinstance(value, type) else type(value)
    if not dc.is_dataclass(cls):
        raise TypeError(f"{cls!r} is not a dataclass")
    return cls


def _renamed_native_field(field: NativeField, name: str) -> NativeField:
    return _clone_native_field(field, name)


def field(value: object, name: str | None = None) -> Field:
    """Return the native field named by a Field, Arrow shape, or dataclass."""

    if name is not None and not isinstance(name, str):
        raise TypeError("name must be str or None")
    if isinstance(value, NativeField):
        if name is None or name == value.name:
            return value
        return _renamed_native_field(value, name)
    cls = value if isinstance(value, type) else type(value)
    if dc.is_dataclass(cls):
        owner = _decorated_field_owner(cls)
        root = _ensure_schema(owner or cls).root
        if name is None or name == root.name:
            return root
        renamed = _renamed_native_field(root, name)
        renamed._freeze()
        return renamed

    import pyarrow as pa  # type: ignore[import-untyped]

    if isinstance(value, pa.Schema):
        return NativeField.from_arrow_schema(
            value, "row" if name is None else name
        )
    if isinstance(value, pa.Field):
        imported = NativeField.from_arrow(value)
        if name is None or name == imported.name:
            return imported
        return _renamed_native_field(imported, name)
    if isinstance(value, pa.DataType):
        # Dictionary ordering and nested datatype flags live on an Arrow
        # Field at the native boundary. A synthetic field retains them while
        # still giving a bare datatype the caller-selected name.
        return NativeField.from_arrow(
            pa.field("value" if name is None else name, value)
        )
    raise TypeError(
        f"{value!r} does not name a field: expected Field, Arrow shape, or dataclass"
    )


def _dataclass_from_field(
    value: NativeField,
    *,
    name: str | None = None,
    module: str | None = None,
) -> type[Any]:
    from ._arrow import dataclass_from_field

    return dataclass_from_field(value, name=name, module=module)


def _dataclass_field_names(cls: type[Any]) -> tuple[str, ...]:
    fields = getattr(cls, "__dataclass_fields__", None)
    return tuple(fields) if fields else ()


def _mapping_from_key_pairs(value: object, names: tuple[str, ...]) -> object:
    """Recover a dataclass-shaped mapping that a mapping key encoded as pairs."""

    if not names or not isinstance(value, tuple) or not value:
        return value
    entries: dict[str, object] = {}
    for item in value:
        if not isinstance(item, tuple) or len(item) != 2:
            return value
        name, item_value = item
        if not isinstance(name, str) or name not in names or name in entries:
            return value
        entries[name] = item_value
    return entries


def _check_options(safe: bool, errors: str) -> None:
    if type(safe) is not bool:
        raise TypeError("safe must be bool")
    if errors not in ("raise", "default"):
        raise ValueError("errors must be 'raise' or 'default'")


def _error(path: str, expected: object, value: object, detail: str | None = None) -> TypeError:
    expected_name = getattr(expected, "__name__", repr(expected))
    message = f"{path}: expected {expected_name}, got {type(value).__name__}"
    if detail:
        message += f" ({detail})"
    return TypeError(message)


_LIST_DATA_TYPE_KINDS = frozenset(
    {"list", "large_list", "list_view", "large_list_view", "fixed_size_list"}
)


def _physical_dtype(field: NativeField) -> DataType:
    dtype = field.dtype
    while True:
        if dtype.id == "dictionary":
            dtype = dtype._dictionary_value_type()
            continue
        if dtype.id == "run_end_encoded":
            dtype = dtype[1].dtype
            continue
        return dtype


def _validate_physical_list_length(
    value: cabc.Sized,
    field: NativeField | None,
    *,
    path: str,
) -> None:
    if field is None:
        return
    dtype = _physical_dtype(field)
    expected = dtype._fixed_size_list_length()
    if expected is not None and len(value) != expected:
        raise TypeError(
            f"{path}: fixed-size-list arrow_type requires exactly {expected} "
            f"items, got {len(value)}"
        )


_TEMPORAL_NANOSECOND_DIVISORS = {
    "s": 1_000_000_000,
    "ms": 1_000_000,
    "us": 1_000,
    "ns": 1,
}


def _temporal_offset(
    value: dt.datetime | dt.time, path: str
) -> dt.timedelta | None:
    if value.tzinfo is None:
        return None
    try:
        offset = value.utcoffset()
    except (OverflowError, ValueError) as error:
        raise TypeError(f"{path}: invalid temporal timezone offset") from error
    if offset is not None and not isinstance(offset, dt.timedelta):
        raise TypeError(f"{path}: invalid temporal timezone offset")
    return offset


def _submicrosecond_nanoseconds(
    value: dt.datetime | dt.time | dt.timedelta,
    path: str,
) -> int:
    # pandas and compatible datetime/timedelta subclasses retain a 0..999 ns
    # remainder outside the standard-library microsecond component.
    attribute = "nanoseconds" if isinstance(value, dt.timedelta) else "nanosecond"
    remainder = getattr(value, attribute, 0)
    try:
        exact = operator.index(remainder)
    except TypeError as error:
        raise TypeError(
            f"{path}: {attribute} must be an exact integer"
        ) from error
    if not 0 <= exact < 1_000:
        raise TypeError(f"{path}: {attribute} must be between 0 and 999")
    return exact


def _timedelta_nanoseconds(value: dt.timedelta, path: str) -> int:
    return (
        (value.days * 86_400 + value.seconds) * 1_000_000
        + value.microseconds
    ) * 1_000 + _submicrosecond_nanoseconds(value, path)


def _validate_physical_temporal(
    value: dt.datetime | dt.time | dt.timedelta,
    field: NativeField | None,
    *,
    path: str,
) -> None:
    if field is None:
        return
    dtype = _physical_dtype(field)
    unit = dtype._time_unit()
    if unit is None:
        return

    if dtype.id == "timestamp" and isinstance(value, dt.datetime):
        timezone = dtype._timezone()
        offset = _temporal_offset(value, path)
        aware = offset is not None
        if timezone is not None and not aware and timezone != "UTC":
            raise TypeError(
                f"{path}: naive datetime is incompatible with zoned "
                f"timestamp[{unit}, {timezone}]"
            )
        if timezone is None and aware:
            raise TypeError(
                f"{path}: timezone-aware datetime is incompatible with "
                f"timezone-less timestamp[{unit}]"
            )
        nanoseconds = value.microsecond * 1_000
        if offset is not None:
            # Arrow timestamps represent the UTC instant. A sub-unit offset
            # can make a locally aligned wall clock lossy after normalization.
            nanoseconds -= _timedelta_nanoseconds(offset, path)
    elif dtype.id in ("time32", "time64") and isinstance(value, dt.time):
        if _temporal_offset(value, path) is not None:
            raise TypeError(
                f"{path}: timezone-aware time is incompatible with Arrow "
                f"{dtype.id}[{unit}]"
            )
        nanoseconds = value.microsecond * 1_000
    elif dtype.id in ("duration32", "duration64") and isinstance(value, dt.timedelta):
        nanoseconds = _timedelta_nanoseconds(value, path)
    else:
        return

    if not isinstance(value, dt.timedelta):
        nanoseconds += _submicrosecond_nanoseconds(value, path)
    divisor = _TEMPORAL_NANOSECOND_DIVISORS.get(unit)
    if divisor is not None and nanoseconds % divisor:
        raise TypeError(
            f"{path}: {dtype.id}[{unit}] would truncate the "
            f"subsecond component of {value!r}"
        )


def _physical_named_child(
    field: NativeField | None,
    name: str,
    *,
    path: str,
) -> NativeField | None:
    if field is None:
        return None
    dtype = _physical_dtype(field)
    if dtype.id != "struct":
        raise TypeError(
            f"{path}: logical structured annotation is incompatible with "
            f"physical arrow_type {dtype}"
        )
    for child in dtype:
        if child.name == name:
            return child
    raise TypeError(
        f"{path}: logical field {name!r} has no matching physical struct child"
    )


def _validate_physical_struct_names(
    field: NativeField | None,
    names: cabc.Iterable[str],
    *,
    path: str,
) -> None:
    if field is None:
        return
    dtype = _physical_dtype(field)
    if dtype.id != "struct":
        raise TypeError(
            f"{path}: logical structured annotation is incompatible with "
            f"physical arrow_type {dtype}"
        )
    logical = frozenset(names)
    physical = frozenset(child.name for child in dtype)
    if logical != physical:
        raise TypeError(
            f"{path}: logical struct fields {sorted(logical)!r} do not match "
            f"physical struct children {sorted(physical)!r}"
        )


def _validate_physical_tuple_arity(
    field: NativeField | None,
    arity: int,
    *,
    path: str,
) -> None:
    if field is None:
        return
    dtype = _physical_dtype(field)
    if dtype.id != "struct" or len(dtype) != arity:
        raise TypeError(
            f"{path}: logical tuple arity {arity} does not match physical "
            f"arrow_type {dtype}"
        )


def _physical_positional_child(
    field: NativeField | None,
    index: int,
    *,
    path: str,
) -> NativeField | None:
    if field is None:
        return None
    dtype = _physical_dtype(field)
    if dtype.id != "struct" or index >= len(dtype):
        raise TypeError(
            f"{path}: logical tuple annotation is incompatible with physical "
            f"arrow_type {dtype}"
        )
    return dtype[index]


def _physical_list_child(
    field: NativeField | None,
    *,
    path: str,
) -> NativeField | None:
    if field is None:
        return None
    dtype = _physical_dtype(field)
    if dtype.id == "map":
        # Nested Arrow map keys are represented at the Python boundary as an
        # association list whose tuple shape is the physical entries Struct.
        return dtype[0]
    if dtype.id not in _LIST_DATA_TYPE_KINDS:
        raise TypeError(
            f"{path}: logical collection annotation is incompatible with "
            f"physical arrow_type {dtype}"
        )
    return dtype[0]


def _physical_map_children(
    field: NativeField | None,
    *,
    path: str,
) -> tuple[NativeField | None, NativeField | None]:
    if field is None:
        return None, None
    dtype = _physical_dtype(field)
    if dtype.id != "map":
        raise TypeError(
            f"{path}: logical mapping annotation is incompatible with physical "
            f"arrow_type {dtype}"
        )
    entries = dtype[0].dtype
    return entries[0], entries[1]


def _physical_union_children(
    field: NativeField | None,
) -> tuple[NativeField, ...]:
    if field is None:
        return ()
    dtype = _physical_dtype(field)
    return tuple(dtype) if dtype.id == "union" else ()


def _physical_union_child_for_hint(
    field: NativeField | None,
    union_hint: Any,
    selected_hint: Any,
    conversion_owner: type[Any],
) -> NativeField | None:
    physical_children = _physical_union_children(field)
    children = iter(physical_children)
    for alternative in get_args(union_hint):
        if _is_none_branch(alternative, conversion_owner):
            continue
        child = next(children, None)
        if alternative is selected_hint or alternative == selected_hint:
            return child
    if physical_children:
        raise TypeError("logical union branch has no matching physical union child")
    return field


def _hint_value_members(hint: Any, owner: type[Any]) -> tuple[Any, ...]:
    """Return flattened non-null Python alternatives in declaration order."""

    hint = _unwrap_hint(hint, owner)
    if get_origin(hint) in _UNION_ORIGINS:
        return tuple(
            member
            for member in get_args(hint)
            if not _is_none_branch(member, owner)
        )
    return () if _is_none_branch(hint, owner) else (hint,)


def _physical_union_hint_groups(
    fields: tuple[NativeField, ...], owner: type[Any]
) -> tuple[tuple[Any, ...], tuple[tuple[int, ...], ...], tuple[Any, ...]]:
    """Mirror Python Union flattening while retaining each physical child."""

    from ._arrow import _hint_from_field

    raw_hints = tuple(
        _hint_from_field(
            field,
            module=__name__,
            owner_name="_PhysicalUnionProjection",
            path=(field.name,),
            materialize_schema=False,
        )
        for field in fields
    )
    unique: list[Any] = []
    groups: list[tuple[int, ...]] = []
    for raw_hint in raw_hints:
        indices: list[int] = []
        for member in _hint_value_members(raw_hint, owner):
            try:
                index = unique.index(member)
            except ValueError:
                index = len(unique)
                unique.append(member)
            if index not in indices:
                indices.append(index)
        groups.append(tuple(indices))
    return tuple(unique), tuple(groups), raw_hints


def _convert_physical_union_value(
    value: _PhysicalUnionValue,
    hint: Any,
    owner: type[Any],
    path: str,
    errors: str,
    physical_field: NativeField | None,
) -> tuple[Any, Any]:
    """Convert an Arrow default through its selected physical Union child."""

    physical_children = _physical_union_children(physical_field)
    if not physical_children:
        raise TypeError(f"{path}: physical Union default has no Union schema")
    if value.field_index < 0 or value.field_index >= len(physical_children):
        raise TypeError(
            f"{path}: physical Union branch index {value.field_index} is out of range"
        )
    selected_field = physical_children[value.field_index]
    if value.value is None:
        if selected_field.nullable or selected_field.dtype.id == "null":
            return None, _NONE_TYPE
        raise TypeError(f"{path}: selected physical Union child is not nullable")

    logical_members = _hint_value_members(hint, owner)
    unique, groups, raw_hints = _physical_union_hint_groups(
        physical_children, owner
    )
    if len(logical_members) != len(unique):
        raise TypeError(
            f"{path}: physical Union default has {len(unique)} Python value "
            f"branches but its projected hint has {len(logical_members)}"
        )
    selected_indices = groups[value.field_index]
    if not selected_indices:
        raise TypeError(f"{path}: selected physical Union child has no value hint")
    selected_members = tuple(logical_members[index] for index in selected_indices)

    from ._hints import _allows_none

    if _allows_none(raw_hints[value.field_index]):
        selected_members = (*selected_members, _NONE_TYPE)
    selected_hint = (
        selected_members[0]
        if len(selected_members) == 1
        else typing.Union[selected_members]
    )
    converted = _convert(
        value.value,
        selected_hint,
        owner,
        path,
        errors,
        physical_field=selected_field,
    )
    return converted, selected_hint


def _is_none_branch(hint: Any, conversion_owner: type[Any] | None) -> bool:
    if conversion_owner is not None and any(
        hint is self_hint for self_hint in _SELF_HINTS
    ):
        hint = conversion_owner
    from ._hints import _is_none_hint

    return _is_none_hint(hint)


def _accept_schema_null(
    value: object,
    field: NativeField | None,
    path: str,
) -> bool:
    if value is not None or field is None:
        return False
    if field.nullable:
        return True
    raise TypeError(f"{path}: field is not nullable")


def _unwrap_hint(hint: Any, owner: type[Any]) -> Any:
    while True:
        if any(hint is self_hint for self_hint in _SELF_HINTS):
            hint = owner
            continue
        if isinstance(hint, dc.InitVar):
            hint = hint.type
            continue
        origin = get_origin(hint)
        if origin in _TRANSPARENT_ORIGINS:
            arguments = get_args(hint)
            hint = arguments[0] if arguments else Any
            continue
        supertype = getattr(hint, "__supertype__", None)
        if supertype is not None:
            hint = supertype
            continue
        alias = origin if type(origin).__name__ == "TypeAliasType" else hint
        if type(alias).__name__ == "TypeAliasType":
            alias_value = getattr(alias, "__value__", Any)
            parameters = getattr(alias, "__type_params__", ())
            arguments = get_args(hint) if alias is origin else ()
            hint = (
                _bind_hint(alias_value, dict(zip(parameters, arguments)))
                if parameters and arguments
                else alias_value
            )
            continue
        if isinstance(hint, typing.TypeVar):
            if hint.__bound__ is not None:
                hint = hint.__bound__
            elif hint.__constraints__:
                hint = typing.Union[hint.__constraints__]
            else:
                hint = Any
            continue
        return hint


def _convert_bool(value: object, path: str) -> bool:
    if type(value) is bool:
        return value
    if type(value) is int and value in (0, 1):
        return bool(value)
    if isinstance(value, str):
        normalized = value.strip().lower()
        if normalized in _TRUE_STRINGS:
            return True
        if normalized in _FALSE_STRINGS:
            return False
    raise _error(path, bool, value, "accepted values are true/false, 1/0, yes/no, or on/off")


def _convert_int(value: object, path: str) -> int:
    if type(value) is int:
        return value
    if isinstance(value, str) and _INTEGER.fullmatch(value.strip()):
        return int(value, 10)
    if isinstance(value, float) and value.is_integer():
        return int(value)
    if isinstance(value, Decimal) and value.is_finite():
        if value == value.to_integral_value():
            return int(value)
    raise _error(path, int, value, "lossless integer conversion required")


def _convert_float(value: object, path: str) -> float:
    # A half-float is the one Arrow value PyArrow hands back as a NumPy scalar
    # rather than a Python one before PyArrow 21, and `numpy.float16` is not a
    # `float` subclass the way `numpy.float64` is. `__float__` is what says a
    # value is a real number, so it is what this accepts: a bool has one and is
    # excluded here because its truthiness is not a number, and a string has
    # none, so the branch below still owns text.
    if not isinstance(value, (bool, str)) and hasattr(value, "__float__"):
        try:
            return float(value)
        except (OverflowError, ValueError) as error:
            raise _error(path, float, value, "representable float required") from error
    if isinstance(value, str):
        try:
            return float(value.strip())
        except ValueError:
            pass
    raise _error(path, float, value)


def _convert_literal(
    value: object,
    hint: Any,
    owner: type[Any],
    path: str,
    errors: str,
    physical_field: NativeField | None,
) -> Any:
    for literal in get_args(hint):
        if literal is None and value is None:
            return None
        try:
            candidate = _convert(
                value,
                type(literal),
                owner,
                path,
                errors,
                physical_field=physical_field,
            )
        except (TypeError, ValueError):
            continue
        if type(candidate) is type(literal) and candidate == literal:
            return literal
    raise _error(path, hint, value, "value is not one of the declared literals")


def _convert_union_branch(
    value: object,
    hint: Any,
    owner: type[Any],
    path: str,
    errors: str,
    physical_field: NativeField | None = None,
) -> tuple[Any, Any]:
    if isinstance(value, _PhysicalUnionValue):
        return _convert_physical_union_value(
            value, hint, owner, path, errors, physical_field
        )
    alternatives = get_args(hint)
    if _accept_schema_null(value, physical_field, path):
        return None, _NONE_TYPE
    if value is None and physical_field is None and any(
        _is_none_branch(alternative, owner) for alternative in alternatives
    ):
        return None, _NONE_TYPE

    # Prefer a branch whose runtime container/scalar already matches. This
    # keeps ``int | str`` applied to ``"42"`` as a string while still
    # recursively checking parameterized members such as ``list[int]``.
    def match_rank(alternative: Any) -> int:
        alternative = _unwrap_hint(alternative, owner)
        candidate = get_origin(alternative) or alternative
        if isinstance(candidate, type):
            if type(value) is candidate:
                return 0
            try:
                if isinstance(value, candidate):
                    return 1
            except TypeError:
                pass
        return 2

    physical_children = _physical_union_children(physical_field)
    physical_value_children = tuple(
        child for child in physical_children if child.dtype.id != "null"
    )
    non_none_count = sum(
        not _is_none_branch(alternative, owner) for alternative in alternatives
    )
    if (
        physical_field is not None
        and non_none_count > 1
        and _physical_dtype(physical_field).id != "union"
    ):
        raise TypeError(
            f"{path}: logical union has {non_none_count} non-None branches but "
            f"physical arrow_type {_physical_dtype(physical_field)} is not a union"
        )
    if physical_children and len(physical_value_children) != non_none_count:
        raise TypeError(
            f"{path}: logical union has {non_none_count} non-None branches but "
            f"physical arrow_type has {len(physical_value_children)} children"
        )
    children = iter(physical_value_children)
    alternatives_with_fields = tuple(
        (
            alternative,
            None
            if _is_none_branch(alternative, owner)
            else next(children, None),
        )
        for alternative in alternatives
    )
    alternatives_with_fields = tuple(
        sorted(alternatives_with_fields, key=lambda item: match_rank(item[0]))
    )
    failures: list[str] = []
    matched_unknown: _UnknownFieldError | None = None
    unmatched_unknown: _UnknownFieldError | None = None
    for alternative, branch_field in alternatives_with_fields:
        if _is_none_branch(alternative, owner):
            continue
        try:
            return (
                _convert(
                    value,
                    alternative,
                    owner,
                    path,
                    errors,
                    physical_field=(
                        branch_field
                        if physical_children
                        else physical_field
                    ),
                ),
                alternative,
            )
        except _UnknownFieldError as error:
            # Another structurally compatible union branch may still accept
            # the mapping, but a final unknown-key failure remains non-
            # recoverable by an outer dataclass default.
            if error.matched:
                if matched_unknown is None:
                    matched_unknown = error
            elif unmatched_unknown is None:
                unmatched_unknown = error
        except (TypeError, ValueError) as error:
            failures.append(str(error))
    if matched_unknown is not None:
        raise matched_unknown
    if failures:
        raise _error(path, hint, value, "; ".join(failures[:3]))
    if unmatched_unknown is not None:
        raise unmatched_unknown
    raise _error(path, hint, value, "; ".join(failures[:3]))


def _convert_union(
    value: object,
    hint: Any,
    owner: type[Any],
    path: str,
    errors: str,
    physical_field: NativeField | None,
) -> Any:
    return _convert_union_branch(
        value,
        hint,
        owner,
        path,
        errors,
        physical_field=physical_field,
    )[0]


def _nested_annotations(
    hint: type[Any], owner: type[Any], path: str
) -> Mapping[str, Any]:
    schema = _ensure_schema(owner)
    cached = schema.nested_hints.get(hint)
    if cached is not None:
        return cached
    with _SCHEMA_LOCK:
        cached = schema.nested_hints.get(hint)
        if cached is not None:
            return cached
        module = sys.modules.get(hint.__module__)
        globalns = vars(module) if module is not None else {}
        namespace: dict[str, Any] = {}
        for base in reversed(hint.__mro__):
            namespace.update(vars(base))
            namespace[base.__name__] = base
        namespace[hint.__name__] = hint
        try:
            resolved = typing.get_type_hints(
                hint,
                globalns=globalns,
                localns=namespace,
                include_extras=True,
            )
        except (NameError, TypeError) as error:
            raise TypeError(
                f"cannot resolve nested annotations for "
                f"{hint.__module__}.{hint.__qualname__} at {path}: {error}"
            ) from error
        cached = types.MappingProxyType(resolved)
        schema.nested_hints[hint] = cached
        return cached


def _is_typed_dict_class(value: object) -> bool:
    checker = getattr(typing, "is_typeddict", None)
    if checker is not None and checker(value):
        return True
    return (
        isinstance(value, type)
        and hasattr(value, "__required_keys__")
        and hasattr(value, "__optional_keys__")
        and bool(_unevaluated_annotations(value))
    )


def _convert_typed_dict(
    value: object,
    hint: type[Any],
    owner: type[Any],
    path: str,
    errors: str,
    bindings: Mapping[object, object] | None = None,
    physical_field: NativeField | None = None,
) -> dict[str, Any]:
    if not isinstance(value, cabc.Mapping):
        raise _error(path, hint, value)
    annotations = _nested_annotations(hint, owner, path)
    if bindings:
        annotations = {
            name: _bind_hint(annotation, bindings)
            for name, annotation in annotations.items()
        }
    _validate_physical_struct_names(
        physical_field, annotations, path=path
    )
    unknown = set(value).difference(annotations)
    if unknown:
        raise _UnknownFieldError(
            f"{path}: unknown keys {sorted(unknown, key=repr)!r}",
            matched=any(key in annotations for key in value),
        )
    required = getattr(hint, "__required_keys__", frozenset(annotations))
    output: dict[str, Any] = {}
    for name, annotation in annotations.items():
        if name not in value:
            if name in required:
                raise TypeError(f"{path}.{name}: missing required value")
            continue
        child_path = f"{path}.{name}"
        output[name] = _convert(
            value[name],
            annotation,
            owner,
            child_path,
            errors,
            physical_field=_physical_named_child(
                physical_field, name, path=child_path
            ),
        )
    return output


def _convert_named_tuple(
    value: object,
    hint: type[Any],
    owner: type[Any],
    path: str,
    errors: str,
    bindings: Mapping[object, object] | None = None,
    physical_field: NativeField | None = None,
) -> Any:
    annotations = _nested_annotations(hint, owner, path)
    if bindings:
        annotations = {
            name: _bind_hint(annotation, bindings)
            for name, annotation in annotations.items()
        }
    names = tuple(annotations)
    _validate_physical_struct_names(physical_field, names, path=path)
    value = _mapping_from_key_pairs(value, names)
    if isinstance(value, cabc.Mapping):
        unknown = set(value).difference(names)
        if unknown:
            raise _UnknownFieldError(
                f"{path}: unknown keys {sorted(unknown, key=repr)!r}",
                matched=any(key in annotations for key in value),
            )
        missing = [name for name in names if name not in value]
        if missing:
            raise TypeError(f"{path}: missing required values {missing!r}")
        converted = [
            _convert(
                value[name],
                annotations[name],
                owner,
                f"{path}.{name}",
                errors,
                physical_field=_physical_named_child(
                    physical_field, name, path=f"{path}.{name}"
                ),
            )
            for name in names
        ]
    elif isinstance(value, (tuple, list)) and len(value) == len(names):
        converted = [
            _convert(
                item,
                annotations[name],
                owner,
                f"{path}[{index}]",
                errors,
                physical_field=_physical_positional_child(
                    physical_field, index, path=f"{path}[{index}]"
                ),
            )
            for index, (name, item) in enumerate(zip(names, value))
        ]
    else:
        raise _error(path, hint, value)
    return hint(*converted)


def _convert_collection(
    value: object,
    hint: Any,
    origin: Any,
    owner: type[Any],
    path: str,
    errors: str,
    physical_field: NativeField | None,
) -> Any:
    if (
        isinstance(value, (str, bytes, bytearray, memoryview, cabc.Mapping))
        or not isinstance(value, cabc.Iterable)
    ):
        raise _error(path, hint, value)
    arguments = get_args(hint)
    item_hint = arguments[0] if arguments else Any
    item_field = _physical_list_child(physical_field, path=path)
    converted = (
        _convert(
            item,
            item_hint,
            owner,
            f"{path}[{index}]",
            errors,
            physical_field=item_field,
        )
        for index, item in enumerate(value)
    )
    if origin in (set, cabc.Set, cabc.MutableSet):
        result: cabc.Sized = set(converted)
    elif origin is frozenset:
        result = frozenset(converted)
    elif origin is collections.deque:
        # A decoded deque arrives as a plain sequence, because a bound is not
        # part of the annotation and no encoding carries it.
        maxlen = value.maxlen if isinstance(value, collections.deque) else None
        result = collections.deque(converted, maxlen=maxlen)
    else:
        result = list(converted)
    _validate_physical_list_length(result, physical_field, path=path)
    return result


def _convert_mapping(
    value: object,
    hint: Any,
    origin: Any,
    owner: type[Any],
    path: str,
    errors: str,
    physical_field: NativeField | None,
) -> Any:
    if not isinstance(value, cabc.Mapping):
        raise _error(path, hint, value)
    arguments = get_args(hint)
    if origin is collections.Counter:
        key_hint = arguments[0] if arguments else Any
        value_hint = int
    else:
        key_hint, value_hint = arguments[:2] if len(arguments) >= 2 else (Any, Any)
    key_field, value_field = _physical_map_children(physical_field, path=path)
    def converted_pairs() -> typing.Iterator[tuple[Any, Any]]:
        seen: dict[Any, tuple[int, object]] = {}
        for index, (key, item) in enumerate(value.items()):
            converted_key = _convert(
                key,
                key_hint,
                owner,
                f"{path}.keys[{index}]",
                errors,
                physical_field=key_field,
            )
            converted_value = _convert(
                item,
                value_hint,
                owner,
                f"{path}[{key!r}]",
                errors,
                physical_field=value_field,
            )
            try:
                previous = seen.get(converted_key, _MISSING_KEY)
            except TypeError as error:
                raise TypeError(
                    f"{path}.keys[{index}]: converted mapping key "
                    f"{converted_key!r} is not hashable"
                ) from error
            if previous is not _MISSING_KEY:
                previous_index, previous_key = typing.cast(
                    tuple[int, object], previous
                )
                raise TypeError(
                    f"{path}.keys[{index}]: key {key!r} collides after safe "
                    f"conversion with key {previous_key!r} at index "
                    f"{previous_index}"
                )
            seen[converted_key] = (index, key)
            yield converted_key, converted_value

    if origin is collections.OrderedDict:
        return collections.OrderedDict(converted_pairs())
    if origin is collections.defaultdict:
        factory = value.default_factory if isinstance(value, collections.defaultdict) else None
        return collections.defaultdict(factory, converted_pairs())
    if origin is collections.Counter:
        return collections.Counter(dict(converted_pairs()))
    converted = dict(converted_pairs())
    if origin is collections.ChainMap:
        return collections.ChainMap(converted)
    if origin not in (dict, cabc.Mapping, cabc.MutableMapping):
        try:
            return origin(converted)
        except (TypeError, ValueError):
            pass
    return converted


def _convert(
    value: object,
    hint: Any,
    owner: type[Any],
    path: str,
    errors: str,
    *,
    physical_field: NativeField | None = None,
) -> Any:
    if isinstance(value, _PhysicalUnionValue):
        return _convert_physical_union_value(
            value, hint, owner, path, errors, physical_field
        )[0]
    if _accept_schema_null(value, physical_field, path):
        return None
    hint = _unwrap_hint(hint, owner)
    origin = get_origin(hint)

    if hint in (Any, object, typing.Any):
        return value
    if hint is _NONE_TYPE:
        if value is None:
            return None
        raise _error(path, hint, value)
    if origin in _UNION_ORIGINS:
        return _convert_union(
            value, hint, owner, path, errors, physical_field
        )
    if origin is typing.Literal:
        return _convert_literal(
            value, hint, owner, path, errors, physical_field
        )
    if isinstance(hint, str) or isinstance(hint, typing.ForwardRef):
        raise TypeError(f"{path}: unresolved annotation {hint!r}")

    if hint is bool:
        return _convert_bool(value, path)
    if hint is int:
        return _convert_int(value, path)
    if hint is float:
        return _convert_float(value, path)
    if hint is str:
        if isinstance(value, str):
            return value
        if isinstance(value, (bytes, bytearray, memoryview)):
            try:
                return bytes(value).decode("utf-8")
            except UnicodeDecodeError as error:
                raise _error(path, str, value, "UTF-8 input required") from error
        if isinstance(value, (pathlib.PurePath, uuid.UUID, enum.Enum)):
            return str(value.value if isinstance(value, enum.Enum) else value)
        raise _error(path, str, value)
    if hint in (bytes, bytearray, memoryview):
        if isinstance(value, str):
            raw = value.encode("utf-8")
        elif isinstance(value, (bytes, bytearray, memoryview)):
            raw = bytes(value)
        else:
            raise _error(path, hint, value)
        return raw if hint is bytes else hint(raw)

    if hint is Decimal:
        if isinstance(value, bool):
            raise _error(path, Decimal, value)
        try:
            return value if isinstance(value, Decimal) else Decimal(str(value))
        except (ValueError, ArithmeticError) as error:
            raise _error(path, Decimal, value) from error
    if hint is uuid.UUID:
        if isinstance(value, uuid.UUID):
            return value
        try:
            return uuid.UUID(str(value))
        except (ValueError, AttributeError) as error:
            raise _error(path, uuid.UUID, value) from error
    if hint is dt.datetime:
        if isinstance(value, dt.datetime):
            converted_datetime = value
        elif isinstance(value, str):
            try:
                converted_datetime = dt.datetime.fromisoformat(value)
            except ValueError as error:
                raise _error(path, dt.datetime, value, "ISO-8601 value required") from error
        else:
            raise _error(path, dt.datetime, value)
        _validate_physical_temporal(
            converted_datetime, physical_field, path=path
        )
        return converted_datetime
    if hint is dt.date:
        if isinstance(value, dt.date) and not isinstance(value, dt.datetime):
            return value
        if isinstance(value, str):
            try:
                return dt.date.fromisoformat(value)
            except ValueError as error:
                raise _error(path, dt.date, value, "ISO-8601 value required") from error
        raise _error(path, dt.date, value)
    if hint is dt.time:
        if isinstance(value, dt.time):
            converted_time = value
        elif isinstance(value, str):
            try:
                converted_time = dt.time.fromisoformat(value)
            except ValueError as error:
                raise _error(path, dt.time, value, "ISO-8601 value required") from error
        else:
            raise _error(path, dt.time, value)
        _validate_physical_temporal(converted_time, physical_field, path=path)
        return converted_time
    if hint is dt.timedelta:
        if isinstance(value, dt.timedelta):
            converted_delta = value
        elif isinstance(value, (int, float)) and not isinstance(value, bool):
            try:
                converted_delta = dt.timedelta(seconds=value)
            except (OverflowError, ValueError) as error:
                raise _error(path, dt.timedelta, value, "finite in-range seconds required") from error
        else:
            raise _error(path, dt.timedelta, value)
        _validate_physical_temporal(converted_delta, physical_field, path=path)
        return converted_delta

    if isinstance(hint, type) and issubclass(hint, enum.Enum):
        if isinstance(value, hint):
            return value
        try:
            return hint(value)
        except (ValueError, TypeError):
            if isinstance(value, str):
                try:
                    return hint[value]
                except KeyError:
                    pass
        raise _error(path, hint, value)

    if isinstance(hint, type) and dc.is_dataclass(hint):
        value = _mapping_from_key_pairs(value, _dataclass_field_names(hint))
        if isinstance(value, hint):
            owner_schema = _ensure_schema(owner)
            _, _, _, projected = _project_dataclass_values(
                value,
                safe=True,
                resolved_cache=owner_schema.nested_hints,
                conversion_owner=owner,
                physical_root=physical_field,
            )
            for _ in projected:
                pass
            return value
        if isinstance(value, cabc.Mapping):
            owner_schema = _ensure_schema(owner)
            return _from_dict(
                hint,
                value,
                safe=True,
                errors=errors,
                path=path,
                resolved_hints=owner_schema.nested_hints.get(hint),
                resolved_cache=owner_schema.nested_hints,
                conversion_owner=owner,
                physical_root=physical_field,
            )
        raise _error(path, hint, value)
    if isinstance(origin, type) and dc.is_dataclass(origin):
        value = _mapping_from_key_pairs(value, _dataclass_field_names(origin))
        direct = dict(zip(getattr(origin, "__parameters__", ()), get_args(hint)))
        if isinstance(value, origin):
            # An instance already owns its dataclass state. Reconstructing it
            # here would rerun __init__/__post_init__ during safe into_dict and
            # could also recompute init=False fields. Export validates its
            # fields in place with the bindings carried by ``hint``.
            owner_schema = _ensure_schema(owner)
            _, _, _, projected = _project_dataclass_values(
                value,
                safe=True,
                resolved_cache=owner_schema.nested_hints,
                conversion_owner=owner,
                bindings=direct,
                physical_root=physical_field,
            )
            for _ in projected:
                pass
            return value
        if isinstance(value, cabc.Mapping):
            owner_schema = _ensure_schema(owner)
            return _from_dict(
                origin,
                value,
                safe=True,
                errors=errors,
                path=path,
                bindings=direct,
                resolved_hints=owner_schema.nested_hints.get(origin),
                resolved_cache=owner_schema.nested_hints,
                conversion_owner=owner,
                physical_root=physical_field,
            )
        raise _error(path, hint, value)
    if _is_typed_dict_class(hint):
        return _convert_typed_dict(
            value,
            hint,
            owner,
            path,
            errors,
            _inherited_bindings(hint, {}),
            physical_field,
        )
    if _is_typed_dict_class(origin):
        direct = dict(zip(getattr(origin, "__parameters__", ()), get_args(hint)))
        return _convert_typed_dict(
            value,
            origin,
            owner,
            path,
            errors,
            _inherited_bindings(origin, direct),
            physical_field,
        )
    if isinstance(hint, type) and issubclass(hint, tuple) and hasattr(hint, "_fields"):
        return _convert_named_tuple(
            value,
            hint,
            owner,
            path,
            errors,
            _inherited_bindings(hint, {}),
            physical_field,
        )
    if isinstance(origin, type) and issubclass(origin, tuple) and hasattr(origin, "_fields"):
        direct = dict(zip(getattr(origin, "__parameters__", ()), get_args(hint)))
        return _convert_named_tuple(
            value,
            origin,
            owner,
            path,
            errors,
            _inherited_bindings(origin, direct),
            physical_field,
        )

    if origin is tuple or hint is tuple:
        if (
            isinstance(value, (str, bytes, bytearray, memoryview, cabc.Mapping))
            or not isinstance(value, cabc.Iterable)
        ):
            raise _error(path, hint, value)
        items = list(value)
        arguments = get_args(hint)
        if not arguments:
            item_field = _physical_list_child(physical_field, path=path)
            converted_tuple = tuple(
                _convert(
                    item,
                    Any,
                    owner,
                    f"{path}[{index}]",
                    errors,
                    physical_field=item_field,
                )
                for index, item in enumerate(items)
            )
            _validate_physical_list_length(
                converted_tuple, physical_field, path=path
            )
            return converted_tuple
        if len(arguments) == 2 and arguments[1] is Ellipsis:
            converted_tuple = tuple(
                _convert(
                    item,
                    arguments[0],
                    owner,
                    f"{path}[{index}]",
                    errors,
                    physical_field=_physical_list_child(physical_field, path=path),
                )
                for index, item in enumerate(items)
            )
            _validate_physical_list_length(
                converted_tuple, physical_field, path=path
            )
            return converted_tuple
        if len(items) != len(arguments):
            raise _error(path, hint, value, f"expected {len(arguments)} items")
        _validate_physical_tuple_arity(
            physical_field, len(arguments), path=path
        )
        return tuple(
            _convert(
                item,
                item_hint,
                owner,
                f"{path}[{index}]",
                errors,
                physical_field=_physical_positional_child(
                    physical_field, index, path=f"{path}[{index}]"
                ),
            )
            for index, (item, item_hint) in enumerate(zip(items, arguments))
        )

    sequence_origins = (
        list,
        set,
        frozenset,
        collections.deque,
        cabc.Sequence,
        cabc.MutableSequence,
        cabc.Iterable,
        cabc.Iterator,
        cabc.Collection,
        cabc.Reversible,
        cabc.Set,
        cabc.MutableSet,
        cabc.KeysView,
        cabc.ValuesView,
        cabc.Generator,
    )
    if origin in sequence_origins or hint in sequence_origins:
        selected_origin = origin or hint
        return _convert_collection(
            value,
            hint,
            selected_origin,
            owner,
            path,
            errors,
            physical_field,
        )

    mapping_origins = (
        dict,
        collections.OrderedDict,
        collections.defaultdict,
        collections.Counter,
        collections.ChainMap,
        cabc.Mapping,
        cabc.MutableMapping,
    )
    if hint in mapping_origins:
        return _convert_mapping(
            value, hint, hint, owner, path, errors, physical_field
        )
    if origin in mapping_origins or (
        isinstance(origin, type) and issubclass(origin, cabc.Mapping)
    ):
        return _convert_mapping(
            value, hint, origin, owner, path, errors, physical_field
        )

    if origin is cabc.ItemsView:
        arguments = get_args(hint)
        key_hint, item_hint = arguments[:2] if len(arguments) >= 2 else (Any, Any)
        if isinstance(value, cabc.Mapping):
            value = value.items()
        if not isinstance(value, cabc.Iterable):
            raise _error(path, hint, value)
        return [
            (
                _convert(
                    key,
                    key_hint,
                    owner,
                    f"{path}[{index}][0]",
                    errors,
                    physical_field=_physical_positional_child(
                        _physical_list_child(physical_field, path=path),
                        0,
                        path=f"{path}[{index}][0]",
                    ),
                ),
                _convert(
                    item,
                    item_hint,
                    owner,
                    f"{path}[{index}][1]",
                    errors,
                    physical_field=_physical_positional_child(
                        _physical_list_child(physical_field, path=path),
                        1,
                        path=f"{path}[{index}][1]",
                    ),
                ),
            )
            for index, (key, item) in enumerate(value)
        ]

    if origin in (type,):
        arguments = get_args(hint)
        expected_class: Any = arguments[0] if arguments else object
        if isinstance(value, type) and (
            expected_class is Any or issubclass(value, expected_class)
        ):
            return value
        raise _error(path, hint, value)
    if origin is cabc.Callable:
        if callable(value):
            return value
        raise _error(path, hint, value)

    if isinstance(hint, type) and issubclass(hint, pathlib.PurePath):
        if isinstance(value, hint):
            return value
        if isinstance(value, bytes):
            value = os.fsdecode(value)
        if isinstance(value, (str, pathlib.PurePath)):
            try:
                return hint(value)
            except (TypeError, NotImplementedError) as error:
                raise _error(path, hint, value) from error
        raise _error(path, hint, value)

    if origin is os.PathLike or hint is os.PathLike:
        if isinstance(value, os.PathLike):
            return value
        if isinstance(value, bytes):
            return pathlib.Path(os.fsdecode(value))
        if isinstance(value, str):
            return pathlib.Path(value)
        raise _error(path, hint, value)

    if isinstance(hint, type):
        if isinstance(value, hint):
            return value
        try:
            return hint(value)
        except (TypeError, ValueError, OverflowError) as error:
            raise _error(path, hint, value) from error
    raise _error(path, hint, value, "unsupported annotation")


def _has_default(field: dc.Field[Any]) -> bool:
    return field.default is not dc.MISSING or field.default_factory is not dc.MISSING


def _constructor_fields(
    cls: type[Any], value_fields: tuple[dc.Field[Any], ...]
) -> tuple[dc.Field[Any], ...]:
    regular = list(value_fields)
    names = {field.name for field in regular}
    initvar_marker = getattr(dc, "_FIELD_INITVAR", None)
    for field in cls.__dataclass_fields__.values():
        if field.name not in names and getattr(field, "_field_type", None) is initvar_marker:
            regular.append(field)
    return tuple(field for field in regular if field.init)


def _from_dict(
    cls: type[_T],
    values: Mapping[str, Any],
    *,
    safe: bool,
    errors: str,
    path: str,
    bindings: Mapping[object, object] | None = None,
    resolved_hints: Mapping[str, Any] | None = None,
    resolved_cache: Mapping[type[Any], Mapping[str, Any]] | None = None,
    conversion_owner: type[Any] | None = None,
    physical_root: NativeField | None = None,
) -> _T:
    _check_options(safe, errors)
    if not isinstance(values, cabc.Mapping):
        raise TypeError(f"{path}: expected a mapping, got {type(values).__name__}")
    if not dc.is_dataclass(cls):
        raise TypeError(f"{cls!r} is not a dataclass")
    if not safe:
        return cls(**values)

    schema = _ensure_schema(
        cls,
        resolved_hints=resolved_hints,
        resolved_cache=resolved_cache,
    )
    if physical_root is not None:
        _validate_physical_struct_names(
            physical_root,
            (field.name for field in schema.value_fields),
            path=path,
        )
    unknown: list[object] | None = None
    for key in values:
        if key not in schema.constructor_names:
            if unknown is None:
                unknown = []
            unknown.append(key)
    if unknown:
        raise _UnknownFieldError(
            f"{path}: unknown keys {sorted(unknown, key=repr)!r}",
            matched=any(key in schema.constructor_names for key in values),
        )

    initial_bindings: Mapping[object, object] = bindings or {}
    binding_contexts = (
        _binding_contexts(cls, initial_bindings) if bindings else None
    )
    converted: dict[str, Any] = {}
    for field in schema.constructor_fields:
        field_path = f"{path}.{field.name}"
        if field.name not in values:
            # Omission follows normal dataclass construction: declared defaults
            # are always honored. The error policy only changes how an invalid
            # value that was actually supplied is handled.
            if _has_default(field):
                continue
            raise TypeError(f"{field_path}: missing required value")
        hint = schema.hints.get(field.name, field.type)
        if binding_contexts is not None:
            hint = _bind_hint(
                hint,
                binding_contexts.get(
                    _annotation_owner(cls, field.name),
                    initial_bindings,
                ),
            )
        if field.name not in schema.field_lookup:
            physical_field = None
        elif physical_root is None:
            physical_field = schema.field_lookup[field.name]
        else:
            physical_field = _physical_named_child(
                physical_root,
                field.name,
                path=field_path,
            )
        try:
            converted[field.name] = _convert(
                values[field.name],
                hint,
                conversion_owner or cls,
                field_path,
                errors,
                physical_field=physical_field,
            )
        except _UnknownFieldError:
            raise
        except (TypeError, ValueError):
            if errors == "default" and _has_default(field):
                continue
            raise
    instance = cls(**converted)
    # Dataclass construction owns default/default_factory evaluation. Validate
    # the resulting values exactly once afterward so omission, errors=default,
    # and init=False fields cannot bypass the cached native Field contract.
    for value_field in schema.value_fields:
        if value_field.name in converted:
            continue
        value_path = f"{path}.{value_field.name}"
        hint = schema.hints.get(value_field.name, value_field.type)
        if bindings:
            hint = _bind_hint(hint, bindings)
        physical_field = (
            schema.field_lookup[value_field.name]
            if physical_root is None
            else _physical_named_child(
                physical_root,
                value_field.name,
                path=value_path,
            )
        )
        _convert(
            getattr(instance, value_field.name),
            hint,
            conversion_owner or cls,
            value_path,
            errors,
            physical_field=physical_field,
        )
    return instance


def from_dict(
    cls: type[_T],
    values: Mapping[str, Any],
    *,
    safe: bool = True,
    errors: _ErrorPolicy = "raise",
) -> _T:
    """Construct a dataclass from a mapping.

    ``safe=True`` recursively validates and losslessly casts annotated values.
    With ``errors='default'``, a missing or invalid known field is omitted only
    when it declares a default or default factory.
    """

    if not isinstance(cls, type):
        raise TypeError(f"{cls!r} is not a dataclass type")
    return _from_dict(cls, values, safe=safe, errors=errors, path=cls.__name__)


def _export(
    value: Any,
    resolved_cache: Mapping[type[Any], Mapping[str, Any]],
    conversion_owner: type[Any],
    hint: Any = Any,
    physical_field: NativeField | None = None,
) -> Any:
    if _accept_schema_null(value, physical_field, "into_dict"):
        return None
    hint = _unwrap_hint(hint, conversion_owner)
    origin = get_origin(hint)
    if origin in _UNION_ORIGINS:
        union_hint = hint
        value, hint = _convert_union_branch(
            value,
            hint,
            conversion_owner,
            "into_dict",
            "raise",
            physical_field=physical_field,
        )
        physical_field = _physical_union_child_for_hint(
            physical_field, union_hint, hint, conversion_owner
        )
        hint = _unwrap_hint(hint, conversion_owner)
        origin = get_origin(hint)

    if dc.is_dataclass(value) and not isinstance(value, type):
        declared = origin if isinstance(origin, type) and dc.is_dataclass(origin) else None
        if declared is not None:
            bindings = dict(
                zip(getattr(declared, "__parameters__", ()), get_args(hint))
            )
        else:
            bindings = {}
        return _to_dict(
            value,
            safe=True,
            resolved_cache=resolved_cache,
            conversion_owner=conversion_owner,
            bindings=bindings,
            physical_root=physical_field,
        )

    arguments = get_args(hint)
    if isinstance(value, cabc.Mapping):
        key_hint, item_hint = (
            arguments[:2] if len(arguments) >= 2 else (Any, Any)
        )
        structured = origin or hint
        annotations: Mapping[str, Any] | None = None
        if _is_typed_dict_class(structured):
            annotations = resolved_cache.get(structured)
            if annotations is not None:
                direct = dict(
                    zip(
                        getattr(structured, "__parameters__", ()),
                        arguments,
                    )
                )
                bindings = _inherited_bindings(structured, direct)
                annotations = {
                    name: _bind_hint(annotation, bindings)
                    for name, annotation in annotations.items()
                }
        if annotations is not None:
            return {
                _export(key, resolved_cache, conversion_owner, key_hint): _export(
                    item,
                    resolved_cache,
                    conversion_owner,
                    annotations.get(key, item_hint),
                    _physical_named_child(
                        physical_field, str(key), path=f"into_dict.{key}"
                    ),
                )
                for key, item in value.items()
            }
        key_field, item_field = _physical_map_children(
            physical_field, path="into_dict"
        )
        return {
            _export(
                key,
                resolved_cache,
                conversion_owner,
                key_hint,
                key_field,
            ): _export(
                item,
                resolved_cache,
                conversion_owner,
                item_hint,
                item_field,
            )
            for key, item in value.items()
        }
    if isinstance(value, list):
        item_hint = arguments[0] if arguments else Any
        item_field = _physical_list_child(physical_field, path="into_dict")
        return [
            _export(
                item,
                resolved_cache,
                conversion_owner,
                item_hint,
                item_field,
            )
            for item in value
        ]
    if isinstance(value, tuple):
        item_hints: tuple[Any, ...]
        repeated_item_field: NativeField | None = None
        structured = origin or hint
        annotations = (
            resolved_cache.get(structured)
            if isinstance(structured, type) and hasattr(structured, "_fields")
            else None
        )
        if annotations is not None:
            direct = dict(
                zip(getattr(structured, "__parameters__", ()), arguments)
            )
            bindings = _inherited_bindings(structured, direct)
            item_hints = tuple(
                _bind_hint(annotation, bindings)
                for annotation in annotations.values()
            )
        elif len(arguments) == 2 and arguments[1] is Ellipsis:
            item_hints = (arguments[0],) * len(value)
            repeated_item_field = _physical_list_child(
                physical_field, path="into_dict"
            )
        elif not arguments and physical_field is not None:
            dtype = _physical_dtype(physical_field)
            if dtype.id in _LIST_DATA_TYPE_KINDS:
                item_hints = (Any,) * len(value)
                repeated_item_field = dtype[0]
            else:
                item_hints = arguments
        else:
            item_hints = arguments
        converted = tuple(
            _export(
                item,
                resolved_cache,
                conversion_owner,
                item_hints[index] if index < len(item_hints) else Any,
                (
                    repeated_item_field
                    if repeated_item_field is not None
                    else _physical_positional_child(
                        physical_field, index, path=f"into_dict[{index}]"
                    )
                ),
            )
            for index, item in enumerate(value)
        )
        if hasattr(value, "_fields"):
            return type(value)(*converted)
        return converted
    if isinstance(value, set):
        item_hint = arguments[0] if arguments else Any
        item_field = _physical_list_child(physical_field, path="into_dict")
        return {
            _export(
                item,
                resolved_cache,
                conversion_owner,
                item_hint,
                item_field,
            )
            for item in value
        }
    if isinstance(value, frozenset):
        item_hint = arguments[0] if arguments else Any
        item_field = _physical_list_child(physical_field, path="into_dict")
        return frozenset(
            _export(
                item,
                resolved_cache,
                conversion_owner,
                item_hint,
                item_field,
            )
            for item in value
        )
    if isinstance(value, collections.deque):
        item_hint = arguments[0] if arguments else Any
        item_field = _physical_list_child(physical_field, path="into_dict")
        return collections.deque(
            (
                _export(
                    item,
                    resolved_cache,
                    conversion_owner,
                    item_hint,
                    item_field,
                )
                for item in value
            ),
            maxlen=value.maxlen,
        )
    return value


def _to_dict(
    value: _T,
    *,
    safe: bool,
    resolved_cache: Mapping[type[Any], Mapping[str, Any]] | None,
    conversion_owner: type[Any] | None,
    bindings: Mapping[object, object] | None = None,
    physical_root: NativeField | None = None,
    drop_nulls: bool = False,
) -> dict[str, Any]:
    if type(drop_nulls) is not bool:
        raise TypeError(f"drop_nulls must be bool, got {type(drop_nulls).__name__}")
    _, context_cache, owner, projected = _project_dataclass_values(
        value,
        safe=safe,
        resolved_cache=resolved_cache,
        conversion_owner=conversion_owner,
        bindings=bindings,
        physical_root=physical_root,
    )
    if not safe:
        return {
            name: converted
            for name, converted, _, _ in projected
            if not (drop_nulls and converted is None)
        }
    # Export first: a field may only become ``None`` once it is lowered.
    exported = (
        (name, _export(converted, context_cache, owner, export_hint, field))
        for name, converted, export_hint, field in projected
    )
    return {
        name: converted
        for name, converted in exported
        if not (drop_nulls and converted is None)
    }


def _project_dataclass_values(
    value: object,
    *,
    safe: bool,
    resolved_cache: Mapping[type[Any], Mapping[str, Any]] | None,
    conversion_owner: type[Any] | None,
    bindings: Mapping[object, object] | None = None,
    physical_root: NativeField | None = None,
) -> tuple[
    type[Any],
    Mapping[type[Any], Mapping[str, Any]],
    type[Any],
    typing.Iterator[tuple[str, Any, Any, NativeField | None]],
]:
    """Share one dataclass validation/casting projection across exports."""

    if type(safe) is not bool:
        raise TypeError("safe must be bool")
    if isinstance(value, type):
        raise TypeError("into_dict expects a dataclass instance")
    cls = _dataclass_type(value)
    if not safe:
        fields = _value_fields(cls)
        projected = (
            (field.name, getattr(value, field.name), field.type, None)
            for field in fields
        )
        return cls, {}, conversion_owner or cls, projected
    schema = _ensure_schema(
        cls,
        resolved_hints=resolved_cache.get(cls) if resolved_cache else None,
        resolved_cache=resolved_cache,
    )
    if physical_root is not None:
        _validate_physical_struct_names(
            physical_root,
            (field.name for field in schema.value_fields),
            path=cls.__name__,
        )
    owner = conversion_owner or cls
    context_cache = resolved_cache or schema.nested_hints
    initial_bindings: Mapping[object, object] = bindings or {}
    binding_contexts = (
        _binding_contexts(cls, initial_bindings) if bindings else None
    )

    def project() -> typing.Iterator[tuple[str, Any, Any, NativeField | None]]:
        for field in schema.value_fields:
            path = f"{cls.__name__}.{field.name}"
            hint = schema.hints.get(field.name, field.type)
            if binding_contexts is not None:
                hint = _bind_hint(
                    hint,
                    binding_contexts.get(
                        _annotation_owner(cls, field.name),
                        initial_bindings,
                    ),
                )
            raw = getattr(value, field.name)
            if field.name not in schema.field_lookup:
                physical_field = None
            elif physical_root is None:
                physical_field = schema.field_lookup[field.name]
            else:
                physical_field = _physical_named_child(
                    physical_root,
                    field.name,
                    path=path,
                )
            resolved_hint = _unwrap_hint(hint, owner)
            if get_origin(resolved_hint) in _UNION_ORIGINS:
                union_hint = resolved_hint
                converted, export_hint = _convert_union_branch(
                    raw,
                    resolved_hint,
                    owner,
                    path,
                    "raise",
                    physical_field=physical_field,
                )
                if converted is not None:
                    physical_field = _physical_union_child_for_hint(
                        physical_field, union_hint, export_hint, owner
                    )
            else:
                converted = _convert(
                    raw,
                    hint,
                    owner,
                    path,
                    "raise",
                    physical_field=physical_field,
                )
                export_hint = hint
            yield field.name, converted, export_hint, physical_field

    return cls, context_cache, owner, project()


def into_dict(
    value: _T, *, safe: bool = True, drop_nulls: bool = False
) -> dict[str, Any]:
    """Return dataclass fields as a dictionary, recursively lowering classes.

    ``drop_nulls`` omits every top-level key whose exported value is ``None``.
    Nested dataclasses keep their own nulls, because dropping them there would
    change the shape a schema declares rather than trimming an optional key.
    """

    return _to_dict(
        value,
        safe=safe,
        resolved_cache=None,
        conversion_owner=None,
        drop_nulls=drop_nulls,
    )


def _scope_base(cls: type[Any]) -> tuple[str, str]:
    scope, _, _ = cls.__qualname__.rpartition(".")
    return cls.__module__, scope


def _scope_key(cls: type[Any]) -> tuple[str, str, object | None]:
    return (*_scope_base(cls), getattr(cls, "__yggdryl_scope_token__", None))


def _register_field_class(
    cls: type[Any],
    localns: Mapping[str, Any] | None,
    token: _ScopeToken,
) -> None:
    namespace = dict(localns or ())
    setattr(cls, "__yggdryl_scope_token__", token)
    with _SCHEMA_LOCK:
        # Keep only annotation-reachable bindings, never the decorator frame.
        relevant = _relevant_namespace(cls, namespace)
        setattr(cls, "__yggdryl_pending_namespace__", (id(cls), relevant))
        _PENDING_SCHEMAS[cls] = token

    # A later sibling may satisfy a name captured by an earlier decorator.
    # Update only classes from the same lexical scope; actual schema building
    # stays lazy until their cached ``field`` staticmethod is called.
    scope = _scope_key(cls)
    with _SCHEMA_LOCK:
        pending = tuple(_PENDING_SCHEMAS.items())
    for candidate, pending_token in pending:
        if candidate is cls or _scope_key(candidate) != scope:
            continue
        with _SCHEMA_LOCK:
            candidate_namespace = _pending_namespace(candidate)
            if candidate_namespace is None or pending_token is not token:
                continue
            merged_namespace = dict(candidate_namespace)
            merged_namespace.update(namespace)
            merged_namespace[cls.__name__] = cls
            candidate_namespace.clear()
            candidate_namespace.update(
                _relevant_namespace(candidate, merged_namespace)
            )


def _hide_private_annotations(
    cls: type[Any], annotations: Mapping[str, Any] | None = None
) -> None:
    annotations = dict(
        _unevaluated_annotations(cls) if annotations is None else annotations
    )
    if not annotations:
        return
    mangled = f"_{cls.__name__.lstrip('_')}__"
    for name in tuple(annotations):
        if name.startswith("__") or name.startswith(mangled):
            del annotations[name]
            # An unannotated dataclasses.Field is itself an error. Other
            # private defaults remain ordinary class attributes.
            if isinstance(cls.__dict__.get(name), dc.Field):
                delattr(cls, name)
    # Python 3.14 defers annotations behind ``__annotate_func__``. Publishing
    # the non-evaluating string view keeps dataclasses from executing a later
    # sibling reference during decoration and leaves get_type_hints to resolve
    # it lazily when ``field`` is first requested.
    setattr(cls, "__annotations__", annotations)


def _decorate_field_class(
    candidate: type[_T],
    options: Mapping[str, Any],
    localns: Mapping[str, Any],
    token: _ScopeToken,
) -> type[_T]:
    annotations = _unevaluated_annotations(candidate)
    inherited_fields = getattr(candidate, "__dataclass_fields__", {})
    resolved_accessor = _resolved_field_descriptor(candidate)
    if (
        "field" in candidate.__dict__
        or "field" in annotations
        or "field" in inherited_fields
        or (
            resolved_accessor is not _MISSING_FIELD_DESCRIPTOR
            and not _is_installed_field_descriptor(resolved_accessor)
        )
    ):
        raise TypeError(
            f"{candidate.__module__}.{candidate.__qualname__} reserves "
            "field for its cached native Field staticmethod"
        )
    original_doc = candidate.__doc__
    _hide_private_annotations(candidate, annotations)
    decorated = dc.dataclass(candidate, **options)
    # ``dataclasses`` synthesizes a signature-shaped class docstring when the
    # source class had none. Restore the source value so schema descriptions
    # are documentation, never generated constructor text.
    decorated.__doc__ = original_doc
    setattr(decorated, "__yggdryl_field_class__", True)
    _install_field_staticmethod(decorated)
    _register_field_class(decorated, localns, token)
    return decorated


def iter_records(reader: Any, cls: type[Any] | None = None) -> typing.Iterator[Any]:
    """Yield mappings or dataclass instances from an Arrow batch reader.

    Only the current batch is lowered to Python mappings. Dataclass conversion
    uses the same recursive native-field-aware conversion as ``from_dict``;
    this is a row adapter, never a second schema implementation.
    """

    if cls is not None and (not isinstance(cls, type) or not dc.is_dataclass(cls)):
        raise TypeError(f"{cls!r} is not a dataclass type")

    def rows() -> typing.Iterator[Any]:
        for batch in reader:
            for values in batch.to_pylist():
                yield values if cls is None else from_dict(cls, values)

    return rows()


__all__ = ["field"]
