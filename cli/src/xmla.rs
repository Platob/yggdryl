//! `ygg xmla`: the XML for Analysis provider from a terminal.
//!
//! `serve` binds the provider in [`yggdryl::xmla`] to a socket over folders
//! of record media - one catalog per folder, its files the tables, its
//! folders the schemas - and answers Discover and Execute until it is
//! stopped. Nothing here decides what a request means: the command parses
//! its arguments, builds the [`Service`] and hands the [`Server`] its socket,
//! so a table served here and a table read through a handle are the same
//! object read by the same code.

use std::process::ExitCode;

use clap::{Args, Subcommand};
use yggdryl::holder::Holder;
use yggdryl::xmla::{Catalog, Server, ServerOptions, Service, ServiceOptions};
use yggdryl::{Error, Result, Url};

use crate::style;

/// What the provider was asked to do.
#[derive(Subcommand)]
#[command(
    after_help = "Examples:\n  ygg xmla serve market=/data/market reference=/data/reference\n  ygg xmla serve /data/market --bind 0.0.0.0:8080 --path /xmla\n  ygg xmla serve market=s3://bucket/market --writable\n\nA catalog is `name=location`, or a location alone, named after its last segment. A location is a folder path, or a URL a holder resolves.\nEvery record file the folder holds is a table; a folder inside it is a schema whose files are its tables.\nThe first line printed is the endpoint, so a script that started the process knows where to connect."
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
    max_body: usize,
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
        let server = Server::bind(service, self.bind.as_str())
            .map_err(Error::from)?
            .with_options(
                ServerOptions::new()
                    .with_path(self.path.clone())
                    .with_max_body(self.max_body),
            );
        // The endpoint first and on its own line, so whatever started the
        // process reads where to connect before anything else is printed.
        println!("{}", server.endpoint());
        for catalog in server.service().catalogs() {
            style::note(&format!(
                "catalog {} over {}{}",
                catalog.name(),
                catalog.url().map_or_else(String::new, ToString::to_string),
                if self.writable { ", writable" } else { "" }
            ));
        }
        server.serve().map_err(Error::from)?;
        Ok(ExitCode::SUCCESS)
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
