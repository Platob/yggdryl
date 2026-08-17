from __future__ import annotations

import dataclasses
from typing import Annotated, TypedDict

import pytest

from yggdryl import DataType, Field
from yggdryl.records import from_dict, record, schema_field, schema_fields, to_dict


@record(frozen=True, slots=True)
class EdgeLeg:
    symbol: str
    quantity: int


@record(frozen=True, slots=True)
class EdgeOrder:
    order_id: int
    active: bool
    legs: list[EdgeLeg]
    note: str | None = None


@dataclasses.dataclass
class PlainValue:
    count: int


@record
class WithDefaults:
    enabled: bool = True
    retries: int = 3
    labels: list[str] = dataclasses.field(default_factory=list)


@dataclasses.dataclass
class RecursiveValue:
    child: RecursiveValue | None = None


class CountPayload(TypedDict):
    count: Annotated[int, {"branch": "count"}]


class LabelPayload(TypedDict):
    label: str


@record
class VariantEnvelope:
    payload: CountPayload | LabelPayload
    history: list[int | str]


def test_record_is_dataclass_and_schema_values_are_singletons() -> None:
    assert dataclasses.is_dataclass(EdgeOrder)
    root = schema_field(EdgeOrder)
    assert root is schema_field(EdgeOrder)
    assert schema_fields(EdgeOrder) is schema_fields(EdgeOrder)
    assert EdgeOrder.schema_field() is schema_field(EdgeOrder)
    assert EdgeOrder.schema_fields() is schema_fields(EdgeOrder)

    assert root.name == "EdgeOrder"
    assert root["python.module"] == __name__
    assert root["python.class"] == "EdgeOrder"
    assert root["python.qualname"] == EdgeOrder.__qualname__
    assert root["python.kind"] == "record"
    assert root.data_type.id == "struct"
    assert tuple(child.name for child in schema_fields(EdgeOrder)) == (
        "order_id",
        "active",
        "legs",
        "note",
    )

    arrow_metadata = root.to_arrow().metadata
    assert arrow_metadata[b"python.module"] == __name__.encode()
    assert arrow_metadata[b"python.class"] == b"EdgeOrder"


def test_explicit_optional_is_the_only_nullable_signal() -> None:
    assert Field.from_pyhint("value", int | None).nullable
    assert not Field.from_pyhint("value", int).nullable
    assert not Field.from_pyhint("value", int | str).nullable
    assert DataType.from_pyhint(int | None) == DataType.from_pyhint(int)
    assert schema_fields(EdgeOrder)[3].nullable
    assert not schema_fields(WithDefaults)[0].nullable


def test_annotated_metadata_and_direct_overlay_are_preserved() -> None:
    value = Field.from_pyhint(
        "count",
        Annotated[int, {"source": "annotation", "priority": "low"}],
        {"priority": "high"},
    )
    assert value["source"] == "annotation"
    assert value["priority"] == "high"


def test_record_unions_materialize_dense_native_tags_at_every_depth() -> None:
    payload = schema_fields(VariantEnvelope)[0].data_type
    history = schema_fields(VariantEnvelope)[1].data_type[0].data_type

    assert payload.id == history.id == "union"
    assert tuple(payload.to_arrow().type_codes) == (0, 1)
    assert tuple(history.to_arrow().type_codes) == (0, 1)
    assert payload[0].data_type["count"]["branch"] == "count"

    count = VariantEnvelope.from_dict(
        {"payload": {"count": "7"}, "history": [1, "two"]}
    )
    label = VariantEnvelope.from_dict(
        {"payload": {"label": "ready"}, "history": ["3", 4]}
    )

    assert count.payload == {"count": 7}
    assert count.history == [1, "two"]
    assert label.payload == {"label": "ready"}
    assert label.history == ["3", 4]
    assert count.to_dict() == {
        "payload": {"count": 7},
        "history": [1, "two"],
    }


def test_deep_record_union_keeps_the_terminal_variant_tags() -> None:
    depth = 12
    deep_hint: object = int | str
    for _ in range(depth):
        deep_hint = list[deep_hint]  # type: ignore[valid-type]

    @record
    class DeepVariant:
        payload: deep_hint  # type: ignore[valid-type]

    data_type = schema_fields(DeepVariant)[0].data_type
    raw: object = "terminal"
    for _ in range(depth):
        assert data_type.id == "list"
        data_type = data_type[0].data_type
        raw = [raw]

    assert data_type.id == "union"
    assert tuple(data_type.to_arrow().type_codes) == (0, 1)
    converted = DeepVariant.from_dict({"payload": raw})
    assert converted.to_dict() == {"payload": raw}


def test_safe_nested_conversion_and_exact_boolean_casting() -> None:
    order = EdgeOrder.from_dict(
        {
            "order_id": "42",
            "active": "false",
            "legs": [{"symbol": "ABC", "quantity": "10"}],
        }
    )
    assert order == EdgeOrder(42, False, [EdgeLeg("ABC", 10)])
    assert order.to_dict() == {
        "order_id": 42,
        "active": False,
        "legs": [{"symbol": "ABC", "quantity": 10}],
        "note": None,
    }

    for value, expected in ((True, True), (1, True), (0, False), ("yes", True), ("off", False)):
        assert from_dict(WithDefaults, {"enabled": value}).enabled is expected
    for value in (2, -1, "perhaps", object()):
        with pytest.raises((TypeError, ValueError), match="enabled"):
            from_dict(WithDefaults, {"enabled": value})


def test_default_policy_uses_declared_defaults_and_fresh_factories() -> None:
    first = WithDefaults.from_dict(
        {"enabled": "ambiguous", "retries": "bad"}, errors="default"
    )
    second = WithDefaults.from_dict({}, errors="default")
    assert (first.enabled, first.retries) == (True, 3)
    assert first.labels == second.labels == []
    assert first.labels is not second.labels

    with pytest.raises((TypeError, ValueError), match="order_id"):
        EdgeOrder.from_dict({}, errors="default")
    with pytest.raises(ValueError, match="errors"):
        EdgeOrder.from_dict({}, errors="ignore")


def test_safe_rejects_unknown_keys_while_shallow_mode_does_not_cast() -> None:
    with pytest.raises((TypeError, ValueError), match="unknown"):
        EdgeOrder.from_dict(
            {"order_id": 1, "active": True, "legs": [], "unknown": 1}
        )

    shallow = EdgeOrder.from_dict(
        {
            "order_id": "42",
            "active": "false",
            "legs": [{"symbol": "ABC", "quantity": "10"}],
        },
        safe=False,
    )
    assert shallow.order_id == "42"
    assert isinstance(shallow.legs[0], dict)


def test_plain_dataclasses_share_schema_and_conversion_implementation() -> None:
    assert schema_field(PlainValue) is schema_field(PlainValue)
    assert schema_field(PlainValue)["python.kind"] == "dataclass"
    value = from_dict(PlainValue, {"count": "7"})
    assert value == PlainValue(7)
    assert to_dict(value) == {"count": 7}


def test_recursive_annotations_fail_without_unbounded_expansion() -> None:
    with pytest.raises((TypeError, ValueError), match="recurs|depth|cycle"):
        schema_field(RecursiveValue)
