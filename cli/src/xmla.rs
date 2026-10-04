//! `yggdryl xmla`: the XML for Analysis provider from a terminal.
//!
//! `serve` routes the provider in [`yggdryl::xmla`] on the crate's HTTP
//! [`Server`] over folders of record media - one catalog per folder, its
//! files the tables, its folders the schemas - and answers Discover and
//! Execute until it is stopped. Nothing here decides what a request means:
//! the command parses its arguments, builds the [`Service`], binds the
//! server and routes the service at its path, so a table served here and a
//! table read through a handle are the same object read by the same code.
//! Behind a reverse proxy, `--public-url`, `--trusted-proxy`,
//! `--forwarded-header`, `--path-prefix` and `--read-timeout` are the
//! server's own options of those names, and the public endpoint is what
//! `DISCOVER_DATASOURCES` states as its `URL`.

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use clap::{Args, Subcommand};
use yggdryl::http::{ForwardedHeader, Server, ServerOptions};
use yggdryl::xmla::{Service, ServiceOptions};
use yggdryl::{Catalog, ObjectValue, Properties, Result, Url};

use crate::{location, style, timeout};

/// What the provider was asked to do.
#[derive(Subcommand)]
#[command(
    after_help = "Examples:\n  yggdryl xmla serve market=/data/market reference=/data/reference\n  yggdryl xmla serve /data/market --bind 0.0.0.0:8080 --path /xmla\n  yggdryl xmla serve market=s3://bucket/market --writable\n  yggdryl xmla serve market=C:\\data\\market --trace C:\\data\\trace\n  yggdryl xmla serve market=/data/market --public-url https://data.example.com/olap --trusted-proxy 10.0.0.0/8 --path-prefix /olap\n  yggdryl xmla serve market=/data/market --trusted-proxy 127.0.0.1 --forwarded-header X-Forwarded-For --forwarded-header X-Forwarded-Proto --forwarded-header X-Forwarded-Prefix\n\nA catalog is `name=location`, or a location alone, named after its last segment. A location is a folder path, or a URL a holder resolves, read as a folder catalog.\nEvery record file the folder holds is a table; a folder inside it is a schema whose files are its tables; a folder laid out as an Iceberg table is a table wherever it sits (the `iceberg` feature reads it).\nThe first line printed is the endpoint on the socket, so a script that started the process knows where to connect; behind a proxy, a note names the public endpoint.\n--trace writes each exchange as it went over the wire, a request file and a response file per exchange, so what a client asked can be read and replayed."
)]
pub enum Command {
    /// Serve catalogs over HTTP, one request per POST, until stopped.
    Serve(Serve),
}

/// The provider's socket and what it serves.
#[derive(Args)]
pub struct Serve {
    /// The catalogs to serve: `name=location`, or a location named after itself.
    #[arg(required = true, value_name = "CATALOG")]
    catalogs: Vec<String>,

    /// The address to listen on; port 0 takes a free one.
    #[arg(long, default_value = "127.0.0.1:8080")]
    bind: String,

    /// The request path the endpoint answers.
    #[arg(long, default_value = "/xmla")]
    path: String,

    /// Let `insert`, `upsert` and `delete` statements write the tables.
    #[arg(long)]
    writable: bool,

    /// The largest request body accepted, in bytes.
    #[arg(long, default_value_t = 16 * 1024 * 1024)]
    max_body: u64,

    /// Write every exchange under this folder: `NNNN-request.http` as read,
    /// `NNNN-response.http` as sent, numbered from 0000 in request order.
    #[arg(long, value_name = "FOLDER")]
    trace: Option<String>,

    /// The URL clients reach the endpoint at, behind a reverse proxy or a
    /// redirect: `https://host[:port][/prefix]`, the prefix being the path the
    /// proxy adds in front of `--path`. It is the base of every URL the
    /// server states and the `URL` of `DISCOVER_DATASOURCES`.
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
    /// `/olap` when the proxy forwards `/olap/xmla` and `--path` is `/xmla`.
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

/// Run one `xmla` verb.
///
/// # Errors
///
/// Returns the holder's refusal of a catalog location, or the socket's.
pub fn run(command: &Command) -> Result<ExitCode> {
    match command {
        Command::Serve(serve) => serve.run(),
    }
}

impl Serve {
    fn run(&self) -> Result<ExitCode> {
        // Every argument is read before a port is taken, so a refused one
        // costs no bind.
        let catalogs = self
            .catalogs
            .iter()
            .map(|spelled| catalog(spelled))
            .collect::<Result<Vec<_>>>()?;
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
        let server = Server::bind_with(&self.bind, options)?;
        // The public endpoint is the one a client invokes the methods at, so
        // it is what DISCOVER_DATASOURCES states; on the socket alone the
        // server's own URL is that endpoint.
        let public_endpoint = server.public_url_of(&self.path)?;
        let mut service_options = ServiceOptions::new().with_writable(self.writable);
        if self.public_url.is_some() {
            service_options = service_options.with_url(public_endpoint.to_string());
        }
        let mut service = Service::new(service_options);
        for catalog in catalogs {
            service = service.with_catalog(catalog);
        }
        let service = Arc::new(service);
        let endpoint = Arc::clone(&service).route(&server, &self.path)?;
        // The endpoint first and on its own line, so whatever started the
        // process reads where to connect before anything else is printed.
        println!("{endpoint}");
        if self.public_url.is_some() {
            style::note(&format!("public endpoint {public_endpoint}"));
        }
        for catalog in service.catalogs() {
            style::note(&format!(
                "catalog {} over {}{}",
                catalog.name(),
                catalog.url().map_or_else(String::new, ToString::to_string),
                if self.writable { ", writable" } else { "" }
            ));
        }
        // The server answers on its own threads until the process is
        // stopped; this one only has to outlive it.
        loop {
            std::thread::park();
        }
    }
}

/// `name=location`, or a location alone named after its last segment
/// ([`location::split`]), as the catalog [`Catalog::from_url`] builds over
/// the location: a folder path or URL is a folder catalog.
fn catalog(spelled: &str) -> Result<Catalog> {
    let (name, location) = location::split(spelled);
    let url = Url::from_location(location)?;
    Catalog::from_url(&url, &Properties::new().with_property("name", name))
}
