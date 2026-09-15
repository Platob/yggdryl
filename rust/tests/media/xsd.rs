//! An XML Schema document as the field it declares.
//!
//! The fixtures are spelled here rather than borrowed from the codec: a schema
//! built by the reader under test proves only that the two agree. The shapes
//! come from real documents - an XMLA rowset carries its schema inline, which
//! is exactly the case this exists for.

use yggdryl::media::xml;
use yggdryl::types::protocol::XmlKind;
use yggdryl::{DataType, Field, Limits, TimeUnit};

fn field_of(schema: &str, root: Option<&str>) -> yggdryl::Result<Field> {
    xml::field_from_xsd(schema.as_bytes(), Limits::default(), root)
}

fn child<'a>(field: &'a Field, name: &str) -> &'a Field {
    field
        .dtype()
        .as_fields()
        .unwrap()
        .iter()
        .find(|child| child.name() == name)
        .unwrap_or_else(|| panic!("no column {name}"))
}

/// The shape an XMLA rowset response carries inline, cut down to its edges.
const ROWSET: &str = r#"
<xsd:schema xmlns:xsd="http://www.w3.org/2001/XMLSchema"
            targetNamespace="urn:schemas-microsoft-com:xml-analysis:rowset"
            elementFormDefault="qualified">
  <xsd:element name="row">
    <xsd:complexType>
      <xsd:sequence>
        <xsd:element name="CATALOG_NAME" type="xsd:string" minOccurs="0"/>
        <xsd:element name="TABLE_ROWS" type="xsd:unsignedLong"/>
        <xsd:element name="COLUMN_SIZE" type="xsd:unsignedInt"/>
        <xsd:element name="ORDINAL" type="xsd:int"/>
        <xsd:element name="IS_NULLABLE" type="xsd:boolean"/>
        <xsd:element name="RATIO" type="xsd:double"/>
        <xsd:element name="PROVIDER_TYPE" type="xsd:string" maxOccurs="unbounded"/>
      </xsd:sequence>
    </xsd:complexType>
  </xsd:element>
</xsd:schema>
"#;

#[test]
fn a_rowset_schema_types_every_column_it_declares() {
    let field = field_of(ROWSET, None).unwrap();
    assert_eq!(field.name(), "row");
    assert!(!field.is_nullable());

    // The eight bounded integers are the only XSD integers with a width, and
    // unsigned ones are common in a real rowset.
    assert_eq!(child(&field, "TABLE_ROWS").dtype(), &DataType::UInt64);
    assert_eq!(child(&field, "COLUMN_SIZE").dtype(), &DataType::UInt32);
    assert_eq!(child(&field, "ORDINAL").dtype(), &DataType::Int32);
    assert_eq!(child(&field, "IS_NULLABLE").dtype(), &DataType::Boolean);
    assert_eq!(child(&field, "RATIO").dtype(), &DataType::Float64);

    // `minOccurs="0"` is the column saying a row may not carry it.
    assert!(child(&field, "CATALOG_NAME").is_nullable());
    assert!(!child(&field, "TABLE_ROWS").is_nullable());

    // Even a flat rowset repeats a column, so a list is not an edge case.
    assert!(matches!(
        child(&field, "PROVIDER_TYPE").dtype(),
        DataType::List(_)
    ));
}

#[test]
fn a_decimal_is_one_only_when_the_schema_says_which_one() {
    // Both digit facets name a precision and a scale; anything less is a value
    // space no fixed-width decimal holds.
    let bounded = field_of(
        r#"<xsd:schema xmlns:xsd="http://www.w3.org/2001/XMLSchema">
             <xsd:element name="r"><xsd:complexType><xsd:sequence>
               <xsd:element name="amount">
                 <xsd:simpleType><xsd:restriction base="xsd:decimal">
                   <xsd:totalDigits value="18"/><xsd:fractionDigits value="4"/>
                 </xsd:restriction></xsd:simpleType>
               </xsd:element>
             </xsd:sequence></xsd:complexType></xsd:element>
           </xsd:schema>"#,
        None,
    )
    .unwrap();
    assert_eq!(
        child(&bounded, "amount").dtype(),
        &DataType::decimal64(18, 4).unwrap()
    );

    let unbounded = field_of(
        r#"<xsd:schema xmlns:xsd="http://www.w3.org/2001/XMLSchema">
             <xsd:element name="r"><xsd:complexType><xsd:sequence>
               <xsd:element name="amount" type="xsd:decimal"/>
               <xsd:element name="count" type="xsd:integer"/>
             </xsd:sequence></xsd:complexType></xsd:element>
           </xsd:schema>"#,
        None,
    )
    .unwrap();
    // Text, and the schema's own name for it is what says the text was a
    // number rather than a string.
    let amount = child(&unbounded, "amount");
    assert!(matches!(amount.dtype(), DataType::String(_)));
    assert_eq!(amount.as_xml().declared_type(), Some("xsd:decimal"));
    assert_eq!(
        child(&unbounded, "count").as_xml().declared_type(),
        Some("xsd:integer")
    );
}

