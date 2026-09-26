//! `ygg xmla`: the XML for Analysis provider from a terminal.
//!
//! `serve` routes the provider in [`yggdryl::xmla`] on the crate's HTTP
//! [`Server`] over folders of record media - one catalog per folder, its
//! files the tables, its folders the schemas - and answers Discover and
//! Execute until it is stopped. Nothing here decides what a request means:
//! the command parses its arguments, builds the [`Service`], binds the
//! server and routes the service at its path, so a table served here and a
//! table read through a handle are the same object read by the same code.

use std::process::ExitCode;
use std::sync::Arc;

use clap::{Args, Subcommand};
use yggdryl::holder::Holder;
use yggdryl::http::{Server, ServerOptions};
use yggdryl::xmla::{Catalog, Service, ServiceOptions};
use yggdryl::{Result, Url};

use crate::style;

/// What the provider was asked to do.
#[derive(Subcommand)]
#[command(
    after_help = "Examples:\n  ygg xmla serve market=/data/market reference=/data/reference\n  ygg xmla serve /data/market --bind 0.0.0.0:8080 --path /xmla\n  ygg xmla serve market=s3://bucket/market --writable\n  ygg xmla serve market=C:\\data\\market --trace C:\\data\\trace\n\nA catalog is `name=location`, or a location alone, named after its last segment. A location is a folder path, or a URL a holder resolves.\nEvery record file the folder holds is a table; a folder inside it is a schema whose files are its tables; a folder laid out as an Iceberg table is a table wherever it sits (the `iceberg` feature reads it).\nThe first line printed is the endpoint, so a script that started the process knows where to connect.\n--trace writes each exchange as it went over the wire, a request file and a response file per exchange, so what a client asked can be read and replayed."
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
        let mut service = Service::new(ServiceOptions::new().with_writable(self.writable));
        for spelled in &self.catalogs {
            service = service.with_catalog(catalog(spelled)?);
        }
        let mut options = ServerOptions::default().with_max_body_size(self.max_body);
        if let Some(trace) = &self.trace {
            options = options.with_trace(holder(trace)?);
        }
        let server = Server::bind_with(&self.bind, options)?;
        let service = Arc::new(service);
        let endpoint = Arc::clone(&service).route(&server, &self.path)?;
        // The endpoint first and on its own line, so whatever started the
        // process reads where to connect before anything else is printed.
        println!("{endpoint}");
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

/// `name=location`, or a location alone named after its last segment, as a
/// catalog over the holder the location names.
fn catalog(spelled: &str) -> Result<Catalog> {
    let (name, location) = match spelled.split_once('=') {
        Some((name, location)) if !name.is_empty() && !name.contains(['/', '\\', ':']) => {
            (name.to_owned(), location)
        }
        _ => (last_segment(spelled), spelled),
    };
    Ok(Catalog::new(name, holder(location)?))
}

/// The last segment of a path or a URL, which names a bare catalog.
fn last_segment(location: &str) -> String {
    location
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .filter(|segment| !segment.is_empty())
        .unwrap_or(location)
        .to_owned()
}

/// A folder path, or a URL a holder resolves.
fn holder(location: &str) -> Result<Holder> {
    // `C:\data` spells a drive, not a scheme, so a URL is one with a `//`.
    if location.contains("://") {
        let url = Url::from_str(location)?;
        return Holder::from_url(&url, std::iter::empty::<(String, String)>());
    }
    Holder::folder(location)
}
