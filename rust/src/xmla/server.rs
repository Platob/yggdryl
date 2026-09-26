//! The HTTP endpoint a provider is reached at.
//!
//! XML for Analysis is SOAP 1.1 over HTTP `POST`, and this is exactly that
//! much of a server: a listener, one thread per connection, each request read
//! through [`crate::xml::soap::http`] and answered by the [`Service`], the
//! response streamed back as chunks so an Execute over a large table is never
//! held whole. A `GET` answers a short description of the endpoint, so a
//! browser or a probe learns what it reached; anything else is refused with
//! the status the binding names.
//!
//! ```no_run
//! use yggdryl::holder::Holder;
//! use yggdryl::xmla::{Catalog, Server, Service, ServiceOptions};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let service = Service::new(ServiceOptions::new())
//!     .with_catalog(Catalog::new("market", Holder::folder("/data/market")?));
//! let server = Server::bind(service, "127.0.0.1:8080")?;
//! println!("serving XMLA at {}", server.endpoint());
//! server.serve()?;
//! # Ok(())
//! # }
//! ```

use std::io::{self, BufReader, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::xml::soap::http::{Request, Status, begin_chunked, write_response};
use crate::xml::soap::{self, Fault};

use super::service::Service;

/// How a server accepts requests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerOptions {
    /// The most bytes one request body may carry; a larger one is refused
    /// with `413`.
    pub max_body: usize,
    /// The request path the endpoint answers at; `None` answers at every
    /// path. A `GET` at another path is `404`.
    pub path: Option<String>,
    /// How long an idle connection is kept open for its next request.
    pub idle_timeout: Duration,
}

impl ServerOptions {
    /// The defaults: sixteen mebibytes per request, every path, thirty
    /// seconds idle.
    #[must_use]
    pub fn new() -> Self {
        Self {
            max_body: 16 << 20,
            path: None,
            idle_timeout: Duration::from_secs(30),
        }
    }

    /// Return these options answering only at `path`.
    #[must_use]
    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    /// Return these options bounding a request body.
    #[must_use]
    pub const fn with_max_body(mut self, max_body: usize) -> Self {
        self.max_body = max_body;
        self
    }
}

impl Default for ServerOptions {
    fn default() -> Self {
        Self::new()
    }
}

/// A bound listener and the provider it serves.
#[derive(Debug)]
pub struct Server {
    listener: TcpListener,
    service: Arc<Service>,
    options: ServerOptions,
}

impl Server {
    /// Bind `address` and serve `service` there; `127.0.0.1:0` binds a free
    /// port, which [`Self::local_addr`] answers.
    ///
    /// # Errors
    ///
    /// Returns the bind failure.
    pub fn bind(service: Service, address: impl ToSocketAddrs) -> io::Result<Self> {
        let listener = TcpListener::bind(address)?;
        Ok(Self {
            listener,
            service: Arc::new(service),
            options: ServerOptions::new(),
        })
    }

    /// Return this server accepting requests under `options`.
    #[must_use]
    pub fn with_options(mut self, options: ServerOptions) -> Self {
        self.options = options;
        self
    }

    /// The address bound.
    ///
    /// # Errors
    ///
    /// Returns the socket's failure to name itself.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// The URL a client invokes the methods at.
    #[must_use]
    pub fn endpoint(&self) -> String {
        let address = self
            .listener
            .local_addr()
            .map_or_else(|_| "0.0.0.0:0".to_owned(), |address| address.to_string());
        let path = self.options.path.as_deref().unwrap_or("/xmla");
        format!("http://{address}{path}")
    }

    /// The provider served.
    #[must_use]
    pub fn service(&self) -> &Arc<Service> {
        &self.service
    }

    /// Accept connections until the listener fails, each on its own thread.
    ///
    /// # Errors
    ///
    /// Returns the listener's failure to accept.
    pub fn serve(self) -> io::Result<()> {
        let stopping = Arc::new(AtomicBool::new(false));
        self.accept_loop(&stopping)
    }

    /// Accept connections on a thread of their own, answering a handle that
    /// stops them.
    #[must_use]
    pub fn spawn(self) -> Running {
        let address = self.listener.local_addr().ok();
        let endpoint = self.endpoint();
        let stopping = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&stopping);
        let service = Arc::clone(&self.service);
        let thread = std::thread::spawn(move || {
            let _ = self.accept_loop(&stop);
        });
        Running {
            thread: Some(thread),
            stopping,
            address,
            endpoint,
            service,
        }
    }

    fn accept_loop(&self, stopping: &Arc<AtomicBool>) -> io::Result<()> {
        for connection in self.listener.incoming() {
            if stopping.load(Ordering::SeqCst) {
                break;
            }
            let stream = connection?;
            let service = Arc::clone(&self.service);
            let options = self.options.clone();
            std::thread::spawn(move || serve_connection(&service, stream, &options));
        }
        Ok(())
    }
}

