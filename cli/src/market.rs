//! `yggdryl market`: the market-data namespace from a terminal.
//!
//! `serve` routes the book service in [`yggdryl::graph`] - the tickers a
//! table holds, the candles a ticker's books fold into, the book standing at
//! an instant and the audit of its events - on the crate's HTTP [`Server`],
//! and beside it the display that reads those routes: the eight files of
//! `node/book/`, embedded in the binary so the page a browser opens and the
//! page the npm package ships are one source. Nothing here decides what a
//! request means: the command parses its arguments, folds every `--capture`
//! into the first table, binds the server, routes the [`BookService`] and
//! the assets at its path, and prints the endpoint - the folder `<path>/`
//! the page and its routes stand in, so the page's relative files resolve
//! where they are served. Behind a reverse proxy,
//! `--public-url`, `--trusted-proxy`, `--forwarded-header`, `--path-prefix`
//! and `--read-timeout` are the server's own options of those names.

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use clap::{Args, Subcommand};
use yggdryl::graph::{BookEvent, BookIterator, BookService, BookServiceOptions, MarketData};
use yggdryl::holder::Holder;
use yggdryl::http::{ForwardedHeader, Method, Request, Response, Server, ServerOptions, Status};
use yggdryl::media::{IORecordOptions, RecordOptions};
use yggdryl::text::{TextOptions, read_text_lines};
use yggdryl::{
    Error, FixCodec, FixRegistry, IOBase, IOKind, IOMedia, Result, Scheme, Timezone, Url,
};

use crate::{location, style, timeout};

/// One display file the binary embeds: the name it is served under below
/// `--path`, its bytes and its `Content-Type`.
struct Asset {
    name: &'static str,
    bytes: &'static [u8],
    content_type: &'static str,
}

impl Asset {
    /// The asset as the server answers it: its bytes under its type,
    /// `Cache-Control: no-cache`.
    fn response(&self) -> Result<Response> {
        Ok(Response::new(Status::OK)
            .with_header("content-type", self.content_type)?
            .with_header("cache-control", "no-cache")?
            .with_body(self.bytes))
    }
}

/// One asset read out of `node/book/` at build time, so the display the
/// package ships and the display the binary serves are one file.
macro_rules! asset {
    ($name:literal, $content_type:literal) => {
        Asset {
            name: $name,
            bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../node/book/", $name)),
            content_type: $content_type,
        }
    };
}

/// The display, in the order `node/book.js` lists its files; `index.html`
/// is the page, which the root path also answers.
const ASSETS: [Asset; 8] = [
    asset!("index.html", "text/html; charset=utf-8"),
    asset!("theme.css", "text/css; charset=utf-8"),
    asset!("theme.js", "text/javascript; charset=utf-8"),
    asset!("api.js", "text/javascript; charset=utf-8"),
    asset!("chart.js", "text/javascript; charset=utf-8"),
    asset!("audit.js", "text/javascript; charset=utf-8"),
    asset!("app.js", "text/javascript; charset=utf-8"),
    asset!("favicon.svg", "image/svg+xml"),
];

/// What the market namespace was asked to do.
#[derive(Subcommand)]
pub enum Command {
    /// Serve tables of market data as the book display over HTTP, until stopped.
    Serve(Serve),
}

/// The example of a capture landing in a folder with nothing in it yet,
/// which only a build with the `iceberg` feature runs - it makes the folder
/// a table - so a build without it states none: every example `--help`
/// states runs in the build that states it.
#[cfg(feature = "iceberg")]
macro_rules! capture_example {
    () => {
        "  yggdryl market serve books=/tmp/books --capture rust/tests/fix/ulbridge.log --timezone Europe/Zurich\n"
    };
}
#[cfg(not(feature = "iceberg"))]
macro_rules! capture_example {
    () => {
        ""
    };
}

