//! `rust/src/xmla/mod.rs`: the six XML for Analysis 1.1 namespaces, every
//! name the module re-exports at `yggdryl::xmla`, and the module's promise
//! end to end - a request built through the API or spelled by hand, answered
//! by the tabular provider, and read back as the rowset, the empty answer,
//! the dataset or the SOAP fault its namespaces say it is.

use std::path::PathBuf;

use yggdryl::holder::{Buffer, Holder};
use yggdryl::media::RecordOptions;
use yggdryl::soap::{ENVELOPE_NAMESPACE, Envelope, Fault, FaultCode};
use yggdryl::xml::Element;
use yggdryl::xmla::definitions::definition_of;
use yggdryl::xmla::service::code;
use yggdryl::xmla::{
    Answer, Catalog, Discover, EMPTY_NAMESPACE, EXCEPTION_NAMESPACE, Execute, MDDATASET_NAMESPACE,
    Method, NAMESPACE, PropertyList, ROWSET_NAMESPACE, Request, RequestType, Response,
    Restrictions, SQL_NAMESPACE, Service, ServiceOptions, Session, XmlaError, property,
};
use yggdryl::{DataType, IOBase, IOMedia, MediaType, MimeType, Scalar, Serie, StructType};

/// The XML Schema namespace a rowset's `xsd:schema` is in.
const XSD: &str = "http://www.w3.org/2001/XMLSchema";

/// A trades table as another writer saves it: a bare rowset `root`, its
/// columns declared under the `xsd` and `sql` prefixes, three rows, the second
/// with no price.
const TRADES: &str = concat!(
    "<root xmlns=\"urn:schemas-microsoft-com:xml-analysis:rowset\" ",
    "xmlns:xsd=\"http://www.w3.org/2001/XMLSchema\" ",
    "xmlns:sql=\"urn:schemas-microsoft-com:xml-sql\">",
    "<xsd:schema targetNamespace=\"urn:schemas-microsoft-com:xml-analysis:rowset\" ",
    "elementFormDefault=\"qualified\">",
    "<xsd:complexType name=\"row\"><xsd:sequence>",
    "<xsd:element sql:field=\"symbol\" name=\"symbol\" type=\"xsd:string\"/>",
    "<xsd:element sql:field=\"price\" name=\"price\" type=\"xsd:double\" minOccurs=\"0\"/>",
    "</xsd:sequence></xsd:complexType></xsd:schema>",
    "<row><symbol>AAPL</symbol><price>187.5</price></row>",
    "<row><symbol>MSFT</symbol></row>",
    "<row><symbol>GOOG</symbol><price>141</price></row>",
    "</root>",
);

