from __future__ import annotations

import collections
import dataclasses
import datetime as dt
import enum
import gc
import pathlib
import sys
import typing
import uuid
import weakref
from decimal import Decimal
from typing import Generic, NamedTuple, TypeVar, TypedDict

import pytest

from yggdryl import Record
from yggdryl.records import from_dict, record, schema_field, schema_fields, to_dict


class Side(enum.Enum):
    BUY = "buy"
    SELL = "sell"


T = TypeVar("T")


@record
class Box(Record, Generic[T]):
    value: T


@record
class IntBox(Box[int]):
    pass


class Pair(NamedTuple, Generic[T]):
    left: T
    right: T


@record
class GenericHolder:
    box: Box[int]
    pair: Pair[int]


@record(frozen=True, slots=True)
class RichValue(Record):
    enabled: bool
    identity: int | str
    pair: tuple[int, str]
    labels: set[str]
    weights: dict[str, int]
    amount: Decimal
    identifier: uuid.UUID
    day: dt.date
    location: pathlib.Path
    side: Side


def test_configured_record_is_real_dataclass_and_typed_mixin_works() -> None:
    value = RichValue.from_dict(
        {
            "enabled": " ON ",
            "identity": "42",
            "pair": ["7", "x"],
            "labels": ["a", "b"],
            "weights": {"a": "2"},
            "amount": "12.50",
            "identifier": "12345678-1234-5678-1234-567812345678",
            "day": "2026-08-14",
            "location": "data/orders",
            "side": "BUY",
        }
    )

    assert dataclasses.is_dataclass(RichValue)
    assert value.enabled is True
    assert value.identity == "42"  # an exact Union member is not eagerly recast
    exported = value.to_dict()
    assert exported["amount"] is value.amount
    assert exported["identifier"] is value.identifier
    assert exported["day"] is value.day
    assert exported["location"] is value.location
    assert exported["side"] is value.side


def test_parameterized_and_inherited_generics_drive_safe_conversion() -> None:
    inherited = IntBox.from_dict({"value": "7"})
    holder = GenericHolder.from_dict(
        {"box": {"value": "8"}, "pair": {"left": "9", "right": 10}}
    )

    assert inherited == IntBox(7)
    assert holder == GenericHolder(Box(8), Pair(9, 10))
    assert schema_fields(IntBox)[0].data_type.id == "int64"

    unchecked = GenericHolder(Box("11"), Pair("12", "13"))
    assert unchecked.to_dict() == {
        "box": {"value": 11},
        "pair": Pair(12, 13),
    }


def test_local_forward_annotations_and_inheritance_resolve_once() -> None:
    @record
    class LocalChild:
        count: int

    @record
    class LocalBase:
        name: str

    @record
    class LocalParent(LocalBase):
        child: LocalChild

    value = LocalParent.from_dict({"name": "nested", "child": {"count": "4"}})
    assert value == LocalParent("nested", LocalChild(4))
    assert tuple(field.name for field in LocalParent.schema_fields()) == ("name", "child")
    assert LocalParent.schema_field() is LocalParent.__yggdryl_field__
    assert LocalParent.schema_fields() is LocalParent.__yggdryl_fields__


def test_later_local_sibling_annotation_is_deferred_then_resolved() -> None:
    @record
    class Earlier:
        later: Later

    assert "__yggdryl_field__" not in Earlier.__dict__

    @record
    class Later:
        count: int

    value = Earlier.from_dict({"later": {"count": "6"}})
    assert value == Earlier(Later(6))
    assert Earlier.schema_field() is Earlier.__yggdryl_field__


def test_explicitly_quoted_local_alias_survives_deferred_resolution() -> None:
    Alias = int

    @record
    class Earlier:
        value: "'Alias'"
        later: Later

    @record
    class Later:
        count: int

    converted = Earlier.from_dict({"value": "3", "later": {"count": "4"}})
    assert converted.value == 3
    assert converted.later == Later(4)


def test_nested_quoted_child_survives_deferred_resolution() -> None:
    @dataclasses.dataclass
    class Child:
        value: int

    @record
    class Earlier:
        children: list["Child"]
        later: Later

    @record
    class Later:
        count: int

    converted = Earlier.from_dict(
        {"children": [{"value": "2"}], "later": {"count": "3"}}
    )
    assert converted.children == [Child(2)]
    assert converted.later == Later(3)


