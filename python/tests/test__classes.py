"""The field classes: the dataclass projection, its Arrow half, and its edges."""

from __future__ import annotations

import dataclasses
import dataclasses as dc
import inspect
import sys
import threading
from concurrent.futures import ThreadPoolExecutor
from typing import Annotated, ClassVar, Generic, TypeVar, TypedDict

import pyarrow as pa
import pytest

import yggdryl
from yggdryl import DataType, Field, field, json, scalar

@scalar(frozen=True, slots=True)
class Leg:
    """One order leg."""

    symbol: str
    quantity: int


@scalar(frozen=True, slots=True)
class Order:
    """An executable order.

    Args:
        order_id: Stable order identifier.
        legs: Ordered executions.
        note: Optional caller note.
    """

    order_id: Annotated[int, {"id": 7}]
    legs: list[Leg]
    note: str | None = None
    category: ClassVar[str] = "cash"
    __scratch: int


def test_scalar_decorator_builds_an_ordinary_dataclass_with_native_field() -> None:
    value = Order(42, [Leg("ABC", 3)])

    assert dataclasses.is_dataclass(Order)
    assert dataclasses.is_dataclass(value)
    assert not hasattr(value, "__dict__")
    assert Order.category == "cash"
    assert tuple(item.name for item in dataclasses.fields(Order)) == (
        "order_id",
        "legs",
        "note",
    )

    root = Order.into_field()
    assert isinstance(root, Field)
    assert isinstance(Order.__dict__["into_field"], staticmethod)
    assert Order.__dict__["into_field"].__func__.__name__ == "into_field"
    assert root is Order.into_field()
    assert root is value.into_field()
    assert root is field(Order)
    assert root is field(value)
    assert root.name == "Order"
    assert root.dtype.id == "struct"
    assert tuple(child.name for child in root.dtype) == (
        "order_id",
        "legs",
        "note",
    )
    assert root.python.kind == "field"
    assert root.metadata["description"] == "An executable order."
    assert root.dtype["order_id"].metadata["description"] == (
        "Stable order identifier."
    )
    assert root.dtype["order_id"].parquet_field_id == 7

    renamed = field(Order, name="order")
    assert renamed is not root
    assert renamed.name == "order"
    assert renamed.dtype == root.dtype


def test_scalar_decorator_is_colocated_with_the_native_scalar_boundary() -> None:
    # The module defines it and the package root binds it, the way the crate
    # root re-exports what each of its files owns.
    assert callable(yggdryl.scalar)
    assert scalar.__module__ == "yggdryl.scalar"
    assert sys.modules["yggdryl.scalar"].scalar is scalar


def test_field_accessor_signatures_are_uniform_and_class_metadata_is_argument_free() -> None:
    signature = inspect.signature(field)

    assert tuple(signature.parameters) == ("value", "name")
    assert signature.parameters["name"].default is None
    assert tuple(inspect.signature(Order.into_field).parameters) == ()
    with pytest.raises(TypeError):
        Order.into_field("renamed")  # type: ignore[call-arg]


def test_codec_materialization_is_recursive_and_schema_checked() -> None:
    encoded = json.dumps(
        {
            "order_id": "42",
            "legs": [{"symbol": "ABC", "quantity": "3"}],
        }
    )
    value = json.loads(encoded, cls=Order)
    assert value == Order(42, [Leg("ABC", 3)])
    assert json.loads(json.dumps(value)) == {
        "order_id": 42,
        "legs": [{"symbol": "ABC", "quantity": 3}],
        "note": None,
    }

    with pytest.raises((TypeError, ValueError), match="unknown"):
        json.loads(
            json.dumps({"order_id": 1, "legs": [], "unknown": True}),
            cls=Order,
        )

    shallow = json.loads(
        encoded,
        cls=Order,
        safe=False,
    )
    assert shallow.order_id == "42"
    assert isinstance(shallow.legs[0], dict)