/// What `market serve --help` states after the arguments: the examples,
/// then how a table, a capture and the endpoint are read.
const AFTER_HELP: &str = concat!(
    "Examples:\n",
    "  yggdryl market serve books=/data/books\n",
    capture_example!(),
    "  yggdryl market serve /data/books --bind 0.0.0.0:8080 --path /book\n",
    "  yggdryl market serve books=/data/books --public-url https://data.example.com/book --trusted-proxy 10.0.0.0/8 --path-prefix /book\n",
    "\n",
    "A table is `name=location`, or a location alone, named after its last segment: an Iceberg table folder, a record leaf (`.arrows`, `.parquet`, `.avro`, `.csv`) or a partitioned folder, each read by one filtered read per request. Two tables of one name are refused.\n",
    "--capture folds a FIX bridge log into the first table before serving: its lines are read under --rowheader and --timezone, walked as the chains they belong to, folded into books on the --snapshot-millis grid and appended as BOOK rows, every capture read before any lands. An empty or absent folder becomes an Iceberg table first (the `iceberg` feature); a leaf takes the rows under its own encoding.\n",
    "The first line printed is the endpoint on the socket, the folder `<path>/` the display stands in, so a script that started the process knows where to connect: the page is its `index.html` - `<path>` itself sends a browser there, and the root path answers it too - and the routes stand under its `api/`."
);

/// The display's socket, what it serves, and what lands before it does.
#[derive(Args)]
#[command(after_help = AFTER_HELP)]
pub struct Serve {
    /// The tables to serve: `name=location`, or a location named after itself.
    #[arg(value_name = "TABLE")]
    tables: Vec<String>,

    /// The address to listen on; port 0 takes a free one.
    #[arg(long, default_value = "127.0.0.1:8080")]
    bind: String,

    /// The path the display answers under: its page is `<path>/index.html`,
    /// which `<path>` itself sends a browser to, and the routes stand under
    /// `<path>/api`.
    #[arg(long, default_value = "/")]
    path: String,

    /// The grid a capture's books are folded on before they land, in
    /// milliseconds: a book lands whole - every entry alive - at every grid
    /// tick holding one and at a full refresh, and as its deltas alone at
    /// every other event, its first appearance included, which follows the
    /// empty book. Zero is no grid, so the book at an instant is rebuilt from
    /// its first appearance or its last full refresh, however far back.
    #[arg(long, default_value_t = 0, value_name = "MILLIS")]
    snapshot_millis: u64,

    /// A FIX bridge log folded into the first table before serving; repeat
    /// for several.
    #[arg(long, value_name = "LOG")]
    capture: Vec<String>,

    /// The FIX dictionary a capture is read with.
    #[arg(long, default_value = "config/fix", value_name = "FOLDER")]
    registry: String,

    /// The row header every capture line opens with, a regex of named
    /// captures; the default is the `ULBridge` log's.
    #[arg(long, default_value = yggdryl::ULBRIDGE_ROWHEADER, value_name = "REGEX")]
    rowheader: String,

    /// The zone the bridge's clock writes its lines in.
    #[arg(long, default_value = "UTC", value_name = "ZONE")]
    timezone: String,

    /// The largest request body accepted, in bytes.
    #[arg(long, default_value_t = 16 * 1024 * 1024)]
    max_body: u64,

    /// Write every exchange under this folder: `NNNN-request.http` as read,
    /// `NNNN-response.http` as sent, numbered from 0000 in request order.
    #[arg(long, value_name = "FOLDER")]
    trace: Option<String>,

    /// The URL clients reach the display at, behind a reverse proxy or a
    /// redirect: `https://host[:port][/prefix]`, the prefix being the path
    /// the proxy adds in front of `--path`.
    #[arg(long, value_name = "URL")]
    public_url: Option<String>,

    /// A proxy whose forwarded fields are believed: an IP address or a CIDR
    /// network; repeat for several.
    #[arg(long = "trusted-proxy", value_name = "IP|CIDR")]
    trusted_proxies: Vec<String>,

    /// A forwarded field read from a trusted proxy - `Forwarded`,
    /// `X-Forwarded-For`, `-Proto`, `-Host`, `-Port` or `-Prefix`; repeat
    /// for several. Given, the list replaces the default `X-Forwarded-For`
    /// and `X-Forwarded-Proto`: name only fields the proxy sets or
    /// overwrites on every request, since one it passes through is the
    /// client's to write.
    #[arg(
        long = "forwarded-header",
        value_name = "FIELD",
        value_parser = |name: &str| ForwardedHeader::from_str(name).map_err(|error| error.to_string())
    )]
    forwarded_headers: Vec<ForwardedHeader>,

    /// A path prefix the proxy leaves on requests, stripped before routing:
    /// `/olap` when the proxy forwards `/olap/book` and `--path` is `/book`.
    #[arg(long, value_name = "PREFIX")]
    path_prefix: Option<String>,

    /// How long a connection may stay quiet, or one request head may take to
    /// arrive whole, before it is closed: seconds, with a fraction and an
    /// optional unit (`30`, `2.5`, `1500ms`), above zero and at most 86400
    /// (one day); keep it above the proxy's own keep-alive timeout.
    #[arg(
        long,
        default_value = "30",
        value_name = "SECONDS",
        value_parser = timeout::read_timeout
    )]
    read_timeout: Duration,
}

