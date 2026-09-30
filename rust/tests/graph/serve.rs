//! `rust/src/graph/serve.rs`: the book service - its readings without HTTP,
//! and every route it answers on the crate's HTTP server: the shapes, the
//! candles' values and their zone alignment, the exclusive `to`, the book at
//! an instant, the audit's roles and sides, the `limit` truncation over a
//! store in any order, the CSV downloads in each coding read back through
//! the CSV medium, every `400` and `404`, the headers every answer carries,
//! the same answers over every medium a table can be and over an empty or
//! absent one - a leaf or a folder - the refusal of a folder holding leaves
//! no encoding reads, the zones `tz` reads, and no credential a location
//! holds in any answer.

use std::collections::BTreeSet;
use std::sync::Arc;

use smol_str::SmolStr;
use yggdryl::graph::{
    BookIterator, BookQuery, BookService, BookServiceOptions, Element, Event, ExecutionEvent,
    Market, MarketData, OrderEvent, QuoteEvent,
};
use yggdryl::holder::{Buffer, Holder};
use yggdryl::http::{Request, Response, Server, Status};
use yggdryl::media::IORecordOptions;
use yggdryl::{
    Codec, DataType, Decimal, IOBase, IOMedia, MimeType, Parameters, Scalar, Scheme, Side, State,
    Timezone, Url,
};

/// Nanoseconds in one second.
const SECOND: i64 = 1_000_000_000;

/// `2026-01-05T10:00:00Z`: the first minute of the fixture.
const T0: i64 = 1_767_607_200 * SECOND;

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
    quote.set_ticker(Some(SmolStr::new(ticker)));
    quote.set_side(side);
    quote.set_price(Some(price.parse().unwrap()));
    quote.set_quantity(Some(Decimal::from_int(quantity)));
    quote.set_state(State::New);
    quote.finalize();
    MarketData::from(quote)
}

/// One finalized execution of `ticker` going by `code`: an order of
/// `quantity` filled whole, so the fill's last quantity is that too.
fn execution(unix: i64, ticker: &str, code: &str, side: Side, quantity: i64) -> MarketData {
    let mut order = OrderEvent::at(unix);
    order.set_crosscode(code.to_owned());
    order.set_ticker(Some(SmolStr::new(ticker)));
    order.set_side(side);
    order.set_price(Some("101".parse().unwrap()));
    order.set_quantity(Some(Decimal::from_int(quantity)));
    order.set_state(State::read("Filled").unwrap());
    let mut fill = ExecutionEvent::from(&order);
    fill.set_lastqty(Some(Decimal::from_int(quantity)));
    let mut execution = MarketData::from(fill);
    execution.finalize();
    execution
}

/// Two tickers across three minutes: `ACME` quoted on both sides in the
/// first minute, requoted and executed in the second and requoted in the
/// third; `BETA` quoted once in each of the first two minutes. Six books.
fn operations() -> Vec<MarketData> {
    vec![
        quote(T0 + 5 * SECOND, "ACME", "AB1", Side::Buy, "100", 10),
        quote(T0 + 5 * SECOND, "ACME", "AA1", Side::Sell, "101", 5),
        quote(T0 + 30 * SECOND, "BETA", "BB1", Side::Buy, "50", 7),
        quote(T0 + 65 * SECOND, "ACME", "AB2", Side::Buy, "100.5", 4),
        execution(T0 + 70 * SECOND, "ACME", "AE1", Side::Sell, 3),
        quote(T0 + 90 * SECOND, "BETA", "BA1", Side::Sell, "52", 8),
        quote(T0 + 125 * SECOND, "ACME", "AA2", Side::Sell, "103", 6),
    ]
}

/// The fixture's books written as `marketdata` rows into a buffer named
/// `books.arrows`.
fn books_holder() -> Holder {
    let books = BookIterator::new(operations().into_iter(), 0)
        .unwrap()
        .map(|book| book.map(MarketData::from));
    let mut holder = Holder::Buffer(
        Buffer::new().with_media_type(Url::from_str("file:///books.arrows").unwrap().media_type()),
    );
    let options = holder.record_options().unwrap();
    holder
        .overwrite_arrow_reader(
            MarketData::arrow_reader(books, None, None).unwrap(),
            &options,
        )
        .unwrap();
    holder
}

/// The fixture's books written under the root an Iceberg table stores them
/// as: every enum a bare `int32` stating no extension, every `uint64` a
/// `decimal(20, 0)`. What the service reads back carries no
/// `marketdatakind` member, only its code.
fn bare_books_holder() -> Holder {
    let books = BookIterator::new(operations().into_iter(), 0)
        .unwrap()
        .map(|book| book.map(MarketData::from));
    let bare = MarketData::field()
        .unwrap()
        .into_scheme_compat(&Scheme::ICEBERG)
        .unwrap();
    let mut holder = Holder::Buffer(
        Buffer::new().with_media_type(Url::from_str("file:///books.arrows").unwrap().media_type()),
    );
    let options = holder.record_options().unwrap().with_field(bare);
    holder
        .overwrite_arrow_reader(
            MarketData::arrow_reader(books, None, None).unwrap(),
            &options,
        )
        .unwrap();
    holder
}

/// The fixture's books appended, as a capture lands them, to an empty
/// buffer named `books.<suffix>` - written by that suffix's medium.
fn captured_holder(suffix: &str) -> Holder {
    let mut holder = Holder::Buffer(
        Buffer::new().with_media_type(
            Url::from_str(&format!("file:///books.{suffix}"))
                .unwrap()
                .media_type(),
        ),
    );
    capture(&mut holder);
    holder
}

/// The fixture's books appended to `holder` through its own record
/// options, as `yggdryl market serve --capture` lands them.
fn capture(holder: &mut Holder) {
    let books = BookIterator::new(operations().into_iter(), 0)
        .unwrap()
        .map(|book| book.map(MarketData::from));
    let options = holder.record_options().unwrap();
    holder
        .append_arrow_reader(
            MarketData::arrow_reader(books, None, None).unwrap(),
            &options,
        )
        .unwrap();
}

/// A service over the fixture as the table `books`.
fn service(options: BookServiceOptions) -> Arc<BookService> {
    Arc::new(BookService::new(options).with_table("books", books_holder()))
}

/// A service over `holder` as the table `books`, routed at `/` on a fresh
/// loopback server.
fn running_over(holder: Holder) -> (Server, Url, Arc<BookService>) {
    let service = Arc::new(BookService::new(BookServiceOptions::new()).with_table("books", holder));
    let server = Server::bind("127.0.0.1:0").unwrap();
    let endpoint = Arc::clone(&service).route(&server, "/").unwrap();
    (server, endpoint, service)
}

