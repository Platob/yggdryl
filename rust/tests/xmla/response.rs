//! `rust/src/xmla/response.rs`: what a request is answered with - the actor
//! every fault names, the XMLA `Error` a fault's detail carries (read from a
//! fault, an element or olap4j's and Mondrian's `error`, written as its four
//! attributes) and the fault built around one, a response read out of literal
//! SOAP 1.1 messages and built envelopes (a rowset - strictly or under a safe
//! cast onto a declared field - nothing, a dataset, or a fault refused as the
//! error it is), and the streaming writers - the reporting one included -
//! whose bytes read back to the same response.
//!
//! `invalid` is crate-private and pinned only through the refusals it spells,
//! whose path is `$.xmla`.

use std::io::Write;

use yggdryl::soap::{Envelope, Fault, FaultCode, Fragment};
use yggdryl::xml::Element;
use yggdryl::xmla::response::{ACTOR, write_rowset_reporting};
use yggdryl::xmla::service::code::EXECUTION_FAILED;
use yggdryl::xmla::{
    Answer, Content, Method, Response, Rowset, Session, XmlaError, fault, write_empty, write_fault,
    write_rowset,
};
use yggdryl::{ArrowCastOptions, DataType, Error, Field, Scalar, Serie, StructType};

const SOAP: &str = "http://schemas.xmlsoap.org/soap/envelope/";
const XMLA: &str = "urn:schemas-microsoft-com:xml-analysis";
const ROWSET: &str = "urn:schemas-microsoft-com:xml-analysis:rowset";
const EMPTY: &str = "urn:schemas-microsoft-com:xml-analysis:empty";
const MDDATASET: &str = "urn:schemas-microsoft-com:xml-analysis:mddataset";
const EXCEPTION: &str = "urn:schemas-microsoft-com:xml-analysis:exception";
const XSD: &str = "http://www.w3.org/2001/XMLSchema";
const XSI: &str = "http://www.w3.org/2001/XMLSchema-instance";
const MONDRIAN: &str = "http://mondrian.sourceforge.net";

/// The rowset schema of the orders table, under the `xsd` prefix: a required
/// `Order Id` spelled `Order_x0020_Id`, and a nullable `Symbol`.
const SCHEMA: &str = concat!(
    "<xsd:schema targetNamespace=\"urn:schemas-microsoft-com:xml-analysis:rowset\" ",
    "xmlns:sql=\"urn:schemas-microsoft-com:xml-sql\" elementFormDefault=\"qualified\">",
    "<xsd:element name=\"root\"><xsd:complexType>",
    "<xsd:sequence minOccurs=\"0\" maxOccurs=\"unbounded\">",
    "<xsd:element name=\"row\" type=\"row\"/>",
    "</xsd:sequence></xsd:complexType></xsd:element>",
    "<xsd:complexType name=\"row\"><xsd:sequence>",
    "<xsd:element sql:field=\"Order Id\" name=\"Order_x0020_Id\" type=\"xsd:int\"/>",
    "<xsd:element sql:field=\"Symbol\" name=\"Symbol\" type=\"xsd:string\" minOccurs=\"0\"/>",
    "</xsd:sequence></xsd:complexType>",
    "</xsd:schema>",
);

/// Two orders: the second states no symbol.
const ROWS: &str = concat!(
    "<row><Order_x0020_Id>7</Order_x0020_Id><Symbol>AAPL</Symbol></row>",
    "<row><Order_x0020_Id>8</Order_x0020_Id></row>",
);

/// A whole Discover answer as a provider indents it: the schema, then the two
/// orders.
const DISCOVER_RESPONSE: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<SOAP-ENV:Envelope xmlns:SOAP-ENV="http://schemas.xmlsoap.org/soap/envelope/">
  <SOAP-ENV:Body>
    <DiscoverResponse xmlns="urn:schemas-microsoft-com:xml-analysis">
      <return>
        <root xmlns="urn:schemas-microsoft-com:xml-analysis:rowset"
              xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
              xmlns:xsd="http://www.w3.org/2001/XMLSchema">
          <xsd:schema targetNamespace="urn:schemas-microsoft-com:xml-analysis:rowset"
                      xmlns:sql="urn:schemas-microsoft-com:xml-sql"
                      elementFormDefault="qualified">
            <xsd:element name="root">
              <xsd:complexType>
                <xsd:sequence minOccurs="0" maxOccurs="unbounded">
                  <xsd:element name="row" type="row"/>
                </xsd:sequence>
              </xsd:complexType>
            </xsd:element>
            <xsd:complexType name="row">
              <xsd:sequence>
                <xsd:element sql:field="Order Id" name="Order_x0020_Id" type="xsd:int"/>
                <xsd:element sql:field="Symbol" name="Symbol" type="xsd:string" minOccurs="0"/>
              </xsd:sequence>
            </xsd:complexType>
          </xsd:schema>
          <row>
            <Order_x0020_Id>7</Order_x0020_Id>
            <Symbol>AAPL</Symbol>
          </row>
          <row>
            <Order_x0020_Id>8</Order_x0020_Id>
          </row>
        </root>
      </return>
    </DiscoverResponse>
  </SOAP-ENV:Body>
</SOAP-ENV:Envelope>
"#;

/// The same answer under prefixes no default spells: `soap` for the
/// envelope, `m` for XMLA, `r` for the rowset and `xs` for XML Schema.
const PREFIXED_RESPONSE: &str = concat!(
    "<?xml version=\"1.0\" encoding=\"utf-8\"?>",
    "<soap:Envelope xmlns:soap=\"http://schemas.xmlsoap.org/soap/envelope/\"><soap:Body>",
    "<m:DiscoverResponse xmlns:m=\"urn:schemas-microsoft-com:xml-analysis\"><m:return>",
    "<r:root xmlns:r=\"urn:schemas-microsoft-com:xml-analysis:rowset\" ",
    "xmlns:xs=\"http://www.w3.org/2001/XMLSchema\">",
    "<xs:schema targetNamespace=\"urn:schemas-microsoft-com:xml-analysis:rowset\" ",
    "xmlns:sql=\"urn:schemas-microsoft-com:xml-sql\" elementFormDefault=\"qualified\">",
    "<xs:complexType name=\"row\"><xs:sequence>",
    "<xs:element sql:field=\"Order Id\" name=\"Order_x0020_Id\" type=\"xs:int\"/>",
    "<xs:element sql:field=\"Symbol\" name=\"Symbol\" type=\"xs:string\" minOccurs=\"0\"/>",
    "</xs:sequence></xs:complexType></xs:schema>",
    "<r:row><r:Order_x0020_Id>7</r:Order_x0020_Id><r:Symbol>AAPL</r:Symbol></r:row>",
    "<r:row><r:Order_x0020_Id>8</r:Order_x0020_Id></r:row>",
    "</r:root></m:return></m:DiscoverResponse></soap:Body></soap:Envelope>",
);

/// A multidimensional dataset's `root`, as a provider answering MDX writes it.
const DATASET: &str = concat!(
    "<root xmlns=\"urn:schemas-microsoft-com:xml-analysis:mddataset\">",
    "<OlapInfo><CubeInfo><Cube><CubeName>Sales</CubeName></Cube></CubeInfo></OlapInfo>",
    "<Axes><Axis name=\"Axis0\"/></Axes>",
    "<CellData><Cell CellOrdinal=\"0\"><Value>42</Value></Cell></CellData>",
    "</root>",
);

/// A message: the envelope under `SOAP-ENV`, `header` when not empty, and
/// `body` as literal markup.
fn envelope(header: &str, body: &str) -> String {
    let header = if header.is_empty() {
        String::new()
    } else {
        format!("<SOAP-ENV:Header>{header}</SOAP-ENV:Header>")
    };
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
         <SOAP-ENV:Envelope xmlns:SOAP-ENV=\"{SOAP}\">{header}\
         <SOAP-ENV:Body>{body}</SOAP-ENV:Body></SOAP-ENV:Envelope>"
    )
}

/// The response element `name` in the XMLA namespace, its `return` holding
/// `returned` as literal markup.
fn response(name: &str, returned: &str) -> String {
    format!("<{name} xmlns=\"{XMLA}\"><return>{returned}</return></{name}>")
}

/// A rowset `root` holding `content`, declared as a provider declares it.
fn rowset_root(content: &str) -> String {
    format!("<root xmlns=\"{ROWSET}\" xmlns:xsi=\"{XSI}\" xmlns:xsd=\"{XSD}\">{content}</root>")
}

/// A fault body: `SOAP-ENV:Client`, its string, the provider as its actor,
/// and `detail` as literal markup.
fn fault_body(detail: &str) -> String {
    format!(
        "<SOAP-ENV:Fault><faultcode>SOAP-ENV:Client</faultcode>\
         <faultstring>The syntax is incorrect.</faultstring>\
         <faultactor>provider</faultactor>{detail}</SOAP-ENV:Fault>"
    )
}

fn read(xml: &str, field: Option<&Field>) -> Response {
    Response::from_bytes(xml.as_bytes(), field).unwrap_or_else(|error| panic!("{error}\n{xml}"))
}

fn refused(xml: &str, field: Option<&Field>) -> Error {
    match Response::from_bytes(xml.as_bytes(), field) {
        Ok(response) => panic!("expected a refusal, read {response:?}\n{xml}"),
        Err(error) => error,
    }
}

/// The path and reason of an invalid-record refusal.
fn invalid_record(error: Error) -> (String, String) {
    match error {
        Error::InvalidRecord { path, reason } => (path.to_string(), reason.to_string()),
        other => panic!("expected an invalid record refusal, got {other:?}"),
    }
}

/// The format and reason of a codec refusal.
fn codec(error: Error) -> (&'static str, String) {
    match error {
        Error::Codec { format, reason, .. } => (format, reason.to_string()),
        other => panic!("expected a codec refusal, got {other:?}"),
    }
}