/// A server accepting on its own thread; dropping it stops the listener.
#[derive(Debug)]
pub struct Running {
    thread: Option<JoinHandle<()>>,
    stopping: Arc<AtomicBool>,
    address: Option<SocketAddr>,
    endpoint: String,
    service: Arc<Service>,
}

impl Running {
    /// The address bound.
    #[must_use]
    pub const fn local_addr(&self) -> Option<SocketAddr> {
        self.address
    }

    /// The URL a client invokes the methods at.
    #[must_use]
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// The provider served.
    #[must_use]
    pub fn service(&self) -> &Arc<Service> {
        &self.service
    }

    /// Stop accepting and join the accept thread. Connections already open
    /// finish their exchange on their own threads.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stopping.store(true, Ordering::SeqCst);
        // The accept loop reads the flag after a connection arrives, so one
        // is made to wake it.
        if let Some(address) = self.address {
            let _ = TcpStream::connect_timeout(&address, Duration::from_secs(1))
                .and_then(|stream| stream.shutdown(Shutdown::Both));
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Answer every request one connection carries, until it closes or asks to.
fn serve_connection(service: &Service, stream: TcpStream, options: &ServerOptions) {
    let _ = stream.set_read_timeout(Some(options.idle_timeout));
    let Ok(read_half) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(read_half);
    let mut writer = stream;
    loop {
        let request = match Request::read(&mut reader, options.max_body) {
            Ok(Some(request)) => request,
            Ok(None) => return,
            Err(error) => {
                let _ = write_response(
                    &mut writer,
                    error.status(),
                    "text/plain; charset=utf-8",
                    false,
                    error.reason().as_bytes(),
                );
                return;
            }
        };
        let keep_alive = request.keep_alive();
        if answer(service, &request, &mut writer, options).is_err() || !keep_alive {
            let _ = writer.shutdown(Shutdown::Both);
            return;
        }
    }
}

/// Answer one request on `writer`.
fn answer(
    service: &Service,
    request: &Request,
    writer: &mut TcpStream,
    options: &ServerOptions,
) -> io::Result<()> {
    let keep_alive = request.keep_alive();
    let path = request.target().split('?').next().unwrap_or("/");
    if let Some(expected) = &options.path {
        if path != expected {
            return write_response(
                writer,
                Status::NotFound,
                "text/plain; charset=utf-8",
                keep_alive,
                format!("the XML for Analysis endpoint is {expected}\n").as_bytes(),
            );
        }
    }
    match request.method() {
        "POST" => {}
        "GET" | "HEAD" => {
            let description = format!(
                "{} {}: an XML for Analysis 1.1 provider. POST a SOAP 1.1 Discover or Execute here.\n",
                service.options().provider_name,
                service.options().provider_version
            );
            return write_response(
                writer,
                Status::Ok,
                "text/plain; charset=utf-8",
                keep_alive,
                if request.method() == "HEAD" {
                    b""
                } else {
                    description.as_bytes()
                },
            );
        }
        other => {
            return write_response(
                writer,
                Status::MethodNotAllowed,
                "text/plain; charset=utf-8",
                keep_alive,
                format!("{other} is not how XML for Analysis is invoked; POST a SOAP message\n")
                    .as_bytes(),
            );
        }
    }
    if let Some(content_type) = request.content_type() {
        let base = content_type.split(';').next().unwrap_or("").trim();
        if !base.contains("xml") {
            return write_response(
                writer,
                Status::UnsupportedMediaType,
                "text/plain; charset=utf-8",
                keep_alive,
                format!("expected an XML content type for a SOAP message, got {base}\n").as_bytes(),
            );
        }
    }
    // A request that cannot be read as XMLA is a fault at `500`, as the SOAP
    // HTTP binding requires; one that can is answered at `200` as a stream,
    // the provider writing the fault it earns into the body itself.
    let parsed = super::request::Request::from_bytes(request.body());
    let request = match parsed {
        Ok(request) => request,
        Err(error) => {
            let fault = Fault::client(error.to_string()).with_actor(super::response::ACTOR);
            let mut body = Vec::new();
            super::response::write_fault(&mut body, &fault).map_err(io::Error::other)?;
            return write_response(
                writer,
                Status::InternalServerError,
                soap::CONTENT_TYPE,
                keep_alive,
                &body,
            );
        }
    };
    let chunked = begin_chunked(&mut *writer, Status::Ok, soap::CONTENT_TYPE, keep_alive)?;
    let chunked = service
        .answer(&request, chunked)
        .map_err(io::Error::other)?;
    chunked.finish()?;
    writer.flush()
}
