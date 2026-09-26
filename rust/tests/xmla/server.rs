//! `rust/src/xmla/server.rs`: the provider reached over a loopback socket
//! with the SOAP 1.1 HTTP binding, one request per POST.

use std::io::{BufRead, BufReader, Cursor, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, StringArray};
use yggdryl::holder::Holder;
use yggdryl::media::RecordOptions;
use yggdryl::xmla::{
    Catalog, Discover, Execute, PropertyList, Request, RequestType, Response, Server,
    ServerOptions, Service, ServiceOptions,
};
use yggdryl::{DataType, IOBase, IOMedia, MimeType, Scalar, StructType};

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

fn running(label: &str) -> yggdryl::xmla::Running {
    let root = catalog_root(label);
    let service = Service::new(ServiceOptions::new()).with_catalog(Catalog::new(
        "market",
        Holder::folder(&root).expect("holds"),
    ));
    Server::bind(service, "127.0.0.1:0")
        .expect("a loopback port")
        .with_options(ServerOptions::new().with_path("/xmla"))
        .spawn()
}

/// One HTTP exchange: the status, the headers and the whole body, a chunked
/// body reassembled.
fn exchange(address: &str, request: &[u8]) -> (u16, Vec<(String, String)>, Vec<u8>) {
    let mut stream = TcpStream::connect(address).expect("the server accepts");
    stream.write_all(request).expect("the request is sent");
    stream.flush().expect("flushed");
    receive(&mut BufReader::new(stream))
}

/// One response off `reader`: the status, the headers and the whole body, a
/// chunked body reassembled.
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

fn post(address: &str, path: &str, body: &[u8], action: &str) -> (u16, Vec<u8>) {
    let head = format!(
        "POST {path} HTTP/1.1\r\nHost: {address}\r\nContent-Type: text/xml; charset=utf-8\r\n\
         SOAPAction: \"{action}\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let mut request = head.into_bytes();
    request.extend_from_slice(body);
    let (status, _, body) = exchange(address, &request);
    (status, body)
}

#[test]
fn a_discover_and_an_execute_travel_over_the_socket() {
    let server = running("exchange");
    let address = server.local_addr().expect("an address").to_string();
    assert_eq!(server.endpoint(), format!("http://{address}/xmla"));

    let discover = Request::from(Discover::new(RequestType::DbschemaTables))
        .into_bytes()
        .expect("a request");
    let (status, body) = post(
        &address,
        "/xmla",
        &discover,
        "urn:schemas-microsoft-com:xml-analysis:Discover",
    );
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
    let response = Response::from_bytes(&body, None).expect("a response");
    let rows = response.rows().expect("a rowset");
    assert_eq!(rows.len(), 1);

    let execute = Request::from(
        Execute::statement("select symbol from trades where size > 200")
            .with_properties(PropertyList::new().with("Catalog", "market")),
    )
    .into_bytes()
    .expect("a request");
    let (status, body) = post(
        &address,
        "/xmla",
        &execute,
        "urn:schemas-microsoft-com:xml-analysis:Execute",
    );
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
    let response = Response::from_bytes(&body, None).expect("a response");
    let rows = response.rows().expect("a rowset");
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows.get(0).expect("a row").into_owned(),
        Scalar::from_sequence([Scalar::from("MSFT")])
    );
    server.stop();
}

