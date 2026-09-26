//! `rust/src/xmla/request.rs`: the two XML for Analysis requests - `Discover`
//! and `Execute` with their builders, the `Command` an Execute runs, the
//! `Session` header block read out of and written into a header, and one
//! `Request` read out of literal SOAP 1.1 envelopes and written back into one.
//!
//! `invalid` is crate-private and pinned only through the refusals it spells,
//! whose path is `$.xmla`.

use yggdryl::soap::{Body, ENVELOPE_NAMESPACE, Envelope, Fault, Fragment};
use yggdryl::xmla::{
    Command, Discover, Execute, Method, NAMESPACE, PropertyList, Request, RequestMethod,
    RequestType, Restrictions, Session,
};
use yggdryl::{Error, Scalar};

/// The Analysis Services scripting namespace, a command language XMLA
/// carries without owning.
const ENGINE: &str = "http://schemas.microsoft.com/analysisservices/2003/engine";

/// A Discover for the tables of `market`, as a client writes one: indented,
/// every XMLA element under the default namespace the payload declares.
const DISCOVER_TABLES: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<SOAP-ENV:Envelope xmlns:SOAP-ENV="http://schemas.xmlsoap.org/soap/envelope/">
  <SOAP-ENV:Body>
    <Discover xmlns="urn:schemas-microsoft-com:xml-analysis">
      <RequestType>DBSCHEMA_TABLES</RequestType>
      <Restrictions>
        <RestrictionList>
          <CATALOG_NAME>market</CATALOG_NAME>
        </RestrictionList>
      </Restrictions>
      <Properties>
        <PropertyList>
          <Content>SchemaData</Content>
          <Format>Tabular</Format>
        </PropertyList>
      </Properties>
    </Discover>
  </SOAP-ENV:Body>
</SOAP-ENV:Envelope>
"#;

/// The same Discover under the `soap:` and `xmla:` prefixes, both declared on
/// the envelope.
const DISCOVER_TABLES_PREFIXED: &str = r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/" xmlns:xmla="urn:schemas-microsoft-com:xml-analysis">
  <soap:Body>
    <xmla:Discover>
      <xmla:RequestType>DBSCHEMA_TABLES</xmla:RequestType>
      <xmla:Restrictions>
        <xmla:RestrictionList>
          <xmla:CATALOG_NAME>market</xmla:CATALOG_NAME>
        </xmla:RestrictionList>
      </xmla:Restrictions>
      <xmla:Properties>
        <xmla:PropertyList>
          <xmla:Content>SchemaData</xmla:Content>
          <xmla:Format>Tabular</xmla:Format>
        </xmla:PropertyList>
      </xmla:Properties>
    </xmla:Discover>
  </soap:Body>
</soap:Envelope>"#;

/// An Execute of one statement with its properties and two parameters.
const EXECUTE_TRADES: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<SOAP-ENV:Envelope xmlns:SOAP-ENV="http://schemas.xmlsoap.org/soap/envelope/">
  <SOAP-ENV:Body>
    <Execute xmlns="urn:schemas-microsoft-com:xml-analysis">
      <Command>
        <Statement>select symbol, price from trades where price &gt; 100 limit 10</Statement>
      </Command>
      <Properties>
        <PropertyList>
          <Catalog>market</Catalog>
          <Format>Tabular</Format>
        </PropertyList>
      </Properties>
      <Parameters>
        <Parameter>
          <Name>symbol</Name>
          <Value>AAPL</Value>
        </Parameter>
        <Parameter>
          <Name>limit</Name>
          <Value>10</Value>
        </Parameter>
      </Parameters>
    </Execute>
  </SOAP-ENV:Body>
</SOAP-ENV:Envelope>"#;

/// The same Execute under the `soap:` prefix, the XMLA elements under
/// `xmla:` declared on the payload itself.
const EXECUTE_TRADES_PREFIXED: &str = r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
  <soap:Body>
    <xmla:Execute xmlns:xmla="urn:schemas-microsoft-com:xml-analysis">
      <xmla:Command>
        <xmla:Statement>select symbol, price from trades where price &gt; 100 limit 10</xmla:Statement>
      </xmla:Command>
      <xmla:Properties>
        <xmla:PropertyList>
          <xmla:Catalog>market</xmla:Catalog>
          <xmla:Format>Tabular</xmla:Format>
        </xmla:PropertyList>
      </xmla:Properties>
      <xmla:Parameters>
        <xmla:Parameter>
          <xmla:Name>symbol</xmla:Name>
          <xmla:Value>AAPL</xmla:Value>
        </xmla:Parameter>
        <xmla:Parameter>
          <xmla:Name>limit</xmla:Name>
          <xmla:Value>10</xmla:Value>
        </xmla:Parameter>
      </xmla:Parameters>
    </xmla:Execute>
  </soap:Body>
</soap:Envelope>"#;

/// A SOAP 1.1 envelope under the specification's `SOAP-ENV` prefix, holding
/// the `header` blocks - no `Header` element when there are none - and the
/// one `body` element.
fn envelope(header: &str, body: &str) -> String {
    let header = if header.is_empty() {
        String::new()
    } else {
        format!("<SOAP-ENV:Header>{header}</SOAP-ENV:Header>")
    };
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
         <SOAP-ENV:Envelope xmlns:SOAP-ENV=\"http://schemas.xmlsoap.org/soap/envelope/\">\
         {header}<SOAP-ENV:Body>{body}</SOAP-ENV:Body></SOAP-ENV:Envelope>"
    )
}

/// A Discover payload in the XMLA namespace holding `children`.
fn discover_body(children: &str) -> String {
    format!("<Discover xmlns=\"urn:schemas-microsoft-com:xml-analysis\">{children}</Discover>")
}

/// An Execute payload in the XMLA namespace holding `children`.
fn execute_body(children: &str) -> String {
    format!("<Execute xmlns=\"urn:schemas-microsoft-com:xml-analysis\">{children}</Execute>")
}

fn read(message: &str) -> Request {
    Request::from_bytes(message.as_bytes()).unwrap_or_else(|error| panic!("{error}\n{message}"))
}

fn refused(message: &str) -> Error {
    match Request::from_bytes(message.as_bytes()) {
        Ok(request) => panic!("expected a refusal, read {request:?}\n{message}"),
        Err(error) => error,
    }
}

/// The reason of a refusal this module raises itself, which is an invalid
/// record at `$.xmla`.
fn xmla_reason(error: &Error) -> String {
    match error {
        Error::InvalidRecord { path, reason } if path == "$.xmla" => reason.to_string(),
        other => panic!("expected a refusal at $.xmla, got {other:?}"),
    }
}

/// The reason of a codec refusal spelled for `expected`: `xml` for the element
/// view, `soap` for the envelope.
fn codec_reason(error: &Error, expected: &str) -> String {
    match error {
        Error::Codec { format, reason, .. } if *format == expected => reason.to_string(),
        other => panic!("expected a {expected} codec refusal, got {other:?}"),
    }
}

fn written(request: &Request) -> String {
    String::from_utf8(request.into_bytes().expect("the request is written")).expect("UTF-8")
}

/// `request` written and read back.
fn round_trip(request: &Request) -> Request {
    let bytes = request.into_bytes().expect("the request is written");
    Request::from_bytes(&bytes)
        .unwrap_or_else(|error| panic!("{error}\n{}", String::from_utf8_lossy(&bytes)))
}

/// The values `name` admits, as text.
fn admitted<'a>(restrictions: &'a Restrictions, name: &str) -> Vec<&'a str> {
    restrictions
        .get(name)
        .unwrap_or_else(|| panic!("{name} is restricted in {restrictions:?}"))
        .iter()
        .map(String::as_str)
        .collect()
}

fn parameters(execute: &Execute) -> Vec<(&str, Scalar)> {
    execute
        .parameters()
        .iter()
        .map(|(name, value)| (name.as_str(), value.clone()))
        .collect()
}

fn properties(list: &PropertyList) -> Vec<(&str, &str)> {
    list.entries()
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect()
}

fn attributes<const N: usize>(entries: [(&str, &str); N]) -> Scalar {
    Scalar::from_struct(entries.map(|(name, value)| (name, Scalar::from(value))))
        .expect("a record of attributes")
}

/// A header block in the XMLA namespace carrying `SessionId="id"`.
fn session_block(name: &str, id: &str) -> Fragment {
    Fragment::in_namespace(name, NAMESPACE, attributes([("@SessionId", id)]))
        .expect("a namespaced block")
}

