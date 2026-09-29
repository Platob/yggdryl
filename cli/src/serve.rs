//! `yggdryl serve`: the book display from a terminal.
//!
//! `serve` routes the book service in [`yggdryl::graph`] - the tickers a
//! table holds, the candles a ticker's books fold into, the book standing at
//! an instant and the audit of its events - on the crate's HTTP [`Server`],
//! and beside it the display that reads those routes: the eight files of
//! `node/book/`, embedded in the binary so the page a browser opens and the
//! page the npm package ships are one source. Nothing here decides what a
//! request means: the command parses its arguments, folds each `--capture`
//! into the first table, binds the server, routes the [`BookService`] and
//! the assets at its path, and prints the endpoint. Behind a reverse proxy,
//! `--public-url`, `--trusted-proxy`, `--forwarded-header`, `--path-prefix`
//! and `--read-timeout` are the server's own options of those names.

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use clap::Args;
use yggdryl::graph::{BookEvent, BookIterator, BookService, BookServiceOptions, MarketData};
use yggdryl::holder::Holder;
use yggdryl::http::{ForwardedHeader, Method, Response, Server, ServerOptions, Status};
use yggdryl::text::{TextOptions, read_text_lines};
use yggdryl::{Error, FixCodec, FixRegistry, IOBase, IOKind, IOMedia, Result, Timezone, Url};

use crate::{location, style};

/// One display file the binary embeds: the name it is served under below
/// `--path`, its bytes and its `Content-Type`.
struct Asset {
    name: &'static str,
    bytes: &'static [u8],
    content_type: &'static str,
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
/// is also what the path itself answers.
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

/// The display's socket, what it serves, and what lands before it does.
#[derive(Args)]
#[command(
    after_help = "Examples:\n  yggdryl serve books=/data/books\n  yggdryl serve books=/tmp/books --capture rust/tests/fix/ulbridge.log --timezone Europe/Zurich\n  yggdryl serve /data/books --bind 0.0.0.0:8080 --path /book\n  yggdryl serve books=s3://bucket/books --public-url https://data.example.com/book --trusted-proxy 10.0.0.0/8 --path-prefix /book\n\nA table is `name=location`, or a location alone, named after its last segment: an Iceberg table folder, a record leaf (`.arrows`, `.parquet`, `.avro`, `.csv`) or a partitioned folder, each read by one filtered read per request.\n--capture folds a FIX bridge log into the first table before serving: its lines are read under --rowheader and --timezone, walked as the chains they belong to, folded into books on the --snapshot-millis grid and appended as BOOK rows. An empty or absent folder becomes an Iceberg table first (the `iceberg` feature); a leaf takes the rows under its own encoding.\nThe first line printed is the endpoint on the socket, so a script that started the process knows where to connect; the display answers there and the routes under `<path>/api`."
)]
pub struct Serve {
    /// The tables to serve: `name=location`, or a location named after itself.
    #[arg(value_name = "TABLE")]
    tables: Vec<String>,

    /// The address to listen on; port 0 takes a free one.
    #[arg(long, default_value = "127.0.0.1:8080")]
    bind: String,

    /// The path the display answers at; the routes stand under `<path>/api`.
    #[arg(long, default_value = "/")]
    path: String,

    /// The grid a capture's books are folded on before they land, in
    /// milliseconds; zero folds one book per event.
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

    /// Seconds a connection may stay quiet, or one request head may take to
    /// arrive whole, before it is closed, from 1 to 86400 (one day); keep it
    /// above the proxy's own keep-alive timeout.
    #[arg(
        long,
        default_value_t = 30,
        value_name = "SECONDS",
        value_parser = clap::value_parser!(u64).range(1..=ServerOptions::MAX_TIMEOUT.as_secs())
    )]
    read_timeout: u64,
}

/// Run `serve`.
///
/// # Errors
///
/// Returns a refused argument - a capture with no table to land in, a zone
/// this build has no rules for, a row header that does not compile, a
/// dictionary or a table location that does not resolve - the socket's
/// refusal, or what folding a capture into its table refuses.
pub fn run(serve: &Serve) -> Result<ExitCode> {
    serve.run()
}

