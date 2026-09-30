//! `BookService`: the HTTP face of a market-data table - the tickers a table
//! holds, the candles a ticker's books fold into, the book standing at an
//! instant and the audit of every entry, delta and execution of the books in
//! a range - as JSON for a display and as CSV for a download.
//!
//! A table is any record location a [`Holder`] reads books from: an Iceberg
//! folder, an Arrow, Parquet or CSV leaf, a partitioned folder. Every reading
//! is one filtered read of it - `marketdatakind = 'BOOK'`, the ticker, the
//! instants - pushed into the location's own record options, under the
//! `marketdata` row where the location declares none, so a store that prunes
//! on them prunes, a medium that states no nested type reads its cells as
//! that row types them, an empty or absent store is the empty reading, and
//! the `BOOK` rows come back as [`BookEvent`]s through
//! [`MarketData::from_arrow_reader`]. Each reading is
//! also answered without HTTP ([`BookService::tickers`],
//! [`BookService::candles`], [`BookService::book`],
//! [`BookService::events`]), so a test, a binding and the CLI reach the same
//! answers the routes do; the routes only read the query, call one and spell
//! its answer.
//!
//! ```no_run
//! use std::sync::Arc;
//! use yggdryl::graph::{BookService, BookServiceOptions};
//! use yggdryl::holder::Holder;
//! use yggdryl::http::Server;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let service = BookService::new(BookServiceOptions::new())
//!     .with_table("books", Holder::folder("/data/books")?);
//! let server = Server::bind("127.0.0.1:8080")?;
//! let endpoint = Arc::new(service).route(&server, "/")?;
//! println!("GET {endpoint}api/tables");
//! # Ok(())
//! # }
//! ```

use std::collections::{BTreeMap, BinaryHeap};
use std::sync::Arc;

use arrow_array::{RecordBatch, RecordBatchReader};
use arrow_schema::{ArrowError, SchemaRef};
use smol_str::{SmolStr, format_smolstr};

use super::arrow::{ALIVE, ASKLIMITS, BIDLIMITS, DELTAS, EXECUTIONS, MARKETDATAKIND};
use super::{
    BookEvent, Candle, CandleIterator, CandleOptions, Element, Event, EventColumn, ExecutionEvent,
    Market, MarketColumn, MarketData, Ohlc,
};
use crate::arrow::BatchReader;
use crate::expression::{Filter, Plan, Projection, Selector, Term};
use crate::holder::{Buffer, Holder};
use crate::http::{Method, Request, Response, Server, Status};
use crate::media::{IORecordOptions, RecordOptions};
use crate::text::expected_got;
use crate::{
    ArrowCastOptions, DataType, Decimal, Error, Field, IOBase, IOMedia, MarketDataKind, MimeType,
    Parameters, Result, Scalar, Serie, SerieReader, Side, StructType, TimeUnit, Timezone, Url,
};

/// The segment every route stands under: `{prefix}/api/<leaf>`.
const API: &str = "api";

/// Every nested column of the `marketdata` row: what an audit row drops.
const NESTED: [&str; 5] = [ALIVE, DELTAS, EXECUTIONS, BIDLIMITS, ASKLIMITS];

/// The interval candles are bucketed by when the query states none.
const DEFAULT_INTERVAL: &str = "1m";

/// The header every answer carries: a reading is of the table as it stands.
const NO_STORE: (&str, &str) = ("cache-control", "no-store");

/// The query parameters a route reads; a refusal located at `$.<one of
/// these>` is the caller's, answered `400`.
const PARAMETERS: [&str; 9] = [
    "table", "ticker", "from", "to", "at", "tz", "interval", "side", "limit",
];

/// The first two columns of an audit row: the book's instant and the role
/// the row plays in it.
const BOOKUNIX: &str = "bookunix";
const ROLE: &str = "role";

/// The three roles an audit row plays in its book.
const ROLE_ALIVE: &str = "alive";
const ROLE_DELTA: &str = "delta";
const ROLE_EXECUTION: &str = "execution";

/// The three codings an audit downloads under, by the suffix that names each.
const AUDIT_SUFFIXES: [&str; 3] = ["csv", "csv.gz", "csv.zst"];

/// Nanoseconds in one second.
const NANOS: i64 = 1_000_000_000;