/// The fault a message's bytes carry.
fn read_fault(bytes: &[u8]) -> Fault {
    Envelope::from_bytes(bytes)
        .unwrap_or_else(|error| panic!("{error}\n{}", String::from_utf8_lossy(bytes)))
        .fault()
        .cloned()
        .unwrap_or_else(|| panic!("expected a fault:\n{}", String::from_utf8_lossy(bytes)))
}

fn text(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).expect("the writer writes UTF-8")
}

/// The orders table: a required `Order Id` and a nullable `Symbol`.
fn orders_field() -> Field {
    StructType::from_fields([
        DataType::Int32.required_field("Order Id"),
        DataType::utf8().nullable_field("Symbol"),
    ])
    .map(DataType::from)
    .expect("a valid root")
    .required_field("row")
}

fn order(id: i32, symbol: Option<&str>) -> Scalar {
    Scalar::from_struct([
        ("Order Id", Scalar::from(id)),
        ("Symbol", symbol.map_or(Scalar::Null, Scalar::from)),
    ])
    .expect("distinct names")
}

fn orders(rows: impl IntoIterator<Item = Scalar>) -> Serie {
    Serie::from_scalars(orders_field(), rows).expect("rows the field accepts")
}

/// The two orders every literal response above holds.
fn two_orders() -> Serie {
    orders([order(7, Some("AAPL")), order(8, None)])
}

fn orders_rowset() -> Rowset {
    Rowset::new(orders_field()).expect("a rowset")
}

/// The session header block a client continuing `sess-1` sends.
fn session() -> Fragment {
    Session::Continue("sess-1".into())
        .into_fragment()
        .expect("a session block")
}

/// A batch source that fails the test when pulled.
fn never_pulled() -> impl Iterator<Item = yggdryl::arrow::Result<Serie>> {
    std::iter::from_fn(|| panic!("the rows are pulled although the content asks for none"))
}

/// A sink refusing every write.
struct FullSink;

impl Write for FullSink {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::other("the sink is full"))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The message of the sink's own failure.
fn sink_failure<T: std::fmt::Debug>(result: yggdryl::Result<T>) -> String {
    match result {
        Err(Error::Io(error)) => error.to_string(),
        Err(other) => panic!("expected the sink's failure, got {other:?}"),
        Ok(written) => panic!("expected the sink's failure, wrote {written:?}"),
    }
}

// The actor and the XMLA `Error`.

#[test]
fn the_actor_every_fault_names_is_yggdryl() {
    assert_eq!(ACTOR, "yggdryl");
}

#[test]
fn a_new_error_is_raised_by_the_actor_and_names_no_help_file() {
    let error = XmlaError::new(3_238_658_057, "The cube is not processed.");
    assert_eq!(error.code(), 3_238_658_057);
    assert_eq!(error.description(), "The cube is not processed.");
    assert_eq!(error.source(), ACTOR);
    assert_eq!(error.help_file(), "");
}

#[test]
fn with_source_names_another_component_and_keeps_the_rest() {
    let error = XmlaError::new(7, "boom").with_source("Analysis Services");
    assert_eq!(error.source(), "Analysis Services");
    assert_eq!(error.code(), 7);
    assert_eq!(error.description(), "boom");
    assert_eq!(error.help_file(), "");
    assert_ne!(error, XmlaError::new(7, "boom"));
}

#[test]
fn into_fragment_is_an_error_element_in_the_exception_namespace() {
    let error =
        XmlaError::new(3_238_658_057, "The syntax for 'selct' is incorrect.").with_source("parser");
    let fragment = error.into_fragment().expect("an Error element");
    assert_eq!(fragment.name(), "Error");
    let element = fragment.element();
    assert_eq!(element.local_name(), "Error");
    assert_eq!(element.namespace(), Some(EXCEPTION));
    let attribute = |name: &str| element.attribute(name).and_then(Scalar::as_str);
    assert_eq!(attribute("ErrorCode"), Some("3238658057"));
    assert_eq!(
        attribute("Description"),
        Some("The syntax for 'selct' is incorrect.")
    );
    assert_eq!(attribute("Source"), Some("parser"));
    // An absent help file is written, empty, as the specification's own
    // faults spell it.
    assert_eq!(attribute("HelpFile"), Some(""));
    let names: Vec<&str> = element.attributes().map(|(name, _)| name).collect();
    assert_eq!(
        names,
        ["Description", "ErrorCode", "HelpFile", "Source", "xmlns"]
    );
    assert_eq!(element.children().count(), 0);
    assert_eq!(element.text(), None);
}

#[test]
fn into_fragment_keeps_unicode_and_markup_characters_verbatim() {
    let description = "Prix « négatif » <0 & \"cité\" – 東京";
    let fragment = XmlaError::new(1, description)
        .into_fragment()
        .expect("an Error element");
    assert_eq!(
        fragment
            .element()
            .attribute("Description")
            .and_then(Scalar::as_str),
        Some(description)
    );
}

#[test]
fn from_fault_of_a_fault_with_no_detail_is_empty() {
    assert!(XmlaError::from_fault(&Fault::client("no detail")).is_empty());
    assert!(XmlaError::from_fault(&Fault::server("no detail").with_actor(ACTOR)).is_empty());
}

#[test]
fn from_fault_skips_detail_elements_that_are_not_errors() {
    let other = Fragment::in_namespace("Trace", "urn:example:trace", Scalar::from("on"))
        .expect("a detail element");
    assert!(XmlaError::from_fault(&Fault::client("x").with_detail(other.clone())).is_empty());

    let error = XmlaError::new(5, "only this one");
    let fault = Fault::client("x")
        .with_detail(other)
        .with_detail(error.into_fragment().expect("an Error element"));
    assert_eq!(XmlaError::from_fault(&fault), vec![error]);
}

#[test]
fn from_fault_reads_back_the_error_fault_builds() {
    let error = XmlaError::new(3_238_658_057, "The syntax is incorrect.").with_source("parser");
    let built = fault(FaultCode::Client, error.clone()).expect("a fault");
    assert_eq!(XmlaError::from_fault(&built), vec![error]);
}

#[test]
fn from_fault_reads_every_error_of_a_literal_detail_in_document_order() {
    let xml = envelope(
        "",
        &fault_body(&format!(
            "<detail>\
             <Error xmlns=\"{EXCEPTION}\" ErrorCode=\"3238658057\" \
             Description=\"The syntax for 'selct' is incorrect.\" \
             Source=\"Analysis Services\" HelpFile=\"http://help.example/1\"/>\
             <Error xmlns=\"{EXCEPTION}\" ErrorCode=\"3238658058\" \
             Description=\"The query was cancelled.\" Source=\"engine\" HelpFile=\"\"/>\
             </detail>"
        )),
    );
    let errors = XmlaError::from_fault(&read_fault(xml.as_bytes()));
    assert_eq!(errors.len(), 2, "{errors:?}");
    assert_eq!(errors[0].code(), 3_238_658_057);
    assert_eq!(
        errors[0].description(),
        "The syntax for 'selct' is incorrect."
    );
    assert_eq!(errors[0].source(), "Analysis Services");
    assert_eq!(errors[0].help_file(), "http://help.example/1");
    assert_eq!(errors[1].code(), 3_238_658_058);
    assert_eq!(errors[1].description(), "The query was cancelled.");
    assert_eq!(errors[1].source(), "engine");
    assert_eq!(errors[1].help_file(), "");
}

#[test]
fn from_fault_reads_the_errors_under_a_messages_element() {
    let xml = envelope(
        "",
        &fault_body(&format!(
            "<detail><Messages xmlns=\"{EXCEPTION}\">\
             <Error ErrorCode=\"1\" Description=\"first\" Source=\"a\" HelpFile=\"\"/>\
             <Warning WarningCode=\"9\" Description=\"not an error\"/>\
             <Error ErrorCode=\"2\" Description=\"second\" Source=\"b\" HelpFile=\"\"/>\
             </Messages></detail>"
        )),
    );
    let errors = XmlaError::from_fault(&read_fault(xml.as_bytes()));
    let read: Vec<(u32, &str, &str)> = errors
        .iter()
        .map(|error| (error.code(), error.description(), error.source()))
        .collect();
    assert_eq!(read, [(1, "first", "a"), (2, "second", "b")]);
}

#[test]
fn a_detail_mixing_messages_and_a_direct_error_reads_in_name_order() {
    // A read fault's detail comes in name order, the one order a parsed
    // document keeps, so the direct `Error` precedes the one under `Messages`
    // whatever the wire held.
    let xml = envelope(
        "",
        &fault_body(&format!(
            "<detail>\
             <Messages xmlns=\"{EXCEPTION}\"><Error ErrorCode=\"1\" Description=\"first\"/></Messages>\
             <Error xmlns=\"{EXCEPTION}\" ErrorCode=\"2\" Description=\"second\"/>\
             </detail>"
        )),
    );
    let codes: Vec<u32> = XmlaError::from_fault(&read_fault(xml.as_bytes()))
        .iter()
        .map(XmlaError::code)
        .collect();
    assert_eq!(codes, [2, 1]);
}

#[test]
fn an_error_without_attributes_reads_its_text_as_the_description_and_code_zero() {
    let xml = envelope(
        "",
        &fault_body(&format!(
            "<detail><Error xmlns=\"{EXCEPTION}\">the cube is not processed</Error></detail>"
        )),
    );
    let errors = XmlaError::from_fault(&read_fault(xml.as_bytes()));
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code(), 0);
    assert_eq!(errors[0].description(), "the cube is not processed");
    assert_eq!(errors[0].source(), "");
    assert_eq!(errors[0].help_file(), "");
}