def test_unresolved_local_record_does_not_leak_through_pending_namespace() -> None:
    def reference() -> weakref.ReferenceType[type[object]]:
        @record
        class Pending:
            missing: NeverDeclared

        return weakref.ref(Pending)

    pending = reference()
    gc.collect()
    assert pending() is None


def test_retained_pending_record_does_not_retain_unrelated_locals() -> None:
    class Payload:
        pass

    def factory() -> tuple[type[object], weakref.ReferenceType[Payload]]:
        unrelated = Payload()

        @record
        class Pending:
            missing: NeverDeclared

        return Pending, weakref.ref(unrelated)

    pending, unrelated = factory()
    gc.collect()
    assert unrelated() is None
    with pytest.raises(TypeError, match="NeverDeclared"):
        pending.schema_field()


def test_pending_generic_alias_back_reference_is_collectable() -> None:
    def reference() -> weakref.ReferenceType[type[object]]:
        class Candidate:
            missing: Missing

        alias = list[Candidate]
        Candidate = record(Candidate)
        assert alias is not None
        return weakref.ref(Candidate)

    candidate = reference()
    gc.collect()
    assert candidate() is None


def test_reciprocal_pending_namespaces_do_not_retain_record_classes() -> None:
    def references() -> tuple[weakref.ReferenceType[type[object]], ...]:
        @record
        class First:
            value: Missing

        @record
        class Second:
            value: Missing

        return weakref.ref(First), weakref.ref(Second)

    first, second = references()
    gc.collect()
    assert first() is second() is None


def test_pending_siblings_never_cross_resolve_factory_invocations() -> None:
    def factory(include_sibling: bool) -> type[object]:
        @record
        class First:
            second: Second

        if include_sibling:
            @record
            class Second:
                value: int

        return First

    unresolved = factory(False)
    resolved = factory(True)

    assert resolved.from_dict({"second": {"value": "5"}}).second.value == 5
    with pytest.raises(TypeError, match="Second"):
        unresolved.schema_field()


def test_passed_pending_record_cannot_join_a_later_factory_scope() -> None:
    def factory(old: object | None = None, complete: bool = False) -> type[object]:
        assert old is old

        @record
        class First:
            second: Second

        if complete:
            @record
            class Second:
                value: int

        return First

    unresolved = factory()
    resolved = factory(unresolved, True)
    assert resolved.from_dict({"second": {"value": "5"}}).second.value == 5
    with pytest.raises(TypeError, match="Second"):
        unresolved.schema_field()


def test_configured_decorator_captures_scope_when_applied() -> None:
    decorate = record(frozen=True)

    @dataclasses.dataclass(frozen=True)
    class Child:
        value: int

    @decorate
    class Parent:
        child: Child

    assert Parent.from_dict({"child": {"value": "4"}}) == Parent(Child(4))


def test_records_nested_in_class_share_parent_scope_without_marker_attribute() -> None:
    Alias = int

    class Namespace:
        @record
        class Earlier:
            value: Alias
            later: Later

        @record
        class Later:
            count: int

    converted = Namespace.Earlier.from_dict(
        {"value": "2", "later": {"count": "3"}}
    )
    assert converted.value == 2
    assert converted.later == Namespace.Later(3)
    assert "__yggdryl_record_invocation_token__" not in Namespace.__dict__


def test_resolved_self_hints_do_not_leak_through_a_global_schema_map() -> None:
    def reference() -> weakref.ReferenceType[type[object]]:
        @record
        class Local:
            kind: type[Local]

        return weakref.ref(Local)

    local = reference()
    gc.collect()
    assert local() is None


def test_resolved_schema_does_not_retain_unrelated_function_locals() -> None:
    class Payload:
        pass

    def factory() -> tuple[type[object], weakref.ReferenceType[Payload]]:
        unrelated = Payload()

        @record
        class Local:
            value: int

        return Local, weakref.ref(unrelated)

    local, unrelated = factory()
    gc.collect()
    assert unrelated() is None
    assert local.from_dict({"value": "3"}).value == 3


