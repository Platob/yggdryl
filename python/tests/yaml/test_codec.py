from __future__ import annotations

import datetime as dt
import io
import pathlib
from collections import OrderedDict, deque
from decimal import Decimal

import pytest

from yggdryl import yaml
from yggdryl.records import record


@record
class Leg:
    symbol: str
    price: Decimal


@record
class Trade:
    trade_id: int
    leg: Leg
    executed_at: dt.datetime


@record(frozen=True)
class Atom:
    value: int


@record
class Containers:
    listed: list[Atom]
    tupled: tuple[Atom, ...]
    mapped: dict[str, Atom]
    queued: deque[Atom]
    grouped: set[Atom]
    frozen: frozenset[Atom]
    ordered: OrderedDict[str, Atom]
    keyed: dict[Atom, str]
    tuple_keyed: dict[tuple[Atom, ...], str]
    frozen_keyed: dict[frozenset[Atom], str]


def test_nested_record_yaml_is_ordinary_yaml_with_no_class_name() -> None:
    value = Trade(
        42,
        Leg("ABC", Decimal("12.50")),
        dt.datetime(2026, 8, 15, 10, 30),
    )

    encoded = value.into_yaml()
    restored = Trade.from_yaml(encoded)

    assert restored == value
    # A custom `!yggdryl/*` tag would make the document unreadable to other
    # YAML implementations, and a class name in it would be a type nothing
    # validates. The document carries neither.
    assert b"!yggdryl/" not in encoded
    assert b"python:" not in encoded
    assert b"Trade" not in encoded
    assert yaml.loads(encoded) == {
        "trade_id": 42,
        "leg": {"symbol": "ABC", "price": Decimal("12.50")},
        "executed_at": "2026-08-15T10:30:00.000000",
    }


def test_nested_records_round_trip_through_supported_collections() -> None:
    first = Atom(1)
    second = Atom(2)
    value = Containers(
        [first],
        (second,),
        {"first": first},
        deque((first, second), maxlen=4),
        {first, second},
        frozenset((first,)),
        OrderedDict((("second", second),)),
        {first: "first", second: "second"},
        {(first, second): "tuple"},
        {frozenset((first, second)): "frozenset"},
    )

    encoded = value.into_yaml()

    # A record used as a mapping key has to be hashable on the way back, so it
    # decodes as the tuple of its entries; the annotation reads it as a record.
    assert Containers.from_yaml(encoded) == value
    assert b"Atom" not in encoded


def test_plain_yaml_mapping_is_inferred_for_record() -> None:
    @record
    class Item:
        id: int

    assert Item.from_("id: 42\n") == Item(42)
    encoded = Item(43).into_(format="yml")
    assert isinstance(encoded, bytes)
    assert Item.from_(encoded, format="yml") == Item(43)


def test_a_document_written_with_a_tag_decodes_as_ordinary_data() -> None:
    # Nothing here writes a tag any more, and `tag` names no kind the value
    # model has, so a document from an older writer or another producer carries
    # a mapping whose outer key happens to be the marker. It stays readable and
    # nothing in it is reinterpreted.
    encoded = '{"$yggdryl": "tag", "tag": "vendor:quantity", "value": {"lots": 4}}\n'

    assert yaml.loads(encoded) == {
        "$yggdryl": "tag",
        "tag": "vendor:quantity",
        "value": {"lots": 4},
    }

    # An envelope naming no kind this build knows is ordinary user data.
    plain = '{"$yggdryl": "vendor:quantity", "value": 4}\n'
    assert yaml.loads(plain) == {"$yggdryl": "vendor:quantity", "value": 4}

    # A YAML application tag is the annotation YAML defines it to be: the node
    # under it decodes as the plain value it annotates.
    assert yaml.loads("!vendor:quantity {lots: 4}\n") == {"lots": 4}


def test_wide_signed_integers_round_trip_without_precision_loss() -> None:
    values = [1 << 100, -(1 << 100)]

    assert yaml.loads(yaml.dumps(values)) == values
    # Wider than 128 bits, an integer has no exact native shape left and keeps
    # only its magnitude, as text.
    assert yaml.loads(yaml.dumps([1 << 300])) == [str(1 << 300)]


def test_an_arbitrary_object_decodes_as_the_mapping_it_lowered_to() -> None:
    class Quote:
        def __init__(self, symbol: str, price: int) -> None:
            self.symbol = symbol
            self.price = price

    encoded = yaml.dumps(Quote("ABC", 10))

    assert yaml.loads(encoded) == {"symbol": "ABC", "price": 10}


def test_yaml_document_stream_uses_incremental_file_reads() -> None:
    class TrackingBytesIO(io.BytesIO):
        def __init__(self, value: bytes) -> None:
            super().__init__(value)
            self.lines = 0

        def readline(self, *args: object, **kwargs: object) -> bytes:
            assert args and isinstance(args[0], int) and args[0] > 0
            self.lines += 1
            return super().readline(*args, **kwargs)

    stream = TrackingBytesIO(b"---\nid: 1\n---\nid: 2\n")
    iterator = yaml.load_all(stream)
    assert stream.lines == 0
    assert next(iterator) == {"id": 1}
    assert stream.lines < 5
    assert next(iterator) == {"id": 2}
    with pytest.raises(StopIteration):
        next(iterator)


def test_yaml_explicit_end_can_precede_implicit_document() -> None:
    stream = io.BytesIO(b"id: 1\n...\nid: 2\n")

    assert list(yaml.load_all(stream)) == [{"id": 1}, {"id": 2}]