/// One reading of a route: the query it was given and the answer it spells.
type Reading = fn(&BookService, &Parameters<'_>) -> Result<Response>;

/// The routes under `{prefix}/api`, each the leaf it answers at and the
/// reading that answers it.
const ROUTES: [(&str, Reading); 9] = [
    ("tables", BookService::answer_tables),
    ("timezones", BookService::answer_timezones),
    ("tickers", BookService::answer_tickers),
    ("candles", BookService::answer_candles),
    ("book", BookService::answer_book),
    ("events", BookService::answer_events),
    ("audit.csv", BookService::answer_audit_csv),
    ("audit.csv.gz", BookService::answer_audit_gzip),
    ("audit.csv.zst", BookService::answer_audit_zstd),
];

/// What a [`BookService`] states about itself.
///
/// ```
/// use yggdryl::graph::BookServiceOptions;
///
/// let options = BookServiceOptions::new().with_max_event_rows(100);
/// assert_eq!(options.snapshot_millis(), 0);
/// assert_eq!(options.max_event_rows(), 100);
/// assert_eq!(BookServiceOptions::default().max_event_rows(), BookServiceOptions::DEFAULT_MAX_EVENT_ROWS);
/// ```
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct BookServiceOptions {
    snapshot_millis: u64,
    max_event_rows: u64,
}

impl Default for BookServiceOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl BookServiceOptions {
    /// The most rows `/api/events` answers when the query states no `limit`,
    /// and the most it answers whatever the query states.
    pub const DEFAULT_MAX_EVENT_ROWS: u64 = 5_000;

    /// No snapshot grid, and [`Self::DEFAULT_MAX_EVENT_ROWS`] event rows.
    #[must_use]
    pub fn new() -> Self {
        Self {
            snapshot_millis: 0,
            max_event_rows: Self::DEFAULT_MAX_EVENT_ROWS,
        }
    }

    /// The same options stating the snapshot grid, milliseconds, the books a
    /// capture is folded on before it lands in a table - what `yggdryl market serve
    /// --snapshot-millis` states and hands to
    /// [`BookIterator::new`](super::BookIterator::new); zero is no grid. The
    /// service reads books already folded and re-folds nothing.
    #[must_use]
    pub fn with_snapshot_millis(mut self, snapshot_millis: u64) -> Self {
        self.snapshot_millis = snapshot_millis;
        self
    }

    /// The same options bounding `/api/events` at `max_event_rows` rows, at
    /// least one; the CSV audit is never bounded.
    #[must_use]
    pub fn with_max_event_rows(mut self, max_event_rows: u64) -> Self {
        self.max_event_rows = max_event_rows.max(1);
        self
    }

    /// The snapshot grid, milliseconds; zero is none.
    #[must_use]
    pub fn snapshot_millis(&self) -> u64 {
        self.snapshot_millis
    }

    /// The most rows `/api/events` answers.
    #[must_use]
    pub fn max_event_rows(&self) -> u64 {
        self.max_event_rows
    }
}

/// One table the service reads books from: a name and the location holding
/// the `marketdata` rows.
#[derive(Debug)]
pub struct BookTable {
    name: String,
    holder: Holder,
}

impl BookTable {
    /// The name every route's `table` parameter states.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The location the books are read from.
    #[must_use]
    pub fn holder(&self) -> &Holder {
        &self.holder
    }
}

/// One question about one ticker's books over one range of instants: what
/// `/api/candles`, `/api/events` and the audit routes read off their query,
/// and what [`BookService::candles`] and [`BookService::events`] answer.
///
/// `from` and `to` are nanoseconds since the epoch, UTC, `to` exclusive.
/// `timezone` is the zone instants are rendered in and a naive instant is
/// read in; `interval` is how candles are bucketed, the default a minute in
/// that zone ([`Self::candle_options`]); `side` keeps the audit to the bid or
/// the ask, `None` both.
///
/// ```
/// use yggdryl::Parameters;
/// use yggdryl::graph::BookQuery;
///
/// # fn main() -> yggdryl::Result<()> {
/// let parameters = Parameters::from_query(
///     "table=books&ticker=ACME&from=2026-01-05T10:00:00&to=2026-01-05T11:00:00&tz=Europe/Zurich&interval=5m&side=bid",
///     true,
/// )?;
/// let query = BookQuery::from_parameters(&parameters)?;
/// assert_eq!(query.ticker, "ACME");
/// // A naive instant is a wall clock in `tz`: 10:00 in Zurich is 09:00Z.
/// assert_eq!(query.from, 1_767_603_600 * 1_000_000_000);
/// assert_eq!(query.to - query.from, 3_600 * 1_000_000_000);
/// assert_eq!(query.candle_options()?.spelling(), "5m");
/// assert_eq!(query.side, Some(yggdryl::Side::Buy));
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct BookQuery {
    /// The table's name ([`BookTable::name`]).
    pub table: String,
    /// The ticker the books state.
    pub ticker: String,
    /// The first instant, nanoseconds UTC, inclusive.
    pub from: i64,
    /// The last instant, nanoseconds UTC, exclusive.
    pub to: i64,
    /// The zone instants are rendered in and a naive instant is read in.
    pub timezone: Timezone,
    /// How candles are bucketed; `None` is a minute in `timezone`.
    pub interval: Option<CandleOptions>,
    /// The side the audit keeps; `None` keeps both.
    pub side: Option<Side>,
}

impl BookQuery {
    /// Reads a query off its parameters: `table` and `ticker` required,
    /// `from` and `to` required ISO-8601 instants - one stating an offset or
    /// `Z` is that instant, a naive one a wall clock in `tz` - `tz` an IANA
    /// zone this build knows (default `UTC`), `interval` a candle spelling
    /// (`30s`, `5m`, `1h`, `1d`), `side` `bid` or `ask`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] located at `$.<parameter>` for one
    /// missing or unreadable, at `$.to` when `from` is not before `to`, at
    /// `$.tz` for a zone this build has no rules for.
    pub fn from_parameters(parameters: &Parameters<'_>) -> Result<Self> {
        let table = required(parameters, "table")?.to_owned();
        let ticker = required(parameters, "ticker")?.to_owned();
        let timezone = zone(parameters)?;
        let from = instant(parameters, "from", timezone)?;
        let to = instant(parameters, "to", timezone)?;
        let interval = parameters
            .get("interval")
            .map(|text| {
                CandleOptions::from_spelling(text).map(|options| options.with_timezone(timezone))
            })
            .transpose()?;
        let side = parameters.get("side").map(side).transpose()?;
        let query = Self {
            table,
            ticker,
            from,
            to,
            timezone,
            interval,
            side,
        };
        query.span_checked()?;
        Ok(query)
    }

    /// The options candles are bucketed by: `interval` where stated, else a
    /// minute aligned to `timezone`.
    ///
    /// # Errors
    ///
    /// Returns an error when the default interval cannot be built, which it
    /// never is.
    pub fn candle_options(&self) -> Result<CandleOptions> {
        match &self.interval {
            Some(options) => Ok(options.clone()),
            None => {
                Ok(CandleOptions::from_spelling(DEFAULT_INTERVAL)?.with_timezone(self.timezone))
            }
        }
    }

    /// `from` before `to`, or the refusal at `$.to`.
    fn span_checked(&self) -> Result<()> {
        if self.from >= self.to {
            return Err(refused(
                "to",
                expected_got(
                    format_smolstr!(
                        "an instant after `from` ({})",
                        iso(self.from, Timezone::UTC)
                    ),
                    iso(self.to, Timezone::UTC),
                ),
            ));
        }
        Ok(())
    }

    /// The rows of this ticker whose instant lies in `[from, to)`.
    fn filter(&self) -> Result<Filter> {
        self.window(None)
    }

    /// The rows of this ticker whose instant lies in `[from, to)` and, where
    /// `last` states one, at or before `last`.
    fn window(&self, last: Option<i64>) -> Result<Filter> {
        let currunix = || Term::column(EventColumn::CurrUnix.name());
        let mut terms = vec![
            category(),
            of_ticker(&self.ticker),
            currunix().ge(literal_instant(self.from)?),
            currunix().lt(literal_instant(self.to)?),
        ];
        if let Some(last) = last {
            terms.push(currunix().le(literal_instant(last)?));
        }
        Ok(Filter::all(terms.into_iter().map(Filter::from)))
    }
}

/// The HTTP face of one or more tables of books.
///
/// Routes under `{prefix}/api`, all `GET`, every answer stating
/// `Content-Type` and `Cache-Control: no-store`; a refusal is JSON
/// `{"error": "<text>"}` - `400` for a parameter, `404` for a table, a
/// ticker or a book there is none of, `500` otherwise, its whole error
/// written to standard error, the server's own log:
///
/// | Route | Parameters | Answer |
/// | --- | --- | --- |
/// | `tables` | | `[{"name","url"}]`, each location without its user information and its query |
/// | `timezones` | | `["UTC", ..]`: `UTC`, then every zone this build has rules for ([`Timezone::registered`]), by name - the zones `tz` reads |
/// | `tickers` | `table` | `[{"ticker","crosscode","from","to","books"}]`, ordered by ticker, `from` the first book's second and `to` the second after the last, so `[from, to)` holds every book |
/// | `candles` | `table`, `ticker`, `from`, `to`, `tz`, `interval` | `{"table","ticker","timezone","interval","from","to","candles":[..]}`, each candle `{start,end,bid,ask,mid,spread,bidqty,askqty,books,executions,volume}` with each reading `{open,high,low,close}` or null |
/// | `book` | `table`, `ticker`, `at`, `tz` | the last book at or before `at`: `{currunix,ticker,crosscode,bestbid,bestask,bidqty,askqty,spread,midpoint,imbalance,islocked,iscrossed,alive,deltas,executions,bidlimits,asklimits}`, the counts of its entries and each side's `[{price,quantity,uuids,tradable}]` |
/// | `events` | `table`, `ticker`, `from`, `to`, `tz`, `side`, `limit` | `{"rows":[..],"truncated":bool}`, one row per alive entry, delta and execution of every book in range, at most `limit` (default and cap [`BookServiceOptions::max_event_rows`]) |
/// | `audit.csv`, `audit.csv.gz`, `audit.csv.zst` | `table`, `ticker`, `from`, `to`, `tz`, `side` | the same rows unbounded, written by the CSV medium under the suffix's coding, `Content-Disposition: attachment` |
///
/// Instants in an answer are ISO-8601 text in `tz`, decimals text, UUIDs
/// their canonical text. No answer carries what authenticates a location -
/// the user information before its host, the query after its path, where a
/// password or a signature travels - which every table's listing and every
/// refusal's text are stated without. An audit row is [`Self::events_field`]: the book's
/// instant, the row's role (`alive`, `delta`, `execution`) and every flat
/// column of [`MarketData::field`]. The readings behind the routes are
/// public, so what a route answers is exactly what [`Self::tickers`],
/// [`Self::candles`], [`Self::book`] and [`Self::events`] answer.
///
/// ```
/// use std::sync::Arc;
/// use yggdryl::graph::{
///     BookIterator, BookQuery, BookService, BookServiceOptions, Element, Event, Market,
///     MarketData, QuoteEvent,
/// };
/// use yggdryl::holder::{Buffer, Holder};
/// use yggdryl::http::{Request, Server, Status};
/// use yggdryl::{Decimal, IOMedia, Side, State, Timezone, Url};
///
/// # fn main() -> yggdryl::Result<()> {
/// let quote = |unix: i64, side: Side, price: i64| -> MarketData {
///     let mut quote = QuoteEvent::at(unix);
///     quote.set_crosscode(format!("Q-{unix}-{}", side.as_str()));
///     quote.set_ticker(Some("ACME".into()));
///     quote.set_side(side);
///     quote.set_price(Some(Decimal::from_int(price)));
///     quote.set_quantity(Some(Decimal::from_int(10)));
///     quote.set_state(State::New);
///     quote.finalize();
///     MarketData::from(quote)
/// };
/// let second = 1_000_000_000;
/// let books = BookIterator::new(
///     vec![quote(10 * second, Side::Buy, 100), quote(70 * second, Side::Sell, 102)].into_iter(),
///     0,
/// )?
/// .map(|book| book.map(MarketData::from));
/// let mut holder = Holder::Buffer(
///     Buffer::new().with_media_type(Url::from_str("file:///books.arrows")?.media_type()),
/// );
/// let options = holder.record_options()?;
/// holder.overwrite_arrow_reader(MarketData::arrow_reader(books, None, None)?, &options)?;
///
/// let service = Arc::new(BookService::new(BookServiceOptions::new()).with_table("books", holder));
/// let query = BookQuery {
///     table: "books".into(),
///     ticker: "ACME".into(),
///     from: 0,
///     to: 120 * second,
///     timezone: Timezone::UTC,
///     interval: None,
///     side: None,
/// };
/// let candles = service.candles(&query)?;
/// assert_eq!(candles.len(), 2);
/// assert_eq!(candles[0].bid.map(|bid| bid.close), Some(Decimal::from_int(100)));
/// assert_eq!(candles[1].ask.map(|ask| ask.open), Some(Decimal::from_int(102)));
///
/// let server = Server::bind("127.0.0.1:0")?;
/// let endpoint = Arc::clone(&service).route(&server, "/")?;
/// let answer = Request::get(&format!("{endpoint}api/tickers?table=books"))?.send()?;
/// assert_eq!(answer.status(), Status::OK);
/// assert!(answer.text()?.contains("\"ticker\":\"ACME\""));
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct BookService {
    options: BookServiceOptions,
    tables: Vec<BookTable>,
}

impl BookService {
    /// A service over no table yet.
    #[must_use]
    pub fn new(options: BookServiceOptions) -> Self {
        Self {
            options,
            tables: Vec::new(),
        }
    }

    /// The same service reading `holder` as the table `name`; a name already
    /// listed is replaced.
    #[must_use]
    pub fn with_table(mut self, name: impl Into<String>, holder: Holder) -> Self {
        let name = name.into();
        self.tables.retain(|table| table.name != name);
        self.tables.push(BookTable { name, holder });
        self
    }

    /// The tables, in the order they were added.
    #[must_use]
    pub fn tables(&self) -> &[BookTable] {
        &self.tables
    }

    /// The options the service was built with.
    #[must_use]
    pub fn options(&self) -> &BookServiceOptions {
        &self.options
    }

    /// The row an audit states: the required struct `event` of `bookunix:
    /// datetime64(ns, UTC) not null`, `role: utf8 not null`, then every flat
    /// column of [`MarketData::field`] - `marketdatakind`, the event, market
    /// and operation columns and `bookscope` - each as that field declares
    /// it.
    ///
    /// # Errors
    ///
    /// Returns an error when a field cannot be built.
    pub fn events_field() -> Result<Field> {
        let root = MarketData::field()?;
        let mut fields = Vec::with_capacity(2 + root.field_len());
        fields.push(
            DataType::datetime64(TimeUnit::Nanosecond, Timezone::UTC)?.required_field(BOOKUNIX),
        );
        fields.push(DataType::utf8().required_field(ROLE));
        fields.extend(
            root.fields()
                .iter()
                .filter(|field| !NESTED.contains(&field.name()))
                .cloned(),
        );
        Ok(DataType::Struct(StructType::from_fields(fields)?).required_field("event"))
    }

    /// Answers the routes under `{prefix}/api` on `server` and returns the
    /// endpoint - `prefix` resolved against the server's own URL
    /// ([`Server::url_of`]); [`Server::public_url_of`] is the one clients
    /// behind a proxy reach. Routing the same prefix again replaces the
    /// service that answered there.
    ///
    /// # Errors
    ///
    /// A `prefix` the server cannot route: one carrying a query, a fragment
    /// or a control byte.
    pub fn route(self: Arc<Self>, server: &Server, prefix: &str) -> Result<Url> {
        let prefix = crate::http::server::normalize_path(prefix)?;
        let endpoint = server.url_of(&prefix)?;
        let base = if prefix == "/" { "" } else { prefix.as_str() };
        for (leaf, reading) in ROUTES {
            let service = Arc::clone(&self);
            server.route(
                Some(Method::Get),
                &format!("{base}/{API}/{leaf}"),
                move |request| Ok(service.answer(request, reading)),
            );
        }
        Ok(endpoint)
    }

    /// The tickers `table` holds books of, ordered by ticker: a sequence of
    /// `{ticker, crosscode, from, to, books}` structs, `from` the whole
    /// second the first book stands in and `to` the whole second after the
    /// last - both `datetime64(ns, UTC)`, the first or the last instant
    /// `i64` nanoseconds hold where that second lies past them - and `books`
    /// how many. A book stating no ticker is not listed, since no query can
    /// name it; an empty or absent table lists none.
    ///
    /// One projected scan: `select ticker, crosscode, currunix where
    /// marketdatakind = 'BOOK'`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Absent`] for a table the service does not hold, and
    /// what reading the table refuses.
    pub fn tickers(&self, table: &str) -> Result<Scalar> {
        let spans = Self::spans(self.table(table)?)?;
        let listed = spans
            .into_iter()
            .map(|(ticker, span)| {
                Ok(object([
                    ("ticker", Scalar::from(ticker)),
                    ("crosscode", Scalar::from(span.crosscode)),
                    (
                        "from",
                        zoned(
                            span.from
                                .div_euclid(NANOS)
                                .checked_mul(NANOS)
                                .unwrap_or(i64::MIN),
                            Timezone::UTC,
                        )?,
                    ),
                    (
                        "to",
                        zoned(
                            span.to
                                .div_euclid(NANOS)
                                .checked_add(1)
                                .and_then(|second| second.checked_mul(NANOS))
                                .unwrap_or(i64::MAX),
                            Timezone::UTC,
                        )?,
                    ),
                    ("books", Scalar::from(span.books)),
                ]))
            })
            .collect::<Result<Vec<Scalar>>>()?;
        Ok(Scalar::from_sequence(listed))
    }

    /// The candles the books of `query.ticker` in `[query.from, query.to)`
    /// fold into under [`BookQuery::candle_options`], through
    /// [`CandleIterator`] over the books sorted by their instant.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] at `$.to` when `from` is not before
    /// `to`, [`Error::Absent`] for a table the service does not hold or a
    /// ticker the table holds no book of, and what reading or folding the
    /// books refuses.
    pub fn candles(&self, query: &BookQuery) -> Result<Vec<Candle>> {
        query.span_checked()?;
        let table = self.table(&query.table)?;
        let books = Self::books(table, &query.filter()?)?;
        if books.is_empty() {
            Self::ticker_known(table, &query.ticker)?;
            return Ok(Vec::new());
        }
        CandleIterator::new(books.into_iter().map(Ok), query.candle_options()?).collect()
    }

    /// The last book of `ticker` at or before `at` (nanoseconds UTC), `None`
    /// when none stands there yet.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Absent`] for a table the service does not hold, and
    /// what reading the table refuses.
    pub fn book(&self, table: &str, ticker: &str, at: i64) -> Result<Option<BookEvent>> {
        let table = self.table(table)?;
        let filter = Filter::all(
            [
                category(),
                of_ticker(ticker),
                Term::column(EventColumn::CurrUnix.name()).le(literal_instant(at)?),
            ]
            .map(Filter::from),
        );
        let mut latest: Option<BookEvent> = None;
        for book in Self::stream(table, &filter)? {
            let book = book?;
            if latest
                .as_ref()
                .is_none_or(|held| held.get_currunix() <= book.get_currunix())
            {
                latest = Some(book);
            }
        }
        Ok(latest)
    }

    /// The audit rows of `query`: for every book of `query.ticker` in
    /// `[query.from, query.to)`, in instant order, its alive entries (role
    /// `alive`), the deltas applied since the book before it (`delta`) and
    /// its executions (`execution`), each kept where it stands on
    /// `query.side` - an execution on its own side - as batches of
    /// [`Self::events_field`]. Unbounded: the CSV audit is this reader
    /// written whole. The books of the range are held, sorted, while the
    /// reader lays their rows out a batch at a time.
    ///
    /// # Errors
    ///
    /// As [`Self::candles`].
    pub fn events(&self, query: &BookQuery) -> Result<BatchReader> {
        query.span_checked()?;
        let table = self.table(&query.table)?;
        let books = Self::books(table, &query.filter()?)?;
        if books.is_empty() {
            Self::ticker_known(table, &query.ticker)?;
        }
        audit_reader(books, query.side, usize::MAX)
    }

    /// The table `name` names.
    fn table(&self, name: &str) -> Result<&BookTable> {
        self.tables
            .iter()
            .find(|table| table.name == name)
            .ok_or_else(|| Error::absent("table", name))
    }

    /// The table's own record options, the `marketdata` row declared where
    /// the location declares none: a medium whose cells state no nested
    /// type, such as a CSV leaf, reads them as the row types them, and an
    /// empty or absent store reads as the empty stream of that row rather
    /// than of no column.
    ///
    /// `None` for a folder holding no leaf - an empty one, one holding only
    /// folders, one not there yet: a container answers the encoding of the
    /// leaves beneath it, so one holding none has no encoding to answer,
    /// and nothing is stored to read. A folder holding leaves no encoding
    /// reads keeps the refusal. The listing that tells the two apart is
    /// made only once the encoding is refused.
    fn read_options(table: &BookTable) -> Result<Option<RecordOptions>> {
        let options = match table.holder.record_options() {
            Ok(options) => options,
            Err(refused) => {
                let holds_nothing = table.holder.is_container()
                    && table
                        .holder
                        .children_where(&[], false)?
                        .next()
                        .transpose()?
                        .is_none();
                return if holds_nothing {
                    Ok(None)
                } else {
                    Err(refused)
                };
            }
        };
        if options.declared().is_some() {
            return Ok(Some(options));
        }
        Ok(Some(options.with_field(MarketData::field()?)))
    }

    /// The rows of `table` `filter` keeps, read through the table's own
    /// record options so a store that prunes on the filter prunes; the
    /// empty stream of the `marketdata` row where the table holds no leaf.
    fn read(table: &BookTable, filter: &Filter) -> Result<BatchReader> {
        match Self::read_options(table)? {
            Some(options) => table
                .holder
                .read_arrow_reader(&options.with_filter(filter)?),
            None => nothing(MarketData::field()?),
        }
    }

    /// The books of `table` `filter` keeps, in stored order, one at a time;
    /// a row of another leaf is skipped.
    fn stream(
        table: &BookTable,
        filter: &Filter,
    ) -> Result<impl Iterator<Item = Result<BookEvent>>> {
        Ok(
            MarketData::from_arrow_reader(Self::read(table, filter)?)?.filter_map(
                |item| match item {
                    Ok(MarketData::BookEvent(book)) => Some(Ok(*book)),
                    Ok(_) => None,
                    Err(error) => Some(Err(error)),
                },
            ),
        )
    }

    /// The books of `table` `filter` keeps, sorted by their instant, ties in
    /// stored order.
    fn books(table: &BookTable, filter: &Filter) -> Result<Vec<BookEvent>> {
        let mut books = Self::stream(table, filter)?.collect::<Result<Vec<_>>>()?;
        books.sort_by_key(Event::get_currunix);
        Ok(books)
    }

    /// The rows `filter` keeps of `table`, projected onto `columns` of the
    /// `marketdata` row - one projected read, which a store keeping its
    /// columns apart answers without reading the rest - as record columns
    /// of the struct `book` of them; none where the table holds no leaf.
    fn projected(table: &BookTable, filter: &Filter, columns: Vec<Field>) -> Result<SerieReader> {
        let select = Selector::new(
            columns
                .iter()
                .map(|column| Projection::column(column.name())),
        );
        let root = DataType::Struct(StructType::from_fields(columns)?).required_field("book");
        let reader = match Self::read_options(table)? {
            Some(options) => table
                .holder
                .read_arrow_reader(&options.with_filter(filter)?.with_select(select)?)?,
            None => nothing(root.clone())?,
        };
        Ok(SerieReader::from_arrow_reader(
            Some(&root),
            reader,
            ArrowCastOptions::new(),
        )?)
    }

    /// The span of each ticker's books in `table`, by ticker.
    fn spans(table: &BookTable) -> Result<BTreeMap<SmolStr, Span>> {
        let ticker = MarketColumn::Ticker.name();
        let crosscode = EventColumn::CrossCode.name();
        let currunix = EventColumn::CurrUnix.name();
        let reader = Self::projected(
            table,
            &Filter::from(category()),
            vec![
                DataType::utf8().nullable_field(ticker),
                DataType::utf8().nullable_field(crosscode),
                DataType::datetime64(TimeUnit::Nanosecond, Timezone::UTC)?.nullable_field(currunix),
            ],
        )?;
        let shape = || unshaped("the ticker, crosscode and currunix columns of a marketdata row");
        let mut spans = BTreeMap::new();
        for rows in reader {
            let rows = rows?;
            let tickers = rows
                .child(ticker)
                .and_then(Serie::as_utf8)
                .ok_or_else(shape)?;
            let codes = rows
                .child(crosscode)
                .and_then(Serie::as_utf8)
                .ok_or_else(shape)?;
            let instants = rows
                .child(currunix)
                .and_then(Serie::as_datetime_nanosecond)
                .ok_or_else(shape)?;
            for index in 0..rows.len() {
                let (Some(ticker), Some(unix)) = (tickers.value(index), instants.value(index))
                else {
                    continue;
                };
                let span = spans.entry(SmolStr::new(ticker)).or_insert_with(|| Span {
                    crosscode: codes
                        .value(index)
                        .map_or_else(|| SmolStr::new(ticker), SmolStr::new),
                    from: unix,
                    to: unix,
                    books: 0,
                });
                span.from = span.from.min(unix);
                span.to = span.to.max(unix);
                span.books += 1;
            }
        }
        Ok(spans)
    }

    /// The refusal a ticker `table` holds no book of earns; nothing for one
    /// it does.
    fn ticker_known(table: &BookTable, ticker: &str) -> Result<()> {
        if Self::spans(table)?.contains_key(ticker) {
            return Ok(());
        }
        Err(Error::absent(
            "ticker",
            format_args!("{}/{ticker}", table.name),
        ))
    }

    /// How many books `filter` keeps of `table`, and the instant the
    /// `want`-th earliest of them stands at - `None` when it keeps fewer:
    /// one projected scan of `currunix`, holding at most `want` instants.
    fn cut(table: &BookTable, filter: &Filter, want: usize) -> Result<(usize, Option<i64>)> {
        let currunix = EventColumn::CurrUnix.name();
        let reader = Self::projected(
            table,
            filter,
            vec![
                DataType::datetime64(TimeUnit::Nanosecond, Timezone::UTC)?.nullable_field(currunix),
            ],
        )?;
        let mut earliest = BinaryHeap::new();
        let mut total = 0_usize;
        for rows in reader {
            let rows = rows?;
            let instants = rows
                .child(currunix)
                .and_then(Serie::as_datetime_nanosecond)
                .ok_or_else(|| unshaped("the currunix column of a marketdata row"))?;
            for unix in (0..rows.len()).filter_map(|index| instants.value(index)) {
                total += 1;
                earliest.push(unix);
                if earliest.len() > want {
                    earliest.pop();
                }
            }
        }
        let cut = if earliest.len() < want {
            None
        } else {
            earliest.peek().copied()
        };
        Ok((total, cut))
    }

    /// The earliest books of `query` whose audit rows reach past `limit`, in
    /// instant order, ties in stored order, and whether the range states
    /// more than `limit` rows.
    ///
    /// The row bound is pushed into the read. A projected scan of the
    /// range's instants ([`Self::cut`]) finds the instant by which its
    /// earliest `limit + 1` books stand, and only the books up to it are
    /// read and built - that count doubled, and the read made again, while
    /// the books read state no more than `limit` rows on `query.side` and
    /// the range holds more. Of the books read, only the earliest are held
    /// ([`Self::select`]), so a request holds at most `limit + 1` books and
    /// `limit + 1` instants whatever its range.
    fn earliest(&self, query: &BookQuery, limit: usize) -> Result<(Vec<BookEvent>, bool)> {
        query.span_checked()?;
        let table = self.table(&query.table)?;
        let mut want = limit.saturating_add(1);
        loop {
            let (total, cut) = Self::cut(table, &query.filter()?, want)?;
            if total == 0 {
                Self::ticker_known(table, &query.ticker)?;
                return Ok((Vec::new(), false));
            }
            let (books, rows, read) = Self::select(table, &query.window(cut)?, query.side, limit)?;
            if rows > limit || cut.is_none() || read >= total {
                return Ok((books, rows > limit));
            }
            want = want.saturating_mul(2);
        }
    }

    /// The earliest books `filter` keeps whose audit rows on `side` reach
    /// past `limit`, in instant order, ties in stored order; how many rows
    /// they state; and how many books were read.
    ///
    /// The books stream from the read in stored order and only the earliest
    /// are held: a book stating no row on `side` is passed over, a book
    /// standing after every held one is passed over once the held ones
    /// state more than `limit` rows, and a book taken in drops the latest
    /// held ones the rest no longer need - so at most `limit + 1` are held,
    /// a store kept in no instant order included.
    fn select(
        table: &BookTable,
        filter: &Filter,
        side: Option<Side>,
        limit: usize,
    ) -> Result<(Vec<BookEvent>, usize, usize)> {
        let mut held: BTreeMap<(i64, usize), (BookEvent, usize)> = BTreeMap::new();
        let (mut rows, mut read) = (0_usize, 0_usize);
        for (ordinal, book) in Self::stream(table, filter)?.enumerate() {
            let book = book?;
            read += 1;
            let stated = audit_rows(&book, side).count();
            let key = (book.get_currunix(), ordinal);
            let past = rows > limit && held.last_key_value().is_some_and(|(last, _)| key > *last);
            if stated == 0 || past {
                continue;
            }
            held.insert(key, (book, stated));
            rows += stated;
            while let Some(last) = held.last_entry() {
                if rows - last.get().1 <= limit {
                    break;
                }
                rows -= last.remove().1;
            }
        }
        let books = held.into_values().map(|(book, _)| book).collect();
        Ok((books, rows, read))
    }

    // --------------------------------------------------------------------
    // The routes
    // --------------------------------------------------------------------

    /// One route's answer: the query read, `reading` run, a refusal spelled
    /// as JSON under the status it earns.
    fn answer(&self, request: &Request, reading: Reading) -> Response {
        let answered = request
            .url()
            .parameters(true)
            .map_err(|error| (Status::BAD_REQUEST, error))
            .and_then(|parameters| {
                reading(self, &parameters).map_err(|error| (status_of(&error), error))
            });
        match answered {
            Ok(response) => response,
            Err((status, error)) => {
                if status == Status::INTERNAL_SERVER_ERROR {
                    eprintln!("{} {}: {error}", request.method(), request.url());
                }
                self.refusal(status, &error)
            }
        }
    }

    /// `{"error": "<text>"}` under `status`, the text [`Self::redacted`].
    fn refusal(&self, status: Status, error: &Error) -> Response {
        let text = self.redacted(&error.to_string());
        let body = object([("error", Scalar::from(text.as_str()))]);
        Response::new(status)
            .with_json(&body)
            .and_then(no_store)
            .unwrap_or_else(|_| Response::new(status).with_text(&text))
    }

    /// `text` with what authenticates each table's location taken out
    /// wherever it is spelled: the user information before the host, `@`
    /// included, and the query after the path, `?` included - where a
    /// password, a token or a signature travels. What a client reads of a
    /// location or a refusal passes through here.
    fn redacted(&self, text: &str) -> String {
        let mut text = text.to_owned();
        for url in self.tables.iter().filter_map(|table| table.holder.url()) {
            let authority = url.authority();
            let user = authority
                .as_str()
                .strip_suffix(authority.host_port())
                .unwrap_or_default();
            if !user.is_empty() {
                text = text.replace(user, "");
            }
            if let Ok(Some(query)) = url.query(false) {
                text = text.replace(&format!("?{query}"), "");
            }
        }
        text
    }

    /// `/api/tables`: `[{"name","url"}]`, each location
    /// [`Self::redacted`], `url` null for a location that has none.
    fn answer_tables(&self, _: &Parameters<'_>) -> Result<Response> {
        let tables = self.tables.iter().map(|table| {
            object([
                ("name", Scalar::from(table.name.as_str())),
                (
                    "url",
                    table.holder.url().map_or(Scalar::Null, |url| {
                        Scalar::from(self.redacted(&url.to_string()))
                    }),
                ),
            ])
        });
        json(&Scalar::from_sequence(tables))
    }

    /// `/api/timezones`: `UTC`, then every zone this build has rules for,
    /// by name - every place zone the `tz` parameter reads, which reads
    /// their aliases and fixed offsets besides.
    fn answer_timezones(&self, _: &Parameters<'_>) -> Result<Response> {
        let zones = std::iter::once(Timezone::UTC)
            .chain(Timezone::registered().filter(|zone| !zone.is_utc()))
            .map(|zone| Scalar::from(zone.as_str()));
        json(&Scalar::from_sequence(zones))
    }

    /// `/api/tickers?table=`: [`Self::tickers`].
    fn answer_tickers(&self, parameters: &Parameters<'_>) -> Result<Response> {
        json(&self.tickers(required(parameters, "table")?)?)
    }

    /// `/api/candles`: [`Self::candles`] under its query, instants in `tz`.
    fn answer_candles(&self, parameters: &Parameters<'_>) -> Result<Response> {
        let query = BookQuery::from_parameters(parameters)?;
        let options = query.candle_options()?;
        let zone = query.timezone;
        let candles = self
            .candles(&query)?
            .iter()
            .map(|candle| candle_json(candle, zone))
            .collect::<Result<Vec<Scalar>>>()?;
        json(&object([
            ("table", Scalar::from(query.table)),
            ("ticker", Scalar::from(query.ticker)),
            ("timezone", Scalar::from(zone.as_str())),
            ("interval", Scalar::from(options.spelling())),
            ("from", zoned(query.from, zone)?),
            ("to", zoned(query.to, zone)?),
            ("candles", Scalar::from_sequence(candles)),
        ]))
    }

    /// `/api/book?table=&ticker=&at=&tz=`: [`Self::book`], or `404`.
    fn answer_book(&self, parameters: &Parameters<'_>) -> Result<Response> {
        let table = required(parameters, "table")?;
        let ticker = required(parameters, "ticker")?;
        let zone = zone(parameters)?;
        let at = instant(parameters, "at", zone)?;
        let book = self.book(table, ticker, at)?.ok_or_else(|| {
            Error::absent(
                "book",
                format_args!("{table}/{ticker} at or before {}", iso(at, zone)),
            )
        })?;
        json(&book_json(&book, zone)?)
    }

    /// `/api/events`: [`Self::events`]'s rows as JSON, at most `limit`, read
    /// holding only the books they come from ([`Self::earliest`]).
    fn answer_events(&self, parameters: &Parameters<'_>) -> Result<Response> {
        let query = BookQuery::from_parameters(parameters)?;
        let limit =
            usize::try_from(limit(parameters, self.options.max_event_rows)?).unwrap_or(usize::MAX);
        let (books, truncated) = self.earliest(&query, limit)?;
        let reader = audit_reader(books, query.side, limit)?;
        let names: Vec<SmolStr> = reader
            .schema()
            .fields()
            .iter()
            .map(|field| SmolStr::new(field.name()))
            .collect();
        let mut rows = Vec::new();
        for batch in SerieReader::from_arrow_reader(None, reader, ArrowCastOptions::new())? {
            let batch = batch?;
            for index in 0..batch.len() {
                let row = batch.scalar(index)?;
                let cells = row.sequence_rows().ok_or_else(|| Error::InvalidRecord {
                    path: format_smolstr!("$[{index}]"),
                    reason: expected_got("an audit row", row.kind()),
                })?;
                let cells = names
                    .iter()
                    .cloned()
                    .zip(cells.iter().map(|cell| in_zone(cell, query.timezone)))
                    .map(|(name, cell)| Ok((name, cell?)))
                    .collect::<Result<Vec<_>>>()?;
                rows.push(Scalar::from_struct(cells)?);
            }
        }
        json(&object([
            ("rows", Scalar::from_sequence(rows)),
            ("truncated", Scalar::from(truncated)),
        ]))
    }

    /// `/api/audit.csv`.
    fn answer_audit_csv(&self, parameters: &Parameters<'_>) -> Result<Response> {
        self.answer_audit(parameters, AUDIT_SUFFIXES[0])
    }

    /// `/api/audit.csv.gz`.
    fn answer_audit_gzip(&self, parameters: &Parameters<'_>) -> Result<Response> {
        self.answer_audit(parameters, AUDIT_SUFFIXES[1])
    }

    /// `/api/audit.csv.zst`.
    fn answer_audit_zstd(&self, parameters: &Parameters<'_>) -> Result<Response> {
        self.answer_audit(parameters, AUDIT_SUFFIXES[2])
    }

    /// The audit of a query as the CSV medium writes it into a buffer named
    /// `audit.<suffix>`, served under the coding's own media type as an
    /// attachment named after the ticker and the range.
    fn answer_audit(&self, parameters: &Parameters<'_>, suffix: &str) -> Result<Response> {
        let query = BookQuery::from_parameters(parameters)?;
        let reader = self.events(&query)?;
        let media_type = Url::from_str(&format!("file:///audit.{suffix}"))?.media_type();
        let mut buffer = Buffer::new().with_media_type(media_type.clone());
        let options = buffer.record_options()?;
        buffer.overwrite_arrow_reader(reader, &options)?;
        let body = buffer.into_bytes();
        let csv = MimeType::CSV;
        let content_type = media_type.encodings().last().unwrap_or(&csv).as_str();
        let filename = format!(
            "audit-{}-{}-{}.{suffix}",
            filename_safe(&query.ticker),
            compact(query.from),
            compact(query.to)
        );
        no_store(Response::new(Status::OK).with_header("content-type", content_type)?)?
            .with_header(
                "content-disposition",
                &format!("attachment; filename=\"{filename}\""),
            )
            .map(|response| response.with_body(body))
    }
}

/// One ticker's books in a table: the first and last instant and the count.
struct Span {
    crosscode: SmolStr,
    from: i64,
    to: i64,
    books: u64,
}

/// One audit row of a book, borrowed: an entry it holds, or an execution.
enum Audited<'book> {
    Entry(&'book MarketData),
    Execution(&'book ExecutionEvent),
}

impl Audited<'_> {
    /// The row's entry as the value [`MarketData::arrow_reader`] writes.
    fn owned(self) -> MarketData {
        match self {
            Self::Entry(entry) => entry.clone(),
            Self::Execution(execution) => MarketData::from(execution.clone()),
        }
    }
}

/// The audit rows of `book` kept to `side`, in audit order, each its role
/// and its entry: the alive entries (`alive`), the deltas (`delta`), the
/// executions (`execution`) - an entry kept where it stands on the side, an
/// execution on its own.
fn audit_rows(
    book: &BookEvent,
    side: Option<Side>,
) -> impl Iterator<Item = (&'static str, Audited<'_>)> {
    let alive = book
        .alive()
        .filter(move |entry| keeps(side, entry.get_side()))
        .map(|entry| (ROLE_ALIVE, Audited::Entry(entry)));
    let deltas = book
        .deltas()
        .filter(move |entry| keeps(side, entry.get_side()))
        .map(|entry| (ROLE_DELTA, Audited::Entry(entry)));
    let executions = book
        .executions()
        .iter()
        .filter(move |execution| keeps(side, execution.get_side()))
        .map(|execution| (ROLE_EXECUTION, Audited::Execution(execution)));
    alive.chain(deltas).chain(executions)
}

/// The first `limit` audit rows of `books` on `side`, laid out a batch at a
/// time: the entries written through [`MarketData::arrow_reader`], their
/// nested columns dropped by one selector, and each row's book instant and
/// role prepended batch by batch. The books are held; an entry is cloned
/// only as the batch holding it is written.
fn audit_reader(books: Vec<BookEvent>, side: Option<Side>, limit: usize) -> Result<BatchReader> {
    let books: Arc<[BookEvent]> = Arc::from(books);
    let entries = {
        let books = Arc::clone(&books);
        (0..books.len())
            .flat_map(move |index| {
                audit_rows(&books[index], side)
                    .map(|(_, entry)| entry.owned())
                    .collect::<Vec<_>>()
            })
            .take(limit)
    };
    let stamps = (0..books.len())
        .flat_map(move |index| {
            let unix = books[index].get_currunix();
            audit_rows(&books[index], side)
                .map(|(role, _)| (unix, role))
                .collect::<Vec<_>>()
        })
        .take(limit);
    let flat = Plan::new()
        .select(Selector::all_except(NESTED))?
        .apply_arrow_reader(MarketData::arrow_reader(entries, None, None)?)?;
    let field = BookService::events_field()?;
    let schema = field.clone().into_arrow_schema()?;
    let fields = field.fields();
    Ok(Box::new(EventRows {
        flat,
        stamps: Box::new(stamps),
        schema,
        bookunix: Arc::new(fields[0].clone()),
        role: Arc::new(fields[1].clone()),
        done: false,
    }))
}

/// The batches of an audit: each flat batch of entries with its book instants
/// and roles laid out in front of it.
struct EventRows {
    flat: BatchReader,
    stamps: Box<dyn Iterator<Item = (i64, &'static str)> + Send>,
    schema: SchemaRef,
    bookunix: Arc<Field>,
    role: Arc<Field>,
    done: bool,
}

impl EventRows {
    /// `batch` with its rows' stamps in front.
    fn stamped(&mut self, batch: &RecordBatch) -> Result<RecordBatch> {
        let rows = batch.num_rows();
        let stamps: Vec<(i64, &'static str)> = self.stamps.by_ref().take(rows).collect();
        if stamps.len() != rows {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$"),
                reason: expected_got(format_smolstr!("{rows} stamped entries"), stamps.len()),
            });
        }
        let column = |field: &Arc<Field>, cells: Vec<Scalar>| -> Result<arrow_array::ArrayRef> {
            Serie::from_scalars(Arc::clone(field), cells)?
                .into_arrow_array()
                .ok_or_else(|| Error::InvalidRecord {
                    path: format_smolstr!("$.{}", field.name()),
                    reason: SmolStr::new_static("expected a column, got a run"),
                })
        };
        let instants = stamps
            .iter()
            .map(|(unix, _)| zoned(*unix, Timezone::UTC))
            .collect::<Result<Vec<Scalar>>>()?;
        let roles = stamps
            .iter()
            .map(|(_, role)| Scalar::from(*role))
            .collect::<Vec<Scalar>>();
        let mut columns = Vec::with_capacity(2 + batch.num_columns());
        columns.push(column(&self.bookunix, instants)?);
        columns.push(column(&self.role, roles)?);
        columns.extend(batch.columns().iter().cloned());
        Ok(RecordBatch::try_new(Arc::clone(&self.schema), columns)?)
    }
}

impl Iterator for EventRows {
    type Item = std::result::Result<RecordBatch, ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let batch = match self.flat.next()? {
            Ok(batch) => batch,
            Err(error) => {
                self.done = true;
                return Some(Err(error));
            }
        };
        let stamped = self.stamped(&batch);
        if stamped.is_err() {
            self.done = true;
        }
        Some(stamped.map_err(|error| ArrowError::ExternalError(Box::new(error))))
    }
}

