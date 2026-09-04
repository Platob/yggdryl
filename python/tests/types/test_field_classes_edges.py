from __future__ import annotations

import dataclasses
from typing import Annotated, TypedDict

import pytest

from yggdryl import DataType, Field, field, scalar
from yggdryl.text import json


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
    root = VariantEnvelope.field()
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

    dtype = DeepVariant.field().dtype["payload"].dtype
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
        RecursiveField.field()
