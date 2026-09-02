from __future__ import annotations

import collections
import collections.abc as cabc
import datetime
import decimal
import enum
import pathlib
import typing
import uuid
from dataclasses import dataclass
from typing import Annotated, Any, Generic, Literal, NamedTuple, NewType, TypeVar, TypedDict

import pyarrow as pa
import pytest

from yggdryl import DataType, Field, Uri, Url, Urn


class QuoteDict(TypedDict):
    symbol: str
    price: decimal.Decimal
    venue: str | None


class Point(NamedTuple):
    x: float
    y: float


@dataclass
class Quote:
    symbol: str
    sizes: list[int | None]
    point: Point


T = TypeVar("T")


@dataclass
class Box(Generic[T]):
    value: T


@dataclass
class IntBox(Box[int]):
    label: str = ""


class Side(enum.Enum):
    BID = "bid"
    ASK = "ask"


class Mixed(enum.Enum):
    ENABLED = True
    COUNT = 2


UserId = NewType("UserId", int)


def test_scalar_hints_have_native_arrow_equivalents() -> None:
    expected = {
        Any: "null",
        object: "null",
        type(None): "null",
        bool: "boolean",
        int: "int64",
        float: "float64",
        str: "utf8",
        bytes: "binary",
        bytearray: "binary",
        memoryview: "binary",
        datetime.datetime: "timestamp",
        datetime.date: "date32",
        datetime.time: "time64",
        datetime.timedelta: "duration64",
        decimal.Decimal: "decimal128",
        uuid.UUID: "utf8",
        pathlib.Path: "utf8",
        Uri: "utf8",
        Url: "utf8",
        Urn: "utf8",
    }

    for hint, kind in expected.items():
        assert DataType.from_pyhint(hint).id == kind

    assert DataType.from_pyhint(datetime.datetime).into_arrow() == pa.timestamp(
        "us", tz="UTC"
    )
    assert DataType.from_pyhint(decimal.Decimal).into_arrow() == pa.decimal128(38, 18)


def test_scalar_subclasses_keep_their_physical_type() -> None:
    class Count(int):
        note: str

    class EventTime(datetime.datetime):
        pass

    class Price(decimal.Decimal):
        pass

    assert DataType.from_pyhint(Count).id == "int64"
    assert DataType.from_pyhint(EventTime).id == "timestamp"
    assert DataType.from_pyhint(Price).id == "decimal128"


def test_only_explicit_none_makes_fields_nullable() -> None:
    assert not Field.from_pyhint("plain", int).nullable
    assert not Field.from_pyhint("default_is_not_inspected", Any).nullable
    assert Field.from_pyhint("optional", int | None).nullable
    assert Field.from_pyhint("none", None).nullable
    assert Field.from_pyhint("literal", Literal["ok", None]).nullable
    assert not Field.from_pyhint("literal", Literal["ok"]).nullable
    assert DataType.from_pyhint(int | None) == DataType.from_pyhint(int)


def test_annotated_metadata_is_string_only_and_explicit_values_win() -> None:
    hint = Annotated[int | None, {"unit": "lots", "source": "annotation"}]
    field = Field.from_pyhint(
        "quantity",
        hint,
        metadata={"source": "caller", "role": "size"},
    )

    assert field.nullable
    assert dict(field.metadata.items()) == {
        "role": "size",
        "source": "caller",
        "unit": "lots",
    }
    with pytest.raises(TypeError, match="str keys to str values"):
        Field.from_pyhint("bad", Annotated[int, {"precision": 4}])


def test_collection_hints_preserve_nested_nullability_and_order() -> None:
    listed = DataType.from_pyhint(list[int | None])
    fixed = DataType.from_pyhint(tuple[int, str | None])
    mapping = DataType.from_pyhint(dict[str, int | None])

    assert listed.id == "list"
    assert listed[0].name == "item"
    assert listed[0].nullable
    assert [field.name for field in fixed] == ["_1", "_2"]
    assert not fixed[0].nullable
    assert fixed[1].nullable
    assert mapping.id == "map"
    entries = mapping[0].dtype
    assert [field.name for field in entries] == ["key", "value"]
    assert not entries[0].nullable
    assert entries[1].nullable
    assert DataType.from_arrow(mapping.into_arrow()) == mapping

    assert DataType.from_pyhint(cabc.Iterable).id == "list"
    assert DataType.from_pyhint(typing.Tuple).id == "list"
    assert DataType.from_pyhint(tuple[()]).id == "struct"
    assert DataType.from_pyhint(cabc.Generator[str, None, None])[0].dtype.id == "utf8"
    items = DataType.from_pyhint(cabc.ItemsView[str, int])
    assert [field.name for field in items[0].dtype] == ["_1", "_2"]
    with pytest.raises(TypeError, match="nullable map key"):
        DataType.from_pyhint(dict[str | None, int])


