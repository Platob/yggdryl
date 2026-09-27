//! `rust/src/xmla/server.rs`: the routes [`Service::route`] answers on the
//! crate's HTTP [`Server`](yggdryl::http::Server) - connection framing,
//! keep-alive, `Expect: 100-continue`, chunked and traced bodies are the
//! server's, proven in `rust/tests/http.rs`; what is proven here is XMLA's
//! own: the `GET` description, the `POST` answer and its faults, the method
//! table, and a traced exchange replayed through [`Service::handle`].

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, StringArray};
use yggdryl::holder::Holder;
use yggdryl::http::{
    Method, Request as HttpRequest, Response as HttpResponse, Server, ServerOptions, Status,
};
use yggdryl::media::RecordOptions;
use yggdryl::soap::Envelope;
use yggdryl::xmla::{
    Catalog, Discover, Execute, PropertyList, Request, RequestType, Response, Service,
    ServiceOptions, XmlaError,
};
use yggdryl::{DataType, IOBase, IOMedia, MimeType, Result, Scalar, StructType, Url};

/// A fresh catalog folder, named after `label`, holding `trades` with two
/// rows.
fn catalog_root(label: &str) -> PathBuf {
    let mut root = yggdryl::local::LocalFolder::temporary()
        .expect("a temporary directory")
        .path()
        .expect("a path");
    root.push(format!(
        "yggdryl-xmla-server-{label}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("the folder is created");
    let field = StructType::from_fields([
        DataType::utf8().required_field("symbol"),
        DataType::Int64.required_field("size"),
    ])
    .map(DataType::from)
    .expect("a valid root")
    .required_field("row");
    let batch = RecordBatch::try_new(
        field.into_arrow_schema().expect("an Arrow schema"),
        vec![
            Arc::new(StringArray::from(vec!["AAPL", "MSFT"])),
            Arc::new(Int64Array::from(vec![100, 250])),
        ],
    )
    .expect("a batch");
    let mut leaf = Holder::folder(&root)
        .expect("the root holds")
        .child_by_path("trades.arrows")
        .expect("the child resolves");
    leaf.overwrite_arrow_reader(
        yggdryl::arrow::batch_reader(batch.schema(), [batch]),
        &RecordOptions::for_mime_type(&MimeType::ARROW_STREAM).expect("IPC options"),
    )
    .expect("the table is written");
    root
}

/// A service over a fresh `catalog_root(label)`, routed at `/xmla` on a
/// fresh loopback server; the endpoint [`Service::route`] answered.
fn running(label: &str) -> (Server, Url, Arc<Service>) {
    running_with(label, ServerOptions::default(), ServiceOptions::new())
}

/// [`running`] with the server and the service options stated.
fn running_with(
    label: &str,
    options: ServerOptions,
    service_options: ServiceOptions,
) -> (Server, Url, Arc<Service>) {
    let root = catalog_root(label);
    let service = Arc::new(Service::new(service_options).with_catalog(Catalog::new(
        "market",
        Holder::folder(&root).expect("holds"),
    )));
    let server = Server::bind_with("127.0.0.1:0", options).expect("a loopback port");
    let endpoint = Arc::clone(&service)
        .route(&server, "/xmla")
        .expect("the path routes");
    (server, endpoint, service)
}

/// One raw HTTP/1.1 exchange on a fresh connection to `server`: the
/// request written whole, the answer read whole.
fn exchange(server: &Server, request: &[u8]) -> (u16, Vec<(String, String)>, Vec<u8>) {
    let mut stream = TcpStream::connect(server.address()).expect("the server accepts");
    stream.write_all(request).expect("the request is sent");
    stream.flush().expect("flushed");
    let mut reader = BufReader::new(stream);
    receive(&mut reader)
}

/// The `URL` `DISCOVER_DATASOURCES` states when posted to `endpoint`.
fn data_source_url(endpoint: &Url) -> Scalar {
    let discover = Request::from(Discover::new(RequestType::DiscoverDatasources))
        .into_bytes()
        .expect("a request");
    let response = post(
        endpoint,
        discover,
        "urn:schemas-microsoft-com:xml-analysis:Discover",
    )
    .expect("the server answers");
    assert_eq!(response.status(), Status::OK);
    let body = response.bytes().expect("a body");
    let discovered = Response::from_bytes(&body, None).expect("a response");
    let rows = discovered.rows().expect("a rowset");
    assert_eq!(rows.len(), 1);
    rows.child("URL")
        .expect("a URL column")
        .scalar(0)
        .expect("one row")
}

/// One SOAP `POST` of `body` to `endpoint` under `SOAPAction: "<action>"`.
fn post(
    endpoint: &Url,
    body: impl Into<yggdryl::http::Body>,
    action: &str,
) -> Result<HttpResponse> {
    HttpRequest::post(&endpoint.to_string(), body)?
        .with_header("content-type", "text/xml; charset=utf-8")?
        .with_header("SOAPAction", &format!("\"{action}\""))?
        .send()
}

/// One HTTP/1.1 response read off `reader`: the status, the headers and the
/// whole body, a chunked body reassembled - for the one exchange raw sockets
/// still have to drive, an `Expect: 100-continue` request.
fn receive<R: BufRead>(reader: &mut R) -> (u16, Vec<(String, String)>, Vec<u8>) {
    let mut line = String::new();
    reader.read_line(&mut line).expect("a status line");
    let status: u16 = line
        .split(' ')
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or_else(|| panic!("a status in {line:?}"));
    let mut headers = Vec::new();
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).expect("a header line");
        let header = header.trim_end_matches(['\r', '\n']);
        if header.is_empty() {
            break;
        }
        let (name, value) = header.split_once(':').expect("a header");
        headers.push((name.trim().to_ascii_lowercase(), value.trim().to_owned()));
    }
    let chunked = headers
        .iter()
        .any(|(name, value)| name == "transfer-encoding" && value == "chunked");
    let mut body = Vec::new();
    if chunked {
        loop {
            let mut size = String::new();
            reader.read_line(&mut size).expect("a chunk size");
            let size = usize::from_str_radix(size.trim(), 16).expect("hexadecimal");
            if size == 0 {
                let mut trailer = String::new();
                reader.read_line(&mut trailer).expect("the trailer");
                break;
            }
            let mut chunk = vec![0; size];
            reader.read_exact(&mut chunk).expect("the chunk");
            body.extend_from_slice(&chunk);
            let mut end = String::new();
            reader.read_line(&mut end).expect("the chunk end");
        }
    } else {
        let length: usize = headers
            .iter()
            .find(|(name, _)| name == "content-length")
            .map(|(_, value)| value.parse().expect("a length"))
            .expect("a Content-Length");
        body.resize(length, 0);
        reader.read_exact(&mut body).expect("the body");
    }
    (status, headers, body)
}

