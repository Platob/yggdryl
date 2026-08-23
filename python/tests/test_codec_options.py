"""Shared structured-text formatting and parser-limit boundary tests."""

from __future__ import annotations

import io
from collections.abc import Callable

import pytest

from yggdryl import Field, Value, json, toml, yaml
from yggdryl import _codec


@pytest.mark.parametrize("codec", [json, yaml, toml])
def test_formatting_is_core_routed_for_bytes_utf8_and_writers(codec: object) -> None:
    value = {"outer": {"answer": 42}}
    dumps = getattr(codec, "dumps")
    dump = getattr(codec, "dump")
    loads = getattr(codec, "loads")

    default = dumps(value)
    compact = dumps(value, indent=None)
    spaced = dumps(value, indent=4)
    tabbed = dumps(value, indent="\t")

    for encoded in (default, compact, spaced, tabbed):
        assert loads(encoded) == value
    assert isinstance(dump(value, utf8=True, indent=2), str)

    destination = io.BytesIO()
    assert dump(value, destination, indent=2) is None
    assert destination.getvalue() == dumps(value, indent=2)


@pytest.mark.parametrize("codec", [json, yaml, toml])
@pytest.mark.parametrize("indent", [True, -1, 256, "spaces"])
def test_formatting_rejects_values_the_core_indent_cannot_represent(
    codec: object, indent: object
) -> None:
    with pytest.raises((TypeError, ValueError), match="indent"):
        getattr(codec, "dumps")({"answer": 42}, indent=indent)


@pytest.mark.parametrize(
    ("loads", "document"),
    [
        (json.loads, b'{"outer":{"answer":42}}'),
        (yaml.loads, b"outer:\n  answer: 42\n"),
        (toml.loads, b"[outer]\nanswer = 42\n"),
    ],
)
def test_nullable_parser_limits_are_uniform(
    loads: Callable[..., object], document: bytes
) -> None:
    assert loads(
        document,
        max_depth=None,
        max_input_bytes=None,
        max_nodes=None,
        max_documents=None,
    ) == {"outer": {"answer": 42}}

    with pytest.raises(ValueError, match="depth|nesting|limit"):
        loads(document, max_depth=1)
    with pytest.raises(ValueError, match="input|byte|limit"):
        loads(document, max_input_bytes=2)
    with pytest.raises(ValueError, match="node|limit"):
        loads(document, max_nodes=1)
    with pytest.raises(ValueError, match="document|limit"):
        loads(document, max_documents=0)


def test_stream_limits_and_field_casting_stay_in_the_core() -> None:
    rows = b'{"id":1}\n{"id":2}\n'
    field = Field("row", "struct<id: int16>", nullable=False)

    exact = list(
        json.load_all(
            io.BytesIO(rows),
            cls=Value,
            field=field,
            max_depth=4,
            max_input_bytes=len(rows),
            max_nodes=4,
            max_documents=2,
        )
    )
    # A Struct Field canonicalizes named records into its ordered row shape.
    assert [row.at(0).kind for row in exact] == ["i16", "i16"]  # type: ignore[union-attr]

    with pytest.raises(ValueError, match="document|limit"):
        list(json.load_all(io.BytesIO(rows), max_documents=1))
    with pytest.raises(ValueError, match="input|byte|limit"):
        list(json.load_all(io.BytesIO(rows), max_input_bytes=len(rows) - 1))


def test_inferred_decode_uses_one_core_parse_with_limits_and_field() -> None:
    field = Field("value", "int16", nullable=False)
    value = _codec._decode_inferred(
        b"42",
        cls=Value,
        field=field,
        max_depth=1,
        max_input_bytes=2,
        max_nodes=1,
        max_documents=1,
    )
    assert value.kind == "i16"


@pytest.mark.parametrize(
    ("data_type", "kind"),
    [
        ("int8", "i8"),
        ("int16", "i16"),
        ("int32", "i32"),
        ("int64", "i64"),
        ("uint8", "u8"),
        ("uint16", "u16"),
        ("uint32", "u32"),
        ("uint64", "u64"),
    ],
)
def test_field_directed_decode_preserves_every_integer_width(
    data_type: str, kind: str
) -> None:
    value = json.loads(
        "7",
        cls=Value,
        field=Field("value", data_type, nullable=False),
    )
    assert value.kind == kind
