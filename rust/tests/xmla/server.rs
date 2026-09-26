//! `rust/src/xmla/server.rs`: the provider reached over a loopback socket
//! with the SOAP 1.1 HTTP binding, one request per POST.

use std::io::{BufRead, BufReader, Read, Write};
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
    let mut reader = BufReader::new(stream);
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