fn tables_discover() -> Discover {
    Discover::new(RequestType::DbschemaTables)
        .with_restrictions(Restrictions::new().with("CATALOG_NAME", "market"))
        .with_properties(
            PropertyList::new()
                .with("Content", "SchemaData")
                .with("Format", "Tabular"),
        )
}

fn trades_execute() -> Execute {
    Execute::statement("select symbol, price from trades where price > 100 limit 10")
        .with_properties(
            PropertyList::new()
                .with("Catalog", "market")
                .with("Format", "Tabular"),
        )
        .with_parameter("symbol", Scalar::from("AAPL"))
        .with_parameter("limit", Scalar::from("10"))
}

// Discover ---------------------------------------------------------------

#[test]
fn a_discover_is_unrestricted_and_has_no_property_until_given_some() {
    let discover = Discover::new(RequestType::DiscoverDatasources);
    assert_eq!(discover.request_type(), &RequestType::DiscoverDatasources);
    assert!(discover.restrictions().is_empty());
    assert!(discover.properties().is_empty());
}

#[test]
fn a_discover_carries_the_restrictions_and_properties_it_is_given_the_last_set_winning() {
    let mut discover = Discover::new(RequestType::DbschemaTables)
        .with_restrictions(Restrictions::new().with("TABLE_SCHEMA", "eu"))
        .with_restrictions(Restrictions::new().with("TABLE_CATALOG", "market"))
        .with_properties(PropertyList::new().with("Content", "Schema"))
        .with_properties(PropertyList::new().with("Format", "Tabular"));
    assert_eq!(discover.request_type(), &RequestType::DbschemaTables);
    assert_eq!(
        admitted(discover.restrictions(), "TABLE_CATALOG"),
        ["market"]
    );
    assert_eq!(
        discover.restrictions().get("TABLE_SCHEMA"),
        None,
        "a later set of restrictions replaces the earlier one"
    );
    assert_eq!(discover.properties().get("Format"), Some("Tabular"));
    assert_eq!(
        discover.properties().get("Content"),
        None,
        "a later property list replaces the earlier one"
    );

    discover.properties_mut().set("Catalog", "market");
    discover.properties_mut().set("format", "Multidimensional");
    assert_eq!(
        properties(discover.properties()),
        [("Format", "Multidimensional"), ("Catalog", "market")],
        "the list is set in place, a name matched case-insensitively"
    );
}

// Command ----------------------------------------------------------------

#[test]
fn a_statement_command_answers_its_text_and_the_statement_name() {
    let command = Command::Statement("select symbol from trades".to_owned());
    assert_eq!(command.statement(), Some("select symbol from trades"));
    assert_eq!(command.name(), "Statement");

    let empty = Command::Statement(String::new());
    assert_eq!(
        empty.statement(),
        Some(""),
        "an empty statement is still one"
    );
    assert_eq!(empty.name(), "Statement");
}

#[test]
fn any_other_command_answers_no_statement_and_its_element_name_as_spelled() {
    let cancel = Command::Other(Fragment::new("Cancel", Scalar::Null));
    assert_eq!(cancel.statement(), None);
    assert_eq!(cancel.name(), "Cancel");

    let batch = Command::Other(Fragment::new(
        "as:Batch",
        attributes([("@xmlns:as", ENGINE)]),
    ));
    assert_eq!(batch.statement(), None);
    assert_eq!(
        batch.name(),
        "as:Batch",
        "the prefix is part of the spelling"
    );
}

// Execute ----------------------------------------------------------------

#[test]
fn an_execute_of_a_statement_is_an_execute_of_the_statement_command() {
    let execute = Execute::statement("select 1");
    assert_eq!(
        execute,
        Execute::new(Command::Statement("select 1".to_owned()))
    );
    assert_eq!(execute.command().statement(), Some("select 1"));
    assert!(execute.properties().is_empty());
    assert!(execute.parameters().is_empty());

    let cancel = Execute::new(Command::Other(Fragment::new("Cancel", Scalar::Null)));
    assert_eq!(cancel.command().name(), "Cancel");
    assert_eq!(cancel.command().statement(), None);
    assert!(cancel.properties().is_empty());
    assert!(cancel.parameters().is_empty());
}

#[test]
fn an_execute_keeps_its_parameters_in_the_order_given_and_its_last_properties() {
    let mut execute = Execute::statement("select * from trades where symbol = @symbol")
        .with_properties(PropertyList::new().with("Content", "Schema"))
        .with_properties(PropertyList::new().with("Format", "Tabular"))
        .with_parameter("symbol", Scalar::from("AAPL"))
        .with_parameter("limit", Scalar::from(10_i64))
        .with_parameter("symbol", Scalar::from("MSFT"));
    assert_eq!(
        parameters(&execute),
        [
            ("symbol", Scalar::from("AAPL")),
            ("limit", Scalar::from(10_i64)),
            ("symbol", Scalar::from("MSFT")),
        ],
        "each parameter is one more, a repeated name included, and keeps its type"
    );
    assert_eq!(properties(execute.properties()), [("Format", "Tabular")]);

    execute.properties_mut().set("Catalog", "market");
    assert_eq!(execute.properties().get("CATALOG"), Some("market"));
    assert_eq!(execute.properties().len(), 2);
}

// Session ----------------------------------------------------------------

#[test]
fn each_session_header_names_its_element_and_the_identifier_it_carries() {
    assert_eq!(Session::Begin.name(), "BeginSession");
    assert_eq!(Session::Begin.session_id(), None);

    let session = Session::Continue("581".into());
    assert_eq!(session.name(), "Session");
    assert_eq!(session.session_id(), Some("581"));

    let end = Session::End("581".into());
    assert_eq!(end.name(), "EndSession");
    assert_eq!(end.session_id(), Some("581"));
}

#[test]
fn a_session_header_is_written_in_the_xmla_namespace_marked_must_understand() {
    for (session, id) in [
        (Session::Continue("581".into()), Some("581")),
        (Session::End("581".into()), Some("581")),
        (Session::Begin, None),
    ] {
        let block = session.into_fragment().expect("a header block");
        assert_eq!(block.name(), session.name());
        let element = block.element();
        assert_eq!(element.local_name(), session.name());
        assert_eq!(element.namespace(), Some(NAMESPACE), "{session:?}");
        assert_eq!(
            element.attribute("SessionId").and_then(Scalar::as_str),
            id,
            "XMLA's unqualified SessionId attribute, absent on BeginSession"
        );
        assert_eq!(
            element.attribute("mustUnderstand").and_then(Scalar::as_str),
            Some("1"),
            "unqualified, as Excel spells it and the reference providers read it: {session:?}"
        );
        assert_eq!(element.children().count(), 0, "a header block is empty");
    }

    // Inside the envelope the block keeps its unqualified mark, in no namespace.
    let request = Request::from(Discover::new(RequestType::DiscoverDatasources))
        .with_session(&Session::Continue("581".into()))
        .expect("a session header");
    let bytes = request.into_bytes().expect("written");
    let envelope = Envelope::from_bytes(&bytes).expect("an envelope");
    assert_eq!(envelope.header().len(), 1);
    let block = envelope.header()[0].element();
    assert_eq!(block.namespace(), Some(NAMESPACE));
    assert_eq!(block.attribute_in(None, "mustUnderstand"), Some("1"));
    assert_eq!(
        block.attribute_in(Some(ENVELOPE_NAMESPACE), "mustUnderstand"),
        None
    );
    assert_eq!(block.attribute_in(None, "SessionId"), Some("581"));
}

#[test]
fn session_read_answers_none_without_a_session_block() {
    assert_eq!(Session::read(&[]).expect("no header"), None);

    let security = Fragment::in_namespace(
        "Security",
        "http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-wssecurity-secext-1.0.xsd",
        Scalar::Null,
    )
    .expect("a block");
    let foreign = Fragment::in_namespace(
        "Session",
        "urn:example:other",
        attributes([("@SessionId", "7")]),
    )
    .expect("a block");
    assert_eq!(
        Session::read(&[security, foreign]).expect("no session block"),
        None,
        "a Session in another namespace is another protocol's header"
    );
}