#[test]
fn the_description_attribute_wins_over_the_element_text() {
    let xml = envelope(
        "",
        &fault_body(&format!(
            "<detail><Error xmlns=\"{EXCEPTION}\" ErrorCode=\"4\" \
             Description=\"from the attribute\">from the text</Error></detail>"
        )),
    );
    let errors = XmlaError::from_fault(&read_fault(xml.as_bytes()));
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].description(), "from the attribute");
}

#[test]
fn a_code_is_read_trimmed_and_one_that_is_no_unsigned_integer_reads_as_zero() {
    let codes: Vec<u32> = [
        " 42 ",
        "4294967295",
        "-1056178166",
        "0xC10A0004",
        "",
        "4294967296",
    ]
    .iter()
    .map(|code| {
        let xml = envelope(
            "",
            &fault_body(&format!(
                "<detail><Error xmlns=\"{EXCEPTION}\" ErrorCode=\"{code}\" \
                     Description=\"d\"/></detail>"
            )),
        );
        let errors = XmlaError::from_fault(&read_fault(xml.as_bytes()));
        assert_eq!(errors.len(), 1, "{code:?}");
        errors[0].code()
    })
    .collect();
    assert_eq!(codes, [42, u32::MAX, 0, 0, 0, 0]);
}

#[test]
fn an_error_element_under_a_prefix_reads_the_same() {
    let xml = envelope(
        "",
        &fault_body(&format!(
            "<detail><ex:Error xmlns:ex=\"{EXCEPTION}\" ErrorCode=\"11\" \
             Description=\"prefixed\" Source=\"s\" HelpFile=\"h\"/></detail>"
        )),
    );
    let errors = XmlaError::from_fault(&read_fault(xml.as_bytes()));
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code(), 11);
    assert_eq!(errors[0].description(), "prefixed");
    assert_eq!(errors[0].source(), "s");
    assert_eq!(errors[0].help_file(), "h");
}

#[test]
fn an_error_s_escaped_and_unicode_attributes_read_decoded() {
    let xml = envelope(
        "",
        &fault_body(&format!(
            "<detail><Error xmlns=\"{EXCEPTION}\" ErrorCode=\"12\" \
             Description=\"a &amp; &lt;b&gt; &quot;c&quot; &#233; – 東京\" \
             Source=\"Zürich\" HelpFile=\"\"/></detail>"
        )),
    );
    let errors = XmlaError::from_fault(&read_fault(xml.as_bytes()));
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].description(), "a & <b> \"c\" é – 東京");
    assert_eq!(errors[0].source(), "Zürich");
}

#[test]
fn from_fault_keeps_the_order_of_a_fault_built_here() {
    // A fault built here answers its detail as given, so a `Messages` added
    // before a direct `Error` is read first - unlike a read fault, whose
    // detail comes in name order.
    let document = yggdryl::from_xml_scalar(format!(
        "<Messages xmlns=\"{EXCEPTION}\"><Error ErrorCode=\"1\" Description=\"first\"/></Messages>"
    ))
    .expect("the messages parse");
    let messages = Fragment::from_element(&Element::root(&document).expect("one element"));
    let direct = XmlaError::new(2, "second");
    let built = Fault::client("x")
        .with_detail(messages)
        .with_detail(direct.into_fragment().expect("an Error element"));
    let codes: Vec<u32> = XmlaError::from_fault(&built)
        .iter()
        .map(XmlaError::code)
        .collect();
    assert_eq!(codes, [1, 2]);
}

#[test]
fn from_fault_reads_a_mondrian_error_by_its_hexadecimal_code_and_its_desc() {
    // olap4j and Mondrian answer `<XA:error><code/><desc/></XA:error>` in
    // their own namespace; the code is hexadecimal, `0x` or not, and one
    // that is not reads as zero.
    let xml = envelope(
        "",
        &fault_body(&format!(
            "<detail>\
             <XA:error xmlns:XA=\"{MONDRIAN}\"><code>00HSBE02</code>\
             <desc>The Mondrian XML: Mondrian Error:Internal error</desc></XA:error>\
             <XA:error xmlns:XA=\"{MONDRIAN}\"><code>1F</code><desc>hex</desc></XA:error>\
             <XA:error xmlns:XA=\"{MONDRIAN}\"><desc>no code</desc>\
             <code> 0xC10A0004 </code></XA:error>\
             </detail>"
        )),
    );
    let errors = XmlaError::from_fault(&read_fault(xml.as_bytes()));
    let read: Vec<(u32, &str, &str, &str)> = errors
        .iter()
        .map(|error| {
            (
                error.code(),
                error.description(),
                error.source(),
                error.help_file(),
            )
        })
        .collect();
    assert_eq!(
        read,
        [
            (
                0,
                "The Mondrian XML: Mondrian Error:Internal error",
                "Mondrian",
                ""
            ),
            (31, "hex", "Mondrian", ""),
            (3_238_658_052, "no code", "Mondrian", ""),
        ]
    );
}

#[test]
fn a_mondrian_error_is_read_only_in_its_namespace_and_with_a_desc() {
    let xml = envelope(
        "",
        &fault_body(&format!(
            "<detail>\
             <error><code>1</code><desc>in no namespace</desc></error>\
             <o:error xmlns:o=\"urn:example:other\"><code>2</code><desc>elsewhere</desc></o:error>\
             <XA:error xmlns:XA=\"{MONDRIAN}\"><code>3</code><desc/></XA:error>\
             <XA:error xmlns:XA=\"{MONDRIAN}\"><code>4</code></XA:error>\
             </detail>"
        )),
    );
    assert!(XmlaError::from_fault(&read_fault(xml.as_bytes())).is_empty());

    // An emptied `desc` is the empty text, which is a description.
    let emptied = envelope(
        "",
        &fault_body(&format!(
            "<detail><XA:error xmlns:XA=\"{MONDRIAN}\"><code>A</code><desc></desc>\
             </XA:error></detail>"
        )),
    );
    let errors = XmlaError::from_fault(&read_fault(emptied.as_bytes()));
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code(), 10);
    assert_eq!(errors[0].description(), "");
    assert_eq!(errors[0].source(), "Mondrian");
}

#[test]
fn from_element_reads_the_attributes_and_olap4j_s_help_spelling() {
    let document = yggdryl::from_xml_scalar(format!(
        "<Messages xmlns=\"{EXCEPTION}\">\
         <EX:Error xmlns:EX=\"{EXCEPTION}\" ErrorCode=\"9\" Source=\"olap4j\" Help=\"h.chm\">\
         from the text</EX:Error>\
         </Messages>"
    ))
    .expect("the messages parse");
    let messages = Element::root(&document).expect("one element");
    let error = messages
        .child(Some(EXCEPTION), "Error")
        .expect("the Error element");
    let read = XmlaError::from_element(&error).expect("an error");
    assert_eq!(read.code(), 9);
    assert_eq!(read.description(), "from the text");
    assert_eq!(read.source(), "olap4j");
    assert_eq!(read.help_file(), "h.chm");

    // `HelpFile` is the specification's spelling and wins over `Help`.
    let document = yggdryl::from_xml_scalar(format!(
        "<Error xmlns=\"{EXCEPTION}\" ErrorCode=\"1\" Description=\"d\" \
         HelpFile=\"spec.chm\" Help=\"olap4j.chm\"/>"
    ))
    .expect("the error parses");
    let read =
        XmlaError::from_element(&Element::root(&document).expect("one element")).expect("an error");
    assert_eq!(read.help_file(), "spec.chm");
    assert_eq!(read.description(), "d");
}

#[test]
fn write_spells_the_four_attributes_escaped_and_declares_no_namespace() {
    let mut written = Vec::new();
    XmlaError::new(7, "a \"b\" <c> & d\ne\tf – 東京")
        .with_source("s'rc")
        .write(&mut written)
        .expect("the error is written");
    assert_eq!(
        text(&written),
        "<Error ErrorCode=\"7\" Description=\"a &quot;b&quot; &lt;c&gt; &amp; d&#10;e&#9;f – 東京\" \
         Source=\"s'rc\" HelpFile=\"\"/>"
    );
}

#[test]
fn write_reads_back_through_from_element_in_the_enclosing_exception_namespace() {
    let error = XmlaError::new(u32::MAX, "Le « cube » n'existe pas & <rien>\n– 東京")
        .with_source("catalog");
    let mut written = format!("<Messages xmlns=\"{EXCEPTION}\">").into_bytes();
    error.write(&mut written).expect("the error is written");
    written.extend_from_slice(b"</Messages>");
    let document = yggdryl::from_xml_scalar(&written)
        .unwrap_or_else(|failure| panic!("{failure}\n{}", text(&written)));
    let messages = Element::root(&document).expect("one element");
    let element = messages
        .child(Some(EXCEPTION), "Error")
        .expect("the Error element is in the exception namespace");
    assert_eq!(XmlaError::from_element(&element), Some(error));
}

#[test]
fn write_refuses_text_xml_cannot_carry_naming_the_code_point() {
    let (format, reason) = codec(
        XmlaError::new(7, "bell \u{7}")
            .write(&mut Vec::new())
            .expect_err("a control character has no XML spelling"),
    );
    assert_eq!(format, "xml");
    assert_eq!(reason, "text carries U+0007, which XML 1.0 cannot spell");

    let (_, reason) = codec(
        XmlaError::new(7, "fine")
            .with_source("\u{FFFE}")
            .write(&mut Vec::new())
            .expect_err("a noncharacter has no XML spelling"),
    );
    assert_eq!(reason, "text carries U+FFFE, which XML 1.0 cannot spell");
}

#[test]
fn write_refuses_a_failing_sink() {
    assert_eq!(
        sink_failure(XmlaError::new(1, "x").write(&mut FullSink)),
        "the sink is full"
    );
}

// `fault`.