#[test]
fn route_answers_the_endpoint_url_and_refuses_a_path_with_a_query() {
    let root = catalog_root("route");
    let service = Arc::new(
        Service::new(ServiceOptions::new()).with_catalog(Catalog::new(
            "market",
            Holder::folder(&root).expect("holds"),
        )),
    );
    let server = Server::bind("127.0.0.1:0").expect("a loopback port");
    let endpoint = Arc::clone(&service)
        .route(&server, "/xmla")
        .expect("the path routes");
    assert_eq!(endpoint, server.url_of("/xmla").expect("the endpoint"));
    let refused = Arc::clone(&service).route(&server, "/xmla?x=1");
    assert!(refused.is_err(), "{refused:?}");
}

#[test]
fn a_discover_and_an_execute_answer_their_rows_under_the_soap_headers() {
    let (_server, endpoint, _service) = running("exchange");
    let discover = Request::from(Discover::new(RequestType::DbschemaTables))
        .into_bytes()
        .expect("a request");
    let response = post(
        &endpoint,
        discover,
        "urn:schemas-microsoft-com:xml-analysis:Discover",
    )
    .expect("the server answers");
    assert_eq!(response.status(), Status::OK);
    assert_eq!(
        response.headers().get("content-type"),
        Some(yggdryl::soap::CONTENT_TYPE)
    );
    assert_eq!(
        response.headers().get("x-transport-caps-negotiation-flags"),
        Some("0,0,0,0,0")
    );
    let body = response.bytes().expect("a body");
    let discovered = Response::from_bytes(&body, None).expect("a response");
    assert_eq!(discovered.rows().expect("a rowset").len(), 1);

    let execute = Request::from(
        Execute::statement("select symbol from trades where size > 200")
            .with_properties(PropertyList::new().with("Catalog", "market")),
    )
    .into_bytes()
    .expect("a request");
    let response = post(
        &endpoint,
        execute,
        "urn:schemas-microsoft-com:xml-analysis:Execute",
    )
    .expect("the server answers");
    assert_eq!(response.status(), Status::OK);
    let body = response.bytes().expect("a body");
    let executed = Response::from_bytes(&body, None).expect("a response");
    let rows = executed.rows().expect("a rowset");
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows.get(0).expect("a row").into_owned(),
        Scalar::from_sequence([Scalar::from("MSFT")])
    );
}

