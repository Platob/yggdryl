//! Process-level checks for `yggdryl market serve`: what it refuses it refuses
//! before binding, and the endpoint it prints answers the display and the
//! book routes; with the `iceberg` feature, a FIX bridge capture folded into
//! a table folder - one the command creates - is served as its books.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

use yggdryl::graph::{BookIterator, Element, Event, Market, MarketData, QuoteEvent};
use yggdryl::holder::Holder;
use yggdryl::http::{Request, Response, Status};
use yggdryl::local::LocalFolder;
use yggdryl::{Decimal, IOBase, IOMedia, Scalar, Side, State};

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// Nanoseconds in one second.
const SECOND: i64 = 1_000_000_000;

/// `2026-01-05T10:00:00Z`: the first minute of the fixture.
const T0: i64 = 1_767_607_200 * SECOND;

/// The bundled bridge capture, beside the core's FIX tests.
const CAPTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../rust/tests/fix/ulbridge.log"
);

/// The committed FIX dictionary.
#[cfg(feature = "iceberg")]
const REGISTRY: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../config/fix");

/// An isolated folder for one test, removed before it is used.
fn isolated(stem: &str) -> PathBuf {
    let mut root = LocalFolder::temporary()
        .expect("native temporary folder")
        .path()
        .expect("native temporary path");
    root.push(format!(
        "ygg-cli-serve-{stem}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&root);
    root
}

/// One finalized quote of `ticker` going by `code`.
fn quote(
    unix: i64,
    ticker: &str,
    code: &str,
    side: Side,
    price: &str,
    quantity: i64,
) -> MarketData {
    let mut quote = QuoteEvent::at(unix);
    quote.set_crosscode(code.to_owned());
    quote.set_ticker(Some(ticker.into()));
    quote.set_side(side);
    quote.set_price(Some(price.parse().expect("a decimal")));
    quote.set_quantity(Some(Decimal::from_int(quantity)));
    quote.set_state(State::New);
    quote.finalize();
    MarketData::from(quote)
}

/// A folder holding one table, `books.arrows`: `ACME` quoted on both sides
/// in the first minute and requoted in the second - three books.
fn books_root() -> PathBuf {
    let root = isolated("books");
    std::fs::create_dir_all(&root).expect("isolated test folder");
    let books = BookIterator::new(
        vec![
            quote(T0 + 5 * SECOND, "ACME", "AB1", Side::Buy, "100", 10),
            quote(T0 + 5 * SECOND, "ACME", "AA1", Side::Sell, "101", 5),
            quote(T0 + 65 * SECOND, "ACME", "AB2", Side::Buy, "100.5", 4),
        ]
        .into_iter()
        .map(Ok),
        0,
    )
    .expect("a book iterator")
    .map(|book| book.map(MarketData::from));
    let mut leaf = Holder::folder(&root)
        .expect("the root holds")
        .child_by_path("books.arrows")
        .expect("the child resolves");
    let options = leaf.record_options().expect("IPC options");
    leaf.overwrite_arrow_reader(
        MarketData::arrow_reader(books, None, None).expect("a reader"),
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

fn command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_yggdryl"));
    command
        .env("NO_COLOR", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

/// The lines `command` prints once started - the endpoint first, then every
/// note - and the process kept alive until the guard drops. `notes` is how
/// many lines are read after the endpoint.
fn started(mut command: Command, notes: usize) -> (Served, String, Vec<String>) {
    let mut child = command.spawn().expect("the display starts");
    let stdout = child.stdout.take().expect("piped stdout");
    let served = Served(child);
    let mut lines = BufReader::new(stdout);
    let mut endpoint = String::new();
    lines.read_line(&mut endpoint).expect("the endpoint line");
    let mut read = Vec::with_capacity(notes);
    for _ in 0..notes {
        let mut note = String::new();
        lines.read_line(&mut note).expect("a note");
        read.push(note.trim().to_owned());
    }
    (served, endpoint.trim().to_owned(), read)
}

/// `GET url`.
fn get(url: &str) -> Response {
    Request::get(url)
        .expect("a request builds")
        .send()
        .expect("the server answers")
}

/// The answer of `GET {endpoint}{leaf}` that is `200` under `content_type`.
fn served_as(endpoint: &str, leaf: &str, content_type: &str) -> Response {
    let response = get(&format!("{endpoint}{leaf}"));
    assert_eq!(response.status(), Status::OK, "{leaf}");
    assert_eq!(
        response.headers().get("content-type"),
        Some(content_type),
        "{leaf}"
    );
    response
}

/// The failure `args` earn before anything is served: the process fails, no
/// endpoint line is printed - a refusal is printed on stdout too, a usage
/// error on stderr - and `named` is in what it says.
fn refused(args: &[&str], named: &[&str]) {
    let output = command()
        .args(["market", "serve"])
        .args(args)
        .output()
        .expect("the process runs");
    assert!(!output.status.success(), "{args:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.lines().any(|line| line.starts_with("http://")),
        "nothing served for {args:?}: {stdout}"
    );
    let text = stdout + String::from_utf8_lossy(&output.stderr);
    for name in named {
        assert!(text.contains(name), "{args:?}: {text}");
    }
}

#[test]
fn serve_refuses_a_capture_with_no_table_to_land_in() {
    refused(
        &["--bind", "127.0.0.1:0", "--capture", CAPTURE],
        &["a capture needs a table to land in", "$.capture"],
    );
}

#[test]
fn serve_refuses_a_zone_it_has_no_rules_for() {
    let root = books_root();
    refused(
        &[
            "--bind",
            "127.0.0.1:0",
            "--timezone",
            "Mars/Olympus",
            root.to_str().expect("a UTF-8 path"),
        ],
        &["$.timezone", "Mars/Olympus"],
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn serve_refuses_a_row_header_that_does_not_compile() {
    let root = books_root();
    refused(
        &[
            "--bind",
            "127.0.0.1:0",
            "--rowheader",
            "^(?P<mtime>",
            root.to_str().expect("a UTF-8 path"),
        ],
        &["rowheader"],
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn serve_refuses_a_dictionary_it_cannot_read() {
    let root = books_root();
    let missing = isolated("registry");
    refused(
        &[
            "--bind",
            "127.0.0.1:0",
            "--capture",
            CAPTURE,
            "--registry",
            missing.to_str().expect("a UTF-8 path"),
            root.to_str().expect("a UTF-8 path"),
        ],
        &["FIX dictionary", "got nothing"],
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn serve_refuses_a_path_it_cannot_route() {
    let root = books_root();
    refused(
        &[
            "--bind",
            "127.0.0.1:0",
            "--path",
            "/book?view=1",
            root.to_str().expect("a UTF-8 path"),
        ],
        &["http path"],
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn serve_refuses_a_grid_a_read_timeout_or_a_forwarded_field_it_cannot_honour() {
    let root = books_root();
    for (flag, value, named) in [
        ("--snapshot-millis", "x", "snapshot-millis"),
        ("--forwarded-header", "X-Real-IP", "forwarded-header"),
        ("--read-timeout", "0", "read-timeout"),
    ] {
        refused(
            &[
                "--bind",
                "127.0.0.1:0",
                flag,
                value,
                root.to_str().expect("a UTF-8 path"),
            ],
            &[named],
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[ignore = "hosts a live server, which races the runner's socket readiness; run with --ignored"]
fn serve_prints_its_endpoint_first_and_answers_the_display_and_the_api() {
    let root = books_root();
    let mut serve = command();
    serve
        .args([
            "market",
            "serve",
            "--bind",
            "127.0.0.1:0",
            "--path",
            "/book",
        ])
        .arg(format!("books={}", root.display()));
    let (served, endpoint, notes) = started(serve, 1);
    assert!(
        endpoint.starts_with("http://127.0.0.1:") && endpoint.ends_with("/book"),
        "{endpoint}"
    );
    assert!(
        notes[0].starts_with("· table books over file://"),
        "{notes:?}"
    );

    // The display: the path itself is the page, and every asset stands
    // beside it under its own name and type.
    let page = served_as(&endpoint, "", "text/html; charset=utf-8")
        .text()
        .expect("the page");
    assert!(
        page.contains("<canvas") && page.contains("src=\"app.js\""),
        "{page}"
    );
    assert_eq!(
        served_as(&endpoint, "/index.html", "text/html; charset=utf-8")
            .text()
            .expect("the page"),
        page
    );
    for (leaf, content_type) in [
        ("/theme.css", "text/css; charset=utf-8"),
        ("/theme.js", "text/javascript; charset=utf-8"),
        ("/api.js", "text/javascript; charset=utf-8"),
        ("/chart.js", "text/javascript; charset=utf-8"),
        ("/audit.js", "text/javascript; charset=utf-8"),
        ("/app.js", "text/javascript; charset=utf-8"),
        ("/favicon.svg", "image/svg+xml"),
    ] {
        let response = served_as(&endpoint, leaf, content_type);
        assert!(!response.bytes().expect("a body").is_empty(), "{leaf}");
    }

    // The routes stand under `<path>/api`.
    let tables = served_as(&endpoint, "/api/tables", "application/json")
        .scalar()
        .expect("JSON");
    let tables = tables.sequence_rows().expect("an array");
    assert_eq!(tables.len(), 1);
    assert_eq!(
        tables[0].as_struct().expect("an object")["name"],
        Scalar::from("books")
    );
    let tickers = served_as(&endpoint, "/api/tickers?table=books", "application/json")
        .text()
        .expect("JSON");
    assert!(tickers.contains("\"ticker\":\"ACME\""), "{tickers}");
    let candles = served_as(
        &endpoint,
        "/api/candles?table=books&ticker=ACME&from=2026-01-05T10:00:00Z&to=2026-01-05T10:02:00Z",
        "application/json",
    )
    .scalar()
    .expect("JSON");
    assert_eq!(
        candles.as_struct().expect("an object")["candles"]
            .sequence_rows()
            .expect("an array")
            .len(),
        2
    );
    drop(served);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn serve_refuses_an_address_it_cannot_bind() {
    let root = books_root();
    refused(
        &[
            "--bind",
            "not-an-address",
            root.to_str().expect("a UTF-8 path"),
        ],
        &[],
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// The whole span of the one ticker `tickers` lists, as the display would
/// send it back: `from` and `to` verbatim.
#[cfg(feature = "iceberg")]
fn span_of(tickers: &Scalar, ticker: &str) -> (String, String) {
    let listed = tickers.sequence_rows().expect("an array");
    let entry = listed
        .iter()
        .map(|entry| entry.as_struct().expect("an object"))
        .find(|entry| entry["ticker"] == Scalar::from(ticker))
        .unwrap_or_else(|| panic!("{ticker} among {tickers:?}"));
    (
        entry["from"].as_str().expect("text").to_owned(),
        entry["to"].as_str().expect("text").to_owned(),
    )
}

/// The capture folded into `root` and served as `books`: the notes, the
/// tickers, the candles over one ticker's span and the gzip audit.
#[cfg(feature = "iceberg")]
fn capture_served(root: &std::path::Path) -> Vec<String> {
    let mut serve = command();
    serve
        .args(["market", "serve", "--bind", "127.0.0.1:0"])
        .args(["--capture", CAPTURE])
        .args(["--registry", REGISTRY])
        .args(["--timezone", "Europe/Zurich"])
        .arg(format!("books={}", root.display()));
    let (served, endpoint, notes) = started(serve, 2);
    assert!(endpoint.starts_with("http://127.0.0.1:"), "{endpoint}");
    let endpoint = endpoint.trim_end_matches('/').to_owned();
    assert!(
        notes[1].starts_with(&format!("· capture {CAPTURE}: "))
            && notes[1].ends_with(" books into books"),
        "{notes:?}"
    );

    let tickers = served_as(&endpoint, "/api/tickers?table=books", "application/json")
        .scalar()
        .expect("JSON");
    let (from, to) = span_of(&tickers, "2454");
    let query = format!("table=books&ticker=2454&from={from}&to={to}");
    let candles = served_as(
        &endpoint,
        &format!("/api/candles?{query}&interval=1h&tz=Europe/Zurich"),
        "application/json",
    )
    .scalar()
    .expect("JSON");
    let candles = candles.as_struct().expect("an object");
    assert_eq!(candles["timezone"], Scalar::from("Europe/Zurich"));
    assert!(
        !candles["candles"]
            .sequence_rows()
            .expect("an array")
            .is_empty(),
        "{candles:?}"
    );

    let audit = served_as(
        &endpoint,
        &format!("/api/audit.csv.gz?{query}"),
        "application/gzip",
    );
    assert!(
        audit
            .headers()
            .get("content-disposition")
            .is_some_and(|value| value.starts_with("attachment; filename=\"audit-2454-")),
        "{:?}",
        audit.headers()
    );
    let csv = yggdryl::gzip::load(&audit.bytes().expect("a body")).expect("gzip decodes");
    let csv = String::from_utf8(csv).expect("UTF-8");
    assert!(csv.starts_with("bookunix,role,marketdatakind,"), "{csv}");
    assert!(csv.lines().count() > 1, "{csv}");
    drop(served);
    notes
}

#[test]
#[cfg(feature = "iceberg")]
#[ignore = "hosts a live server, which races the runner's socket readiness; run with --ignored"]
fn serve_folds_a_capture_into_an_iceberg_table_and_serves_its_books() {
    use yggdryl::Scheme;
    use yggdryl::iceberg::{FormatVersion, PartitionSpec, Table};

    let root = isolated("iceberg");
    std::fs::create_dir_all(&root).expect("isolated test folder");
    // The row as Iceberg states it: the `uint64` codes as `decimal(20, 0)`.
    Table::create(
        Holder::folder(&root).expect("the root holds"),
        FormatVersion::V2,
        MarketData::field()
            .expect("the marketdata row")
            .into_scheme_compat(&Scheme::ICEBERG)
            .expect("the widening Iceberg needs"),
        PartitionSpec::unpartitioned(),
    )
    .expect("an empty table");
    let notes = capture_served(&root);
    // The table was there, so the command created nothing.
    assert!(
        notes[0].starts_with("· table books over file://"),
        "{notes:?}"
    );
    assert!(!notes[0].ends_with(", created"), "{notes:?}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[cfg(feature = "iceberg")]
#[ignore = "hosts a live server, which races the runner's socket readiness; run with --ignored"]
fn serve_makes_an_absent_folder_the_table_a_capture_lands_in() {
    use yggdryl::iceberg::Table;

    // Nothing is there, so one command creates the table and serves it.
    let root = isolated("created");
    let notes = capture_served(&root);
    assert!(notes[0].ends_with(", created"), "{notes:?}");
    let table = Table::locate(Holder::folder(&root).expect("the root holds"))
        .expect("the folder reads")
        .expect("a table was created");
    assert!(
        table.metadata().current_snapshot().is_some(),
        "the capture was committed"
    );
    let _ = std::fs::remove_dir_all(&root);
}