#[test]
fn a_temporal_is_an_instant_only_when_its_offset_is_settled() {
    let field = field_of(
        r#"<xsd:schema xmlns:xsd="http://www.w3.org/2001/XMLSchema">
             <xsd:element name="r"><xsd:complexType><xsd:sequence>
               <xsd:element name="loose" type="xsd:dateTime"/>
               <xsd:element name="stamped" type="xsd:dateTimeStamp"/>
               <xsd:element name="naive">
                 <xsd:simpleType><xsd:restriction base="xsd:dateTime">
                   <xsd:explicitTimezone value="prohibited"/>
                 </xsd:restriction></xsd:simpleType>
               </xsd:element>
               <xsd:element name="year" type="xsd:gYear"/>
             </xsd:sequence></xsd:complexType></xsd:element>
           </xsd:schema>"#,
        None,
    )
    .unwrap();

    // An optional offset is two different columns, so it stays text.
    assert!(matches!(
        child(&field, "loose").dtype(),
        DataType::String(_)
    ));
    // Required by definition.
    assert_eq!(
        child(&field, "stamped").dtype(),
        &DataType::datetime64(TimeUnit::Microsecond, yggdryl::Timezone::UTC).unwrap()
    );
    // Prohibited, so naive.
    assert!(matches!(
        child(&field, "naive").dtype(),
        DataType::DateTime64 { .. }
    ));
    // A partial calendar value is not an instant: reading it as one would
    // invent a month and a day.
    assert!(matches!(child(&field, "year").dtype(), DataType::String(_)));
}

#[test]
fn an_attribute_a_schema_declares_is_a_column_that_knows_it_is_one() {
    let field = field_of(
        r#"<xsd:schema xmlns:xsd="http://www.w3.org/2001/XMLSchema">
             <xsd:element name="Amt">
               <xsd:complexType>
                 <xsd:sequence><xsd:element name="Sub" type="xsd:int"/></xsd:sequence>
                 <xsd:attribute name="Ccy" type="xsd:string" use="required"/>
                 <xsd:attribute name="Note" type="xsd:string"/>
               </xsd:complexType>
             </xsd:element>
           </xsd:schema>"#,
        None,
    )
    .unwrap();
    let ccy = child(&field, "Ccy");
    assert_eq!(ccy.as_xml().kind().unwrap(), XmlKind::Attribute);
    assert!(!ccy.is_nullable(), "use=required is a column that is there");
    assert!(child(&field, "Note").is_nullable());
    assert_eq!(
        child(&field, "Sub").as_xml().kind().unwrap(),
        XmlKind::Element
    );
}

