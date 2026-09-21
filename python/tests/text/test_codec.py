"""The structured-codec facade: its fields, its options, and what it returns."""

from __future__ import annotations

import datetime as dt
import io
import pathlib
from collections.abc import Callable
from decimal import Decimal

import pytest

from yggdryl import Field, IOBase, Scalar, json, scalar, toml, yaml
from yggdryl.text import _codec, codec

def test_from_io_infers_content_once_and_keeps_exact_field_casting() -> None:
    exact = codec.from_io(
        b'{"id":7}',
        cls=Scalar,
        field=Field("row", "struct<id: int16>", nullable=False),
        max_depth=2,
        max_input_bytes=8,
        max_nodes=3,
        max_documents=1,
    )
    assert isinstance(exact, Scalar)
    assert exact.at(0) is not None and exact.at(0).kind == "i16"


def test_from_io_and_into_io_infer_declared_path_suffixes(
    tmp_path: pathlib.Path,
) -> None:
    target = tmp_path / "value.yaml"
    assert codec.into_io({"answer": 42}, target, indent=2) is None
    assert codec.from_io(target) == {"answer": 42}

    with pytest.raises(ValueError, match="contradicts"):
        codec.into_io({"answer": 42}, target, format="toml")
    with pytest.raises(ValueError, match="contradicts"):
        codec.from_io(target, format="json")


def test_into_io_defaults_anonymous_output_to_json() -> None:
    assert codec.into_io({"answer": 42}) == b'{"answer":42}'
    assert codec.into_io({"answer": 42}, utf8=True, indent=2) == (
        '{\n  "answer": 42\n}'
    )

    destination = io.BytesIO()
    codec.into_stream({"answer": 42}, destination)
    assert destination.getvalue() == b'{"answer":42}'


def test_explicit_json_lines_redirects_to_collection_and_lazy_stream_paths() -> None:
    rows = ({"id": 1}, {"id": 2})
    encoded = codec.into_io(rows, format="json_lines")
    assert encoded == b'{"id":1}\n{"id":2}\n'
    assert list(codec.from_io(encoded, format="json_lines")) == list(rows)
    assert list(
        codec.from_stream(
            io.BytesIO(encoded), format="json_lines", max_documents=2
        )
    ) == list(rows)

    destination = io.BytesIO()
    codec.into_stream(rows, destination, format="json_lines")
    assert destination.getvalue() == encoded


def test_stream_yaml_is_lazy_multi_document_and_single_formats_are_values() -> None:
    yaml_documents = b"id: 1\n---\nid: 2\n"
    assert list(codec.from_stream(io.BytesIO(yaml_documents), format="yaml")) == [
        {"id": 1},
        {"id": 2},
    ]
    assert codec.from_stream(io.BytesIO(b'{"id":1}'), format="json") == {
        "id": 1
    }


def test_anonymous_stream_inference_is_one_document_and_bounded() -> None:
    assert codec.from_io(io.BytesIO(b"answer = 42\n")) == {"answer": 42}
    with pytest.raises(ValueError, match="exceeds|limit"):
        codec.from_io(io.BytesIO(b'{"answer":42}'), max_input_bytes=2)


@scalar
class Trade:
    amount: Decimal
    payload: bytes
    at: dt.datetime


def amount_field() -> Field:
    return Field("amount", "decimal128(8, 2)", nullable=False)


def test_explicit_field_restores_natural_scalars() -> None:
    field = amount_field()

    assert json.loads('"12.50"', field=field) == Decimal("12.50")
    assert yaml.loads('"12.50"\n', field=field) == Decimal("12.50")
    assert list(json.loads_all('"12.50"\n"3.40"\n', field=field)) == [
        Decimal("12.50"),
        Decimal("3.40"),
    ]
    assert list(
        json.load_all(io.BytesIO(b'"12.50"\n"3.40"\n'), field=field)
    ) == [Decimal("12.50"), Decimal("3.40")]


@pytest.mark.parametrize(
    ("codec", "source"),
    [
        (
            json,
            '{"amount":"12.500000000000000000","payload":"AP8=",'
            '"at":"2026-08-15T10:30:00.000000Z"}',
        ),
        (
            yaml,
            "amount: '12.500000000000000000'\npayload: AP8=\n"
            "at: '2026-08-15T10:30:00.000000Z'\n",
        ),
        (
            toml,
            "amount = '12.500000000000000000'\npayload = 'AP8='\n"
            "at = '2026-08-15T10:30:00.000000Z'\n",
        ),
    ],
)
def test_dataclass_target_and_explicit_field_share_one_decode(codec: object, source: str) -> None:
    expected = Trade(
        Decimal("12.500000000000000000"),
        b"\x00\xff",
        dt.datetime(2026, 8, 15, 10, 30, tzinfo=dt.timezone.utc),
    )

    assert codec.loads(source, cls=Trade, field=Trade) == expected  # type: ignore[attr-defined]