#[test]
fn session_read_answers_the_block_it_finds_in_the_xmla_namespace_or_in_none() {
    let begin = Fragment::in_namespace("BeginSession", NAMESPACE, Scalar::Null).expect("a block");
    assert_eq!(Session::read(&[begin]).expect("read"), Some(Session::Begin));

    let unqualified = Fragment::new("BeginSession", Scalar::Null);
    assert_eq!(
        Session::read(&[unqualified]).expect("read"),
        Some(Session::Begin),
        "an unqualified block is the one the specification names"
    );

    let named = session_block("BeginSession", "chosen-by-the-client");
    assert_eq!(
        Session::read(&[named]).expect("read"),
        Some(Session::Begin),
        "the identifier is the server's to choose, so one a client names is not read"
    );

    assert_eq!(
        Session::read(&[session_block("Session", "581")]).expect("read"),
        Some(Session::Continue("581".into()))
    );
    assert_eq!(
        Session::read(&[session_block("EndSession", "581")]).expect("read"),
        Some(Session::End("581".into()))
    );

    let prefixed = Fragment::new(
        "x:Session",
        attributes([("@xmlns:x", NAMESPACE), ("@SessionId", "9")]),
    );
    assert_eq!(
        Session::read(&[prefixed]).expect("read"),
        Some(Session::Continue("9".into())),
        "a prefix reads by the namespace it names"
    );

    let security =
        Fragment::in_namespace("Security", "urn:example:security", Scalar::Null).expect("a block");
    assert_eq!(
        Session::read(&[security, session_block("Session", "élan-7")]).expect("read"),
        Some(Session::Continue("élan-7".into())),
        "another block beside the session is passed over"
    );
}

#[test]
fn a_session_or_end_session_block_without_a_session_id_is_refused() {
    for name in ["Session", "EndSession"] {
        let reason = format!("the {name} header names no SessionId");

        let bare = Fragment::in_namespace(name, NAMESPACE, Scalar::Null).expect("a block");
        let error = Session::read(&[bare]).expect_err("no identifier");
        assert_eq!(xmla_reason(&error), reason);
        assert_eq!(
            error.to_string(),
            format!("invalid record value at $.xmla: {reason}")
        );

        let empty = session_block(name, "");
        assert_eq!(
            xmla_reason(&Session::read(&[empty]).expect_err("an empty identifier")),
            reason,
            "an empty identifier names none"
        );

        let prefixed = Fragment::new(
            format!("xmla:{name}"),
            attributes([("@xmlns:xmla", NAMESPACE)]),
        );
        assert_eq!(
            xmla_reason(&Session::read(&[prefixed]).expect_err("no identifier")),
            reason,
            "the refusal names the block by its local name"
        );
    }
}

#[test]
fn more_than_one_session_block_is_refused() {
    let begin = Fragment::in_namespace("BeginSession", NAMESPACE, Scalar::Null).expect("a block");
    let error =
        Session::read(&[begin, session_block("Session", "581")]).expect_err("two session blocks");
    assert_eq!(
        xmla_reason(&error),
        "a request carries more than one session header"
    );

    let error = Session::read(&[session_block("Session", "1"), session_block("Session", "1")])
        .expect_err("the same block twice");
    assert_eq!(
        xmla_reason(&error),
        "a request carries more than one session header"
    );
}

#[test]
fn a_written_session_header_reads_back_as_the_same_session() {
    for session in [
        Session::Begin,
        Session::Continue("581".into()),
        Session::End("581".into()),
        Session::Continue("sé ssion \"7\" <&> '→'".into()),
    ] {
        let block = session.into_fragment().expect("a header block");
        assert_eq!(
            Session::read(std::slice::from_ref(&block)).expect("read"),
            Some(session.clone())
        );

        let request = Request::from(Discover::new(RequestType::DiscoverDatasources))
            .with_session(&session)
            .expect("a session header");
        assert_eq!(
            round_trip(&request).session().expect("read"),
            Some(session),
            "{}",
            written(&request)
        );
    }
}

// Request ----------------------------------------------------------------

#[test]
fn a_request_answers_its_method_kind_and_properties() {
    let discover = tables_discover();
    let request = Request::new(RequestMethod::Discover(discover.clone()));
    assert_eq!(request, Request::from(discover.clone()));
    assert!(request.header().is_empty());
    assert_eq!(request.method(), &RequestMethod::Discover(discover.clone()));
    assert_eq!(request.kind(), Method::Discover);
    assert_eq!(request.discover(), Some(&discover));
    assert_eq!(request.execute(), None);
    assert_eq!(request.properties(), discover.properties());
    assert_eq!(request.session().expect("no header"), None);

    let execute = trades_execute();
    let request = Request::new(RequestMethod::Execute(execute.clone()));
    assert_eq!(request, Request::from(execute.clone()));
    assert!(request.header().is_empty());
    assert_eq!(request.method(), &RequestMethod::Execute(execute.clone()));
    assert_eq!(request.kind(), Method::Execute);
    assert_eq!(request.discover(), None);
    assert_eq!(request.execute(), Some(&execute));
    assert_eq!(request.properties(), execute.properties());
    assert_eq!(request.session().expect("no header"), None);
}

#[test]
fn a_request_carries_its_header_blocks_in_the_order_added() {
    let security =
        Fragment::in_namespace("Security", "urn:example:security", Scalar::Null).expect("a block");
    let session = Session::Continue("581".into());
    let request = Request::from(Execute::statement("select 1"))
        .with_header(security.clone())
        .with_session(&session)
        .expect("a session header");
    assert_eq!(
        request.header(),
        [security, session.into_fragment().expect("a header block")]
    );
    assert_eq!(request.session().expect("one session"), Some(session));

    let doubled = request
        .with_session(&Session::Begin)
        .expect("a second block is added");
    assert_eq!(doubled.header().len(), 3);
    assert_eq!(
        xmla_reason(&doubled.session().expect_err("two session headers")),
        "a request carries more than one session header"
    );
}

// Reading ----------------------------------------------------------------

#[test]
fn a_discover_envelope_reads_its_request_type_restrictions_and_properties() {
    let request = read(DISCOVER_TABLES);
    assert_eq!(request.kind(), Method::Discover);
    assert!(request.header().is_empty());
    assert_eq!(request.execute(), None);
    assert_eq!(request.session().expect("no header"), None);

    let discover = request.discover().expect("a Discover");
    assert_eq!(discover.request_type(), &RequestType::DbschemaTables);
    assert_eq!(
        admitted(discover.restrictions(), "CATALOG_NAME"),
        ["market"]
    );
    assert_eq!(discover.restrictions().len(), 1);
    assert_eq!(discover.properties().get("Content"), Some("SchemaData"));
    assert_eq!(discover.properties().get("Format"), Some("Tabular"));
    assert_eq!(discover.properties().len(), 2);
    assert_eq!(request.properties(), discover.properties());
    assert_eq!(discover, &tables_discover());
}

#[test]
fn a_discover_with_empty_restrictions_and_no_properties_reads_as_unrestricted() {
    for children in [
        "<RequestType>DISCOVER_DATASOURCES</RequestType><Restrictions/>",
        "<RequestType>DISCOVER_DATASOURCES</RequestType>\
         <Restrictions><RestrictionList/></Restrictions>",
        "<RequestType>DISCOVER_DATASOURCES</RequestType>\
         <Restrictions><RestrictionList></RestrictionList></Restrictions>\
         <Properties><PropertyList/></Properties>",
        "<RequestType>DISCOVER_DATASOURCES</RequestType><Restrictions></Restrictions><Properties/>",
        "<RequestType>DISCOVER_DATASOURCES</RequestType>",
    ] {
        let request = read(&envelope("", &discover_body(children)));
        assert_eq!(
            request.discover(),
            Some(&Discover::new(RequestType::DiscoverDatasources)),
            "{children}"
        );
    }
}

#[test]
fn a_restriction_written_several_times_admits_each_value_in_document_order() {
    let request = read(&envelope(
        "",
        &discover_body(
            "<RequestType>DBSCHEMA_TABLES</RequestType><Restrictions><RestrictionList>\
             <TABLE_TYPE>TABLE</TABLE_TYPE><TABLE_CATALOG>marché</TABLE_CATALOG>\
             <TABLE_TYPE>VIEW</TABLE_TYPE><TABLE_TYPE></TABLE_TYPE>\
             </RestrictionList></Restrictions>",
        ),
    ));
    let restrictions = request.discover().expect("a Discover").restrictions();
    assert_eq!(
        admitted(restrictions, "TABLE_TYPE"),
        ["TABLE", "VIEW", ""],
        "an element holding nothing admits the empty text"
    );
    assert_eq!(admitted(restrictions, "table_catalog"), ["marché"]);
    assert_eq!(restrictions.len(), 2);
}

