//! Process-level checks for `ygg xmla serve`: the endpoint it prints answers
//! the SOAP 1.1 HTTP binding, and what it refuses it refuses before binding.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

use yggdryl::holder::Holder;
use yggdryl::local::LocalFolder;
use yggdryl::media::IORecordOptions;
use yggdryl::xmla::{Discover, Request, RequestType, Response};
use yggdryl::{DataType, IOBase, IOMedia, Scalar, StructType};

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// A catalog folder holding one table, `trades.arrows`.
fn catalog_root() -> PathBuf {
    let mut root = LocalFolder::temporary()
        .expect("native temporary folder")
        .path()
        .expect("native temporary path");
    root.push(format!(
        "ygg-cli-xmla-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("isolated test folder");
    let field = StructType::from_fields([
        DataType::utf8().required_field("symbol"),
        DataType::Int64.required_field("size"),
    ])
    .map(DataType::from)
    .expect("a valid root")
    .required_field("row");
    let mut leaf = Holder::folder(&root)
        .expect("the root holds")
        .child_by_path("trades.arrows")
        .expect("the child resolves");
    let options = leaf
        .record_options()
        .expect("IPC options")
        .with_field(field);
    leaf.overwrite_records(
        [
            Scalar::from_sequence([Scalar::from("AAPL"), Scalar::from(100_i64)]),
            Scalar::from_sequence([Scalar::from("MSFT"), Scalar::from(250_i64)]),
        ],
        &options,
    )
    .expect("the table is written");
    root
}

/// The served process, killed when the test ends however it ends.
struct Served(Child);

impl Drop for Served {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn ygg() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ygg"));
    command
        .env("NO_COLOR", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

/// One HTTP exchange: the status and the whole body, a chunked body
/// reassembled.
fn exchange(address: &str, request: &[u8]) -> (u16, Vec<u8>) {
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
    let mut chunked = false;
    let mut length = None;
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).expect("a header line");
        let header = header.trim_end_matches(['\r', '\n']);
        if header.is_empty() {
            break;
        }
        let (name, value) = header.split_once(':').expect("a header");
        match name.trim().to_ascii_lowercase().as_str() {
            "transfer-encoding" => chunked = value.trim() == "chunked",
            "content-length" => length = value.trim().parse::<usize>().ok(),
            _ => {}
        }
    }
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
        body.resize(length.expect("a content length"), 0);
        reader.read_exact(&mut body).expect("the body");
    }
    (status, body)
}

#[test]
fn serve_prints_its_endpoint_first_and_answers_a_discover() {
    let root = catalog_root();
    let mut child = ygg()
        .args(["xmla", "serve", "--bind", "127.0.0.1:0", "--path", "/olap"])
        .arg(format!("market={}", root.display()))
        .spawn()
        .expect("the provider starts");
    let stdout = child.stdout.take().expect("piped stdout");
    let served = Served(child);
    let mut lines = BufReader::new(stdout);
    let mut endpoint = String::new();
    lines.read_line(&mut endpoint).expect("the endpoint line");
    let endpoint = endpoint.trim();
    assert!(
        endpoint.starts_with("http://127.0.0.1:") && endpoint.ends_with("/olap"),
        "{endpoint}"
    );
    let address = endpoint
        .trim_start_matches("http://")
        .split('/')
        .next()
        .expect("an authority");

    let payload = Request::from(Discover::new(RequestType::DbschemaTables))
        .into_bytes()
        .expect("a request encodes");
    let mut request = format!(
        "POST /olap HTTP/1.1\r\nHost: {address}\r\nContent-Type: text/xml; charset=utf-8\r\n\
         SOAPAction: \"urn:schemas-microsoft-com:xml-analysis:Discover\"\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    )
    .into_bytes();
    request.extend_from_slice(&payload);
    let (status, body) = exchange(address, &request);
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
    let response = Response::from_bytes(&body, None).expect("a DiscoverResponse");
    let tables = response.rows().expect("a rowset");
    assert_eq!(tables.len(), 1);
    assert_eq!(
        tables
            .child("TABLE_NAME")
            .and_then(|column| column.scalar(0).ok()),
        Some(Scalar::from("trades"))
    );
    drop(served);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn serve_without_a_catalog_is_a_usage_error() {
    let output = ygg()
        .args(["xmla", "serve"])
        .output()
        .expect("the process runs");
    assert!(!output.status.success());
    let text = String::from_utf8_lossy(&output.stderr);
    assert!(text.contains("CATALOG"), "{text}");
}

#[test]
fn serve_refuses_an_address_it_cannot_bind() {
    let root = catalog_root();
    let output = ygg()
        .args(["xmla", "serve", "--bind", "not-an-address"])
        .arg(root.to_str().expect("a UTF-8 path"))
        .output()
        .expect("the process runs");
    assert!(!output.status.success());
    let text = String::from_utf8_lossy(&output.stdout) + String::from_utf8_lossy(&output.stderr);
    assert!(
        text.contains("not-an-address") || text.to_ascii_lowercase().contains("address"),
        "{text}"
    );
    let _ = std::fs::remove_dir_all(&root);
}