#[test]
fn fault_carries_the_actor_the_description_and_the_error_detail() {
    let error = XmlaError::new(3_238_658_052, "The provider is down.").with_source("engine");
    let built = fault(FaultCode::Server, error.clone()).expect("a fault");
    assert_eq!(built.code(), &FaultCode::Server);
    assert_eq!(built.subcode(), None);
    assert_eq!(built.string(), "The provider is down.");
    // The fault names the crate as its actor whichever source the error
    // names.
    assert_eq!(built.actor(), Some(ACTOR));
    assert_eq!(built.detail().len(), 1);
    assert_eq!(built.detail()[0].name(), "Error");
    assert_eq!(built.detail()[0].element().namespace(), Some(EXCEPTION));
    assert_eq!(XmlaError::from_fault(&built), vec![error]);

    let client = fault(FaultCode::Client, XmlaError::new(1, "bad request")).expect("a fault");
    assert_eq!(client.code(), &FaultCode::Client);
    assert_eq!(client.string(), "bad request");
}

// Reading: the refusals.

#[test]
fn a_fault_body_is_refused_naming_the_fault_and_its_first_xmla_error() {
    let xml = envelope(
        "",
        &fault_body(&format!(
            "<detail><Error xmlns=\"{EXCEPTION}\" ErrorCode=\"3238658057\" \
             Description=\"The syntax for 'selct' is incorrect.\" \
             Source=\"Analysis Services\" HelpFile=\"\"/></detail>"
        )),
    );
    let (path, reason) = invalid_record(refused(&xml, None));
    assert_eq!(path, "$.xmla");
    assert_eq!(
        reason,
        "SOAP fault SOAP-ENV:Client: The syntax is incorrect. \
         (3238658057 The syntax for 'selct' is incorrect.)"
    );
    // A declared field changes nothing about a fault.
    let (_, declared) = invalid_record(refused(&xml, Some(&orders_field())));
    assert_eq!(declared, reason);
}

#[test]
fn a_fault_with_no_xmla_error_is_refused_by_its_code_and_string_alone() {
    let xml = envelope(
        "",
        "<SOAP-ENV:Fault><faultcode>SOAP-ENV:Server</faultcode>\
         <faultstring>the provider is down</faultstring></SOAP-ENV:Fault>",
    );
    let (path, reason) = invalid_record(refused(&xml, None));
    assert_eq!(path, "$.xmla");
    assert_eq!(reason, "SOAP fault SOAP-ENV:Server: the provider is down");
}

#[test]
fn a_fault_with_several_errors_is_refused_naming_the_first() {
    let xml = envelope(
        "",
        &fault_body(&format!(
            "<detail>\
             <Error xmlns=\"{EXCEPTION}\" ErrorCode=\"1\" Description=\"first\"/>\
             <Error xmlns=\"{EXCEPTION}\" ErrorCode=\"2\" Description=\"second\"/>\
             </detail>"
        )),
    );
    let (_, reason) = invalid_record(refused(&xml, None));
    assert!(reason.ends_with(" (1 first)"), "{reason}");
    assert!(!reason.contains("second"), "{reason}");
}

#[test]
fn a_fault_with_a_subcode_is_refused_naming_it() {
    let xml = envelope(
        "",
        "<SOAP-ENV:Fault><faultcode>SOAP-ENV:Client.Authentication</faultcode>\
         <faultstring>who are you</faultstring></SOAP-ENV:Fault>",
    );
    let (_, reason) = invalid_record(refused(&xml, None));
    assert_eq!(
        reason,
        "SOAP fault SOAP-ENV:Client.Authentication: who are you"
    );
}

#[test]
fn a_body_that_is_not_a_response_is_refused_naming_it() {
    let xml = envelope(
        "",
        &format!(
            "<Discover xmlns=\"{XMLA}\"><RequestType>DISCOVER_DATASOURCES</RequestType></Discover>"
        ),
    );
    let (path, reason) = invalid_record(refused(&xml, None));
    assert_eq!(path, "$.xmla");
    assert_eq!(
        reason,
        "expected `DiscoverResponse` or `ExecuteResponse` in \
         \"urn:schemas-microsoft-com:xml-analysis\", got `Discover`"
    );
}

#[test]
fn an_unknown_response_name_is_refused_naming_it() {
    let xml = envelope(
        "",
        &response(
            "StatementResponse",
            &rowset_root(&format!("{SCHEMA}{ROWS}")),
        ),
    );
    let (_, reason) = invalid_record(refused(&xml, None));
    assert!(reason.ends_with("got `StatementResponse`"), "{reason}");
}

#[test]
fn a_response_in_another_namespace_is_refused_naming_it_as_spelled() {
    let root = rowset_root(&format!("{SCHEMA}{ROWS}"));
    let defaulted = envelope(
        "",
        &format!(
            "<DiscoverResponse xmlns=\"urn:example:other\"><return>{root}</return></DiscoverResponse>"
        ),
    );
    let (_, reason) = invalid_record(refused(&defaulted, None));
    assert!(reason.ends_with("got `DiscoverResponse`"), "{reason}");

    let prefixed = envelope(
        "",
        &format!(
            "<o:DiscoverResponse xmlns:o=\"urn:example:other\"><o:return>{root}</o:return>\
             </o:DiscoverResponse>"
        ),
    );
    let (_, reason) = invalid_record(refused(&prefixed, None));
    assert!(reason.ends_with("got `o:DiscoverResponse`"), "{reason}");
}

#[test]
fn a_response_without_a_return_is_refused_naming_the_response() {
    let xml = envelope("", &format!("<DiscoverResponse xmlns=\"{XMLA}\"/>"));
    let (format, reason) = codec(refused(&xml, None));
    assert_eq!(format, "xml");
    assert_eq!(
        reason,
        "expected one `return` element under `DiscoverResponse`, found none"
    );
}

#[test]
fn a_response_with_two_returns_is_refused() {
    let root = rowset_root(SCHEMA);
    let xml = envelope(
        "",
        &format!(
            "<ExecuteResponse xmlns=\"{XMLA}\"><return>{root}</return><return>{root}</return>\
             </ExecuteResponse>"
        ),
    );
    let (format, reason) = codec(refused(&xml, None));
    assert_eq!(format, "xml");
    assert_eq!(
        reason,
        "expected one `return` element under `ExecuteResponse`, found several"
    );
}

#[test]
fn a_return_without_a_root_is_refused() {
    for returned in [
        format!("<DiscoverResponse xmlns=\"{XMLA}\"><return/></DiscoverResponse>"),
        response("DiscoverResponse", "   "),
        response("DiscoverResponse", "just text"),
        response("DiscoverResponse", "<rows><row/></rows>"),
    ] {
        let xml = envelope("", &returned);
        let (path, reason) = invalid_record(refused(&xml, None));
        assert_eq!(path, "$.xmla");
        assert_eq!(reason, "the response's `return` holds no `root`");
    }
}

#[test]
fn a_return_with_two_roots_is_refused() {
    // The rustdoc refuses a body that does not hold one `return` and one
    // `root`: a second result is not silently dropped, whichever namespace
    // either is in.
    for roots in [
        format!(
            "{}{}",
            rowset_root(&format!("{SCHEMA}{ROWS}")),
            rowset_root(SCHEMA)
        ),
        format!("{}<root xmlns=\"{EMPTY}\"/>", rowset_root(SCHEMA)),
        format!("<root xmlns=\"{EMPTY}\"/><x:root xmlns:x=\"urn:example:cube\"/>"),
    ] {
        let xml = envelope("", &response("DiscoverResponse", &roots));
        let (path, reason) = invalid_record(refused(&xml, None));
        assert_eq!(path, "$.xmla");
        assert_eq!(
            reason,
            "the response's `return` holds several `root` elements; a response carries one result"
        );
    }
}

#[test]
fn a_root_in_a_namespace_that_names_no_result_is_refused_naming_it() {
    let xml = envelope(
        "",
        &response("ExecuteResponse", "<root xmlns=\"urn:example:cube\"/>"),
    );
    let (path, reason) = invalid_record(refused(&xml, None));
    assert_eq!(path, "$.xmla");
    assert_eq!(
        reason,
        "the response's `root` is in \"urn:example:cube\", which names no result this crate reads"
    );
}

#[test]
fn a_root_that_inherits_the_xmla_namespace_is_refused_naming_it() {
    // An undeclared `root` under a response in the XMLA namespace is in that
    // namespace, which is none of the three a result is in.
    let xml = envelope(
        "",
        &response("DiscoverResponse", &format!("<root>{ROWS}</root>")),
    );
    let (_, reason) = invalid_record(refused(&xml, Some(&orders_field())));
    assert_eq!(
        reason,
        "the response's `root` is in \"urn:schemas-microsoft-com:xml-analysis\", \
         which names no result this crate reads"
    );
}

#[test]
fn a_rowset_without_a_schema_is_refused_when_no_field_is_declared() {
    let xml = envelope("", &response("ExecuteResponse", &rowset_root(ROWS)));
    let (_, reason) = invalid_record(refused(&xml, None));
    assert_eq!(
        reason,
        "the rowset carries no schema and no field is declared to read its rows by"
    );
}

#[test]
fn a_cell_its_column_cannot_read_is_refused_naming_its_row() {
    let rows = "<row><Order_x0020_Id>7</Order_x0020_Id></row>\
                <row><Order_x0020_Id>seven</Order_x0020_Id></row>";
    let xml = envelope(
        "",
        &response("DiscoverResponse", &rowset_root(&format!("{SCHEMA}{rows}"))),
    );
    let (path, _) = invalid_record(refused(&xml, None));
    assert_eq!(path, "$[1]");
}

