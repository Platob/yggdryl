from __future__ import annotations

import datetime as dt
import io
import pathlib
from collections import OrderedDict, deque
from decimal import Decimal

import pytest

from yggdryl import scalar, yaml


@scalar
class Leg:
    symbol: str
    price: Decimal


@scalar
class Trade:
    trade_id: int
    leg: Leg
    executed_at: dt.datetime


@scalar(frozen=True)
class Atom:
    value: int


@scalar
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


def test_nested_field_class_yaml_is_ordinary_yaml_with_no_class_name() -> None:
    value = Trade(
        42,
        Leg("ABC", Decimal("12.50")),
        dt.datetime(2026, 8, 15, 10, 30),
    )

    encoded = yaml.dumps(value)
    restored = yaml.loads(encoded, cls=Trade)

    assert restored == value
    # No custom tag metadata is written, so the document stays language-portable.
    assert b"python:" not in encoded
    assert b"Trade" not in encoded
    assert yaml.loads(encoded) == {
        "trade_id": 42,
        "leg": {"symbol": "ABC", "price": "12.50"},
        "executed_at": "2026-08-15T10:30:00.000000",
    }


def test_nested_field_classes_round_trip_through_supported_collections() -> None:
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

    encoded = yaml.dumps(value)

    # A field class used as a mapping key has to be hashable on the way back,
    # so it decodes as the tuple of its entries; the annotation restores it.
    assert yaml.loads(encoded, cls=Containers) == value
    assert b"Atom" not in encoded


def test_plain_yaml_mapping_materializes_a_field_class() -> None:
    @scalar
    class Item:
        id: int

    assert yaml.loads("id: 42\n", cls=Item) == Item(42)
    encoded = yaml.dumps(Item(43))
    assert isinstance(encoded, bytes)
    assert yaml.loads(encoded, cls=Item) == Item(43)


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
            self.reads = 0

        def read(self, size: int) -> bytes:
            assert size > 0
            self.reads += 1
            return super().read(min(size, 4))

    padding = b"x" * 20_000
    stream = TrackingBytesIO(
        b"---\nid: 1\n---\nid: 2\npadding: " + padding + b"\n"
    )
    iterator = yaml.load_all(stream)
    assert stream.reads == 0
    assert next(iterator) == {"id": 1}
    assert stream.reads > 0
    assert stream.tell() < len(stream.getvalue())
    assert next(iterator) == {"id": 2, "padding": padding.decode()}
    with pytest.raises(StopIteration):
        next(iterator)
    assert not stream.closed


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


def test_yaml_stream_errors_use_core_cumulative_offsets() -> None:
    iterator = yaml.load_all(io.BytesIO(b"id: 1\n---\nitems: [1, 2\n"))
    assert next(iterator) == {"id": 1}
    with pytest.raises(
        ValueError,
        match=r"invalid yaml data at byte 23: .*expected ',' or ']'",
    ):
        next(iterator)
    with pytest.raises(StopIteration):
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


def test_strings_are_content_and_pathlike_values_are_always_paths(
    tmp_path: pathlib.Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    import yggdryl._codec as codec

    assert yaml.loads("answer: 42") == {"answer": 42}
    existing = tmp_path / "existing.yaml"
    existing.write_text("answer: 43\n")

    def unexpected_probe(_value: object) -> bool:
        raise AssertionError("string sources must not probe the filesystem")

    monkeypatch.setattr(
        codec.os.path,
        "isfile",
        unexpected_probe,
    )
    assert yaml.loads(str(existing)) == str(existing)
    missing = tmp_path / "missing.yaml"
    with pytest.raises(FileNotFoundError):
        yaml.loads(missing)