impl Serve {
    fn run(&self) -> Result<ExitCode> {
        // Every argument is read before a port is taken, and the port before
        // a capture lands: a refused argument costs no bind, and a refused
        // bind no ingest that a second run would repeat.
        let tables: Vec<(String, &str)> = self
            .tables
            .iter()
            .map(|spelled| location::split(spelled))
            .collect();
        if !self.capture.is_empty() && tables.is_empty() {
            return Err(refused(
                "$.capture",
                "a capture needs a table to land in: expected a TABLE beside --capture, got none",
            ));
        }
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
        let mut holders = tables
            .iter()
            .map(|(_, location)| resource(location))
            .collect::<Result<Vec<_>>>()?;
        let server = Server::bind_with(&self.bind, self.server_options()?)?;
        let public_endpoint = server.public_url_of(&self.path)?;

        // The captures land in the first table before it is served.
        let mut created = false;
        let mut landed = Vec::with_capacity(self.capture.len());
        if let Some(codec) = &codec {
            let (name, location) = &tables[0];
            created = prepared(location)?;
            let table = &mut holders[0];
            for log in &self.capture {
                let books = ingest(table, log, codec, &reading, self.snapshot_millis)?;
                landed.push((log.as_str(), books, name.as_str()));
            }
        }

        let mut service =
            BookService::new(BookServiceOptions::new().with_snapshot_millis(self.snapshot_millis));
        for ((name, _), holder) in tables.iter().zip(holders) {
            service = service.with_table(name, holder);
        }
        let service = Arc::new(service);
        let endpoint = Arc::clone(&service).route(&server, &self.path)?;
        respond_assets(&server, &self.path)?;
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
        for (log, books, name) in landed {
            style::note(&format!("capture {log}: {books} books into {name}"));
        }
        // The server answers on its own threads until the process is
        // stopped; this one only has to outlive it.
        loop {
            std::thread::park();
        }
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
            .with_read_timeout(Duration::from_secs(self.read_timeout))
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
/// location as one file.
fn resource(location: &str) -> Result<Holder> {
    if location.contains("://") {
        return location::folder(location);
    }
    let held = Holder::local(location)?;
    if matches!(held.kind(), IOKind::Unknown) {
        return location::folder(location);
    }
    Ok(held)
}

/// One capture folded into `table`: its lines read under `reading`, walked
/// as the chains they belong to, folded into books on the `snapshot_millis`
/// grid and appended as `BOOK` rows; how many books landed.
///
/// The walk needs the whole capture and the count is what the note says, so
/// the books are held once, bounded by the capture's own size, before they
/// are written.
fn ingest(
    table: &mut Holder,
    log: &str,
    codec: &FixCodec,
    reading: &TextOptions,
    snapshot_millis: u64,
) -> Result<usize> {
    let capture = location::file(log)?;
    let walked = codec
        .lifecycle(codec.parse_text_lines(read_text_lines(&capture, reading)?))
        .collect::<Result<Vec<_>>>()?;
    let books = BookIterator::new(codec.market_data(walked), snapshot_millis)?
        .collect::<Result<Vec<BookEvent>>>()?;
    let landed = books.len();
    let rows = MarketData::arrow_reader(
        books
            .into_iter()
            .map(|book| Ok::<_, Error>(MarketData::from(book))),
        None,
        None,
    )?;
    let options = table.record_options()?;
    table.append_arrow_reader(rows, &options)?;
    Ok(landed)
}

/// A folder that holds nothing yet made the Iceberg table the captures
/// land in, so one command serves a capture with no table prepared by hand;
/// whether it was created. A leaf, a table already there and a folder with
/// anything in it are left as they are. The table's schema is the
/// `marketdata` row as Iceberg can state it - its `uint64` codes widened to
/// `decimal(20, 0)` - and every read casts the rows back onto the row field.
#[cfg(feature = "iceberg")]
fn prepared(location: &str) -> Result<bool> {
    use yggdryl::Scheme;
    use yggdryl::iceberg::{FormatVersion, PartitionSpec, Table};

    if !resource(location)?.is_container() {
        return Ok(false);
    }
    if Table::locate(location::folder(location)?)?.is_some() {
        return Ok(false);
    }
    if location::folder(location)?
        .children_where(&[], false)?
        .next()
        .is_some()
    {
        return Ok(false);
    }
    Table::create(
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

/// The display's files answered under `path`: the path itself and
/// `index.html` as the page, every other asset under its own name.
fn respond_assets(server: &Server, path: &str) -> Result<()> {
    let base = path.trim_end_matches('/');
    for asset in &ASSETS {
        let response = Response::new(Status::OK)
            .with_header("content-type", asset.content_type)?
            .with_header("cache-control", "no-cache")?
            .with_body(asset.bytes);
        if asset.name == "index.html" {
            server.respond(Some(Method::Get), &format!("{base}/"), response);
        }
        let response = Response::new(Status::OK)
            .with_header("content-type", asset.content_type)?
            .with_header("cache-control", "no-cache")?
            .with_body(asset.bytes);
        server.respond(
            Some(Method::Get),
            &format!("{base}/{}", asset.name),
            response,
        );
    }
    Ok(())
}

/// An argument refused by name.
fn refused(path: &'static str, reason: &str) -> Error {
    Error::InvalidRecord {
        path: path.into(),
        reason: reason.into(),
    }
}
