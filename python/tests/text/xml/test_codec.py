from __future__ import annotations

import dataclasses
import datetime as dt
import io
import pathlib
from decimal import Decimal

import pytest

from yggdryl import Field, Scalar
from yggdryl.text import xml


def test_public_surface_is_deliberately_single_document() -> None:
    for name in ("dump_all", "dumps_all", "load_all", "loads_all"):
        assert not hasattr(xml, name)


def test_a_document_is_one_root_element_keyed_by_its_name() -> None:
    encoded = xml.dumps({"trade": {"symbol": "AAPL"}})

    assert encoded == b"<trade><symbol>AAPL</symbol></trade>"
    assert xml.loads(encoded) == {"trade": {"symbol": "AAPL"}}

    for value in (None, 42, [1, 2], {"a": 1, "b": 2}):
        with pytest.raises(ValueError):
            xml.dumps(value)


def test_attributes_and_character_data_are_keyed_apart_from_elements() -> None:
    decoded = xml.loads(b'<trade id="7">filled</trade>')

    assert decoded == {"trade": {"@id": "7", "#text": "filled"}}
    assert xml.dumps(decoded) == b'<trade id="7">filled</trade>'


def test_a_repeated_element_is_a_sequence_and_an_empty_one_is_absence() -> None:
    decoded = xml.loads(b"<row><tag>a</tag><tag>b</tag><none/></row>")

    assert decoded == {"row": {"tag": ["a", "b"], "none": None}}
    assert xml.dumps(decoded) == b"<row><none/><tag>a</tag><tag>b</tag></row>"


def test_every_leaf_is_text_until_a_field_types_it() -> None:
    document = b"<row><size>100</size><price>12.50</price><day>2026-08-15</day></row>"

    assert xml.loads(document) == {
        "row": {"size": "100", "price": "12.50", "day": "2026-08-15"}
    }

    field = Field(
        "row",
        "struct<size: int64 not null, price: decimal(18,2) not null, day: date32 not null>",
        nullable=False,
    )
    assert xml.loads(document, field=field) == {
        "size": 100,
        "price": Decimal("12.50"),
        "day": dt.date(2026, 8, 15),
    }


def test_exact_scalars_use_their_interoperable_spelling() -> None:
    value = {
        "row": {
            "payload": b"\x00\xff",
            "decimal": Decimal("123.4500"),
            "date": dt.date(2026, 8, 15),
            "datetime": dt.datetime(2026, 8, 15, 12, 3, 4, 5),
            "zoned": dt.datetime(2026, 8, 15, 12, tzinfo=dt.timezone.utc),
            "delta": dt.timedelta(days=-2, seconds=3, microseconds=4),
        }
    }

    encoded = xml.dumps(value)
    restored = xml.loads(encoded)["row"]

    assert isinstance(encoded, bytes)
    assert restored["payload"] == "AP8="
    assert restored["decimal"] == "123.4500"
    assert restored["date"] == "2026-08-15"
    assert restored["datetime"] == "2026-08-15T12:03:04.000005"
    assert restored["zoned"] == "2026-08-15T12:00:00.000000Z"
    assert restored["delta"] == "-PT172796.999996S"


def test_a_dataclass_lowers_to_the_element_that_names_it() -> None:
    @dataclasses.dataclass
    class Point:
        x: int
        y: int

    encoded = xml.dumps({"point": Point(2, 3)})

    assert encoded == b"<point><x>2</x><y>3</y></point>"
    assert b"python:" not in encoded
    # A document is one root element, so reconstruction names that root: the
    # target sees the one-entry record, never the element's contents alone.
    assert xml.loads(encoded)["point"] == {"x": "2", "y": "3"}
    field = Field("point", "struct<x: int64 not null, y: int64 not null>", nullable=False)
    assert xml.loads(encoded, field=field) == {"x": 2, "y": 3}


def test_a_refused_document_names_the_byte_it_stopped_at() -> None:
    with pytest.raises(ValueError, match="invalid xml data"):
        xml.loads(b"<row>text<id>1</id></row>")
    with pytest.raises(ValueError, match="exactly one root element"):
        xml.loads(b"<a/><b/>")
    with pytest.raises(ValueError):
        xml.dumps({"1st": None})


def test_indent_changes_bytes_and_never_meaning() -> None:
    value = {"row": {"a": "x", "b": {"c": "y"}}}

    compact = xml.dumps(value)
    indented = xml.dumps(value, indent=2)

    assert compact == b"<row><a>x</a><b><c>y</c></b></row>"
    assert indented == b"<row>\n  <a>x</a>\n  <b>\n    <c>y</c>\n  </b>\n</row>"
    assert xml.loads(indented) == value


def test_source_intent_is_type_driven_for_text_binary_and_paths(
    tmp_path: pathlib.Path,
) -> None:
    path = tmp_path / "value.xml"
    path.write_bytes(b"<row><value>42</value></row>")

    assert xml.loads(path) == {"row": {"value": "42"}}
    with pytest.raises(ValueError, match="invalid xml data"):
        xml.loads(str(path))
    assert xml.loads("<row><value>43</value></row>") == {"row": {"value": "43"}}
    assert xml.loads(bytearray(b"<row/>")) == {"row": None}
    assert xml.loads(memoryview(b"<row/>")) == {"row": None}

    binary = io.BytesIO(b"<row><value>44</value></row>")
    text = io.StringIO("<row><value>45</value></row>")
    assert xml.loads(binary) == {"row": {"value": "44"}}
    assert xml.loads(text) == {"row": {"value": "45"}}
    assert not binary.closed
    assert not text.closed


def test_a_destination_writes_the_document_whole(tmp_path: pathlib.Path) -> None:
    path = tmp_path / "out.xml"
    value = {"row": {"id": "1"}}

    assert xml.dump(value, path) is None
    assert path.read_bytes() == b"<row><id>1</id></row>"
    assert xml.dump(value) == b"<row><id>1</id></row>"
    assert xml.dump(value, utf8=True) == "<row><id>1</id></row>"


def test_limits_bound_what_one_document_may_cost() -> None:
    deep = "<a>" * 40 + "</a>" * 40

    with pytest.raises(ValueError, match="nesting depth limit exceeded"):
        xml.loads(deep, max_depth=8)
    with pytest.raises(ValueError, match="input byte limit exceeded"):
        xml.loads("<row/>", max_input_bytes=3)
    with pytest.raises(ValueError, match="node limit exceeded"):
        xml.loads("<row><a/><b/><c/></row>", max_nodes=2)


def test_placeholders_substitute_inside_a_document() -> None:
    decoded = xml.loads(
        "<row><symbol>{{ SYMBOL }}</symbol></row>",
        placeholders={"SYMBOL": "AAPL"},
    )

    assert decoded == {"row": {"symbol": "AAPL"}}


def test_the_core_scalar_is_returned_without_lowering() -> None:
    decoded = xml.loads(b"<row><id>1</id></row>", cls=Scalar)

    assert isinstance(decoded, Scalar)
    assert decoded["row"]["id"] == Scalar("1")