#[test]
fn a_request_type_reads_case_insensitively_trimmed_and_an_unknown_one_is_kept_as_spelled() {
    for (spelled, expected) in [
        ("DBSCHEMA_TABLES", RequestType::DbschemaTables),
        ("dbschema_tables", RequestType::DbschemaTables),
        ("\n  Discover_Properties\t", RequestType::DiscoverProperties),
        ("MDSCHEMA_CUBES", RequestType::MdschemaCubes),
        ("MY_ROWSET", RequestType::Other("MY_ROWSET".into())),
        (" my_rowset ", RequestType::Other("my_rowset".into())),
    ] {
        let request = read(&envelope(
            "",
            &discover_body(&format!("<RequestType>{spelled}</RequestType>")),
        ));
        assert_eq!(
            request.discover().expect("a Discover").request_type(),
            &expected,
            "{spelled:?}"
        );
    }
}

#[test]
fn a_discover_without_exactly_one_request_type_is_refused_by_name() {
    let error = refused(&envelope("", &discover_body("<Restrictions/>")));
    assert_eq!(
        codec_reason(&error, "xml"),
        "expected one `RequestType` element under `Discover`, found none"
    );

    let error = refused(&envelope(
        "",
        &discover_body(
            "<RequestType>DBSCHEMA_TABLES</RequestType><RequestType>DBSCHEMA_COLUMNS</RequestType>",
        ),
    ));
    assert_eq!(
        codec_reason(&error, "xml"),
        "expected one `RequestType` element under `Discover`, found several"
    );

    let error = refused(&envelope(
        "",
        &discover_body("<RequestType xmlns=\"urn:example:other\">DBSCHEMA_TABLES</RequestType>"),
    ));
    assert_eq!(
        codec_reason(&error, "xml"),
        "expected one `RequestType` element under `Discover`, found none",
        "a RequestType in another namespace is not the one XMLA names"
    );

    let error = refused(&envelope(
        "",
        "<xmla:Discover xmlns:xmla=\"urn:schemas-microsoft-com:xml-analysis\"/>",
    ));
    assert_eq!(
        codec_reason(&error, "xml"),
        "expected one `RequestType` element under `xmla:Discover`, found none",
        "the payload is named as it is spelled"
    );

    for empty in [
        "<RequestType/>",
        "<RequestType></RequestType>",
        "<RequestType> \n </RequestType>",
    ] {
        let error = refused(&envelope("", &discover_body(empty)));
        match &error {
            Error::InvalidRecord { path, reason } => {
                assert_eq!(path, "$.RequestType", "{empty}");
                assert_eq!(reason, "expected a rowset name, got \"\"", "{empty}");
            }
            other => panic!("expected a RequestType refusal for {empty}, got {other:?}"),
        }
    }
}

#[test]
fn an_execute_envelope_reads_its_statement_properties_and_parameters() {
    let request = read(EXECUTE_TRADES);
    assert_eq!(request.kind(), Method::Execute);
    assert!(request.header().is_empty());
    assert_eq!(request.discover(), None);

    let execute = request.execute().expect("an Execute");
    assert_eq!(
        execute.command().statement(),
        Some("select symbol, price from trades where price > 100 limit 10")
    );
    assert_eq!(execute.command().name(), "Statement");
    assert_eq!(
        properties(execute.properties()),
        [("Catalog", "market"), ("Format", "Tabular")]
    );
    assert_eq!(
        parameters(execute),
        [
            ("symbol", Scalar::from("AAPL")),
            ("limit", Scalar::from("10")),
        ],
        "each parameter in document order, its value the text it holds"
    );
    assert_eq!(request.properties(), execute.properties());
    assert_eq!(execute, &trades_execute());
}

#[test]
fn a_parameter_value_is_its_natural_value_and_absent_is_null() {
    let request = read(&envelope(
        "",
        &execute_body(
            "<Command><Statement>select 1</Statement></Command><Parameters>\
             <Parameter><Name>missing</Name></Parameter>\
             <Parameter><Name>closed</Name><Value/></Parameter>\
             <Parameter><Name>empty</Name><Value></Value></Parameter>\
             <Parameter><Name>padded</Name><Value>  AAPL  </Value></Parameter>\
             <Parameter><Name>nested</Name><Value><symbol>AAPL</symbol></Value></Parameter>\
             </Parameters>",
        ),
    ));
    assert_eq!(
        parameters(request.execute().expect("an Execute")),
        [
            ("missing", Scalar::Null),
            ("closed", Scalar::Null),
            ("empty", Scalar::from("")),
            ("padded", Scalar::from("  AAPL  ")),
            (
                "nested",
                Scalar::from_struct([("symbol", Scalar::from("AAPL"))]).expect("a record")
            ),
        ]
    );

    let request = read(&envelope(
        "",
        &execute_body("<Command><Statement>select 1</Statement></Command><Parameters/>"),
    ));
    assert!(
        request
            .execute()
            .expect("an Execute")
            .parameters()
            .is_empty(),
        "an empty Parameters element names none"
    );
}

#[test]
fn a_statement_keeps_its_text_exactly_cdata_and_references_resolved() {
    for (spelled, expected) in [
        (
            "<![CDATA[select * from trades where price < 100 & size > 0]]>",
            "select * from trades where price < 100 & size > 0",
        ),
        (
            "select &apos;é&apos; &#x2192; &quot;x&quot; &amp; y",
            "select 'é' → \"x\" & y",
        ),
        ("  select 1\n", "  select 1\n"),
        ("", ""),
    ] {
        let request = read(&envelope(
            "",
            &execute_body(&format!(
                "<Command><Statement>{spelled}</Statement></Command>"
            )),
        ));
        assert_eq!(
            request.execute().expect("an Execute").command().statement(),
            Some(expected),
            "{spelled:?}"
        );
    }

    let request = read(&envelope(
        "",
        &execute_body("<Command><Statement/></Command>"),
    ));
    assert_eq!(
        request.execute().expect("an Execute").command().statement(),
        Some(""),
        "a self-closed statement is the empty one"
    );
}

#[test]
fn an_execute_command_holding_another_element_reads_as_that_element() {
    let request = read(&envelope("", &execute_body("<Command><Cancel/></Command>")));
    let execute = request.execute().expect("an Execute");
    assert_eq!(
        execute,
        &Execute::new(Command::Other(Fragment::new("Cancel", Scalar::Null)))
    );
    assert_eq!(execute.command().name(), "Cancel");
    assert_eq!(execute.command().statement(), None);

    let request = read(&envelope(
        "",
        &execute_body(
            "<Command><Batch xmlns=\"http://schemas.microsoft.com/analysisservices/2003/engine\">\
             <Process/></Batch></Command>",
        ),
    ));
    let Command::Other(batch) = request.execute().expect("an Execute").command() else {
        panic!("expected another command");
    };
    assert_eq!(batch.name(), "Batch");
    assert_eq!(batch.element().namespace(), Some(ENGINE));
    assert_eq!(batch.element().children().count(), 1);

    let request = read(&envelope(
        "",
        &execute_body(
            "<Command><Statement xmlns=\"urn:example:other\">select 1</Statement></Command>",
        ),
    ));
    let command = request.execute().expect("an Execute").command();
    assert_eq!(
        command.statement(),
        None,
        "a Statement in another namespace is another command"
    );
    assert_eq!(command.name(), "Statement");
}

#[test]
fn an_execute_without_exactly_one_command_element_is_refused() {
    let error = refused(&envelope("", &execute_body("<Properties/>")));
    assert_eq!(
        codec_reason(&error, "xml"),
        "expected one `Command` element under `Execute`, found none"
    );

    let error = refused(&envelope(
        "",
        &execute_body(
            "<Command><Statement>a</Statement></Command><Command><Statement>b</Statement></Command>",
        ),
    ));
    assert_eq!(
        codec_reason(&error, "xml"),
        "expected one `Command` element under `Execute`, found several"
    );

    for command in [
        "<Command/>",
        "<Command></Command>",
        "<Command>  </Command>",
        "<Command>select 1</Command>",
    ] {
        let error = refused(&envelope("", &execute_body(command)));
        assert_eq!(
            xmla_reason(&error),
            "the Execute command holds no element",
            "{command}"
        );
    }

    for command in [
        "<Command><Statement>a</Statement><Statement>b</Statement></Command>",
        "<Command><Statement>a</Statement><Cancel/></Command>",
    ] {
        let error = refused(&envelope("", &execute_body(command)));
        assert_eq!(
            xmla_reason(&error),
            "the Execute command holds several elements where one is expected",
            "{command}"
        );
        assert_eq!(
            error.to_string(),
            "invalid record value at $.xmla: \
             the Execute command holds several elements where one is expected"
        );
    }
}