/// A fresh folder under the temporary directory, unique to `label` and this
/// process.
fn scratch(label: &str) -> PathBuf {
    let mut root = yggdryl::local::LocalFolder::temporary()
        .expect("a temporary directory")
        .path()
        .expect("a path");
    root.push(format!("yggdryl-xmla-mod-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("the folder is created");
    root
}

/// A catalog folder holding the literal `trades.xmla` table.
fn trades_catalog(label: &str) -> PathBuf {
    let root = scratch(label);
    std::fs::write(root.join("trades.xmla"), TRADES).expect("the table is written");
    root
}

/// The provider with its default options and no catalog at all.
fn empty_service() -> Service {
    Service::new(ServiceOptions::new())
}

/// A SOAP 1.1 message around `body`, spelled by hand.
fn envelope(body: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
         <SOAP-ENV:Envelope xmlns:SOAP-ENV=\"{ENVELOPE_NAMESPACE}\">\
         <SOAP-ENV:Body>{body}</SOAP-ENV:Body></SOAP-ENV:Envelope>"
    )
}

/// The bytes of the message `request` is sent as, answered by `service` as
/// the bytes of its response.
fn answered(service: &Service, request: impl Into<Request>) -> Vec<u8> {
    let message = request.into().into_bytes().expect("the request is written");
    service
        .handle(&message, Vec::new())
        .expect("the answer is written")
}

/// `bytes` read as a response, a refusal panicking with the document.
fn response_of(bytes: &[u8]) -> Response {
    Response::from_bytes(bytes, None)
        .unwrap_or_else(|error| panic!("{error}\n{}", String::from_utf8_lossy(bytes)))
}

/// The fault `bytes` carry, panicking with the document when they carry none.
fn fault_of(bytes: &[u8]) -> Fault {
    Envelope::from_bytes(bytes)
        .expect("an envelope")
        .fault()
        .cloned()
        .unwrap_or_else(|| panic!("expected a fault:\n{}", String::from_utf8_lossy(bytes)))
}

/// Every row of a rowset response as the named record its field reads.
fn named_rows(response: &Response) -> Vec<Scalar> {
    let rows = response.rows().expect("a rowset");
    let field = response.rowset().expect("a rowset").field();
    (0..rows.len())
        .map(|index| {
            let row = rows.get(index).expect("a row").into_owned();
            field.into_natural_value(row).expect("a named row")
        })
        .collect()
}

/// The cell `column` of a named row, null where the row holds none.
fn cell(row: &Scalar, column: &str) -> Scalar {
    row.get_key_str(column).cloned().unwrap_or(Scalar::Null)
}

/// The column names of a rowset response, in column order.
fn column_names(response: &Response) -> Vec<String> {
    response
        .rowset()
        .expect("a rowset")
        .field()
        .fields()
        .iter()
        .map(|field| field.name().to_owned())
        .collect()
}

/// The column names of the rowset `request_type` names, in column order.
fn definition_columns(request_type: &RequestType) -> Vec<String> {
    definition_of(request_type)
        .expect("a rowset this provider answers")
        .field()
        .fields()
        .iter()
        .map(|field| field.name().to_owned())
        .collect()
}

/// The one XMLA error a fault's detail carries.
fn xmla_error(fault: &Fault) -> XmlaError {
    let errors = XmlaError::from_fault(fault);
    assert_eq!(errors.len(), 1, "one Error in the detail of {fault}");
    errors.into_iter().next().expect("one error")
}

// ----------------------------------------------------------------------------
// What the namespaces keep out, and the unqualified reading they let in.
// ----------------------------------------------------------------------------

#[test]
fn a_request_in_another_namespace_is_refused_naming_the_method_namespace() {
    let message = envelope(
        "<Discover xmlns=\"urn:example:not-xmla\">\
         <RequestType>DISCOVER_DATASOURCES</RequestType></Discover>",
    );
    let refusal = Request::from_bytes(message.as_bytes())
        .expect_err("a Discover outside the XMLA namespace is not a request");
    let text = refusal.to_string();
    assert!(text.contains(&format!("{NAMESPACE:?}")), "{text}");
    assert!(text.contains("\"urn:example:not-xmla\""), "{text}");
    assert!(text.contains("`Discover`"), "{text}");
}

#[test]
fn an_unqualified_request_is_read_as_the_method_namespace_s_and_answered() {
    // The intake reading: a client that qualifies nothing means the element
    // the specification names; only another namespace is another element.
    let message = envelope(
        "<Discover><RequestType>DISCOVER_DATASOURCES</RequestType>\
         <Restrictions><RestrictionList/></Restrictions>\
         <Properties><PropertyList/></Properties></Discover>",
    );
    let request = Request::from_bytes(message.as_bytes()).expect("the request is read");
    assert_eq!(
        request.discover().map(Discover::request_type),
        Some(&RequestType::DiscoverDatasources)
    );
    let bytes = empty_service()
        .handle(message.as_bytes(), Vec::new())
        .expect("the answer is written");
    let response = response_of(&bytes);
    assert_eq!(response.method(), Method::Discover);
    assert_eq!(named_rows(&response).len(), 1);
}

#[test]
fn a_namespace_that_differs_only_in_case_is_another_namespace() {
    // Namespace names compare character by character (Namespaces in XML
    // 1.0, section 2.3): the upper-cased URN names no XMLA element.
    let upper = NAMESPACE.to_uppercase();
    let message = envelope(&format!(
        "<Discover xmlns=\"{upper}\"><RequestType>DISCOVER_DATASOURCES</RequestType></Discover>"
    ));
    let refusal = Request::from_bytes(message.as_bytes())
        .expect_err("an upper-cased namespace is not the XMLA namespace");
    let text = refusal.to_string();
    assert!(text.contains(&format!("{upper:?}")), "{text}");
    assert!(text.contains(&format!("{NAMESPACE:?}")), "{text}");
}

#[test]
fn a_request_service_refuses_is_answered_with_a_client_fault_not_an_error() {
    let message = envelope(
        "<Discover xmlns=\"urn:example:not-xmla\">\
         <RequestType>DISCOVER_DATASOURCES</RequestType></Discover>",
    );
    let bytes = empty_service()
        .handle(message.as_bytes(), Vec::new())
        .expect("the fault is written");
    let fault = fault_of(&bytes);
    assert_eq!(fault.code(), &FaultCode::Client);
    assert!(fault.string().contains(NAMESPACE), "{fault}");
    assert_eq!(xmla_error(&fault).code(), code::BAD_REQUEST);
}

#[test]
fn a_response_in_another_namespace_is_refused_naming_the_method_namespace() {
    let message = envelope(&format!(
        "<DiscoverResponse xmlns=\"urn:example:not-xmla\"><return>\
         <root xmlns=\"{ROWSET_NAMESPACE}\"/></return></DiscoverResponse>"
    ));
    let refusal = Response::from_bytes(message.as_bytes(), None)
        .expect_err("a response outside the XMLA namespace is not a response");
    let text = refusal.to_string();
    assert!(text.contains(&format!("{NAMESPACE:?}")), "{text}");
    assert!(text.contains("`DiscoverResponse`"), "{text}");
}

#[test]
fn a_root_in_a_namespace_near_the_rowset_one_is_refused_naming_it() {
    let near = format!("{ROWSET_NAMESPACE}s");
    let message = envelope(&format!(
        "<ExecuteResponse xmlns=\"{NAMESPACE}\"><return>\
         <root xmlns=\"{near}\"/></return></ExecuteResponse>"
    ));
    let refusal = Response::from_bytes(message.as_bytes(), None)
        .expect_err("a root in an unknown namespace names no result");
    let text = refusal.to_string();
    assert!(text.contains(&format!("{near:?}")), "{text}");
}

#[test]
fn a_failure_is_a_soap_fault_whose_detail_carries_the_xmla_error() {
    let discover = Discover::new(RequestType::DiscoverDatasources)
        .with_restrictions(Restrictions::new().with("CATALOG_NAME", "market"));
    let bytes = answered(&empty_service(), discover);
    let fault = fault_of(&bytes);
    assert_eq!(fault.code(), &FaultCode::Client);
    assert_eq!(fault.actor(), Some("yggdryl"));
    assert!(fault.string().contains("CATALOG_NAME"), "{fault}");
    let [detail] = fault.detail() else {
        panic!("one detail element in {fault}");
    };
    assert!(
        detail.element().is_in(EXCEPTION_NAMESPACE, "Error"),
        "the detail is the XMLA Error in the exception namespace: {detail:?}"
    );
    let error = xmla_error(&fault);
    assert_eq!(error.code(), code::BAD_RESTRICTION);
    assert_eq!(error.description(), fault.string());
    assert_eq!(error.source(), "yggdryl");
    let refusal = Response::from_bytes(&bytes, None).expect_err("a fault is not an answer");
    let text = refusal.to_string();
    assert!(text.contains("CATALOG_NAME"), "{text}");
    assert!(
        text.contains(&format!("({} ", code::BAD_RESTRICTION)),
        "the refusal carries the XMLA error code: {text}"
    );
}

#[test]
fn every_multidimensional_rowset_is_read_and_refused_by_name() {
    let service = empty_service();
    let mdschema: Vec<&RequestType> = RequestType::ALL
        .iter()
        .filter(|request_type| request_type.is_mdschema())
        .collect();
    assert!(!mdschema.is_empty());
    for request_type in mdschema {
        let message = Request::from(Discover::new(request_type.clone()))
            .into_bytes()
            .expect("the request is written");
        let read = Request::from_bytes(&message).expect("the request is read");
        assert_eq!(
            read.discover().map(Discover::request_type),
            Some(request_type)
        );
        let bytes = service
            .handle(&message, Vec::new())
            .expect("the fault is written");
        let fault = fault_of(&bytes);
        assert_eq!(fault.code(), &FaultCode::Client, "{request_type}");
        assert!(
            fault.string().contains(request_type.as_str()),
            "the fault names {request_type}: {fault}"
        );
        assert_eq!(xmla_error(&fault).code(), code::UNSUPPORTED_REQUEST_TYPE);
    }
}

#[test]
fn a_multidimensional_format_is_refused_by_name_for_either_method() {
    let properties = PropertyList::new().with(property::FORMAT, "Multidimensional");
    let service = empty_service();
    let discover =
        Discover::new(RequestType::DiscoverDatasources).with_properties(properties.clone());
    let execute = Execute::statement("select * from trades").with_properties(properties);
    for request in [Request::from(discover), Request::from(execute)] {
        let bytes = service
            .answer(&request, Vec::new())
            .expect("the fault is written");
        let fault = fault_of(&bytes);
        assert_eq!(fault.code(), &FaultCode::Client, "{:?}", request.kind());
        assert!(fault.string().contains("Format"), "{fault}");
        assert!(fault.string().contains("Tabular"), "{fault}");
        assert_eq!(xmla_error(&fault).code(), code::UNSUPPORTED_FORMAT);
    }
}

#[test]
fn an_mdx_statement_is_refused_and_never_answered() {
    let root = trades_catalog("mdx");
    let service = empty_service().with_catalog(Catalog::new(
        "market",
        Holder::folder(&root).expect("the catalog holds"),
    ));
    let execute = Execute::statement(
        "SELECT {[Measures].[Sales Amount]} ON COLUMNS, {[Date].[Calendar Year].MEMBERS} ON ROWS \
         FROM [Adventure Works]",
    );
    let bytes = answered(&service, execute);
    let fault = fault_of(&bytes);
    assert_eq!(fault.code(), &FaultCode::Client);
    assert_eq!(xmla_error(&fault).code(), code::BAD_STATEMENT);
    assert!(
        Response::from_bytes(&bytes, None).is_err(),
        "an MDX statement earns no rowset and no dataset"
    );
}

#[test]
fn an_mdx_statement_is_refused_naming_mdx() {
    // The module doc: "the MDX a multidimensional provider answers [is] read
    // and refused by name, never answered". A client that sent MDX learns
    // from the fault that this provider speaks none, not where the
    // expression grammar stopped reading it.
    let root = trades_catalog("mdx-by-name");
    let service = empty_service().with_catalog(Catalog::new(
        "market",
        Holder::folder(&root).expect("the catalog holds"),
    ));
    let execute =
        Execute::statement("SELECT {[Measures].[Sales Amount]} ON COLUMNS FROM [Adventure Works]");
    let fault = fault_of(&answered(&service, execute));
    assert!(
        fault.string().contains("MDX"),
        "the refusal names MDX: {fault}"
    );
}

// ----------------------------------------------------------------------------
// The six namespaces.
// ----------------------------------------------------------------------------

#[test]
fn the_six_namespaces_spell_the_xmla_1_1_urns() {
    assert_eq!(NAMESPACE, "urn:schemas-microsoft-com:xml-analysis");
    assert_eq!(
        ROWSET_NAMESPACE,
        "urn:schemas-microsoft-com:xml-analysis:rowset"
    );
    assert_eq!(
        MDDATASET_NAMESPACE,
        "urn:schemas-microsoft-com:xml-analysis:mddataset"
    );
    assert_eq!(
        EMPTY_NAMESPACE,
        "urn:schemas-microsoft-com:xml-analysis:empty"
    );
    assert_eq!(
        EXCEPTION_NAMESPACE,
        "urn:schemas-microsoft-com:xml-analysis:exception"
    );
    assert_eq!(SQL_NAMESPACE, "urn:schemas-microsoft-com:xml-sql");
}

#[test]
fn every_result_namespace_extends_the_method_namespace_by_one_segment() {
    for (namespace, segment) in [
        (ROWSET_NAMESPACE, "rowset"),
        (MDDATASET_NAMESPACE, "mddataset"),
        (EMPTY_NAMESPACE, "empty"),
        (EXCEPTION_NAMESPACE, "exception"),
    ] {
        assert_eq!(
            namespace.strip_prefix(NAMESPACE),
            Some(format!(":{segment}").as_str())
        );
    }
}

#[test]
fn the_sql_namespace_lies_outside_the_method_namespace() {
    assert!(!SQL_NAMESPACE.starts_with(NAMESPACE));
    assert!(SQL_NAMESPACE.starts_with("urn:schemas-microsoft-com:"));
}

#[test]
fn the_six_namespaces_are_distinct_and_none_is_the_soap_envelope() {
    let all = [
        NAMESPACE,
        ROWSET_NAMESPACE,
        MDDATASET_NAMESPACE,
        EMPTY_NAMESPACE,
        EXCEPTION_NAMESPACE,
        SQL_NAMESPACE,
    ];
    for (index, namespace) in all.iter().enumerate() {
        assert!(!all[..index].contains(namespace), "{namespace} repeats");
        assert_ne!(*namespace, ENVELOPE_NAMESPACE);
        assert_eq!(
            namespace.trim(),
            *namespace,
            "no whitespace around {namespace}"
        );
    }
}

#[test]
fn both_requests_are_written_in_the_method_namespace() {
    for (request, local) in [
        (
            Request::from(Discover::new(RequestType::DiscoverDatasources)),
            "Discover",
        ),
        (Request::from(Execute::statement("select 1")), "Execute"),
    ] {
        let bytes = request.into_bytes().expect("the request is written");
        let envelope = Envelope::from_bytes(&bytes).expect("an envelope");
        let payload = envelope.payload().expect("a payload");
        let element = payload.element();
        assert!(element.is_in(NAMESPACE, local), "{}", element.name());
        assert_eq!(element.namespace(), Some(NAMESPACE));
    }
}

#[test]
fn an_xmla_error_is_written_in_the_exception_namespace() {
    let error = XmlaError::new(code::UNKNOWN_TABLE, "no table `trades`");
    let fragment = error.into_fragment().expect("the Error element is built");
    let element = fragment.element();
    assert!(element.is_in(EXCEPTION_NAMESPACE, "Error"));
    let fault = yggdryl::xmla::fault(FaultCode::Server, error.clone()).expect("the fault is built");
    let bytes = yggdryl::xmla::write_fault(Vec::new(), &fault).expect("the fault is written");
    let read = fault_of(&bytes);
    let [detail] = read.detail() else {
        panic!("one detail element in {read}");
    };
    assert_eq!(detail.element().namespace(), Some(EXCEPTION_NAMESPACE));
    assert_eq!(XmlaError::from_fault(&read), vec![error]);
}

#[test]
fn a_rowset_declares_its_columns_under_the_sql_namespace() {
    let field = StructType::from_fields([
        DataType::Int32.required_field("Order Id"),
        DataType::utf8().nullable_field("Symbol"),
    ])
    .map(DataType::from)
    .expect("a valid root")
    .required_field("row");
    let rowset = yggdryl::xmla::Rowset::new(field).expect("a rowset");
    let mut document = Vec::new();
    rowset
        .write_root(&mut document, std::iter::empty(), true, true)
        .expect("the root is written");
    let value = yggdryl::from_xml_scalar(&document).expect("the root parses");
    let root = Element::root(&value).expect("a root element");
    assert!(root.is_in(ROWSET_NAMESPACE, "root"));
    assert_eq!(root.scope().resolve("EX"), Some(EXCEPTION_NAMESPACE));
    let schema = root.child(Some(XSD), "schema").expect("the schema");
    assert_eq!(
        schema.attribute("targetNamespace").and_then(Scalar::as_str),
        Some(ROWSET_NAMESPACE)
    );
    let row_type = schema
        .children_in(Some(XSD), "complexType")
        .into_iter()
        .find(|held| held.attribute("name").and_then(Scalar::as_str) == Some("row"))
        .expect("the row type");
    let sequence = row_type.child(Some(XSD), "sequence").expect("a sequence");
    let declared: Vec<(String, String)> = sequence
        .children_in(Some(XSD), "element")
        .iter()
        .map(|declaration| {
            (
                declaration
                    .attribute_in(Some(SQL_NAMESPACE), "field")
                    .expect("a sql:field")
                    .to_owned(),
                declaration
                    .attribute("name")
                    .and_then(Scalar::as_str)
                    .expect("a name")
                    .to_owned(),
            )
        })
        .collect();
    assert_eq!(
        declared,
        [
            ("Order Id".to_owned(), "Order_x0020_Id".to_owned()),
            ("Symbol".to_owned(), "Symbol".to_owned()),
        ]
    );
}

#[test]
fn a_root_in_the_empty_namespace_answers_nothing() {
    let written = yggdryl::xmla::write_empty(Vec::new(), &[], Method::Execute)
        .expect("the empty answer is written");
    let text = String::from_utf8(written.clone()).expect("UTF-8");
    // The empty root declares the instance, schema and exception namespaces
    // beside its own, as the reference providers write it.
    assert!(
        text.contains(&format!("<root xmlns=\"{EMPTY_NAMESPACE}\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xmlns:xsd=\"http://www.w3.org/2001/XMLSchema\" xmlns:EX=\"urn:schemas-microsoft-com:xml-analysis:exception\"/>")),
        "{text}"
    );
    let literal = envelope(&format!(
        "<ExecuteResponse xmlns=\"{NAMESPACE}\"><return><root xmlns=\"{EMPTY_NAMESPACE}\"/>\
         </return></ExecuteResponse>"
    ));
    for bytes in [written, literal.into_bytes()] {
        let response = response_of(&bytes);
        assert_eq!(response.method(), Method::Execute);
        assert_eq!(response.answer(), &Answer::Empty);
        assert!(response.rows().is_none());
        assert!(response.rowset().is_none());
    }
}

#[test]
fn a_root_in_the_mddataset_namespace_is_kept_as_the_dataset_it_is() {
    let message = envelope(&format!(
        "<ExecuteResponse xmlns=\"{NAMESPACE}\"><return>\
         <root xmlns=\"{MDDATASET_NAMESPACE}\"><OlapInfo/><Axes/><CellData/></root>\
         </return></ExecuteResponse>"
    ));
    let response = response_of(message.as_bytes());
    assert_eq!(response.method(), Method::Execute);
    let Answer::Dataset(dataset) = response.answer() else {
        panic!("a dataset, got {:?}", response.answer());
    };
    let element = dataset.element();
    assert!(element.is_in(MDDATASET_NAMESPACE, "root"));
    let children: Vec<&str> = element.children().map(|child| child.local_name()).collect();
    let mut sorted = children.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, ["Axes", "CellData", "OlapInfo"]);
    assert!(response.rows().is_none());
}