#[test]
fn bytes_that_are_not_a_soap_envelope_are_refused_by_the_envelope() {
    let bare = response("DiscoverResponse", &rowset_root(&format!("{SCHEMA}{ROWS}")));
    let (format, reason) = codec(refused(&bare, None));
    assert_eq!(format, "soap");
    assert!(
        reason.starts_with("expected the SOAP 1.1 `Envelope`"),
        "{reason}"
    );

    let (format, _) = codec(refused("<unclosed", None));
    assert_eq!(format, "xml");

    // A missing document - no bytes, blanks, a declaration alone - is no
    // envelope either.
    for missing in ["", "   ", "<?xml version=\"1.0\"?>"] {
        let (format, reason) = codec(refused(missing, None));
        assert_eq!(format, "xml");
        assert_eq!(reason, "expected the root element", "{missing:?}");
    }

    let empty_body = envelope("", "");
    let (format, reason) = codec(refused(&empty_body, None));
    assert_eq!(format, "soap");
    assert_eq!(reason, "the SOAP body holds no element");
}

// Reading: the answers.

#[test]
fn a_discover_response_reads_its_schema_and_its_rows() {
    let response = read(DISCOVER_RESPONSE, None);
    assert_eq!(response.method(), Method::Discover);
    assert!(response.header().is_empty());
    let rowset = response.rowset().expect("a rowset");
    assert_eq!(rowset.field(), &orders_field());
    assert_eq!(rowset.element_names(), ["Order_x0020_Id", "Symbol"]);
    assert_eq!(response.rows(), Some(&two_orders()));
    match response.answer() {
        Answer::Rowset { rowset, rows } => {
            assert_eq!(rowset.field(), &orders_field());
            assert_eq!(rows, &two_orders());
        }
        other => panic!("expected a rowset, got {other:?}"),
    }
}

#[test]
fn reading_one_message_twice_answers_equal_responses() {
    assert_eq!(read(DISCOVER_RESPONSE, None), read(DISCOVER_RESPONSE, None));
}

#[test]
fn an_execute_response_in_the_empty_namespace_answers_nothing() {
    let xml = envelope(
        "",
        &response("ExecuteResponse", &format!("<root xmlns=\"{EMPTY}\"/>")),
    );
    let response = read(&xml, None);
    assert_eq!(response.method(), Method::Execute);
    assert_eq!(response.answer(), &Answer::Empty);
    assert_eq!(response.rows(), None);
    assert!(response.rowset().is_none());
    // A declared field changes nothing about an empty answer.
    assert_eq!(read(&xml, Some(&orders_field())).answer(), &Answer::Empty);
}

#[test]
fn an_execute_response_reads_a_rowset() {
    let xml = envelope(
        "",
        &response("ExecuteResponse", &rowset_root(&format!("{SCHEMA}{ROWS}"))),
    );
    let response = read(&xml, None);
    assert_eq!(response.method(), Method::Execute);
    assert_eq!(response.rowset().map(Rowset::field), Some(&orders_field()));
    assert_eq!(response.rows(), Some(&two_orders()));
}

#[test]
fn a_rowset_carrying_its_schema_and_no_rows_reads_as_no_rows() {
    let xml = envelope("", &response("DiscoverResponse", &rowset_root(SCHEMA)));
    let response = read(&xml, None);
    assert_eq!(response.rowset().map(Rowset::field), Some(&orders_field()));
    assert_eq!(response.rows().map(Serie::len), Some(0));
}

#[test]
fn a_root_with_rows_and_no_schema_reads_under_the_declared_field() {
    let xml = envelope("", &response("ExecuteResponse", &rowset_root(ROWS)));
    let field = orders_field();
    let response = read(&xml, Some(&field));
    assert_eq!(response.rowset().map(Rowset::field), Some(&field));
    assert_eq!(response.rows(), Some(&two_orders()));
}

#[test]
fn a_declared_field_casts_a_rowset_that_carries_its_own_schema() {
    // `from_envelope`: `field` is what the rows come back as - a rowset
    // carrying its schema is read under its own columns (`Order Id` is an
    // `xsd:int` here) and cast onto the declared field, as every medium casts
    // onto a declared field.
    let declared = StructType::from_fields([
        DataType::Int64.required_field("Order Id"),
        DataType::utf8().nullable_field("Symbol"),
    ])
    .map(DataType::from)
    .expect("a valid root")
    .required_field("declared");
    let response = read(DISCOVER_RESPONSE, Some(&declared));
    assert_eq!(response.rowset().map(Rowset::field), Some(&declared));
    let rows = response.rows().expect("a rowset");
    assert_eq!(rows.field().map(Field::name), Some("declared"));
    let ids = rows.child("Order Id").expect("the cast column");
    assert_eq!(ids.field().map(Field::dtype), Some(&DataType::Int64));
    assert_eq!(ids.scalar(0).expect("the first row"), Scalar::from(7_i64));
    assert_eq!(ids.scalar(1).expect("the second row"), Scalar::from(8_i64));
    assert_eq!(
        rows.child("Symbol")
            .and_then(|symbols| symbols.scalar(1).ok()),
        Some(Scalar::Null),
        "the null cell survives the cast"
    );
}

#[test]
fn a_response_under_non_default_prefixes_reads_the_same() {
    let response = read(PREFIXED_RESPONSE, None);
    assert_eq!(response.method(), Method::Discover);
    assert_eq!(response.rowset().map(Rowset::field), Some(&orders_field()));
    assert_eq!(response.rows(), Some(&two_orders()));
    assert_eq!(response, read(DISCOVER_RESPONSE, None));
}

#[test]
fn a_response_in_no_namespace_reads_as_the_xmla_one() {
    let xml = envelope(
        "",
        &format!(
            "<DiscoverResponse><return><root xmlns:xsd=\"{XSD}\">{SCHEMA}{ROWS}</root></return>\
             </DiscoverResponse>"
        ),
    );
    let response = read(&xml, None);
    assert_eq!(response.method(), Method::Discover);
    assert_eq!(response.rowset().map(Rowset::field), Some(&orders_field()));
    assert_eq!(response.rows(), Some(&two_orders()));
}

#[test]
fn unicode_cells_read_as_written() {
    let rows = "<row><Order_x0020_Id>1</Order_x0020_Id><Symbol>Zürich – 東京 🚀</Symbol></row>";
    let xml = envelope(
        "",
        &response("ExecuteResponse", &rowset_root(&format!("{SCHEMA}{rows}"))),
    );
    assert_eq!(
        read(&xml, None).rows(),
        Some(&orders([order(1, Some("Zürich – 東京 🚀"))]))
    );
}

#[test]
fn a_multidimensional_root_is_kept_as_its_natural_value() {
    let xml = envelope("", &response("ExecuteResponse", DATASET));
    let response = read(&xml, None);
    assert_eq!(response.method(), Method::Execute);
    assert_eq!(response.rows(), None);
    assert!(response.rowset().is_none());
    let Answer::Dataset(fragment) = response.answer() else {
        panic!("expected a dataset, got {:?}", response.answer());
    };
    assert_eq!(fragment.name(), "root");
    assert_eq!(fragment.element().namespace(), Some(MDDATASET));
    let document = yggdryl::from_xml_scalar(DATASET).expect("the dataset parses");
    let root = Element::root(&document).expect("one document element");
    assert_eq!(fragment.value(), root.value());
    let dataset = fragment.element();
    let info = dataset
        .child(Some(MDDATASET), "OlapInfo")
        .expect("OlapInfo");
    let cube_info = info.child(Some(MDDATASET), "CubeInfo").expect("CubeInfo");
    let cube = cube_info.child(Some(MDDATASET), "Cube").expect("Cube");
    let name = cube.child(Some(MDDATASET), "CubeName").expect("CubeName");
    assert_eq!(name.text(), Some("Sales"));
}

#[test]
fn a_dataset_root_whose_prefix_is_declared_above_it_keeps_its_namespace() {
    // The `md` prefix is declared on the response element; the dataset's
    // fragment is one element "with its in-scope namespace declarations", so
    // it still resolves the prefix it was read under.
    let xml = envelope(
        "",
        &format!(
            "<DiscoverResponse xmlns=\"{XMLA}\" xmlns:md=\"{MDDATASET}\"><return>\
             <md:root><md:CellData><md:Cell CellOrdinal=\"0\"><md:Value>42</md:Value></md:Cell>\
             </md:CellData></md:root></return></DiscoverResponse>"
        ),
    );
    let response = read(&xml, None);
    let Answer::Dataset(fragment) = response.answer() else {
        panic!("expected a dataset, got {:?}", response.answer());
    };
    assert_eq!(fragment.name(), "md:root");
    assert_eq!(fragment.element().namespace(), Some(MDDATASET));
}

#[test]
fn the_header_blocks_are_read_beside_the_answer() {
    let xml = envelope(
        &format!("<Session xmlns=\"{XMLA}\" SOAP-ENV:mustUnderstand=\"1\" SessionId=\"42\"/>"),
        &response("ExecuteResponse", &format!("<root xmlns=\"{EMPTY}\"/>")),
    );
    let response = read(&xml, None);
    assert_eq!(response.header().len(), 1);
    assert_eq!(response.header()[0].name(), "Session");
    assert_eq!(
        Session::read(response.header()).expect("a session header"),
        Some(Session::Continue("42".into()))
    );
}

#[test]
fn a_fault_carrying_a_mondrian_error_is_refused_naming_it() {
    let xml = envelope(
        "",
        &fault_body(&format!(
            "<detail><XA:error xmlns:XA=\"{MONDRIAN}\"><code>1F</code>\
             <desc>The Mondrian XML: no cube</desc></XA:error></detail>"
        )),
    );
    let (path, reason) = invalid_record(refused(&xml, None));
    assert_eq!(path, "$.xmla");
    assert_eq!(
        reason,
        "SOAP fault SOAP-ENV:Client: The syntax is incorrect. (31 The Mondrian XML: no cube)"
    );
}