def test_defaults_are_fresh_and_invalid_values_only_fall_back_on_request() -> None:
    @record
    class Defaults:
        enabled: bool = True
        attempts: int = 3
        labels: list[str] = dataclasses.field(default_factory=list)

    first = Defaults.from_dict({})
    second = Defaults.from_dict({})
    assert first.labels == second.labels == []
    assert first.labels is not second.labels

    with pytest.raises(TypeError, match="attempts"):
        Defaults.from_dict({"attempts": "bad"})
    fallback = Defaults.from_dict({"enabled": "bad", "attempts": "bad"}, errors="default")
    assert (fallback.enabled, fallback.attempts) == (True, 3)


def test_timedelta_overflow_uses_the_selected_error_policy() -> None:
    @record
    class Timeout:
        duration: dt.timedelta = dt.timedelta(seconds=5)

    with pytest.raises(TypeError, match="duration"):
        Timeout.from_dict({"duration": float("inf")})
    assert Timeout.from_dict(
        {"duration": float("inf")}, errors="default"
    ).duration == dt.timedelta(seconds=5)


def test_float_overflow_uses_the_selected_error_policy() -> None:
    @record
    class Price:
        value: float = 1.5

    enormous = 10**10_000
    with pytest.raises(TypeError, match="value"):
        Price.from_dict({"value": enormous})
    assert Price.from_dict({"value": enormous}, errors="default").value == 1.5


def test_decimal_to_int_is_lossless_and_bool_is_never_an_integer() -> None:
    @record
    class Count:
        value: int

    assert Count.from_dict({"value": Decimal("4.000")}).value == 4
    for invalid in (Decimal("4.5"), Decimal("NaN"), True):
        with pytest.raises(TypeError, match="value"):
            Count.from_dict({"value": invalid})


def test_safe_collections_reject_mapping_as_sequence_and_shallow_is_shallow() -> None:
    @record
    class Container:
        values: list[int]

    with pytest.raises(TypeError, match="values"):
        Container.from_dict({"values": {"1": "2"}})
    shallow = Container.from_dict({"values": {"1": "2"}}, safe=False)
    assert shallow.values == {"1": "2"}


def test_initvar_is_constructor_input_but_not_arrow_or_dict_field() -> None:
    @record
    class Scaled:
        value: int
        scale: dataclasses.InitVar[int]
        result: int = dataclasses.field(init=False)

        def __post_init__(self, scale: int) -> None:
            self.result = self.value * scale

    value = Scaled.from_dict({"value": "3", "scale": "4"})
    assert value.result == 12
    assert to_dict(value) == {"value": 3, "result": 12}
    assert tuple(field.name for field in schema_fields(Scaled)) == ("value", "result")


def test_ordinary_dataclass_cache_and_record_custom_methods_are_preserved() -> None:
    @dataclasses.dataclass
    class Plain:
        value: int

    assert schema_field(Plain) is schema_field(Plain)
    assert schema_field(Plain) is Plain.__yggdryl_field__
    assert schema_fields(Plain) is Plain.__yggdryl_fields__
    assert from_dict(Plain, {"value": "8"}) == Plain(8)

    @record
    class Custom:
        value: int

        @classmethod
        def schema_field(cls) -> str:
            return "custom"

    assert Custom.schema_field() == "custom"
    assert schema_field(Custom) is Custom.__yggdryl_field__


def test_schema_cache_is_class_local_and_native_fields_are_frozen() -> None:
    @record
    class Base:
        base: int

    @dataclasses.dataclass
    class Child(Base):
        child: str

    @record
    class Container:
        child: Child

    assert schema_field(Child) is not schema_field(Base)
    assert schema_field(Child)["python.kind"] == "dataclass"
    assert schema_fields(Container)[0]["python.kind"] == "dataclass"
    assert tuple(field.name for field in schema_fields(Child)) == ("base", "child")

    root = schema_field(Child)
    child = schema_fields(Child)[0]
    arrow_before = root.to_arrow()
    with pytest.raises(TypeError, match="frozen"):
        child["probe"] = "yes"
    with pytest.raises(TypeError, match="frozen"):
        root["probe"] = "yes"
    assert root.to_arrow() == arrow_before