#[test]
fn a_response_is_read_by_its_namespaces_whatever_prefixes_spell_them() {
    let message = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
         <soap:Envelope xmlns:soap=\"{ENVELOPE_NAMESPACE}\">\n  <soap:Body>\n    \
         <xa:DiscoverResponse xmlns:xa=\"{NAMESPACE}\">\n      <xa:return>\n        \
         <rs:root xmlns:rs=\"{ROWSET_NAMESPACE}\" xmlns:s=\"{XSD}\" xmlns:q=\"{SQL_NAMESPACE}\">\n          \
         <s:schema targetNamespace=\"{ROWSET_NAMESPACE}\" elementFormDefault=\"qualified\">\n            \
         <s:complexType name=\"row\">\n              <s:sequence>\n                \
         <s:element q:field=\"Key Word\" name=\"Key_x0020_Word\" type=\"s:string\"/>\n              \
         </s:sequence>\n            </s:complexType>\n          </s:schema>\n          \
         <rs:row><rs:Key_x0020_Word>select</rs:Key_x0020_Word></rs:row>\n          \
         <rs:row><rs:Key_x0020_Word>from</rs:Key_x0020_Word></rs:row>\n        \
         </rs:root>\n      </xa:return>\n    </xa:DiscoverResponse>\n  </soap:Body>\n\
         </soap:Envelope>\n"
    );
    let response = response_of(message.as_bytes());
    assert_eq!(response.method(), Method::Discover);
    assert_eq!(column_names(&response), ["Key Word"]);
    let words: Vec<Scalar> = named_rows(&response)
        .iter()
        .map(|row| cell(row, "Key Word"))
        .collect();
    assert_eq!(words, [Scalar::from("select"), Scalar::from("from")]);
}

// ----------------------------------------------------------------------------
// Every re-exported name resolves at `yggdryl::xmla`, as the item its own
// module declares.
// ----------------------------------------------------------------------------

#[test]
fn the_vocabulary_names_resolve_at_the_module_root() {
    use yggdryl::xmla::vocabulary;
    let method: vocabulary::Method = yggdryl::xmla::Method::Discover;
    assert_eq!(method.as_str(), "Discover");
    let format: vocabulary::Format = yggdryl::xmla::Format::Tabular;
    assert_eq!(format.as_str(), "Tabular");
    let content: vocabulary::Content = yggdryl::xmla::Content::SchemaData;
    assert_eq!(content.as_str(), "SchemaData");
    let request_type: vocabulary::RequestType = yggdryl::xmla::RequestType::DiscoverDatasources;
    assert_eq!(request_type.as_str(), "DISCOVER_DATASOURCES");
    let properties: vocabulary::PropertyList =
        yggdryl::xmla::PropertyList::new().with(yggdryl::xmla::property::CATALOG, "market");
    assert_eq!(properties.catalog(), Some("market"));
    assert_eq!(
        yggdryl::xmla::property::CATALOG,
        vocabulary::property::CATALOG
    );
    let restrictions: vocabulary::Restrictions =
        yggdryl::xmla::Restrictions::new().with("TABLE_NAME", "trades");
    assert_eq!(
        restrictions.get("table_name"),
        Some(["trades".to_owned()].as_slice())
    );
    let access: vocabulary::Access = yggdryl::xmla::Access::ReadWrite;
    assert_eq!(access.as_str(), "ReadWrite");
    let mode: vocabulary::AuthenticationMode = yggdryl::xmla::AuthenticationMode::Unauthenticated;
    assert_eq!(mode.as_str(), "Unauthenticated");
    let axis: vocabulary::AxisFormat = yggdryl::xmla::AxisFormat::TupleFormat;
    assert_eq!(axis.as_str(), "TupleFormat");
    let mdx: vocabulary::MdxSupport = yggdryl::xmla::MdxSupport::Core;
    assert_eq!(mdx.as_str(), "Core");
    let provider: vocabulary::ProviderType = yggdryl::xmla::ProviderType::Tdp;
    assert_eq!(provider.as_str(), "TDP");
    let state: vocabulary::StateSupport = yggdryl::xmla::StateSupport::Sessions;
    assert_eq!(state.as_str(), "Sessions");
}

#[test]
fn the_request_names_resolve_at_the_module_root() {
    use yggdryl::xmla::request;
    let discover: request::Discover = yggdryl::xmla::Discover::new(RequestType::DbschemaTables);
    let command: request::Command = yggdryl::xmla::Command::Statement("select 1".to_owned());
    let execute: request::Execute = yggdryl::xmla::Execute::new(command);
    assert_eq!(execute.command().statement(), Some("select 1"));
    let method: request::RequestMethod = yggdryl::xmla::RequestMethod::Discover(discover.clone());
    let request: request::Request = yggdryl::xmla::Request::new(method);
    assert_eq!(request.discover(), Some(&discover));
    assert_eq!(request.kind(), Method::Discover);
    let session: request::Session = yggdryl::xmla::Session::Continue("abc".into());
    assert_eq!(session.session_id(), Some("abc"));
    let block = session.into_fragment().expect("the header block is built");
    assert!(block.element().is_in(NAMESPACE, "Session"));
}

#[test]
fn the_response_names_resolve_at_the_module_root() {
    use yggdryl::xmla::response;
    let error: response::XmlaError = yggdryl::xmla::XmlaError::new(7, "no table");
    let fault: Fault =
        yggdryl::xmla::fault(FaultCode::Client, error.clone()).expect("the fault is built");
    assert_eq!(
        fault,
        response::fault(FaultCode::Client, error.clone()).expect("built")
    );
    let faulted = yggdryl::xmla::write_fault(Vec::new(), &fault).expect("the fault is written");
    assert_eq!(
        faulted,
        response::write_fault(Vec::new(), &fault).expect("the fault is written")
    );
    assert_eq!(XmlaError::from_fault(&fault_of(&faulted)), vec![error]);

    let empty = yggdryl::xmla::write_empty(Vec::new(), &[], Method::Discover)
        .expect("the empty answer is written");
    let read: response::Response =
        yggdryl::xmla::Response::from_bytes(&empty, None).expect("a response");
    assert!(matches!(read.answer(), response::Answer::Empty));

    let field = StructType::from_fields([DataType::utf8().required_field("Keyword")])
        .map(DataType::from)
        .expect("a valid root")
        .required_field("row");
    let rowset = yggdryl::xmla::Rowset::new(field.clone()).expect("a rowset");
    let rows = Serie::from_scalars(
        field,
        [Scalar::from_struct([("Keyword", Scalar::from("select"))]).expect("a row")],
    )
    .expect("the rows");
    let written = yggdryl::xmla::write_rowset(
        Vec::new(),
        &[],
        Method::Discover,
        &rowset,
        std::iter::once(Ok(rows.clone())),
        yggdryl::xmla::Content::SchemaData,
    )
    .expect("the rowset is written");
    let read = response_of(&written);
    assert_eq!(read.rows(), Some(&rows));
    assert_eq!(
        read.rowset().map(yggdryl::xmla::Rowset::field),
        Some(rowset.field())
    );
}

#[test]
fn the_rowset_names_resolve_at_the_module_root() {
    use yggdryl::xmla::rowset;
    let field = StructType::from_fields([DataType::Int32.required_field("Order Id")])
        .map(DataType::from)
        .expect("a valid root")
        .required_field("row");
    let built: rowset::Rowset = yggdryl::xmla::Rowset::new(field).expect("a rowset");
    assert_eq!(built.element_names(), ["Order_x0020_Id"]);
    let xsd: rowset::XsdType = yggdryl::xmla::XsdType::Int;
    assert_eq!(yggdryl::xmla::XsdType::of(&DataType::Int32), Some(xsd));
    assert_eq!(xsd.as_str(), "xsd:int");
    assert_eq!(yggdryl::xmla::encode_name("Order Id"), "Order_x0020_Id");
    assert_eq!(
        yggdryl::xmla::encode_name("Order Id"),
        rowset::encode_name("Order Id")
    );
    assert_eq!(yggdryl::xmla::decode_name("Order_x0020_Id"), "Order Id");
    assert_eq!(
        yggdryl::xmla::decode_name("Order_x0020_Id"),
        rowset::decode_name("Order_x0020_Id")
    );
}

#[test]
fn the_medium_names_resolve_at_the_module_root() {
    use yggdryl::xmla::{media, options};
    let settings: options::XmlaOptions = yggdryl::xmla::XmlaOptions::new();
    let field = StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("symbol"),
    ])
    .map(DataType::from)
    .expect("a valid root")
    .required_field("row");
    let rows = Serie::from_scalars(
        field.clone(),
        [
            Scalar::from_struct([
                ("id", Scalar::from(1_i64)),
                ("symbol", Scalar::from("AAPL")),
            ])
            .expect("a row"),
            Scalar::from_struct([("id", Scalar::from(2_i64)), ("symbol", Scalar::Null)])
                .expect("a row"),
        ],
    )
    .expect("the rows");
    let mut held = Buffer::new().with_media_type(MediaType::from_file_name("orders.xmla"));
    yggdryl::xmla::overwrite_arrow_reader(
        &mut held,
        rows.into_arrow_reader().expect("a reader"),
        &settings,
    )
    .expect("the document is written");
    assert_eq!(
        yggdryl::xmla::read_field(&held, &settings).expect("the schema"),
        field
    );
    let batches: usize = yggdryl::xmla::read_batch_reader(&held, None, &settings)
        .expect("a reader")
        .map(|batch| batch.expect("a batch").num_rows())
        .sum();
    assert_eq!(batches, 2);
    let wrapped: media::Xmla<Buffer> = yggdryl::xmla::Xmla::new(held);
    assert_eq!(wrapped.options(), &settings);
}

#[test]
fn the_catalog_names_resolve_at_the_module_root() {
    use yggdryl::xmla::{catalog, dbtype};
    let root = trades_catalog("catalog-names");
    let market: catalog::Catalog =
        yggdryl::xmla::Catalog::new("market", Holder::folder(&root).expect("the catalog holds"));
    let tables: Vec<catalog::Table> = market.tables().expect("the catalog lists");
    let names: Vec<(&str, Option<&str>, &str)> = tables
        .iter()
        .map(|table: &yggdryl::xmla::Table| (table.catalog(), table.schema(), table.name()))
        .collect();
    assert_eq!(names, [("market", None, "trades")]);
    let indicator: dbtype::DbType = yggdryl::xmla::DbType::of(&DataType::Int32);
    assert_eq!(indicator, yggdryl::xmla::DbType::I4);
    assert_eq!(indicator.code(), 3);
}

#[test]
fn the_provider_names_resolve_at_the_module_root() {
    use yggdryl::xmla::{server, service};
    let options: service::ServiceOptions = yggdryl::xmla::ServiceOptions::new();
    let provider: service::Service = yggdryl::xmla::Service::new(options);
    let outcome: Result<service::Execution, Fault> =
        provider.execute(&Execute::statement("select * from trades"));
    let Err(fault) = outcome else {
        panic!("no catalog holds `trades`");
    };
    assert_eq!(xmla_error(&fault).code(), code::UNKNOWN_TABLE);
    let bound: server::Server = yggdryl::xmla::Server::bind(provider, "127.0.0.1:0")
        .expect("a loopback port")
        .with_options(yggdryl::xmla::ServerOptions::new().with_path("/xmla"));
    assert!(bound.endpoint().ends_with("/xmla"), "{}", bound.endpoint());
    let running: server::Running = bound.spawn();
    assert!(running.local_addr().is_some());
    assert!(running.service().catalogs().is_empty());
    running.stop();
}

// ----------------------------------------------------------------------------
// The module's promise, end to end.
// ----------------------------------------------------------------------------

#[test]
fn a_discover_built_through_the_api_over_no_catalog_reads_back_as_the_datasources_rowset() {
    let service = empty_service();
    assert!(service.catalogs().is_empty());
    let request = Request::from(Discover::new(RequestType::DiscoverDatasources));
    let bytes = answered(&service, request);
    let response = response_of(&bytes);
    assert_eq!(response.method(), Method::Discover);
    assert!(response.header().is_empty());
    assert_eq!(
        column_names(&response),
        definition_columns(&RequestType::DiscoverDatasources)
    );
    let rows = named_rows(&response);
    assert_eq!(rows.len(), 1, "one data source");
    let row = &rows[0];
    assert_eq!(cell(row, "DataSourceName"), Scalar::from("yggdryl"));
    assert_eq!(
        cell(row, "DataSourceDescription"),
        Scalar::from("Catalogs of record media, served over XML for Analysis")
    );
    assert!(cell(row, "URL").is_null(), "no URL was given");
    assert_eq!(
        cell(row, "DataSourceInfo"),
        Scalar::from("Provider=yggdryl;Data Source=yggdryl")
    );
    assert_eq!(cell(row, "ProviderName"), Scalar::from("yggdryl"));
    assert_eq!(
        cell(row, "ProviderType"),
        Scalar::from_sequence([Scalar::from("TDP")])
    );
    assert_eq!(
        cell(row, "AuthenticationMode"),
        Scalar::from("Unauthenticated")
    );
}