impl RecordBatchReader for EventRows {
    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }
}

// ------------------------------------------------------------------------
// The filters
// ------------------------------------------------------------------------

/// The empty stream of the rows `root` states: what a table holding no leaf
/// reads as.
fn nothing(root: Field) -> Result<BatchReader> {
    Ok(crate::arrow::batch_reader(root.into_arrow_schema()?, []))
}

/// A projected read answering columns other than the ones it asked for:
/// what no store answers, since the read casts onto the columns it names.
fn unshaped(expected: &'static str) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new_static("$"),
        reason: expected_got(expected, "another shape"),
    }
}

/// `marketdatakind = 'BOOK'`.
///
/// The member, never its name: a store may keep the column without its enum
/// identity - Iceberg stores a member as its `int` code and states no
/// extension - and a name beside a bare integer compares as text and matches
/// nothing, where the member meets an enum column and a bare code alike as
/// the `int32` it is.
fn category() -> Term {
    Term::column(MARKETDATAKIND).eq(Term::literal(MarketDataKind::Book))
}

/// `ticker = '<ticker>'`.
fn of_ticker(ticker: &str) -> Term {
    Term::column(MarketColumn::Ticker.name()).eq(Term::literal(ticker))
}

/// The instant `unix` as the `datetime64(ns, UTC)` literal a `currunix`
/// column compares to.
fn literal_instant(unix: i64) -> Result<Term> {
    Ok(Term::literal(zoned(unix, Timezone::UTC)?))
}