def test_plain_dataclasses_compile_to_the_same_native_field_model() -> None:
    @dataclasses.dataclass
    class Point:
        x: int
        y: int | None = None

    root = field(Point)
    assert root is field(Point(1))
    assert root.python.kind == "dataclass"
    assert tuple(child.name for child in root.dtype) == ("x", "y")
    assert json.loads('{"x":"3"}', cls=Point) == Point(3)


def test_later_local_annotations_are_resolved_lazily() -> None:
    @scalar
    class Parent:
        child: Child

    assert "__yggdryl_class_schema__" not in Parent.__dict__

    @scalar
    class Child:
        count: int

    assert (
        Parent.into_field().dtype["child"].dtype
        == Child.into_field().dtype
    )
    assert json.loads('{"child":{"count":"4"}}', cls=Parent) == Parent(Child(4))


def test_nested_field_class_keeps_its_native_datatype() -> None:
    child = Field(
        "payload",
        DataType.from_fields((Field("narrow", "int16", nullable=False),)),
        nullable=False,
        metadata={"owner": "child"},
    )
    Payload = child.into_dataclass(name="Payload", module=__name__)

    @scalar
    class Envelope:
        payload: Payload

    assert Payload.into_field() is child
    assert (
        Envelope.into_field()
        .dtype["payload"]
        .dtype["narrow"]
        .dtype.id
        == "int16"
    )


def test_generated_class_field_is_authoritative_for_pyhint_inference() -> None:
    root = Field(
        "payload",
        DataType.from_fields(
            (
                Field("narrow", "int16", nullable=False),
                Field("wide", "uint32", nullable=False),
            )
        ),
        nullable=False,
    )
    Generated = root.into_dataclass(name="Generated")

    assert Generated.into_field() is root
    assert DataType.from_pyhint(Generated) == root.dtype
    inferred = Field.from_pyhint("generated", Generated)
    assert inferred.dtype == root.dtype
    assert inferred.dtype["narrow"].dtype.id == "int16"


def test_inherited_generated_fields_keep_their_exact_native_layout() -> None:
    narrow = Field(
        "narrow",
        "int16",
        nullable=False,
        metadata={"unit": "ticks"},
    )
    lookup = Field.from_arrow(
        pa.field(
            "lookup",
            pa.map_(pa.int8(), pa.string(), keys_sorted=True),
            nullable=False,
        )
    )
    category = Field.from_arrow(
        pa.field(
            "category",
            pa.dictionary(pa.int8(), pa.string(), ordered=True),
            nullable=False,
        )
    )
    category.set_dictionary_options(37, True)
    root = Field(
        "Base",
        DataType.from_fields((narrow, lookup, category)),
        nullable=False,
    )
    Base = root.into_dataclass(name="Base")

    @scalar
    class DecoratedChild(Base):
        extra: int

    @dataclasses.dataclass
    class PlainChild(Base):
        extra: int

    for child in (DecoratedChild.into_field(), field(PlainChild)):
        assert child["narrow"] == root["narrow"]
        assert child["lookup"] == root["lookup"]
        assert child["category"] == root["category"]
        assert child["lookup"].into_arrow().type.keys_sorted
        assert child["category"].dictionary_id == 37


def test_nested_subclasses_keep_exact_fields_before_their_own_field_access() -> None:
    root = Field(
        "Base",
        DataType.from_fields(
            (
                Field(
                    "narrow",
                    "int16",
                    nullable=False,
                    metadata={"unit": "ticks"},
                ),
            )
        ),
        nullable=False,
    )
    Base = root.into_dataclass(name="ExactBase")

    @scalar
    class LazyChild(Base):
        extra: int

    @scalar
    class DecoratedParent:
        child: LazyChild

    assert "__yggdryl_class_schema__" not in LazyChild.__dict__
    assert DecoratedParent.into_field()["child"]["narrow"] == root["narrow"]
    lazy_root = LazyChild.into_field()
    assert lazy_root is LazyChild.into_field()
    assert lazy_root["narrow"] == root["narrow"]

    @dataclasses.dataclass
    class PlainChild(Base):
        extra: int

    assert DataType.from_pyhint(PlainChild)["narrow"] == root["narrow"]

    @scalar
    class PlainParent:
        child: PlainChild

    assert PlainParent.into_field()["child"]["narrow"] == root["narrow"]

    ItemT = TypeVar("ItemT")

    @scalar
    class GenericChild(Base, Generic[ItemT]):
        item: ItemT

    @scalar
    class GenericParent:
        child: GenericChild[str]

    nested = GenericParent.into_field()["child"]
    assert nested["narrow"] == root["narrow"]
    assert nested["item"].dtype.id == "utf8"