def test_yaml_stream_markers_accept_separation_whitespace_and_comments() -> None:
    content = (
        b"---  # first document\n"
        b"id: 1\n"
        b"... \t# explicit end\n"
        b"---\t# second document\n"
        b"id: 2\n"
    )

    expected = list(yaml.loads_all(content))
    assert expected == [{"id": 1}, {"id": 2}]
    assert list(yaml.load_all(io.BytesIO(content))) == expected


def test_yaml_stream_accepts_binary_and_text_lone_cr_line_breaks() -> None:
    content = "---\rid: 1\r---\rid: 2\r"
    expected = list(yaml.loads_all(content.encode()))

    assert expected == [{"id": 1}, {"id": 2}]
    assert list(yaml.load_all(io.BytesIO(content.encode()))) == expected
    assert list(yaml.load_all(io.StringIO(content))) == expected


def test_yaml_stream_preserves_fragmented_crlf_and_lone_cr_boundaries() -> None:
    class FragmentReader:
        def __init__(self) -> None:
            self.fragments = iter(
                (b"--", b"-\r", b"\n", b"id: 1\r-", b"--\r", b"id: 2", b"\r")
            )

        def readline(self, size: int) -> bytes:
            assert size > 0
            return next(self.fragments, b"")

    expected = [{"id": 1}, {"id": 2}]
    assert list(yaml.load_all(FragmentReader())) == expected


@pytest.mark.parametrize("text", (False, True))
def test_yaml_stream_rejects_hostile_reader_over_return(text: bool) -> None:
    class OversizedReader:
        def read(self, size: int) -> bytes | str:
            value = " " if text else b" "
            return value * (size + 1)

    with pytest.raises(OSError, match="more .* than requested"):
        list(yaml.load_all(OversizedReader()))


def test_yaml_stream_does_not_treat_marker_prefix_scalars_as_markers() -> None:
    content = (
        b"---\n"
        b"---#not a start\n"
        b"continued\n"
        b"---\n"
        b"...#not an end\n"
        b"continued\n"
    )

    expected = list(yaml.loads_all(content))
    assert expected == [
        "---#not a start continued",
        "...#not an end continued",
    ]
    assert list(yaml.load_all(io.BytesIO(content))) == expected


@pytest.mark.parametrize("control", (b"\x0b", b"\x0c"))
def test_yaml_stream_routes_control_whitespace_through_the_core(
    control: bytes,
) -> None:
    content = control + b"\n"

    expected = list(yaml.loads_all(content))
    assert expected == [None]
    assert list(yaml.load_all(io.BytesIO(content))) == expected


@pytest.mark.parametrize("style", ("|", "|-", "|+", ">", ">-", ">+"))
@pytest.mark.parametrize("line_break", ("\n", "\r", "\r\n"))
def test_yaml_stream_preserves_empty_block_scalar_chomping_at_document_split(
    style: str,
    line_break: str,
) -> None:
    content = (
        f"text: {style}{line_break}"
        f"---{line_break}"
        f"id: 2{line_break}"
    ).encode()

    expected = list(yaml.loads_all(content))
    assert expected == [{"text": ""}, {"id": 2}]
    assert list(yaml.load_all(io.BytesIO(content))) == expected


def test_yaml_stream_preserves_explicit_and_spelled_null_documents() -> None:
    content = b"---\nnull\n---\n...\n---\nid: 3\n"

    assert list(yaml.loads_all(content)) == [None, None, {"id": 3}]
    assert list(yaml.load_all(io.BytesIO(content))) == [None, None, {"id": 3}]


def test_yaml_stream_accumulates_short_readline_fragments() -> None:
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

    stream = FragmentReader(b"---\nid: 1\n...\nid: 2\n")
    assert list(yaml.load_all(stream)) == [{"id": 1}, {"id": 2}]


def test_yaml_stream_does_not_split_indented_block_scalar_markers() -> None:
    content = b"---\ntext: |\n  ---\n  ...\n---\nid: 2\n"

    assert list(yaml.load_all(io.BytesIO(content))) == [
        {"text": "---\n...\n"},
        {"id": 2},
    ]


def test_yaml_stream_errors_report_original_cumulative_byte() -> None:
    iterator = yaml.load_all(io.BytesIO(b"id: 1\n---\nitems: [1, 2\n"))
    assert next(iterator) == {"id": 1}
    with pytest.raises(
        ValueError,
        match=r"YAML document 2: .*at byte 23 \(document byte 17\)",
    ):
        next(iterator)


def test_python_key_collisions_are_rejected() -> None:
    with pytest.raises(ValueError, match="mapping keys collide"):
        yaml.loads("1: integer\n1.0: float\n")


def test_yaml_dump_all_streams_generator(tmp_path: pathlib.Path) -> None:
    path = tmp_path / "values.yaml"
    seen: list[int] = []

    def values():
        for index in range(3):
            seen.append(index)
            yield {"id": index}

    yaml.dump_all(values(), path)

    assert seen == [0, 1, 2]
    assert list(yaml.load_all(path)) == [{"id": 0}, {"id": 1}, {"id": 2}]


def test_str_non_file_is_content_but_pathlike_is_always_path(
    tmp_path: pathlib.Path,
) -> None:
    assert yaml.loads("answer: 42") == {"answer": 42}
    missing = tmp_path / "missing.yaml"
    with pytest.raises(FileNotFoundError):
        yaml.loads(missing)