def test_adopting_an_inferred_dataclass_rebuilds_record_identity() -> None:
    @dataclasses.dataclass
    class Existing:
        value: int

    old_root = schema_field(Existing)
    assert old_root["python.kind"] == "dataclass"

    adopted = record(Existing)
    new_root = schema_field(adopted)
    assert adopted is Existing
    assert new_root is not old_root
    assert new_root["python.kind"] == "record"


@pytest.mark.skipif(sys.version_info < (3, 12), reason="PEP 695 requires Python 3.12")
def test_parameterized_pep695_alias_is_expanded_for_conversion() -> None:
    namespace: dict[str, object] = {}
    exec("type Vec[T] = list[T]", namespace)
    Vec = namespace["Vec"]

    @record
    class Aliased:
        values: Vec[int]  # type: ignore[index,valid-type]

    assert Aliased.from_dict({"values": ["1", 2]}).values == [1, 2]


def test_readonly_typed_dict_wrapper_is_transparent_to_conversion() -> None:
    ReadOnly = getattr(typing, "ReadOnly", None)
    if ReadOnly is None:
        from typing_extensions import ReadOnly as ReadOnlyExtension

        ReadOnly = ReadOnlyExtension

    class Payload(TypedDict):
        value: ReadOnly[int]

    @record
    class Envelope:
        payload: Payload

    assert Envelope.from_dict({"payload": {"value": "7"}}).payload == {"value": 7}


def test_nested_local_struct_annotations_reuse_captured_namespace() -> None:
    @record
    class Child:
        value: int

    class Payload(TypedDict):
        child: Child

    class PayloadTuple(NamedTuple):
        child: Child

    @record
    class Envelope:
        payload: Payload
        pair: PayloadTuple

    value = Envelope.from_dict(
        {
            "payload": {"child": {"value": "3"}},
            "pair": {"child": {"value": "4"}},
        }
    )
    assert value.payload["child"] == Child(3)
    assert value.pair == PayloadTuple(Child(4))


def test_nested_ordinary_local_dataclass_reuses_resolved_hint_cache() -> None:
    Value = int

    @dataclasses.dataclass
    class Child:
        value: Value

    @record
    class Envelope:
        child: Child

    converted = Envelope.from_dict({"child": {"value": "6"}})
    assert converted.child == Child(6)
    assert schema_fields(Child)[0].data_type.id == "int64"


def test_nested_inherited_generic_dataclass_binds_type_variables() -> None:
    ValueT = TypeVar("ValueT")

    @dataclasses.dataclass
    class GenericValue(Generic[ValueT]):
        value: ValueT

    @dataclasses.dataclass
    class IntegerValue(GenericValue[int]):
        pass

    @record
    class Envelope:
        value: IntegerValue

    converted = Envelope.from_dict({"value": {"value": "7"}})
    assert converted.value == IntegerValue(7)
    assert schema_fields(IntegerValue)[0].data_type.id == "int64"


def test_inherited_binding_keeps_deep_local_dependency_cache() -> None:
    LocalInteger = int

    @dataclasses.dataclass
    class Deep:
        value: LocalInteger

    ValueT = TypeVar("ValueT")

    @dataclasses.dataclass
    class GenericValue(Generic[ValueT]):
        value: ValueT

    @dataclasses.dataclass
    class DeepValue(GenericValue[Deep]):
        pass

    @record
    class Envelope:
        value: DeepValue

    converted = Envelope.from_dict(
        {"value": {"value": {"value": "11"}}}
    )
    assert converted.value == DeepValue(Deep(11))


def test_parameterized_record_keeps_explicit_deep_conversion_context() -> None:
    LocalInteger = int

    @dataclasses.dataclass
    class Deep:
        value: LocalInteger

    @record
    class Envelope:
        value: Box[Deep]

    converted = Envelope.from_dict(
        {"value": {"value": {"value": "12"}}}
    )
    assert converted.value == Box(Deep(12))

    direct = Envelope(Box(Deep("13")))  # type: ignore[arg-type]
    assert direct.to_dict() == {"value": {"value": {"value": 13}}}