def test_plain_dataclass_into_field_attribute_does_not_override_annotations() -> None:
    unrelated = Field("unrelated", "utf8", nullable=False)

    @dataclasses.dataclass
    class Plain:
        value: int

        @staticmethod
        def into_field() -> Field:
            return unrelated

    assert Plain.into_field() is unrelated
    assert DataType.from_pyhint(Plain)["value"].dtype.id == "int64"
    assert Field.from_pyhint("plain", Plain)["value"].dtype.id == "int64"
    assert field(Plain)["value"].dtype.id == "int64"


def test_generic_inheritance_reinfers_a_specialized_member() -> None:
    ItemT = TypeVar("ItemT")

    @scalar
    class Box(Generic[ItemT]):
        item: ItemT

    @scalar
    class IntegerBox(Box[int]):
        pass

    assert Box.into_field()["item"].dtype.id == "null"
    assert IntegerBox.into_field()["item"].dtype.id == "int64"

    LeftT = TypeVar("LeftT")
    RightT = TypeVar("RightT")

    @scalar
    class Pair(Generic[LeftT, RightT]):
        left: LeftT
        right: RightT

    @scalar
    class Swapped(Pair[RightT, LeftT], Generic[LeftT, RightT]):
        pass

    @scalar
    class Concrete(Swapped[int, str]):
        pass

    assert Concrete.into_field()["left"].dtype.id == "utf8"
    assert Concrete.into_field()["right"].dtype.id == "int64"
    assert json.loads('{"left":"x","right":"3"}', cls=Concrete) == Concrete(
        left="x",
        right=3,
    )


def test_field_cache_is_thread_safe_and_published_once() -> None:
    @scalar
    class Reading:
        value: int

    barrier = threading.Barrier(8)

    def read() -> Field:
        barrier.wait()
        return Reading.into_field()

    with ThreadPoolExecutor(max_workers=8) as pool:
        roots = tuple(pool.map(lambda _: read(), range(8)))

    assert all(root is roots[0] for root in roots)


def test_decorator_options_and_reserved_into_field_collision() -> None:
    @scalar(kw_only=True, order=True)
    class Quote:
        bid: float
        ask: float

    quote = Quote(bid=1.0, ask=2.0)
    assert quote < Quote(bid=2.0, ask=3.0)

    with pytest.raises(TypeError, match="reserves into_field"):

        @scalar
        class Invalid:
            into_field = "custom"
            value: int

    with pytest.raises(TypeError, match="reserves into_field"):

        @scalar
        class InvalidAnnotation:
            into_field: int
            value: int

    @dataclasses.dataclass
    class InheritedMember:
        into_field: int

    with pytest.raises(TypeError, match="reserves into_field"):

        @scalar
        class InvalidInheritedMember(InheritedMember):
            value: int

    @scalar
    class FieldBase:
        value: int

    class OverriddenAccessor(FieldBase):
        @staticmethod
        def into_field() -> Field:
            return Field("unrelated", DataType.int64())

    with pytest.raises(TypeError, match="reserves into_field"):

        @scalar
        class InvalidInheritedOverride(OverriddenAccessor):
            extra: int

    class HiddenAccessor(FieldBase):
        into_field = None

    with pytest.raises(TypeError, match="reserves into_field"):

        @scalar
        class InvalidHiddenOverride(HiddenAccessor):
            extra: int