#[test]
fn a_parameter_without_a_name_is_refused() {
    for parameter in [
        "<Parameter><Value>1</Value></Parameter>",
        "<Parameter><Name/><Value>1</Value></Parameter>",
        "<Parameter><Name xmlns=\"urn:example:other\">a</Name><Value>1</Value></Parameter>",
        "<Parameter><Name>a</Name><Value>1</Value></Parameter><Parameter><Value>2</Value></Parameter>",
    ] {
        let error = refused(&envelope(
            "",
            &execute_body(&format!(
                "<Command><Statement>select 1</Statement></Command><Parameters>{parameter}</Parameters>"
            )),
        ));
        assert_eq!(
            xmla_reason(&error),
            "a Parameter without a Name",
            "{parameter}"
        );
    }
}

#[test]
fn envelopes_under_other_prefixes_read_as_the_same_request() {
    assert_eq!(read(DISCOVER_TABLES_PREFIXED), read(DISCOVER_TABLES));
    assert_eq!(
        read(DISCOVER_TABLES_PREFIXED).discover(),
        Some(&tables_discover())
    );

    assert_eq!(read(EXECUTE_TRADES_PREFIXED), read(EXECUTE_TRADES));
    assert_eq!(
        read(EXECUTE_TRADES_PREFIXED).execute(),
        Some(&trades_execute())
    );
}

#[test]
fn an_unqualified_payload_reads_as_the_xmla_method() {
    let request = read(&envelope(
        "",
        "<Discover><RequestType>DISCOVER_DATASOURCES</RequestType></Discover>",
    ));
    assert_eq!(
        request.discover(),
        Some(&Discover::new(RequestType::DiscoverDatasources))
    );

    let request = read(&envelope(
        "",
        "<Execute><Command><Statement>select 1</Statement></Command></Execute>",
    ));
    assert_eq!(request.execute(), Some(&Execute::statement("select 1")));
}

#[test]
fn a_payload_that_is_neither_discover_nor_execute_is_refused_naming_it() {
    for (payload, got) in [
        (
            "<Cancel xmlns=\"urn:schemas-microsoft-com:xml-analysis\"/>",
            "`Cancel` in \"urn:schemas-microsoft-com:xml-analysis\"",
        ),
        (
            "<DiscoverResponse xmlns=\"urn:schemas-microsoft-com:xml-analysis\"><return/></DiscoverResponse>",
            "`DiscoverResponse` in \"urn:schemas-microsoft-com:xml-analysis\"",
        ),
        (
            "<discover xmlns=\"urn:schemas-microsoft-com:xml-analysis\"/>",
            "`discover` in \"urn:schemas-microsoft-com:xml-analysis\"",
        ),
        ("<Frobnicate/>", "`Frobnicate` in no namespace"),
    ] {
        let error = refused(&envelope("", payload));
        let reason = format!(
            "expected `Discover` or `Execute` in \"urn:schemas-microsoft-com:xml-analysis\", got {got}"
        );
        assert_eq!(xmla_reason(&error), reason, "{payload}");
        assert_eq!(
            error.to_string(),
            format!("invalid record value at $.xmla: {reason}")
        );
    }
}

#[test]
fn a_method_in_another_namespace_is_refused_naming_both_namespaces() {
    for (payload, got) in [
        (
            "<Discover xmlns=\"urn:example:other\"><RequestType>DBSCHEMA_TABLES</RequestType></Discover>",
            "`Discover` in \"urn:example:other\"",
        ),
        (
            "<o:Execute xmlns:o=\"urn:example:other\"><o:Command><o:Statement>select 1</o:Statement></o:Command></o:Execute>",
            "`o:Execute` in \"urn:example:other\"",
        ),
        (
            "<Discover xmlns=\"urn:schemas-microsoft-com:xml-analysis:rowset\"><RequestType>X</RequestType></Discover>",
            "`Discover` in \"urn:schemas-microsoft-com:xml-analysis:rowset\"",
        ),
    ] {
        let error = refused(&envelope("", payload));
        assert_eq!(
            xmla_reason(&error),
            format!(
                "expected `Discover` or `Execute` in \"urn:schemas-microsoft-com:xml-analysis\", got {got}"
            ),
            "{payload}"
        );
    }
}

#[test]
fn a_fault_body_is_refused_naming_the_fault() {
    let error = refused(&envelope(
        "",
        "<SOAP-ENV:Fault><faultcode>SOAP-ENV:Client</faultcode>\
         <faultstring>bad request</faultstring></SOAP-ENV:Fault>",
    ));
    assert_eq!(
        xmla_reason(&error),
        "expected a Discover or Execute, got a SOAP fault SOAP-ENV:Client: bad request"
    );

    let fault = Envelope::from_fault(Fault::server("the catalog is busy").with_subcode("Busy"));
    let error = Request::from_envelope(&fault).expect_err("a fault is no request");
    assert_eq!(
        xmla_reason(&error),
        "expected a Discover or Execute, got a SOAP fault SOAP-ENV:Server.Busy: the catalog is busy"
    );
}

#[test]
fn a_message_the_envelope_refuses_is_refused_with_the_envelope_reason() {
    let error = refused(&discover_body("<RequestType>DBSCHEMA_TABLES</RequestType>"));
    assert_eq!(
        codec_reason(&error, "soap"),
        "expected the SOAP 1.1 `Envelope` in \"http://schemas.xmlsoap.org/soap/envelope/\", \
         got `Discover` in \"urn:schemas-microsoft-com:xml-analysis\""
    );

    let error = refused(&envelope("", ""));
    assert_eq!(
        codec_reason(&error, "soap"),
        "the SOAP body holds no element"
    );

    let error = refused(&envelope(
        "",
        &format!(
            "{}{}",
            discover_body("<RequestType>DBSCHEMA_TABLES</RequestType>"),
            execute_body("<Command><Statement>select 1</Statement></Command>")
        ),
    ));
    assert_eq!(
        codec_reason(&error, "soap"),
        "the SOAP body holds several elements where one method call is expected"
    );

    let error = refused("not a message");
    codec_reason(&error, "xml");

    let error = refused(&envelope("", "<Discover>").replace("</SOAP-ENV:Body>", ""));
    codec_reason(&error, "xml");
}

#[test]
fn a_header_with_begin_session_reads_as_a_session_request() {
    let request = read(&envelope(
        "<BeginSession xmlns=\"urn:schemas-microsoft-com:xml-analysis\" SOAP-ENV:mustUnderstand=\"1\"/>",
        &execute_body(
            "<Command><Statement>select 1</Statement></Command>\
             <Properties><PropertyList><Catalog>market</Catalog></PropertyList></Properties>",
        ),
    ));
    assert_eq!(
        request.session().expect("one session"),
        Some(Session::Begin)
    );
    assert_eq!(request.header().len(), 1);
    let block = request.header()[0].element();
    assert_eq!(block.name(), "BeginSession");
    assert_eq!(block.namespace(), Some(NAMESPACE));
    assert_eq!(
        block.attribute_in(Some(ENVELOPE_NAMESPACE), "mustUnderstand"),
        Some("1"),
        "the envelope's prefix resolves in the block read out of it"
    );
    assert_eq!(
        request.execute(),
        Some(
            &Execute::statement("select 1")
                .with_properties(PropertyList::new().with("Catalog", "market"))
        )
    );

    for (block, session) in [
        (
            "<Session xmlns=\"urn:schemas-microsoft-com:xml-analysis\" SessionId=\"581\"/>",
            Session::Continue("581".into()),
        ),
        (
            "<EndSession xmlns=\"urn:schemas-microsoft-com:xml-analysis\" SessionId=\"581\"/>",
            Session::End("581".into()),
        ),
        (
            "<Security xmlns=\"urn:example:security\"/>\
             <Session xmlns=\"urn:schemas-microsoft-com:xml-analysis\" SessionId=\"581\"/>",
            Session::Continue("581".into()),
        ),
    ] {
        let request = read(&envelope(
            block,
            &discover_body("<RequestType>DISCOVER_DATASOURCES</RequestType>"),
        ));
        assert_eq!(
            request.session().expect("one session"),
            Some(session),
            "{block}"
        );
    }
}