#[test]
fn a_get_describes_the_endpoint_with_no_negotiation_header() {
    let (_server, endpoint, _service) = running("get");
    let response = HttpRequest::get(&endpoint.to_string())
        .expect("a request builds")
        .send()
        .expect("the server answers");
    assert_eq!(response.status(), Status::OK);
    assert!(
        !response
            .headers()
            .contains_key("x-transport-caps-negotiation-flags")
    );
    let text = response.text().expect("a body");
    assert!(text.contains("XML for Analysis"), "{text}");
}

#[test]
fn a_head_answers_the_gets_head_with_no_body() {
    let (_server, endpoint, _service) = running("head");
    let response = HttpRequest::head(&endpoint.to_string())
        .expect("a request builds")
        .send()
        .expect("the server answers");
    assert_eq!(response.status(), Status::OK);
    assert!(response.bytes().expect("a body").is_empty());
}

#[test]
fn a_put_is_refused_by_the_methods_routed() {
    let (_server, endpoint, _service) = running("put");
    let response = HttpRequest::new(Method::Put, endpoint)
        .send()
        .expect("the server answers");
    assert_eq!(response.status(), Status::METHOD_NOT_ALLOWED);
    assert_eq!(response.headers().get("allow"), Some("GET, HEAD, POST"));
}

#[test]
fn another_path_answers_404() {
    let (server, _endpoint, _service) = running("elsewhere");
    let response = HttpRequest::get(&format!("http://{}/elsewhere", server.address()))
        .expect("a request builds")
        .send()
        .expect("the server answers");
    assert_eq!(response.status(), Status::NOT_FOUND);
}

#[test]
fn a_non_xml_content_type_is_a_client_fault_under_the_negotiation_header() {
    let (_server, endpoint, _service) = running("non-xml");
    let response = HttpRequest::post(&endpoint.to_string(), "<a/>")
        .expect("a request builds")
        .with_header("content-type", "text/plain")
        .expect("a header")
        .send()
        .expect("the server answers");
    assert_eq!(response.status(), Status::OK);
    assert_eq!(
        response.headers().get("x-transport-caps-negotiation-flags"),
        Some("0,0,0,0,0")
    );
    let body = response.bytes().expect("a body");
    let envelope = Envelope::from_bytes(&body).expect("an envelope");
    let fault = envelope.fault().expect("a fault");
    let errors = XmlaError::from_fault(fault);
    assert_eq!(errors.first().map(XmlaError::code), Some(1));
}

#[test]
fn an_empty_post_body_answers_a_fault() {
    let (_server, endpoint, _service) = running("empty");
    let response = HttpRequest::post(&endpoint.to_string(), Vec::<u8>::new())
        .expect("a request builds")
        .send()
        .expect("the server answers");
    assert_eq!(response.status(), Status::OK);
    let body = response.bytes().expect("a body");
    assert!(
        Envelope::from_bytes(&body)
            .expect("an envelope")
            .fault()
            .is_some()
    );
}

