//! `rust/src/soap/mod.rs`: the SOAP 1.1 envelope - its constants, the
//! fragments a header block and a body element are, the fault that stands in
//! for a body, the envelope's reading from bytes and its natural value, and
//! the whole and streaming writers.
//!
//! `codec_error` is crate-private and pinned only through the refusals it
//! spells, whose `format` is `soap`.

use std::io::Write;

use yggdryl::soap::{
    ACTION_HEADER, Body, CONTENT_TYPE, ENCODING_NAMESPACE, ENVELOPE_NAMESPACE, Envelope,
    EnvelopeWriter, Fault, FaultCode, Fragment, PREFIX,
};
use yggdryl::{Error, Limits, Scalar};

const DECLARATION: &str = "<?xml version=\"1.0\" encoding=\"utf-8\"?>";
const XMLA: &str = "urn:schemas-microsoft-com:xml-analysis";
const SOAP12: &str = "http://www.w3.org/2003/05/soap-envelope";

fn record<const N: usize>(entries: [(&str, Scalar); N]) -> Scalar {
    Scalar::from_struct(entries).expect("distinct names")
}

fn text(value: &str) -> Scalar {
    Scalar::from(value)
}

/// `local` under `prefix`, the empty prefix spelling it unqualified.
fn qualified(prefix: &str, local: &str) -> String {
    if prefix.is_empty() {
        local.to_owned()
    } else {
        format!("{prefix}:{local}")
    }
}

/// An envelope bound to the SOAP 1.1 namespace under `prefix` - the empty
/// prefix declaring it as the default namespace - holding `header` (when
/// not empty) and `body` as literal markup.
fn envelope(prefix: &str, header: &str, body: &str) -> String {
    let declaration = if prefix.is_empty() {
        format!("xmlns=\"{ENVELOPE_NAMESPACE}\"")
    } else {
        format!("xmlns:{prefix}=\"{ENVELOPE_NAMESPACE}\"")
    };
    let root = qualified(prefix, "Envelope");
    let head = if header.is_empty() {
        String::new()
    } else {
        let name = qualified(prefix, "Header");
        format!("<{name}>{header}</{name}>")
    };
    let name = qualified(prefix, "Body");
    format!("<{root} {declaration}>{head}<{name}>{body}</{name}></{root}>")
}

fn read(xml: &str) -> Envelope {
    Envelope::from_bytes(xml.as_bytes()).unwrap_or_else(|error| panic!("{error}\n{xml}"))
}

/// The codec refusal `input` earns: its format and its reason.
fn refusal(result: yggdryl::Result<Envelope>) -> (&'static str, String) {
    match result {
        Err(Error::Codec { format, reason, .. }) => (format, reason.to_string()),
        Err(other) => panic!("expected a codec refusal, got {other}"),
        Ok(envelope) => panic!("expected a refusal, read {envelope:?}"),
    }
}

fn refused(xml: &str) -> (&'static str, String) {
    refusal(Envelope::from_bytes(xml.as_bytes()))
}

fn utf8(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).expect("the writer writes UTF-8")
}

/// Where `needle` first occurs in `haystack`, panicking with both when it
/// does not.
fn at(haystack: &str, needle: &str) -> usize {
    haystack
        .find(needle)
        .unwrap_or_else(|| panic!("{needle:?} is not in {haystack}"))
}

/// A sink that refuses every write once closed, reached through
/// `EnvelopeWriter::body` to close it mid-message.
struct Gate {
    open: bool,
    bytes: Vec<u8>,
}