/// Run one `market` verb.
///
/// # Errors
///
/// Returns a refused argument - two tables of one name, a capture with no
/// table to land in, a path the server cannot route, a zone this build has
/// no rules for, a row header that does not compile, a dictionary, a table
/// or a capture location that does not resolve - the socket's refusal, or
/// what reading a capture or landing it in its table refuses.
pub fn run(command: &Command) -> Result<ExitCode> {
    match command {
        Command::Serve(serve) => serve.run(),
    }
}

impl Serve {
    fn run(&self) -> Result<ExitCode> {
        // Every argument is read before a port is taken, and the port, the
        // endpoint and every capture before one lands: a refused argument
        // costs no bind, and a refused bind, endpoint or capture no ingest
        // that a second run would repeat.
        let tables = self.tables()?;
        if !self.capture.is_empty() && tables.is_empty() {
            return Err(refused(
                "$.capture",
                "a capture needs a table to land in: expected a TABLE beside --capture, got none",
            ));
        }
        let base = route_base(&self.path)?;
        let timezone = self.timezone()?;
        let mut reading = TextOptions::new()
            .try_with_rowheader(&self.rowheader)?
            .with_timezone(timezone);
        reading.start_rownum = Some(1);
        reading.parse_mimetype = true;
        // The dictionary is read only where a capture needs it.
        let codec = if self.capture.is_empty() {
            None
        } else {
            Some(self.codec(&reading)?)
        };
        let captures = self
            .capture
            .iter()
            .map(|log| location::file(log))
            .collect::<Result<Vec<_>>>()?;
        let mut holders = tables
            .iter()
            .map(|(_, location)| resource(location))
            .collect::<Result<Vec<_>>>()?;
        let server = Server::bind_with(&self.bind, self.server_options()?)?;
        // The folder the page and its routes stand in, where the page's
        // relative files resolve: the endpoint.
        let folder = format!("{base}/");
        let endpoint = server.url_of(&folder)?;
        let public_endpoint = server.public_url_of(&folder)?;

        // Every capture is read before any lands, and they land in the first
        // table in one append, before it is served.
        let mut created = false;
        let mut landed = Vec::with_capacity(self.capture.len());
        if let Some(codec) = &codec {
            let mut books = Vec::new();
            for (log, capture) in self.capture.iter().zip(&captures) {
                let before = books.len();
                books.extend(fold(capture, codec, &reading, self.snapshot_millis)?);
                landed.push((log.as_str(), books.len() - before));
            }
            created = prepared(tables[0].1)?;
            land(&mut holders[0], books)?;
        }

        let mut service =
            BookService::new(BookServiceOptions::new().with_snapshot_millis(self.snapshot_millis));
        for ((name, _), holder) in tables.iter().zip(holders) {
            service = service.with_table(name, holder);
        }
        let service = Arc::new(service);
        Arc::clone(&service).route(&server, &folder)?;
        respond_assets(&server, &base)?;
        // The endpoint first and on its own line, so whatever started the
        // process reads where to connect before anything else is printed.
        println!("{endpoint}");
        if self.public_url.is_some() {
            style::note(&format!("public endpoint {public_endpoint}"));
        }
        for (index, table) in service.tables().iter().enumerate() {
            style::note(&format!(
                "table {} over {}{}",
                table.name(),
                table
                    .holder()
                    .url()
                    .map_or_else(String::new, ToString::to_string),
                if created && index == 0 {
                    ", created"
                } else {
                    ""
                }
            ));
        }
        for (log, books) in landed {
            style::note(&format!(
                "capture {log}: {books} books into {}",
                tables[0].0
            ));
        }
        // The server answers on its own threads until the process is
        // stopped; this one only has to outlive it.
        loop {
            std::thread::park();
        }
    }