#[test]
fn from_envelope_refuses_a_fault_built_here_naming_its_error() {
    let message = Envelope::from_fault(
        fault(FaultCode::Server, XmlaError::new(10, "the table is gone")).expect("a fault"),
    );
    for result in [
        Response::from_envelope(&message, None),
        Response::from_envelope_with(&message, Some(&orders_field()), ArrowCastOptions::default()),
    ] {
        let (path, reason) = invalid_record(result.expect_err("a fault is refused"));
        assert_eq!(path, "$.xmla");
        assert_eq!(
            reason,
            "SOAP fault SOAP-ENV:Server: the table is gone (10 the table is gone)"
        );
    }
}

#[test]
fn from_envelope_reads_a_message_built_in_memory() {
    let document = yggdryl::from_xml_scalar(response(
        "DiscoverResponse",
        &rowset_root(&format!("{SCHEMA}{ROWS}")),
    ))
    .expect("the response parses");
    let payload = Fragment::from_element(&Element::root(&document).expect("one element"));
    let message = Envelope::from_payload(payload).with_header(session());
    let response = Response::from_envelope(&message, None).expect("a response");
    assert_eq!(response.method(), Method::Discover);
    assert_eq!(
        Session::read(response.header()).expect("a session header"),
        Some(Session::Continue("sess-1".into()))
    );
    assert_eq!(response.rowset().map(Rowset::field), Some(&orders_field()));
    assert_eq!(response.rows(), Some(&two_orders()));
    assert_eq!(
        Response::from_envelope_with(&message, None, ArrowCastOptions::default())
            .expect("a response"),
        response
    );
}

/// The orders table with `Symbol` declared as a number, which `AAPL` is not.
fn numeric_symbols() -> Field {
    StructType::from_fields([
        DataType::Int32.required_field("Order Id"),
        DataType::Int32.nullable_field("Symbol"),
    ])
    .map(DataType::from)
    .expect("a valid root")
    .required_field("row")
}

/// A rowset stating `Symbol` as `xsd:string`, its first symbol `AAPL` and
/// its second `42`.
fn stated_symbols() -> String {
    let rows = "<row><Order_x0020_Id>7</Order_x0020_Id><Symbol>AAPL</Symbol></row>\
                <row><Order_x0020_Id>8</Order_x0020_Id><Symbol>42</Symbol></row>";
    envelope(
        "",
        &response("ExecuteResponse", &rowset_root(&format!("{SCHEMA}{rows}"))),
    )
}

#[test]
fn a_declared_field_reads_a_stated_column_strictly_naming_the_cell_it_cannot_hold() {
    let xml = stated_symbols();
    let (path, reason) = invalid_record(refused(&xml, Some(&numeric_symbols())));
    assert_eq!(path, "$[0]");
    assert!(reason.contains("Symbol"), "{reason}");
    assert!(reason.contains("expected int32, got string"), "{reason}");

    let message = Envelope::from_bytes(xml.as_bytes()).expect("an envelope");
    let strict = Response::from_envelope_with(
        &message,
        Some(&numeric_symbols()),
        ArrowCastOptions::default().with_safe(false),
    );
    let (path, _) = invalid_record(strict.expect_err("a strict cast refuses `AAPL`"));
    assert_eq!(path, "$[0]");
}

#[test]
fn from_envelope_with_a_safe_cast_nulls_what_the_declared_field_cannot_hold() {
    let message = Envelope::from_bytes(stated_symbols().as_bytes()).expect("an envelope");
    let response = Response::from_envelope_with(
        &message,
        Some(&numeric_symbols()),
        ArrowCastOptions::default(),
    )
    .expect("a safe cast reads every row");
    assert_eq!(response.method(), Method::Execute);
    assert_eq!(
        response.rowset().map(Rowset::field),
        Some(&numeric_symbols())
    );
    let symbols = response
        .rows()
        .and_then(|rows| rows.child("Symbol"))
        .expect("the symbol column");
    assert_eq!(symbols.scalar(0).expect("the first row"), Scalar::Null);
    assert_eq!(
        symbols.scalar(1).expect("the second row"),
        Scalar::from(42_i32)
    );
}

#[test]
fn an_unqualified_return_and_root_under_a_prefixed_response_read_as_the_xmla_ones() {
    let xml = envelope(
        "",
        &format!(
            "<m:ExecuteResponse xmlns:m=\"{XMLA}\"><return>\
             <root xmlns:xsd=\"{XSD}\">{SCHEMA}{ROWS}</root></return></m:ExecuteResponse>"
        ),
    );
    let response = read(&xml, None);
    assert_eq!(response.method(), Method::Execute);
    assert_eq!(response.rowset().map(Rowset::field), Some(&orders_field()));
    assert_eq!(response.rows(), Some(&two_orders()));
}

#[test]
fn a_return_in_another_namespace_is_refused_as_missing() {
    let xml = envelope(
        "",
        &format!(
            "<DiscoverResponse xmlns=\"{XMLA}\"><o:return xmlns:o=\"urn:example:other\">{}\
             </o:return></DiscoverResponse>",
            rowset_root(SCHEMA)
        ),
    );
    let (format, reason) = codec(refused(&xml, None));
    assert_eq!(format, "xml");
    assert_eq!(
        reason,
        "expected one `return` element under `DiscoverResponse`, found none"
    );
}

#[test]
fn a_response_name_is_matched_case_sensitively() {
    for name in ["discoverResponse", "EXECUTERESPONSE", "Executeresponse"] {
        let xml = envelope("", &response(name, &rowset_root(SCHEMA)));
        let (path, reason) = invalid_record(refused(&xml, None));
        assert_eq!(path, "$.xmla");
        assert_eq!(
            reason,
            format!(
                "expected `DiscoverResponse` or `ExecuteResponse` in \
                 \"urn:schemas-microsoft-com:xml-analysis\", got `{name}`"
            )
        );
    }
}

#[test]
fn a_prefixed_root_in_the_empty_namespace_answers_nothing_whatever_it_holds() {
    let xml = envelope(
        "",
        &response(
            "ExecuteResponse",
            &format!("<e:root xmlns:e=\"{EMPTY}\" note=\"done\"><e:note>3 rows</e:note></e:root>"),
        ),
    );
    let response = read(&xml, None);
    assert_eq!(response.method(), Method::Execute);
    assert_eq!(response.answer(), &Answer::Empty);
}

#[test]
fn an_empty_dataset_root_is_kept_as_a_dataset() {
    let xml = envelope(
        "",
        &response("ExecuteResponse", &format!("<root xmlns=\"{MDDATASET}\"/>")),
    );
    let response = read(&xml, Some(&orders_field()));
    let Answer::Dataset(fragment) = response.answer() else {
        panic!("expected a dataset, got {:?}", response.answer());
    };
    assert_eq!(fragment.name(), "root");
    assert_eq!(fragment.element().namespace(), Some(MDDATASET));
    assert_eq!(fragment.element().children().count(), 0);
    assert!(response.rows().is_none());
}

#[test]
fn cells_marked_nil_self_closed_or_emptied_read_as_null_null_and_the_empty_text() {
    let rows = format!(
        "<row><Order_x0020_Id>1</Order_x0020_Id><Symbol xsi:nil=\"true\"/></row>\
         <row><Order_x0020_Id>2</Order_x0020_Id><Symbol i:nil=\"1\" xmlns:i=\"{XSI}\"/></row>\
         <row><Order_x0020_Id>3</Order_x0020_Id><Symbol/></row>\
         <row><Order_x0020_Id>4</Order_x0020_Id><Symbol></Symbol></row>\
         <row><Order_x0020_Id> 5 </Order_x0020_Id><Symbol>  </Symbol></row>\
         <row><Order_x0020_Id>6</Order_x0020_Id><Symbol>a &amp; &lt;b&gt; &#x1F680;</Symbol></row>"
    );
    let xml = envelope(
        "",
        &response("ExecuteResponse", &rowset_root(&format!("{SCHEMA}{rows}"))),
    );
    assert_eq!(
        read(&xml, None).rows(),
        Some(&orders([
            order(1, None),
            order(2, None),
            order(3, None),
            order(4, Some("")),
            order(5, Some("  ")),
            order(6, Some("a & <b> 🚀")),
        ]))
    );
}

#[test]
fn a_row_missing_a_required_column_is_refused_naming_the_row() {
    let rows = "<row><Order_x0020_Id>7</Order_x0020_Id></row><row><Symbol>X</Symbol></row>";
    let xml = envelope(
        "",
        &response("ExecuteResponse", &rowset_root(&format!("{SCHEMA}{rows}"))),
    );
    let (path, reason) = invalid_record(refused(&xml, None));
    assert_eq!(path, "$[1]");
    assert!(
        reason.contains("`Order Id` is missing from the row"),
        "{reason}"
    );
}

#[test]
fn an_error_the_provider_reported_inside_the_rowset_is_refused_naming_it() {
    for reported in [
        format!(
            "<Messages xmlns=\"{EXCEPTION}\"><Error ErrorCode=\"3238658057\" \
             Description=\"the query was cancelled\" Source=\"engine\" HelpFile=\"\"/></Messages>"
        ),
        format!(
            "<EX:Error xmlns:EX=\"{EXCEPTION}\" ErrorCode=\"3238658057\">the query was cancelled\
             </EX:Error>"
        ),
    ] {
        let xml = envelope(
            "",
            &response(
                "ExecuteResponse",
                &rowset_root(&format!("{SCHEMA}{ROWS}{reported}")),
            ),
        );
        let (path, reason) = invalid_record(refused(&xml, None));
        assert_eq!(path, "$.rowset");
        assert_eq!(
            reason,
            "the provider reported an error inside the rowset: the query was cancelled (3238658057)"
        );
    }
}

// Writing.