/// Every route over `holder` answers what it answers over the fixture's
/// Arrow leaf: the same JSON, the same audit once its coding is undone.
fn answers_as_the_arrow_leaf(holder: Holder) {
    let (_arrow_server, arrow, _) = running("/", BookServiceOptions::new());
    let (_server, endpoint, _) = running_over(holder);
    let at = [
        ("table", "books"),
        ("ticker", "ACME"),
        ("at", "2026-01-05T10:01:40Z"),
    ];
    let mut bids = range().to_vec();
    bids.push(("side", "bid"));
    let asked: [(&str, &[(&str, &str)]); 9] = [
        ("tables", &[]),
        ("tickers", &[("table", "books")]),
        ("candles", &range()),
        ("book", &at),
        ("events", &range()),
        ("events", &bids),
        ("audit.csv", &range()),
        ("audit.csv.gz", &range()),
        ("audit.csv.zst", &range()),
    ];
    for (leaf, query) in asked {
        let (held, expected) = (get(&endpoint, leaf, query), get(&arrow, leaf, query));
        assert_eq!(
            held.status(),
            Status::OK,
            "{leaf}: {}",
            held.text().unwrap()
        );
        if leaf == "tables" {
            // Each lists its own location.
            continue;
        }
        let body = |response: &Response| {
            let bytes = response.bytes().unwrap().to_vec();
            match leaf.rsplit_once('.').map(|(_, suffix)| suffix) {
                Some("gz") => Codec::from_mime_type(&MimeType::GZIP).load(&bytes).unwrap(),
                Some("zst") => Codec::from_mime_type(&MimeType::ZSTD).load(&bytes).unwrap(),
                _ => bytes,
            }
        };
        assert_eq!(
            String::from_utf8(body(&held)).unwrap(),
            String::from_utf8(body(&expected)).unwrap(),
            "{leaf}"
        );
    }
}

/// The service routed at `prefix` on a fresh loopback server; the endpoint
/// `route` answered.
fn running(prefix: &str, options: BookServiceOptions) -> (Server, Url, Arc<BookService>) {
    let service = service(options);
    let server = Server::bind("127.0.0.1:0").unwrap();
    let endpoint = Arc::clone(&service).route(&server, prefix).unwrap();
    (server, endpoint, service)
}

/// `GET {endpoint}/api/{leaf}?{query}`.
fn get(endpoint: &Url, leaf: &str, query: &[(&str, &str)]) -> Response {
    let base = endpoint.to_string();
    let base = base.trim_end_matches('/');
    Request::get(&format!("{base}/api/{leaf}"))
        .unwrap()
        .with_query(query.iter().copied())
        .unwrap()
        .send()
        .unwrap()
}

/// The JSON body of an answer that is `200`.
fn ok_json(response: &Response) -> Scalar {
    assert_eq!(
        response.status(),
        Status::OK,
        "{}",
        response.text().unwrap()
    );
    assert_eq!(
        response.headers().get("content-type"),
        Some("application/json")
    );
    assert_eq!(response.headers().get("cache-control"), Some("no-store"));
    response.scalar().unwrap()
}

/// The `error` text of a refusal under `status`.
fn refused(response: &Response, status: Status) -> String {
    assert_eq!(response.status(), status, "{}", response.text().unwrap());
    assert_eq!(
        response.headers().get("content-type"),
        Some("application/json")
    );
    assert_eq!(response.headers().get("cache-control"), Some("no-store"));
    let body = response.scalar().unwrap();
    body.as_struct().unwrap()["error"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// The member `name` of a JSON object.
fn member<'a>(value: &'a Scalar, name: &str) -> &'a Scalar {
    value
        .as_struct()
        .unwrap_or_else(|| panic!("an object, got {value:?}"))
        .get(name)
        .unwrap_or_else(|| panic!("a member {name} in {value:?}"))
}

/// The text a member states.
fn text(value: &Scalar, name: &str) -> String {
    member(value, name).as_str().unwrap().to_owned()
}

/// The items of a JSON array.
fn items(value: &Scalar) -> Vec<Scalar> {
    value.sequence_rows().unwrap().to_vec()
}

/// The three-minute range of the fixture.
fn range() -> [(&'static str, &'static str); 4] {
    [
        ("table", "books"),
        ("ticker", "ACME"),
        ("from", "2026-01-05T10:00:00Z"),
        ("to", "2026-01-05T10:03:00Z"),
    ]
}

/// The query over the fixture's range without HTTP.
fn query() -> BookQuery {
    BookQuery {
        table: "books".into(),
        ticker: "ACME".into(),
        from: T0,
        to: T0 + 180 * SECOND,
        timezone: Timezone::UTC,
        interval: None,
        side: None,
    }
}

#[test]
fn the_options_state_their_defaults_and_bound_the_event_rows() {
    let options = BookServiceOptions::new();
    assert_eq!(options.snapshot_millis(), 0);
    assert_eq!(options.max_event_rows(), 5_000);
    assert_eq!(options, BookServiceOptions::default());
    let options = options.with_snapshot_millis(250).with_max_event_rows(0);
    assert_eq!(options.snapshot_millis(), 250);
    assert_eq!(
        options.max_event_rows(),
        1,
        "a bound of nothing answers one row"
    );
}

#[test]
fn a_table_named_again_replaces_the_earlier_one() {
    let service = BookService::new(BookServiceOptions::new())
        .with_table("books", Holder::Buffer(Buffer::new()))
        .with_table("other", Holder::Buffer(Buffer::new()))
        .with_table("books", books_holder());
    let names: Vec<&str> = service.tables().iter().map(|table| table.name()).collect();
    assert_eq!(names, ["other", "books"]);
    assert!(
        service.tables()[1]
            .holder()
            .url()
            .is_some_and(|url| url.to_string().starts_with("mem://"))
    );
    assert_eq!(service.options(), &BookServiceOptions::new());
}

#[test]
fn the_events_field_is_the_flat_marketdata_row_behind_its_stamp() {
    let field = BookService::events_field().unwrap();
    assert_eq!(field.name(), "event");
    assert!(!field.is_nullable());
    let names: Vec<&str> = field.fields().iter().map(|child| child.name()).collect();
    assert_eq!(&names[..3], ["bookunix", "role", "marketdatakind"]);
    // The stamp, the kind, the fifteen event, twenty-eight market and four
    // operation columns, then `bookscope`.
    assert_eq!(names.len(), 2 + 1 + 15 + 28 + 4 + 1);
    assert!(
        !names
            .iter()
            .any(|name| ["alive", "deltas", "executions", "bidlimits", "asklimits"].contains(name))
    );
    assert_eq!(
        field.fields()[0].dtype().to_string(),
        "datetime64(ns,\"UTC\")"
    );
    assert!(!field.fields()[1].is_nullable());
}