#[test]
fn an_adomdnet_shaped_request_gets_the_interim_continue_then_a_chunked_answer() {
    let (server, _endpoint, _service) = running("adomd");
    let address = server.address().to_string();
    let discover = Request::from(Discover::new(RequestType::DbschemaTables))
        .into_bytes()
        .expect("a request");
    // ADOMD.NET opens every body with the UTF-8 byte-order mark.
    let mut body = vec![0xEF, 0xBB, 0xBF];
    body.extend_from_slice(&discover);
    let mut stream = TcpStream::connect(&address).expect("the server accepts");
    let head = format!(
        "POST /xmla HTTP/1.1\r\nHost: {address}\r\nContent-Type: text/xml\r\n\
         Expect: 100-continue\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(head.as_bytes()).expect("the head is sent");
    stream.flush().expect("flushed");
    let mut reader = BufReader::new(stream.try_clone().expect("a read half"));
    let mut interim = String::new();
    reader.read_line(&mut interim).expect("the interim status");
    assert_eq!(interim, "HTTP/1.1 100 Continue\r\n");
    interim.clear();
    reader.read_line(&mut interim).expect("the blank line");
    assert_eq!(interim, "\r\n");
    for chunk in body.chunks(64) {
        stream
            .write_all(format!("{:x}\r\n", chunk.len()).as_bytes())
            .expect("a chunk size");
        stream.write_all(chunk).expect("a chunk");
        stream.write_all(b"\r\n").expect("the chunk end");
    }
    stream.write_all(b"0\r\n\r\n").expect("the last chunk");
    stream.flush().expect("flushed");
    let (status, headers, body) = receive(&mut reader);
    assert_eq!(status, 200);
    assert!(
        headers
            .iter()
            .any(|(name, value)| name == "transfer-encoding" && value == "chunked"),
        "{headers:?}"
    );
    let response = Response::from_bytes(&body, None).expect("a response");
    assert_eq!(response.rows().map(yggdryl::Serie::len), Some(1));
}

#[test]
fn behind_a_trusted_proxy_the_forwarded_endpoint_is_described_and_a_discover_answers() {
    let options = ServerOptions::default()
        .with_trusted_proxies(["127.0.0.1"])
        .expect("the loopback network")
        .with_path_prefix("/olap")
        .expect("a prefix");
    let (server, _endpoint, _service) = running_with("proxied", options, ServiceOptions::new());
    let address = server.address();
    // As the documented nginx setup forwards: the public `Host`, the scheme
    // overwritten, the client appended; a host the client stated itself is
    // passed through, and not read.
    let forwarded = "Host: data.example.com\r\nX-Forwarded-Proto: https\r\nX-Forwarded-Host: evil.example\r\nX-Forwarded-For: 203.0.113.9\r\n";
    let (status, _, body) = exchange(
        &server,
        format!("GET /olap/xmla?probe=1 HTTP/1.1\r\n{forwarded}Connection: close\r\n\r\n")
            .as_bytes(),
    );
    assert_eq!(status, 200);
    let text = String::from_utf8_lossy(&body);
    assert!(
        text.contains("at https://data.example.com/olap/xmla."),
        "{text}"
    );
    let discover = Request::from(Discover::new(RequestType::DbschemaTables))
        .into_bytes()
        .expect("a request");
    let mut request = format!(
        "POST /olap/xmla HTTP/1.1\r\n{forwarded}Content-Type: text/xml\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n",
        discover.len()
    )
    .into_bytes();
    request.extend_from_slice(&discover);
    let (status, headers, body) = exchange(&server, &request);
    assert_eq!(status, 200);
    assert!(
        headers
            .iter()
            .any(|(name, value)| name == "x-transport-caps-negotiation-flags"
                && value == "0,0,0,0,0"),
        "{headers:?}"
    );
    let response = Response::from_bytes(&body, None).expect("a response");
    assert_eq!(response.rows().map(yggdryl::Serie::len), Some(1));

    // Off the proxy's path the same route answers, described where it was
    // reached.
    let (status, _, body) = exchange(
        &server,
        format!("GET /xmla HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n").as_bytes(),
    );
    assert_eq!(status, 200);
    assert!(
        String::from_utf8_lossy(&body).contains(&format!("at http://{address}/xmla.")),
        "{}",
        String::from_utf8_lossy(&body)
    );
}

#[test]
fn a_post_to_the_path_with_a_trailing_slash_is_served_and_a_get_of_it_is_308() {
    let (server, endpoint, _service) = running("slashed");
    let slashed = format!("{endpoint}/");
    let discover = Request::from(Discover::new(RequestType::DbschemaTables))
        .into_bytes()
        .expect("a request");
    let response = post(
        &Url::from_str(&slashed).expect("a URL"),
        discover,
        "urn:schemas-microsoft-com:xml-analysis:Discover",
    )
    .expect("the server answers");
    assert_eq!(response.status(), Status::OK);
    let body = response.bytes().expect("a body");
    let discovered = Response::from_bytes(&body, None).expect("a response");
    assert_eq!(discovered.rows().expect("a rowset").len(), 1);
    assert_eq!(
        server.requests().last().map(|recorded| recorded.status),
        Some(Status::OK),
        "served, never redirected"
    );

    let address = server.address();
    let (status, headers, body) = exchange(
        &server,
        format!("GET /xmla/?x=1 HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n")
            .as_bytes(),
    );
    assert_eq!(status, 308);
    assert!(
        headers
            .iter()
            .any(|(name, value)| name == "location" && value == "../xmla?x=1"),
        "{headers:?}"
    );
    assert!(body.is_empty());
}

#[test]
fn discover_datasources_states_the_public_endpoint_when_the_options_name_it() {
    let public = Url::from_str("https://data.example.com/olap").expect("a URL");
    let server_options = ServerOptions::default().with_public_url(public);
    let (server, endpoint, _service) =
        running_with("public", server_options, ServiceOptions::new());
    assert_eq!(data_source_url(&endpoint), Scalar::Null, "unstated");
    assert_eq!(
        server.public_url_of("/xmla").expect("a URL").to_string(),
        "https://data.example.com/olap/xmla"
    );

    let public = Url::from_str("https://data.example.com/olap").expect("a URL");
    let server_options = ServerOptions::default().with_public_url(public);
    let service_options = ServiceOptions::new().with_url("https://data.example.com/olap/xmla");
    let (_server, endpoint, _service) = running_with("stated", server_options, service_options);
    assert_eq!(
        data_source_url(&endpoint),
        Scalar::from("https://data.example.com/olap/xmla")
    );
}

#[test]
fn a_session_posting_through_a_308_reaches_the_route_with_its_body() {
    let (server, endpoint, _service) = running("redirected");
    server.respond(
        Some(Method::Post),
        "/old",
        HttpResponse::new(Status::PERMANENT_REDIRECT)
            .with_header("location", "xmla")
            .expect("a header"),
    );
    let discover = Request::from(
        Discover::new(RequestType::DbschemaTables)
            .with_properties(PropertyList::new().with("Catalog", "market")),
    )
    .into_bytes()
    .expect("a request");
    let old = server.url_of("/old").expect("a URL");
    let response = post(
        &old,
        discover,
        "urn:schemas-microsoft-com:xml-analysis:Discover",
    )
    .expect("the server answers");
    assert_eq!(response.status(), Status::OK);
    assert_eq!(response.url(), &endpoint, "landed on the route");
    let body = response.bytes().expect("a body");
    let discovered = Response::from_bytes(&body, None).expect("a response");
    assert_eq!(discovered.rows().expect("a rowset").len(), 1);
    let statuses: Vec<u16> = server
        .requests()
        .iter()
        .map(|recorded| recorded.status.code())
        .collect();
    assert_eq!(statuses, [308, 200]);
}

#[test]
fn a_trace_writes_each_exchange_as_it_went_over_the_wire_and_the_request_replays() {
    let root = catalog_root("traced");
    let trace = root.parent().expect("a parent").join(format!(
        "yggdryl-xmla-server-trace-log-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&trace);
    let service = Arc::new(
        Service::new(ServiceOptions::new()).with_catalog(Catalog::new(
            "market",
            Holder::folder(&root).expect("holds"),
        )),
    );
    let server = Server::bind_with(
        "127.0.0.1:0",
        ServerOptions::default().with_trace(Holder::folder(&trace).expect("a trace folder")),
    )
    .expect("a loopback port");
    let endpoint = Arc::clone(&service)
        .route(&server, "/xmla")
        .expect("the path routes");
    let discover = Request::from(
        Discover::new(RequestType::DbschemaTables)
            .with_properties(PropertyList::new().with("Catalog", "market")),
    )
    .into_bytes()
    .expect("a request");
    let response = post(
        &endpoint,
        discover.clone(),
        "urn:schemas-microsoft-com:xml-analysis:Discover",
    )
    .expect("the server answers");
    assert_eq!(response.status(), Status::OK);
    let live = response.bytes().expect("a body");

    // Each pair is completed as its exchange ends, on the connection's own
    // thread: a moment after the client read the last byte.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline
        && !std::fs::read(trace.join("0000-response.http"))
            .is_ok_and(|bytes| bytes.ends_with(b"0\r\n\r\n"))
    {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let read = |name: &str| {
        std::fs::read(trace.join(name)).unwrap_or_else(|error| panic!("{name}: {error}"))
    };
    let traced_request = read("0000-request.http");
    assert!(
        traced_request.starts_with(b"POST /xmla HTTP/1.1\r\n"),
        "{}",
        String::from_utf8_lossy(&traced_request)
    );
    assert!(traced_request.ends_with(&discover));
    let traced_response = read("0000-response.http");
    assert!(
        traced_response.starts_with(b"HTTP/1.1 200 OK\r\n"),
        "{}",
        String::from_utf8_lossy(&traced_response)
    );
    assert!(
        String::from_utf8_lossy(&traced_response)
            .to_ascii_lowercase()
            .contains("transfer-encoding: chunked"),
        "the SOAP answer is chunked on the wire\n{}",
        String::from_utf8_lossy(&traced_response)
    );

    // The traced request replays through the service and answers the same
    // rows.
    let replayed = HttpRequest::from_bytes(&traced_request).expect("the traced request reads");
    let again = service
        .handle(replayed.body().as_bytes(), Vec::new())
        .expect("answered");
    let live = Response::from_bytes(&live, None).expect("a response");
    let again = Response::from_bytes(&again, None).expect("a response");
    assert_eq!(live.rows(), again.rows());
    server.shutdown().expect("the server stops");
    let _ = std::fs::remove_dir_all(&trace);
}
