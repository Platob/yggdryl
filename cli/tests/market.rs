//! Process-level checks for `yggdryl market serve`: what it refuses it refuses
//! before a capture lands, every example its help states runs, and the
//! endpoint it prints answers the display - the page and every file it names,
//! resolved as a browser resolves them - and the book routes; with the
//! `iceberg` feature, a FIX bridge capture folded into a table folder - one
//! the command creates - is served as its books.

use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

use yggdryl::graph::{BookIterator, Element, Event, Market, MarketData, QuoteEvent};
use yggdryl::holder::Holder;
use yggdryl::http::{Request, Response, Status};
use yggdryl::local::LocalFolder;
use yggdryl::{Decimal, IOBase, IOMedia, Scalar, Side, State, Url};

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

/// The checkout, where a help example's relative paths resolve.
const CHECKOUT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/..");

/// The committed FIX dictionary.
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
    quote.set_ticker(Some(ticker.into()), true);
    quote.set_side(side, true);
    quote.set_price(Some(price.parse().expect("a decimal")), true);
    quote.set_quantity(Some(Decimal::from_int(quantity)), true);
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

/// `GET url`, redirects followed.
fn get(url: &str) -> Response {
    Request::get(url)
        .expect("a request builds")
        .send()
        .expect("the server answers")
}

/// The answer of `GET` on `reference` resolved against `base` as a browser
/// resolves it (RFC 3986), `200` under `content_type`.
fn served_as(base: &str, reference: &str, content_type: &str) -> Response {
    let url = Url::from_str(base)
        .expect("a URL")
        .join_reference(reference)
        .expect("the reference resolves");
    let response = get(&url.to_string());
    assert_eq!(response.status(), Status::OK, "{url}");
    assert_eq!(
        response.headers().get("content-type"),
        Some(content_type),
        "{url}"
    );
    response
}

/// Every `src="..."` and `href="..."` of `html` that names a file, and
/// every `from './...'` of a module: what a browser fetches next.
fn named_files(text: &str) -> Vec<String> {
    let mut named = Vec::new();
    for opening in ["src=\"", "href=\"", "from '"] {
        let close = if opening.ends_with('\'') { '\'' } else { '"' };
        let mut rest = text;
        while let Some(at) = rest.find(opening) {
            rest = &rest[at + opening.len()..];
            let end = rest.find(close).expect("a closed attribute");
            let reference = &rest[..end];
            if !reference.starts_with('#') && !named.iter().any(|seen| seen == reference) {
                named.push(reference.to_owned());
            }
            rest = &rest[end..];
        }
    }
    named
}

/// The display answering at `endpoint`, as a browser loads it: the page the
/// endpoint leads to, then every file the page names and every module those
/// import, each resolved against the document naming it and answered `200`
/// under its own type. The page's URL.
fn assert_display(endpoint: &str) -> Url {
    let page = get(endpoint);
    assert_eq!(page.status(), Status::OK, "{endpoint}");
    assert_eq!(
        page.headers().get("content-type"),
        Some("text/html; charset=utf-8"),
        "{endpoint}"
    );
    let at = page.url().clone();
    let html = page.text().expect("the page");
    assert!(
        html.contains("<canvas") && html.contains("src=\"app.js\""),
        "{html}"
    );
    let mut pending: Vec<(Url, String)> = named_files(&html)
        .into_iter()
        .map(|reference| (at.clone(), reference))
        .collect();
    let mut fetched = Vec::new();
    while let Some((document, reference)) = pending.pop() {
        let url = document
            .join_reference(&reference)
            .expect("the reference resolves");
        if fetched.contains(&url) {
            continue;
        }
        let content_type = match reference.rsplit('.').next() {
            Some("css") => "text/css; charset=utf-8",
            Some("js") => "text/javascript; charset=utf-8",
            Some("svg") => "image/svg+xml",
            other => panic!("{reference}: a file of no type the display serves ({other:?})"),
        };
        let response = served_as(&url.to_string(), "", content_type);
        let body = response.bytes().expect("a body");
        assert!(!body.is_empty(), "{url}");
        if content_type.starts_with("text/javascript") {
            let module = String::from_utf8(body.to_vec()).expect("UTF-8");
            pending.extend(
                named_files(&module)
                    .into_iter()
                    .map(|reference| (url.clone(), reference)),
            );
        }
        fetched.push(url);
    }
    for file in [
        "theme.css",
        "theme.js",
        "api.js",
        "chart.js",
        "audit.js",
        "app.js",
        "favicon.svg",
    ] {
        let url = at.join_reference(file).expect("a file resolves");
        assert!(fetched.contains(&url), "{url} among {fetched:?}");
    }
    at
}