#[test]
fn a_query_reads_its_parameters_and_refuses_each_by_name() {
    let read =
        |query: &str| BookQuery::from_parameters(&Parameters::from_query(query, true).unwrap());
    let query = read(
        "table=books&ticker=ACME&from=2026-01-05T10:00:00&to=2026-01-05T11:00:00&tz=Europe/Zurich&interval=5m&side=ask",
    )
    .unwrap();
    assert_eq!(query.table, "books");
    assert_eq!(query.ticker, "ACME");
    // Naive instants are wall clocks in `tz`: 10:00 Zurich is 09:00Z.
    assert_eq!(query.from, T0 - 3_600 * SECOND);
    assert_eq!(query.to, T0);
    assert_eq!(query.timezone.as_str(), "Europe/Zurich");
    assert_eq!(query.candle_options().unwrap().spelling(), "5m");
    assert_eq!(
        query.candle_options().unwrap().timezone().as_str(),
        "Europe/Zurich"
    );
    assert_eq!(query.side, Some(Side::Sell));

    let query =
        read("table=books&ticker=ACME&from=2026-01-05T10:00:00Z&to=2026-01-05T12:00:00%2B01:00")
            .unwrap();
    assert_eq!((query.from, query.to), (T0, T0 + 3_600 * SECOND));
    assert_eq!(query.timezone, Timezone::UTC);
    assert_eq!(query.candle_options().unwrap().spelling(), "1m");
    assert_eq!(query.interval, None);
    assert_eq!(query.side, None);

    let refusal = |query: &str| read(query).unwrap_err().to_string();
    assert_eq!(
        refusal("ticker=ACME&from=2026-01-05T10:00:00Z&to=2026-01-05T11:00:00Z"),
        "invalid record value at $.table: expected a value, got none"
    );
    assert!(
        refusal("table=books&from=2026-01-05T10:00:00Z&to=2026-01-05T11:00:00Z")
            .starts_with("invalid record value at $.ticker:")
    );
    assert!(
        refusal("table=books&ticker=ACME&to=2026-01-05T11:00:00Z")
            .starts_with("invalid record value at $.from: expected a value")
    );
    assert!(
        refusal("table=books&ticker=ACME&from=yesterday&to=2026-01-05T11:00:00Z")
            .starts_with("invalid record value at $.from: expected an ISO-8601 instant")
    );
    assert_eq!(
        refusal("table=books&ticker=ACME&from=2026-01-05T11:00:00Z&to=2026-01-05T11:00:00Z"),
        "invalid record value at $.to: expected an instant after `from` (2026-01-05T11:00:00.000000000Z), got 2026-01-05T11:00:00.000000000Z"
    );
    assert!(refusal("table=books&ticker=ACME&from=2026-01-05T10:00:00Z&to=2026-01-05T11:00:00Z&tz=Mars/Olympus").starts_with("invalid record value at $.tz:"));
    assert!(
        refusal(
            "table=books&ticker=ACME&from=2026-01-05T10:00:00Z&to=2026-01-05T11:00:00Z&interval=1x"
        )
        .starts_with("invalid record value at $.interval:")
    );
    assert!(
        refusal(
            "table=books&ticker=ACME&from=2026-01-05T10:00:00Z&to=2026-01-05T11:00:00Z&side=left"
        )
        .starts_with("invalid record value at $.side: expected `bid` or `ask`")
    );
}

#[test]
fn the_readings_answer_over_a_store_that_kept_no_enum_identity() {
    let holder = bare_books_holder();
    let stored = holder
        .read_arrow_field(&holder.record_options().unwrap())
        .unwrap();
    let kind = stored
        .fields()
        .iter()
        .find(|column| column.name() == "marketdatakind")
        .unwrap();
    assert_eq!(kind.dtype(), &DataType::Int32, "{kind:?}");

    let service = BookService::new(BookServiceOptions::new()).with_table("books", holder);
    let tickers = items(&service.tickers("books").unwrap());
    assert_eq!(tickers.len(), 2, "{tickers:?}");
    assert_eq!(text(&tickers[0], "ticker"), "ACME");
    assert_eq!(member(&tickers[0], "books"), &Scalar::from(4_u64));
    assert_eq!(text(&tickers[1], "ticker"), "BETA");

    let candles = service.candles(&query()).unwrap();
    assert_eq!(candles.len(), 3);
    assert_eq!(candles[1].executions, 1);

    let book = service
        .book("books", "ACME", T0 + 100 * SECOND)
        .unwrap()
        .unwrap();
    assert_eq!(book.get_currunix(), T0 + 70 * SECOND);
}

#[test]
fn the_readings_answer_without_http() {
    let service = service(BookServiceOptions::new());

    let tickers = items(&service.tickers("books").unwrap());
    assert_eq!(tickers.len(), 2);
    assert_eq!(text(&tickers[0], "ticker"), "ACME");
    assert_eq!(text(&tickers[1], "ticker"), "BETA");
    assert_eq!(member(&tickers[0], "books"), &Scalar::from(4_u64));

    let candles = service.candles(&query()).unwrap();
    assert_eq!(candles.len(), 3);
    assert_eq!(
        candles[0].bid.map(|bid| bid.close),
        Some(Decimal::from_int(100))
    );
    assert_eq!(
        candles[1].bid.map(|bid| bid.open),
        Some("100.5".parse().unwrap())
    );
    assert_eq!(candles[1].executions, 1);
    assert_eq!(candles[1].volume, Decimal::from_int(3));

    let book = service
        .book("books", "ACME", T0 + 100 * SECOND)
        .unwrap()
        .unwrap();
    assert_eq!(book.get_currunix(), T0 + 70 * SECOND);
    assert_eq!(book.best_price(Side::Buy), Some("100.5".parse().unwrap()));
    assert!(service.book("books", "ACME", T0).unwrap().is_none());
    assert!(
        service
            .book("books", "NONE", T0 + 100 * SECOND)
            .unwrap()
            .is_none()
    );

    let rows: Vec<_> = service
        .events(&query())
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let total: usize = rows.iter().map(|batch| batch.num_rows()).sum();
    assert!(
        total > 4,
        "every book's entries, deltas and executions: {total}"
    );
    assert_eq!(
        rows[0].schema(),
        BookService::events_field()
            .unwrap()
            .into_arrow_schema()
            .unwrap()
    );

    let absent = service.tickers("nope").unwrap_err().to_string();
    assert_eq!(absent, "expected a table at \"nope\", got nothing");
    let mut unknown = query();
    unknown.ticker = "NONE".into();
    assert_eq!(
        service.candles(&unknown).unwrap_err().to_string(),
        "expected a ticker at \"books/NONE\", got nothing"
    );
    let unread = service
        .events(&unknown)
        .err()
        .map(|error| error.to_string())
        .unwrap();
    assert!(unread.starts_with("expected a ticker"), "{unread}");
    let mut empty = query();
    empty.to = T0 + SECOND;
    assert!(
        service.candles(&empty).unwrap().is_empty(),
        "a known ticker with no book in range"
    );
    let mut backwards = query();
    backwards.to = T0;
    assert!(
        service
            .candles(&backwards)
            .unwrap_err()
            .to_string()
            .starts_with("invalid record value at $.to:")
    );
}

#[test]
fn a_csv_leaf_a_capture_landed_in_answers_every_route() {
    // Nothing in the CSV states the nested columns' types: the service reads
    // it under the `marketdata` row, which the location declares none of.
    answers_as_the_arrow_leaf(captured_holder("csv"));
}

