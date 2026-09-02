from __future__ import annotations

import dataclasses
import datetime as dt
import enum
import io
import os
import pathlib
import subprocess
import sys
import uuid
import zoneinfo
from collections import Counter, OrderedDict, defaultdict, deque
from decimal import Decimal
from typing import NamedTuple

import pytest

from yggdryl import DataType, Field, Uri, Url, Urn, json, scalar, yaml


@scalar
class Fill:
    price: Decimal
    when: dt.datetime


@scalar
class Order:
    order_id: int
    fill: Fill


def test_first_class_scalars_round_trip_exactly() -> None:
    value = {
        "bytes": b"\x00\xff",
        "decimal": Decimal("123.4500"),
        "date": dt.date(2026, 8, 15),
        "time": dt.time(12, 3, 4, 5),
        "naive": dt.datetime(2026, 8, 15, 12, 3, 4, 5),
        "zoned": dt.datetime(
            2026, 8, 15, 12, 3, 4, 5, tzinfo=zoneinfo.ZoneInfo("Europe/Paris")
        ),
        "utc": dt.datetime(2026, 8, 15, tzinfo=dt.timezone.utc),
        "offset": dt.datetime(
            2026, 8, 15, tzinfo=dt.timezone(dt.timedelta(hours=5, minutes=30))
        ),
        "delta": dt.timedelta(days=-2, seconds=3, microseconds=4),
    }

    encoded = json.dumps(value)
    restored = json.loads(encoded)

    assert isinstance(encoded, bytes)
    # Types JSON cannot prove use interoperable text. An explicit Field is what
    # restores bytes, decimals, and exact temporal widths.
    assert restored["bytes"] == "AP8="
    assert restored["decimal"] == "123.4500"
    assert restored["date"] == "2026-08-15"
    assert restored["time"] == "12:03:04.000005"
    assert restored["naive"] == "2026-08-15T12:03:04.000005"
    # A zone survives as the zone name, not as the offset it happened to be
    # at, because the offset alone cannot say Europe/Paris.
    assert restored["zoned"] == "2026-08-15T12:03:04.000005+02:00[Europe/Paris]"
    assert restored["utc"] == "2026-08-15T00:00:00.000000Z"
    assert restored["offset"] == "2026-08-15T00:00:00.000000+05:30"
    assert restored["delta"] == "-PT172796.999996S"


def test_temporal_and_decimal_names_are_the_cross_language_ones() -> None:
    value = {
        "at": dt.datetime(2026, 8, 15, tzinfo=dt.timezone.utc),
        "on": dt.date(2026, 8, 15),
        "price": Decimal("-10.50"),
    }

    encoded = json.dumps(value).decode()

    # Values outside JSON's grammar use ordinary interoperable strings.
    assert '"at":"2026-08-15T00:00:00.000000Z"' in encoded
    assert '"on":"2026-08-15"' in encoded
    assert '"price":"-10.50"' in encoded
    assert "python:" not in encoded