    /// The tables as spelled: `name=location`, or a location named after its
    /// last segment ([`location::split`]). Two of one name are refused, since
    /// a route's `table` names one and a capture lands in the first.
    fn tables(&self) -> Result<Vec<(String, &str)>> {
        let mut tables: Vec<(String, &str)> = Vec::with_capacity(self.tables.len());
        for (index, spelled) in self.tables.iter().enumerate() {
            let (name, location) = location::split(spelled);
            if let Some((_, first)) = tables.iter().find(|(named, _)| *named == name) {
                return Err(refused(
                    &format!("$.tables[{index}]"),
                    &format!(
                        "expected one table per name, got {name:?} over both {first:?} and {location:?}"
                    ),
                ));
            }
            tables.push((name, location));
        }
        Ok(tables)
    }

    /// The zone the captures' clock is written in: one this build has rules
    /// for, since a naive line clock has to be converted.
    fn timezone(&self) -> Result<Timezone> {
        let zone = Timezone::from_str(&self.timezone)?;
        if !zone.is_known() {
            return Err(refused(
                "$.timezone",
                &format!(
                    "expected an IANA zone this build has rules for, or a fixed offset, got {:?}",
                    self.timezone
                ),
            ));
        }
        Ok(zone)
    }

    /// The codec the captures are read with: the dictionary under
    /// `--registry`, told what the row header's captures are called so a
    /// message carries where it was read from.
    ///
    /// A store with nothing in it loads as a dictionary of the crate's own
    /// fields alone, which would read every capture line as unknown and
    /// serve no book; a folder listing nothing is refused by name instead.
    fn codec(&self, reading: &TextOptions) -> Result<FixCodec> {
        let store = location::folder(&self.registry)?;
        if store.children_where(&[], false)?.next().is_none() {
            return Err(Error::absent(
                "FIX dictionary",
                store
                    .url()
                    .map_or_else(|| self.registry.clone(), ToString::to_string),
            ));
        }
        let registry = Arc::new(FixRegistry::from_handle(&store)?);
        let names: Vec<String> = reading.capture_names().map(ToOwned::to_owned).collect();
        Ok(FixCodec::new(registry).with_capture_names(names))
    }

    /// The server's options as the arguments state them.
    fn server_options(&self) -> Result<ServerOptions> {
        let mut options = ServerOptions::default()
            .with_max_body_size(self.max_body)
            .with_read_timeout(self.read_timeout)
            .with_trusted_proxies(&self.trusted_proxies)?;
        if !self.forwarded_headers.is_empty() {
            options = options.with_forwarded_headers(self.forwarded_headers.iter().copied());
        }
        if let Some(trace) = &self.trace {
            options = options.with_trace(location::folder(trace)?);
        }
        if let Some(public_url) = &self.public_url {
            options = options.with_public_url(Url::from_str(public_url)?);
        }
        if let Some(prefix) = &self.path_prefix {
            options = options.with_path_prefix(prefix)?;
        }
        Ok(options)
    }
}

/// The holder a table location names: a record leaf where a file is, a
/// folder where a folder is - and where nothing is yet, the folder a
/// capture makes a table of, since a write would otherwise settle the
/// location as one file. Served with no capture, a folder holding nothing
/// is a table holding no book, and reading it creates nothing.
fn resource(location: &str) -> Result<Holder> {
    let url = Url::from_location(location)?;
    if location::is_place(&url)? {
        let held = location::from_url(&url)?;
        if !matches!(held.kind(), IOKind::Unknown) {
            return Ok(held);
        }
    }
    location::folder_of(&url)
}

/// `--path` spelled as a route is, by the server's one path grammar - the
/// one a path prefix shares, which [`ServerOptions::with_path_prefix`]
/// reads without a server: one leading slash and no trailing one, and empty
/// for the root. A query, a fragment or a control byte is refused by byte.
fn route_base(path: &str) -> Result<String> {
    Ok(ServerOptions::default()
        .with_path_prefix(path)?
        .path_prefix()
        .unwrap_or_default()
        .to_owned())
}

/// One capture read under `reading`, walked as the chains it belongs to and
/// folded into books on the `snapshot_millis` grid.
///
/// The walk needs the whole capture and the note counts its books, so they
/// are held, bounded by the captures' own sizes, until every capture has
/// been read and they land ([`land`]).
fn fold(
    capture: &Holder,
    codec: &FixCodec,
    reading: &TextOptions,
    snapshot_millis: u64,
) -> Result<Vec<BookEvent>> {
    let walked = codec
        .lifecycle(codec.parse_text_lines(read_text_lines(capture, reading)?))
        .collect::<Result<Vec<_>>>()?;
    BookIterator::new(codec.market_data(walked), snapshot_millis)?.collect()
}