#[test]
fn the_datasources_answer_is_the_rowset_the_provider_discovers() {
    let service = empty_service();
    let discover = Discover::new(RequestType::DiscoverDatasources);
    let (definition, discovered) = service
        .discover(&discover)
        .unwrap_or_else(|fault| panic!("{fault}"));
    let bytes = answered(&service, discover);
    let response = Response::from_bytes(&bytes, Some(definition.field()))
        .unwrap_or_else(|error| panic!("{error}\n{}", String::from_utf8_lossy(&bytes)));
    assert_eq!(response.rows(), Some(&discovered));
    assert_eq!(
        response.rowset().map(|rowset| rowset.field()),
        Some(definition.field())
    );
}

#[test]
fn a_discover_answer_names_each_part_by_its_xmla_namespace() {
    let bytes = answered(
        &empty_service(),
        Discover::new(RequestType::DiscoverDatasources),
    );
    let envelope = Envelope::from_bytes(&bytes).expect("an envelope");
    let payload = envelope.payload().expect("a payload, not a fault");
    let response = payload.element();
    assert!(response.is_in(NAMESPACE, "DiscoverResponse"));
    let returned = response
        .one_child_in(NAMESPACE, "return")
        .expect("one return");
    let roots: Vec<Element<'_>> = returned
        .children()
        .filter(|child| child.local_name() == "root")
        .collect();
    let [root] = roots.as_slice() else {
        panic!("one root, got {}", roots.len());
    };
    assert_eq!(root.namespace(), Some(ROWSET_NAMESPACE));
    let schema = root.child(Some(XSD), "schema").expect("the schema");
    let row_type = schema
        .children_in(Some(XSD), "complexType")
        .into_iter()
        .find(|held| held.attribute("name").and_then(Scalar::as_str) == Some("row"))
        .expect("the row type");
    let sequence = row_type.child(Some(XSD), "sequence").expect("a sequence");
    let declared: Vec<String> = sequence
        .children_in(Some(XSD), "element")
        .iter()
        .map(|declaration| {
            declaration
                .attribute_in(Some(SQL_NAMESPACE), "field")
                .expect("a sql:field")
                .to_owned()
        })
        .collect();
    assert_eq!(
        declared,
        definition_columns(&RequestType::DiscoverDatasources)
    );
    assert_eq!(root.children_in(Some(ROWSET_NAMESPACE), "row").len(), 1);
}

#[test]
fn a_rowset_declares_its_columns_once_before_its_rows() {
    let bytes = answered(
        &empty_service(),
        Discover::new(RequestType::DiscoverDatasources),
    );
    let text = String::from_utf8(bytes).expect("UTF-8");
    assert_eq!(text.matches("<xsd:schema").count(), 1, "{text}");
    let schema = text.find("<xsd:schema").expect("a schema");
    let row = text.find("<row>").expect("a row");
    assert!(schema < row, "{text}");
}

#[test]
fn a_discover_spelled_by_hand_under_another_prefix_is_answered() {
    let message = envelope(&format!(
        "<x:Discover xmlns:x=\"{NAMESPACE}\">\
         <x:RequestType>  DISCOVER_DATASOURCES  </x:RequestType>\
         <x:Restrictions><x:RestrictionList>\
         <x:ProviderType>TDP</x:ProviderType>\
         </x:RestrictionList></x:Restrictions>\
         <x:Properties><x:PropertyList><x:Format>Tabular</x:Format></x:PropertyList></x:Properties>\
         </x:Discover>"
    ));
    let request = Request::from_bytes(message.as_bytes()).expect("the request is read");
    let discover = request.discover().expect("a Discover");
    assert_eq!(discover.request_type(), &RequestType::DiscoverDatasources);
    assert_eq!(
        discover.restrictions().get("ProviderType"),
        Some(["TDP".to_owned()].as_slice())
    );
    assert_eq!(discover.properties().get(property::FORMAT), Some("Tabular"));
    let bytes = empty_service()
        .handle(message.as_bytes(), Vec::new())
        .expect("the answer is written");
    let response = response_of(&bytes);
    let rows = named_rows(&response);
    assert_eq!(rows.len(), 1);
    assert_eq!(cell(&rows[0], "DataSourceName"), Scalar::from("yggdryl"));
}

#[test]
fn a_catalog_rowset_over_no_catalog_is_empty_and_keeps_its_columns() {
    let bytes = answered(
        &empty_service(),
        Discover::new(RequestType::DbschemaCatalogs),
    );
    let response = response_of(&bytes);
    assert_eq!(response.rows().map(Serie::len), Some(0));
    assert_eq!(
        column_names(&response),
        definition_columns(&RequestType::DbschemaCatalogs)
    );
}

#[test]
fn a_data_source_named_in_unicode_and_markup_reads_back_as_named() {
    let mut options = ServiceOptions::new();
    options.data_source_name = "Marché 東京 ✓".to_owned();
    options.data_source_description = "a <table> & \"quoted\" 'text'".to_owned();
    let service = Service::new(options);
    let bytes = answered(&service, Discover::new(RequestType::DiscoverDatasources));
    let rows = named_rows(&response_of(&bytes));
    let [row] = rows.as_slice() else {
        panic!("one data source, got {}", rows.len());
    };
    assert_eq!(cell(row, "DataSourceName"), Scalar::from("Marché 東京 ✓"));
    assert_eq!(
        cell(row, "DataSourceDescription"),
        Scalar::from("a <table> & \"quoted\" 'text'")
    );
    assert_eq!(
        cell(row, "DataSourceInfo"),
        Scalar::from("Provider=yggdryl;Data Source=Marché 東京 ✓")
    );
}

#[test]
fn the_module_doc_statement_is_answered_as_a_rowset_of_the_catalog_table() {
    let root = trades_catalog("statement");
    let service = empty_service().with_catalog(Catalog::new(
        "market",
        Holder::folder(&root).expect("the catalog holds"),
    ));
    let execute =
        Execute::statement("select symbol, price from trades where price is not null limit 10");
    let bytes = answered(&service, execute);
    let response = response_of(&bytes);
    assert_eq!(response.method(), Method::Execute);
    assert_eq!(column_names(&response), ["symbol", "price"]);
    let rows: Vec<(Scalar, Scalar)> = named_rows(&response)
        .iter()
        .map(|row| (cell(row, "symbol"), cell(row, "price")))
        .collect();
    assert_eq!(
        rows,
        [
            (Scalar::from("AAPL"), Scalar::from(187.5_f64)),
            (Scalar::from("GOOG"), Scalar::from(141.0_f64)),
        ]
    );
}

#[test]
fn the_catalog_table_is_listed_as_the_tabular_provider_s_table() {
    let root = trades_catalog("tables");
    let service = empty_service().with_catalog(Catalog::new(
        "market",
        Holder::folder(&root).expect("the catalog holds"),
    ));
    let bytes = answered(&service, Discover::new(RequestType::DbschemaTables));
    let rows = named_rows(&response_of(&bytes));
    let tables: Vec<(Scalar, Scalar, Scalar)> = rows
        .iter()
        .map(|row| {
            (
                cell(row, "TABLE_CATALOG"),
                cell(row, "TABLE_SCHEMA"),
                cell(row, "TABLE_NAME"),
            )
        })
        .collect();
    assert_eq!(
        tables,
        [(Scalar::from("market"), Scalar::Null, Scalar::from("trades"))]
    );
}

// ----------------------------------------------------------------------------
// Where a namespace refusal is located, and what else the namespaces keep out.
// ----------------------------------------------------------------------------

/// Whether `error` is the XMLA intake's refusal: an invalid record located at
/// `$.xmla`, the path the module's shared refusal names.
fn is_xmla_refusal(error: &yggdryl::Error) -> bool {
    matches!(error, yggdryl::Error::InvalidRecord { path, .. } if path == "$.xmla")
}

/// An `ExecuteResponse` around `root`, spelled by hand.
fn execute_response(root: &str) -> String {
    envelope(&format!(
        "<ExecuteResponse xmlns=\"{NAMESPACE}\"><return>{root}</return></ExecuteResponse>"
    ))
}

#[test]
fn a_namespace_refusal_is_an_invalid_record_located_at_the_xmla_path() {
    let request = envelope(
        "<Discover xmlns=\"urn:example:not-xmla\">\
         <RequestType>DISCOVER_DATASOURCES</RequestType></Discover>",
    );
    let refusal = Request::from_bytes(request.as_bytes()).expect_err("not a request");
    assert!(is_xmla_refusal(&refusal), "{refusal:?}");
    assert!(
        refusal
            .to_string()
            .starts_with("invalid record value at $.xmla: "),
        "{refusal}"
    );
    let response = envelope(&format!(
        "<DiscoverResponse xmlns=\"urn:example:not-xmla\"><return>\
         <root xmlns=\"{ROWSET_NAMESPACE}\"/></return></DiscoverResponse>"
    ));
    let refusal = Response::from_bytes(response.as_bytes(), None).expect_err("not a response");
    assert!(is_xmla_refusal(&refusal), "{refusal:?}");
}

#[test]
fn an_element_that_is_no_method_is_refused_naming_it_and_its_namespace() {
    let quoted = format!("{NAMESPACE:?}");
    for (body, named, namespace) in [
        (
            format!("<Cancel xmlns=\"{NAMESPACE}\"/>"),
            "`Cancel`",
            quoted.as_str(),
        ),
        ("<Cancel/>".to_owned(), "`Cancel`", "no namespace"),
        // XML names are case-sensitive: `discover` is not the method.
        (
            format!(
                "<discover xmlns=\"{NAMESPACE}\">\
                 <RequestType>DISCOVER_DATASOURCES</RequestType></discover>"
            ),
            "`discover`",
            quoted.as_str(),
        ),
        // A response sent where a request belongs.
        (
            format!("<DiscoverResponse xmlns=\"{NAMESPACE}\"><return/></DiscoverResponse>"),
            "`DiscoverResponse`",
            quoted.as_str(),
        ),
    ] {
        let message = envelope(&body);
        let refusal = Request::from_bytes(message.as_bytes()).expect_err("no method");
        assert!(is_xmla_refusal(&refusal), "{refusal:?}");
        let text = refusal.to_string();
        assert!(text.contains("`Discover` or `Execute`"), "{text}");
        assert!(text.contains(named), "{text}");
        assert!(text.contains(&format!(" in {namespace}")), "{text}");
    }
}

#[test]
fn a_method_in_any_namespace_but_the_method_one_is_refused_naming_that_namespace() {
    let near_slash = format!("{NAMESPACE}/");
    let near_colon = format!("{NAMESPACE}:");
    for namespace in [
        ROWSET_NAMESPACE,
        MDDATASET_NAMESPACE,
        EMPTY_NAMESPACE,
        EXCEPTION_NAMESPACE,
        SQL_NAMESPACE,
        near_slash.as_str(),
        near_colon.as_str(),
    ] {
        for body in [
            format!(
                "<Discover xmlns=\"{namespace}\">\
                 <RequestType>DISCOVER_DATASOURCES</RequestType></Discover>"
            ),
            format!(
                "<Execute xmlns=\"{namespace}\">\
                 <Command><Statement>select 1</Statement></Command></Execute>"
            ),
        ] {
            let message = envelope(&body);
            let refusal = Request::from_bytes(message.as_bytes())
                .expect_err("a method outside the method namespace is not a request");
            let text = refusal.to_string();
            assert!(text.contains(&format!("in {namespace:?}")), "{text}");
            assert!(text.contains(&format!("{NAMESPACE:?}")), "{text}");
        }
    }
}