def test_values_without_a_native_shape_lower_and_lose_their_class() -> None:
    value = {
        "huge": 1 << 300,
        "uuid": uuid.UUID("12345678-1234-5678-1234-567812345678"),
        "path": pathlib.PurePosixPath("nested/file.arrow"),
        "tuple": (1, "two"),
        "set": {3, 1, 2},
        "frozen": frozenset(("b", "a")),
        "deque": deque((1, 2, 3), maxlen=4),
        "bytearray": bytearray(b"abc"),
        "memoryview": memoryview(b"xyz"),
        "complex": complex(-1.5, 2.25),
        "range": range(-4, 11, 3),
        "slice": slice(None, 8, 2),
        "counter": Counter("ababa"),
        "ordered": OrderedDict((("b", 2), ("a", 1))),
        "defaultdict": defaultdict(list, {"a": [1]}),
    }

    restored = json.loads(json.dumps(value))

    # An integer wider than 128 bits keeps its magnitude and loses its type.
    assert restored["huge"] == str(1 << 300)
    assert restored["uuid"] == "12345678-1234-5678-1234-567812345678"
    assert restored["path"] == "nested/file.arrow"
    assert restored["tuple"] == [1, "two"]
    # A set has no order, so it is written sorted and read back as a list.
    assert restored["set"] == [1, 2, 3]
    assert restored["frozen"] == ["a", "b"]
    # A deque's bound is not part of the value, so it does not survive either.
    assert restored["deque"] == [1, 2, 3]
    assert restored["bytearray"] == "YWJj"
    assert restored["memoryview"] == "eHl6"
    assert restored["complex"] == [-1.5, 2.25]
    assert restored["range"] == [-4, 11, 3]
    assert restored["slice"] == [None, 8, 2]
    assert restored["counter"] == {"a": 3, "b": 2}
    assert list(restored["ordered"].items()) == [("a", 1), ("b", 2)]
    assert restored["defaultdict"] == {"a": [1]}
    assert {name: type(item) for name, item in restored.items()} == {
        "huge": str,
        "uuid": str,
        "path": str,
        "tuple": list,
        "set": list,
        "frozen": list,
        "deque": list,
        "bytearray": str,
        "memoryview": str,
        "complex": list,
        "range": list,
        "slice": list,
        "counter": dict,
        "ordered": dict,
        "defaultdict": dict,
    }


def test_a_subclass_lowers_as_the_type_it_subclasses() -> None:
    class MutableBytes(bytearray):
        pass

    class ComplexNumber(complex):
        pass

    class LimitedDeque(deque[int]):
        pass

    class Labels(dict[str, int]):
        pass

    class Ticks(list[int]):
        pass

    restored = json.loads(
        json.dumps(
            [
                MutableBytes(b"abc"),
                ComplexNumber(1.5, -2.25),
                LimitedDeque((1, 2), maxlen=3),
                Labels(a=1),
                Ticks([7]),
            ]
        )
    )

    assert restored == ["YWJj", [1.5, -2.25], [1, 2], {"a": 1}, [7]]


def test_schema_wrappers_lower_structurally_and_locations_lower_to_text() -> None:
    values = [
        DataType("list<int64>"),
        Field("price", "decimal(18,4)", nullable=False),
        Uri("s3://warehouse/orders/data.parquet"),
        Url("https://example.com/orders?id=42"),
        Urn("urn:yggdryl:orders:42"),
    ]
    expected = [
        {
            "field": {
                "dtype": {"type": "int64"},
                "metadata": {},
                "name": "item",
                "nullable": True,
            },
            "type": "list",
        },
        {
            "dtype": {"precision": 18, "scale": 4, "type": "decimal128"},
            "metadata": {},
            "name": "price",
            "nullable": False,
        },
        "s3://warehouse/orders/data.parquet",
        "https://example.com/orders?id=42",
        "urn:yggdryl:orders:42",
    ]

    for codec in (json, yaml):
        restored = codec.loads(codec.dumps(values))
        assert restored == expected
        assert DataType.from_dict(restored[0]) == values[0]
        assert Field.from_dict(restored[1]) == values[1]
        assert [
            type(value).from_str(text)
            for value, text in zip(values[2:], restored[2:])
        ] == values[2:]


def test_dataclasses_named_tuples_and_enums_lower_to_their_shape() -> None:
    class Side(enum.Enum):
        BUY = "buy"

    class Point(NamedTuple):
        x: int
        y: int

    @dataclasses.dataclass
    class Leg:
        side: Side
        point: Point

    encoded = json.dumps(Leg(Side.BUY, Point(2, 3)))

    # A member is its value, a named tuple is its members, a dataclass is its
    # fields. None of the three names its class in the document.
    assert json.loads(encoded) == {"side": "buy", "point": {"x": 2, "y": 3}}
    assert b"python:" not in encoded


def test_an_arbitrary_object_lowers_to_its_attributes() -> None:
    class Quote:
        def __init__(self, symbol: str, price: int) -> None:
            self.symbol = symbol
            self.price = price

    assert json.loads(json.dumps(Quote("ABC", 10))) == {
        "symbol": "ABC",
        "price": 10,
    }

    assert json.loads(json.dumps(object())) == {}


