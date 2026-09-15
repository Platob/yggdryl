//! Documents from the wild, read as rows.
//!
//! Every fixture here is the shape of a real document - a SOAP envelope, an
//! XMLA rowset carrying its own schema, an ISO 20022 amount, a syndication
//! feed - cut down to the edge it exercises. They are spelled here rather than
//! generated, because a fixture built by the codec under test proves only that
//! the codec agrees with itself.

use yggdryl::holder::Buffer;
use yggdryl::media::{IORecordOptions, RecordOptions, xml};
use yggdryl::types::protocol::XmlKind;
use yggdryl::{DataType, IOBase, IOMedia, Limits, Scalar, Url};

fn handle_of(name: &str, document: &str) -> Buffer {
    let mut handle = Buffer::new().with_media_type(
        Url::from_str(&format!("file:///{name}"))
            .unwrap()
            .media_type(),
    );
    handle.write_all_bytes(document.as_bytes()).unwrap();
    handle
}

fn rows_named(handle: &Buffer, row: &str) -> Vec<arrow_array::RecordBatch> {
    let mut options = handle.record_options().unwrap();
    let RecordOptions::Xml(xml) = &mut options else {
        panic!("expected XML options");
    };
    xml.row_element = Some(row.into());
    handle
        .read_arrow_reader(&options)
        .unwrap()
        .map(|batch| batch.unwrap())
        .collect()
}

/// An XMLA rowset response: the schema travels inside the document that uses
/// it, which is the case the XSD reader exists for.
const XMLA_ROWSET: &str = r#"<root xmlns="urn:schemas-microsoft-com:xml-analysis:rowset"
      xmlns:xsd="http://www.w3.org/2001/XMLSchema"
      xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <xsd:schema targetNamespace="urn:schemas-microsoft-com:xml-analysis:rowset"
              elementFormDefault="qualified">
    <xsd:element name="row">
      <xsd:complexType>
        <xsd:sequence>
          <xsd:element minOccurs="0" name="CATALOG_NAME" type="xsd:string"/>
          <xsd:element name="COLUMN_SIZE" type="xsd:unsignedInt"/>
          <xsd:element minOccurs="0" maxOccurs="unbounded" name="ProviderType" type="xsd:string"/>
        </xsd:sequence>
      </xsd:complexType>
    </xsd:element>
  </xsd:schema>
  <row>
    <CATALOG_NAME>FoodMart</CATALOG_NAME>
    <COLUMN_SIZE>255</COLUMN_SIZE>
    <ProviderType>TDP</ProviderType>
    <ProviderType>MDP</ProviderType>
  </row>
  <row>
    <COLUMN_SIZE>8</COLUMN_SIZE>
  </row>
</root>"#;

#[test]
fn an_xmla_rowset_is_read_under_the_schema_it_carries() {
    // The schema is an ordinary element of the document, so the row element
    // has to be named: `<root>` holds a schema and then rows.
    let handle = handle_of("rowset.xml", XMLA_ROWSET);

    // Pull the inline schema out and let it type the rows. This is the whole
    // point: no inference, no guessing, the document's own declaration.
    let start = XMLA_ROWSET.find("<xsd:schema").unwrap();
    let end = XMLA_ROWSET.find("</xsd:schema>").unwrap() + "</xsd:schema>".len();
    let schema = format!(
        "<xsd:schema xmlns:xsd=\"http://www.w3.org/2001/XMLSchema\"{}",
        &XMLA_ROWSET[start + "<xsd:schema".len()..end]
    );
    let field = xml::field_from_xsd(schema.as_bytes(), Limits::default(), None).unwrap();

    let mut options = handle.record_options().unwrap().with_field(field.clone());
    let RecordOptions::Xml(settings) = &mut options else {
        panic!("expected XML options");
    };
    settings.row_element = Some("row".into());

    let batches: Vec<_> = handle
        .read_arrow_reader(&options)
        .unwrap()
        .map(|batch| batch.unwrap())
        .collect();
    let rows: usize = batches.iter().map(arrow_array::RecordBatch::num_rows).sum();
    assert_eq!(rows, 2);

    // The schema's types are what the columns are, including the unsigned
    // integer a rowset uses constantly and the column that repeats.
    let children = field.dtype().as_fields().unwrap();
    let size = children.iter().find(|c| c.name() == "COLUMN_SIZE").unwrap();
    assert_eq!(size.dtype(), &DataType::UInt32);
    let provider = children
        .iter()
        .find(|c| c.name() == "ProviderType")
        .unwrap();
    assert!(matches!(provider.dtype(), DataType::List(_)));
}

#[test]
fn a_soap_envelope_reads_whatever_prefix_the_server_chose() {
    // Clients write the envelope with a default namespace and no prefix;
    // servers reply with SOAP-ENV:, soapenv:, soap: or m: for the same
    // elements. A reader keyed on the prefix passes against one server and
    // fails against the next, so the namespace is what is matched.
    for document in [
        r#"<Envelope xmlns="http://schemas.xmlsoap.org/soap/envelope/">
             <Header/>
             <Body><Item><id>1</id></Item><Item><id>2</id></Item></Body>
           </Envelope>"#,
        r#"<SOAP-ENV:Envelope xmlns:SOAP-ENV="http://schemas.xmlsoap.org/soap/envelope/">
             <SOAP-ENV:Header/>
             <SOAP-ENV:Body><Item><id>1</id></Item><Item><id>2</id></Item></SOAP-ENV:Body>
           </SOAP-ENV:Envelope>"#,
    ] {
        let handle = handle_of("soap.xml", document);
        let batches = rows_named(&handle, "Item");
        let rows: usize = batches.iter().map(arrow_array::RecordBatch::num_rows).sum();
        assert_eq!(rows, 2, "{document}");
    }
}