#[test]
fn a_request_argument_in_another_namespace_is_not_the_argument_the_method_needs() {
    let message = envelope(&format!(
        "<Discover xmlns=\"{NAMESPACE}\">\
         <o:RequestType xmlns:o=\"urn:example:other\">DISCOVER_DATASOURCES</o:RequestType>\
         </Discover>"
    ));
    let refusal = Request::from_bytes(message.as_bytes())
        .expect_err("a RequestType in another namespace is not the Discover's");
    let text = refusal.to_string();
    assert!(text.contains("`RequestType`"), "{text}");
    assert!(text.contains("found none"), "{text}");
    let fault = fault_of(
        &empty_service()
            .handle(message.as_bytes(), Vec::new())
            .expect("the fault is written"),
    );
    assert_eq!(fault.code(), &FaultCode::Client);
    assert!(fault.string().contains("`RequestType`"), "{fault}");
    assert_eq!(xmla_error(&fault).code(), code::BAD_REQUEST);
}

#[test]
fn a_qualified_and_an_unqualified_argument_are_one_argument_given_twice() {
    // The unqualified reading makes `RequestType` and `x:RequestType` the
    // same element, so a Discover carrying both carries it twice.
    let message = envelope(&format!(
        "<x:Discover xmlns:x=\"{NAMESPACE}\">\
         <x:RequestType>DISCOVER_DATASOURCES</x:RequestType>\
         <RequestType>DBSCHEMA_TABLES</RequestType></x:Discover>"
    ));
    let refusal =
        Request::from_bytes(message.as_bytes()).expect_err("two request types are not one");
    let text = refusal.to_string();
    assert!(text.contains("`RequestType`"), "{text}");
    assert!(text.contains("found several"), "{text}");
}

#[test]
fn a_statement_in_another_namespace_is_a_command_the_provider_refuses_by_name() {
    let message = envelope(&format!(
        "<Execute xmlns=\"{NAMESPACE}\"><Command>\
         <o:Statement xmlns:o=\"urn:example:other\">select 1</o:Statement>\
         </Command></Execute>"
    ));
    let request = Request::from_bytes(message.as_bytes()).expect("the request is read");
    let execute = request.execute().expect("an Execute");
    assert_eq!(execute.command().statement(), None);
    assert_eq!(execute.command().name(), "o:Statement");
    let fault = fault_of(
        &empty_service()
            .handle(message.as_bytes(), Vec::new())
            .expect("the fault is written"),
    );
    assert_eq!(fault.code(), &FaultCode::Client);
    assert!(fault.string().contains("`o:Statement`"), "{fault}");
    assert_eq!(xmla_error(&fault).code(), code::UNSUPPORTED_COMMAND);
}

#[test]
fn a_session_block_in_another_namespace_is_a_header_the_provider_does_not_understand() {
    let message = format!(
        "<SOAP-ENV:Envelope xmlns:SOAP-ENV=\"{ENVELOPE_NAMESPACE}\"><SOAP-ENV:Header>\
         <o:Session xmlns:o=\"urn:example:other\" SessionId=\"abc\" mustUnderstand=\"1\"/>\
         </SOAP-ENV:Header><SOAP-ENV:Body>\
         <Discover xmlns=\"{NAMESPACE}\"><RequestType>DISCOVER_DATASOURCES</RequestType></Discover>\
         </SOAP-ENV:Body></SOAP-ENV:Envelope>"
    );
    let request = Request::from_bytes(message.as_bytes()).expect("the request is read");
    assert_eq!(
        request.session().expect("no session block of XMLA's"),
        None,
        "a Session in another namespace is not the XMLA session"
    );
    let fault = fault_of(
        &empty_service()
            .handle(message.as_bytes(), Vec::new())
            .expect("the fault is written"),
    );
    assert_eq!(fault.code(), &FaultCode::MustUnderstand);
    assert!(fault.string().contains("`o:Session`"), "{fault}");
    assert!(fault.detail().is_empty(), "a header fault has no detail");
}

#[test]
fn a_session_the_provider_opens_is_named_in_the_method_namespace_and_honoured() {
    let service = empty_service();
    let opening = Request::from(Discover::new(RequestType::DiscoverDatasources))
        .with_session(&Session::Begin)
        .expect("the header is built");
    let response = response_of(&answered(&service, opening));
    let [block] = response.header() else {
        panic!("one header block, got {:?}", response.header());
    };
    assert!(block.element().is(Some(NAMESPACE), "Session"));
    let session = Session::read(response.header())
        .expect("the session is read")
        .expect("a session is answered");
    let id = session.session_id().expect("an identifier").to_owned();
    assert!(!id.is_empty());
    let continuing = Request::from(Discover::new(RequestType::DiscoverDatasources))
        .with_session(&Session::Continue(id.as_str().into()))
        .expect("the header is built");
    let echoed = response_of(&answered(&service, continuing));
    assert_eq!(
        Session::read(echoed.header()).expect("the session is read"),
        Some(Session::Continue(id.as_str().into()))
    );
}

#[test]
fn a_fault_where_a_request_belongs_is_refused_and_answered_with_a_client_fault() {
    let message = format!(
        "<SOAP-ENV:Envelope xmlns:SOAP-ENV=\"{ENVELOPE_NAMESPACE}\"><SOAP-ENV:Body>\
         <SOAP-ENV:Fault><faultcode>SOAP-ENV:Server</faultcode>\
         <faultstring>down</faultstring></SOAP-ENV:Fault>\
         </SOAP-ENV:Body></SOAP-ENV:Envelope>"
    );
    let refusal = Request::from_bytes(message.as_bytes()).expect_err("a fault is no request");
    assert!(is_xmla_refusal(&refusal), "{refusal:?}");
    let text = refusal.to_string();
    assert!(text.contains("expected a Discover or Execute"), "{text}");
    assert!(text.contains("down"), "{text}");
    let fault = fault_of(
        &empty_service()
            .handle(message.as_bytes(), Vec::new())
            .expect("the fault is written"),
    );
    assert_eq!(fault.code(), &FaultCode::Client);
    assert_eq!(xmla_error(&fault).code(), code::BAD_REQUEST);
}

#[test]
fn an_empty_or_blank_message_is_answered_with_a_client_fault() {
    for message in [&b""[..], b"   \n", b"<?xml version=\"1.0\"?>"] {
        assert!(Request::from_bytes(message).is_err());
        assert!(Response::from_bytes(message, None).is_err());
        let fault = fault_of(
            &empty_service()
                .handle(message, Vec::new())
                .expect("the fault is written"),
        );
        assert_eq!(fault.code(), &FaultCode::Client);
        assert_eq!(xmla_error(&fault).code(), code::BAD_REQUEST);
    }
}

#[test]
fn a_root_in_a_namespace_that_names_no_result_is_refused_naming_it() {
    for namespace in [
        NAMESPACE,
        EXCEPTION_NAMESPACE,
        SQL_NAMESPACE,
        ENVELOPE_NAMESPACE,
    ] {
        let message = execute_response(&format!("<root xmlns=\"{namespace}\"/>"));
        let refusal = Response::from_bytes(message.as_bytes(), None)
            .expect_err("a root in no result namespace is no answer");
        assert!(is_xmla_refusal(&refusal), "{refusal:?}");
        let text = refusal.to_string();
        assert!(text.contains(&format!("{namespace:?}")), "{text}");
        assert!(text.contains("names no result"), "{text}");
    }
}

#[test]
fn a_return_in_another_namespace_is_not_the_response_s_return() {
    let message = envelope(&format!(
        "<DiscoverResponse xmlns=\"{NAMESPACE}\">\
         <o:return xmlns:o=\"urn:example:other\"><root xmlns=\"{ROWSET_NAMESPACE}\"/></o:return>\
         </DiscoverResponse>"
    ));
    let refusal = Response::from_bytes(message.as_bytes(), None)
        .expect_err("a return in another namespace is not the response's");
    let text = refusal.to_string();
    assert!(text.contains("`return`"), "{text}");
    assert!(text.contains("found none"), "{text}");
}

#[test]
fn a_response_that_qualifies_nothing_is_read_as_the_rowset_it_spells() {
    let message = envelope(&format!(
        "<DiscoverResponse><return><root xmlns:xsd=\"{XSD}\">\
         <xsd:schema><xsd:complexType name=\"row\"><xsd:sequence>\
         <xsd:element name=\"Key_x0020_Word\" type=\"xsd:string\"/>\
         </xsd:sequence></xsd:complexType></xsd:schema>\
         <row><Key_x0020_Word>select</Key_x0020_Word></row>\
         </root></return></DiscoverResponse>"
    ));
    let response = response_of(message.as_bytes());
    assert_eq!(response.method(), Method::Discover);
    // No `sql:field`: the column is named by its element, the escape decoded.
    assert_eq!(column_names(&response), ["Key Word"]);
    let rows = named_rows(&response);
    assert_eq!(
        rows.iter()
            .map(|row| cell(row, "Key Word"))
            .collect::<Vec<_>>(),
        [Scalar::from("select")]
    );
}

#[test]
fn a_row_in_another_namespace_is_no_row_of_the_rowset() {
    // The `ExecuteResponse` declares the method namespace as the default, so
    // a bare `<row>` under a prefixed root inherits it: that row is in the
    // method namespace, not the rowset's, and only `xmlns=""` makes one
    // unqualified.
    let message = execute_response(&format!(
        "<rs:root xmlns:rs=\"{ROWSET_NAMESPACE}\" xmlns:xsd=\"{XSD}\">\
         <xsd:schema targetNamespace=\"{ROWSET_NAMESPACE}\"><xsd:complexType name=\"row\">\
         <xsd:sequence><xsd:element name=\"Keyword\" type=\"xsd:string\"/></xsd:sequence>\
         </xsd:complexType></xsd:schema>\
         <rs:row><Keyword>select</Keyword></rs:row>\
         <o:row xmlns:o=\"urn:example:other\"><Keyword>from</Keyword></o:row>\
         <row><Keyword>into</Keyword></row>\
         <row xmlns=\"\"><Keyword>where</Keyword></row>\
         </rs:root>"
    ));
    let response = response_of(message.as_bytes());
    let words: Vec<Scalar> = named_rows(&response)
        .iter()
        .map(|row| cell(row, "Keyword"))
        .collect();
    assert_eq!(words, [Scalar::from("select"), Scalar::from("where")]);
}

#[test]
fn an_execute_that_answers_nothing_is_answered_in_the_empty_namespace() {
    let root = trades_catalog("content-none");
    let service = empty_service().with_catalog(Catalog::new(
        "market",
        Holder::folder(&root).expect("the catalog holds"),
    ));
    let execute = Execute::statement("select * from trades")
        .with_properties(PropertyList::new().with(property::CONTENT, "None"));
    let bytes = answered(&service, execute);
    let envelope = Envelope::from_bytes(&bytes).expect("an envelope");
    let payload = envelope.payload().expect("a payload, not a fault");
    let response = payload.element();
    assert!(response.is(Some(NAMESPACE), "ExecuteResponse"));
    let returned = response
        .one_child_in(NAMESPACE, "return")
        .expect("one return");
    let root = returned
        .children()
        .find(|child| child.local_name() == "root")
        .expect("a root");
    assert_eq!(root.namespace(), Some(EMPTY_NAMESPACE));
    assert_eq!(response_of(&bytes).answer(), &Answer::Empty);
}

