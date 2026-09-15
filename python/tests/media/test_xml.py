"""The Python view of the XML codecs.

The boundary owns no XML model, so these assert that every spelling a caller
reaches for arrives at the same native answer, and that what the core refuses
the binding refuses too.
"""

from __future__ import annotations

import pickle
from pathlib import Path

import pytest

from yggdryl import Field, IOBase, RecordOptions
from yggdryl.media import xml

ROWSET_SCHEMA = b"""<xsd:schema xmlns:xsd="http://www.w3.org/2001/XMLSchema">
  <xsd:element name="row">
    <xsd:complexType>
      <xsd:sequence>
        <xsd:element name="size" type="xsd:unsignedInt"/>
        <xsd:element name="amount" type="xsd:decimal"/>
        <xsd:element minOccurs="0" name="note" type="xsd:string"/>
      </xsd:sequence>
      <xsd:attribute name="Ccy" type="xsd:string" use="required"/>
    </xsd:complexType>
  </xsd:element>
</xsd:schema>"""


def test_a_document_reads_as_the_value_it_states() -> None:
    # Attributes and child elements are one namespace of column names, and an
    # element carrying both attributes and characters keeps the characters.
    assert xml.loads('<Amt Ccy="EUR">9.50</Amt>') == {"Ccy": "EUR", "value": "9.50"}
    assert xml.loads("<a>text</a>") == "text"
    assert xml.loads("<a/>") == ""
    assert xml.loads("<r><b>1</b><b>2</b></r>") == {"b": ["1", "2"]}


@pytest.mark.parametrize(
    "document",
    [
        "<r><a>1</a></r>",
        b"<r><a>1</a></r>",
        bytearray(b"<r><a>1</a></r>"),
        memoryview(b"<r><a>1</a></r>"),
    ],
)
def test_every_spelling_of_the_input_reads_the_same(document: object) -> None:
    assert xml.loads(document) == {"a": "1"}


def test_a_value_writes_and_reads_back() -> None:
    document = xml.dumps({"symbol": "AAPL", "size": "100"}, "Order")
    assert b"<symbol>AAPL</symbol>" in document
    assert xml.loads(document) == {"symbol": "AAPL", "size": "100"}


def test_the_root_name_is_an_argument_because_xml_has_no_anonymous_document() -> None:
    assert b"<Trade>" in xml.dumps({"id": "1"}, "Trade")
    with pytest.raises(ValueError):
        xml.dumps({"id": "1"}, "not a name")


def test_a_schema_answers_the_field_it_declares() -> None:
    field = xml.schema(ROWSET_SCHEMA)
    assert isinstance(field, Field)
    assert field.name == "row"

    children = {child.name: child for child in field.explode_fields()}
    # The eight bounded integers are the only XSD integers with a width.
    assert str(children["size"].dtype) == "uint32"
    # An unfaceted decimal is unbounded, so it travels as text and says what
    # the schema called it.
    assert str(children["amount"].dtype) == "utf8"
    assert children["amount"].metadata["xml:type"] == "xsd:decimal"
    # minOccurs="0" is nullable; use="required" is not.
    assert children["note"].nullable
    assert not children["Ccy"].nullable
    assert children["Ccy"].metadata["xml:kind"] == "attribute"


def test_a_schema_written_back_reads_to_the_same_field() -> None:
    field = xml.schema(ROWSET_SCHEMA)
    assert xml.schema(xml.schema_dumps(field)) == field


def test_a_declared_field_types_the_text_a_document_carries() -> None:
    field = xml.schema(ROWSET_SCHEMA)
    value = xml.loads_with_field(
        '<row Ccy="EUR"><size>42</size><amount>1.5</amount></row>', field
    )
    # The row canonicalizes to an ordered sequence under its field.
    # A row canonicalizes to an ordered sequence in the field's own child
    # order: the attribute the schema declared, then the sequence it named.
    assert value.as_py() == ["EUR", 42, "1.5", None]


