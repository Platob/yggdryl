//! The HTTP endpoint a provider is reached at.
//!
//! XML for Analysis is SOAP 1.1 over HTTP `POST`, and this is exactly that
//! much of a server: a listener, one thread per connection, each request read
//! through [`crate::soap::http`] and answered by the [`Service`], the
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

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::IOBase;
use crate::holder::Holder;
use crate::soap::http::{Request, Status, begin_chunked, write_response};
use crate::soap::{self, Fault, FaultCode};

use super::service::Service;

/// How a server accepts requests.
#[derive(Debug)]
pub struct ServerOptions {
    /// The most bytes one request body may carry; a larger one is refused
    /// with `413`.
    pub max_body: usize,
    /// The request path the endpoint answers at; `None` answers at every
    /// path. A `GET` at another path is `404`.
    pub path: Option<String>,
    /// How long an idle connection is kept open for its next request.
    pub idle_timeout: Duration,
    /// The folder each exchange is written to, as it went over the wire:
    /// `NNNN-request.http` holds the request's bytes exactly as they were
    /// consumed, `NNNN-response.http` the response's exactly as they were
    /// sent, the interim `100 Continue` and the chunked framing included,
    /// `NNNN` counting from `0000` across every connection in the order the
    /// requests were read. A request file replays through
    /// [`Service::handle`] with its body. `None` traces nothing, and a trace
    /// holds at most one chunk of a response at a time: the rest is appended
    /// as it is sent.
    pub trace: Option<Holder>,
}

impl ServerOptions {
    /// The defaults: sixteen mebibytes per request, every path, thirty
    /// seconds idle, no trace.
    #[must_use]
    pub fn new() -> Self {
        Self {
            max_body: 16 << 20,
            path: None,
            idle_timeout: Duration::from_secs(30),
            trace: None,
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

    /// Return these options writing every exchange under `folder`.
    #[must_use]
    pub fn with_trace(mut self, folder: Holder) -> Self {
        self.trace = Some(folder);
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
    options: Arc<ServerOptions>,
    /// The number the next traced exchange takes, shared by every connection.
    sequence: Arc<AtomicU64>,
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
            options: Arc::new(ServerOptions::new()),
            sequence: Arc::new(AtomicU64::new(0)),
        })
    }