#[test]
fn a_fault_another_provider_spelled_is_read_as_the_xmla_error_its_detail_carries() {
    // The shape a reference provider answers a failed statement with: the
    // `Error` under a prefix of its own, its text escaped.
    let message = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
         <soap:Envelope xmlns:soap=\"{ENVELOPE_NAMESPACE}\"><soap:Body><soap:Fault>\
         <faultcode>soap:Server</faultcode>\
         <faultstring>Query (1, 8) The syntax for &apos;ON&apos; is incorrect.</faultstring>\
         <detail><x:Error xmlns:x=\"{EXCEPTION_NAMESPACE}\" ErrorCode=\"3238658121\" \
         Description=\"Query (1, 8) The syntax for &apos;ON&apos; is incorrect. &#x2014; &lt;MDX&gt;\" \
         Source=\"Microsoft SQL Server 2019 Analysis Services\" HelpFile=\"\"/></detail>\
         </soap:Fault></soap:Body></soap:Envelope>"
    );
    let fault = fault_of(message.as_bytes());
    assert_eq!(fault.code(), &FaultCode::Server);
    let [detail] = fault.detail() else {
        panic!("one detail element in {fault}");
    };
    assert!(detail.element().is(Some(EXCEPTION_NAMESPACE), "Error"));
    let error = xmla_error(&fault);
    assert_eq!(error.code(), 3_238_658_121);
    assert_eq!(
        error.description(),
        "Query (1, 8) The syntax for 'ON' is incorrect. \u{2014} <MDX>"
    );
    assert_eq!(
        error.source(),
        "Microsoft SQL Server 2019 Analysis Services"
    );
    assert_eq!(error.help_file(), "");
    let refusal =
        Response::from_bytes(message.as_bytes(), None).expect_err("a fault is not an answer");
    let text = refusal.to_string();
    assert!(
        text.contains("(3238658121 Query (1, 8) The syntax for 'ON' is incorrect. \u{2014} <MDX>)"),
        "{text}"
    );
}

// ----------------------------------------------------------------------------
// The module's promise: what a Discover asks the provider about itself and
// about a catalog.
// ----------------------------------------------------------------------------

#[test]
fn the_request_types_the_provider_lists_are_exactly_the_ones_it_answers() {
    let service = empty_service();
    let listed = named_rows(&response_of(&answered(
        &service,
        Discover::new(RequestType::DiscoverSchemaRowsets),
    )));
    let names: Vec<String> = listed
        .iter()
        .map(|row| {
            cell(row, "SchemaName")
                .as_str()
                .expect("a SchemaName")
                .to_owned()
        })
        .collect();
    assert!(
        names.iter().any(|name| name == "DISCOVER_SCHEMA_ROWSETS"),
        "{names:?}"
    );
    assert!(
        names.iter().all(|name| !name.starts_with("MDSCHEMA_")),
        "a tabular provider lists no multidimensional rowset: {names:?}"
    );
    for name in &names {
        assert!(
            RequestType::ALL
                .iter()
                .any(|request_type| request_type.as_str() == name),
            "{name} is a request type XMLA defines"
        );
    }
    let mut asked: Vec<RequestType> = RequestType::ALL.to_vec();
    asked.push(RequestType::Other("VENDOR_ROWSET".into()));
    for request_type in asked {
        let bytes = answered(&service, Discover::new(request_type.clone()));
        if names.iter().any(|name| name == request_type.as_str()) {
            let response = response_of(&bytes);
            assert_eq!(
                column_names(&response),
                definition_columns(&request_type),
                "{request_type}"
            );
        } else {
            let fault = fault_of(&bytes);
            assert_eq!(fault.code(), &FaultCode::Client, "{request_type}");
            assert!(
                fault.string().contains(request_type.as_str()),
                "the fault names {request_type}: {fault}"
            );
            assert_eq!(xmla_error(&fault).code(), code::UNSUPPORTED_REQUEST_TYPE);
        }
    }
}

#[test]
fn the_properties_the_provider_states_carry_the_values_the_request_set() {
    let service = empty_service();
    let discover = Discover::new(RequestType::DiscoverProperties).with_properties(
        PropertyList::new()
            .with(property::CATALOG, "market")
            .with(property::FORMAT, "Tabular"),
    );
    let rows = named_rows(&response_of(&answered(&service, discover)));
    let value_of = |name: &str| {
        rows.iter()
            .find(|row| cell(row, "PropertyName") == Scalar::from(name))
            .map(|row| cell(row, "Value"))
    };
    assert_eq!(value_of(property::CATALOG), Some(Scalar::from("market")));
    assert_eq!(value_of(property::FORMAT), Some(Scalar::from("Tabular")));
    assert_eq!(
        value_of(property::PROVIDER_NAME),
        Some(Scalar::from("yggdryl"))
    );

    let narrowed = Discover::new(RequestType::DiscoverProperties)
        .with_restrictions(Restrictions::new().with("PropertyName", property::CATALOG));
    let rows = named_rows(&response_of(&answered(&service, narrowed)));
    let [row] = rows.as_slice() else {
        panic!("one property, got {}", rows.len());
    };
    assert_eq!(cell(row, "PropertyName"), Scalar::from(property::CATALOG));
    // Unset, the catalog is the empty text: a value, not an absent one.
    assert_eq!(cell(row, "Value"), Scalar::from(""));
}

/// A catalog holding the literal trades table.
fn trades_service(label: &str) -> Service {
    let root = trades_catalog(label);
    empty_service().with_catalog(Catalog::new(
        "market",
        Holder::folder(&root).expect("the catalog holds"),
    ))
}

#[test]
fn the_columns_of_a_literal_table_state_its_declared_types_and_nullability() {
    let service = trades_service("columns");
    let rows = named_rows(&response_of(&answered(
        &service,
        Discover::new(RequestType::DbschemaColumns),
    )));
    let columns: Vec<[Scalar; 5]> = rows
        .iter()
        .map(|row| {
            [
                cell(row, "TABLE_NAME"),
                cell(row, "COLUMN_NAME"),
                cell(row, "ORDINAL_POSITION"),
                cell(row, "IS_NULLABLE"),
                cell(row, "DATA_TYPE"),
            ]
        })
        .collect();
    // `symbol` declares no `minOccurs`, so it is required; `price` declares
    // `minOccurs="0"`, so it may be absent.
    assert_eq!(
        columns,
        [
            [
                Scalar::from("trades"),
                Scalar::from("symbol"),
                Scalar::from(1_u32),
                Scalar::from(false),
                Scalar::from(yggdryl::xmla::DbType::Wstr.code()),
            ],
            [
                Scalar::from("trades"),
                Scalar::from("price"),
                Scalar::from(2_u32),
                Scalar::from(true),
                Scalar::from(yggdryl::xmla::DbType::R8.code()),
            ],
        ]
    );
}

#[test]
fn every_datatype_a_column_states_is_one_the_provider_types_list() {
    let service = trades_service("provider-types");
    let types: Vec<Scalar> = named_rows(&response_of(&answered(
        &service,
        Discover::new(RequestType::DbschemaProviderTypes),
    )))
    .iter()
    .map(|row| cell(row, "DATA_TYPE"))
    .collect();
    assert!(!types.is_empty());
    let stated = named_rows(&response_of(&answered(
        &service,
        Discover::new(RequestType::DbschemaColumns),
    )));
    assert_eq!(stated.len(), 2);
    for column in &stated {
        let data_type = cell(column, "DATA_TYPE");
        assert!(
            types.contains(&data_type),
            "{data_type:?} of {:?} is listed among {types:?}",
            cell(column, "COLUMN_NAME")
        );
    }
}

#[test]
fn content_decides_whether_the_columns_the_rows_or_both_are_written() {
    let service = empty_service();
    let definition =
        definition_of(&RequestType::DiscoverDatasources).expect("a rowset this provider answers");
    let asking = |content: &str| {
        answered(
            &service,
            Discover::new(RequestType::DiscoverDatasources)
                .with_properties(PropertyList::new().with(property::CONTENT, content)),
        )
    };

    let schema = response_of(&asking("Schema"));
    assert_eq!(schema.rows().map(Serie::len), Some(0));
    assert_eq!(
        column_names(&schema),
        definition_columns(&RequestType::DiscoverDatasources)
    );

    let data = asking("Data");
    let text = String::from_utf8(data.clone()).expect("UTF-8");
    assert!(
        !text.contains("<xsd:schema"),
        "no schema is written: {text}"
    );
    let refusal = Response::from_bytes(&data, None)
        .expect_err("rows without a schema read under no declared field");
    assert!(refusal.to_string().contains("no schema"), "{refusal}");
    let (_, discovered) = service
        .discover(&Discover::new(RequestType::DiscoverDatasources))
        .unwrap_or_else(|fault| panic!("{fault}"));
    let read = Response::from_bytes(&data, Some(definition.field()))
        .unwrap_or_else(|error| panic!("{error}\n{text}"));
    assert_eq!(read.rows(), Some(&discovered));
}

#[test]
fn empty_and_blank_text_in_a_discover_answer_reads_back_as_written() {
    let mut options = ServiceOptions::new().with_url("");
    options.data_source_description = "  \t ".to_owned();
    let service = Service::new(options);
    let rows = named_rows(&response_of(&answered(
        &service,
        Discover::new(RequestType::DiscoverDatasources),
    )));
    let [row] = rows.as_slice() else {
        panic!("one data source, got {}", rows.len());
    };
    assert_eq!(
        cell(row, "URL"),
        Scalar::from(""),
        "the empty URL is no null"
    );
    assert_eq!(cell(row, "DataSourceDescription"), Scalar::from("  \t "));
}

// ----------------------------------------------------------------------------
// The module's promise: a table is any leaf a record medium reads, or a
// folder that reads as one.
// ----------------------------------------------------------------------------

/// A rowset document of one required `symbol` column holding `symbols`.
fn symbols_document(symbols: &[&str]) -> String {
    let rows: String = symbols
        .iter()
        .map(|symbol| format!("<row><symbol>{symbol}</symbol></row>"))
        .collect();
    format!(
        "<root xmlns=\"{ROWSET_NAMESPACE}\" xmlns:xsd=\"{XSD}\" xmlns:sql=\"{SQL_NAMESPACE}\">\
         <xsd:schema targetNamespace=\"{ROWSET_NAMESPACE}\" elementFormDefault=\"qualified\">\
         <xsd:complexType name=\"row\"><xsd:sequence>\
         <xsd:element sql:field=\"symbol\" name=\"symbol\" type=\"xsd:string\"/>\
         </xsd:sequence></xsd:complexType></xsd:schema>{rows}</root>"
    )
}

/// The `symbol` cells of the rowset `statement` is answered with, sorted.
fn symbols_selected(service: &Service, statement: &str) -> Vec<Scalar> {
    let response = response_of(&answered(service, Execute::statement(statement)));
    let mut symbols: Vec<Scalar> = named_rows(&response)
        .iter()
        .map(|row| cell(row, "symbol"))
        .collect();
    symbols.sort();
    symbols
}