/// Whether an entry on `entry` stands on the side `asked` keeps: every side
/// when none was asked, the bids for a bid, the asks for an ask, the exact
/// member otherwise.
fn keeps(asked: Option<Side>, entry: Side) -> bool {
    match asked {
        None => true,
        Some(side) if side.is_bid() => entry.is_bid(),
        Some(side) if side.is_ask() => entry.is_ask(),
        Some(side) => entry == side,
    }
}

// ------------------------------------------------------------------------
// The parameters
// ------------------------------------------------------------------------

/// The refusal of the parameter `name`, located at `$.<name>`.
fn refused(name: &str, reason: SmolStr) -> Error {
    Error::InvalidRecord {
        path: format_smolstr!("$.{name}"),
        reason,
    }
}

/// The parameter `name`, refused when absent or empty.
fn required<'parameters>(
    parameters: &'parameters Parameters<'_>,
    name: &str,
) -> Result<&'parameters str> {
    parameters
        .get(name)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| refused(name, expected_got("a value", "none")))
}

/// The `tz` parameter: an IANA zone this build has rules for, `UTC` when
/// absent.
fn zone(parameters: &Parameters<'_>) -> Result<Timezone> {
    let Some(text) = parameters.get("tz").filter(|value| !value.is_empty()) else {
        return Ok(Timezone::UTC);
    };
    let refusal = || {
        refused(
            "tz",
            expected_got(
                "an IANA time zone this build knows (`UTC`, `Europe/Zurich`)",
                format_smolstr!("{text:?}"),
            ),
        )
    };
    let zone = Timezone::from_str(text).map_err(|_| refusal())?;
    if !zone.is_known() || zone.is_naive() {
        return Err(refusal());
    }
    Ok(zone)
}