/// The failure `args` earn before anything is served: the process fails, no
/// endpoint line is printed - a refusal is printed on stdout too, a usage
/// error on stderr - and `named` is in what it says. A process that prints
/// an endpoint instead is stopped there, since it would serve until killed.
fn refused(args: &[&str], named: &[&str]) {
    let mut child = command()
        .args(["market", "serve"])
        .args(args)
        .spawn()
        .expect("the process runs");
    let mut stdout = BufReader::new(child.stdout.take().expect("piped stdout"));
    let mut said = String::new();
    stdout.read_line(&mut said).expect("stdout reads");
    if said.starts_with("http://") {
        let _ = child.kill();
        let _ = child.wait();
        panic!("nothing served for {args:?}: {said}");
    }
    stdout.read_to_string(&mut said).expect("stdout reads");
    let output = child.wait_with_output().expect("the process ends");
    assert!(!output.status.success(), "{args:?}");
    let text = said + &String::from_utf8_lossy(&output.stderr);
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

/// The bytes of `leaf`, to show a refused run landed nothing in it.
fn size_of(leaf: &std::path::Path) -> u64 {
    std::fs::metadata(leaf).expect("the leaf is there").len()
}

#[test]
fn serve_refuses_a_path_it_cannot_route_before_a_capture_lands() {
    let root = books_root();
    let leaf = root.join("books.arrows");
    let before = size_of(&leaf);
    for (path, named) in [
        ("/book?view=1", "\"/book?view=1\""),
        ("/book#top", "\"/book#top\""),
        ("/book\u{7}", "http path"),
        ("/my book", "url reference"),
    ] {
        refused(
            &[
                "--bind",
                "127.0.0.1:0",
                "--capture",
                CAPTURE,
                "--registry",
                REGISTRY,
                "--path",
                path,
                leaf.to_str().expect("a UTF-8 path"),
            ],
            &[named],
        );
        assert_eq!(size_of(&leaf), before, "{path:?} landed a capture");
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn serve_refuses_a_capture_it_cannot_read_before_any_lands() {
    let root = books_root();
    let leaf = root.join("books.arrows");
    let before = size_of(&leaf);
    refused(
        &[
            "--bind",
            "127.0.0.1:0",
            "--capture",
            CAPTURE,
            "--capture",
            "http://[::1/bridge.log",
            "--registry",
            REGISTRY,
            leaf.to_str().expect("a UTF-8 path"),
        ],
        &["missing its closing bracket"],
    );
    assert_eq!(size_of(&leaf), before, "the first capture landed");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn serve_refuses_two_tables_of_one_name() {
    let root = books_root();
    let first = root.join("books.arrows");
    let second = root.join("again").join("books.arrows");
    std::fs::create_dir_all(second.parent().expect("a parent")).expect("a folder");
    std::fs::copy(&first, &second).expect("a second table");
    let before = size_of(&first);
    let first = first.to_str().expect("a UTF-8 path");
    let second = second.to_str().expect("a UTF-8 path");
    // Named alike by `name=`, and alike by their last segment.
    for tables in [
        [format!("books={first}"), format!("books={second}")],
        [first.to_owned(), second.to_owned()],
    ] {
        let name = if tables[0].starts_with("books=") {
            "\"books\""
        } else {
            "\"books.arrows\""
        };
        refused(
            &[
                "--bind",
                "127.0.0.1:0",
                "--capture",
                CAPTURE,
                "--registry",
                REGISTRY,
                &tables[0],
                &tables[1],
            ],
            &["$.tables[1]", name, first, second],
        );
    }
    assert_eq!(
        size_of(&root.join("books.arrows")),
        before,
        "a capture landed"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// The examples `market serve --help` states, one per entry, split into
/// their arguments after `yggdryl`: a line the help wrapped continues the
/// example above it.
fn help_examples() -> Vec<Vec<String>> {
    let output = command()
        .args(["market", "serve", "--help"])
        .output()
        .expect("the process runs");
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).expect("UTF-8");
    let mut examples: Vec<String> = Vec::new();
    for line in help
        .lines()
        .skip_while(|line| *line != "Examples:")
        .skip(1)
        .take_while(|line| !line.trim().is_empty())
    {
        if let Some(example) = line.trim().strip_prefix("yggdryl ") {
            examples.push(example.to_owned());
        } else {
            let last = examples.last_mut().expect("a wrapped example");
            last.push(' ');
            last.push_str(line.trim());
        }
    }
    examples
        .iter()
        .map(|example| example.split_whitespace().map(ToOwned::to_owned).collect())
        .collect()
}

#[test]
fn serve_runs_every_example_its_help_states() {
    let examples = help_examples();
    assert!(examples.len() >= 3, "{examples:?}");
    for example in examples {
        let root = isolated("example");
        // The example as stated, but on a free loopback port and with every
        // absolute location under an isolated folder; its relative paths
        // resolve in the checkout.
        let mut args = Vec::with_capacity(example.len() + 2);
        let mut arguments = example.iter();
        let mut bound = false;
        while let Some(argument) = arguments.next() {
            if argument.starts_with("--") {
                let value = arguments.next().expect("every flag takes a value");
                args.push(argument.clone());
                if argument == "--bind" {
                    bound = true;
                    args.push("127.0.0.1:0".to_owned());
                } else {
                    args.push(value.clone());
                }
            } else if let Some((name, location)) = argument
                .split_once('=')
                .filter(|(_, location)| location.starts_with('/'))
            {
                args.push(format!("{name}={}{location}", root.display()));
            } else if argument.starts_with('/') {
                args.push(format!("{}{argument}", root.display()));
            } else {
                args.push(argument.clone());
            }
        }
        if !bound {
            args.extend(["--bind".to_owned(), "127.0.0.1:0".to_owned()]);
        }
        let mut serve = command();
        serve.current_dir(CHECKOUT).args(&args);
        let (served, endpoint, _) = started(serve, 0);
        assert!(
            endpoint.starts_with("http://127.0.0.1:"),
            "yggdryl {}: {endpoint}",
            example.join(" ")
        );
        drop(served);
        let _ = std::fs::remove_dir_all(&root);
    }
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
    // `--path` as spelled, the endpoint's path, and where the page stands.
    for (path, folder) in [
        ("/book", "/book/"),
        ("deep/book/", "/deep/book/"),
        ("/", "/"),
    ] {
        let mut serve = command();
        serve
            .args(["market", "serve", "--bind", "127.0.0.1:0", "--path", path])
            .arg(format!("books={}", root.display()));
        let (served, endpoint, notes) = started(serve, 1);
        assert!(
            notes[0].starts_with("· table books over file://"),
            "{notes:?}"
        );

        // The display: the endpoint leads to the page, and every file it
        // names answers where a browser resolves it - under the folder the
        // endpoint is.
        let page = assert_display(&endpoint);
        let origin = endpoint
            .strip_suffix(folder)
            .unwrap_or_else(|| panic!("{path}: {endpoint} is the folder {folder}"));
        assert!(origin.starts_with("http://127.0.0.1:"), "{endpoint}");
        if path == "/" {
            assert_eq!(page.to_string(), endpoint);
        } else {
            assert_eq!(page.to_string(), format!("{endpoint}index.html"));
            // The path spelled bare is sent into its folder, relatively, so
            // the page is right under any prefix a proxy adds.
            let bare = Request::get(endpoint.trim_end_matches('/'))
                .expect("a request builds")
                .with_follow_redirects(false)
                .send()
                .expect("the server answers");
            assert_eq!(bare.status(), Status::PERMANENT_REDIRECT, "{endpoint}");
            assert_eq!(
                bare.headers().get("location"),
                Some("book/index.html"),
                "{endpoint}"
            );
            assert_display(endpoint.trim_end_matches('/'));
        }
        // `index.html` in the folder is the page, at the root too.
        assert_eq!(
            served_as(&endpoint, "index.html", "text/html; charset=utf-8")
                .text()
                .expect("the page"),
            get(&page.to_string()).text().expect("the page")
        );

        // The routes stand under `<path>/api`, where the page reads them.
        let tables = served_as(&page.to_string(), "api/tables", "application/json")
            .scalar()
            .expect("JSON");
        let tables = tables.sequence_rows().expect("an array");
        assert_eq!(tables.len(), 1);
        assert_eq!(
            tables[0].as_struct().expect("an object")["name"],
            Scalar::from("books")
        );
        let tickers = served_as(&endpoint, "api/tickers?table=books", "application/json")
            .text()
            .expect("JSON");
        assert!(tickers.contains("\"ticker\":\"ACME\""), "{tickers}");
        let candles = served_as(
            &endpoint,
            "api/candles?table=books&ticker=ACME&from=2026-01-05T10:00:00Z&to=2026-01-05T10:02:00Z",
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
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[ignore = "hosts a live server, which races the runner's socket readiness; run with --ignored"]
fn serve_answers_an_empty_or_absent_folder_as_a_table_of_no_books() {
    // A folder with nothing in it and a path where nothing is: served with
    // no capture, each is a table holding no book, and reading it creates
    // nothing.
    let root = isolated("empty");
    std::fs::create_dir_all(root.join("empty")).expect("isolated test folder");
    let absent = root.join("absent");
    let mut serve = command();
    serve
        .args(["market", "serve", "--bind", "127.0.0.1:0"])
        .arg(format!("empty={}", root.join("empty").display()))
        .arg(format!("absent={}", absent.display()));
    let (served, endpoint, notes) = started(serve, 2);
    for (note, table) in notes.iter().zip(["empty", "absent"]) {
        assert!(
            note.starts_with(&format!("· table {table} over file://"))
                && !note.ends_with(", created"),
            "{notes:?}"
        );
    }
    assert_display(&endpoint);
    for table in ["empty", "absent"] {
        let tickers = served_as(
            &endpoint,
            &format!("api/tickers?table={table}"),
            "application/json",
        )
        .text()
        .expect("JSON");
        assert_eq!(tickers, "[]", "{table}");
        let range =
            format!("table={table}&ticker=ACME&from=2026-01-05T10:00:00Z&to=2026-01-05T10:02:00Z");
        for leaf in ["candles", "events", "audit.csv"] {
            let url = Url::from_str(&endpoint)
                .expect("a URL")
                .join_reference(&format!("api/{leaf}?{range}"))
                .expect("the reference resolves");
            let answer = get(&url.to_string());
            assert_eq!(answer.status(), Status::NOT_FOUND, "{url}");
            let error = answer.scalar().expect("JSON");
            assert_eq!(
                error.as_struct().expect("an object")["error"],
                Scalar::from(format!(
                    "expected a ticker at \"{table}/ACME\", got nothing"
                )),
                "{url}"
            );
        }
        let url = Url::from_str(&endpoint)
            .expect("a URL")
            .join_reference(&format!(
                "api/book?table={table}&ticker=ACME&at=2026-01-05T10:01:00Z"
            ))
            .expect("the reference resolves");
        assert_eq!(get(&url.to_string()).status(), Status::NOT_FOUND, "{url}");
    }
    drop(served);
    assert!(!absent.exists(), "a reading creates nothing");
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
    assert!(
        endpoint.starts_with("http://127.0.0.1:") && endpoint.ends_with('/'),
        "{endpoint}"
    );
    assert!(
        notes[1].starts_with(&format!("· capture {CAPTURE}: "))
            && notes[1].ends_with(" books into books"),
        "{notes:?}"
    );

    let tickers = served_as(&endpoint, "api/tickers?table=books", "application/json")
        .scalar()
        .expect("JSON");
    let (from, to) = span_of(&tickers, "2454");
    let query = format!("table=books&ticker=2454&from={from}&to={to}");
    let candles = served_as(
        &endpoint,
        &format!("api/candles?{query}&interval=1h&tz=Europe/Zurich"),
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
        &format!("api/audit.csv.gz?{query}"),
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
    use yggdryl::iceberg::{FormatVersion, IcebergTable, PartitionSpec};

    let root = isolated("iceberg");
    std::fs::create_dir_all(&root).expect("isolated test folder");
    // The row as Iceberg states it: the `uint64` codes as `decimal(20, 0)`.
    IcebergTable::create(
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
    use yggdryl::iceberg::IcebergTable;

    // Nothing is there, so one command creates the table and serves it.
    let root = isolated("created");
    let notes = capture_served(&root);
    assert!(notes[0].ends_with(", created"), "{notes:?}");
    let table = IcebergTable::locate(Holder::folder(&root).expect("the root holds"))
        .expect("the folder reads")
        .expect("a table was created");
    assert!(
        table.metadata().unwrap().current_snapshot().is_some(),
        "the capture was committed"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[cfg(feature = "iceberg")]
#[ignore = "hosts a live server, which races the runner's socket readiness; run with --ignored"]
fn serve_folds_a_capture_into_a_record_leaf_and_serves_its_books() {
    // Each leaf the help lists takes the capture and answers every reading:
    // an Avro leaf through the row Iceberg states, a CSV leaf through the
    // row every read declares.
    for leaf in ["books.arrows", "books.parquet", "books.avro", "books.csv"] {
        let root = isolated(&format!("leaf-{leaf}"));
        std::fs::create_dir_all(&root).expect("isolated test folder");
        let notes = capture_served(&root.join(leaf));
        assert!(
            notes[0].starts_with("· table books over file://"),
            "{leaf}: {notes:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