/// `books` appended to `table` as `BOOK` rows, in one append through the
/// table's own record options - one commit on an Iceberg table.
///
/// Avro spells no unsigned 64-bit integer, so an Avro leaf that declares no
/// row takes the one Iceberg states - its `uint64` codes widened to
/// `decimal(20, 0)`, Iceberg's own types being what an Avro data file holds
/// - and every read casts the rows back onto the row field.
fn land(table: &mut Holder, books: Vec<BookEvent>) -> Result<()> {
    let rows = MarketData::arrow_reader(
        books
            .into_iter()
            .map(|book| Ok::<_, Error>(MarketData::from(book))),
        None,
        None,
    )?;
    let mut options = table.record_options()?;
    if matches!(options, RecordOptions::Avro(_)) && options.field().is_none() {
        options.set_field(MarketData::field()?.into_scheme_compat(&Scheme::ICEBERG)?);
    }
    table.append_arrow_reader(rows, &options)?;
    Ok(())
}

/// A folder that holds nothing yet made the Iceberg table the captures
/// land in, so one command serves a capture with no table prepared by hand;
/// whether it was created. A leaf, a table already there and a folder with
/// anything in it are left as they are. The table's schema is the
/// `marketdata` row as Iceberg can state it - its `uint64` codes widened to
/// `decimal(20, 0)` - and every read casts the rows back onto the row field.
#[cfg(feature = "iceberg")]
fn prepared(location: &str) -> Result<bool> {
    use yggdryl::iceberg::{FormatVersion, IcebergTable, PartitionSpec};

    if !resource(location)?.is_container() {
        return Ok(false);
    }
    if IcebergTable::locate(location::folder(location)?)?.is_some() {
        return Ok(false);
    }
    if location::folder(location)?
        .children_where(&[], false)?
        .next()
        .is_some()
    {
        return Ok(false);
    }
    IcebergTable::create(
        location::folder(location)?,
        FormatVersion::V2,
        MarketData::field()?.into_scheme_compat(&Scheme::ICEBERG)?,
        PartitionSpec::unpartitioned(),
    )?;
    Ok(true)
}

/// Without the `iceberg` feature nothing is made a table: a folder takes
/// the rows through the partition writer, a leaf through its encoding. The
/// `Result` is the signature the call site reads in both builds.
#[cfg(not(feature = "iceberg"))]
#[allow(clippy::unnecessary_wraps)]
fn prepared(_: &str) -> Result<bool> {
    Ok(false)
}

/// The display's files answered under `base` ([`route_base`]), each under
/// its own name, `index.html` the page.
///
/// A page's relative `theme.css`, `app.js` and `api/` resolve against the
/// folder its URL ends in, and the server sends a path spelled with a
/// trailing slash to the bare one, so under a path the page stands at
/// `<path>/index.html` and the bare `<path>` sends a browser there
/// ([`into_folder`]). The root is its own folder, and answers the page.
fn respond_assets(server: &Server, base: &str) -> Result<()> {
    for asset in &ASSETS {
        server.respond(
            Some(Method::Get),
            &format!("{base}/{}", asset.name),
            asset.response()?,
        );
        if base.is_empty() && asset.name == "index.html" {
            server.respond(Some(Method::Get), "/", asset.response()?);
        }
    }
    if !base.is_empty() {
        server.route(Some(Method::Get), base, into_folder);
    }
    Ok(())
}

/// The bare path sent into its folder's page by a `308` whose `Location` is
/// relative - the path's last segment as the request spelled it, then
/// `/index.html` and the query - so it is right under any prefix a proxy
/// adds, as the server's own trailing-slash redirect is.
fn into_folder(request: &Request) -> Result<Response> {
    let url = request.url();
    let path = url.path_text(false)?;
    let last = path.rsplit('/').next().unwrap_or_default();
    let query = url
        .query(false)?
        .map_or_else(String::new, |query| format!("?{query}"));
    Response::new(Status::PERMANENT_REDIRECT)
        .with_header("location", &format!("{last}/index.html{query}"))?
        .with_header("cache-control", "no-cache")
}

/// An argument refused by name.
fn refused(path: &str, reason: &str) -> Error {
    Error::InvalidRecord {
        path: path.into(),
        reason: reason.into(),
    }
}