/// The instant parameter `name`: ISO-8601 text, one stating an offset or
/// `Z` the instant it names, a naive one a wall clock in `zone`; refused
/// when absent.
fn instant(parameters: &Parameters<'_>, name: &str, zone: Timezone) -> Result<i64> {
    let text = required(parameters, name)?;
    let refusal = || {
        refused(
            name,
            expected_got(
                "an ISO-8601 instant (`2026-01-05T10:00:00Z`, or a wall clock read in `tz`)",
                format_smolstr!("{text:?}"),
            ),
        )
    };
    let dtype = DataType::datetime64(TimeUnit::Nanosecond, zone)?;
    let read = crate::text::arrow::parse_capture(text, &dtype, None).map_err(|_| refusal())?;
    read.as_datetime64()
        .map(|(count, ..)| count)
        .ok_or_else(refusal)
}

/// The `side` parameter: `bid` or `ask`, ASCII case ignored.
fn side(text: &str) -> Result<Side> {
    if text.eq_ignore_ascii_case("bid") {
        return Ok(Side::Buy);
    }
    if text.eq_ignore_ascii_case("ask") {
        return Ok(Side::Sell);
    }
    Err(refused(
        "side",
        expected_got("`bid` or `ask`", format_smolstr!("{text:?}")),
    ))
}