def test_placeholders_resolve_before_field_interpretation() -> None:
    value = yaml.loads(
        '"{{ AMOUNT }}"\n',
        field=amount_field(),
        placeholders={"AMOUNT": "12.50"},
    )

    assert value == Decimal("12.50")


@pytest.mark.parametrize("codec", [json, yaml, toml])
def test_dump_returns_bytes_or_utf8_and_still_writes(codec: object) -> None:
    binary = codec.dump({"id": 1})  # type: ignore[attr-defined]
    text = codec.dump({"id": 1}, utf8=True)  # type: ignore[attr-defined]
    destination = io.StringIO()
    written = codec.dump({"id": 1}, destination)  # type: ignore[attr-defined]

    assert isinstance(binary, bytes)
    assert isinstance(text, str)
    assert binary.decode() == text
    assert written is None
    assert destination.getvalue() == text


def test_wrong_field_is_reported_by_the_core() -> None:
    with pytest.raises(ValueError, match="decimal"):
        json.loads('"not-a-decimal"', field=amount_field())


@pytest.mark.parametrize("format", [json, yaml])
def test_cls_value_preserves_the_core_scalar(format: object) -> None:
    native = format.loads("1", cls=Scalar)  # type: ignore[attr-defined]

    assert isinstance(native, Scalar)
    assert native.kind == "u64"
    assert format.loads("1") == 1  # type: ignore[attr-defined]


def test_cls_value_preserves_typed_toml_root_canonicalization() -> None:
    field = Field(
        "row",
        "struct<value: float16 not null>",
        False,
    )

    native = toml.loads("value = 1.5", field=field, cls=Scalar)

    assert isinstance(native, Scalar)
    assert native.kind == "sequence"
    assert toml.loads("value = 1.5", field=field) == {"value": 1.5}


@pytest.mark.parametrize(
    ("format", "content"),
    [
        (json, "1.5\n2.5\n"),
        (yaml, "1\n---\n2\n"),
    ],
)
def test_all_and_lazy_streams_return_native_scalars(format: object, content: str) -> None:
    eager = list(  # type: ignore[attr-defined]
        format.loads_all(content, cls=Scalar)
    )
    lazy = list(  # type: ignore[attr-defined]
        format.load_all(io.StringIO(content), cls=Scalar)
    )

    expected_kind = "f64" if format is json else "u64"
    assert [value.kind for value in eager] == [expected_kind, expected_kind]
    assert [value.kind for value in lazy] == [expected_kind, expected_kind]
    assert all(isinstance(value, Scalar) for value in [*eager, *lazy])


@pytest.mark.parametrize("source", [b"1", "1"])
def test_inferred_decode_returns_the_native_scalar_directly(source: bytes | str) -> None:
    native = _codec._decode_inferred(source, cls=Scalar)

    assert isinstance(native, Scalar)
    assert native.kind == "u64"


def test_single_reader_and_path_return_native_scalars(
    tmp_path: pathlib.Path,
) -> None:
    path = tmp_path / "value.json"
    path.write_bytes(b"1")

    reader_value = json.loads(io.BytesIO(b"1"), cls=Scalar)
    path_value = json.loads(path, cls=Scalar)

    assert reader_value.kind == "u64"
    assert path_value.kind == "u64"


def test_iobase_read_scalar_selects_exact_scalar_without_changing_default() -> None:
    handle = IOBase.from_bytes(b"1")
    handle.media_type = "application/json"

    assert handle.read_scalar() == 1
    native = handle.read_scalar(cls=Scalar)
    assert isinstance(native, Scalar)
    assert native.kind == "u64"

    with pytest.raises(TypeError, match="cls must be Scalar or None"):
        handle.read_scalar(cls=float)


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
            cls=Scalar,
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
        cls=Scalar,
        field=field,
        max_depth=1,
        max_input_bytes=2,
        max_nodes=1,
        max_documents=1,
    )
    assert value.kind == "i16"


@pytest.mark.parametrize(
    ("dtype", "kind"),
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
    dtype: str, kind: str
) -> None:
    value = json.loads(
        "7",
        cls=Scalar,
        field=Field("value", dtype, nullable=False),
    )
    assert value.kind == kind