#[test]
fn the_binding_refuses_what_is_not_a_soap_post_by_status() {
    let server = running("statuses");
    let address = server.local_addr().expect("an address").to_string();

    let (status, _, body) = exchange(
        &address,
        format!("GET /xmla HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n").as_bytes(),
    );
    assert_eq!(status, 200);
    assert!(String::from_utf8_lossy(&body).contains("XML for Analysis"));

    let (status, _, _) = exchange(
        &address,
        format!("GET /elsewhere HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n")
            .as_bytes(),
    );
    assert_eq!(status, 404);

    let (status, _, _) = exchange(
        &address,
        format!("PUT /xmla HTTP/1.1\r\nHost: {address}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .as_bytes(),
    );
    assert_eq!(status, 405);

    let (status, _, _) = exchange(
        &address,
        format!(
            "POST /xmla HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/json\r\n\
             Content-Length: 2\r\nConnection: close\r\n\r\n{{}}"
        )
        .as_bytes(),
    );
    // Every fault goes out at 200 in a SOAP body, as the reference providers
    // answer and as XMLA clients read a fault; nothing a SOAP body can carry
    // is a 4xx or a 5xx.
    assert_eq!(status, 200, "a body that is no XML is a fault at 200");

    let (status, body) = post(&address, "/xmla", b"<not-soap/>", "");
    assert_eq!(
        status, 200,
        "a message that is no request is a fault at 200"
    );
    let body = String::from_utf8_lossy(&body);
    assert!(
        body.contains("Fault") && body.contains("ErrorCode=\"1\""),
        "{body}"
    );

    let (status, _, _) = exchange(
        &address,
        format!("POST /xmla HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n").as_bytes(),
    );
    assert_eq!(status, 411, "a body with no length is refused");
    server.stop();
}

#[test]
fn a_connection_carries_several_requests_and_a_chunked_body() {
    let server = running("keep-alive");
    let address = server.local_addr().expect("an address").to_string();
    let mut stream = TcpStream::connect(&address).expect("the server accepts");
    let request = Request::from(Discover::new(RequestType::DiscoverDatasources))
        .into_bytes()
        .expect("a request");
    for round in 0..2 {
        // The second request arrives chunked, the way a client streaming its
        // message would send it.
        let mut wire =
            format!("POST /xmla HTTP/1.1\r\nHost: {address}\r\nContent-Type: text/xml\r\n")
                .into_bytes();
        if round == 0 {
            wire.extend_from_slice(format!("Content-Length: {}\r\n\r\n", request.len()).as_bytes());
            wire.extend_from_slice(&request);
        } else {
            wire.extend_from_slice(b"Transfer-Encoding: chunked\r\n\r\n");
            for chunk in request.chunks(100) {
                wire.extend_from_slice(format!("{:x}\r\n", chunk.len()).as_bytes());
                wire.extend_from_slice(chunk);
                wire.extend_from_slice(b"\r\n");
            }
            wire.extend_from_slice(b"0\r\n\r\n");
        }
        stream.write_all(&wire).expect("sent");
        let mut reader = BufReader::new(stream.try_clone().expect("a reader"));
        let mut line = String::new();
        reader.read_line(&mut line).expect("a status line");
        assert!(line.starts_with("HTTP/1.1 200"), "round {round}: {line}");
        let mut chunked = false;
        loop {
            let mut header = String::new();
            reader.read_line(&mut header).expect("a header");
            let header = header.trim_end_matches(['\r', '\n']).to_ascii_lowercase();
            if header.is_empty() {
                break;
            }
            chunked |= header == "transfer-encoding: chunked";
        }
        assert!(chunked, "a rowset streams as chunks");
        let mut body = Vec::new();
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
        let response = Response::from_bytes(&body, None).expect("a response");
        assert_eq!(response.rows().expect("a rowset").len(), 1, "round {round}");
        // The reader's buffer must be empty for the next round to read from
        // the stream itself; a response is consumed whole above.
        assert!(reader.buffer().is_empty());
    }
    server.stop();
}

#[test]
fn a_client_expecting_a_continue_gets_it_before_it_sends_the_body() {
    let server = running("continue");
    let address = server.local_addr().expect("an address").to_string();
    let body = Request::from(Discover::new(RequestType::DbschemaCatalogs))
        .into_bytes()
        .expect("a request");
    let mut stream = TcpStream::connect(&address).expect("the server accepts");
    let head = format!(
        "POST /xmla HTTP/1.1\r\nHost: {address}\r\nContent-Type: text/xml\r\n\
         SOAPAction: \"urn:schemas-microsoft-com:xml-analysis:Discover\"\r\n\
         Expect: 100-continue\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).expect("the head is sent");
    stream.flush().expect("flushed");
    let mut reader = BufReader::new(stream.try_clone().expect("a read half"));
    // The interim status arrives while the body is still held back.
    let mut interim = String::new();
    reader.read_line(&mut interim).expect("the interim status");
    assert_eq!(interim, "HTTP/1.1 100 Continue\r\n");
    interim.clear();
    reader.read_line(&mut interim).expect("the blank line");
    assert_eq!(interim, "\r\n");
    stream.write_all(&body).expect("the body is sent");
    stream.flush().expect("flushed");
    let (status, headers, body) = receive(&mut reader);
    assert_eq!(status, 200);
    assert!(
        headers.contains(&(
            "x-transport-caps-negotiation-flags".to_owned(),
            "0,0,0,0,0".to_owned()
        )),
        "{headers:?}"
    );
    let response = Response::from_bytes(&body, None).expect("a response");
    assert_eq!(response.rows().map(yggdryl::Serie::len), Some(1));
    server.stop();
}

#[test]
fn every_soap_answer_carries_the_negotiation_flags_and_a_text_answer_none() {
    let server = running("negotiation");
    let address = server.local_addr().expect("an address").to_string();
    let flags = (
        "x-transport-caps-negotiation-flags".to_owned(),
        "0,0,0,0,0".to_owned(),
    );
    // A refused message is a SOAP fault, and carries them.
    let (status, headers, _) = exchange(
        &address,
        format!(
            "POST /xmla HTTP/1.1\r\nHost: {address}\r\nContent-Type: text/plain\r\n\
             Content-Length: 2\r\nConnection: close\r\n\r\n{{}}"
        )
        .as_bytes(),
    );
    assert_eq!(status, 200);
    assert!(headers.contains(&flags), "{headers:?}");
    // A message that is XML but no XMLA is answered by the service, as a
    // fault in the chunked answer, under the same flags.
    let (status, headers, body) = exchange(
        &address,
        format!(
            "POST /xmla HTTP/1.1\r\nHost: {address}\r\nContent-Type: text/xml\r\n\
             Content-Length: 7\r\nConnection: close\r\n\r\n<a/>   "
        )
        .as_bytes(),
    );
    assert_eq!(status, 200);
    assert!(headers.contains(&flags), "{headers:?}");
    assert!(
        yggdryl::soap::Envelope::from_bytes(&body)
            .expect("an envelope")
            .fault()
            .is_some()
    );
    // The endpoint's description is text, not SOAP.
    let (status, headers, _) = exchange(
        &address,
        format!("GET /xmla HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n").as_bytes(),
    );
    assert_eq!(status, 200);
    assert!(!headers.iter().any(|(name, _)| name == &flags.0));
    server.stop();
}

#[test]
fn a_trace_writes_each_exchange_as_it_went_over_the_wire_and_the_request_replays() {
    let root = catalog_root("traced");
    let trace = root.parent().expect("a parent").join(format!(
        "yggdryl-xmla-server-trace-log-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&trace);
    let service = Service::new(ServiceOptions::new()).with_catalog(Catalog::new(
        "market",
        Holder::folder(&root).expect("holds"),
    ));
    let server = Server::bind(service, "127.0.0.1:0")
        .expect("a loopback port")
        .with_options(
            ServerOptions::new()
                .with_path("/xmla")
                .with_trace(Holder::folder(&trace).expect("a trace folder")),
        )
        .spawn();
    let service = Arc::clone(server.service());
    let address = server.local_addr().expect("an address").to_string();
    let discover = Request::from(
        Discover::new(RequestType::DbschemaTables)
            .with_properties(PropertyList::new().with("Catalog", "market")),
    )
    .into_bytes()
    .expect("a request");
    let (status, live) = post(
        &address,
        "/xmla",
        &discover,
        "urn:schemas-microsoft-com:xml-analysis:Discover",
    );
    assert_eq!(status, 200);
    // A request refused on its head is traced too, with the refusal.
    let (status, _, _) = exchange(
        &address,
        format!(
            "POST /xmla HTTP/1.1\r\nHost: {address}\r\nContent-Type: text/xml\r\n\
             Connection: close\r\n\r\n"
        )
        .as_bytes(),
    );
    assert_eq!(status, 411);
    server.stop();
    // Each pair is completed as its exchange ends, on the connection's own
    // thread: a moment after the client read the last byte.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline
        && !std::fs::read(trace.join("0001-response.http"))
            .is_ok_and(|bytes| bytes.starts_with(b"HTTP/1.1 411"))
    {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let read = |name: &str| {
        std::fs::read(trace.join(name)).unwrap_or_else(|error| panic!("{name}: {error}"))
    };
    let request = read("0000-request.http");
    assert!(
        request.starts_with(b"POST /xmla HTTP/1.1\r\n"),
        "{}",
        String::from_utf8_lossy(&request)
    );
    assert!(request.ends_with(&discover));
    let response = read("0000-response.http");
    assert!(
        response.starts_with(b"HTTP/1.1 200 OK\r\n"),
        "{}",
        String::from_utf8_lossy(&response)
    );
    assert!(response.ends_with(b"0\r\n\r\n"));
    assert!(read("0001-request.http").starts_with(b"POST /xmla HTTP/1.1\r\n"));
    assert!(read("0001-response.http").starts_with(b"HTTP/1.1 411 Length Required\r\n"));
    assert!(!trace.join("0002-request.http").exists());
    // The traced request replays through the service and answers the same rows.
    let mut interim = Vec::new();
    let replayed =
        yggdryl::soap::http::Request::read(&mut Cursor::new(&request[..]), &mut interim, 1 << 20)
            .expect("the traced request reads")
            .expect("a request");
    let again = service
        .handle(replayed.body(), Vec::new())
        .expect("answered");
    let live = Response::from_bytes(&live, None).expect("a response");
    let again = Response::from_bytes(&again, None).expect("a response");
    assert_eq!(live.rows(), again.rows());
    let _ = std::fs::remove_dir_all(&trace);
}