def test_field_is_an_ordinary_member_name() -> None:
    """The accessor is named `into_field`, so `field` is the caller's to use."""

    @scalar
    class Row:
        field: str
        value: int

    root = Row.into_field()
    assert tuple(child.name for child in root.dtype) == ("field", "value")
    assert root.dtype["field"].dtype.id == "utf8"
    assert Row("custom", 1).field == "custom"
    assert field(Row) is root


def test_inherited_staticmethod_keeps_its_decorated_owner() -> None:
    @scalar
    class Base:
        value: int

    class Undecorated(Base):
        pass

    @scalar
    class Decorated(Base):
        extra: int

    assert "into_field" not in Undecorated.__dict__
    assert Undecorated.into_field() is Base.into_field()
    assert field(Undecorated) is Base.into_field()
    assert Decorated.into_field() is Decorated.into_field()
    assert field(Decorated) is Decorated.into_field()
    assert Decorated.into_field() is not Base.into_field()


def _schema() -> pa.Schema:
    return pa.schema(
        [
            pa.field("identifier", pa.int16(), nullable=False, metadata={"id": "7"}),
            pa.field(
                "payload",
                pa.struct(
                    [
                        pa.field("label", pa.string(), nullable=False),
                        pa.field("score", pa.float32()),
                    ]
                ),
                nullable=False,
            ),
            pa.field("tags", pa.list_(pa.field("item", pa.string(), False))),
        ],
        metadata={"source": "arrow"},
    )


def test_arrow_schema_import_is_one_native_struct_field() -> None:
    schema = _schema()
    root = Field.from_arrow_schema(schema, name="event")

    assert root.name == "event"
    assert not root.nullable
    assert root.dtype.id == "struct"
    assert root.metadata["source"] == "arrow"
    assert tuple(child.name for child in root.dtype) == (
        "identifier",
        "payload",
        "tags",
    )
    assert root.dtype["identifier"].dtype.id == "int16"
    assert root.dtype["payload"].dtype["score"].dtype.id == "float32"
    assert root.into_arrow_schema() == schema
    assert root.into_arrow_schema() == schema


def test_native_field_materializes_plain_nested_dataclasses() -> None:
    root = Field.from_arrow_schema(_schema(), name="event")
    Event = root.into_dataclass(name="Event", module=__name__)

    assert dataclasses.is_dataclass(Event)
    assert Event.__name__ == "Event"
    assert Event.__module__ == __name__
    assert isinstance(Event.__dict__["into_field"], staticmethod)
    assert Event.into_field() is root
    assert Event.into_field() is Event.into_field()

    members = dataclasses.fields(Event)
    assert tuple(member.name for member in members) == (
        "identifier",
        "payload",
        "tags",
    )
    Payload = members[1].type
    assert dataclasses.is_dataclass(Payload)
    assert Payload.into_field() == root.dtype["payload"]
    assert Payload.into_field() is Payload.into_field()

    value = Event(identifier=1, payload=Payload(label="ok"), tags=None)
    assert value.identifier == 1
    assert value.payload.label == "ok"
    assert value.payload.score is None


def test_generated_class_preserves_narrow_and_nested_native_types() -> None:
    root = Field(
        "row",
        DataType.from_fields(
            (
                Field("small", "int8", nullable=False),
                Field(
                    "nested",
                    DataType.from_fields((Field("amount", "decimal128(12, 2)"),)),
                    nullable=False,
                    metadata={"role": "payload"},
                ),
            )
        ),
        nullable=False,
        metadata={"owner": "root"},
    )
    Row = root.into_dataclass(name="Row")

    assert Row.into_field() is root
    assert Row.into_field().dtype["small"].dtype.id == "int8"
    Nested = dataclasses.fields(Row)[1].type
    assert Nested.into_field() == root.dtype["nested"]
    assert Nested.into_field() is Nested.into_field()
    assert Nested.into_field().metadata["role"] == "payload"
    assert field(Row) is root
    assert root.into_arrow_schema() == Row.into_field().into_arrow_schema()