    /// Return this server accepting requests under `options`.
    #[must_use]
    pub fn with_options(mut self, options: ServerOptions) -> Self {
        self.options = Arc::new(options);
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
            let options = Arc::clone(&self.options);
            let sequence = Arc::clone(&self.sequence);
            std::thread::spawn(move || serve_connection(&service, stream, &options, &sequence));
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

/// The content-negotiation flags every SOAP answer carries: MS-SSAS 2.1.2's
/// `NEGO,REQ_SX,REQ_XPRESS,RESP_SX,RESP_XPRESS`, all zero - plain text XML
/// both ways, which a client that offered the binary or the compressed
/// encodings reads as the server declining them.
const NEGOTIATION: [(&str, &str); 1] = [("X-Transport-Caps-Negotiation-Flags", "0,0,0,0,0")];

/// The media type of the text answers: the endpoint's description, a refusal.
const TEXT: &str = "text/plain; charset=utf-8";

/// How much of a response a trace gathers before appending it to its file.
const TRACE_CHUNK: usize = 64 * 1024;

/// Answer a message that is no XMLA request with a `Client` fault carrying
/// the XMLA `Error`, at `200` like every fault.
fn refuse<W: Write>(writer: &mut W, keep_alive: bool, description: String) -> io::Result<()> {
    let fault = super::response::fault(
        FaultCode::Client,
        super::response::XmlaError::new(super::service::code::BAD_REQUEST, description.clone()),
    )
    .unwrap_or_else(|_| Fault::client(description).with_actor(super::response::ACTOR));
    let mut body = Vec::new();
    super::response::write_fault(&mut body, &[], &fault).map_err(io::Error::other)?;
    write_response(
        writer,
        Status::Ok,
        soap::CONTENT_TYPE,
        keep_alive,
        &NEGOTIATION,
        &body,
    )
}

/// Answer every request one connection carries, until it closes or asks to.
///
/// Under a trace, a request is numbered once it has been read - a
/// connection that opens and closes without one takes no number - its bytes
/// are written, and the response leaf takes what the tee sends from then
/// on, the interim status it may already hold included. A trace that cannot
/// be written closes the connection: a trace is asked for to be read, and a
/// gap in it would be read as an exchange that never happened.
fn serve_connection(
    service: &Service,
    stream: TcpStream,
    options: &ServerOptions,
    sequence: &AtomicU64,
) {
    let _ = stream.set_read_timeout(Some(options.idle_timeout));
    let Ok(read_half) = stream.try_clone() else {
        return;
    };
    let tracing = options.trace.is_some();
    let mut reader = Recording::new(BufReader::new(read_half), tracing);
    let mut writer = Tee::new(stream, tracing);
    loop {
        let read = Request::read(&mut reader, &mut writer, options.max_body);
        if matches!(read, Ok(None)) {
            return;
        }
        if let Some(folder) = &options.trace {
            let number = sequence.fetch_add(1, Ordering::Relaxed);
            let opened = trace_request(folder, number, &reader.recorded())
                .and_then(|response| writer.attach(response));
            if opened.is_err() {
                let _ = writer.inner.shutdown(Shutdown::Both);
                return;
            }
        }
        let keep_alive = match read {
            Ok(Some(request)) => {
                let keep_alive = request.keep_alive();
                answer(service, &request, &mut writer, options).map(|()| keep_alive)
            }
            Ok(None) => unreachable!("answered above"),
            Err(error) => write_response(
                &mut writer,
                error.status(),
                TEXT,
                false,
                &[],
                error.reason().as_bytes(),
            )
            .map(|()| false),
        };
        let traced = writer.detach();
        if !matches!((keep_alive, traced), (Ok(true), Ok(()))) {
            let _ = writer.inner.shutdown(Shutdown::Both);
            return;
        }
    }
}

/// Write exchange `number`'s request under `folder` and hand back the leaf
/// its response is appended to, created empty so a failed answer still
/// leaves the pair.
fn trace_request(folder: &Holder, number: u64, request: &[u8]) -> crate::Result<Holder> {
    folder
        .child_by_path(&format!("{number:04}-request.http"))?
        .write_all_bytes(request)?;
    let mut response = folder.child_by_path(&format!("{number:04}-response.http"))?;
    response.write_all_bytes(&[])?;
    Ok(response)
}

/// Answer one request on `writer`.
fn answer<W: Write>(
    service: &Service,
    request: &Request,
    writer: &mut W,
    options: &ServerOptions,
) -> io::Result<()> {
    let keep_alive = request.keep_alive();
    let path = request.target().split('?').next().unwrap_or("/");
    if let Some(expected) = &options.path {
        if path != expected {
            return write_response(
                writer,
                Status::NotFound,
                TEXT,
                keep_alive,
                &[],
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
                TEXT,
                keep_alive,
                &[],
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
                TEXT,
                keep_alive,
                &[],
                format!("{other} is not how XML for Analysis is invoked; POST a SOAP message\n")
                    .as_bytes(),
            );
        }
    }
    // Every fault goes out at `200` in a SOAP body carrying the XMLA `Error`,
    // the way the reference providers answer and the way XMLA clients read:
    // a client such as xmla4js parses a fault only out of a `2xx` answer,
    // and SOAP 1.1's `500` for a fault is what none of them sends.
    if let Some(content_type) = request.content_type() {
        let base = content_type.split(';').next().unwrap_or("").trim();
        if !base.contains("xml") {
            return refuse(
                writer,
                keep_alive,
                format!("expected an XML content type for a SOAP message, got {base}"),
            );
        }
    }
    // The service is the one door the body goes through: it parses the
    // message and answers it, or writes the fault the parse earns.
    let chunked = begin_chunked(
        &mut *writer,
        Status::Ok,
        soap::CONTENT_TYPE,
        keep_alive,
        &NEGOTIATION,
    )?;
    let chunked = service
        .handle(request.body(), chunked)
        .map_err(io::Error::other)?;
    chunked.finish()?;
    writer.flush()
}

/// A reader that keeps the bytes it hands out, so a request is traced
/// exactly as it was consumed: the read-ahead the buffer beneath holds for
/// the next request is not part of this one. Holding nothing when no trace
/// is asked for, it reads straight through.
struct Recording<R: BufRead> {
    inner: R,
    kept: Option<Vec<u8>>,
}

impl<R: BufRead> Recording<R> {
    fn new(inner: R, keep: bool) -> Self {
        Self {
            inner,
            kept: keep.then(Vec::new),
        }
    }

    /// The bytes consumed since they were last taken.
    fn recorded(&mut self) -> Vec<u8> {
        self.kept.as_mut().map(std::mem::take).unwrap_or_default()
    }
}

impl<R: BufRead> Read for Recording<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.kept.is_none() {
            return self.inner.read(buffer);
        }
        let available = self.inner.fill_buf()?;
        let taken = available.len().min(buffer.len());
        buffer[..taken].copy_from_slice(&available[..taken]);
        self.consume(taken);
        Ok(taken)
    }
}