#[test]
fn a_session_header_is_refused_where_it_is_read_not_where_the_request_is() {
    let request = read(&envelope(
        "<Session xmlns=\"urn:schemas-microsoft-com:xml-analysis\"/>",
        &discover_body("<RequestType>DISCOVER_DATASOURCES</RequestType>"),
    ));
    assert_eq!(
        request.discover(),
        Some(&Discover::new(RequestType::DiscoverDatasources)),
        "the method reads whatever its header holds"
    );
    assert_eq!(
        xmla_reason(&request.session().expect_err("no identifier")),
        "the Session header names no SessionId"
    );

    let request = read(&envelope(
        "<BeginSession xmlns=\"urn:schemas-microsoft-com:xml-analysis\"/>\
         <EndSession xmlns=\"urn:schemas-microsoft-com:xml-analysis\" SessionId=\"581\"/>",
        &discover_body("<RequestType>DISCOVER_DATASOURCES</RequestType>"),
    ));
    assert_eq!(
        xmla_reason(&request.session().expect_err("two session blocks")),
        "a request carries more than one session header"
    );
}

#[test]
fn a_header_under_a_prefix_declared_on_the_envelope_reads_its_session() {
    let request = read(
        "<soap:Envelope xmlns:soap=\"http://schemas.xmlsoap.org/soap/envelope/\" \
         xmlns:xmla=\"urn:schemas-microsoft-com:xml-analysis\">\
         <soap:Header><xmla:Session soap:mustUnderstand=\"1\" SessionId=\"581\"/></soap:Header>\
         <soap:Body><xmla:Discover><xmla:RequestType>DISCOVER_DATASOURCES</xmla:RequestType>\
         </xmla:Discover></soap:Body></soap:Envelope>",
    );
    assert_eq!(
        request.session().expect("one session"),
        Some(Session::Continue("581".into()))
    );
    assert_eq!(request.header()[0].name(), "xmla:Session");
    assert_eq!(
        request.discover(),
        Some(&Discover::new(RequestType::DiscoverDatasources))
    );
}

#[test]
fn from_envelope_reads_an_envelope_built_in_memory() {
    let payload = Fragment::in_namespace(
        "Discover",
        NAMESPACE,
        Scalar::from_struct([("RequestType", Scalar::from("DBSCHEMA_CATALOGS"))])
            .expect("a record"),
    )
    .expect("a namespaced payload");
    let begin = Session::Begin.into_fragment().expect("a header block");
    let envelope = Envelope::new(Body::Payload(payload)).with_header(begin);
    let request = Request::from_envelope(&envelope).expect("a request");
    assert_eq!(
        request.discover(),
        Some(&Discover::new(RequestType::DbschemaCatalogs))
    );
    assert_eq!(request.header(), envelope.header());
    assert_eq!(
        request.session().expect("one session"),
        Some(Session::Begin)
    );
}

// Writing ----------------------------------------------------------------

#[test]
fn a_discover_is_written_as_the_xmla_discover_element_in_its_namespace() {
    let request = Request::from(
        Discover::new(RequestType::DbschemaTables)
            .with_restrictions(
                Restrictions::new()
                    .with("TABLE_CATALOG", "market")
                    .with("TABLE_TYPE", "TABLE")
                    .with("TABLE_TYPE", "VIEW"),
            )
            .with_properties(PropertyList::new().with("Format", "Tabular")),
    );
    let text = written(&request);
    assert!(
        text.starts_with(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
             <SOAP-ENV:Envelope xmlns:SOAP-ENV=\"http://schemas.xmlsoap.org/soap/envelope/\">"
        ),
        "{text}"
    );
    assert!(
        !text.contains("Header"),
        "no header block, no Header: {text}"
    );
    for expected in [
        "<SOAP-ENV:Body><Discover xmlns=\"urn:schemas-microsoft-com:xml-analysis\">",
        "<RequestType>DBSCHEMA_TABLES</RequestType>",
        "<Restrictions><RestrictionList><TABLE_CATALOG>market</TABLE_CATALOG>\
         <TABLE_TYPE>TABLE</TABLE_TYPE><TABLE_TYPE>VIEW</TABLE_TYPE></RestrictionList></Restrictions>",
        "<Properties><PropertyList><Format>Tabular</Format></PropertyList></Properties>",
        "</Discover></SOAP-ENV:Body></SOAP-ENV:Envelope>",
    ] {
        assert!(text.contains(expected), "{expected} in {text}");
    }

    let envelope = request.into_envelope().expect("an envelope");
    assert!(envelope.header().is_empty());
    let payload = envelope.payload().expect("a payload");
    assert_eq!(payload.name(), Method::Discover.as_str());
    assert_eq!(payload.element().namespace(), Some(NAMESPACE));
    // The natural envelope is a record, sorted by name; the bytes keep the
    // order the XMLA schema declares, and read back as the same request.
    let position = |needle: &str| text.find(needle).unwrap_or_else(|| panic!("{needle} in {text}"));
    assert!(
        position("<RequestType>") < position("<Restrictions>")
            && position("<Restrictions>") < position("<Properties>"),
        "{text}"
    );
    assert_eq!(
        Request::from_bytes(&request.into_bytes().expect("written")).expect("read back"),
        request
    );

    let text = written(&Request::from(Discover::new(
        RequestType::DiscoverDatasources,
    )));
    assert!(
        text.contains("<Restrictions><RestrictionList/></Restrictions>")
            && text.contains("<Properties><PropertyList/></Properties>"),
        "the arguments are written even when empty: {text}"
    );
}

#[test]
fn an_execute_is_written_with_its_command_properties_and_parameters() {
    let request = Request::from(
        Execute::statement("select symbol from trades")
            .with_properties(PropertyList::new().with("Catalog", "market"))
            .with_parameter("symbol", Scalar::from("AAPL"))
            .with_parameter("limit", Scalar::from("10")),
    );
    let text = written(&request);
    for expected in [
        "<SOAP-ENV:Body><Execute xmlns=\"urn:schemas-microsoft-com:xml-analysis\">",
        "<Command><Statement>select symbol from trades</Statement></Command>",
        "<Properties><PropertyList><Catalog>market</Catalog></PropertyList></Properties>",
        "<Parameters><Parameter><Name>symbol</Name><Value>AAPL</Value></Parameter>\
         <Parameter><Name>limit</Name><Value>10</Value></Parameter></Parameters>",
        "</Execute></SOAP-ENV:Body></SOAP-ENV:Envelope>",
    ] {
        assert!(text.contains(expected), "{expected} in {text}");
    }
    let envelope = request.into_envelope().expect("an envelope");
    let payload = envelope.payload().expect("a payload");
    assert_eq!(payload.name(), Method::Execute.as_str());
    assert_eq!(payload.element().namespace(), Some(NAMESPACE));

    let text = written(&Request::from(Execute::statement("select 1")));
    assert!(
        !text.contains("Parameters"),
        "no parameter, no Parameters: {text}"
    );

    let text = written(&Request::from(Execute::new(Command::Other(Fragment::new(
        "Cancel",
        Scalar::Null,
    )))));
    assert!(text.contains("<Command><Cancel/></Command>"), "{text}");
}

#[test]
fn a_session_header_is_written_before_the_body() {
    let request = Request::from(Discover::new(RequestType::DiscoverDatasources))
        .with_session(&Session::Continue("581".into()))
        .expect("a session header");
    let text = written(&request);
    let header = text.find("<SOAP-ENV:Header>").expect("a Header");
    let block = text.find("<Session ").expect("the Session block");
    let body = text.find("<SOAP-ENV:Body>").expect("a Body");
    assert!(header < block && block < body, "{text}");
    for expected in [
        "SessionId=\"581\"",
        " mustUnderstand=\"1\"",
        "xmlns=\"urn:schemas-microsoft-com:xml-analysis\"",
    ] {
        assert!(text[block..body].contains(expected), "{expected} in {text}");
    }
    assert_eq!(
        request.into_envelope().expect("an envelope").header(),
        request.header()
    );
}

