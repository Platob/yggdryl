//! `rust/src/xmla/mod.rs`: the six XML for Analysis 1.1 namespaces, every
//! name the module re-exports at `yggdryl::xmla`, and the module's promise
//! end to end - a request built through the API or spelled by hand, answered
//! by the tabular provider, and read back as the rowset, the empty answer,
//! the dataset or the SOAP fault its namespaces say it is.

use std::path::PathBuf;

use yggdryl::holder::{Buffer, Holder};
use yggdryl::soap::{ENVELOPE_NAMESPACE, Envelope, Fault, FaultCode};
use yggdryl::xml::Element;
use yggdryl::xmla::definitions::definition_of;
use yggdryl::xmla::service::code;
use yggdryl::xmla::{
    Answer, Catalog, Discover, EMPTY_NAMESPACE, EXCEPTION_NAMESPACE, Execute, MDDATASET_NAMESPACE,
    Method, NAMESPACE, PropertyList, ROWSET_NAMESPACE, Request, RequestType, Response,
    Restrictions, SQL_NAMESPACE, Service, ServiceOptions, XmlaError, property,
};
use yggdryl::{DataType, MediaType, Scalar, Serie, StructType};

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