impl<R: BufRead> BufRead for Recording<R> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.inner.fill_buf()
    }

    fn consume(&mut self, amount: usize) {
        if let Some(kept) = &mut self.kept {
            // What is consumed is the front of the buffer `fill_buf` last
            // handed out, which asking again returns without a read.
            if let Ok(available) = self.inner.fill_buf() {
                kept.extend_from_slice(&available[..amount.min(available.len())]);
            }
        }
        self.inner.consume(amount);
    }
}

/// A writer that copies what it sends into the exchange's trace leaf, one
/// chunk at a time: the copy gathers up to [`TRACE_CHUNK`] bytes, is
/// appended to the leaf once it holds that much, and is emptied into it
/// when the exchange ends. What is sent before a leaf is attached - the
/// interim status - waits in the copy for it.
struct Tee<W: Write> {
    inner: W,
    copy: Option<TraceCopy>,
}

struct TraceCopy {
    pending: Vec<u8>,
    leaf: Option<Holder>,
}

impl TraceCopy {
    fn spill(&mut self) -> crate::Result<()> {
        if let Some(leaf) = &mut self.leaf {
            if !self.pending.is_empty() {
                leaf.append_bytes(&self.pending)?;
                self.pending.clear();
            }
        }
        Ok(())
    }
}

impl<W: Write> Tee<W> {
    fn new(inner: W, keep: bool) -> Self {
        Self {
            inner,
            copy: keep.then(|| TraceCopy {
                pending: Vec::new(),
                leaf: None,
            }),
        }
    }

    /// Send the copy to `leaf` from here on, what is already gathered first.
    fn attach(&mut self, leaf: Holder) -> crate::Result<()> {
        if let Some(copy) = &mut self.copy {
            copy.leaf = Some(leaf);
            copy.spill()?;
        }
        Ok(())
    }

    /// Append what is gathered and let the leaf go: the exchange is over.
    fn detach(&mut self) -> crate::Result<()> {
        if let Some(copy) = &mut self.copy {
            copy.spill()?;
            copy.leaf = None;
        }
        Ok(())
    }
}

impl<W: Write> Write for Tee<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let sent = self.inner.write(bytes)?;
        if let Some(copy) = &mut self.copy {
            copy.pending.extend_from_slice(&bytes[..sent]);
            if copy.pending.len() >= TRACE_CHUNK {
                copy.spill().map_err(io::Error::other)?;
            }
        }
        Ok(sent)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