#[test]
fn a_discover_round_trips_through_its_bytes() {
    let discover = Discover::new(RequestType::DbschemaColumns)
        .with_restrictions(
            Restrictions::new()
                .with("COLUMN_NAME", "prix €")
                .with("TABLE_CATALOG", "marché <&> \"q\"")
                .with("TABLE_NAME", "")
                .with("TABLE_NAME", "trades"),
        )
        .with_properties(
            PropertyList::new()
                .with("Catalog", "marché")
                .with("Content", "SchemaData")
                .with("Format", "Tabular"),
        );
    let request = Request::from(discover.clone());
    assert_eq!(round_trip(&request), request);
    assert_eq!(round_trip(&request).discover(), Some(&discover));

    for request_type in [
        RequestType::DiscoverDatasources,
        RequestType::MdschemaCubes,
        RequestType::Other("MY_ROWSET".into()),
    ] {
        let request = Request::from(Discover::new(request_type));
        assert_eq!(round_trip(&request), request);
    }
}

#[test]
fn an_execute_with_parameters_round_trips_through_its_bytes() {
    let request = Request::from(trades_execute());
    assert_eq!(round_trip(&request), request);

    let execute = Execute::statement("select * from t where a < 1 & b > \"x\" and c = 'é' →")
        .with_properties(
            PropertyList::new()
                .with("Catalog", "market")
                .with("Timeout", "30"),
        )
        .with_parameter("symbol", Scalar::from("AAPL"))
        .with_parameter("limit", Scalar::from("10"))
        .with_parameter("absent", Scalar::Null)
        .with_parameter("empty", Scalar::from(""));
    let request = Request::from(execute.clone());
    assert_eq!(round_trip(&request), request);
    assert_eq!(
        parameters(round_trip(&request).execute().expect("an Execute")),
        parameters(&execute),
        "parameters keep the order written, not their names' order"
    );

    let cancel = Request::from(Execute::new(Command::Other(Fragment::new(
        "Cancel",
        Scalar::Null,
    ))));
    assert_eq!(round_trip(&cancel), cancel);
}

#[test]
fn a_typed_parameter_is_written_as_its_text_and_reads_back_as_text() {
    let request = Request::from(
        Execute::statement("select * from trades limit @limit")
            .with_parameter("limit", Scalar::from(10_i64)),
    );
    let text = written(&request);
    assert!(
        text.contains("<Parameter><Name>limit</Name><Value>10</Value></Parameter>"),
        "{text}"
    );
    assert_eq!(
        parameters(round_trip(&request).execute().expect("an Execute")),
        [("limit", Scalar::from("10"))],
        "the wire carries text, and text is what reads back"
    );
}

#[test]
fn a_request_with_a_session_header_round_trips_through_its_bytes() {
    for session in [
        Session::Begin,
        Session::Continue("581".into()),
        Session::End("581".into()),
    ] {
        let request = Request::from(trades_execute())
            .with_session(&session)
            .expect("a session header");
        assert_eq!(round_trip(&request), request, "{session:?}");
    }
}

#[test]
fn the_session_header_and_the_method_survive_a_round_trip() {
    let request = Request::from(trades_execute())
        .with_session(&Session::Continue("581".into()))
        .expect("a session header");
    let read = round_trip(&request);
    assert_eq!(read.method(), request.method());
    assert_eq!(
        read.session().expect("one session"),
        Some(Session::Continue("581".into()))
    );
    assert_eq!(read.header().len(), 1);
    assert_eq!(read.header()[0].name(), request.header()[0].name());
    assert_eq!(read.header()[0].value(), request.header()[0].value());
}

#[test]
fn a_written_discover_orders_its_arguments_as_the_xmla_schema_declares() {
    // XMLA 1.1's Discover is a sequence: RequestType, Restrictions, Properties.
    let text = written(&Request::from(tables_discover()));
    let at = |tag: &str| text.find(tag).unwrap_or_else(|| panic!("{tag} in {text}"));
    assert!(
        at("<RequestType>") < at("<Restrictions>") && at("<Restrictions>") < at("<Properties>"),
        "{text}"
    );
}

#[test]
fn a_command_under_a_prefix_declared_above_it_keeps_that_namespace() {
    let request = read(
        "<soap:Envelope xmlns:soap=\"http://schemas.xmlsoap.org/soap/envelope/\" \
         xmlns:xmla=\"urn:schemas-microsoft-com:xml-analysis\" \
         xmlns:as=\"http://schemas.microsoft.com/analysisservices/2003/engine\">\
         <soap:Body><xmla:Execute><xmla:Command><as:Batch><as:Process/></as:Batch>\
         </xmla:Command></xmla:Execute></soap:Body></soap:Envelope>",
    );
    let Command::Other(batch) = request.execute().expect("an Execute").command() else {
        panic!("expected another command");
    };
    assert_eq!(batch.name(), "as:Batch");
    assert_eq!(
        batch.element().namespace(),
        Some(ENGINE),
        "the prefix declared on the envelope still names the command's namespace"
    );

    let again = round_trip(&request);
    let Command::Other(batch) = again.execute().expect("an Execute").command() else {
        panic!("expected another command");
    };
    assert_eq!(
        batch.element().namespace(),
        Some(ENGINE),
        "written back, the prefix is still declared: {}",
        written(&request)
    );
}

