"""Tests."""

from __future__ import annotations

import dataclasses
import datetime as dt
import io
import pathlib
from decimal import Decimal

import pytest

from yggdryl import DataType, Field, Scalar, scalar, xml
from yggdryl.text import codec


@scalar(frozen=True, slots=True)
class Fill:
    price: Decimal
    observed_at: dt.datetime


@scalar(frozen=True, slots=True)
class Order:
    order_id: int
    fill: Fill
    labels: tuple[str, ...] = ()


def test_public_surface_is_deliberately_single_document() -> None:
    for name in ("dump_all", "dumps_all", "load_all", "loads_all"):
        assert not hasattr(xml, name)


def test_a_document_is_a_record_naming_its_root_and_round_trips() -> None:
    source = (
        '<order id="7"><symbol>AAPL &amp; co</symbol>'
        "<leg>1</leg><leg>2</leg><note/></order>"
    )
    value = xml.loads(source)
    assert value == {
        "order": {
            "@id": "7",
            "symbol": "AAPL & co",
            "leg": ["1", "2"],
            "note": None,
        }
    }
    encoded = xml.dumps(value)
    assert encoded == (
        b'<order id="7"><leg>1</leg><leg>2</leg><note/>'
        b"<symbol>AAPL &amp; co</symbol></order>"
    )
    assert xml.loads(encoded) == value
    assert xml.loads(encoded, cls=Scalar).kind == "struct"
    assert xml.dumps(value, indent=2) == (
        b'<order id="7">\n  <leg>1</leg>\n  <leg>2</leg>\n  <note/>\n'
        b"  <symbol>AAPL &amp; co</symbol>\n</order>"
    )


def test_roots_that_are_not_one_element_are_refused() -> None:
    for value in (None, 42, [1, 2], {"a": 1, "b": 2}, {}, {1: "one"}):
        with pytest.raises(ValueError, match="document element|root must be a record|names must be strings"):
            xml.dumps(value)
    with pytest.raises(ValueError, match="would repeat the root"):
        xml.dumps({"rows": [1, 2]})
    with pytest.raises(ValueError, match="sequence inside a sequence"):
        xml.dumps({"a": {"m": [[1]]}})
    with pytest.raises(ValueError, match="expected an XML name"):
        xml.dumps({"1st": 1})


def test_leaves_write_their_text_spellings_and_read_back_as_text() -> None:
    value = {
        "row": {
            "payload": b"\x00\xff",
            "decimal": Decimal("123.4500"),
            "date": dt.date(2026, 8, 15),
            "datetime": dt.datetime(2026, 8, 15, 12, 3, 4, 5, tzinfo=dt.timezone.utc),
            "flag": True,
            "count": 3,
            "ratio": 1.5,
        }
    }
    encoded = xml.dumps(value)
    # A Python mapping keeps its own order across the boundary; a record read
    # back is sorted, as every document here is.
    assert encoded == (
        b"<row><payload>AP8=</payload><decimal>123.4500</decimal><date>2026-08-15</date>"
        b"<datetime>2026-08-15T12:03:04.000005Z</datetime><flag>true</flag>"
        b"<count>3</count><ratio>1.5</ratio></row>"
    )
    # XML proves text and nothing else.
    assert xml.loads(encoded) == {
        "row": {
            "count": "3",
            "date": "2026-08-15",
            "datetime": "2026-08-15T12:03:04.000005Z",
            "decimal": "123.4500",
            "flag": "true",
            "payload": "AP8=",
            "ratio": "1.5",
        }
    }


def test_a_field_types_the_document_element_and_a_class_is_that_field() -> None:
    value = Order(
        7,
        Fill(Decimal("12.50"), dt.datetime(2026, 8, 15, 8, tzinfo=dt.timezone.utc)),
        ("urgent", "auction"),
    )
    encoded = xml.dumps({"order": value})
    assert encoded == (
        b"<order><fill><observed_at>2026-08-15T08:00:00.000000Z</observed_at>"
        b"<price>12.50</price></fill><labels>urgent</labels><labels>auction</labels>"
        b"<order_id>7</order_id></order>"
    )
    assert xml.loads(encoded, cls=Order) == value
    assert xml.loads(encoded, cls=Order, field=Order) == value
    # The rule is the format's, not the module's: the generic door reads the
    # same way under every spelling of the format.
    assert codec.from_io(encoded, format="xml", cls=Order) == value
    assert codec.from_io(encoded, format="application/xml", cls=Order) == value
    assert xml.loads(encoded, field=Order) == {
        "fill": {"observed_at": value.fill.observed_at, "price": Decimal("12.50")},
        "labels": ["urgent", "auction"],
        "order_id": 7,
    }

    # One repeated element read once is one item; none is the empty sequence.
    one = xml.loads(b"<o><order_id>1</order_id><fill><price>1</price><observed_at>2026-08-15T08:00:00Z</observed_at></fill><labels>x</labels></o>", cls=Order)
    assert one.labels == ("x",)
    none = xml.loads(b"<o><order_id>1</order_id><fill><price>1</price><observed_at>2026-08-15T08:00:00Z</observed_at></fill></o>", cls=Order)
    assert none.labels == ()

    field = Field("row", "struct<amount: decimal128(8, 2) not null, note: utf8>", nullable=False)
    assert xml.loads("<row><amount> 12.50 </amount><note/></row>", field=field) == {
        "amount": Decimal("12.50"),
        "note": None,
    }
    exact = xml.loads("<row><amount>12.50</amount><note>x</note></row>", field=field, cls=Scalar)
    assert isinstance(exact, Scalar)
    assert exact.at(0) == DataType("decimal128(8, 2)").scalar("12.50")