impl Write for Gate {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        if self.open {
            self.bytes.extend_from_slice(buffer);
            Ok(buffer.len())
        } else {
            Err(std::io::Error::other("the sink is closed"))
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The payload the XMLA examples send: `Discover` in the XMLA namespace.
fn discover() -> Fragment {
    Fragment::in_namespace(
        "Discover",
        XMLA,
        record([("RequestType", text("DISCOVER_DATASOURCES"))]),
    )
    .expect("a record takes a default namespace")
}

/// A header block in its own namespace, marked `mustUnderstand`.
fn transaction() -> Fragment {
    Fragment::new(
        "t:Transaction",
        record([
            ("@xmlns:t", text("urn:example:transaction")),
            ("@SOAP-ENV:mustUnderstand", text("1")),
            ("#text", text("5")),
        ]),
    )
}

/// A fault carrying every part SOAP 1.1 gives one.
fn full_fault() -> Fault {
    Fault::client("bad password")
        .with_subcode("Authentication")
        .with_actor("http://example.com/xmla")
        .with_detail(Fragment::new(
            "e:Reason",
            record([
                ("@xmlns:e", text("urn:example:error")),
                ("#text", text("expired")),
            ]),
        ))
}

#[test]
fn the_constants_spell_soap_1_1_and_its_http_binding() {
    assert_eq!(
        ENVELOPE_NAMESPACE,
        "http://schemas.xmlsoap.org/soap/envelope/"
    );
    assert_eq!(
        ENCODING_NAMESPACE,
        "http://schemas.xmlsoap.org/soap/encoding/"
    );
    assert_eq!(PREFIX, "SOAP-ENV");
    assert_eq!(CONTENT_TYPE, "text/xml; charset=utf-8");
    assert_eq!(ACTION_HEADER, "SOAPAction");
}

#[test]
fn a_document_element_other_than_envelope_is_refused_naming_it() {
    let xml = format!(
        "<soap:Letter xmlns:soap=\"{ENVELOPE_NAMESPACE}\"><soap:Body><m:A xmlns:m=\"urn:m\"/></soap:Body></soap:Letter>"
    );
    let (format, reason) = refused(&xml);
    assert_eq!(format, "soap");
    assert_eq!(
        reason,
        format!(
            "expected the SOAP 1.1 `Envelope` in {ENVELOPE_NAMESPACE:?}, got `soap:Letter` in {ENVELOPE_NAMESPACE:?}"
        )
    );
}

#[test]
fn an_envelope_in_another_namespace_is_refused_naming_both_namespaces() {
    let soap12 = format!(
        "<env:Envelope xmlns:env=\"{SOAP12}\"><env:Body><m:A xmlns:m=\"urn:m\"/></env:Body></env:Envelope>"
    );
    let (format, reason) = refused(&soap12);
    assert_eq!(format, "soap");
    assert_eq!(
        reason,
        format!(
            "expected the SOAP 1.1 `Envelope` in {ENVELOPE_NAMESPACE:?}, got `env:Envelope` in {SOAP12:?}"
        )
    );

    let (_, reason) = refused("<Envelope><Body><A/></Body></Envelope>");
    assert!(
        reason.ends_with("got `Envelope` in no namespace"),
        "{reason}"
    );

    // A prefix declared nowhere puts the element in no namespace, whatever
    // the prefix happens to be spelled as.
    let (_, reason) =
        refused("<SOAP-ENV:Envelope><SOAP-ENV:Body><A/></SOAP-ENV:Body></SOAP-ENV:Envelope>");
    assert!(
        reason.ends_with("got `SOAP-ENV:Envelope` in no namespace"),
        "{reason}"
    );
}

#[test]
fn an_envelope_without_exactly_one_body_is_refused_naming_the_body() {
    let none = format!("<soap:Envelope xmlns:soap=\"{ENVELOPE_NAMESPACE}\"/>");
    let (format, reason) = refused(&none);
    assert_eq!(format, "xml", "the count is the element view's refusal");
    assert_eq!(
        reason,
        "expected one `Body` element under `soap:Envelope`, found none"
    );

    let header_only =
        envelope("soap", "<h:A xmlns:h=\"urn:h\"/>", "").replace("<soap:Body></soap:Body>", "");
    let (_, reason) = refused(&header_only);
    assert_eq!(
        reason,
        "expected one `Body` element under `soap:Envelope`, found none"
    );

    let twice = format!(
        "<soap:Envelope xmlns:soap=\"{ENVELOPE_NAMESPACE}\"><soap:Body><m:A xmlns:m=\"urn:m\"/></soap:Body><soap:Body><m:B xmlns:m=\"urn:m\"/></soap:Body></soap:Envelope>"
    );
    let (_, reason) = refused(&twice);
    assert_eq!(
        reason,
        "expected one `Body` element under `soap:Envelope`, found several"
    );

    // A body in another namespace is not the envelope's body.
    let foreign = format!(
        "<soap:Envelope xmlns:soap=\"{ENVELOPE_NAMESPACE}\" xmlns:x=\"urn:x\"><x:Body><m:A xmlns:m=\"urn:m\"/></x:Body></soap:Envelope>"
    );
    let (_, reason) = refused(&foreign);
    assert!(reason.ends_with("found none"), "{reason}");
}

#[test]
fn a_body_holding_no_element_is_refused() {
    for body in ["", "   ", "just text", "<![CDATA[<m:A/>]]>"] {
        let (format, reason) = refused(&envelope("soap", "", body));
        assert_eq!(format, "soap", "{body:?}");
        assert_eq!(reason, "the SOAP body holds no element", "{body:?}");
    }
    let self_closed =
        format!("<soap:Envelope xmlns:soap=\"{ENVELOPE_NAMESPACE}\"><soap:Body/></soap:Envelope>");
    assert_eq!(refused(&self_closed).1, "the SOAP body holds no element");

    let attributes_only = format!(
        "<soap:Envelope xmlns:soap=\"{ENVELOPE_NAMESPACE}\"><soap:Body soap:encodingStyle=\"{ENCODING_NAMESPACE}\"/></soap:Envelope>"
    );
    assert_eq!(
        refused(&attributes_only).1,
        "the SOAP body holds no element",
        "an attribute is not an element"
    );
}

#[test]
fn a_body_holding_several_elements_is_refused() {
    let several = "the SOAP body holds several elements where one method call is expected";
    for body in [
        "<m:A xmlns:m=\"urn:m\"/><m:B xmlns:m=\"urn:m\"/>",
        "<m:A xmlns:m=\"urn:m\"/><m:A xmlns:m=\"urn:m\"/>",
        "<m:A xmlns:m=\"urn:m\"/><soap:Fault><faultcode>soap:Server</faultcode><faultstring>x</faultstring></soap:Fault>",
    ] {
        let (format, reason) = refused(&envelope("soap", "", body));
        assert_eq!(format, "soap", "{body}");
        assert_eq!(reason, several, "{body}");
    }
}

#[test]
fn a_fault_without_a_faultcode_is_refused() {
    for fault in [
        "<soap:Fault><faultstring>no code</faultstring></soap:Fault>",
        "<soap:Fault><faultcode/><faultstring>a nil code</faultstring></soap:Fault>",
        "<soap:Fault/>",
        "<soap:Fault xmlns:x=\"urn:x\"><x:faultcode>soap:Server</x:faultcode><faultstring>a code in another namespace</faultstring></soap:Fault>",
    ] {
        let (format, reason) = refused(&envelope("soap", "", fault));
        assert_eq!(format, "soap", "{fault}");
        assert_eq!(reason, "a SOAP fault without a `faultcode`", "{fault}");
    }
}

#[test]
fn a_fault_without_a_faultstring_reads_as_the_empty_string() {
    let read = read(&envelope(
        "soap",
        "",
        "<soap:Fault><faultcode>soap:Server</faultcode></soap:Fault>",
    ));
    let fault = read.fault().expect("a fault");
    assert_eq!(fault.code(), &FaultCode::Server);
    assert_eq!(fault.string(), "");
    assert_eq!(fault.actor(), None);
    assert!(fault.detail().is_empty());
}

#[test]
fn bytes_that_are_not_xml_are_refused_by_the_xml_codec() {
    match Envelope::from_bytes(b"<soap:Envelope \xff/>") {
        Err(Error::Codec { format, reason, .. }) => {
            assert_eq!(format, "xml");
            assert_eq!(reason, "input is not valid UTF-8");
        }
        other => panic!("expected a UTF-8 refusal, got {other:?}"),
    }
    for markup in [
        "",
        "not markup",
        "<soap:Envelope xmlns:soap=\"http://schemas.xmlsoap.org/soap/envelope/\">",
        "<a></b>",
    ] {
        let (format, _) = refusal(Envelope::from_bytes(markup.as_bytes()));
        assert_eq!(format, "xml", "{markup:?}");
    }
}

#[test]
fn limits_bound_the_bytes_depth_nodes_and_documents_an_envelope_reads() {
    let xml = envelope(
        "soap",
        "",
        "<m:Get xmlns:m=\"urn:m\"><symbol>DIS</symbol></m:Get>",
    );
    let bytes = xml.as_bytes();
    let generous = Limits::new(8, bytes.len(), 64, 1);
    let read = Envelope::from_bytes_with_limits(bytes, generous).expect("within every limit");
    assert_eq!(read.payload().expect("a payload").name(), "m:Get");

    match Envelope::from_bytes_with_limits(bytes, Limits::new(8, bytes.len() - 1, 64, 1)) {
        Err(Error::Codec {
            format,
            position,
            reason,
        }) => {
            assert_eq!(format, "xml");
            assert_eq!(position, bytes.len() - 1, "the position is the limit");
            assert_eq!(reason, "input byte limit exceeded");
        }
        other => panic!("expected the byte limit, got {other:?}"),
    }

    // Envelope, Body, Get and symbol are four levels.
    let (_, reason) = refusal(Envelope::from_bytes_with_limits(
        bytes,
        Limits::new(3, bytes.len(), 64, 1),
    ));
    assert_eq!(reason, "nesting depth limit exceeded");
    assert!(Envelope::from_bytes_with_limits(bytes, Limits::new(4, bytes.len(), 64, 1)).is_ok());

    let (_, reason) = refusal(Envelope::from_bytes_with_limits(
        bytes,
        Limits::new(8, bytes.len(), 2, 1),
    ));
    assert_eq!(reason, "decoded node limit exceeded");

    let (_, reason) = refusal(Envelope::from_bytes_with_limits(
        bytes,
        Limits::new(8, bytes.len(), 64, 0),
    ));
    assert_eq!(reason, "document limit exceeded");
}

#[test]
fn a_natural_value_that_is_no_document_is_refused() {
    let (format, reason) = refusal(Envelope::from_natural(&Scalar::from(1)));
    assert_eq!(format, "xml");
    assert!(
        reason.starts_with("expected an XML document, the record naming its document element"),
        "{reason}"
    );

    let (_, reason) = refusal(Envelope::from_natural(&record([])));
    assert!(reason.ends_with("got an empty record"), "{reason}");

    let (_, reason) = refusal(Envelope::from_natural(&record([
        ("a", Scalar::Null),
        ("b", Scalar::Null),
    ])));
    assert!(reason.ends_with("got several entries"), "{reason}");
}

#[test]
fn an_envelope_reads_under_any_prefix_bound_to_the_envelope_namespace() {
    for prefix in ["SOAP-ENV", "soap", "s", "env", ""] {
        let xml = envelope(
            prefix,
            "",
            &format!(
                "<Discover xmlns=\"{XMLA}\"><RequestType>DISCOVER_DATASOURCES</RequestType></Discover>"
            ),
        );
        let read = read(&xml);
        assert!(read.header().is_empty(), "{prefix:?}");
        assert!(read.fault().is_none(), "{prefix:?}");
        let payload = read.payload().expect("a payload");
        assert_eq!(payload.name(), "Discover", "{prefix:?}");
        let element = payload.element();
        assert_eq!(element.namespace(), Some(XMLA), "{prefix:?}");
        let request_type = element
            .child(Some(XMLA), "RequestType")
            .expect("the request type");
        assert_eq!(
            request_type.text(),
            Some("DISCOVER_DATASOURCES"),
            "{prefix:?}"
        );
    }
}

#[test]
fn a_payload_resolves_prefixes_declared_on_the_envelope() {
    let xml = format!(
        "<s:Envelope xmlns:s=\"{ENVELOPE_NAMESPACE}\" xmlns:m=\"urn:example:market\">\
         <s:Body><m:GetPrice><m:symbol>DIS</m:symbol></m:GetPrice></s:Body></s:Envelope>"
    );
    let read = read(&xml);
    let payload = read.payload().expect("a payload");
    assert_eq!(payload.name(), "m:GetPrice", "the name keeps its prefix");
    assert_eq!(
        payload.value(),
        &record([("m:symbol", text("DIS"))]),
        "the value is the natural value, with no declaration in it"
    );
    let element = payload.element();
    assert_eq!(element.local_name(), "GetPrice");
    assert_eq!(element.prefix(), Some("m"));
    assert_eq!(element.namespace(), Some("urn:example:market"));
    assert_eq!(
        element.scope().resolve("s"),
        Some(ENVELOPE_NAMESPACE),
        "the envelope's declarations stay in scope"
    );
    let symbol = element
        .child(Some("urn:example:market"), "symbol")
        .expect("the symbol");
    assert_eq!(symbol.text(), Some("DIS"));
}

#[test]
fn the_soap_1_1_request_example_reads_with_its_encoding_style() {
    // SOAP 1.1, section 6.3, verbatim but for its indentation.
    let xml = format!(
        "<SOAP-ENV:Envelope\n  xmlns:SOAP-ENV=\"{ENVELOPE_NAMESPACE}\"\n  \
         SOAP-ENV:encodingStyle=\"{ENCODING_NAMESPACE}\">\n   <SOAP-ENV:Body>\n       \
         <m:GetLastTradePrice xmlns:m=\"Some-URI\">\n           <symbol>DIS</symbol>\n       \
         </m:GetLastTradePrice>\n   </SOAP-ENV:Body>\n</SOAP-ENV:Envelope>"
    );
    let read = read(&xml);
    let payload = read.payload().expect("a payload");
    assert_eq!(payload.name(), "m:GetLastTradePrice");
    let element = payload.element();
    assert_eq!(element.namespace(), Some("Some-URI"));
    assert_eq!(
        element
            .child(None, "symbol")
            .expect("an unqualified symbol")
            .text(),
        Some("DIS"),
        "indentation between elements is dropped"
    );

    let document = yggdryl::xml::from_bytes(xml.as_bytes()).expect("XML");
    let root = yggdryl::xml::Element::root(&document).expect("a root");
    assert_eq!(
        root.attribute_in(Some(ENVELOPE_NAMESPACE), "encodingStyle"),
        Some(ENCODING_NAMESPACE)
    );
}

#[test]
fn a_header_block_keeps_its_must_understand_attribute_and_scope() {
    let xml = envelope(
        "soap",
        "<t:Transaction xmlns:t=\"urn:example:transaction\" soap:mustUnderstand=\"1\">5</t:Transaction>",
        "<m:Get xmlns:m=\"urn:m\"/>",
    );
    let read = read(&xml);
    let [block] = read.header() else {
        panic!("one header block, got {:?}", read.header());
    };
    assert_eq!(block.name(), "t:Transaction");
    let element = block.element();
    assert_eq!(element.namespace(), Some("urn:example:transaction"));
    assert_eq!(element.text(), Some("5"));
    assert_eq!(
        element.attribute_in(Some(ENVELOPE_NAMESPACE), "mustUnderstand"),
        Some("1"),
        "the `soap` prefix the envelope declared resolves at the block"
    );
    assert_eq!(element.attribute("soap:mustUnderstand"), Some(&text("1")));

    let payload = read.payload().expect("a payload");
    assert_eq!(payload.name(), "m:Get");
    assert_eq!(
        payload.value(),
        &record([("@xmlns:m", text("urn:m"))]),
        "a self-closed payload holds only its declaration"
    );
    assert_eq!(payload.element().text(), None);
}

#[test]
fn an_unqualified_body_is_the_envelopes_and_an_unqualified_fault_is_a_payload() {
    let xml = format!(
        "<soap:Envelope xmlns:soap=\"{ENVELOPE_NAMESPACE}\"><Body><Fault>application-level</Fault></Body></soap:Envelope>"
    );
    let read = read(&xml);
    assert!(
        read.fault().is_none(),
        "a `Fault` in no namespace is not SOAP's"
    );
    let payload = read.payload().expect("a payload");
    assert_eq!(payload.name(), "Fault");
    assert_eq!(payload.element().text(), Some("application-level"));
}

#[test]
fn the_four_fault_codes_read_under_any_envelope_prefix() {
    for prefix in ["SOAP-ENV", "soap", "s", "env", ""] {
        for (spelled, code) in [
            ("VersionMismatch", FaultCode::VersionMismatch),
            ("MustUnderstand", FaultCode::MustUnderstand),
            ("Client", FaultCode::Client),
            ("Server", FaultCode::Server),
        ] {
            let fault = format!(
                "<{fault}><faultcode>{code}</faultcode><faultstring>SOAP {spelled} Error</faultstring></{fault}>",
                fault = qualified(prefix, "Fault"),
                code = qualified(prefix, spelled),
            );
            let read = read(&envelope(prefix, "", &fault));
            let held = read.fault().expect("a fault");
            assert_eq!(held.code(), &code, "{prefix:?} {spelled}");
            assert_eq!(held.subcode(), None, "{prefix:?} {spelled}");
            assert_eq!(held.string(), format!("SOAP {spelled} Error"));
            assert!(read.payload().is_none());
        }
    }
}

#[test]
fn an_unprefixed_or_padded_faultcode_is_read_as_the_envelopes() {
    for spelled in ["Client", "  Client  ", "\n soap:Client\t"] {
        let fault = format!(
            "<soap:Fault><faultcode>{spelled}</faultcode><faultstring>x</faultstring></soap:Fault>"
        );
        let read = read(&envelope("soap", "", &fault));
        assert_eq!(
            read.fault().expect("a fault").code(),
            &FaultCode::Client,
            "{spelled:?}"
        );
    }
}

#[test]
fn a_dotted_faultcode_reads_as_code_and_subcode() {
    let fault = "<env:Fault><faultcode>env:Client.Authentication</faultcode><faultstring>who are you</faultstring></env:Fault>";
    let read = read(&envelope("env", "", fault));
    let held = read.fault().expect("a fault");
    assert_eq!(held.code(), &FaultCode::Client);
    assert_eq!(held.subcode(), Some("Authentication"));
    assert_eq!(
        held.to_string(),
        "SOAP fault SOAP-ENV:Client.Authentication: who are you",
        "the code displays under the prefix the writer uses, not the one read"
    );

    // Only the first dot splits: the rest is the subcode as spelled.
    let fault = "<env:Fault><faultcode>env:Server.Storage.Full</faultcode><faultstring>x</faultstring></env:Fault>";
    let read = self::read(&envelope("env", "", fault));
    let held = read.fault().expect("a fault");
    assert_eq!(held.code(), &FaultCode::Server);
    assert_eq!(held.subcode(), Some("Storage.Full"));
}

#[test]
fn a_faultcode_outside_the_envelope_namespace_is_kept_as_spelled() {
    for (spelled, why) in [
        ("x:Throttled", "a prefix bound to another namespace"),
        (
            "x:Throttled.Hard",
            "a dotted code in another namespace keeps its dot",
        ),
        ("undeclared:Server", "a prefix bound nowhere"),
        ("soap:Bogus", "a local name SOAP 1.1 does not define"),
        (
            "soap:Bogus.Sub",
            "an undefined code keeps its subcode spelled",
        ),
        ("Bogus", "an unprefixed code SOAP 1.1 does not define"),
    ] {
        let fault = format!(
            "<soap:Fault xmlns:x=\"urn:example:vendor\"><faultcode>{spelled}</faultcode><faultstring>x</faultstring></soap:Fault>"
        );
        let read = read(&envelope("soap", "", &fault));
        let held = read.fault().expect("a fault");
        assert_eq!(
            held.code(),
            &FaultCode::Other(spelled.into()),
            "{why}: {spelled}"
        );
        assert_eq!(held.subcode(), None, "{why}: {spelled}");
        assert_eq!(held.code().as_str(), spelled, "{why}");
        assert_eq!(held.code().to_string(), spelled, "{why}");
    }
}

#[test]
fn a_faultcode_prefix_declared_on_the_faultcode_element_resolves() {
    // Namespaces in XML 1.0, section 6.1: the scope of a declaration
    // includes the element it is made on, so a QName in `faultcode`'s content
    // resolves a prefix `faultcode` itself declares.
    let fault = format!(
        "<soap:Fault><faultcode xmlns:e=\"{ENVELOPE_NAMESPACE}\">e:Client</faultcode><faultstring>x</faultstring></soap:Fault>"
    );
    let read = read(&envelope("soap", "", &fault));
    assert_eq!(read.fault().expect("a fault").code(), &FaultCode::Client);
}

#[test]
fn a_faultcode_prefix_carrying_a_dot_resolves() {
    // An NCName may carry a dot, so `soap.v1` is one prefix and the subcode
    // separator is only the dot past the colon.
    let xml = format!(
        "<soap.v1:Envelope xmlns:soap.v1=\"{ENVELOPE_NAMESPACE}\"><soap.v1:Body><soap.v1:Fault>\
         <faultcode>soap.v1:Client.Authentication</faultcode><faultstring>x</faultstring>\
         </soap.v1:Fault></soap.v1:Body></soap.v1:Envelope>"
    );
    let read = read(&xml);
    let held = read.fault().expect("a fault");
    assert_eq!(held.code(), &FaultCode::Client);
    assert_eq!(held.subcode(), Some("Authentication"));
}

#[test]
fn the_soap_1_1_fault_example_reads_its_detail_under_its_own_namespace() {
    // SOAP 1.1, section 6.4, with a `faultactor` added.
    let xml = format!(
        "<SOAP-ENV:Envelope\n  xmlns:SOAP-ENV=\"{ENVELOPE_NAMESPACE}\">\n   <SOAP-ENV:Body>\n       \
         <SOAP-ENV:Fault>\n           <faultcode>SOAP-ENV:Server</faultcode>\n           \
         <faultstring>Server Error</faultstring>\n           \
         <faultactor>http://example.com/stock</faultactor>\n           <detail>\n               \
         <e:myfaultdetails xmlns:e=\"Some-URI\">\n                 \
         <message>My application didn't work</message>\n                 \
         <errorcode>1001</errorcode>\n               </e:myfaultdetails>\n           </detail>\n       \
         </SOAP-ENV:Fault>\n   </SOAP-ENV:Body>\n</SOAP-ENV:Envelope>"
    );
    let read = read(&xml);
    let fault = read.fault().expect("a fault");
    assert_eq!(fault.code(), &FaultCode::Server);
    assert_eq!(fault.subcode(), None);
    assert_eq!(fault.string(), "Server Error");
    assert_eq!(fault.actor(), Some("http://example.com/stock"));
    let [detail] = fault.detail() else {
        panic!("one detail element, got {:?}", fault.detail());
    };
    assert_eq!(detail.name(), "e:myfaultdetails");
    let element = detail.element();
    assert_eq!(element.namespace(), Some("Some-URI"));
    assert_eq!(
        element.child(None, "message").expect("a message").text(),
        Some("My application didn't work")
    );
    assert_eq!(
        element
            .child(None, "errorcode")
            .expect("an error code")
            .text(),
        Some("1001")
    );
    assert_eq!(
        element.scope().resolve("SOAP-ENV"),
        Some(ENVELOPE_NAMESPACE),
        "a detail element remembers the envelope's declarations"
    );
}

#[test]
fn a_faultstring_is_kept_exactly_as_the_document_spells_it() {
    let fault = "<soap:Fault><faultcode>soap:Client</faultcode>\
                 <faultstring>  Échec d’authentification — 認証失敗 &amp; &lt;retry&gt;  </faultstring>\
                 </soap:Fault>";
    let read = read(&envelope("soap", "", fault));
    assert_eq!(
        read.fault().expect("a fault").string(),
        "  Échec d’authentification — 認証失敗 & <retry>  "
    );
}

#[test]
fn header_blocks_are_written_as_given_and_read_in_name_order() {
    // A parsed document is a record sorted by name, the one order it keeps,
    // so a read message's blocks come in name order whatever the wire held.
    let xml = envelope(
        "soap",
        "<t:Transaction xmlns:t=\"urn:t\">5</t:Transaction><a:Auth xmlns:a=\"urn:a\">token</a:Auth>",
        "<m:Get xmlns:m=\"urn:m\"/>",
    );
    let read = read(&xml);
    let names: Vec<&str> = read.header().iter().map(Fragment::name).collect();
    assert_eq!(
        names,
        ["a:Auth", "t:Transaction"],
        "`Envelope::header` answers a read message's blocks in name order"
    );

    let built = Envelope::from_payload(Fragment::new("m:Get", Scalar::Null))
        .with_header(Fragment::new("t:Transaction", text("5")))
        .with_header(Fragment::new("a:Auth", text("token")));
    let written = built.into_bytes().expect("a message");
    let written = utf8(&written);
    assert!(
        at(written, "<t:Transaction>") < at(written, "<a:Auth>"),
        "the blocks are written in the order they were added: {written}"
    );
}

#[test]
fn detail_elements_are_written_as_given_and_read_in_name_order() {
    let fault = "<soap:Fault><faultcode>soap:Server</faultcode><faultstring>x</faultstring>\
                 <detail><z:Cause xmlns:z=\"urn:z\">disk</z:Cause><a:Hint xmlns:a=\"urn:a\">retry</a:Hint></detail>\
                 </soap:Fault>";
    let read = read(&envelope("soap", "", fault));
    let names: Vec<&str> = read
        .fault()
        .expect("a fault")
        .detail()
        .iter()
        .map(Fragment::name)
        .collect();
    assert_eq!(
        names,
        ["a:Hint", "z:Cause"],
        "`Fault::detail` answers a read fault's elements in name order"
    );

    let built = Envelope::from_fault(
        Fault::server("x")
            .with_detail(Fragment::new("z:Cause", text("disk")))
            .with_detail(Fragment::new("a:Hint", text("retry"))),
    );
    let written = built.into_bytes().expect("a message");
    let written = utf8(&written);
    assert!(
        at(written, "<z:Cause>") < at(written, "<a:Hint>"),
        "the detail is written in the order it was added: {written}"
    );
}

#[test]
fn a_fragment_holds_its_name_and_value() {
    let value = record([("@xmlns:m", text("urn:m")), ("symbol", text("DIS"))]);
    let fragment = Fragment::new("m:GetPrice", value.clone());
    assert_eq!(fragment.name(), "m:GetPrice");
    assert_eq!(fragment.value(), &value);
    assert_eq!(fragment, Fragment::new("m:GetPrice", value.clone()));
    assert_ne!(fragment, Fragment::new("m:GetQuote", value.clone()));

    let element = fragment.element();
    assert_eq!(element.name(), "m:GetPrice");
    assert_eq!(element.local_name(), "GetPrice");
    assert_eq!(
        element.namespace(),
        Some("urn:m"),
        "the value's own declaration is in scope"
    );
    assert_eq!(
        element.child(None, "symbol").expect("a symbol").text(),
        Some("DIS")
    );
    assert_eq!(fragment.clone().into_value(), value);

    let bare = Fragment::new("Answer", text("42"));
    assert_eq!(
        bare.element().namespace(),
        None,
        "a fragment built with no declaration is in no namespace"
    );
    assert_eq!(bare.element().text(), Some("42"));

    let unicode = Fragment::new("réponse", text("valeur — 値"));
    assert_eq!(unicode.name(), "réponse");
    assert_eq!(unicode.element().text(), Some("valeur — 値"));
}

#[test]
fn a_fragment_in_a_namespace_declares_it_as_the_default() {
    let fragment = discover();
    assert_eq!(fragment.name(), "Discover");
    assert_eq!(
        fragment.value(),
        &record([
            ("@xmlns", text(XMLA)),
            ("RequestType", text("DISCOVER_DATASOURCES")),
        ])
    );
    let element = fragment.element();
    assert_eq!(element.namespace(), Some(XMLA));
    assert!(element.is(Some(XMLA), "Discover"));
    assert_eq!(
        element
            .child(Some(XMLA), "RequestType")
            .expect("the child inherits the default")
            .text(),
        Some("DISCOVER_DATASOURCES")
    );

    let answer = Fragment::in_namespace("Answer", "urn:example", text("42")).expect("text");
    assert_eq!(
        answer.value(),
        &record([("@xmlns", text("urn:example")), ("#text", text("42"))])
    );
    assert_eq!(answer.element().namespace(), Some("urn:example"));
    assert_eq!(answer.element().text(), Some("42"));

    let empty = Fragment::in_namespace("Empty", "urn:example", Scalar::Null).expect("null");
    assert_eq!(empty.value(), &record([("@xmlns", text("urn:example"))]));
    assert_eq!(empty.element().namespace(), Some("urn:example"));

    let prefixed = Fragment::in_namespace(
        "Row",
        "urn:example",
        record([("@xmlns:x", text("urn:x")), ("x:cell", text("1"))]),
    )
    .expect("a prefix beside the default");
    assert_eq!(prefixed.element().namespace(), Some("urn:example"));
    assert_eq!(
        prefixed
            .element()
            .child(Some("urn:x"), "cell")
            .expect("the prefixed cell")
            .text(),
        Some("1")
    );

    let written =
        yggdryl::xml::into_utf8(&record([(answer.name(), answer.value().clone())])).expect("XML");
    assert_eq!(written, "<Answer xmlns=\"urn:example\">42</Answer>");
}

#[test]
fn a_fragment_refuses_a_second_default_namespace_or_a_non_text_leaf() {
    let declared = record([("@xmlns", text("urn:first")), ("a", text("1"))]);
    match Fragment::in_namespace("Row", "urn:second", declared) {
        Err(Error::Codec { format, reason, .. }) => {
            assert_eq!(format, "soap");
            assert_eq!(reason, "the element already declares a default namespace");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }

    for (value, kind) in [
        (Scalar::from(42), "i32"),
        (Scalar::from_sequence([text("a"), text("b")]), "serie"),
    ] {
        match Fragment::in_namespace("Row", "urn:example", value) {
            Err(Error::Codec { format, reason, .. }) => {
                assert_eq!(format, "soap");
                assert_eq!(
                    reason,
                    format!("expected a record or text for a namespaced element, got {kind}")
                );
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
    }
}

#[test]
fn fault_codes_spell_their_local_name_and_display_under_the_envelope_prefix() {
    for (code, local) in [
        (FaultCode::VersionMismatch, "VersionMismatch"),
        (FaultCode::MustUnderstand, "MustUnderstand"),
        (FaultCode::Client, "Client"),
        (FaultCode::Server, "Server"),
    ] {
        assert_eq!(code.as_str(), local);
        assert_eq!(code.to_string(), format!("SOAP-ENV:{local}"));
        assert_eq!(code.to_string(), format!("{PREFIX}:{local}"));
        assert_eq!(code.clone(), code);
    }
    let other = FaultCode::Other("x:Throttled".into());
    assert_eq!(other.as_str(), "x:Throttled");
    assert_eq!(other.to_string(), "x:Throttled", "no prefix is added");
    assert_eq!(other, FaultCode::Other("x:Throttled".into()));
    assert_ne!(other, FaultCode::Other("x:Other".into()));
    assert_ne!(FaultCode::Client, FaultCode::Server);
    assert_ne!(FaultCode::Client, FaultCode::Other("Client".into()));
}

#[test]
fn a_fault_builds_with_code_subcode_actor_and_detail() {
    let fault = Fault::new(FaultCode::VersionMismatch, "speak SOAP 1.1");
    assert_eq!(fault.code(), &FaultCode::VersionMismatch);
    assert_eq!(fault.subcode(), None);
    assert_eq!(fault.string(), "speak SOAP 1.1");
    assert_eq!(fault.actor(), None);
    assert!(fault.detail().is_empty());

    assert_eq!(Fault::client("x").code(), &FaultCode::Client);
    assert_eq!(Fault::server("x").code(), &FaultCode::Server);
    assert_eq!(Fault::client("x"), Fault::new(FaultCode::Client, "x"));
    assert_eq!(Fault::server("x"), Fault::new(FaultCode::Server, "x"));

    let fault = full_fault();
    assert_eq!(fault.code(), &FaultCode::Client);
    assert_eq!(fault.subcode(), Some("Authentication"));
    assert_eq!(fault.string(), "bad password");
    assert_eq!(fault.actor(), Some("http://example.com/xmla"));
    let [detail] = fault.detail() else {
        panic!("one detail element, got {:?}", fault.detail());
    };
    assert_eq!(detail.name(), "e:Reason");
    assert_eq!(detail.element().namespace(), Some("urn:example:error"));

    let twice = Fault::server("x")
        .with_detail(Fragment::new("a", text("1")))
        .with_detail(Fragment::new("b", text("2")));
    let names: Vec<&str> = twice.detail().iter().map(Fragment::name).collect();
    assert_eq!(names, ["a", "b"], "each detail is appended in turn");

    let replaced = Fault::server("x")
        .with_subcode("A")
        .with_subcode("B")
        .with_actor("one")
        .with_actor("two");
    assert_eq!(
        replaced.subcode(),
        Some("B"),
        "a later subcode replaces the first"
    );
    assert_eq!(
        replaced.actor(),
        Some("two"),
        "a later actor replaces the first"
    );
}

#[test]
fn a_fault_displays_its_code_subcode_and_string_and_is_an_error() {
    assert_eq!(
        Fault::server("disk full").to_string(),
        "SOAP fault SOAP-ENV:Server: disk full"
    );
    assert_eq!(
        full_fault().to_string(),
        "SOAP fault SOAP-ENV:Client.Authentication: bad password",
        "the actor and the detail are not part of the text"
    );
    assert_eq!(
        Fault::new(FaultCode::Other("x:Throttled".into()), "slow down").to_string(),
        "SOAP fault x:Throttled: slow down"
    );
    assert_eq!(
        Fault::client("").to_string(),
        "SOAP fault SOAP-ENV:Client: ",
        "an empty string is displayed as it is"
    );

    let error: Box<dyn std::error::Error + Send + Sync> = Box::new(Fault::client("bad"));
    assert_eq!(error.to_string(), "SOAP fault SOAP-ENV:Client: bad");
    assert!(error.source().is_none());
    let fault = error.downcast::<Fault>().expect("the fault itself");
    assert_eq!(fault.string(), "bad");
}

#[test]
fn an_envelope_answers_its_header_body_payload_and_fault() {
    let envelope = Envelope::new(Body::Payload(discover()));
    assert!(envelope.header().is_empty());
    assert_eq!(envelope.body(), &Body::Payload(discover()));
    assert_eq!(envelope.payload(), Some(&discover()));
    assert_eq!(envelope.fault(), None);
    assert_eq!(Envelope::from_payload(discover()), envelope);

    let faulted = Envelope::from_fault(full_fault());
    assert_eq!(faulted, Envelope::new(Body::Fault(full_fault())));
    assert_eq!(faulted.body(), &Body::Fault(full_fault()));
    assert_eq!(faulted.fault(), Some(&full_fault()));
    assert_eq!(faulted.payload(), None);

    let headed = envelope
        .clone()
        .with_header(transaction())
        .with_header(Fragment::new("a:Auth", text("token")));
    let names: Vec<&str> = headed.header().iter().map(Fragment::name).collect();
    assert_eq!(names, ["t:Transaction", "a:Auth"]);
    assert_eq!(headed.payload(), Some(&discover()));
    assert_ne!(headed, envelope, "the header is part of the message");
}

#[test]
fn into_payload_hands_the_payload_over_and_turns_a_fault_into_the_error() {
    assert_eq!(
        Envelope::from_payload(discover())
            .with_header(transaction())
            .into_payload()
            .expect("a payload"),
        discover()
    );

    match Envelope::from_fault(full_fault()).into_payload() {
        Err(Error::Codec { format, reason, .. }) => {
            assert_eq!(format, "soap");
            assert_eq!(
                reason,
                "SOAP fault SOAP-ENV:Client.Authentication: bad password"
            );
        }
        other => panic!("expected the fault as the error, got {other:?}"),
    }
    let error = Envelope::from_fault(Fault::server("down"))
        .into_payload()
        .expect_err("a fault");
    assert_eq!(
        error.to_string(),
        "invalid soap data at byte 0: SOAP fault SOAP-ENV:Server: down"
    );
}

#[test]
fn into_bytes_opens_with_the_declaration_and_writes_under_the_envelope_prefix() {
    let message = Envelope::from_payload(Fragment::new(
        "m:GetLastTradePrice",
        record([("@xmlns:m", text("Some-URI")), ("symbol", text("DIS"))]),
    ));
    let bytes = message.into_bytes().expect("a message");
    assert_eq!(
        utf8(&bytes),
        format!(
            "{DECLARATION}<SOAP-ENV:Envelope xmlns:SOAP-ENV=\"{ENVELOPE_NAMESPACE}\"><SOAP-ENV:Body>\
             <m:GetLastTradePrice xmlns:m=\"Some-URI\"><symbol>DIS</symbol></m:GetLastTradePrice>\
             </SOAP-ENV:Body></SOAP-ENV:Envelope>"
        )
    );

    let natural = message.into_natural().expect("a document");
    assert_eq!(
        natural,
        record([(
            "SOAP-ENV:Envelope",
            record([
                ("@xmlns:SOAP-ENV", text(ENVELOPE_NAMESPACE)),
                (
                    "SOAP-ENV:Body",
                    record([(
                        "m:GetLastTradePrice",
                        record([("@xmlns:m", text("Some-URI")), ("symbol", text("DIS"))]),
                    )]),
                ),
            ]),
        )]),
        "no header block writes no `Header`"
    );
}

#[test]
fn the_header_is_written_before_the_body() {
    // SOAP 1.1, section 4.2: "If present, the Header element MUST be the
    // first immediate child element of a SOAP Envelope."
    let message = Envelope::from_payload(discover()).with_header(transaction());
    let bytes = message.into_bytes().expect("a message");
    let written = utf8(&bytes);
    assert!(
        at(written, "<SOAP-ENV:Header>") < at(written, "<SOAP-ENV:Body>"),
        "the Header precedes the Body: {written}"
    );
}

#[test]
fn a_fault_writes_code_string_actor_and_detail_in_that_order() {
    // SOAP 1.1, section 4.4 and its envelope schema: `Fault` is the sequence
    // faultcode, faultstring, faultactor, detail.
    let expected = "<SOAP-ENV:Fault><faultcode>SOAP-ENV:Client.Authentication</faultcode>\
                    <faultstring>bad password</faultstring>\
                    <faultactor>http://example.com/xmla</faultactor>\
                    <detail><e:Reason xmlns:e=\"urn:example:error\">expired</e:Reason></detail>\
                    </SOAP-ENV:Fault>";

    let whole = Envelope::from_fault(full_fault())
        .into_bytes()
        .expect("a message");
    assert!(
        utf8(&whole).contains(expected),
        "the whole writer: {}",
        utf8(&whole)
    );

    let streamed = EnvelopeWriter::begin(Vec::new(), &[])
        .expect("an open envelope")
        .fault(&full_fault())
        .expect("a closed envelope");
    assert!(
        utf8(&streamed).contains(expected),
        "the streaming writer: {}",
        utf8(&streamed)
    );
}

#[test]
fn a_fault_without_actor_or_detail_writes_neither() {
    let bytes = Envelope::from_fault(Fault::server("down"))
        .into_bytes()
        .expect("a message");
    let written = utf8(&bytes);
    assert!(
        written.contains("<faultcode>SOAP-ENV:Server</faultcode>"),
        "{written}"
    );
    assert!(
        written.contains("<faultstring>down</faultstring>"),
        "{written}"
    );
    assert!(!written.contains("faultactor"), "{written}");
    assert!(!written.contains("detail"), "{written}");
}

#[test]
fn into_writer_writes_the_bytes_into_bytes_answers() {
    for message in [
        Envelope::from_payload(discover()).with_header(transaction()),
        Envelope::from_fault(full_fault()),
    ] {
        let mut output = Vec::new();
        message.into_writer(&mut output).expect("written");
        assert_eq!(output, message.into_bytes().expect("a message"));
        assert!(output.starts_with(DECLARATION.as_bytes()));
    }

    let closed = Gate {
        open: false,
        bytes: Vec::new(),
    };
    let error = Envelope::from_payload(discover())
        .into_writer(closed)
        .expect_err("the sink refuses");
    assert!(matches!(error, Error::Io(_)), "{error}");
    assert!(error.to_string().contains("the sink is closed"), "{error}");
}

#[test]
fn what_xml_cannot_spell_is_refused_on_write() {
    let bad_name = Envelope::from_payload(Fragment::new("1st", Scalar::Null));
    let error = bad_name.into_bytes().expect_err("not an XML name");
    assert!(
        error.to_string().contains("expected an XML name"),
        "{error}"
    );
    let error = bad_name
        .into_writer(Vec::new())
        .expect_err("not an XML name");
    assert!(
        error.to_string().contains("expected an XML name"),
        "{error}"
    );

    let bad_key = Envelope::from_payload(Fragment::new("m:A", record([("#comment", text("x"))])));
    let error = bad_key.into_bytes().expect_err("no `#comment`");
    assert!(
        error.to_string().contains("`#text` is the only `#` key"),
        "{error}"
    );
}

#[test]
fn a_repeated_header_or_detail_name_is_refused_on_write() {
    let xml = envelope(
        "soap",
        "<t:Hop xmlns:t=\"urn:t\">1</t:Hop><t:Hop xmlns:t=\"urn:t\">2</t:Hop>",
        "<m:Get xmlns:m=\"urn:m\"/>",
    );
    let read = read(&xml);
    let names: Vec<&str> = read.header().iter().map(Fragment::name).collect();
    assert_eq!(names, ["t:Hop", "t:Hop"], "a repeated block is two blocks");
    let texts: Vec<Option<String>> = read
        .header()
        .iter()
        .map(|block| block.element().text().map(str::to_owned))
        .collect();
    assert_eq!(
        texts,
        [Some("1".to_owned()), Some("2".to_owned())],
        "a repeated name keeps document order"
    );

    match read.into_natural() {
        Err(Error::Codec { format, reason, .. }) => {
            assert_eq!(format, "value");
            assert_eq!(reason, "record contains a duplicate field name");
        }
        other => panic!("expected the repeated name refused, got {other:?}"),
    }
    assert!(read.into_bytes().is_err());

    let detail = Envelope::from_fault(
        Fault::server("x")
            .with_detail(Fragment::new("d", text("1")))
            .with_detail(Fragment::new("d", text("2"))),
    );
    let error = detail.into_natural().expect_err("a repeated detail name");
    assert!(error.to_string().contains("duplicate"), "{error}");
}

#[test]
fn an_envelope_round_trips_through_its_bytes() {
    let message = Envelope::from_payload(discover()).with_header(transaction());
    let bytes = message.into_bytes().expect("a message");
    let read = Envelope::from_bytes(&bytes).expect("read back");

    let [block] = read.header() else {
        panic!("one header block, got {:?}", read.header());
    };
    assert_eq!(block.name(), transaction().name());
    assert_eq!(block.value(), transaction().value());
    assert_eq!(
        block
            .element()
            .attribute_in(Some(ENVELOPE_NAMESPACE), "mustUnderstand"),
        Some("1"),
        "the block's `SOAP-ENV` prefix resolves against the written envelope"
    );
    let payload = read.payload().expect("a payload");
    assert_eq!(payload.name(), discover().name());
    assert_eq!(payload.value(), discover().value());
    assert_eq!(payload.element().namespace(), Some(XMLA));

    assert_eq!(
        read.into_natural().expect("a document"),
        message.into_natural().expect("a document")
    );
    assert_eq!(
        read.into_bytes().expect("a message"),
        bytes,
        "a read message writes the same bytes"
    );
    assert_eq!(
        Envelope::from_bytes(&read.into_bytes().expect("a message")).expect("read again"),
        read,
        "a read message is a fixed point"
    );

    for fault in [
        Fault::server("down"),
        Fault::new(FaultCode::VersionMismatch, "Échec — 失敗 & <retry>").with_actor("urn:actor"),
        Fault::client("bad password").with_subcode("Authentication"),
        Fault::new(FaultCode::MustUnderstand, "t:Transaction"),
    ] {
        let bytes = Envelope::from_fault(fault.clone())
            .into_bytes()
            .expect("a message");
        let read = Envelope::from_bytes(&bytes).expect("read back");
        assert_eq!(read.fault(), Some(&fault), "{}", utf8(&bytes));
        assert_eq!(read, Envelope::from_fault(fault));
    }

    let bytes = Envelope::from_fault(full_fault())
        .into_bytes()
        .expect("a message");
    let read = Envelope::from_bytes(&bytes).expect("read back");
    let fault = read.fault().expect("a fault");
    assert_eq!(fault.code(), full_fault().code());
    assert_eq!(fault.subcode(), full_fault().subcode());
    assert_eq!(fault.string(), full_fault().string());
    assert_eq!(fault.actor(), full_fault().actor());
    let [detail] = fault.detail() else {
        panic!("one detail element, got {:?}", fault.detail());
    };
    assert_eq!(detail.name(), "e:Reason");
    assert_eq!(detail.value(), full_fault().detail()[0].value());
    assert_eq!(detail.element().namespace(), Some("urn:example:error"));
}

#[test]
fn an_envelope_round_trips_through_its_natural_value() {
    let message = Envelope::from_payload(discover()).with_header(transaction());
    let natural = message.into_natural().expect("a document");
    let read = Envelope::from_natural(&natural).expect("read back");
    assert_eq!(read.into_natural().expect("a document"), natural);
    assert_eq!(
        read.payload().expect("a payload").value(),
        discover().value()
    );
    assert_eq!(read.header()[0].value(), transaction().value());

    let natural = Envelope::from_fault(full_fault())
        .into_natural()
        .expect("a document");
    assert_eq!(
        natural,
        record([(
            "SOAP-ENV:Envelope",
            record([
                ("@xmlns:SOAP-ENV", text(ENVELOPE_NAMESPACE)),
                (
                    "SOAP-ENV:Body",
                    record([(
                        "SOAP-ENV:Fault",
                        record([
                            ("faultcode", text("SOAP-ENV:Client.Authentication")),
                            ("faultstring", text("bad password")),
                            ("faultactor", text("http://example.com/xmla")),
                            (
                                "detail",
                                record([(
                                    "e:Reason",
                                    record([
                                        ("@xmlns:e", text("urn:example:error")),
                                        ("#text", text("expired")),
                                    ]),
                                )]),
                            ),
                        ]),
                    )]),
                ),
            ]),
        )])
    );
    let read = Envelope::from_natural(&natural).expect("read back");
    assert_eq!(read.fault().expect("a fault").string(), "bad password");
    assert_eq!(read.into_natural().expect("a document"), natural);

    // A hand-built natural value under a prefix of its own reads the same.
    let literal = record([(
        "env:Envelope",
        record([
            ("@xmlns:env", text(ENVELOPE_NAMESPACE)),
            ("env:Header", record([("h:Session", text("abc"))])),
            (
                "env:Body",
                record([(
                    "Execute",
                    record([("@xmlns", text(XMLA)), ("Command", Scalar::Null)]),
                )]),
            ),
        ]),
    )]);
    let read = Envelope::from_natural(&literal).expect("a literal envelope");
    assert_eq!(read.header()[0].name(), "h:Session");
    assert_eq!(read.header()[0].element().text(), Some("abc"));
    let payload = read.payload().expect("a payload");
    assert!(payload.element().is(Some(XMLA), "Execute"));
    assert!(
        payload
            .element()
            .child(Some(XMLA), "Command")
            .expect("the command")
            .is_nil()
    );
}

#[test]
fn the_envelope_writer_streams_a_body_written_through_it() {
    let mut output = Vec::new();
    let mut writer = EnvelopeWriter::begin(&mut output, &[]).expect("an open envelope");
    writer
        .body()
        .write_all(format!("<ExecuteResponse xmlns=\"{XMLA}\"><return>").as_bytes())
        .expect("the opening");
    for row in ["1", "2", "3"] {
        write!(writer.body(), "<row>{row}</row>").expect("a row");
    }
    writer
        .body()
        .write_all(b"</return></ExecuteResponse>")
        .expect("the closing");
    let sink = writer.finish().expect("a closed envelope");
    sink.push(b'\n');

    assert_eq!(
        utf8(&output),
        format!(
            "{DECLARATION}<SOAP-ENV:Envelope xmlns:SOAP-ENV=\"{ENVELOPE_NAMESPACE}\"><SOAP-ENV:Body>\
             <ExecuteResponse xmlns=\"{XMLA}\"><return><row>1</row><row>2</row><row>3</row></return>\
             </ExecuteResponse></SOAP-ENV:Body></SOAP-ENV:Envelope>\n"
        ),
        "finish hands back the very sink it was given"
    );

    let read = Envelope::from_bytes(&output).expect("read back");
    assert!(read.header().is_empty());
    let payload = read.payload().expect("a payload");
    let response = payload.element();
    assert!(response.is(Some(XMLA), "ExecuteResponse"));
    let rows: Vec<Option<String>> = response
        .child(Some(XMLA), "return")
        .expect("the return")
        .children_in(Some(XMLA), "row")
        .iter()
        .map(|row| row.text().map(str::to_owned))
        .collect();
    assert_eq!(
        rows,
        [
            Some("1".to_owned()),
            Some("2".to_owned()),
            Some("3".to_owned())
        ]
    );
}

#[test]
fn the_envelope_writer_writes_header_blocks_before_the_body_in_the_order_given() {
    let blocks = [
        Fragment::new(
            "a:Auth",
            record([("@xmlns:a", text("urn:a")), ("#text", text("token"))]),
        ),
        transaction(),
    ];
    let mut writer = EnvelopeWriter::begin(Vec::new(), &blocks).expect("an open envelope");
    writer
        .body()
        .write_all(b"<m:Get xmlns:m=\"urn:m\"/>")
        .expect("the payload");
    let output = writer.finish().expect("a closed envelope");

    assert_eq!(
        utf8(&output),
        format!(
            "{DECLARATION}<SOAP-ENV:Envelope xmlns:SOAP-ENV=\"{ENVELOPE_NAMESPACE}\">\
             <SOAP-ENV:Header><a:Auth xmlns:a=\"urn:a\">token</a:Auth>\
             <t:Transaction SOAP-ENV:mustUnderstand=\"1\" xmlns:t=\"urn:example:transaction\">5</t:Transaction>\
             </SOAP-ENV:Header><SOAP-ENV:Body><m:Get xmlns:m=\"urn:m\"/></SOAP-ENV:Body></SOAP-ENV:Envelope>"
        )
    );

    let read = Envelope::from_bytes(&output).expect("read back");
    let names: Vec<&str> = read.header().iter().map(Fragment::name).collect();
    assert_eq!(names, ["a:Auth", "t:Transaction"]);
    assert_eq!(
        read.header()[1]
            .element()
            .attribute_in(Some(ENVELOPE_NAMESPACE), "mustUnderstand"),
        Some("1")
    );
    assert_eq!(read.payload().expect("a payload").name(), "m:Get");
}

#[test]
fn a_fault_after_begin_lands_in_the_body() {
    let header = [transaction()];
    let fault = Fault::new(
        FaultCode::MustUnderstand,
        "t:Transaction was not understood",
    )
    .with_actor("http://example.com/xmla");
    let output = EnvelopeWriter::begin(Vec::new(), &header)
        .expect("an open envelope")
        .fault(&fault)
        .expect("a closed envelope");
    let written = utf8(&output);
    assert!(written.starts_with(DECLARATION), "{written}");
    assert!(
        written.ends_with("</SOAP-ENV:Fault></SOAP-ENV:Body></SOAP-ENV:Envelope>"),
        "{written}"
    );

    let read = Envelope::from_bytes(&output).expect("read back");
    assert_eq!(read.fault(), Some(&fault));
    assert_eq!(read.header()[0].name(), "t:Transaction");

    let streamed = EnvelopeWriter::begin(Vec::new(), &[])
        .expect("an open envelope")
        .fault(&full_fault())
        .expect("a closed envelope");
    let whole = Envelope::from_fault(full_fault())
        .into_bytes()
        .expect("a message");
    assert_eq!(
        Envelope::from_bytes(&streamed)
            .expect("read back")
            .into_natural()
            .expect("a document"),
        Envelope::from_bytes(&whole)
            .expect("read back")
            .into_natural()
            .expect("a document"),
        "the streamed fault reads as the whole writer's"
    );
}

#[test]
fn finish_hands_back_the_sink_and_an_empty_body_is_refused_on_read() {
    let output = EnvelopeWriter::begin(Vec::new(), &[])
        .expect("an open envelope")
        .finish()
        .expect("a closed envelope");
    assert_eq!(
        utf8(&output),
        format!(
            "{DECLARATION}<SOAP-ENV:Envelope xmlns:SOAP-ENV=\"{ENVELOPE_NAMESPACE}\"><SOAP-ENV:Body></SOAP-ENV:Body></SOAP-ENV:Envelope>"
        )
    );
    let (format, reason) = refusal(Envelope::from_bytes(&output));
    assert_eq!(format, "soap");
    assert_eq!(reason, "the SOAP body holds no element");
}

#[test]
fn the_envelope_writer_refuses_a_header_block_xml_cannot_spell_and_a_failing_sink() {
    let error = EnvelopeWriter::begin(Vec::new(), &[Fragment::new("1st", text("x"))])
        .err()
        .expect("not an XML name");
    assert!(
        error.to_string().contains("expected an XML name"),
        "{error}"
    );

    let closed = Gate {
        open: false,
        bytes: Vec::new(),
    };
    let error = EnvelopeWriter::begin(closed, &[])
        .err()
        .expect("the sink refuses the declaration");
    assert!(matches!(error, Error::Io(_)), "{error}");
    assert!(error.to_string().contains("the sink is closed"), "{error}");

    let open = Gate {
        open: true,
        bytes: Vec::new(),
    };
    let mut writer = EnvelopeWriter::begin(open, &[]).expect("an open envelope");
    assert!(
        utf8(&writer.body().bytes).ends_with("<SOAP-ENV:Body>"),
        "the body is open when `begin` returns"
    );
    writer.body().open = false;
    let error = writer.finish().err().expect("the sink refuses the close");
    assert!(error.to_string().contains("the sink is closed"), "{error}");

    let mut writer = EnvelopeWriter::begin(
        Gate {
            open: true,
            bytes: Vec::new(),
        },
        &[],
    )
    .expect("an open envelope");
    writer.body().open = false;
    let error = writer
        .fault(&Fault::server("x"))
        .err()
        .expect("the sink refuses the fault");
    assert!(error.to_string().contains("the sink is closed"), "{error}");

    let bad_detail = Fault::server("x").with_detail(Fragment::new("1st", text("x")));
    let error = EnvelopeWriter::begin(Vec::new(), &[])
        .expect("an open envelope")
        .fault(&bad_detail)
        .expect_err("not an XML name");
    assert!(
        error.to_string().contains("expected an XML name"),
        "{error}"
    );
}

#[test]
fn zz_probe() {
    let e = ENVELOPE_NAMESPACE;
    let cases: Vec<String> = vec![
        // 0 read payload declared on envelope, re-write
        format!("<s:Envelope xmlns:s=\"{e}\" xmlns:m=\"urn:m\"><s:Body><m:A><m:b>1</m:b></m:A></s:Body></s:Envelope>"),
        // 1 unqualified Header
        format!("<soap:Envelope xmlns:soap=\"{e}\"><Header><h:A xmlns:h=\"urn:h\">1</h:A></Header><soap:Body><m:A xmlns:m=\"urn:m\">x</m:A></soap:Body></soap:Envelope>"),
        // 2 whitespace-only faultcode
        envelope("soap", "", "<soap:Fault><faultcode>   </faultcode><faultstring>x</faultstring></soap:Fault>"),
        // 3 whitespace-only faultstring
        envelope("soap", "", "<soap:Fault><faultcode>soap:Client</faultcode><faultstring>   </faultstring></soap:Fault>"),
        // 4 empty faultactor both ways
        envelope("soap", "", "<soap:Fault><faultcode>soap:Client</faultcode><faultstring>x</faultstring><faultactor></faultactor></soap:Fault>"),
        envelope("soap", "", "<soap:Fault><faultcode>soap:Client</faultcode><faultstring>x</faultstring><faultactor/></soap:Fault>"),
        // 6 xsi:nil faultcode
        envelope("soap", "", "<soap:Fault xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"><faultcode xsi:nil=\"true\"/><faultstring>x</faultstring></soap:Fault>"),
        // 7 faultcode shadowing soap
        envelope("soap", "", "<soap:Fault><faultcode xmlns:soap=\"urn:other\">soap:Client</faultcode><faultstring>x</faultstring></soap:Fault>"),
        // 8 qualified faultcode children
        envelope("soap", "", "<soap:Fault><soap:faultcode>soap:Server</soap:faultcode><soap:faultstring>q</soap:faultstring><soap:faultactor>a</soap:faultactor><soap:detail><d>1</d></soap:detail></soap:Fault>"),
        // 9 empty subcode
        envelope("soap", "", "<soap:Fault><faultcode>soap:Client.</faultcode><faultstring>x</faultstring></soap:Fault>"),
        // 10 CRLF
        format!("<?xml version=\"1.0\"?>\r\n<soap:Envelope xmlns:soap=\"{e}\">\r\n<soap:Body>\r\n<m:A xmlns:m=\"urn:m\">x\r\ny</m:A>\r\n</soap:Body>\r\n</soap:Envelope>\r\n"),
        // 11 detail children repeated names
        envelope("soap", "", "<soap:Fault><faultcode>soap:Client</faultcode><faultstring>x</faultstring><detail><d>1</d><d>2</d></detail></soap:Fault>"),
        // 12 detail with declaration on detail
        envelope("soap", "", "<soap:Fault><faultcode>soap:Client</faultcode><faultstring>x</faultstring><detail xmlns:e=\"urn:e\"><e:A>1</e:A></detail></soap:Fault>"),
        // 13 Header with text only and Header self closed
        envelope("soap", "just text", "<m:A xmlns:m=\"urn:m\"/>"),
        format!("<soap:Envelope xmlns:soap=\"{e}\"><soap:Header/><soap:Body><m:A xmlns:m=\"urn:m\"/></soap:Body></soap:Envelope>"),
        // 15 BOM
        format!("\u{feff}<soap:Envelope xmlns:soap=\"{e}\"><soap:Body><m:A xmlns:m=\"urn:m\"/></soap:Body></soap:Envelope>"),
        // 16 xsi:nil payload
        format!("<soap:Envelope xmlns:soap=\"{e}\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"><soap:Body><m:A xmlns:m=\"urn:m\" xsi:nil=\"true\"/></soap:Body></soap:Envelope>"),
        // 17 comment and PI in body
        envelope("soap", "", "<!-- c --><?pi x?><m:A xmlns:m=\"urn:m\">1</m:A>"),
        // 18 faultcode padded other
        envelope("soap", "", "<soap:Fault xmlns:x=\"urn:x\"><faultcode>  x:T  </faultcode><faultstring>x</faultstring></soap:Fault>"),
        // 19 escaped faultcode
        envelope("soap", "", "<soap:Fault><faultcode>soap:Client.&#65;&amp;</faultcode><faultstring>x</faultstring></soap:Fault>"),
        // 20 Fault element with attrs in envelope namespace with Envelope default namespace
        format!("<Envelope xmlns=\"{e}\"><Body><Fault><faultcode>Server</faultcode><faultstring>x</faultstring></Fault></Body></Envelope>"),
        // 21 faultcode with child element and no text
        envelope("soap", "", "<soap:Fault><faultcode><x/></faultcode><faultstring>x</faultstring></soap:Fault>"),
        // 22 faultcode with default ns
        envelope("soap", "", "<soap:Fault><faultcode xmlns=\"urn:x\">soap:Client</faultcode><faultstring>x</faultstring></soap:Fault>"),
        // 23 ":Client" empty prefix under default envelope namespace
        format!("<Envelope xmlns=\"{e}\"><Body><Fault><faultcode>:Client</faultcode><faultstring>x</faultstring></Fault></Body></Envelope>"),
        // 24 unqualified Envelope under default envelope namespace + faultcode with prefix undeclared
        envelope("soap", "", "<soap:Fault><faultcode>soap:Client:Extra</faultcode><faultstring>x</faultstring></soap:Fault>"),
    ];
    for (i, xml) in cases.iter().enumerate() {
        let r = Envelope::from_bytes(xml.as_bytes());
        println!("{i}: {r:?}");
        if let Ok(env) = &r {
            let b = env.into_bytes();
            println!("   bytes: {:?}", b.as_ref().map(|b| String::from_utf8_lossy(b).into_owned()));
            if let Ok(b) = b {
                let again = Envelope::from_bytes(&b);
                println!("   again eq: {:?}", again.as_ref().map(|a| a == env));
                if let Ok(a) = &again { if let Some(p) = a.payload() { println!("   ns: {:?}", p.element().namespace()); } }
            }
            println!("   natural: {:?}", env.into_natural().map(|_| ()));
        }
    }
}