#[test]
fn a_type_no_column_can_hold_is_refused_by_name() {
    for (declared, expected) in [
        ("xsd:error", "empty value space"),
        ("xsd:NOTATION", "only what the schema"),
    ] {
        let schema = format!(
            r#"<xsd:schema xmlns:xsd="http://www.w3.org/2001/XMLSchema">
                 <xsd:element name="r"><xsd:complexType><xsd:sequence>
                   <xsd:element name="c" type="{declared}"/>
                 </xsd:sequence></xsd:complexType></xsd:element>
               </xsd:schema>"#
        );
        let error = field_of(&schema, None).unwrap_err().to_string();
        assert!(error.contains(expected), "{declared} gave {error}");
    }

    // Mixed content and a wildcard are shapes a row has no cell for.
    for (schema, expected) in [
        (
            r#"<xsd:schema xmlns:xsd="http://www.w3.org/2001/XMLSchema">
                 <xsd:element name="r"><xsd:complexType mixed="true"><xsd:sequence>
                   <xsd:element name="c" type="xsd:string"/>
                 </xsd:sequence></xsd:complexType></xsd:element>
               </xsd:schema>"#,
            "declared mixed",
        ),
        (
            r#"<xsd:schema xmlns:xsd="http://www.w3.org/2001/XMLSchema">
                 <xsd:element name="r"><xsd:complexType><xsd:sequence>
                   <xsd:any/>
                 </xsd:sequence></xsd:complexType></xsd:element>
               </xsd:schema>"#,
            "xs:any",
        ),
    ] {
        let error = field_of(schema, None).unwrap_err().to_string();
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn a_schema_declaring_several_roots_asks_which_one() {
    let schema = r#"<xsd:schema xmlns:xsd="http://www.w3.org/2001/XMLSchema">
        <xsd:element name="a"><xsd:complexType><xsd:sequence>
          <xsd:element name="x" type="xsd:int"/></xsd:sequence></xsd:complexType></xsd:element>
        <xsd:element name="b"><xsd:complexType><xsd:sequence>
          <xsd:element name="y" type="xsd:int"/></xsd:sequence></xsd:complexType></xsd:element>
      </xsd:schema>"#;
    let error = field_of(schema, None).unwrap_err().to_string();
    assert!(error.contains("name the one to read"), "{error}");
    assert_eq!(field_of(schema, Some("b")).unwrap().name(), "b");
    let missing = field_of(schema, Some("c")).unwrap_err().to_string();
    assert!(missing.contains("does not"), "{missing}");
}

#[test]
fn a_named_type_declared_in_the_document_resolves_to_its_own_shape() {
    let field = field_of(
        r#"<xsd:schema xmlns:xsd="http://www.w3.org/2001/XMLSchema">
             <xsd:simpleType name="uuid">
               <xsd:restriction base="xsd:string"/>
             </xsd:simpleType>
             <xsd:complexType name="leg">
               <xsd:sequence><xsd:element name="notional" type="xsd:long"/></xsd:sequence>
             </xsd:complexType>
             <xsd:element name="Trade"><xsd:complexType><xsd:sequence>
               <xsd:element name="id" type="uuid"/>
               <xsd:element name="leg" type="leg" maxOccurs="unbounded"/>
             </xsd:sequence></xsd:complexType></xsd:element>
           </xsd:schema>"#,
        None,
    )
    .unwrap();
    assert!(matches!(child(&field, "id").dtype(), DataType::String(_)));
    let leg = child(&field, "leg");
    let DataType::List(item) = leg.dtype() else {
        panic!("expected a list of legs, got {:?}", leg.dtype());
    };
    assert!(matches!(item.dtype(), DataType::Struct(_)));
}

#[test]
fn a_document_the_schema_describes_reads_under_the_field_it_answers() {
    use yggdryl::holder::Buffer;
    use yggdryl::media::IORecordOptions;
    use yggdryl::{IOBase, IOMedia, Url};

    // The point of the whole thing: a schema produces a field, and the
    // declared-field path every read already has does the rest.
    let field = field_of(ROWSET, None).unwrap();
    let mut handle =
        Buffer::new().with_media_type(Url::from_str("file:///rowset.xml").unwrap().media_type());
    handle
        .write_all_bytes(
            b"<root><row><CATALOG_NAME>db</CATALOG_NAME><TABLE_ROWS>42</TABLE_ROWS>\
              <COLUMN_SIZE>8</COLUMN_SIZE><ORDINAL>1</ORDINAL><IS_NULLABLE>true</IS_NULLABLE>\
              <RATIO>0.5</RATIO><PROVIDER_TYPE>TDP</PROVIDER_TYPE>\
              <PROVIDER_TYPE>MDP</PROVIDER_TYPE></row></root>",
        )
        .unwrap();
    let options = handle.record_options().unwrap().with_field(field);
    let batches: Vec<_> = handle
        .read_arrow_reader(&options)
        .unwrap()
        .map(|batch| batch.unwrap())
        .collect();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].num_rows(), 1);
}

#[test]
fn a_schema_this_read_writes_back_to_the_same_field() {
    // The property that makes both directions worth having: a field read from
    // a schema, written as a schema, and read again is the same field. The
    // columns that had to travel as text remember what they were, so
    // `xsd:decimal` comes back as `xsd:decimal` rather than as a string.
    let field = field_of(ROWSET, None).unwrap();
    let written = xml::field_into_xsd(&field, yggdryl::text::Formatting::new()).unwrap();
    let again = xml::field_from_xsd(&written, Limits::default(), None).unwrap();
    assert_eq!(again, field, "{}", String::from_utf8_lossy(&written));

    let typed = field_of(
        r#"<xsd:schema xmlns:xsd="http://www.w3.org/2001/XMLSchema">
             <xsd:element name="r"><xsd:complexType>
               <xsd:sequence>
                 <xsd:element name="amount" type="xsd:decimal"/>
                 <xsd:element name="when" type="xsd:dateTimeStamp"/>
               </xsd:sequence>
               <xsd:attribute name="Ccy" type="xsd:string" use="required"/>
             </xsd:complexType></xsd:element>
           </xsd:schema>"#,
        None,
    )
    .unwrap();
    let written = xml::field_into_xsd(&typed, yggdryl::text::Formatting::new()).unwrap();
    let rendered = String::from_utf8_lossy(&written).into_owned();
    assert!(rendered.contains(r#"type="xsd:decimal""#), "{rendered}");
    assert!(
        rendered.contains(r#"<xsd:attribute name="Ccy""#)
            || rendered.contains(r#"<xs:attribute name="Ccy""#),
        "{rendered}"
    );
    assert_eq!(
        xml::field_from_xsd(&written, Limits::default(), None).unwrap(),
        typed,
        "{rendered}"
    );
}