def test_dataclass_reconstruction_requires_an_explicit_target() -> None:
    @dataclasses.dataclass
    class Point:
        x: int
        y: int

    value = Point(2, 3)
    encoded = xml.dumps({"point": value})

    assert b"PYTHON:" not in encoded
    assert xml.loads(encoded) == {"point": {"x": "2", "y": "3"}}
    assert xml.loads(encoded, cls=Point) == value


def test_source_intent_is_type_driven_for_text_binary_and_paths(
    tmp_path: pathlib.Path,
) -> None:
    path = tmp_path / "value.xml"
    path.write_bytes(b"<v>42</v>")

    assert xml.loads(path) == {"v": "42"}
    with pytest.raises(ValueError, match="root element"):
        xml.loads(str(path))
    assert xml.loads("<v>43</v>") == {"v": "43"}
    assert xml.loads(bytearray(b"<v>43</v>")) == {"v": "43"}
    assert xml.loads(memoryview(b"<v>43</v>")) == {"v": "43"}
    binary = io.BytesIO(b"<v>44</v>")
    text = io.StringIO("<v>45</v>")
    assert xml.loads(binary) == {"v": "44"}
    assert xml.loads(text) == {"v": "45"}
    assert not binary.closed
    assert not text.closed


def test_dump_writes_bytes_utf8_paths_and_writers(tmp_path: pathlib.Path) -> None:
    value = {"v": {"a": 1}}
    assert xml.dump(value) == b"<v><a>1</a></v>"
    assert xml.dump(value, utf8=True) == "<v><a>1</a></v>"

    path = tmp_path / "value.xml"
    assert xml.dump(value, path) is None
    assert path.read_bytes() == b"<v><a>1</a></v>"
    assert xml.loads(path) == {"v": {"a": "1"}}

    binary = io.BytesIO()
    assert xml.dump(value, binary) is None
    assert binary.getvalue() == b"<v><a>1</a></v>"
    text = io.StringIO()
    assert xml.dump(value, text, indent=2) is None
    assert text.getvalue() == "<v>\n  <a>1</a>\n</v>"


def test_a_refused_value_does_not_truncate_an_existing_path(
    tmp_path: pathlib.Path,
) -> None:
    path = tmp_path / "value.xml"
    path.write_bytes(b"<keep/>")
    with pytest.raises(ValueError, match="sequence inside a sequence"):
        xml.dump({"a": {"m": [[1]]}}, path)
    assert path.read_bytes() == b"<keep/>"


def test_malformed_documents_and_limits_are_refused_by_name() -> None:
    with pytest.raises(ValueError, match="expected `</b>`"):
        xml.loads("<a><b></a>")
    with pytest.raises(ValueError, match="never closed"):
        xml.loads("<a><b>")
    with pytest.raises(ValueError, match="after the root element"):
        xml.loads("<a/><b/>")
    with pytest.raises(ValueError, match="&custom;"):
        xml.loads("<a>&custom;</a>")
    with pytest.raises(ValueError, match="depth"):
        xml.loads("<a><b><c/></b></a>", max_depth=2)
    with pytest.raises(ValueError, match="node"):
        xml.loads("<a><b/><c/><d/></a>", max_nodes=2)
    with pytest.raises(ValueError, match="byte"):
        xml.loads("<a>text</a>", max_input_bytes=4)


def test_placeholders_resolve_in_a_document() -> None:
    assert xml.loads(
        "<cfg><port>{{ PORT }}</port><path>{{ ROOT }}/app</path></cfg>",
        placeholders={"PORT": 8080, "ROOT": "/srv"},
    ) == {"cfg": {"port": 8080, "path": "/srv/app"}}
    assert xml.loads("<a>{{ MISSING }}</a>")["a"] == "{{ MISSING }}"


def test_generic_codec_infers_xml_from_content_and_suffix(
    tmp_path: pathlib.Path,
) -> None:
    assert codec.from_io(b"<v>1</v>") == {"v": "1"}
    target = tmp_path / "value.xml"
    codec.into_io({"v": {"a": 1}}, target)
    assert target.read_bytes() == b"<v><a>1</a></v>"
    assert codec.from_io(target) == {"v": {"a": "1"}}
    assert codec.into_io({"v": 1}, format="xml") == b"<v>1</v>"
    for name in ("xml", "application/xml", "text/xml", ".xml"):
        assert codec.from_io(b"<v>1</v>", format=name) == {"v": "1"}
    # The generic facade writes one value: a list is one sequence, and a
    # sequence is not a document element.
    with pytest.raises(ValueError, match="document element"):
        codec.into_io([{"a": 1}], format="xml")