#[test]
fn a_soap_fault_is_a_document_like_any_other() {
    // The error path is still XML: an unprefixed faultcode inside a prefixed
    // Fault, and a detail whose payload is attributes only.
    let document = r#"<SOAP-ENV:Envelope xmlns:SOAP-ENV="http://schemas.xmlsoap.org/soap/envelope/">
      <SOAP-ENV:Body>
        <SOAP-ENV:Fault>
          <faultcode>XMLAnalysisError.0x80000005</faultcode>
          <faultstring>The provider encountered an error.</faultstring>
          <detail>
            <Error ErrorCode="2147483653" Description="An unexpected error." Source="Provider"/>
          </detail>
        </SOAP-ENV:Fault>
      </SOAP-ENV:Body>
    </SOAP-ENV:Envelope>"#;
    let value = xml::from_utf8(document).unwrap();
    // The walk reaches it wherever it sits; nothing here needs a row model.
    assert!(
        format!("{value:?}").contains("XMLAnalysisError"),
        "{value:?}"
    );
}

#[test]
fn an_iso_20022_amount_keeps_its_currency_and_its_number() {
    // `<InstdAmt Ccy="EUR">1234.56</InstdAmt>` is the shape a payment message
    // is made of: the currency is an attribute and the amount is the text.
    // Losing either would make the message meaningless.
    let document = r#"<Document xmlns="urn:iso:std:iso:20022:tech:xsd:pain.001.001.03">
      <CdtTrfTxInf>
        <PmtId><EndToEndId>E2E-1</EndToEndId></PmtId>
        <Amt><InstdAmt Ccy="EUR">1234.56</InstdAmt></Amt>
      </CdtTrfTxInf>
      <CdtTrfTxInf>
        <PmtId><EndToEndId>E2E-2</EndToEndId></PmtId>
        <Amt><InstdAmt Ccy="USD">99.00</InstdAmt></Amt>
      </CdtTrfTxInf>
    </Document>"#;
    let value = xml::from_utf8(document).unwrap();
    let first = value
        .get_key_str("CdtTrfTxInf")
        .and_then(|txs| txs.as_sequence().map(|rows| rows[0].clone()))
        .unwrap();
    let amount = first
        .get_key_str("Amt")
        .and_then(|amt| amt.get_key_str("InstdAmt"))
        .unwrap()
        .clone();
    assert_eq!(
        amount.get_key_str("Ccy").and_then(Scalar::as_str),
        Some("EUR")
    );
    assert_eq!(
        amount.get_key_str("value").and_then(Scalar::as_str),
        Some("1234.56")
    );
}

#[test]
fn a_syndication_feed_is_a_table_of_its_entries() {
    // The classic repeated-element rowset, and the wrapper carries metadata
    // that is not a row - which is what naming the row element is for.
    let document = r#"<rss version="2.0">
      <channel>
        <title>Example</title>
        <item><title>First</title><guid>1</guid></item>
        <item><title>Second</title><guid>2</guid></item>
        <item><title>Third</title><guid>3</guid></item>
      </channel>
    </rss>"#;
    let handle = handle_of("feed.xml", document);
    let batches = rows_named(&handle, "item");
    let rows: usize = batches.iter().map(arrow_array::RecordBatch::num_rows).sum();
    assert_eq!(rows, 3);
}

#[test]
fn a_document_that_is_prose_is_refused_rather_than_flattened() {
    // XHTML and SVG are mixed content everywhere. A row model has no cell for
    // characters beside elements, and saying so is better than inventing one.
    for document in [
        r#"<p xmlns="http://www.w3.org/1999/xhtml">Hello <b>world</b> and more</p>"#,
        r#"<text xmlns="http://www.w3.org/2000/svg">Label <tspan>x</tspan></text>"#,
    ] {
        let error = xml::from_utf8(document).unwrap_err().to_string();
        assert!(error.contains("got both"), "{document} gave {error}");
    }
}

#[test]
fn an_xsi_nil_column_is_absent_rather_than_empty() {
    let document = r#"<rows xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
      <r><a>1</a><b xsi:nil="true"/></r>
      <r><a>2</a><b></b></r>
    </rows>"#;
    let value = xml::from_utf8(document).unwrap();
    let rows = value
        .get_key_str("r")
        .unwrap()
        .as_sequence()
        .unwrap()
        .to_vec();
    // Nil is absence; an empty element is the empty string. They are different
    // facts and a reader that folded them would lose one.
    assert_eq!(rows[0].get_key_str("b"), Some(&Scalar::Null));
    assert_eq!(rows[1].get_key_str("b").and_then(Scalar::as_str), Some(""));
}

#[test]
fn a_schema_with_a_target_namespace_says_so_on_the_field_it_answers() {
    // What a future XMLA or Excel layer keys on: the namespace travels with
    // the field rather than being re-derived from the document each time.
    let schema = r#"<xsd:schema xmlns:xsd="http://www.w3.org/2001/XMLSchema"
        targetNamespace="urn:schemas-microsoft-com:xml-analysis:rowset"
        elementFormDefault="qualified">
      <xsd:element name="row"><xsd:complexType><xsd:sequence>
        <xsd:element name="c" type="xsd:string"/>
      </xsd:sequence></xsd:complexType></xsd:element>
    </xsd:schema>"#;
    let field = xml::field_from_xsd(schema.as_bytes(), Limits::default(), None).unwrap();
    let column = field.dtype().as_fields().unwrap()[0].clone();
    assert_eq!(
        column.as_xml().namespace(),
        Some("urn:schemas-microsoft-com:xml-analysis:rowset")
    );
    assert_eq!(column.as_xml().kind().unwrap(), XmlKind::Element);
}