def test_dataclass_decodes_only_through_an_explicit_target() -> None:
    @dataclasses.dataclass
    class Point:
        x: int
        y: int

    point = Point(2, 3)
    encoded = json.dumps(point)

    # Nothing in the document names the class, so an untargeted decode is data.
    assert json.loads(encoded) == {"x": 2, "y": 3}
    assert json.loads(encoded, cls=Point) == point


def test_dataclass_dump_hot_path_does_not_call_dataclasses_fields(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    @dataclasses.dataclass
    class Point:
        x: int
        y: int

    def fail_fields(value: object) -> object:
        raise AssertionError(f"dataclasses.fields allocated for {value!r}")

    monkeypatch.setattr(dataclasses, "fields", fail_fields)
    assert json.loads(json.dumps(Point(2, 3))) == {"x": 2, "y": 3}
    field_bytes = json.dumps(Order(
        7, Fill(Decimal("1.25"), dt.datetime(2026, 8, 15))
    ))
    assert isinstance(field_bytes, bytes)


def test_nested_field_class_round_trips_as_plain_nested_mappings() -> None:
    value = Order(7, Fill(Decimal("12.5"), dt.datetime(2026, 8, 15, 8)))

    encoded = json.dumps(value)
    restored = json.loads(encoded, cls=Order)

    assert restored == value
    # The document is the data and nothing else; the target supplies the type.
    # Read without a target, the instant is the classic string it was written
    # as - the dataclass above is what turned it back into a datetime.
    assert b"python:" not in encoded
    assert json.loads(encoded) == {
        "order_id": 7,
        "fill": {"price": "12.5", "when": "2026-08-15T08:00:00.000000"},
    }


def test_nested_dataclass_is_cast_then_constructed_exactly_once() -> None:
    post_init_calls: list[int] = []

    @dataclasses.dataclass
    class Child:
        value: int

        def __post_init__(self) -> None:
            assert type(self.value) is int
            post_init_calls.append(self.value)

    @scalar
    class Parent:
        child: Child

    encoded = json.dumps({"child": {"value": "7"}})

    restored = json.loads(encoded, cls=Parent)

    assert restored.child.value == 7
    assert post_init_calls == [7]


def test_field_class_codec_uses_shared_safe_conversion() -> None:
    @scalar
    class Casted:
        value: int | None

    encoded = json.dumps({"value": "7"})
    assert json.loads(encoded, cls=Casted).value == 7
    shallow = json.loads(encoded, cls=Casted, safe=False)
    assert shallow.value == "7"


def test_field_class_target_rejects_a_document_of_another_shape() -> None:
    @scalar
    class Other:
        symbol: str

    value = Order(1, Fill(Decimal("1"), dt.datetime(2026, 8, 15)))

    with pytest.raises(TypeError, match="unknown keys"):
        json.loads(json.dumps(value), cls=Other)


def test_source_intent_is_type_driven_without_existence_probes(
    tmp_path: pathlib.Path,
) -> None:
    path = tmp_path / "value.json"
    path.write_bytes(b'{"value":42}')

    assert json.loads(path) == {"value": 42}
    with pytest.raises(ValueError, match="invalid json data"):
        json.loads(str(path))
    assert json.loads('{"value":43}') == {"value": 43}
    stream = io.BytesIO(b'{"value":44}')
    assert json.loads(stream) == {"value": 44}
    assert not stream.closed


def test_json_lines_content_and_lazy_file_stream(tmp_path: pathlib.Path) -> None:
    path = tmp_path / "rows.jsonl"
    produced: list[int] = []

    def values():
        for value in range(3):
            produced.append(value)
            yield {"value": value}

    json.dump_all(values(), path)
    assert produced == [0, 1, 2]
    assert list(json.load_all(path)) == [
        {"value": 0},
        {"value": 1},
        {"value": 2},
    ]
    assert list(json.loads_all(path.read_bytes())) == [
        {"value": 0},
        {"value": 1},
        {"value": 2},
    ]


def test_buffered_collection_encode_is_bounded_and_closes_generator() -> None:
    for encode_all in (json.dumps_all, yaml.dumps_all):
        closed = False

        def values():
            nonlocal closed
            try:
                for index in range(1_025):
                    yield index
            finally:
                closed = True

        with pytest.raises(ValueError, match="exceeds the 1024-document limit"):
            encode_all(values())
        assert closed


def test_json_lines_materializes_field_classes_lazily(tmp_path: pathlib.Path) -> None:
    value = Order(1, Fill(Decimal("2"), dt.datetime(2026, 8, 15)))
    one = tmp_path / "one.jsonl"
    empty = tmp_path / "empty.jsonl"
    two = tmp_path / "two.jsonl"
    json.dump_all((value,), one)
    empty.write_bytes(b"")
    json.dump_all((value, value), two)

    assert list(json.load_all(one, cls=Order)) == [value]
    assert list(json.load_all(empty, cls=Order)) == []
    assert list(json.load_all(two, cls=Order)) == [value, value]


def test_generic_output_infers_nonexistent_string_destinations(
    tmp_path: pathlib.Path,
) -> None:
    value = Order(1, Fill(Decimal("2"), dt.datetime(2026, 8, 15)))
    json_path = str(tmp_path / "new.json")
    json_lines_path = str(tmp_path / "new.jsonl")
    yaml_path = str(tmp_path / "new.yaml")

    assert json.dump(value, json_path) is None
    assert json.dump_all((value,), json_lines_path) is None
    assert yaml.dump(value, yaml_path) is None
    assert json.loads(pathlib.Path(json_path), cls=Order) == value
    assert list(json.load_all(pathlib.Path(json_lines_path), cls=Order)) == [value]
    assert yaml.loads(pathlib.Path(yaml_path), cls=Order) == value


def test_bounded_reader_and_partial_writer() -> None:
    class BoundedReader:
        def __init__(self) -> None:
            self.remaining = bytearray(b'{"value":42}')

        def read(self, size: int) -> bytes:
            assert 0 < size < 70_000_000
            if not self.remaining:
                return b""
            count = min(size, 3)
            result = bytes(self.remaining[:count])
            del self.remaining[:count]
            return result

    class PartialWriter:
        def __init__(self) -> None:
            self.data = bytearray()

        def write(self, value: bytes) -> int:
            count = min(3, len(value))
            self.data.extend(value[:count])
            return count

    assert json.loads(BoundedReader()) == {"value": 42}
    writer = PartialWriter()
    json.dump({"value": 42}, writer)
    assert bytes(writer.data) == json.dumps({"value": 42})
    owned_by_caller = io.BytesIO()
    json.dump({"value": 42}, owned_by_caller)
    assert not owned_by_caller.closed
    text_stream = io.StringIO()
    json.dump({"value": 42}, text_stream)
    assert json.loads(text_stream.getvalue()) == {"value": 42}
    assert not text_stream.closed


def test_bounded_reader_detects_a_tail_after_the_limit(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    import yggdryl._codec as codec

    class TinyReader:
        def __init__(self) -> None:
            self.remaining = bytearray(b"12345")

        def read(self, size: int) -> bytes:
            assert size > 0
            if not self.remaining:
                return b""
            result = bytes(self.remaining[:2])
            del self.remaining[:2]
            return result

    monkeypatch.setattr(codec, "_MAX_STREAM_BYTES", 4)
    with pytest.raises(ValueError, match="exceeds 4 bytes"):
        codec.read_bytes(TinyReader())

    assert codec.read_bytes(io.BytesIO(b"1234")) == b"1234"


def test_binary_writer_type_error_after_progress_never_retries_as_text() -> None:
    class FailingWriter:
        def __init__(self) -> None:
            self.calls: list[bytes | str] = []

        def write(self, value: bytes | str) -> int:
            self.calls.append(value)
            if len(self.calls) == 1:
                assert isinstance(value, bytes)
                return 2
            raise TypeError("deliberate failure")

    writer = FailingWriter()
    with pytest.raises(TypeError, match="deliberate failure"):
        json.dump({"value": 42}, writer)
    assert len(writer.calls) == 2
    assert all(isinstance(value, bytes) for value in writer.calls)


def test_json_lines_accumulates_short_readline_fragments() -> None:
    class FragmentReader:
        def __init__(self, value: bytes) -> None:
            self.remaining = bytearray(value)

        def readline(self, size: int) -> bytes:
            assert size > 0
            if not self.remaining:
                return b""
            count = min(size, 2)
            newline = self.remaining.find(b"\n", 0, count)
            if newline >= 0:
                count = newline + 1
            result = bytes(self.remaining[:count])
            del self.remaining[:count]
            return result

    stream = FragmentReader(b'{"id":1}\n{"id":2}\n')
    assert list(json.load_all(stream)) == [{"id": 1}, {"id": 2}]


def test_stream_record_decode_validates_its_target_once(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    import yggdryl._codec as codec

    encoded = json.dumps_all(
        (
            Order(1, Fill(Decimal("1.5"), dt.datetime(2026, 8, 15))),
            Order(2, Fill(Decimal("2.5"), dt.datetime(2026, 8, 16))),
        )
    )
    original = codec._check_decode_options
    calls = 0

    def counted(cls: object, safe: bool, errors: str) -> None:
        nonlocal calls
        calls += 1
        original(cls, safe, errors)  # type: ignore[arg-type]

    monkeypatch.setattr(codec, "_check_decode_options", counted)
    restored = list(json.load_all(io.BytesIO(encoded), cls=Order))

    assert [value.order_id for value in restored] == [1, 2]
    assert calls == 1


def test_json_lines_enforces_cumulative_byte_and_document_budgets(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    import yggdryl._codec as codec

    monkeypatch.setattr(codec, "_MAX_STREAM_BYTES", 12)
    iterator = json.load_all(io.BytesIO(b'{"id":1}\n{"id":2}\n'))
    assert next(iterator) == {"id": 1}
    with pytest.raises(
        ValueError, match=r"invalid json data at byte 9: input byte limit exceeded"
    ):
        next(iterator)

    monkeypatch.setattr(codec, "_MAX_STREAM_BYTES", 64)
    monkeypatch.setattr(codec, "_MAX_STREAM_DOCUMENTS", 2)
    iterator = json.load_all(io.BytesIO(b"1\n2\n3\n"))
    assert next(iterator) == 1
    assert next(iterator) == 2
    with pytest.raises(
        ValueError, match=r"invalid json data at byte 4: document limit exceeded"
    ):
        next(iterator)


def test_json_lines_stream_errors_use_core_cumulative_offsets() -> None:
    stream = io.BytesIO(b'{"id":1}\n{bad}\n')
    iterator = json.load_all(stream)
    assert next(iterator) == {"id": 1}
    with pytest.raises(
        ValueError,
        match=r"invalid json data at byte 10: JSON object key must be a string",
    ):
        next(iterator)
    with pytest.raises(StopIteration):
        next(iterator)
    assert not stream.closed


def test_json_lines_reader_errors_preserve_identity_and_fuse() -> None:
    expected = RuntimeError("reader failed")

    class FailingReader:
        def __init__(self) -> None:
            self.calls = 0

        def readline(self, size: int) -> bytes:
            assert size > 0
            self.calls += 1
            if self.calls == 1:
                return b"1\n"
            raise expected

    iterator = json.load_all(FailingReader())
    assert next(iterator) == 1
    with pytest.raises(RuntimeError) as failure:
        next(iterator)
    assert failure.value is expected
    with pytest.raises(StopIteration):
        next(iterator)


@pytest.mark.parametrize("control", (b"\x0b", b"\x0c"))
def test_json_lines_stream_does_not_skip_non_json_whitespace(
    control: bytes,
) -> None:
    iterator = json.load_all(io.BytesIO(control + b"\n1\n"))

    with pytest.raises(
        ValueError,
        match=r"invalid json data at byte 0: expected a JSON value",
    ):
        next(iterator)


@pytest.mark.parametrize(
    ("content", "expected"),
    [
        (b"42", 42),
        (b"-1", -1),
        (b"1.5", 1.5),
        (b"null  \n", None),
    ],
)
def test_generic_content_inference_accepts_json_scalars(
    content: bytes, expected: object
) -> None:
    from yggdryl._codec import infer_format

    assert infer_format(content) == "json"
    assert json.loads(content) == expected


@pytest.mark.parametrize(
    ("alias", "expected"),
    [
        (" JSON ", "json"),
        (".NDJSON", "json_lines"),
        ("json-lines", "json_lines"),
        ("application/json", "json"),
        ("application/toml", "toml"),
        (".YML", "yaml"),
    ],
)
def test_format_aliases_delegate_to_the_native_format_parser(
    alias: str, expected: str
) -> None:
    import yggdryl._codec as codec

    assert codec._normalize_format(alias) == expected
    assert codec._normalize_format(alias) == codec._native._codec_normalize_format(
        alias
    )


@pytest.mark.parametrize(
    ("name", "expected"),
    [
        ("events.JSON", "json"),
        ("events.JSON ", "json"),
        ("events.NDJSON", "json_lines"),
        ("config.TOML", "toml"),
        ("schema.YML", "yaml"),
    ],
)
def test_source_and_destination_suffixes_use_the_native_path_table(
    tmp_path: pathlib.Path, name: str, expected: str
) -> None:
    import yggdryl._codec as codec

    path = tmp_path / name
    native = codec._native._codec_infer_path(os.fspath(path))
    assert codec.infer_format(path) == native == expected
    assert codec.infer_destination_format(path) == native


def test_named_stream_path_inference_does_not_consume_the_source() -> None:
    import yggdryl._codec as codec

    class NamedSource:
        name = "events.ndjson"

        def read(self, size: int = -1) -> bytes:
            raise AssertionError(f"named source was unexpectedly read with size {size}")

    source = NamedSource()
    selected, retained = codec.prepare_source(source)
    assert selected == "json_lines"
    assert retained is source


def test_cycles_and_excessive_depth_are_rejected() -> None:
    cyclic: list[object] = []
    cyclic.append(cyclic)
    with pytest.raises(ValueError, match="cyclic"):
        json.dumps(cyclic)

    deep: object = None
    for _ in range(130):
        deep = [deep]
    with pytest.raises(ValueError, match="level codec limit"):
        json.dumps(deep)

    @scalar
    class CyclicField:
        values: list[int]

    values: list[object] = []
    values.append(values)
    with pytest.raises(ValueError, match="cyclic"):
        json.dumps(CyclicField(values))  # type: ignore[arg-type]


def test_maximum_native_nesting_is_safe_in_a_subprocess() -> None:
    code = """
from yggdryl import json

value = None
for _ in range(127):
    value = [value]
encoded = json.dumps(value)
assert json.loads(encoded) == value

too_deep = [value]
try:
    json.dumps(too_deep)
except ValueError as error:
    assert "128-level codec limit" in str(error)
else:
    raise AssertionError("128 nested containers must be rejected")
print("safe")
"""
    result = subprocess.run(
        [sys.executable, "-c", code],
        check=False,
        capture_output=True,
        text=True,
    )

    assert result.returncode == 0, result.stderr
    assert result.stdout.strip() == "safe"


def test_decode_options_are_validated_before_parsing() -> None:
    with pytest.raises(TypeError, match="cls must be a type"):
        json.loads(b"not json", cls=42)  # type: ignore[call-overload]
    with pytest.raises(TypeError, match="safe must be bool"):
        json.loads(b"null", safe=1)  # type: ignore[call-overload]
    with pytest.raises(ValueError, match="errors must be"):
        json.loads(b"null", errors="ignore")  # type: ignore[call-overload]


def test_text_and_io_sources_redirect_without_python_byte_staging(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    import yggdryl._codec as codec

    def staged_read_forbidden(source: object) -> bytes:
        raise AssertionError(f"unexpected Python staging for {source!r}")

    monkeypatch.setattr(codec, "read_bytes", staged_read_forbidden)
    assert json.loads('{"label":"café"}') == {"label": "café"}
    assert json.loads(io.StringIO('{"label":"café"}')) == {
        "label": "café"
    }
    assert yaml.loads(io.BytesIO("label: café\n".encode())) == {
        "label": "café"
    }
    assert list(json.loads_all("1\n2\n")) == [1, 2]
    assert list(yaml.loads_all("---\nid: 1\n---\nid: 2\n")) == [
        {"id": 1},
        {"id": 2},
    ]


def test_field_class_decode_reads_anonymous_io_exactly_once() -> None:
    value = Order(7, Fill(Decimal("12.50"), dt.datetime(2026, 8, 15)))

    assert json.loads(io.StringIO(json.dumps(value).decode()), cls=Order) == value
    assert yaml.loads(io.BytesIO(yaml.dumps(value)), cls=Order) == value

    class ReadOnce:
        def __init__(self, payload: bytes) -> None:
            self.payload = payload
            self.calls = 0

        def read(self, size: int) -> bytes:
            assert size > 0
            self.calls += 1
            if self.calls == 1:
                return self.payload
            if self.calls == 2:
                return b""
            raise AssertionError("anonymous stream was read a second time")

    source = ReadOnce(json.dumps(value))
    assert json.loads(source, cls=Order) == value
    assert source.calls == 2


def test_every_string_source_is_document_content() -> None:
    assert json.loads('"missing/value.json"') == "missing/value.json"
    with pytest.raises(TypeError, match="must be a mapping"):
        json.loads('"missing/value.yaml"', cls=Order)


@pytest.mark.parametrize("codec", (json, yaml))
def test_single_dump_conversion_error_does_not_truncate_existing_path(
    codec: object, tmp_path: pathlib.Path
) -> None:
    path = tmp_path / "existing.data"
    path.write_bytes(b"keep me")
    cyclic: list[object] = []
    cyclic.append(cyclic)

    with pytest.raises(ValueError, match="cyclic"):
        codec.dump(cyclic, path)  # type: ignore[attr-defined]

    assert path.read_bytes() == b"keep me"


def test_bytes_pathlike_uses_normal_python_path_semantics(
    tmp_path: pathlib.Path,
) -> None:
    class BytesPath(os.PathLike[bytes]):
        def __init__(self, value: bytes) -> None:
            self.value = value

        def __fspath__(self) -> bytes:
            return self.value

    destination = BytesPath(os.fsencode(tmp_path / "value.json"))
    json.dump({"value": 42}, destination)
    assert json.loads(destination) == {"value": 42}


@pytest.mark.parametrize("codec", (json, yaml))
def test_partial_text_writer_preserves_large_unicode(codec: object) -> None:
    class PartialTextWriter:
        def __init__(self) -> None:
            self.parts: list[str] = []
            self.binary_attempts = 0

        def write(self, value: bytes | str) -> int:
            if isinstance(value, bytes):
                self.binary_attempts += 1
                raise TypeError("text only")
            count = min(3, len(value))
            self.parts.append(value[:count])
            return count

    value = {"label": "é🙂" * 5_000}
    destination = PartialTextWriter()
    codec.dump(value, destination)  # type: ignore[attr-defined]

    assert destination.binary_attempts == 1
    assert codec.loads("".join(destination.parts)) == value  # type: ignore[attr-defined]


def test_native_reader_rejects_hostile_oversized_chunks() -> None:
    class OversizedReader:
        def read(self, size: int) -> bytes:
            return b" " * (size + 1)

    with pytest.raises(OSError, match="more data than requested"):
        json.loads(OversizedReader())


def test_text_reader_replays_multibyte_bytes_beyond_requested_buffer() -> None:
    label = "é🙂" * 5_000
    source = io.StringIO('{"label":"' + label + '"}')

    assert json.loads(source) == {"label": label}


@pytest.mark.parametrize("codec", (json, yaml))
def test_buffered_writer_coalesces_native_serializer_tokens(codec: object) -> None:
    class CountingWriter:
        def __init__(self) -> None:
            self.chunks: list[bytes] = []

        def write(self, value: bytes) -> int:
            self.chunks.append(value)
            return len(value)

    value = {"values": list(range(5_000))}
    destination = CountingWriter()
    codec.dump(value, destination)  # type: ignore[attr-defined]

    assert len(destination.chunks) < 100
    assert codec.loads(b"".join(destination.chunks)) == value  # type: ignore[attr-defined]