#[test]
fn a_leaf_of_another_record_medium_and_a_partitioned_tree_are_tables_too() {
    let root = trades_catalog("any-medium");
    let quotes = StructType::from_fields([
        DataType::utf8().required_field("symbol"),
        DataType::Float64.required_field("bid"),
    ])
    .map(DataType::from)
    .expect("a valid root")
    .required_field("row");
    let rows = Serie::from_scalars(
        quotes,
        [Scalar::from_struct([
            ("symbol", Scalar::from("TSLA")),
            ("bid", Scalar::from(250.5_f64)),
        ])
        .expect("a row")],
    )
    .expect("the rows");
    let mut leaf = Holder::folder(&root)
        .expect("the root holds")
        .child_by_path("quotes.arrows")
        .expect("the child resolves");
    leaf.overwrite_arrow_reader(
        rows.into_arrow_reader().expect("a reader"),
        &RecordOptions::for_mime_type(&MimeType::ARROW_STREAM).expect("IPC options"),
    )
    .expect("the IPC table is written");
    for (day, symbols) in [("1", &["AAPL"][..]), ("2", &["MSFT", "NVDA"][..])] {
        let partition = root.join("desk").join("fills").join(format!("day={day}"));
        std::fs::create_dir_all(&partition).expect("the partition is made");
        std::fs::write(partition.join("part.xmla"), symbols_document(symbols))
            .expect("the partition is written");
    }
    let service = empty_service().with_catalog(Catalog::new(
        "market",
        Holder::folder(&root).expect("the catalog holds"),
    ));

    let listed = named_rows(&response_of(&answered(
        &service,
        Discover::new(RequestType::DbschemaTables),
    )));
    let mut tables: Vec<(Scalar, Scalar)> = listed
        .iter()
        .map(|row| (cell(row, "TABLE_SCHEMA"), cell(row, "TABLE_NAME")))
        .collect();
    tables.sort();
    assert_eq!(
        tables,
        [
            (Scalar::Null, Scalar::from("quotes")),
            (Scalar::Null, Scalar::from("trades")),
            (Scalar::from("desk"), Scalar::from("fills")),
        ]
    );

    let schemata: Vec<(Scalar, Scalar)> = named_rows(&response_of(&answered(
        &service,
        Discover::new(RequestType::DbschemaSchemata),
    )))
    .iter()
    .map(|row| (cell(row, "CATALOG_NAME"), cell(row, "SCHEMA_NAME")))
    .collect();
    assert_eq!(
        schemata,
        [(Scalar::from("market"), Scalar::from("desk"))],
        "a folder holding tables is a schema; a table folder is not"
    );

    assert_eq!(
        symbols_selected(&service, "select symbol from quotes"),
        [Scalar::from("TSLA")]
    );
    assert_eq!(
        symbols_selected(&service, "select symbol from market.desk.fills"),
        [
            Scalar::from("AAPL"),
            Scalar::from("MSFT"),
            Scalar::from("NVDA")
        ]
    );
}

#[test]
fn the_rows_a_document_holds_are_counted_through_the_record_surface() {
    let media_type = || MediaType::from_file_name("trades.xmla");
    let held = Buffer::from_bytes(TRADES.as_bytes().to_vec()).with_media_type(media_type());
    assert_eq!(IOMedia::row_size(&held).expect("the rows are counted"), 3);
    let none = Buffer::from_bytes(symbols_document(&[]).into_bytes()).with_media_type(media_type());
    assert_eq!(IOMedia::row_size(&none).expect("the rows are counted"), 0);
    let one =
        Buffer::from_bytes(symbols_document(&["AAPL"]).into_bytes()).with_media_type(media_type());
    assert_eq!(IOMedia::row_size(&one).expect("the rows are counted"), 1);
    // A missing document reads as the empty stream.
    let empty = Buffer::new().with_media_type(media_type());
    assert_eq!(IOMedia::row_size(&empty).expect("nothing is counted"), 0);
}

// ----------------------------------------------------------------------------
// The module's promise: MDX is refused by name, and nothing else is called
// MDX.
// ----------------------------------------------------------------------------

/// The fault a trades catalog answers `statement` with.
fn statement_fault(label: &str, statement: &str) -> Fault {
    fault_of(&answered(
        &trades_service(label),
        Execute::statement(statement),
    ))
}

#[test]
fn every_mdx_spelling_the_provider_recognises_is_refused_naming_mdx() {
    for statement in [
        "WITH MEMBER [Measures].[Double] AS [Measures].[Sales] * 2 \
         SELECT [Measures].[Double] ON COLUMNS FROM [Sales]",
        "SELECT NON EMPTY {[Product].[Category].MEMBERS} ON AXIS(0) FROM [Sales]",
        "select {[Measures].[Sales]} on columns from [Sales]",
        "WITH SET Top5 AS TopCount(Product.Members, 5) SELECT Top5 ON ROWS FROM Sales",
        "SELECT FROM [Sales] WHERE ([Measures].[Sales])",
    ] {
        let fault = statement_fault("mdx-spellings", statement);
        assert_eq!(fault.code(), &FaultCode::Client, "{statement}");
        assert!(fault.string().contains("MDX"), "{statement}: {fault}");
        assert_eq!(xmla_error(&fault).code(), code::BAD_STATEMENT);
    }
}

#[test]
fn an_mdx_axis_spelled_by_ordinal_or_on_its_own_line_is_refused_naming_mdx() {
    // MDX names an axis `COLUMNS`, `ROWS`, `AXIS(n)` or the bare ordinal `n`,
    // and a query is as often laid out one clause per line as on one.
    let unnamed: Vec<(&str, String)> = [
        "SELECT Measures.MEMBERS ON 0 FROM Sales",
        "SELECT Measures.MEMBERS ON 0, Product.MEMBERS ON 1 FROM Sales",
        "SELECT [Measures].MEMBERS\nON COLUMNS\nFROM [Sales]",
        "SELECT [Measures].MEMBERS\tON ROWS FROM [Sales]",
    ]
    .into_iter()
    .filter_map(|statement| {
        let fault = statement_fault("mdx-axes", statement);
        assert_eq!(
            xmla_error(&fault).code(),
            code::BAD_STATEMENT,
            "{statement:?}"
        );
        (!fault.string().contains("MDX")).then(|| (statement, fault.string().to_owned()))
    })
    .collect();
    assert!(
        unnamed.is_empty(),
        "MDX refused without naming MDX: {unnamed:#?}"
    );
}

#[test]
fn a_statement_the_grammar_refuses_that_is_not_mdx_is_not_called_mdx() {
    let fault = statement_fault("not-mdx", "select symbol, from trades where");
    assert_eq!(fault.code(), &FaultCode::Client);
    assert_eq!(xmla_error(&fault).code(), code::BAD_STATEMENT);
    assert!(!fault.string().contains("MDX"), "{fault}");
}

// ----------------------------------------------------------------------------
// A literal rowset's edges, read through the module's response door.
// ----------------------------------------------------------------------------

#[test]
fn a_literal_rowset_reads_nil_escaped_unicode_blank_and_repeated_cells() {
    let message = execute_response(&format!(
        "<root xmlns=\"{ROWSET_NAMESPACE}\" xmlns:i=\"http://www.w3.org/2001/XMLSchema-instance\" \
         xmlns:s=\"{XSD}\" xmlns:q=\"{SQL_NAMESPACE}\">\
         <s:schema targetNamespace=\"{ROWSET_NAMESPACE}\" elementFormDefault=\"qualified\">\
         <s:complexType name=\"row\"><s:sequence>\
         <s:element q:field=\"Name\" name=\"Name\" type=\"s:string\"/>\
         <s:element q:field=\"Note\" name=\"Note\" type=\"s:string\" minOccurs=\"0\"/>\
         <s:element q:field=\"Qty\" name=\"Qty\" type=\"s:int\" minOccurs=\"0\"/>\
         <s:element q:field=\"Tags\" name=\"Tags\" type=\"s:string\" minOccurs=\"0\" \
         maxOccurs=\"unbounded\"/>\
         </s:sequence></s:complexType></s:schema>\
         <row><Name>Caf&#xE9; &amp; &lt;\u{6771}\u{4EAC}&gt;</Name><Note i:nil=\"true\"/>\
         <Qty>3</Qty><Tags>a</Tags><Tags>b</Tags></row>\
         <row><Name>plain</Name><Note>n</Note><Qty i:nil=\"true\"/></row>\
         <row><Name>one</Name><Tags>solo</Tags></row>\
         <row><Name>  padded  </Name><Note></Note><Qty> 42 </Qty></row>\
         </root>"
    ));
    let response = response_of(message.as_bytes());
    assert_eq!(column_names(&response), ["Name", "Note", "Qty", "Tags"]);
    let rows: Vec<[Scalar; 4]> = named_rows(&response)
        .iter()
        .map(|row| {
            [
                cell(row, "Name"),
                cell(row, "Note"),
                cell(row, "Qty"),
                cell(row, "Tags"),
            ]
        })
        .collect();
    let tags = |items: &[&str]| Scalar::from_sequence(items.iter().map(|item| Scalar::from(*item)));
    assert_eq!(
        rows,
        [
            [
                Scalar::from("Caf\u{e9} & <\u{6771}\u{4EAC}>"),
                Scalar::Null,
                Scalar::from(3_i32),
                tags(&["a", "b"]),
            ],
            [
                Scalar::from("plain"),
                Scalar::from("n"),
                Scalar::Null,
                tags(&[]),
            ],
            [
                Scalar::from("one"),
                Scalar::Null,
                Scalar::Null,
                tags(&["solo"]),
            ],
            [
                Scalar::from("  padded  "),
                Scalar::from(""),
                Scalar::from(42_i32),
                tags(&[]),
            ],
        ]
    );
}

#[test]
fn a_rowset_declaring_one_column_twice_is_refused_naming_it() {
    let message = execute_response(&format!(
        "<root xmlns=\"{ROWSET_NAMESPACE}\" xmlns:xsd=\"{XSD}\" xmlns:sql=\"{SQL_NAMESPACE}\">\
         <xsd:schema><xsd:complexType name=\"row\"><xsd:sequence>\
         <xsd:element sql:field=\"symbol\" name=\"symbol\" type=\"xsd:string\"/>\
         <xsd:element sql:field=\"symbol\" name=\"symbol2\" type=\"xsd:string\"/>\
         </xsd:sequence></xsd:complexType></xsd:schema>\
         <row><symbol>AAPL</symbol><symbol2>MSFT</symbol2></row></root>"
    ));
    let refusal = Response::from_bytes(message.as_bytes(), None)
        .expect_err("two columns of one name are no rowset");
    let text = refusal.to_string();
    assert!(text.contains("duplicate"), "{text}");
    assert!(text.contains("\"symbol\""), "{text}");
}

#[test]
fn a_required_cell_missing_from_a_row_is_refused_naming_the_row_and_column() {
    let message = execute_response(&format!(
        "<root xmlns=\"{ROWSET_NAMESPACE}\" xmlns:xsd=\"{XSD}\" xmlns:sql=\"{SQL_NAMESPACE}\">\
         <xsd:schema><xsd:complexType name=\"row\"><xsd:sequence>\
         <xsd:element sql:field=\"symbol\" name=\"symbol\" type=\"xsd:string\"/>\
         </xsd:sequence></xsd:complexType></xsd:schema>\
         <row><symbol>AAPL</symbol></row><row/></root>"
    ));
    let refusal = Response::from_bytes(message.as_bytes(), None)
        .expect_err("a required column absent from a row");
    let text = refusal.to_string();
    assert!(text.contains("$[1]"), "{text}");
    assert!(text.contains("`symbol`"), "{text}");
}

// ----------------------------------------------------------------------------
// The re-exported writers and readers over more than the one-batch path.
// ----------------------------------------------------------------------------

/// A one-column rowset field: a required `Key Word` text column.
fn keyword_field() -> yggdryl::Field {
    StructType::from_fields([DataType::utf8().required_field("Key Word")])
        .map(DataType::from)
        .expect("a valid root")
        .required_field("row")
}

/// A record column of `keyword_field` holding `words`.
fn keywords(words: &[&str]) -> Serie {
    Serie::from_scalars(
        keyword_field(),
        words
            .iter()
            .map(|word| Scalar::from_struct([("Key Word", Scalar::from(*word))]).expect("a row")),
    )
    .expect("the rows")
}

/// The `Key Word` cells of a rowset response, in row order.
fn words_of(response: &Response) -> Vec<Scalar> {
    named_rows(response)
        .iter()
        .map(|row| cell(row, "Key Word"))
        .collect()
}