def test_decorated_dataclass_round_trips_through_arrow_schema() -> None:
    @scalar
    class Quote:
        symbol: str
        bid: float
        ask: float | None = None

    schema = Quote.into_field().into_arrow_schema()
    imported = Field.from_arrow_schema(schema, name=Quote.into_field().name)
    Restored = imported.into_dataclass(name="Restored")

    assert imported == Quote.into_field()
    assert Restored.into_field() is imported
    assert tuple(item.name for item in dataclasses.fields(Restored)) == (
        "symbol",
        "bid",
        "ask",
    )
    assert Restored(symbol="ABC", bid=1.0).ask is None


def test_into_dataclass_requires_a_non_nullable_struct_root() -> None:
    for root in (
        Field("scalar", "int64", nullable=False),
        Field("nullable", DataType.from_fields(()), nullable=True),
    ):
        with pytest.raises((TypeError, ValueError), match="non-nullable|Struct"):
            root.into_dataclass()


@pytest.mark.parametrize("invalid", ("class", "with-dash", "__private"))
def test_invalid_python_member_names_are_refused(invalid: str) -> None:
    root = Field.from_arrow_schema(
        pa.schema([pa.field(invalid, pa.int64(), nullable=False)]),
        name="valid_root",
    )
    with pytest.raises(TypeError, match=invalid):
        root.into_dataclass()


def test_invalid_python_root_name_is_refused() -> None:
    root = Field.from_arrow_schema(
        pa.schema([pa.field("value", pa.int64(), nullable=False)]),
        name="invalid-root",
    )
    # The class name a materialized dataclass would take crosses the native
    # declaration, so the refusal is the core's and names the key it failed.
    with pytest.raises(ValueError, match="PYTHON:qualname.*invalid-root"):
        root.into_dataclass()
    with pytest.raises(ValueError, match="PYTHON:module.*not-a-module"):
        root.into_dataclass(name="Row", module="not-a-module")

    # What a declaration may be stored as and what a generated class may be
    # named are different questions. A qualified name, the `<locals>` segment
    # Python writes for a class declared in a function, and a non-identifier
    # alphanumeric are all storable and none of them is a legal class name.
    qualified = Field.from_arrow_schema(
        pa.schema([pa.field("value", pa.int64(), nullable=False)]),
        name="Outer.Inner",
    )
    with pytest.raises(TypeError, match="non-keyword Python identifier"):
        qualified.into_dataclass()
    for storable_name in ("<locals>", "\u00b2"):
        with pytest.raises(TypeError, match="non-keyword Python identifier"):
            root.into_dataclass(name=storable_name)
    with pytest.raises(TypeError, match="dotted Python identifier"):
        root.into_dataclass(name="Row", module="\u00b2x")

    # Every generated class name is one Python could have written.
    assert root.into_dataclass(name="Row", module=__name__).__name__.isidentifier()


def test_into_field_is_reserved_for_generated_classes() -> None:
    root = Field.from_arrow_schema(
        pa.schema([pa.field("into_field", pa.int64(), nullable=False)]),
        name="row",
    )

    with pytest.raises(TypeError, match="into_field"):
        root.into_dataclass()


@pytest.mark.parametrize(
    "column", ["__dict__", "__slots__", "__weakref__", "__yggdryl_class_schema__"]
)
def test_a_dunder_column_is_reserved_for_generated_classes(column: str) -> None:
    # A generated class carries its slots, its schema cache and its decoration
    # markers under dunder names, so the whole shape is refused rather than
    # each name listed.
    root = Field.from_arrow_schema(
        pa.schema([pa.field(column, pa.int64(), nullable=False)]),
        name="row",
    )

    with pytest.raises(TypeError, match="conflicts with the field-class API"):
        root.into_dataclass()


def test_field_column_materializes_for_generated_classes() -> None:
    root = Field.from_arrow_schema(
        pa.schema([pa.field("field", pa.int64(), nullable=False)]),
        name="row",
    )
    Row = root.into_dataclass(name="Row", module=__name__)

    assert Row.into_field() is root
    assert tuple(member.name for member in dataclasses.fields(Row)) == ("field",)
    assert Row(field=1).field == 1


