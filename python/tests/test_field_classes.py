from __future__ import annotations

import dataclasses
import inspect
import threading
from concurrent.futures import ThreadPoolExecutor
from typing import Annotated, ClassVar, Generic, TypeVar

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

    root = Order.field()
    assert isinstance(root, Field)
    assert isinstance(Order.__dict__["field"], staticmethod)
    assert Order.__dict__["field"].__func__.__name__ == "field"
    assert root is Order.field()
    assert root is value.field()
    assert root is field(Order)
    assert root is field(value)
    assert root.name == "Order"
    assert root.dtype.id == "struct"
    assert tuple(child.name for child in root.dtype) == (
        "order_id",
        "legs",
        "note",
    )
    assert root.metadata["python.kind"] == "field"
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
    assert inspect.ismodule(yggdryl.fields.scalar)
    assert callable(yggdryl.scalar)
    assert scalar.__module__ == "yggdryl.scalar"


def test_field_accessor_signatures_are_uniform_and_class_metadata_is_argument_free() -> None:
    signature = inspect.signature(field)

    assert tuple(signature.parameters) == ("value", "name")
    assert signature.parameters["name"].default is None
    assert tuple(inspect.signature(Order.field).parameters) == ()
    with pytest.raises(TypeError):
        Order.field("renamed")  # type: ignore[call-arg]


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
    assert root.metadata["python.kind"] == "dataclass"
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
        Parent.field().dtype["child"].dtype
        == Child.field().dtype
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

    assert Payload.field() is child
    assert (
        Envelope.field()
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

    assert Generated.field() is root
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

    for child in (DecoratedChild.field(), field(PlainChild)):
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
    assert DecoratedParent.field()["child"]["narrow"] == root["narrow"]
    lazy_root = LazyChild.field()
    assert lazy_root is LazyChild.field()
    assert lazy_root["narrow"] == root["narrow"]

    @dataclasses.dataclass
    class PlainChild(Base):
        extra: int

    assert DataType.from_pyhint(PlainChild)["narrow"] == root["narrow"]

    @scalar
    class PlainParent:
        child: PlainChild

    assert PlainParent.field()["child"]["narrow"] == root["narrow"]

    ItemT = TypeVar("ItemT")

    @scalar
    class GenericChild(Base, Generic[ItemT]):
        item: ItemT

    @scalar
    class GenericParent:
        child: GenericChild[str]

    nested = GenericParent.field()["child"]
    assert nested["narrow"] == root["narrow"]
    assert nested["item"].dtype.id == "utf8"


def test_plain_dataclass_field_attribute_does_not_override_annotations() -> None:
    unrelated = Field("unrelated", "utf8", nullable=False)

    @dataclasses.dataclass
    class Plain:
        value: int

        @staticmethod
        def field() -> Field:
            return unrelated

    assert Plain.field() is unrelated
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

    assert Box.field()["item"].dtype.id == "null"
    assert IntegerBox.field()["item"].dtype.id == "int64"

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

    assert Concrete.field()["left"].dtype.id == "utf8"
    assert Concrete.field()["right"].dtype.id == "int64"
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
        return Reading.field()

    with ThreadPoolExecutor(max_workers=8) as pool:
        roots = tuple(pool.map(lambda _: read(), range(8)))

    assert all(root is roots[0] for root in roots)


def test_decorator_options_and_reserved_field_collision() -> None:
    @scalar(kw_only=True, order=True)
    class Quote:
        bid: float
        ask: float

    quote = Quote(bid=1.0, ask=2.0)
    assert quote < Quote(bid=2.0, ask=3.0)

    with pytest.raises(TypeError, match="reserves field"):

        @scalar
        class Invalid:
            field = "custom"
            value: int

    with pytest.raises(TypeError, match="reserves field"):

        @scalar
        class InvalidAnnotation:
            field: int
            value: int

    @dataclasses.dataclass
    class InheritedMember:
        field: int

    with pytest.raises(TypeError, match="reserves field"):

        @scalar
        class InvalidInheritedMember(InheritedMember):
            value: int

    @scalar
    class FieldBase:
        value: int

    class OverriddenAccessor(FieldBase):
        @staticmethod
        def field() -> Field:
            return Field("unrelated", DataType.int64())

    with pytest.raises(TypeError, match="reserves field"):

        @scalar
        class InvalidInheritedOverride(OverriddenAccessor):
            extra: int

    class HiddenAccessor(FieldBase):
        field = None

    with pytest.raises(TypeError, match="reserves field"):

        @scalar
        class InvalidHiddenOverride(HiddenAccessor):
            extra: int


def test_inherited_staticmethod_keeps_its_decorated_owner() -> None:
    @scalar
    class Base:
        value: int

    class Undecorated(Base):
        pass

    @scalar
    class Decorated(Base):
        extra: int

    assert "field" not in Undecorated.__dict__
    assert Undecorated.field() is Base.field()
    assert field(Undecorated) is Base.field()
    assert Decorated.field() is Decorated.field()
    assert field(Decorated) is Decorated.field()
    assert Decorated.field() is not Base.field()