#[test]
fn write_rowset_round_trips_the_header_the_method_the_field_and_the_rows() {
    let rows = orders([
        order(7, Some("AAPL")),
        order(8, None),
        order(-9, Some("Zürich – 東京 & <co>")),
    ]);
    for method in Method::ALL {
        let bytes = write_rowset(
            Vec::new(),
            &[session()],
            method,
            &orders_rowset(),
            [Ok(rows.clone())],
            Content::SchemaData,
        )
        .expect("the response is written");
        let response = Response::from_bytes(&bytes, None)
            .unwrap_or_else(|error| panic!("{error}\n{}", text(&bytes)));
        assert_eq!(response.method(), method);
        assert_eq!(
            Session::read(response.header()).expect("a session header"),
            Some(Session::Continue("sess-1".into()))
        );
        assert_eq!(
            response.rowset().map(Rowset::field),
            Some(orders_rowset().field())
        );
        assert_eq!(response.rows(), Some(&rows));
    }
}

#[test]
fn write_rowset_opens_the_response_element_in_the_xmla_namespace() {
    let bytes = write_rowset(
        Vec::new(),
        &[],
        Method::Discover,
        &orders_rowset(),
        [Ok(two_orders())],
        Content::SchemaData,
    )
    .expect("the response is written");
    let written = text(&bytes);
    assert!(
        written.starts_with("<?xml version=\"1.0\" encoding=\"utf-8\"?><SOAP-ENV:Envelope"),
        "{written}"
    );
    assert!(!written.contains("SOAP-ENV:Header"), "{written}");
    assert!(
        written.contains(&format!(
            "<SOAP-ENV:Body><DiscoverResponse xmlns=\"{XMLA}\"><return><root xmlns=\"{ROWSET}\""
        )),
        "{written}"
    );
    assert!(
        written
            .ends_with("</root></return></DiscoverResponse></SOAP-ENV:Body></SOAP-ENV:Envelope>"),
        "{written}"
    );
    assert!(written.contains(ROWS), "{written}");
}

#[test]
fn write_rowset_writes_every_batch_in_order() {
    let first = orders([order(1, Some("A")), order(2, None)]);
    let second = orders([order(3, Some("C"))]);
    let bytes = write_rowset(
        Vec::new(),
        &[],
        Method::Execute,
        &orders_rowset(),
        [Ok(first), Ok(orders([])), Ok(second)],
        Content::SchemaData,
    )
    .expect("the response is written");
    assert_eq!(
        read(text(&bytes), None).rows(),
        Some(&orders([
            order(1, Some("A")),
            order(2, None),
            order(3, Some("C"))
        ]))
    );
}

#[test]
fn write_rowset_with_schema_content_writes_no_rows_and_pulls_none() {
    let bytes = write_rowset(
        Vec::new(),
        &[],
        Method::Execute,
        &orders_rowset(),
        never_pulled(),
        Content::Schema,
    )
    .expect("the response is written");
    let written = text(&bytes);
    assert!(written.contains("<xsd:schema"), "{written}");
    assert!(!written.contains("<row>"), "{written}");
    let response = read(written, None);
    assert_eq!(response.rowset().map(Rowset::field), Some(&orders_field()));
    assert_eq!(response.rows().map(Serie::len), Some(0));
}

#[test]
fn write_rowset_with_data_content_writes_no_schema() {
    let bytes = write_rowset(
        Vec::new(),
        &[session()],
        Method::Execute,
        &orders_rowset(),
        [Ok(two_orders())],
        Content::Data,
    )
    .expect("the response is written");
    let written = text(&bytes);
    assert!(!written.contains("xsd:schema"), "{written}");
    assert!(written.contains(ROWS), "{written}");

    let (_, reason) = invalid_record(refused(written, None));
    assert_eq!(
        reason,
        "the rowset carries no schema and no field is declared to read its rows by"
    );
    let response = read(written, Some(&orders_field()));
    assert_eq!(response.method(), Method::Execute);
    assert_eq!(
        Session::read(response.header()).expect("a session header"),
        Some(Session::Continue("sess-1".into()))
    );
    assert_eq!(response.rows(), Some(&two_orders()));
}

#[test]
fn write_rowset_with_no_content_writes_an_empty_root_and_pulls_no_rows() {
    let bytes = write_rowset(
        Vec::new(),
        &[],
        Method::Execute,
        &orders_rowset(),
        never_pulled(),
        Content::None,
    )
    .expect("the response is written");
    let written = text(&bytes);
    assert!(!written.contains("xsd:schema"), "{written}");
    assert!(!written.contains("<row>"), "{written}");
    assert!(
        written.contains(&format!("<root xmlns=\"{ROWSET}\"")),
        "{written}"
    );

    refused(written, None);
    let response = read(written, Some(&orders_field()));
    assert_eq!(response.rows().map(Serie::len), Some(0));
}

#[test]
fn write_rowset_streams_each_batch_as_it_is_pulled() {
    let mut output = Vec::new();
    let failure = yggdryl::arrow::Error::Unsupported {
        kind: "feed",
        reason: "the feed dropped".to_owned(),
    };
    let result = write_rowset(
        &mut output,
        &[],
        Method::Execute,
        &orders_rowset(),
        [Ok(orders([order(7, Some("AAPL"))])), Err(failure)]
            .into_iter()
            .chain(never_pulled()),
        Content::SchemaData,
    );
    let (path, reason) = invalid_record(result.expect_err("the failing batch is refused"));
    assert_eq!(path, "$.rows");
    assert!(reason.contains("the feed dropped"), "{reason}");
    // The first batch reached the sink before the second was pulled, and
    // nothing was closed after the failure.
    let written = text(&output);
    assert!(
        written.contains("<row><Order_x0020_Id>7</Order_x0020_Id><Symbol>AAPL</Symbol></row>"),
        "{written}"
    );
    assert!(!written.contains("</root>"), "{written}");
}

#[test]
fn write_rowset_refuses_a_batch_of_another_field_naming_the_column() {
    let other = StructType::from_fields([
        DataType::Int64.required_field("Order Id"),
        DataType::utf8().nullable_field("Symbol"),
    ])
    .map(DataType::from)
    .expect("a valid root")
    .required_field("row");
    let batch = Serie::from_scalars(
        other,
        [
            Scalar::from_struct([("Order Id", Scalar::from(7_i64)), ("Symbol", Scalar::Null)])
                .expect("distinct names"),
        ],
    )
    .expect("a batch");
    let result = write_rowset(
        Vec::new(),
        &[],
        Method::Execute,
        &orders_rowset(),
        [Ok(batch)],
        Content::SchemaData,
    );
    let (_, reason) = invalid_record(result.expect_err("the batch is refused"));
    assert!(reason.contains("column `Order Id`"), "{reason}");
}

#[test]
fn write_rowset_refuses_a_failing_sink() {
    let result = write_rowset(
        FullSink,
        &[],
        Method::Discover,
        &orders_rowset(),
        [Ok(two_orders())],
        Content::SchemaData,
    );
    assert_eq!(sink_failure(result.map(|_| ())), "the sink is full");
}

#[test]
fn write_empty_round_trips_the_header_and_the_method() {
    for method in Method::ALL {
        let bytes = write_empty(Vec::new(), &[session()], method).expect("the response is written");
        let written = text(&bytes);
        assert!(
            written.contains(&format!(
                "<{} xmlns=\"{XMLA}\"><return><root xmlns=\"{EMPTY}\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xmlns:xsd=\"http://www.w3.org/2001/XMLSchema\" xmlns:EX=\"urn:schemas-microsoft-com:xml-analysis:exception\"/></return></{}>",
                method.response_name(),
                method.response_name()
            )),
            "{written}"
        );
        let response = read(written, None);
        assert_eq!(response.method(), method);
        assert_eq!(response.answer(), &Answer::Empty);
        assert_eq!(
            Session::read(response.header()).expect("a session header"),
            Some(Session::Continue("sess-1".into()))
        );
    }
}

#[test]
fn write_empty_keeps_every_header_block() {
    let trace = Fragment::in_namespace("Trace", "urn:example:trace", Scalar::from("on"))
        .expect("a header block");
    let bytes = write_empty(Vec::new(), &[session(), trace], Method::Execute)
        .expect("the response is written");
    let response = read(text(&bytes), None);
    let mut names: Vec<&str> = response.header().iter().map(Fragment::name).collect();
    names.sort_unstable();
    assert_eq!(names, ["Session", "Trace"]);
    let trace = response
        .header()
        .iter()
        .find(|block| block.name() == "Trace")
        .expect("the trace block");
    assert_eq!(trace.element().namespace(), Some("urn:example:trace"));
    assert_eq!(trace.element().text(), Some("on"));
}

#[test]
fn write_empty_refuses_a_failing_sink() {
    let result = write_empty(FullSink, &[], Method::Execute);
    assert_eq!(sink_failure(result.map(|_| ())), "the sink is full");
}

#[test]
fn write_fault_reads_back_as_the_same_fault() {
    let error = XmlaError::new(3_238_658_057, "Le « cube » n'existe pas & <rien> – 東京")
        .with_source("catalog");
    let built = fault(FaultCode::Client, error.clone()).expect("a fault");
    let bytes = write_fault(Vec::new(), &built).expect("the fault is written");
    let written = text(&bytes);
    assert!(!written.contains("SOAP-ENV:Header"), "{written}");

    let read_back = read_fault(&bytes);
    assert_eq!(read_back.code(), &FaultCode::Client);
    assert_eq!(read_back.subcode(), None);
    assert_eq!(read_back.string(), error.description());
    assert_eq!(read_back.actor(), Some(ACTOR));
    assert_eq!(read_back.detail().len(), 1);
    assert_eq!(read_back.detail()[0].element().local_name(), "Error");
    assert_eq!(read_back.detail()[0].element().namespace(), Some(EXCEPTION));
    assert_eq!(XmlaError::from_fault(&read_back), vec![error.clone()]);

    let (path, reason) = invalid_record(refused(written, None));
    assert_eq!(path, "$.xmla");
    assert_eq!(
        reason,
        format!(
            "SOAP fault SOAP-ENV:Client: {} ({} {})",
            error.description(),
            error.code(),
            error.description()
        )
    );
}

