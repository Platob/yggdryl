//! Process-level checks for `ygg xmla serve`: the endpoint it prints answers
//! the SOAP 1.1 HTTP binding, and what it refuses it refuses before binding.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

use yggdryl::holder::Holder;
use yggdryl::http::{Request as HttpRequest, Status};
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

    let payload = Request::from(Discover::new(RequestType::DbschemaTables))
        .into_bytes()
        .expect("a request encodes");
    let response = HttpRequest::post(endpoint, payload)
        .expect("a request builds")
        .with_header("content-type", "text/xml; charset=utf-8")
        .expect("a header")
        .with_header(
            "SOAPAction",
            "\"urn:schemas-microsoft-com:xml-analysis:Discover\"",
        )
        .expect("a header")
        .send()
        .expect("the server answers");
    assert_eq!(response.status(), Status::OK);
    let body = response.bytes().expect("a body");
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

#[test]
fn serve_traces_each_exchange_under_the_folder_asked_for() {
    let root = catalog_root();
    let trace = root.with_extension("trace");
    let _ = std::fs::remove_dir_all(&trace);
    let mut child = ygg()
        .args(["xmla", "serve", "--bind", "127.0.0.1:0", "--trace"])
        .arg(trace.to_str().expect("a UTF-8 path"))
        .arg(format!("market={}", root.display()))
        .spawn()
        .expect("the provider starts");
    let stdout = child.stdout.take().expect("piped stdout");
    let served = Served(child);
    let mut lines = BufReader::new(stdout);
    let mut endpoint = String::new();
    lines.read_line(&mut endpoint).expect("the endpoint line");
    let endpoint = endpoint.trim();
    let payload = Request::from(Discover::new(RequestType::DbschemaCatalogs))
        .into_bytes()
        .expect("a request encodes");
    let response = HttpRequest::post(endpoint, payload.clone())
        .expect("a request builds")
        .with_header("content-type", "text/xml")
        .expect("a header")
        .send()
        .expect("the server answers");
    assert_eq!(response.status(), Status::OK);
    // The response file is completed once the exchange ends, on the
    // connection's own thread: a moment after the client read the last byte.
    let response_file = trace.join("0000-response.http");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline
        && !std::fs::read(&response_file).is_ok_and(|bytes| bytes.ends_with(b"0\r\n\r\n"))
    {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    drop(served);
    let traced_request =
        std::fs::read(trace.join("0000-request.http")).expect("the request is traced");
    assert!(
        traced_request.starts_with(b"POST /xmla HTTP/1.1\r\n"),
        "{}",
        String::from_utf8_lossy(&traced_request)
    );
    assert!(traced_request.ends_with(&payload));
    let traced_response = std::fs::read(response_file).expect("the response is traced");
    assert!(
        traced_response.starts_with(b"HTTP/1.1 200 OK\r\n")
            && traced_response.ends_with(b"0\r\n\r\n"),
        "{}",
        String::from_utf8_lossy(&traced_response)
    );
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&trace);
}