/// The `limit` parameter: a count of rows, `max` when absent, never above
/// it.
fn limit(parameters: &Parameters<'_>, max: u64) -> Result<u64> {
    let Some(text) = parameters.get("limit").filter(|value| !value.is_empty()) else {
        return Ok(max);
    };
    let limit: u64 = text.parse().map_err(|_| {
        refused(
            "limit",
            expected_got("a count of rows", format_smolstr!("{text:?}")),
        )
    })?;
    Ok(limit.min(max))
}

/// The status a refusal earns: the caller's parameter `400`, what there is
/// none of `404`, anything else `500`.
fn status_of(error: &Error) -> Status {
    match error {
        Error::Absent { .. } => Status::NOT_FOUND,
        Error::InvalidRecord { path, .. }
            if path
                .strip_prefix("$.")
                .is_some_and(|name| PARAMETERS.contains(&name)) =>
        {
            Status::BAD_REQUEST
        }
        _ => Status::INTERNAL_SERVER_ERROR,
    }
}

// ------------------------------------------------------------------------
// The answers
// ------------------------------------------------------------------------

/// A JSON `200` stating `Cache-Control: no-store`.
fn json(value: &Scalar) -> Result<Response> {
    no_store(Response::new(Status::OK).with_json(value)?)
}

/// `response` stating `Cache-Control: no-store`.
fn no_store(response: Response) -> Result<Response> {
    response.with_header(NO_STORE.0, NO_STORE.1)
}