def test_a_handle_named_xml_answers_the_record_surface() -> None:
    options = RecordOptions("trades.xml")
    assert str(options.mime_type) == "application/xml"


def test_a_write_takes_the_layout_every_other_codec_takes() -> None:
    value = {"symbol": "AAPL"}
    # Omitted is XML's own readable layout; None puts it all on one line.
    assert b"\n  <symbol>" in xml.dumps(value, "Order")
    assert b"\n" not in xml.dumps(value, "Order", indent=None).removeprefix(
        b'<?xml version="1.0" encoding="UTF-8"?>'
    )
    assert b"\n    <symbol>" in xml.dumps(value, "Order", indent=4)
    assert b"\n\t<symbol>" in xml.dumps(value, "Order", indent="\t")
    assert b"?><xs:schema" in xml.schema_dumps(
        xml.schema(ROWSET_SCHEMA), indent=None
    )
    with pytest.raises(TypeError):
        xml.dumps(value, "Order", indent="wide")


def test_an_xml_record_read_is_configured_through_its_options() -> None:
    options = RecordOptions("feed.xml")

    assert options.document == "rows"
    assert options.row_element is None
    assert options.max_nodes == RecordOptions("other.xml").max_nodes

    options.document = "channel"
    options.row_element = "item"
    options.max_nodes = 1_000

    assert options.document == "channel"
    assert options.row_element == "item"
    assert options.max_nodes == 1_000
    # One bound of the budget is set without resetting the others.
    assert options.max_input_bytes == RecordOptions("other.xml").max_input_bytes

    options.row_element = None
    assert options.row_element is None

    # A name no element can be called is refused where it was set.
    with pytest.raises(ValueError, match="an XML element can be called"):
        options.document = "1st"
    assert options.document == "channel"


def test_a_setting_one_encoding_has_is_absent_on_the_others() -> None:
    options = RecordOptions("trades.avro")

    assert options.document is None
    assert options.row_element is None
    assert options.max_nodes is None
    with pytest.raises(ValueError, match="expected XML options"):
        options.row_element = "item"


def test_a_named_row_element_reads_a_wrapper_holding_more_than_rows(
    tmp_path: Path,
) -> None:
    feed = tmp_path / "feed.xml"
    feed.write_bytes(
        b"<rss><channel><title>Example</title>"
        b"<item><guid>1</guid></item><item><guid>2</guid></item>"
        b"</channel></rss>"
    )
    handle = IOBase(feed)

    # The wrapper is the row when nothing names one: one channel, holding a
    # title beside the items it nests.
    assert handle.read_arrow_reader().read_all().num_rows == 1

    rows = handle.read_arrow_reader(row_element="item").read_all()
    assert rows.num_rows == 2
    assert rows.column_names == ["guid"]


def test_the_xml_options_survive_a_pickle_round_trip() -> None:
    options = RecordOptions("feed.xml")
    options.document = "channel"
    options.row_element = "item"
    options.max_nodes = 1_000

    restored = pickle.loads(pickle.dumps(options))
    assert restored.document == "channel"
    assert restored.row_element == "item"
    assert restored.max_nodes == 1_000
    assert restored == options


def test_what_the_core_refuses_the_binding_refuses() -> None:
    # Mixed content is the one shape a row has no cell for.
    with pytest.raises(ValueError):
        xml.loads("<r>text<c/></r>")
    # A prefix nothing declared would fold two vocabularies into one column.
    with pytest.raises(ValueError):
        xml.loads("<p:a/>")
    # XML cannot carry these, and no reference can spell them either.
    with pytest.raises(ValueError):
        xml.loads("<a>&#0;</a>")
    with pytest.raises(TypeError):
        xml.loads(object())


def test_the_decode_budget_is_reachable_from_python() -> None:
    deep = "<a>" * 40 + "</a>" * 40
    with pytest.raises(ValueError):
        xml.loads(deep, max_depth=8)
    with pytest.raises(ValueError):
        xml.loads("<r><a>1</a></r>", max_input_bytes=4)