#[test]
fn a_rowset_written_from_several_batches_declares_its_columns_once_and_keeps_row_order() {
    let rowset = yggdryl::xmla::Rowset::new(keyword_field()).expect("a rowset");
    let written = yggdryl::xmla::write_rowset(
        Vec::new(),
        &[],
        Method::Execute,
        &rowset,
        [
            Ok(keywords(&["select", "from"])),
            Ok(keywords(&[])),
            Ok(keywords(&["where"])),
        ],
        yggdryl::xmla::Content::SchemaData,
    )
    .expect("the rowset is written");
    let text = String::from_utf8(written.clone()).expect("UTF-8");
    assert_eq!(text.matches("<xsd:schema").count(), 1, "{text}");
    assert_eq!(text.matches("<row>").count(), 3, "{text}");
    let response = response_of(&written);
    assert_eq!(response.method(), Method::Execute);
    assert_eq!(
        words_of(&response),
        [
            Scalar::from("select"),
            Scalar::from("from"),
            Scalar::from("where")
        ]
    );

    let nothing = yggdryl::xmla::write_rowset(
        Vec::new(),
        &[],
        Method::Execute,
        &rowset,
        std::iter::empty(),
        yggdryl::xmla::Content::SchemaData,
    )
    .expect("the rowset is written");
    let response = response_of(&nothing);
    assert_eq!(response.rows().map(Serie::len), Some(0));
    assert_eq!(column_names(&response), ["Key Word"]);
}

#[test]
fn a_batch_the_rowset_cannot_hold_fails_the_write_or_is_reported_in_the_exception_namespace() {
    let rowset = yggdryl::xmla::Rowset::new(keyword_field()).expect("a rowset");
    let numbers = StructType::from_fields([DataType::Int64.required_field("Key Word")])
        .map(DataType::from)
        .expect("a valid root")
        .required_field("row");
    let wrong = || {
        Serie::from_scalars(
            numbers.clone(),
            [Scalar::from_struct([("Key Word", Scalar::from(7_i64))]).expect("a row")],
        )
        .expect("the rows")
    };

    let refusal = yggdryl::xmla::write_rowset(
        Vec::new(),
        &[],
        Method::Execute,
        &rowset,
        [Ok(keywords(&["select"])), Ok(wrong())],
        yggdryl::xmla::Content::SchemaData,
    )
    .expect_err("a batch of another column type is no row of this rowset");
    assert!(refusal.to_string().contains("`Key Word`"), "{refusal}");

    let (bytes, failed) = yggdryl::xmla::response::write_rowset_reporting(
        Vec::new(),
        &[],
        Method::Execute,
        &rowset,
        [Ok(keywords(&["select"])), Ok(wrong())],
        yggdryl::xmla::Content::SchemaData,
    )
    .expect("the failure is reported inside a complete document");
    let failed = failed.expect("the failure is handed back");
    assert_eq!(failed.code(), code::EXECUTION_FAILED);
    assert!(failed.description().contains("`Key Word`"), "{failed:?}");
    let envelope = Envelope::from_bytes(&bytes).expect("a complete envelope");
    let payload = envelope.payload().expect("a payload");
    let response = payload.element();
    let returned = response
        .one_child_in(NAMESPACE, "return")
        .expect("one return");
    let root = returned
        .children()
        .find(|child| child.local_name() == "root")
        .expect("a root");
    assert_eq!(root.namespace(), Some(ROWSET_NAMESPACE));
    let messages = root
        .child(Some(EXCEPTION_NAMESPACE), "Messages")
        .expect("the Messages in the exception namespace");
    let error = messages
        .child(Some(EXCEPTION_NAMESPACE), "Error")
        .expect("the Error in the exception namespace");
    assert_eq!(XmlaError::from_element(&error), Some(failed.clone()));
    assert_eq!(root.children_in(Some(ROWSET_NAMESPACE), "row").len(), 1);
    let refusal =
        Response::from_bytes(&bytes, None).expect_err("a reported failure is no shorter rowset");
    let text = refusal.to_string();
    assert!(
        text.contains("reported an error inside the rowset"),
        "{text}"
    );
    assert!(text.contains(failed.description()), "{text}");
}

#[test]
fn the_medium_reads_a_literal_document_s_columns_and_rows() {
    let settings = yggdryl::xmla::XmlaOptions::new();
    let held = Buffer::from_bytes(TRADES.as_bytes().to_vec())
        .with_media_type(MediaType::from_file_name("trades.xmla"));
    let expected = StructType::from_fields([
        DataType::utf8().required_field("symbol"),
        DataType::Float64.nullable_field("price"),
    ])
    .map(DataType::from)
    .expect("a valid root")
    .required_field("row");
    assert_eq!(
        yggdryl::xmla::read_field(&held, &settings).expect("the schema"),
        expected
    );
    let batches: Vec<_> = yggdryl::xmla::read_batch_reader(&held, None, &settings)
        .expect("a reader")
        .map(|batch| batch.expect("a batch"))
        .collect();
    let [batch] = batches.as_slice() else {
        panic!("one batch, got {}", batches.len());
    };
    assert_eq!(batch.num_rows(), 3);
    assert_eq!(batch.column(1).null_count(), 1, "MSFT has no price");

    let empty = Buffer::new().with_media_type(MediaType::from_file_name("trades.xmla"));
    let refusal = yggdryl::xmla::read_field(&empty, &settings)
        .expect_err("an empty document states no columns");
    assert!(
        refusal
            .to_string()
            .contains("empty document declares no schema"),
        "{refusal}"
    );
}

#[test]
fn the_rows_an_execution_streams_are_the_rows_its_answer_carries() {
    let service = trades_service("execution");
    let execute =
        Execute::statement("select symbol, price from trades where price is not null limit 10");
    let Ok(yggdryl::xmla::Execution::Rowset { rowset, rows }) = service.execute(&execute) else {
        panic!("the statement is answered with a rowset");
    };
    let streamed: Vec<Scalar> = rows
        .flat_map(|batch| {
            let batch = batch.expect("a batch");
            (0..batch.len())
                .map(|index| batch.get(index).expect("a row").into_owned())
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(streamed.len(), 2);
    let bytes = answered(&service, execute);
    let response = Response::from_bytes(&bytes, Some(rowset.field()))
        .unwrap_or_else(|error| panic!("{error}\n{}", String::from_utf8_lossy(&bytes)));
    let carried = response.rows().expect("a rowset");
    let carried: Vec<Scalar> = (0..carried.len())
        .map(|index| carried.get(index).expect("a row").into_owned())
        .collect();
    assert_eq!(carried, streamed);
}

#[test]
fn only_the_schema_instance_namespace_marks_a_cell_nil() {
    let message = execute_response(&format!(
        "<root xmlns=\"{ROWSET_NAMESPACE}\" xmlns:xsd=\"{XSD}\" \
         xmlns:w=\"http://www.w3.org/2001/XMLSchema-instance\" xmlns:o=\"urn:example:other\">\
         <xsd:schema><xsd:complexType name=\"row\"><xsd:sequence>\
         <xsd:element name=\"Note\" type=\"xsd:string\" minOccurs=\"0\"/>\
         </xsd:sequence></xsd:complexType></xsd:schema>\
         <row><Note w:nil=\"1\"/></row>\
         <row><Note w:nil=\" true \"/></row>\
         <row><Note w:nil=\"false\">kept</Note></row>\
         <row><Note o:nil=\"true\">foreign</Note></row>\
         </root>"
    ));
    let response = response_of(message.as_bytes());
    let notes: Vec<Scalar> = named_rows(&response)
        .iter()
        .map(|row| cell(row, "Note"))
        .collect();
    assert_eq!(
        notes,
        [
            Scalar::Null,
            Scalar::Null,
            Scalar::from("kept"),
            Scalar::from("foreign")
        ]
    );
}

#[test]
fn a_table_whose_stored_document_is_broken_earns_no_rows() {
    let root = scratch("broken");
    std::fs::write(
        root.join("broken.xmla"),
        format!("<root xmlns=\"{ROWSET_NAMESPACE}\"><row><symbol>AAPL</symbol>"),
    )
    .expect("the table is written");
    let service = empty_service().with_catalog(Catalog::new(
        "market",
        Holder::folder(&root).expect("the catalog holds"),
    ));
    let bytes = answered(&service, Execute::statement("select * from broken"));
    let refusal = Response::from_bytes(&bytes, None)
        .expect_err("a document that does not parse answers no rowset");
    assert!(
        refusal
            .to_string()
            .contains(&format!("({} ", code::EXECUTION_FAILED)),
        "{refusal}"
    );
    let fault = fault_of(&bytes);
    assert_eq!(
        fault.code(),
        &FaultCode::Server,
        "this side is at fault: {fault}"
    );
    assert_eq!(xmla_error(&fault).code(), code::EXECUTION_FAILED);
    assert!(
        fault.string().contains("broken.xmla"),
        "the fault names the table: {fault}"
    );
}

#[test]
fn a_column_s_name_and_type_are_read_by_namespace_not_by_prefix() {
    // `q` is the SQL namespace and `s` XML Schema's; `o` is neither, and
    // `xsd` is bound here to a namespace that is not XML Schema's.
    let message = execute_response(&format!(
        "<root xmlns=\"{ROWSET_NAMESPACE}\" xmlns:s=\"{XSD}\" xmlns:q=\"{SQL_NAMESPACE}\" \
         xmlns:o=\"urn:example:other\" xmlns:xsd=\"urn:example:not-xml-schema\">\
         <s:schema><s:complexType name=\"row\"><s:sequence>\
         <s:element q:field=\"Named\" o:field=\"Other\" name=\"A\" type=\"s:int\"/>\
         <s:element o:field=\"Foreign\" name=\"Key_x0020_Word\" type=\"xsd:int\"/>\
         <s:element field=\"Bare\" name=\"C\" type=\"o:int\"/>\
         </s:sequence></s:complexType></s:schema>\
         <row><A>1</A><Key_x0020_Word>2</Key_x0020_Word><C>3</C></row>\
         </root>"
    ));
    let response = response_of(message.as_bytes());
    assert_eq!(column_names(&response), ["Named", "Key Word", "C"]);
    let rows = named_rows(&response);
    let [row] = rows.as_slice() else {
        panic!("one row, got {}", rows.len());
    };
    assert_eq!(cell(row, "Named"), Scalar::from(1_i32));
    // A type outside XML Schema's namespace is one this reader does not
    // know, and a type it does not know is text.
    assert_eq!(cell(row, "Key Word"), Scalar::from("2"));
    assert_eq!(cell(row, "C"), Scalar::from("3"));
}

#[test]
fn a_schema_in_another_namespace_is_no_schema_of_the_rowset() {
    let message = execute_response(&format!(
        "<root xmlns=\"{ROWSET_NAMESPACE}\" xmlns:o=\"urn:example:other\">\
         <o:schema><o:complexType name=\"row\"><o:sequence>\
         <o:element name=\"Keyword\" type=\"o:string\"/>\
         </o:sequence></o:complexType></o:schema>\
         <row><Keyword>select</Keyword></row></root>"
    ));
    let refusal = Response::from_bytes(message.as_bytes(), None)
        .expect_err("rows with no XML Schema state no columns");
    assert!(refusal.to_string().contains("no schema"), "{refusal}");
    let read = Response::from_bytes(message.as_bytes(), Some(&keyword_row_field()))
        .expect("a declared field reads the rows the document cannot type");
    assert_eq!(
        read.rows().map(Serie::len),
        Some(1),
        "the row is read under the declared field"
    );
}

/// A one-column rowset field: a required `Keyword` text column.
fn keyword_row_field() -> yggdryl::Field {
    StructType::from_fields([DataType::utf8().required_field("Keyword")])
        .map(DataType::from)
        .expect("a valid root")
        .required_field("row")
}