#[test]
fn an_avro_leaf_holding_the_row_as_iceberg_states_it_answers_every_route() {
    // Avro spells no `uint64`, so the rows land under the row as Iceberg
    // states it - the codes as `decimal(20, 0)` - and read back under the
    // `marketdata` row the service declares.
    let mut holder = Holder::Buffer(
        Buffer::new().with_media_type(Url::from_str("file:///books.avro").unwrap().media_type()),
    );
    let books = BookIterator::new(operations().into_iter(), 0)
        .unwrap()
        .map(|book| book.map(MarketData::from));
    let options = holder.record_options().unwrap().with_field(
        MarketData::field()
            .unwrap()
            .into_scheme_compat(&Scheme::ICEBERG)
            .unwrap(),
    );
    holder
        .overwrite_arrow_reader(
            MarketData::arrow_reader(books, None, None).unwrap(),
            &options,
        )
        .unwrap();
    answers_as_the_arrow_leaf(holder);
}

#[cfg(feature = "parquet")]
#[test]
fn a_parquet_leaf_a_capture_landed_in_answers_every_route() {
    answers_as_the_arrow_leaf(captured_holder("parquet"));
}

#[cfg(feature = "iceberg")]
#[test]
fn an_iceberg_table_a_capture_landed_in_answers_every_route() {
    use yggdryl::iceberg::{FormatVersion, PartitionSpec, Table};
    use yggdryl::local::LocalFolder;

    let path = std::env::temp_dir().join(format!(
        "yggdryl-graph-serve-iceberg-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    Table::create(
        LocalFolder::new(&path).unwrap(),
        FormatVersion::V2,
        MarketData::field()
            .unwrap()
            .into_scheme_compat(&Scheme::ICEBERG)
            .unwrap(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    let mut holder = Holder::folder(&path).unwrap();
    capture(&mut holder);
    answers_as_the_arrow_leaf(holder);
    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn an_empty_or_absent_table_answers_the_empty_reading() {
    let absent =
        std::env::temp_dir().join(format!("yggdryl-graph-serve-absent-{}", std::process::id()));
    let folders =
        std::env::temp_dir().join(format!("yggdryl-graph-serve-empty-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folders);
    // A folder holding nothing, and one holding only a folder: neither
    // holds a leaf, so neither states an encoding to read.
    std::fs::create_dir_all(folders.join("empty")).unwrap();
    std::fs::create_dir_all(folders.join("nested/year=2026")).unwrap();
    let empty = |suffix: &str| {
        Holder::Buffer(
            Buffer::new().with_media_type(
                Url::from_str(&format!("file:///books.{suffix}"))
                    .unwrap()
                    .media_type(),
            ),
        )
    };
    let service = Arc::new(
        BookService::new(BookServiceOptions::new())
            .with_table("arrows", empty("arrows"))
            .with_table("csv", empty("csv"))
            .with_table("absent", Holder::file(absent.join("books.arrows")).unwrap())
            .with_table("absentcsv", Holder::file(absent.join("books.csv")).unwrap())
            .with_table("folder", Holder::folder(folders.join("empty")).unwrap())
            .with_table("nested", Holder::folder(folders.join("nested")).unwrap())
            .with_table(
                "absentfolder",
                Holder::folder(absent.join("books")).unwrap(),
            ),
    );
    let server = Server::bind("127.0.0.1:0").unwrap();
    let endpoint = Arc::clone(&service).route(&server, "/").unwrap();
    let tables = [
        "arrows",
        "csv",
        "absent",
        "absentcsv",
        "folder",
        "nested",
        "absentfolder",
    ];
    for table in tables {
        let tickers = ok_json(&get(&endpoint, "tickers", &[("table", table)]));
        assert!(items(&tickers).is_empty(), "{table}: {tickers:?}");
        let mut asked = range().to_vec();
        asked[0] = ("table", table);
        for leaf in ["candles", "events", "audit.csv"] {
            assert_eq!(
                refused(&get(&endpoint, leaf, &asked), Status::NOT_FOUND),
                format!("expected a ticker at \"{table}/ACME\", got nothing"),
                "{table} {leaf}"
            );
        }
        assert!(
            refused(
                &get(
                    &endpoint,
                    "book",
                    &[
                        ("table", table),
                        ("ticker", "ACME"),
                        ("at", "2026-01-05T10:01:40Z")
                    ]
                ),
                Status::NOT_FOUND
            )
            .starts_with("expected a book at"),
            "{table}"
        );

        // The readings without HTTP answer the same.
        assert!(
            items(&service.tickers(table).unwrap()).is_empty(),
            "{table}"
        );
        assert!(
            service
                .book(table, "ACME", T0 + 100 * SECOND)
                .unwrap()
                .is_none(),
            "{table}"
        );
        let mut asked = query();
        asked.table = table.into();
        assert!(service.candles(&asked).unwrap_err().is_absent(), "{table}");
        assert!(
            service
                .events(&asked)
                .err()
                .is_some_and(|error| error.is_absent()),
            "{table}"
        );
    }
    let _ = std::fs::remove_dir_all(&folders);
}

#[test]
fn a_folder_holding_no_record_is_refused_by_its_encoding() {
    let folder =
        std::env::temp_dir().join(format!("yggdryl-graph-serve-notes-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("notes.bin"), b"\x00\x01").unwrap();
    let service = Arc::new(
        BookService::new(BookServiceOptions::new())
            .with_table("notes", Holder::folder(&folder).unwrap()),
    );
    let server = Server::bind("127.0.0.1:0").unwrap();
    let endpoint = Arc::clone(&service).route(&server, "/").unwrap();

    // What it holds is not the empty reading: no encoding reads it, and the
    // refusal says so.
    let error = refused(
        &get(&endpoint, "tickers", &[("table", "notes")]),
        Status::INTERNAL_SERVER_ERROR,
    );
    assert!(
        error.starts_with("invalid record value at $: expected a record encoding"),
        "{error}"
    );
    let mut asked = range().to_vec();
    asked[0] = ("table", "notes");
    for leaf in ["candles", "events", "audit.csv"] {
        assert_eq!(
            refused(&get(&endpoint, leaf, &asked), Status::INTERNAL_SERVER_ERROR),
            error,
            "{leaf}"
        );
    }
    assert_eq!(service.tickers("notes").unwrap_err().to_string(), error);
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn no_answer_carries_a_credential_a_location_holds() {
    // A port nothing listens on: every read of the table fails in its
    // transport, and the failure names the location it was reading.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let location =
        format!("http://alice:s3cr3t@127.0.0.1:{port}/books.arrows?sv=2020-01-01&sig=SECRETSIG");
    let remote = Holder::from_url(
        &Url::from_str(&location).unwrap(),
        std::iter::empty::<(&str, &str)>(),
    )
    .unwrap();
    let service =
        Arc::new(BookService::new(BookServiceOptions::new()).with_table("remote", remote));
    let server = Server::bind("127.0.0.1:0").unwrap();
    let endpoint = Arc::clone(&service).route(&server, "/").unwrap();
    let secrets = ["alice", "s3cr3t", "SECRETSIG", "sv=2020-01-01"];

    // The location is stated without its user information and its query.
    let answer = get(&endpoint, "tables", &[]);
    let tables = items(&ok_json(&answer));
    assert_eq!(
        text(&tables[0], "url"),
        format!("http://127.0.0.1:{port}/books.arrows")
    );
    let body = answer.text().unwrap();
    assert!(
        !secrets.iter().any(|secret| body.contains(secret)),
        "{body}"
    );

    // A read that fails answers 500, naming the location the same way.
    for (leaf, query) in [
        ("tickers", vec![("table", "remote")]),
        ("candles", {
            let mut asked = range().to_vec();
            asked[0] = ("table", "remote");
            asked
        }),
    ] {
        let answer = get(&endpoint, leaf, &query);
        let error = refused(&answer, Status::INTERNAL_SERVER_ERROR);
        assert!(
            error.contains(&format!("http://127.0.0.1:{port}/books.arrows")),
            "{leaf}: {error}"
        );
        let body = answer.text().unwrap();
        assert!(
            !secrets.iter().any(|secret| body.contains(secret)),
            "{leaf}: {body}"
        );
    }

    // The reading without HTTP is the server's own, and keeps the whole error.
    let error = service.tickers("remote").unwrap_err().to_string();
    assert!(error.contains("alice:s3cr3t@"), "{error}");
}

#[test]
fn tickers_state_the_span_of_a_book_at_either_end_of_the_instants() {
    // `2262-04-11T23:47:16.5Z`: the second after it is past `i64`
    // nanoseconds, so the span closes at the last instant there is.
    let late = 9_223_372_036_500_000_000_i64;
    // `1677-09-21T00:12:43.645224192Z`: its whole second opens before the
    // first instant there is, so the span opens there.
    let early = i64::MIN + 500_000_000;
    let operations = vec![
        quote(early, "EARLY", "E1", Side::Buy, "1", 1),
        quote(late, "LATE", "L1", Side::Buy, "1", 1),
    ];
    let books = BookIterator::new(operations.into_iter(), 0)
        .unwrap()
        .map(|book| book.map(MarketData::from));
    let mut holder = Holder::Buffer(
        Buffer::new().with_media_type(Url::from_str("file:///books.arrows").unwrap().media_type()),
    );
    let options = holder.record_options().unwrap();
    holder
        .overwrite_arrow_reader(
            MarketData::arrow_reader(books, None, None).unwrap(),
            &options,
        )
        .unwrap();
    let (_server, endpoint, _) = running_over(holder);
    let tickers = items(&ok_json(&get(&endpoint, "tickers", &[("table", "books")])));
    assert_eq!(tickers.len(), 2);
    assert_eq!(text(&tickers[0], "ticker"), "EARLY");
    assert_eq!(text(&tickers[0], "from"), "1677-09-21T00:12:43.145224192Z");
    assert_eq!(text(&tickers[0], "to"), "1677-09-21T00:12:44.000000000Z");
    assert_eq!(text(&tickers[1], "ticker"), "LATE");
    assert_eq!(text(&tickers[1], "from"), "2262-04-11T23:47:16.000000000Z");
    assert_eq!(text(&tickers[1], "to"), "2262-04-11T23:47:16.854775807Z");
}

#[test]
fn timezones_list_every_zone_the_tz_parameter_reads() {
    let (_server, endpoint, _) = running("/", BookServiceOptions::new());
    let zones: Vec<String> = items(&ok_json(&get(&endpoint, "timezones", &[])))
        .iter()
        .map(|zone| zone.as_str().unwrap().to_owned())
        .collect();
    assert_eq!(zones[0], "UTC");
    assert!(
        zones[1..].windows(2).all(|pair| pair[0] < pair[1]),
        "sorted, each once: {zones:?}"
    );
    assert!(!zones[1..].iter().any(|zone| zone == "UTC"));
    assert!(zones.iter().any(|zone| zone == "Europe/Zurich"));
    assert_eq!(
        zones.len(),
        1 + Timezone::registered().filter(|zone| !zone.is_utc()).count()
    );
    for zone in &zones {
        let query = format!(
            "table=books&ticker=ACME&from=2026-01-05T10:00:00Z&to=2026-01-05T11:00:00Z&tz={}",
            zone.replace('+', "%2B")
        );
        let read = BookQuery::from_parameters(&Parameters::from_query(&query, true).unwrap())
            .unwrap_or_else(|error| panic!("{zone}: {error}"));
        assert_eq!(read.timezone.as_str(), zone);
    }
}

#[test]
fn events_answer_the_earliest_rows_of_a_store_holding_them_out_of_order() {
    // The fixture's books stored latest first: every bound answers the rows
    // the books stored in order answer, the earliest book's first.
    let mut stored = BookIterator::new(operations().into_iter(), 0)
        .unwrap()
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap();
    stored.reverse();
    let mut holder = Holder::Buffer(
        Buffer::new().with_media_type(Url::from_str("file:///books.arrows").unwrap().media_type()),
    );
    let options = holder.record_options().unwrap();
    holder
        .overwrite_arrow_reader(
            MarketData::arrow_reader(
                stored.into_iter().map(|book| Ok(MarketData::from(book))),
                None,
                None,
            )
            .unwrap(),
            &options,
        )
        .unwrap();
    let (_sorted_server, sorted, _) = running("/", BookServiceOptions::new());
    let (_server, endpoint, _) = running_over(holder);
    let all = items(member(&ok_json(&get(&sorted, "events", &range())), "rows"));
    for limit in 1..=all.len() + 1 {
        let limit = limit.to_string();
        for side in [None, Some("bid"), Some("ask")] {
            let mut asked = range().to_vec();
            asked.push(("limit", &limit));
            if let Some(side) = side {
                asked.push(("side", side));
            }
            assert_eq!(
                ok_json(&get(&endpoint, "events", &asked)),
                ok_json(&get(&sorted, "events", &asked)),
                "limit {limit}, side {side:?}"
            );
        }
    }
    let first = ok_json(&get(
        &endpoint,
        "events",
        &[&range()[..], &[("limit", "1")]].concat(),
    ));
    assert_eq!(member(&first, "truncated"), &Scalar::from(true));
    let rows = items(member(&first, "rows"));
    assert_eq!(text(&rows[0], "bookunix"), "2026-01-05T10:00:05.000000000Z");
}

#[test]
fn tables_lists_every_table_with_its_url() {
    let (_server, endpoint, _) = running("/", BookServiceOptions::new());
    let tables = items(&ok_json(&get(&endpoint, "tables", &[])));
    assert_eq!(tables.len(), 1);
    assert_eq!(text(&tables[0], "name"), "books");
    assert!(
        text(&tables[0], "url").starts_with("mem://"),
        "a buffer names its memory"
    );
}

#[test]
fn tickers_span_the_books_of_each_ticker() {
    let (_server, endpoint, _) = running("/", BookServiceOptions::new());
    let tickers = items(&ok_json(&get(&endpoint, "tickers", &[("table", "books")])));
    assert_eq!(tickers.len(), 2);
    let acme = &tickers[0];
    assert_eq!(text(acme, "ticker"), "ACME");
    assert_eq!(text(acme, "crosscode"), "ACME");
    assert_eq!(text(acme, "from"), "2026-01-05T10:00:05.000000000Z");
    assert_eq!(
        text(acme, "to"),
        "2026-01-05T10:02:06.000000000Z",
        "the second after the last book"
    );
    assert_eq!(member(acme, "books"), &Scalar::from(4_u64));
    let beta = &tickers[1];
    assert_eq!(text(beta, "ticker"), "BETA");
    assert_eq!(text(beta, "from"), "2026-01-05T10:00:30.000000000Z");
    assert_eq!(text(beta, "to"), "2026-01-05T10:01:31.000000000Z");
    assert_eq!(member(beta, "books"), &Scalar::from(2_u64));

    assert_eq!(
        refused(
            &get(&endpoint, "tickers", &[("table", "nope")]),
            Status::NOT_FOUND
        ),
        "expected a table at \"nope\", got nothing"
    );
    assert_eq!(
        refused(&get(&endpoint, "tickers", &[]), Status::BAD_REQUEST),
        "invalid record value at $.table: expected a value, got none"
    );
}

#[test]
fn candles_fold_the_books_of_the_range_by_the_minute() {
    let (_server, endpoint, _) = running("/", BookServiceOptions::new());
    let answer = ok_json(&get(&endpoint, "candles", &range()));
    assert_eq!(text(&answer, "table"), "books");
    assert_eq!(text(&answer, "ticker"), "ACME");
    assert_eq!(text(&answer, "timezone"), "UTC");
    assert_eq!(text(&answer, "interval"), "1m");
    assert_eq!(text(&answer, "from"), "2026-01-05T10:00:00.000000000Z");
    assert_eq!(text(&answer, "to"), "2026-01-05T10:03:00.000000000Z");
    let candles = items(member(&answer, "candles"));
    assert_eq!(candles.len(), 3);

    let first = &candles[0];
    assert_eq!(text(first, "start"), "2026-01-05T10:00:00.000000000Z");
    assert_eq!(text(first, "end"), "2026-01-05T10:01:00.000000000Z");
    let bid = member(first, "bid");
    assert_eq!(text(bid, "open"), "100");
    assert_eq!(text(bid, "close"), "100");
    assert_eq!(text(member(first, "ask"), "open"), "101");
    assert_eq!(text(member(first, "mid"), "close"), "100.5");
    assert_eq!(text(member(first, "spread"), "high"), "1");
    assert_eq!(text(first, "bidqty"), "10");
    assert_eq!(text(first, "askqty"), "5");
    assert_eq!(member(first, "books"), &Scalar::from(1_u64));
    assert_eq!(member(first, "executions"), &Scalar::from(0_u64));
    assert_eq!(text(first, "volume"), "0");

    let second = &candles[1];
    assert_eq!(text(second, "start"), "2026-01-05T10:01:00.000000000Z");
    assert_eq!(text(member(second, "bid"), "open"), "100.5");
    assert_eq!(text(member(second, "bid"), "high"), "100.5");
    assert_eq!(text(member(second, "spread"), "close"), "0.5");
    assert_eq!(member(second, "books"), &Scalar::from(2_u64));
    assert_eq!(member(second, "executions"), &Scalar::from(1_u64));
    assert_eq!(text(second, "volume"), "3");

    let third = &candles[2];
    assert_eq!(text(third, "start"), "2026-01-05T10:02:00.000000000Z");
    assert_eq!(
        text(member(third, "ask"), "close"),
        "101",
        "the best ask stays 101 under the new 103"
    );
    assert_eq!(text(third, "askqty"), "5");
}

#[test]
fn a_one_sided_range_states_no_mid_and_the_to_is_exclusive() {
    let (_server, endpoint, _) = running("/", BookServiceOptions::new());
    let beta = ok_json(&get(
        &endpoint,
        "candles",
        &[
            ("table", "books"),
            ("ticker", "BETA"),
            ("from", "2026-01-05T10:00:00Z"),
            ("to", "2026-01-05T10:01:30Z"),
            ("interval", "30s"),
        ],
    ));
    let candles = items(member(&beta, "candles"));
    // `to` is exclusive: the 10:01:30 book is not in [10:00, 10:01:30).
    assert_eq!(candles.len(), 1);
    assert_eq!(text(&candles[0], "start"), "2026-01-05T10:00:30.000000000Z");
    assert_eq!(text(member(&candles[0], "bid"), "open"), "50");
    assert_eq!(member(&candles[0], "ask"), &Scalar::Null);
    assert_eq!(member(&candles[0], "mid"), &Scalar::Null);
    assert_eq!(member(&candles[0], "spread"), &Scalar::Null);
    assert_eq!(member(&candles[0], "askqty"), &Scalar::Null);

    let both = ok_json(&get(
        &endpoint,
        "candles",
        &[
            ("table", "books"),
            ("ticker", "BETA"),
            ("from", "2026-01-05T10:00:00Z"),
            ("to", "2026-01-05T10:01:31Z"),
            ("interval", "30s"),
        ],
    ));
    let candles = items(member(&both, "candles"));
    assert_eq!(candles.len(), 2);
    assert_eq!(text(member(&candles[1], "mid"), "open"), "51");
}

#[test]
fn candles_align_to_the_zone_asked_and_render_in_it() {
    let (_server, endpoint, _) = running("/", BookServiceOptions::new());
    let answer = ok_json(&get(
        &endpoint,
        "candles",
        &[
            ("table", "books"),
            ("ticker", "ACME"),
            ("from", "2026-01-05T11:00:00"),
            ("to", "2026-01-05T11:03:00"),
            ("tz", "Europe/Zurich"),
            ("interval", "1h"),
        ],
    ));
    assert_eq!(text(&answer, "timezone"), "Europe/Zurich");
    assert_eq!(text(&answer, "interval"), "1h");
    // A naive `from` is a Zurich wall clock: 11:00 there is 10:00Z.
    assert_eq!(
        text(&answer, "from"),
        "2026-01-05T11:00:00.000000000+01:00[Europe/Zurich]"
    );
    let candles = items(member(&answer, "candles"));
    assert_eq!(candles.len(), 1);
    assert_eq!(
        text(&candles[0], "start"),
        "2026-01-05T11:00:00.000000000+01:00[Europe/Zurich]"
    );
    assert_eq!(
        text(&candles[0], "end"),
        "2026-01-05T12:00:00.000000000+01:00[Europe/Zurich]"
    );
    assert_eq!(member(&candles[0], "books"), &Scalar::from(4_u64));
}

#[test]
fn candles_refuse_what_they_cannot_read() {
    let (_server, endpoint, _) = running("/", BookServiceOptions::new());
    let with = |name: &str, value: &str| -> Vec<(&'static str, String)> {
        range()
            .iter()
            .map(|(key, held)| {
                (
                    *key,
                    if *key == name {
                        value.to_owned()
                    } else {
                        (*held).to_owned()
                    },
                )
            })
            .collect()
    };
    let send = |query: Vec<(&'static str, String)>| {
        let query: Vec<(&str, &str)> = query
            .iter()
            .map(|(key, value)| (*key, value.as_str()))
            .collect();
        get(&endpoint, "candles", &query)
    };
    assert_eq!(
        refused(
            &send(with("to", "2026-01-05T10:00:00Z")),
            Status::BAD_REQUEST
        ),
        "invalid record value at $.to: expected an instant after `from` (2026-01-05T10:00:00.000000000Z), got 2026-01-05T10:00:00.000000000Z"
    );
    assert!(
        refused(&send(with("from", "10 o'clock")), Status::BAD_REQUEST)
            .starts_with("invalid record value at $.from:")
    );
    assert_eq!(
        refused(&send(with("ticker", "NONE")), Status::NOT_FOUND),
        "expected a ticker at \"books/NONE\", got nothing"
    );
    assert_eq!(
        refused(&send(with("table", "nope")), Status::NOT_FOUND),
        "expected a table at \"nope\", got nothing"
    );
    let mut bad_interval = range().to_vec();
    bad_interval.push(("interval", "soon"));
    assert!(
        refused(
            &get(&endpoint, "candles", &bad_interval),
            Status::BAD_REQUEST
        )
        .starts_with("invalid record value at $.interval:")
    );
    let mut bad_zone = range().to_vec();
    bad_zone.push(("tz", "Nowhere/Here"));
    assert!(
        refused(&get(&endpoint, "candles", &bad_zone), Status::BAD_REQUEST)
            .starts_with("invalid record value at $.tz:")
    );
    let mut empty = range().to_vec();
    empty[3] = ("to", "2026-01-05T10:00:01Z");
    let answer = ok_json(&get(&endpoint, "candles", &empty));
    assert!(
        items(member(&answer, "candles")).is_empty(),
        "a known ticker with no book in range answers no candle"
    );
}

#[test]
fn the_book_at_an_instant_is_the_last_at_or_before_it() {
    let (_server, endpoint, _) = running("/", BookServiceOptions::new());
    let book = ok_json(&get(
        &endpoint,
        "book",
        &[
            ("table", "books"),
            ("ticker", "ACME"),
            ("at", "2026-01-05T10:01:40Z"),
        ],
    ));
    assert_eq!(text(&book, "currunix"), "2026-01-05T10:01:10.000000000Z");
    assert_eq!(text(&book, "ticker"), "ACME");
    assert_eq!(text(&book, "crosscode"), "ACME");
    assert_eq!(text(&book, "bestbid"), "100.5");
    assert_eq!(text(&book, "bestask"), "101");
    assert_eq!(text(&book, "bidqty"), "4");
    assert_eq!(text(&book, "askqty"), "5");
    assert_eq!(text(&book, "spread"), "0.5");
    assert_eq!(text(&book, "midpoint"), "100.75");
    assert_eq!(text(&book, "imbalance"), "-0.111111111111111111");
    assert_eq!(member(&book, "islocked"), &Scalar::from(false));
    assert_eq!(member(&book, "iscrossed"), &Scalar::from(false));
    assert_eq!(member(&book, "alive"), &Scalar::from(3_u64));
    assert_eq!(member(&book, "executions"), &Scalar::from(1_u64));
    let bids = items(member(&book, "bidlimits"));
    assert_eq!(bids.len(), 2);
    assert_eq!(text(&bids[0], "price"), "100.5");
    assert_eq!(text(&bids[0], "quantity"), "4");
    assert_eq!(items(member(&bids[0], "uuids")).len(), 1);
    assert_eq!(member(&bids[0], "tradable"), &Scalar::from(true));
    assert_eq!(text(&bids[1], "price"), "100");
    let asks = items(member(&book, "asklimits"));
    assert_eq!(asks.len(), 1);
    assert_eq!(text(&asks[0], "quantity"), "5");

    // The instant as a Zurich wall clock, rendered back in Zurich.
    let zoned = ok_json(&get(
        &endpoint,
        "book",
        &[
            ("table", "books"),
            ("ticker", "ACME"),
            ("at", "2026-01-05T11:00:05"),
            ("tz", "Europe/Zurich"),
        ],
    ));
    assert_eq!(
        text(&zoned, "currunix"),
        "2026-01-05T11:00:05.000000000+01:00[Europe/Zurich]"
    );
    assert_eq!(member(&zoned, "alive"), &Scalar::from(2_u64));

    assert_eq!(
        refused(
            &get(
                &endpoint,
                "book",
                &[
                    ("table", "books"),
                    ("ticker", "ACME"),
                    ("at", "2026-01-05T10:00:01Z")
                ]
            ),
            Status::NOT_FOUND
        ),
        "expected a book at \"books/ACME at or before 2026-01-05T10:00:01.000000000Z\", got nothing"
    );
    assert!(
        refused(
            &get(&endpoint, "book", &[("table", "books"), ("ticker", "ACME")]),
            Status::BAD_REQUEST
        )
        .starts_with("invalid record value at $.at:")
    );
}

#[test]
fn events_list_every_entry_delta_and_execution_of_the_books_in_range() {
    let (_server, endpoint, _) = running("/", BookServiceOptions::new());
    let answer = ok_json(&get(&endpoint, "events", &range()));
    assert_eq!(member(&answer, "truncated"), &Scalar::from(false));
    let rows = items(member(&answer, "rows"));
    let roles: BTreeSet<String> = rows.iter().map(|row| text(row, "role")).collect();
    assert_eq!(
        roles,
        BTreeSet::from([
            "alive".to_owned(),
            "delta".to_owned(),
            "execution".to_owned()
        ])
    );
    assert!(rows.iter().all(|row| text(row, "ticker") == "ACME"));
    let instants: BTreeSet<String> = rows.iter().map(|row| text(row, "bookunix")).collect();
    assert_eq!(
        instants,
        BTreeSet::from([
            "2026-01-05T10:00:05.000000000Z".to_owned(),
            "2026-01-05T10:01:05.000000000Z".to_owned(),
            "2026-01-05T10:01:10.000000000Z".to_owned(),
            "2026-01-05T10:02:05.000000000Z".to_owned(),
        ])
    );
    let executions: Vec<&Scalar> = rows
        .iter()
        .filter(|row| text(row, "role") == "execution")
        .collect();
    assert_eq!(executions.len(), 1);
    assert_eq!(
        text(executions[0], "bookunix"),
        "2026-01-05T10:01:10.000000000Z"
    );
    assert_eq!(text(executions[0], "side"), "SELL");
    assert_eq!(text(executions[0], "marketdatakind"), "EXEC");
    assert_eq!(text(executions[0], "quantity"), "3");
    let first_book: Vec<&Scalar> = rows
        .iter()
        .filter(|row| text(row, "bookunix") == "2026-01-05T10:00:05.000000000Z")
        .collect();
    // Two entries alive and the two deltas that placed them.
    assert_eq!(first_book.len(), 4);
    assert_eq!(
        first_book
            .iter()
            .filter(|row| text(row, "role") == "alive")
            .count(),
        2
    );
    let names: BTreeSet<&str> = rows[0]
        .as_struct()
        .unwrap()
        .keys()
        .map(SmolStr::as_str)
        .collect();
    assert!(names.contains("curruuid") && names.contains("prevuuid") && names.contains("state"));
    assert!(!names.contains("alive") && !names.contains("bidlimits"));
    assert_eq!(names.len(), 51);
    // A UUID is its canonical text.
    assert_eq!(text(&rows[0], "curruuid").len(), 36);

    let mut bids = range().to_vec();
    bids.push(("side", "bid"));
    let bids = items(member(&ok_json(&get(&endpoint, "events", &bids)), "rows"));
    assert!(!bids.is_empty());
    assert!(bids.iter().all(|row| text(row, "side") == "BUYS"));
    assert!(bids.iter().all(|row| text(row, "role") != "execution"));

    let mut asks = range().to_vec();
    asks.push(("side", "ASK"));
    let asks = items(member(&ok_json(&get(&endpoint, "events", &asks)), "rows"));
    assert!(asks.iter().all(|row| text(row, "side") == "SELL"));
    assert_eq!(
        asks.iter()
            .filter(|row| text(row, "role") == "execution")
            .count(),
        1
    );
    assert_eq!(bids.len() + asks.len(), rows.len());

    let mut zoned = range().to_vec();
    zoned.push(("tz", "Europe/Zurich"));
    let zoned = items(member(&ok_json(&get(&endpoint, "events", &zoned)), "rows"));
    assert_eq!(
        text(&zoned[0], "bookunix"),
        "2026-01-05T11:00:05.000000000+01:00[Europe/Zurich]"
    );
    assert_eq!(
        text(&zoned[0], "currunix"),
        "2026-01-05T11:00:05.000000000+01:00[Europe/Zurich]"
    );
}

#[test]
fn events_are_bounded_by_the_limit_and_the_options() {
    let (_server, endpoint, _) = running("/", BookServiceOptions::new().with_max_event_rows(6));
    let mut two = range().to_vec();
    two.push(("limit", "2"));
    let answer = ok_json(&get(&endpoint, "events", &two));
    assert_eq!(items(member(&answer, "rows")).len(), 2);
    assert_eq!(member(&answer, "truncated"), &Scalar::from(true));

    let answer = ok_json(&get(&endpoint, "events", &range()));
    assert_eq!(
        items(member(&answer, "rows")).len(),
        6,
        "the options bound a query stating no limit"
    );
    assert_eq!(member(&answer, "truncated"), &Scalar::from(true));

    let mut many = range().to_vec();
    many.push(("limit", "1000"));
    let answer = ok_json(&get(&endpoint, "events", &many));
    assert_eq!(
        items(member(&answer, "rows")).len(),
        6,
        "a limit above the options' is the options'"
    );

    let mut bad = range().to_vec();
    bad.push(("limit", "some"));
    assert!(
        refused(&get(&endpoint, "events", &bad), Status::BAD_REQUEST)
            .starts_with("invalid record value at $.limit:")
    );
}

#[test]
fn the_audit_downloads_as_csv_in_each_coding_and_reads_back() {
    let (_server, endpoint, service) = running("/", BookServiceOptions::new());
    let expected: usize = service
        .events(&query())
        .unwrap()
        .map(|batch| batch.unwrap().num_rows())
        .sum();
    assert!(expected > 0);
    for (suffix, content_type, coding) in [
        ("csv", "text/csv", None),
        ("csv.gz", "application/gzip", Some(MimeType::GZIP)),
        ("csv.zst", "application/zstd", Some(MimeType::ZSTD)),
    ] {
        let response = get(&endpoint, &format!("audit.{suffix}"), &range());
        assert_eq!(
            response.status(),
            Status::OK,
            "{}",
            response.text().unwrap()
        );
        assert_eq!(
            response.headers().get("content-type"),
            Some(content_type),
            "{suffix}"
        );
        assert_eq!(response.headers().get("cache-control"), Some("no-store"));
        assert_eq!(
            response.headers().get("content-disposition"),
            Some(
                format!(
                    "attachment; filename=\"audit-ACME-20260105T100000Z-20260105T100300Z.{suffix}\""
                )
                .as_str()
            )
        );
        let body = response.bytes().unwrap().to_vec();
        let decoded = match coding {
            Some(coding) => Codec::from_mime_type(&coding).load(&body).unwrap(),
            None => body.clone(),
        };
        let header = std::str::from_utf8(&decoded)
            .unwrap()
            .lines()
            .next()
            .unwrap()
            .to_owned();
        assert!(
            header.starts_with("bookunix,role,marketdatakind,currunix,"),
            "{suffix}: {header}"
        );

        // The bytes read back through the CSV medium under the suffix's own coding.
        let readback = Buffer::from_bytes(body).with_media_type(
            Url::from_str(&format!("file:///audit.{suffix}"))
                .unwrap()
                .media_type(),
        );
        let options = readback.record_options().unwrap();
        let rows: usize = readback
            .read_arrow_reader(&options)
            .unwrap()
            .map(|batch| batch.unwrap().num_rows())
            .sum();
        assert_eq!(rows, expected, "{suffix}");
        let field = readback.read_arrow_field(&options).unwrap();
        let names: Vec<&str> = field.fields().iter().map(|child| child.name()).collect();
        assert_eq!(&names[..3], ["bookunix", "role", "marketdatakind"]);
        assert_eq!(names.len(), 51);
    }

    let mut bids = range().to_vec();
    bids.push(("side", "bid"));
    let lines = get(&endpoint, "audit.csv", &bids)
        .text()
        .unwrap()
        .lines()
        .count();
    let json = items(member(&ok_json(&get(&endpoint, "events", &bids)), "rows"));
    assert_eq!(lines, json.len() + 1, "the header and one line per bid row");

    assert_eq!(
        refused(
            &get(
                &endpoint,
                "audit.csv.gz",
                &[
                    ("table", "books"),
                    ("ticker", "NONE"),
                    ("from", "2026-01-05T10:00:00Z"),
                    ("to", "2026-01-05T10:03:00Z")
                ]
            ),
            Status::NOT_FOUND
        ),
        "expected a ticker at \"books/NONE\", got nothing"
    );
    assert!(
        refused(
            &get(&endpoint, "audit.csv", &[("table", "books")]),
            Status::BAD_REQUEST
        )
        .starts_with("invalid record value at $.ticker:")
    );
}

#[test]
fn a_prefix_routes_the_api_under_it_and_nothing_else_is_answered() {
    let (server, endpoint, _) = running("/books", BookServiceOptions::new());
    assert!(endpoint.to_string().ends_with("/books"), "{endpoint}");
    let tables = items(&ok_json(&get(&endpoint, "tables", &[])));
    assert_eq!(text(&tables[0], "name"), "books");
    let elsewhere = Request::get(&server.url_of("/api/tables").unwrap().to_string())
        .unwrap()
        .send()
        .unwrap();
    assert_eq!(elsewhere.status(), Status::NOT_FOUND);
    let unknown = Request::get(&server.url_of("/books/api/nothing").unwrap().to_string())
        .unwrap()
        .send()
        .unwrap();
    assert_eq!(unknown.status(), Status::NOT_FOUND);
    let posted = Request::post(&format!("{endpoint}/api/tables"), "")
        .unwrap()
        .send()
        .unwrap();
    assert_eq!(posted.status(), Status::METHOD_NOT_ALLOWED);
    assert!(
        Arc::new(BookService::new(BookServiceOptions::new()))
            .route(&server, "/x?y")
            .is_err()
    );
}