def test_arrow_schema_round_trip_preserves_sorted_map_layout() -> None:
    schema = pa.schema(
        [
            pa.field(
                "lookup",
                pa.map_(pa.string(), pa.int32(), keys_sorted=True),
                nullable=False,
            )
        ]
    )
    root = Field.from_arrow_schema(schema, name="row")

    assert root.dtype["lookup"].dtype.into_arrow().keys_sorted
    assert root.into_arrow_schema() == schema


def test_arrow_schema_round_trip_preserves_dictionary_identity() -> None:
    dictionary = Field.from_pyhint(
        "symbol",
        Annotated[
            str,
            ("arrow_type", pa.dictionary(pa.int16(), pa.string(), ordered=True)),
            ("dictionary_id", 29),
            ("dictionary_is_ordered", True),
        ],
    )
    root = Field(
        "row",
        DataType.from_fields((dictionary,)),
        nullable=False,
    )
    restored = Field.from_arrow_schema(root.into_arrow_schema(), name="row")

    assert restored.dtype["symbol"].dictionary_id == 29
    assert restored.dtype["symbol"].dictionary_is_ordered is True


def test_arrow_schema_round_trip_rehydrates_registered_extension_identity() -> None:
    class SchemaExtension(pa.ExtensionType):
        def __init__(self) -> None:
            super().__init__(pa.int32(), "tests.field-class.schema-extension")

        def __arrow_ext_serialize__(self) -> bytes:
            return b"v1"

        @classmethod
        def __arrow_ext_deserialize__(
            cls, storage_type: pa.DataType, serialized: bytes
        ) -> SchemaExtension:
            assert storage_type == pa.int32()
            assert serialized == b"v1"
            return cls()

    extension = SchemaExtension()
    pa.register_extension_type(extension)
    try:
        schema = pa.schema(
            [pa.field("payload", extension, nullable=False)],
            metadata={"source": "extension"},
        )
        root = Field.from_arrow_schema(schema, name="row")
        exported = root.into_arrow_schema()
        restored = Field.from_arrow_schema(exported, name="row")

        assert exported.field("payload").type == extension
        assert restored.dtype["payload"].into_arrow().type == extension
        assert restored.metadata["source"] == "extension"
    finally:
        pa.unregister_extension_type(extension.extension_name)


def test_field_renames_a_pyarrow_dictionary_field_without_losing_options() -> None:
    arrow = pa.field(
        "symbol",
        pa.dictionary(pa.int16(), pa.large_string(), ordered=True),
        nullable=False,
        metadata={"source": "arrow"},
    )
    native = field(arrow, name="venue")

    assert native.name == "venue"
    assert not native.nullable
    assert native.metadata["source"] == "arrow"
    assert native.dictionary_is_ordered is True
    assert native.into_arrow().type == arrow.type


def test_field_bare_pyarrow_dictionary_type_preserves_ordering() -> None:
    arrow = pa.dictionary(pa.uint8(), pa.string(), ordered=True)
    native = field(arrow, name="category")

    assert native.name == "category"
    assert native.nullable
    assert native.dictionary_is_ordered is True
    assert native.into_arrow().type == arrow


def test_field_name_is_uniform_for_every_arrow_shape() -> None:
    schema = pa.schema([pa.field("value", pa.int64(), nullable=False)])

    assert field(schema, name="").name == ""
    assert field(pa.int64(), name="").name == ""
    for value in (schema, schema.field(0), pa.int64()):
        with pytest.raises(TypeError, match="name must be str or None"):
            field(value, name=7)  # type: ignore[arg-type]


class CountPayload(TypedDict):
    count: Annotated[int, {"branch": "count"}]


class LabelPayload(TypedDict):
    label: str


@scalar
class VariantEnvelope:
    payload: CountPayload | LabelPayload
    history: list[int | str]


@scalar
class WithDefaults:
    enabled: bool = True
    retries: int = 3
    labels: list[str] = dataclasses.field(default_factory=list)