def test_parameterized_dataclass_output_never_reconstructs_the_instance() -> None:
    ValueT = TypeVar("ValueT")
    post_init_values: list[object] = []

    @dataclasses.dataclass
    class Tracked(Generic[ValueT]):
        value: ValueT
        derived: str = dataclasses.field(init=False)

        def __post_init__(self) -> None:
            post_init_values.append(self.value)
            self.derived = f"derived:{self.value}"

    @record
    class Envelope:
        tracked: Tracked[int]
        nested: list[Tracked[int]]

    tracked = Tracked("7")  # type: ignore[arg-type]
    nested = Tracked("8")  # type: ignore[arg-type]
    assert post_init_values == ["7", "8"]

    assert Envelope(tracked, [nested]).to_dict() == {
        "tracked": {"value": 7, "derived": "derived:7"},
        "nested": [{"value": 8, "derived": "derived:8"}],
    }
    assert post_init_values == ["7", "8"]
    assert tracked.value == "7"


def test_safe_output_preserves_the_successful_structural_union_hint() -> None:
    class BoxPayload(TypedDict):
        box: Box[int]

    class OtherPayload(TypedDict):
        other: str

    @record
    class Envelope:
        payload: BoxPayload | OtherPayload
        values: dict[str, int] | dict[str, Box[int]]

    value = Envelope(
        payload={"box": Box("3")},  # type: ignore[typeddict-item]
        values={"item": Box("4")},  # type: ignore[dict-item]
    )
    assert value.to_dict() == {
        "payload": {"box": {"value": 3}},
        "values": {"item": {"value": 4}},
    }


def test_concrete_generic_typed_dict_and_named_tuple_bind_members() -> None:
    ValueT = TypeVar("ValueT")

    class GenericPayload(TypedDict, Generic[ValueT]):
        value: ValueT

    class IntegerPayload(GenericPayload[int]):
        pass

    class GenericPair(NamedTuple, Generic[ValueT]):
        value: ValueT

    class IntegerPair(GenericPair[int]):
        pass

    @record
    class Envelope:
        payload: IntegerPayload
        pair: IntegerPair

    converted = Envelope.from_dict(
        {"payload": {"value": "5"}, "pair": ["6"]}
    )
    assert converted.payload == {"value": 5}
    assert converted.pair == IntegerPair(6)
    children = schema_fields(Envelope)
    assert children[0].data_type["value"].data_type.id == "int64"
    assert children[1].data_type["value"].data_type.id == "int64"


def test_deep_newtype_dependency_keeps_its_resolved_subtree() -> None:
    LocalInteger = int

    @dataclasses.dataclass
    class Leaf:
        value: LocalInteger

    WrappedLeaf = typing.NewType("WrappedLeaf", Leaf)

    @dataclasses.dataclass
    class Branch:
        leaf: WrappedLeaf

    @record
    class Tree:
        branch: Branch

    converted = Tree.from_dict({"branch": {"leaf": {"value": "9"}}})
    assert converted.branch.leaf == Leaf(9)


def test_class_local_annotation_alias_overrides_captured_outer_alias() -> None:
    X = str

    @record
    class LocalAlias:
        X = int
        value: X

    assert LocalAlias.from_dict({"value": "7"}).value == 7
    assert schema_fields(LocalAlias)[0].data_type.id == "int64"


def test_mapping_subclasses_and_counter_value_types_convert() -> None:
    @record
    class Mappings:
        chain: collections.ChainMap[str, int]
        counts: collections.Counter[str]

    value = Mappings.from_dict(
        {"chain": {"a": "2"}, "counts": {"filled": "3"}}
    )
    assert isinstance(value.chain, collections.ChainMap)
    assert value.chain["a"] == 2
    assert value.counts == collections.Counter(filled=3)

    @record
    class BareCounter:
        counts: collections.Counter

    bare = BareCounter.from_dict({"counts": {"filled": "4"}})
    assert bare.counts == collections.Counter(filled=4)


def test_safe_output_reuses_local_nested_annotation_context() -> None:
    Value = int

    @dataclasses.dataclass
    class Child:
        value: Value

    @record
    class Envelope:
        child: Child

    value = Envelope(Child("8"))  # type: ignore[arg-type]
    assert value.to_dict() == {"child": {"value": 8}}