#[test]
fn zz_probe() {
    let show = |label: &str, message: &str| match Request::from_bytes(message.as_bytes()) {
        Ok(request) => println!("{label}: OK {:?}", request.method()),
        Err(error) => println!("{label}: ERR {error:?}"),
    };
    // 1. unqualified command inside a prefixed Execute
    let request = read(
        "<soap:Envelope xmlns:soap=\"http://schemas.xmlsoap.org/soap/envelope/\">\
         <soap:Body><xmla:Execute xmlns:xmla=\"urn:schemas-microsoft-com:xml-analysis\">\
         <xmla:Command><Cancel/></xmla:Command></xmla:Execute></soap:Body></soap:Envelope>",
    );
    let Command::Other(cancel) = request.execute().unwrap().command() else { panic!() };
    println!("1 read ns {:?}", cancel.element().namespace());
    let again = round_trip(&request);
    let Command::Other(cancel2) = again.execute().unwrap().command() else { panic!() };
    println!("1 again ns {:?} eq {}", cancel2.element().namespace(), again == request);
    println!("1 written {}", written(&request));
    // 2. prefixed xmla:Cancel in prefixed envelope
    let request = read(
        "<soap:Envelope xmlns:soap=\"http://schemas.xmlsoap.org/soap/envelope/\" xmlns:xmla=\"urn:schemas-microsoft-com:xml-analysis\">\
         <soap:Body><xmla:Execute>\
         <xmla:Command><xmla:Cancel/></xmla:Command></xmla:Execute></soap:Body></soap:Envelope>",
    );
    let again = round_trip(&request);
    println!("2 eq {} written {}", again == request, written(&request));
    println!("2 {:?}\n2 {:?}", request.execute().unwrap().command(), again.execute().unwrap().command());
    // 3. as:Batch into_envelope
    let request = read(
        "<soap:Envelope xmlns:soap=\"http://schemas.xmlsoap.org/soap/envelope/\" \
         xmlns:xmla=\"urn:schemas-microsoft-com:xml-analysis\" \
         xmlns:as=\"http://schemas.microsoft.com/analysisservices/2003/engine\">\
         <soap:Body><xmla:Execute><xmla:Command><as:Batch><as:Process/></as:Batch>\
         </xmla:Command></xmla:Execute></soap:Body></soap:Envelope>",
    );
    let natural = request.into_envelope().unwrap();
    let from = Request::from_envelope(&natural).unwrap();
    let Command::Other(batch) = from.execute().unwrap().command() else { panic!() };
    println!("3 from_envelope ns {:?} eq {}", batch.element().namespace(), from == request);
    match natural.into_bytes() {
        Ok(bytes) => { println!("3 env bytes {}", String::from_utf8_lossy(&bytes)); show("3 reread", &String::from_utf8_lossy(&bytes)); }
        Err(e) => println!("3 env bytes ERR {e:?}"),
    }
    // 4. whitespace-only
    for (label, children) in [
        ("4 stmt ws", "<Command><Statement>   </Statement></Command>"),
        ("4 value ws", "<Command><Statement>s</Statement></Command><Parameters><Parameter><Name>a</Name><Value>   </Value></Parameter></Parameters>"),
        ("4 name ws", "<Command><Statement>s</Statement></Command><Parameters><Parameter><Name>  </Name><Value>1</Value></Parameter></Parameters>"),
        ("4 prop ws", "<Command><Statement>s</Statement></Command><Properties><PropertyList><Catalog>  </Catalog><Format> Tabular </Format></PropertyList></Properties>"),
        ("4 dup prop", "<Command><Statement>s</Statement></Command><Properties><PropertyList><format>Multidimensional</format><Format>Tabular</Format></PropertyList></Properties>"),
        ("4 dup prop same", "<Command><Statement>s</Statement></Command><Properties><PropertyList><Format>Tabular</Format><Format>Multidimensional</Format></PropertyList></Properties>"),
        ("4 two props", "<Command><Statement>s</Statement></Command><Properties><PropertyList><Format>Tabular</Format></PropertyList></Properties><Properties><PropertyList><Catalog>x</Catalog></PropertyList></Properties>"),
        ("4 two params", "<Command><Statement>s</Statement></Command><Parameters><Parameter><Name>a</Name><Value>1</Value></Parameter></Parameters><Parameters><Parameter><Name>b</Name><Value>2</Value></Parameter></Parameters>"),
        ("4 two names", "<Command><Statement>s</Statement></Command><Parameters><Parameter><Name>a</Name><Name>b</Name><Value>1</Value></Parameter></Parameters>"),
        ("4 two values", "<Command><Statement>s</Statement></Command><Parameters><Parameter><Name>a</Name><Value>1</Value><Value>2</Value></Parameter></Parameters>"),
        ("4 xsi type", "<Command><Statement>s</Statement></Command><Parameters><Parameter><Name>a</Name><Value xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:type=\"xsd:int\">10</Value></Parameter></Parameters>"),
        ("4 xsi nil", "<Command><Statement>s</Statement></Command><Parameters><Parameter><Name>a</Name><Value xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:nil=\"true\"/></Parameter></Parameters>"),
        ("4 foreign param", "<Command><Statement>s</Statement></Command><Parameters><Parameter xmlns=\"urn:other\"><Name>a</Name><Value>1</Value></Parameter></Parameters>"),
        ("4 stmt attr", "<Command><Statement xml:space=\"preserve\">select 1</Statement></Command>"),
        ("4 stmt child", "<Command><Statement><b>x</b></Statement></Command>"),
        ("4 stmt mixed", "<Command><Statement>select <b/> 1</Statement></Command>"),
        ("4 foreign command", "<o:Command xmlns:o=\"urn:other\"><Statement>s</Statement></o:Command>"),
        ("4 foreign props", "<Command><Statement>s</Statement></Command><Properties xmlns=\"urn:other\"><PropertyList><Format>Tabular</Format></PropertyList></Properties>"),
    ] {
        show(label, &envelope("", &execute_body(children)));
    }
    for (label, children) in [
        ("5 value children", "<RequestType>DBSCHEMA_TABLES</RequestType><Restrictions><RestrictionList><TABLE_TYPE><Value>TABLE</Value><Value>VIEW</Value></TABLE_TYPE></RestrictionList></Restrictions>"),
        ("5 value one", "<RequestType>DBSCHEMA_TABLES</RequestType><Restrictions><RestrictionList><TABLE_TYPE><Value>TABLE</Value></TABLE_TYPE><TABLE_TYPE>VIEW</TABLE_TYPE></RestrictionList></Restrictions>"),
        ("5 value empty", "<RequestType>DBSCHEMA_TABLES</RequestType><Restrictions><RestrictionList><TABLE_TYPE><Value/><x:Value xmlns:x=\"urn:o\">V</x:Value></TABLE_TYPE></RestrictionList></Restrictions>"),
        ("5 other child", "<RequestType>DBSCHEMA_TABLES</RequestType><Restrictions><RestrictionList><TABLE_TYPE><Other>x</Other></TABLE_TYPE></RestrictionList></Restrictions>"),
        ("5 no list", "<RequestType>DBSCHEMA_TABLES</RequestType><Restrictions><CATALOG_NAME>x</CATALOG_NAME></Restrictions>"),
        ("5 two restrictions", "<RequestType>DBSCHEMA_TABLES</RequestType><Restrictions><RestrictionList><A>1</A></RestrictionList></Restrictions><Restrictions><RestrictionList><B>2</B></RestrictionList></Restrictions>"),
        ("5 ws restriction", "<RequestType>DBSCHEMA_TABLES</RequestType><Restrictions><RestrictionList><A>  </A><B> b </B></RestrictionList></Restrictions>"),
        ("5 prop empty", "<RequestType>DBSCHEMA_TABLES</RequestType><Properties><PropertyList><Catalog/></PropertyList></Properties>"),
    ] {
        show(label, &envelope("", &discover_body(children)));
    }
    // 6 writer errors
    for request in [
        Request::from(Execute::statement("select \u{0}")),
        Request::from(Execute::statement("s").with_properties(PropertyList::new().with("bad name", "x"))),
        Request::from(Execute::statement("s").with_properties(PropertyList::new().with("@xmlns", "urn:evil"))),
        Request::from(Execute::statement("s").with_properties(PropertyList::new().with("#text", "x"))),
        Request::from(Execute::statement("s").with_properties(PropertyList::new().with("Catalog", "\u{1}"))),
        Request::from(Discover::new(RequestType::DbschemaTables).with_restrictions(Restrictions::new().with("1st", "x"))),
        Request::from(Discover::new(RequestType::DbschemaTables).with_restrictions(Restrictions::new().with("A", "\u{b}"))),
        Request::from(Discover::new(RequestType::Other("\u{1}".into()))),
        Request::from(Execute::statement("s").with_parameter("a\u{0}", Scalar::from("1"))),
        Request::from(Execute::statement("s").with_parameter("a", Scalar::from_sequence([Scalar::from("x"), Scalar::from("y")]))),
        Request::from(Execute::statement("s").with_parameter("a", Scalar::from("\u{2}"))),
        Request::from(Execute::statement("s")).with_header(Fragment::new("bad name", Scalar::Null)),
        Request::from(Execute::statement("s")).with_header(Fragment::new("Security", Scalar::Null)).with_header(Fragment::new("Security", Scalar::Null)),
        Request::from(Execute::new(Command::Other(Fragment::new("bad name", Scalar::Null)))),
    ] {
        let bytes = request.into_bytes();
        let env = request.into_envelope();
        let env_bytes = env.as_ref().map(|e| e.into_bytes());
        println!("6 {:?}\n   bytes {:?}\n   env {:?}\n   env_bytes {:?}", request.method(), bytes.map(|b| String::from_utf8_lossy(&b).into_owned()), env.as_ref().map(|_| ()), env_bytes.map(|r| r.map(|b| String::from_utf8_lossy(&b).into_owned())));
    }
    // 7 session answer
    let answer = Session::Continue("581".into()).into_answer_fragment().unwrap();
    println!("7 {:?}", answer);
    let bytes = Request::from(Execute::statement("s")).with_header(answer).into_bytes().unwrap();
    println!("7 {}", String::from_utf8_lossy(&bytes));
    let s = Request::from(Execute::statement("s")).with_session(&Session::Continue("sé \"7\" <&> '→'\t\n".into())).unwrap();
    println!("8 {}", written(&s));
    println!("8 {:?}", round_trip(&s).session());
    let d = Request::from(Discover::new(RequestType::Other("A&B<c>".into())).with_restrictions(Restrictions::new().with("Z", "q\"'<&>").with("A", "1")).with_properties(PropertyList::new().with("Format", "x").with("Catalog", "c")));
    println!("9 {}", written(&d));
    let e = d.into_envelope().unwrap();
    println!("9 natural children {:?}", e.payload().unwrap().element().children().map(|c| c.name().to_owned()).collect::<Vec<_>>());
    println!("9 from_envelope eq {}", Request::from_envelope(&e).unwrap() == d);
    let x = Request::from(trades_execute().with_parameter("n", Scalar::Null));
    let e = x.into_envelope().unwrap();
    println!("10 natural children {:?}", e.payload().unwrap().element().children().map(|c| c.name().to_owned()).collect::<Vec<_>>());
    println!("10 from_envelope eq {}", Request::from_envelope(&e).unwrap() == x);
    println!("10 env bytes eq {}", Request::from_bytes(&e.into_bytes().unwrap()).unwrap() == x);
    let ws = Request::from(Execute::statement("   ").with_parameter("w", Scalar::from("  ")).with_properties(PropertyList::new().with("Catalog", "  ")));
    println!("11 {:?}", round_trip(&ws).method());
}