#[test]
fn write_fault_keeps_a_subcode_and_several_errors_in_order() {
    let first = XmlaError::new(1, "first");
    let second = XmlaError::new(2, "second").with_source("engine");
    let built = Fault::server("two things went wrong")
        .with_subcode("Engine")
        .with_actor(ACTOR)
        .with_detail(first.into_fragment().expect("an Error element"))
        .with_detail(second.into_fragment().expect("an Error element"));
    let bytes = write_fault(Vec::new(), &built).expect("the fault is written");
    let read_back = read_fault(&bytes);
    assert_eq!(read_back.code(), &FaultCode::Server);
    assert_eq!(read_back.subcode(), Some("Engine"));
    assert_eq!(read_back.string(), "two things went wrong");
    assert_eq!(read_back.actor(), Some(ACTOR));
    assert_eq!(XmlaError::from_fault(&read_back), vec![first, second]);

    let (_, reason) = invalid_record(refused(text(&bytes), None));
    assert_eq!(
        reason,
        "SOAP fault SOAP-ENV:Server.Engine: two things went wrong (1 first)"
    );
}

#[test]
fn write_fault_refuses_a_failing_sink() {
    let built = fault(FaultCode::Server, XmlaError::new(1, "x")).expect("a fault");
    let result = write_fault(FullSink, &built);
    assert_eq!(sink_failure(result.map(|_| ())), "the sink is full");
}

#[test]
fn write_rowset_refuses_a_cell_xml_cannot_carry_naming_its_row_and_column() {
    let result = write_rowset(
        Vec::new(),
        &[],
        Method::Execute,
        &orders_rowset(),
        [Ok(orders([
            order(7, Some("AAPL")),
            order(8, Some("bell \u{7}")),
        ]))],
        Content::SchemaData,
    );
    let (path, reason) = invalid_record(result.expect_err("the cell is refused"));
    assert_eq!(path, "$[1].Symbol");
    assert!(
        reason.ends_with("text carries U+0007, which XML 1.0 cannot spell"),
        "{reason}"
    );
}

#[test]
fn write_empty_refuses_a_header_block_xml_cannot_carry() {
    let block = Fragment::in_namespace("Trace", "urn:example:trace", Scalar::from("bell \u{7}"))
        .expect("a header block");
    let (format, reason) = codec(
        write_empty(Vec::new(), &[block], Method::Execute)
            .expect_err("the header block has no XML spelling"),
    );
    assert_eq!(format, "xml");
    assert!(reason.contains("U+0007"), "{reason}");
}

#[test]
fn write_fault_spells_the_fault_in_the_envelope_schema_s_order() {
    let built = fault(FaultCode::Client, XmlaError::new(3, "desc & <x>")).expect("a fault");
    let bytes = write_fault(Vec::new(), &built).expect("the fault is written");
    let written = text(&bytes);
    assert!(
        written.starts_with(&format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?><SOAP-ENV:Envelope xmlns:SOAP-ENV=\"{SOAP}\">\
             <SOAP-ENV:Body><SOAP-ENV:Fault><faultcode>SOAP-ENV:Client</faultcode>\
             <faultstring>desc &amp; &lt;x&gt;</faultstring><faultactor>yggdryl</faultactor>\
             <detail><Error "
        )),
        "{written}"
    );
    assert!(
        written.contains(&format!("xmlns=\"{EXCEPTION}\"")),
        "{written}"
    );
    assert!(
        written.ends_with("/></detail></SOAP-ENV:Fault></SOAP-ENV:Body></SOAP-ENV:Envelope>"),
        "{written}"
    );
}

#[test]
fn write_fault_refuses_a_description_xml_cannot_carry() {
    // `fault` builds a fault of any text; the text XML 1.0 cannot spell is
    // refused where it is written, naming the code point.
    let built = fault(FaultCode::Client, XmlaError::new(3, "bell \u{7}")).expect("a fault");
    let (format, reason) =
        codec(write_fault(Vec::new(), &built).expect_err("the fault string has no XML spelling"));
    assert_eq!(format, "xml");
    assert_eq!(reason, "text carries U+0007, which XML 1.0 cannot spell");
}

// `write_rowset_reporting`.

#[test]
fn write_rowset_reporting_writes_what_write_rowset_writes_when_nothing_fails() {
    for content in [Content::SchemaData, Content::Data] {
        let expected = write_rowset(
            Vec::new(),
            &[session()],
            Method::Discover,
            &orders_rowset(),
            [Ok(two_orders())],
            content,
        )
        .expect("the response is written");
        let (bytes, failed) = write_rowset_reporting(
            Vec::new(),
            &[session()],
            Method::Discover,
            &orders_rowset(),
            [Ok(two_orders())],
            content,
        )
        .expect("the response is written");
        assert_eq!(failed, None);
        assert_eq!(text(&bytes), text(&expected));
    }

    let (bytes, failed) = write_rowset_reporting(
        Vec::new(),
        &[],
        Method::Execute,
        &orders_rowset(),
        never_pulled(),
        Content::Schema,
    )
    .expect("the response is written");
    assert_eq!(failed, None);
    assert_eq!(read(text(&bytes), None).rows().map(Serie::len), Some(0));
}

#[test]
fn write_rowset_reporting_reports_a_failing_batch_inside_the_root_and_closes_the_document() {
    let failure = yggdryl::arrow::Error::Unsupported {
        kind: "feed",
        reason: "the feed dropped".to_owned(),
    };
    let batches = [Ok(orders([order(7, Some("AAPL"))])), Err(failure)]
        .into_iter()
        .chain(never_pulled());
    let (bytes, failed) = write_rowset_reporting(
        Vec::new(),
        &[],
        Method::Execute,
        &orders_rowset(),
        batches,
        Content::SchemaData,
    )
    .expect("the failure is reported, not raised");
    let failed = failed.expect("the failure is handed back");
    assert_eq!(failed.code(), EXECUTION_FAILED);
    assert_eq!(failed.source(), ACTOR);
    assert_eq!(failed.help_file(), "");
    assert!(
        failed
            .description()
            .starts_with("invalid record value at $.rows: ")
    );
    assert!(failed.description().ends_with("the feed dropped"));

    let written = text(&bytes);
    assert!(
        written.ends_with(&format!(
            "<row><Order_x0020_Id>7</Order_x0020_Id><Symbol>AAPL</Symbol></row>\
             <Messages xmlns=\"{EXCEPTION}\"><Error ErrorCode=\"10\" Description=\"{}\" \
             Source=\"yggdryl\" HelpFile=\"\"/></Messages>\
             </root></return></ExecuteResponse></SOAP-ENV:Body></SOAP-ENV:Envelope>",
            failed.description()
        )),
        "{written}"
    );
    // The document is whole, and a client reading it is told what failed.
    let (path, reason) = invalid_record(refused(written, None));
    assert_eq!(path, "$.rowset");
    assert_eq!(
        reason,
        format!(
            "the provider reported an error inside the rowset: {} (10)",
            failed.description()
        )
    );
}

#[test]
fn write_rowset_reporting_reports_a_batch_of_another_field_before_any_of_its_rows() {
    let other = StructType::from_fields([
        DataType::Int64.required_field("Order Id"),
        DataType::utf8().nullable_field("Symbol"),
    ])
    .map(DataType::from)
    .expect("a valid root")
    .required_field("row");
    let foreign = Serie::from_scalars(
        other,
        [
            Scalar::from_struct([("Order Id", Scalar::from(9_i64)), ("Symbol", Scalar::Null)])
                .expect("distinct names"),
        ],
    )
    .expect("a batch");
    let (bytes, failed) = write_rowset_reporting(
        Vec::new(),
        &[],
        Method::Execute,
        &orders_rowset(),
        [Ok(two_orders()), Ok(foreign)],
        Content::SchemaData,
    )
    .expect("the failure is reported, not raised");
    let failed = failed.expect("the failure is handed back");
    assert!(
        failed.description().contains("column `Order Id`"),
        "{failed:?}"
    );
    let written = text(&bytes);
    assert!(written.contains(ROWS), "{written}");
    assert!(
        !written.contains("<Order_x0020_Id>9</Order_x0020_Id>"),
        "{written}"
    );
    let (path, _) = invalid_record(refused(written, None));
    assert_eq!(path, "$.rowset");
}

#[test]
fn write_rowset_reporting_never_leaves_a_cell_xml_cannot_carry_in_an_ill_formed_document() {
    // The rustdoc: a failure after the answer has begun is reported and "the
    // document is closed, so the client reads a complete answer naming the
    // failure", and a cell with no XML spelling is an error "before any row
    // of a batch is written". Either way, what reaches the sink is never a
    // row cut open in the middle.
    let result = write_rowset_reporting(
        Vec::new(),
        &[],
        Method::Execute,
        &orders_rowset(),
        [Ok(orders([
            order(7, Some("AAPL")),
            order(8, Some("bell \u{7}")),
        ]))],
        Content::SchemaData,
    );
    match result {
        Err(error) => {
            let (path, _) = invalid_record(error);
            assert_eq!(path, "$[1].Symbol");
        }
        Ok((bytes, failed)) => {
            assert!(failed.is_some(), "the failure is handed back");
            let message = Envelope::from_bytes(&bytes)
                .unwrap_or_else(|error| panic!("{error}\n{}", text(&bytes)));
            assert!(message.payload().is_some(), "{}", text(&bytes));
        }
    }
}

#[test]
fn write_rowset_reporting_refuses_a_failing_sink() {
    let result = write_rowset_reporting(
        FullSink,
        &[],
        Method::Discover,
        &orders_rowset(),
        [Ok(two_orders())],
        Content::SchemaData,
    );
    assert_eq!(sink_failure(result.map(|_| ())), "the sink is full");
}
