"""The Python view of the XML codecs.

The boundary owns no XML model, so these assert that every spelling a caller
reaches for arrives at the same native answer, and that what the core refuses
the binding refuses too.
"""

from __future__ import annotations

import pytest

from yggdryl import Field, RecordOptions
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