def test_optional_is_the_only_nullable_signal() -> None:
    assert Field.from_pyhint("value", int | None).nullable
    assert not Field.from_pyhint("value", int).nullable
    assert not Field.from_pyhint("value", int | str).nullable
    assert DataType.from_pyhint(int | None) == DataType.from_pyhint(int)


def test_unions_compile_to_dense_native_tags_at_every_depth() -> None:
    root = VariantEnvelope.into_field()
    payload = root.dtype["payload"].dtype
    history = root.dtype["history"].dtype[0].dtype

    assert payload.id == history.id == "union"
    assert tuple(payload.into_arrow().type_codes) == (0, 1)
    assert tuple(history.into_arrow().type_codes) == (0, 1)
    assert payload[0].dtype["count"].metadata["branch"] == "count"

    value = json.loads(
        json.dumps({"payload": {"count": "7"}, "history": [1, "two"]}),
        cls=VariantEnvelope,
    )
    assert value.payload == {"count": 7}
    assert value.history == [1, "two"]
    assert json.loads(json.dumps(value)) == {
        "payload": {"count": 7},
        "history": [1, "two"],
    }


def test_default_policy_uses_declared_values_and_fresh_factories() -> None:
    first = json.loads(
        json.dumps({"enabled": "ambiguous", "retries": "bad"}),
        cls=WithDefaults,
        errors="default",
    )
    second = json.loads("{}", cls=WithDefaults, errors="default")

    assert (first.enabled, first.retries) == (True, 3)
    assert first.labels == second.labels == []
    assert first.labels is not second.labels


def test_deep_union_keeps_terminal_variant_tags() -> None:
    depth = 12
    deep_hint: object = int | str
    for _ in range(depth):
        deep_hint = list[deep_hint]  # type: ignore[valid-type]

    @scalar
    class DeepVariant:
        payload: deep_hint  # type: ignore[valid-type]

    dtype = DeepVariant.into_field().dtype["payload"].dtype
    raw: object = "terminal"
    for _ in range(depth):
        assert dtype.id == "list"
        dtype = dtype[0].dtype
        raw = [raw]

    assert dtype.id == "union"
    value = json.loads(json.dumps({"payload": raw}), cls=DeepVariant)
    assert json.loads(json.dumps(value)) == {"payload": raw}


def test_recursive_annotations_fail_without_unbounded_expansion() -> None:
    @dataclasses.dataclass
    class RecursiveValue:
        child: RecursiveValue | None = None

    with pytest.raises((TypeError, ValueError), match="recurs|depth|cycle"):
        field(RecursiveValue)

    @scalar
    class RecursiveField:
        child: RecursiveField | None = None

    with pytest.raises((TypeError, ValueError), match="recurs|depth|cycle"):
        RecursiveField.into_field()


@pytest.mark.skipif(sys.version_info < (3, 14), reason="PEP 649 is Python 3.14+")
def test_lazy_sibling_and_private_default_factory_without_future_annotations() -> None:
    @scalar
    class Earlier:
        later: Later
        __cache: list[int] = dc.field(default_factory=list)

    @scalar
    class Later:
        count: int

    assert tuple(item.name for item in dc.fields(Earlier)) == ("later",)
    assert tuple(item.name for item in Earlier.into_field().dtype) == (
        "later",
    )
    assert (
        Earlier.into_field().dtype["later"].dtype
        == Later.into_field().dtype
    )
    assert Earlier(Later(3)) == Earlier(later=Later(count=3))


@pytest.mark.skipif(sys.version_info < (3, 14), reason="PEP 649 is Python 3.14+")
def test_plain_private_annotation_without_future_is_not_a_dataclass_field() -> None:
    @scalar
    class Reading:
        value: int
        __scratch: str

    assert tuple(item.name for item in dc.fields(Reading)) == ("value",)
    assert tuple(item.name for item in Reading.into_field().dtype) == (
        "value",
    )
    assert Reading(7).value == 7