def test_nested_schema_cache_keeps_only_reachable_classes() -> None:
    def factory() -> tuple[
        type[object],
        weakref.ReferenceType[type[object]],
        weakref.ReferenceType[type[object]],
    ]:
        @dataclasses.dataclass
        class Left:
            value: int

        @dataclasses.dataclass
        class Right:
            value: int

        @record
        class Pair:
            left: Left
            right: Right

        Pair.from_dict(
            {"left": {"value": 1}, "right": {"value": 2}}
        )
        schema_field(Left)
        return Left, weakref.ref(Right), weakref.ref(Pair)

    left, right, pair = factory()
    gc.collect()
    assert schema_field(left)["python.class"] == "Left"
    assert right() is None
    assert pair() is None


def test_to_dict_checks_and_casts_without_mutating_the_instance() -> None:
    @record
    class Mutable:
        count: int

    value = Mutable(1)
    value.count = "9"  # type: ignore[assignment]
    assert value.to_dict() == {"count": 9}
    assert value.count == "9"
    assert value.to_dict(safe=False) == {"count": "9"}


def test_invalid_options_unknown_fields_and_required_defaults_are_explicit() -> None:
    @record
    class Required:
        value: int

    with pytest.raises(TypeError, match="missing"):
        Required.from_dict({}, errors="default")
    with pytest.raises(TypeError, match="unknown"):
        Required.from_dict({"value": 1, "extra": 2}, errors="default")
    with pytest.raises(ValueError, match="errors"):
        Required.from_dict({"value": 1}, errors="coerce")
    with pytest.raises(TypeError, match="safe"):
        Required.from_dict({"value": 1}, safe=1)  # type: ignore[arg-type]


def test_nested_unknown_keys_are_never_hidden_by_default_fallback() -> None:
    @record
    class Inner:
        value: int = 1

    @record
    class Outer:
        inner: Inner = dataclasses.field(default_factory=Inner)

    with pytest.raises(TypeError, match="unknown"):
        Outer.from_dict({"inner": {"unknown": 1}}, errors="default")


def test_union_unknown_precedence_respects_structural_branch_matches() -> None:
    class A(TypedDict):
        x: int

    class B(TypedDict):
        y: int

    @record
    class Choice:
        value: A | B = dataclasses.field(default_factory=lambda: {"x": 1})

    assert Choice.from_dict(
        {"value": {"x": "bad"}}, errors="default"
    ).value == {"x": 1}
    with pytest.raises(TypeError, match="unknown"):
        Choice.from_dict({"value": {"x": 1, "extra": 2}}, errors="default")


def test_to_dict_drop_nulls_omits_only_top_level_none_values() -> None:
    """``drop_nulls`` trims optional keys without reshaping nested records."""

    @record
    class Address:
        city: str | None
        zip_code: str | None

    @record
    class Person:
        name: str
        nickname: str | None
        address: Address | None

    person = Person(
        name="Ada",
        nickname=None,
        address=Address(city="London", zip_code=None),
    )

    # Default keeps every declared key, including the null ones.
    complete = person.to_dict()
    assert complete == {
        "name": "Ada",
        "nickname": None,
        "address": {"city": "London", "zip_code": None},
    }

    # Opting in drops the top-level null. The nested record keeps its own
    # nulls, because dropping them there would change its declared shape.
    trimmed = person.to_dict(drop_nulls=True)
    assert trimmed == {
        "name": "Ada",
        "address": {"city": "London", "zip_code": None},
    }
    assert "nickname" not in trimmed

    # A null nested record is itself dropped at the top level.
    solo = Person(name="Ada", nickname=None, address=None)
    assert solo.to_dict(drop_nulls=True) == {"name": "Ada"}

    # The free function mirrors the method.
    assert to_dict(person, drop_nulls=True) == trimmed


def test_to_dict_drop_nulls_applies_without_safe_conversion() -> None:
    @record
    class Row:
        id: int
        note: str | None

    row = Row(id=1, note=None)
    assert row.to_dict(safe=False) == {"id": 1, "note": None}
    assert row.to_dict(safe=False, drop_nulls=True) == {"id": 1}


def test_to_dict_drop_nulls_rejects_a_non_boolean() -> None:
    @record
    class Row:
        id: int

    with pytest.raises(TypeError, match="drop_nulls must be bool, got str"):
        Row(id=1).to_dict(drop_nulls="yes")  # type: ignore[arg-type]