def test_nested_hint_inference_uses_native_builders_without_pyarrow_round_trips(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    import yggdryl.fields._hints as hint_impl

    def unexpected_arrow_factory(*args: object, **kwargs: object) -> object:
        raise AssertionError(
            f"annotation inference called a PyArrow schema factory: {args!r}, {kwargs!r}"
        )

    for name in ("list_", "map_", "struct", "union"):
        monkeypatch.setattr(pa, name, unexpected_arrow_factory)
    original_import_module = hint_impl.importlib.import_module

    def import_without_pyarrow(name: str, package: str | None = None) -> object:
        if name == "pyarrow":
            raise AssertionError("annotation inference imported PyArrow")
        return original_import_module(name, package)

    monkeypatch.setattr(hint_impl.importlib, "import_module", import_without_pyarrow)

    @dataclass
    class NativeNested:
        labels: list[Annotated[int | None, {"unit": "ticks"}]]
        dimensions: dict[str, int]

    inferred = DataType.from_pyhint(NativeNested)
    labels = inferred["labels"].dtype
    dimensions = inferred["dimensions"].dtype

    assert inferred.id == "struct"
    assert labels.id == "list"
    assert labels[0].nullable
    assert labels[0].metadata["unit"] == "ticks"
    assert dimensions.id == "map"
    assert [field.name for field in dimensions[0].dtype] == ["key", "value"]

    deep: object = Annotated[int, {"depth": "leaf"}]
    for _ in range(16):
        deep = list[deep]  # type: ignore[valid-type]
    current = DataType.from_pyhint(deep)
    leaf: Field | None = None
    for _ in range(16):
        assert current.id == "list"
        leaf = current[0]
        current = leaf.dtype
    assert leaf is not None
    assert current.id == "int64"
    assert leaf.metadata["depth"] == "leaf"


def test_items_view_and_union_inference_preserve_native_child_state() -> None:
    items = DataType.from_pyhint(
        cabc.ItemsView[Annotated[str, {"role": "key"}], int | None]
    )
    pair = items[0].dtype
    union = DataType.from_pyhint(Annotated[int, {"source": "integer"}] | str)

    assert items.id == "list"
    assert pair.id == "struct"
    assert pair[0].metadata["role"] == "key"
    assert pair[1].nullable
    assert union.id == "union"
    assert union[0].metadata["source"] == "integer"
    assert [field.dictionary_id for field in union] == [None, None]
    assert union.into_arrow().mode == "dense"
    assert tuple(union.into_arrow().type_codes) == (0, 1)


def test_deep_union_inference_assigns_exact_tags_at_each_variant_boundary() -> None:
    hint = list[
        dict[str, Annotated[int, {"branch": "count"}] | str]
        | tuple[bytes, float]
    ]

    inferred = DataType.from_pyhint(hint)
    outer = inferred[0].dtype
    mapping = outer[0].dtype
    mapping_value = mapping[0].dtype[1].dtype

    assert inferred.id == "list"
    assert outer.id == "union"
    assert outer.into_arrow().mode == "dense"
    assert tuple(outer.into_arrow().type_codes) == (0, 1)
    assert [member.dtype.id for member in outer] == ["map", "struct"]
    assert mapping_value.id == "union"
    assert tuple(mapping_value.into_arrow().type_codes) == (0, 1)
    assert mapping_value[0].metadata["branch"] == "count"


def test_counter_and_generic_mapping_subclasses_keep_parameters() -> None:
    counter = DataType.from_pyhint(collections.Counter[str])
    chain = DataType.from_pyhint(collections.ChainMap[str, int])

    counter_entries = counter[0].dtype
    chain_entries = chain[0].dtype
    assert [field.dtype.id for field in counter_entries] == ["utf8", "int64"]
    assert [field.dtype.id for field in chain_entries] == ["utf8", "int64"]


def test_struct_hints_are_deterministic_and_keep_class_identity() -> None:
    quote_dict = DataType.from_pyhint(QuoteDict)
    point = DataType.from_pyhint(Point)
    quote = DataType.from_pyhint(Quote)
    boxed = DataType.from_pyhint(Box[int])
    inherited_box = DataType.from_pyhint(IntBox)

    assert [field.name for field in quote_dict] == ["symbol", "price", "venue"]
    assert quote_dict[2].nullable
    assert [field.name for field in point] == ["x", "y"]
    assert [field.name for field in quote] == ["symbol", "sizes", "point"]
    assert boxed[0].dtype.id == "int64"
    assert inherited_box[0].dtype.id == "int64"

    root = Field.from_pyhint("quote", Quote)
    assert root.metadata["python.module"] == __name__
    assert root.metadata["python.class"] == "Quote"
    assert root.metadata["python.qualname"] == "Quote"
    assert root.metadata["python.kind"] == "dataclass"


def test_literal_enum_newtype_typevar_and_union_inference() -> None:
    constrained = TypeVar("constrained", int, str)
    bounded = TypeVar("bounded", bound=float)

    assert DataType.from_pyhint(Literal[1, 2]).id == "int64"
    assert DataType.from_pyhint(Side).id == "utf8"
    assert DataType.from_pyhint(UserId).id == "int64"
    assert DataType.from_pyhint(bounded).id == "float64"

    union = DataType.from_pyhint(constrained)
    assert union.id == "union"
    assert [field.name for field in union] == ["int", "str"]

    # bool is deliberately not collapsed into Python's int physical type.
    mixed = DataType.from_pyhint(Mixed)
    assert mixed.id == "union"
    assert [field.dtype.id for field in mixed] == ["boolean", "int64"]


def test_typing_extensions_wrappers_match_stdlib_semantics() -> None:
    typing_extensions = pytest.importorskip("typing_extensions")
    required = typing_extensions.Required[int]
    optional_key = typing_extensions.NotRequired[str | None]
    annotated = typing_extensions.Annotated[int, {"unit": "ticks"}]

    assert not Field.from_pyhint("required", required).nullable
    assert Field.from_pyhint("optional_key", optional_key).nullable
    assert Field.from_pyhint("annotated", annotated).metadata["unit"] == "ticks"


def test_internal_namespace_resolves_deep_local_struct_annotations() -> None:
    typing_extensions = pytest.importorskip("typing_extensions")
    ReadOnly = typing_extensions.ReadOnly
    LocalScalar = Annotated[int, {"unit": "local"}]

    class LocalPayload(TypedDict):
        value: ReadOnly[LocalScalar]

    class LocalPair(NamedTuple):
        payload: LocalPayload

    @dataclass
    class LocalEnvelope:
        pair: LocalPair

    Shadowed = str

    @dataclass
    class LocalShadow:
        Shadowed = int
        value: Shadowed

    from yggdryl.fields._hints import _field_from_pyhint

    root = _field_from_pyhint(
        "envelope",
        LocalEnvelope,
        localns=locals(),
    )
    value = root.dtype["pair"].dtype["payload"].dtype["value"]
    assert value.dtype.id == "int64"
    assert value.metadata["unit"] == "local"
    shadow = _field_from_pyhint(
        "shadow",
        LocalShadow,
        localns=locals(),
    )
    assert shadow.dtype["value"].dtype.id == "int64"


@pytest.mark.skipif(
    not hasattr(typing, "TypeAliasType"), reason="PEP 695 requires Python 3.12+"
)
def test_pep695_alias_members_preserve_none_and_annotated_metadata() -> None:
    namespace: dict[str, object] = {"Annotated": Annotated, "__name__": __name__}
    exec(
        "type Nil = None\n"
        "type Maybe[T] = T | None\n"
        "type Tagged[T] = Annotated[T, {'unit': 'alias'}]\n"
        "type Fixed = Annotated[int, {'source': 'fixed'}]\n",
        namespace,
    )
    nil = namespace["Nil"]
    maybe = namespace["Maybe"]
    tagged = namespace["Tagged"]
    fixed = namespace["Fixed"]

    alias_member = Field.from_pyhint("value", int | nil)  # type: ignore[operator]
    generic_optional = Field.from_pyhint("value", maybe[int])  # type: ignore[index]
    generic_tagged = Field.from_pyhint("value", tagged[int])  # type: ignore[index]
    fixed_tagged = Field.from_pyhint("value", fixed)

    assert alias_member.nullable and alias_member.dtype.id == "int64"
    assert generic_optional.nullable and generic_optional.dtype.id == "int64"
    assert generic_tagged.metadata["unit"] == "alias"
    assert fixed_tagged.metadata["source"] == "fixed"
    assert generic_tagged.metadata["python.kind"] == "type_alias"


def test_recursive_deep_and_unresolved_annotations_fail_cleanly() -> None:
    @dataclass
    class Node:
        child: Node | None

    with pytest.raises(TypeError, match="recursive"):
        DataType.from_pyhint(Node)

    nested: object = int
    for _ in range(70):
        nested = list[nested]  # type: ignore[valid-type]
    with pytest.raises(TypeError, match="depth"):
        DataType.from_pyhint(nested)

    with pytest.raises(TypeError, match="unresolved forward"):
        DataType.from_pyhint("MissingType")
    assert DataType.from_pyhint("int").id == "int64"