/// A struct of `entries`, whose names are this module's own and distinct.
fn object<const N: usize>(entries: [(&'static str, Scalar); N]) -> Scalar {
    Scalar::from_struct(entries).expect("the names are literals of this module, each once")
}

/// The instant `unix` (nanoseconds UTC) as a `datetime64(ns, <zone>)` value:
/// what JSON renders as ISO-8601 text in `zone`.
fn zoned(unix: i64, zone: Timezone) -> Result<Scalar> {
    Scalar::datetime64(unix, TimeUnit::Nanosecond, zone)
}

/// The ISO-8601 text of `unix` in `zone`, for a message.
fn iso(unix: i64, zone: Timezone) -> String {
    crate::temporal::format_timestamp(unix, TimeUnit::Nanosecond, &zone)
        .map_or_else(|| unix.to_string(), String::from)
}

/// `unix` to the second as `YYYYMMDDTHHMMSSZ`, for a file name.
fn compact(unix: i64) -> String {
    crate::temporal::format_timestamp(unix.div_euclid(NANOS), TimeUnit::Second, &Timezone::UTC)
        .map_or_else(
            || unix.to_string(),
            |text| text.chars().filter(|c| *c != '-' && *c != ':').collect(),
        )
}

/// `text` with every byte a file name or a header would trip on replaced
/// by `_`.
fn filename_safe(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// `cell` with an instant restated in `zone`, every other value as it is.
fn in_zone(cell: &Scalar, zone: Timezone) -> Result<Scalar> {
    match cell.as_datetime64() {
        Some((count, unit, _)) => Scalar::datetime64(count, unit, zone),
        None => Ok(cell.clone()),
    }
}

/// A decimal or null.
fn decimal(value: Option<Decimal>) -> Scalar {
    value.map_or(Scalar::Null, Scalar::from)
}

/// `{open, high, low, close}`, or null for no reading.
fn ohlc_json(reading: Option<Ohlc>) -> Scalar {
    match reading {
        None => Scalar::Null,
        Some(reading) => object([
            ("open", Scalar::from(reading.open)),
            ("high", Scalar::from(reading.high)),
            ("low", Scalar::from(reading.low)),
            ("close", Scalar::from(reading.close)),
        ]),
    }
}

/// One candle as `/api/candles` spells it, its edges in `zone`.
fn candle_json(candle: &Candle, zone: Timezone) -> Result<Scalar> {
    Ok(object([
        ("start", zoned(candle.start, zone)?),
        ("end", zoned(candle.end, zone)?),
        ("bid", ohlc_json(candle.bid)),
        ("ask", ohlc_json(candle.ask)),
        ("mid", ohlc_json(candle.mid)),
        ("spread", ohlc_json(candle.spread)),
        ("bidqty", decimal(candle.bidqty)),
        ("askqty", decimal(candle.askqty)),
        ("books", Scalar::from(candle.books)),
        ("executions", Scalar::from(candle.executions)),
        ("volume", Scalar::from(candle.volume)),
    ]))
}

/// One book as `/api/book` spells it: its instant in `zone`, the readings of
/// its touch, the counts of its entries and each side's limits.
fn book_json(book: &BookEvent, zone: Timezone) -> Result<Scalar> {
    let limits =
        |side: Side| Scalar::from_sequence(book.limits(side).map(|limit| limit.into_scalar()));
    let count = |count: usize| Scalar::from(u64::try_from(count).unwrap_or(u64::MAX));
    Ok(object([
        ("currunix", zoned(book.get_currunix(), zone)?),
        (
            "ticker",
            book.get_ticker().map_or(Scalar::Null, Scalar::from),
        ),
        ("crosscode", Scalar::from(book.get_crosscode())),
        ("bestbid", decimal(book.best_price(Side::Buy))),
        ("bestask", decimal(book.best_price(Side::Sell))),
        ("bidqty", decimal(book.best_quantity(Side::Buy))),
        ("askqty", decimal(book.best_quantity(Side::Sell))),
        ("spread", decimal(book.spread())),
        ("midpoint", decimal(book.bbo_midpoint())),
        ("imbalance", decimal(book.imbalance(1))),
        ("islocked", Scalar::from(book.is_locked())),
        ("iscrossed", Scalar::from(book.is_crossed())),
        ("alive", count(book.alive().count())),
        ("deltas", count(book.deltas().count())),
        ("executions", count(book.executions().len())),
        ("bidlimits", limits(Side::Buy)),
        ("asklimits", limits(Side::Sell)),
    ]))
}
