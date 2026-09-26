#![allow(dead_code)]
//! `rust/tests/support/http_server.rs`: the in-process HTTP/1.1 server the
//! `http` suites and benchmarks run against.
//!
//! [`HttpServer`] speaks enough HTTP/1.1 - keep-alive, `Content-Length` and
//! chunked request bodies, `Expect: 100-continue` - to exercise a client end
//! to end over a real loopback socket, and every answer it gives is scripted
//! from the test: resources (`PUT` stores, `GET`/`HEAD` read, `DELETE`
//! removes), byte ranges with their validators, chunked and content-coded
//! bodies, redirects, fixed statuses with `Retry-After`, severed transfers,
//! pagination in four shapes, Basic authentication, cookies and rate-limit
//! headers. Every request is recorded so a test can count and inspect what
//! went on the wire. It is a leaf file included with `#[path]` from the `http`
//! test harness and from benchmarks, so it names nothing of the crate.
//!
//! Every answer is deterministic: `ETag` is the quoted FNV-1a hash of the
//! bytes served, `Last-Modified` is one fixed instant, and a pagination
//! cursor is the index of the page it names.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;

use base64::Engine as _;

/// `Last-Modified` of every resource, as RFC 9110 spells an HTTP-date.
pub const LAST_MODIFIED: &str = "Wed, 01 Jan 2020 00:00:00 GMT";
/// `Content-Type` of a resource stored without one.
pub const DEFAULT_CONTENT_TYPE: &str = "application/octet-stream";
/// Body bytes a [`HttpServer::fail_after`] answer sends before the socket is
/// cut; a shorter body is refused before any byte of the answer.
pub const FAIL_AFTER_BYTES: usize = 1024;
/// Payload bytes per chunk of a [`HttpServer::set_chunked`] answer.
pub const CHUNK_SIZE: usize = 4096;
/// The realm every `WWW-Authenticate` challenge names.
pub const REALM: &str = "test";
/// Longest request or header line accepted; a longer one is malformed.
const MAX_LINE: u64 = 8192;
/// Most header lines accepted on one request.
const MAX_HEADERS: usize = 256;
/// Largest request body accepted; a longer one is malformed.
const MAX_BODY: usize = 256 << 20;
/// The methods a resource answers, as `Allow` lists them.
const ALLOW: &str = "GET, HEAD, PUT, POST, PATCH, DELETE, OPTIONS";

/// One request the server handled, as a test inspects it.
#[derive(Clone, Debug)]
pub struct Recorded {
    /// The HTTP method as sent.
    pub method: String,
    /// The request path exactly as sent (percent escapes retained), without
    /// the query.
    pub path: String,
    /// The query, percent-decoded, in wire order.
    pub query: Vec<(String, String)>,
    /// The request headers with lowercase names, in wire order.
    pub headers: Vec<(String, String)>,
    /// Bytes in the request body; the body itself is not retained.
    pub body_len: usize,
    /// The status the server answered; `0` when the connection was closed
    /// before any byte of the answer ([`HttpServer::fail_after`]).
    pub status: u16,
}

impl Recorded {
    /// The first header named `name` (lower case), as sent.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header, _)| header == name)
            .map(|(_, value)| value.as_str())
    }

    /// The first query parameter named `name`, percent-decoded.
    pub fn query(&self, name: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(parameter, _)| parameter == name)
            .map(|(_, value)| value.as_str())
    }
}

/// How a paginated path tells the client where the next page is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageMode {
    /// A `Link: <url>; rel="next"` header; the page is selected by the
    /// `page` query parameter, `0` when absent.
    Link,
    /// A JSON body `{"items":[...],"next_cursor":"c1"}`, the cursor read back
    /// from the `cursor` query parameter; the last page carries `null`.
    Cursor,
    /// Rows selected by the `offset` query parameter (and a `limit`, else the
    /// first page's row count) in a JSON body `{"items":[...],"total":N}`.
    Offset,
    /// A JSON body `{"items":[...],"next":"<absolute url>"}`, `null` on the
    /// last page; the page is selected by the `page` query parameter.
    Url,
}

/// The test server: a loopback listener, its accept thread, and the scripted
/// answers every handled request reads. Dropping it stops the listener.
pub struct HttpServer {
    /// Shared with the accept thread and every connection thread.
    inner: Arc<Inner>,
    /// The accept thread, joined on drop.
    accept: Option<JoinHandle<()>>,
}

impl HttpServer {
    /// Bind `127.0.0.1:0` and start accepting.
    ///
    /// # Panics
    ///
    /// When the loopback listener cannot be bound.
    pub fn start() -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind a loopback listener");
        let address = listener
            .local_addr()
            .expect("a bound listener has an address");
        let inner = Arc::new(Inner::new(address));
        let shared = Arc::clone(&inner);
        let accept = std::thread::spawn(move || {
            for connection in listener.incoming() {
                if shared.stopping.load(Ordering::SeqCst) {
                    break;
                }
                if let Ok(stream) = connection {
                    let inner = Arc::clone(&shared);
                    inner.connections.fetch_add(1, Ordering::Relaxed);
                    std::thread::spawn(move || serve(&inner, stream));
                }
            }
        });
        Self {
            inner,
            accept: Some(accept),
        }
    }

    /// `http://127.0.0.1:PORT`.
    pub fn endpoint(&self) -> String {
        self.inner.endpoint()
    }

    /// `http://127.0.0.1:PORT/path`; a path without its leading slash gets
    /// one.
    pub fn url(&self, path: &str) -> String {
        self.inner.url(path)
    }

    /// The bound port.
    pub fn port(&self) -> u16 {
        self.inner.address.port()
    }

    /// Connections accepted so far, so a test can see whether a client pools
    /// them.
    pub fn connections(&self) -> usize {
        self.inner.connections.load(Ordering::Relaxed)
    }

    /// Store `bytes` under `path` with `content_type`, as a `PUT` would;
    /// `None` is [`DEFAULT_CONTENT_TYPE`].
    pub fn put_resource(&self, path: &str, bytes: &[u8], content_type: Option<&str>) {
        self.inner.state().resources.insert(
            path.to_owned(),
            Resource {
                bytes: bytes.to_vec(),
                content_type: content_type.unwrap_or(DEFAULT_CONTENT_TYPE).to_owned(),
            },
        );
    }

    /// What a `PUT` or [`put_resource`](Self::put_resource) stored under
    /// `path`: the bytes and the `Content-Type` recorded with them.
    pub fn resource(&self, path: &str) -> Option<(Vec<u8>, String)> {
        self.inner
            .state()
            .resources
            .get(path)
            .map(|resource| (resource.bytes.clone(), resource.content_type.clone()))
    }

    /// Whether `Range` is honoured on `path` (`true`, the default: `206` with
    /// `Content-Range`, `Accept-Ranges: bytes`, `ETag`, `Last-Modified`;
    /// `If-Range` honoured with `412` on a mismatch; `416` past the end). With
    /// `false` a `Range` is ignored, the whole body answers `200` and no
    /// `Accept-Ranges` is sent.
    pub fn set_ranges(&self, path: &str, ranges: bool) {
        self.inner.script(path, |script| script.no_ranges = !ranges);
    }

    /// Answer `path` with `Transfer-Encoding: chunked` and no
    /// `Content-Length`, in [`CHUNK_SIZE`] chunks.
    pub fn set_chunked(&self, path: &str, chunked: bool) {
        self.inner.script(path, |script| script.chunked = chunked);
    }

    /// Serve the resource at `path` coded with `Content-Encoding: <coding>`;
    /// `gzip`, `deflate` (zlib-framed, as HTTP spells it) or `zstd`. The
    /// ranges, the length and the `ETag` are then those of the coded bytes.
    ///
    /// # Panics
    ///
    /// On a coding that is none of the three.
    pub fn set_encoding(&self, path: &str, coding: &str) {
        assert!(
            matches!(coding, "gzip" | "deflate" | "zstd"),
            "unsupported content coding {coding:?}"
        );
        self.inner
            .script(path, |script| script.encoding = Some(coding.to_owned()));
    }

    /// Answer every request of `path` with `status` and `Location: location`.
    pub fn redirect(&self, path: &str, status: u16, location: &str) {
        self.inner.script(path, |script| {
            script.redirect = Some((status, location.to_owned()))
        });
    }

    /// Answer every request of `path` with `status` and an empty body,
    /// `Retry-After: retry_after` when given, until [`reset`](Self::reset).
    pub fn set_status(&self, path: &str, status: u16, retry_after: Option<&str>) {
        self.inner.script(path, |script| {
            script.status = Some((status, retry_after.map(str::to_owned)))
        });
    }

    /// Answer the next `times` requests of `path` with `status` (and
    /// `Retry-After: retry_after` when given), then serve it as scripted.
    pub fn fail_status(&self, path: &str, status: u16, retry_after: Option<&str>, times: u32) {
        let retry_after = retry_after.map(str::to_owned);
        self.inner.script(path, |script| {
            script.failing_status = Some((status, retry_after, times))
        });
    }

    /// Sever the next `count` answers of `path`: a body longer than
    /// [`FAIL_AFTER_BYTES`] is cut after that many bytes, a shorter one is
    /// refused before any byte of the answer.
    pub fn fail_after(&self, path: &str, count: u32) {
        self.inner.script(path, |script| script.fail_after = count);
    }

    /// Cut the next answer of `path` after `byte` body bytes: the head goes
    /// out whole, declaring the full length, and the socket closes there.
    pub fn cut_body_at(&self, path: &str, byte: usize) {
        self.inner.script(path, |script| script.cut_at = Some(byte));
    }

    /// Paginate `GET path`: each of `pages` is the JSON text of that page's
    /// `items` array, and `mode` says how the next page is announced.
    ///
    /// # Panics
    ///
    /// When a page is not a JSON array.
    pub fn paginate(&self, path: &str, pages: Vec<String>, mode: PageMode) {
        let rows = pages
            .iter()
            .map(|page| {
                json_array_items(page).unwrap_or_else(|| panic!("page is not a JSON array: {page}"))
            })
            .collect();
        self.inner.script(path, |script| {
            script.pages = Some(Paginated { pages, rows, mode })
        });
    }

    /// Answer `path` with `401` and `WWW-Authenticate: Basic realm="test"`
    /// unless the `Authorization` header is the Basic credential of `user`
    /// and `password`.
    pub fn require_basic(&self, path: &str, user: &str, password: &str) {
        self.inner.script(path, |script| {
            script.basic = Some(basic_credential(user, password))
        });
    }

    /// Carry `Set-Cookie: header` on every answer of `path`. Every request's
    /// `Cookie` header is recorded ([`cookies`](Self::cookies)).
    pub fn set_cookie(&self, path: &str, header: &str) {
        self.inner
            .script(path, |script| script.cookie = Some(header.to_owned()));
    }

    /// Carry `RateLimit-Remaining`, `RateLimit-Reset`,
    /// `X-RateLimit-Remaining` and `X-RateLimit-Reset` on the next answer
    /// of `path`, the reset in delta seconds.
    pub fn rate_limit(&self, path: &str, remaining: u64, reset_seconds: u64) {
        self.inner.script(path, |script| {
            script.rate_limit = Some((remaining, reset_seconds))
        });
    }

    /// Forget every script of `path`: the resource stays, ranges are honoured
    /// again, nothing is redirected, refused, cut, paginated, challenged or
    /// decorated.
    pub fn reset(&self, path: &str) {
        self.inner.state().scripts.remove(path);
    }

    /// The `Cookie` header of every recorded request that carried one, in
    /// order.
    pub fn cookies(&self) -> Vec<String> {
        self.inner
            .log()
            .iter()
            .filter_map(|recorded| recorded.header("cookie").map(str::to_owned))
            .collect()
    }

    /// Every request handled since the last [`clear_requests`](Self::clear_requests),
    /// in order, while recording was on.
    pub fn requests(&self) -> Vec<Recorded> {
        self.inner.log().clone()
    }

    /// Requests handled since the last [`clear_requests`](Self::clear_requests),
    /// recording or not.
    pub fn request_count(&self) -> usize {
        self.inner.count.load(Ordering::Relaxed)
    }

    /// Forget the recorded requests and zero the count.
    pub fn clear_requests(&self) {
        self.inner.log().clear();
        self.inner.count.store(0, Ordering::Relaxed);
    }

    /// Record handled requests (`true`, the default) or only count them.
    pub fn set_recording(&self, recording: bool) {
        self.inner.recording.store(recording, Ordering::SeqCst);
    }
}

impl Drop for HttpServer {
    fn drop(&mut self) {
        self.inner.stopping.store(true, Ordering::SeqCst);
        // A queued connection may let accept close the listener before this
        // wake-up connects. Bound the refused-connect delay on Windows.
        drop(TcpStream::connect_timeout(
            &self.inner.address,
            std::time::Duration::from_millis(100),
        ));
        if let Some(accept) = self.accept.take() {
            drop(accept.join());
        }
    }
}

/// What the accept thread and the connection threads share.
struct Inner {
    /// The bound loopback address.
    address: SocketAddr,
    /// Raised by `Drop` so the accept loop exits on its wake-up connection.
    stopping: AtomicBool,
    /// Append handled requests to `log`.
    recording: AtomicBool,
    /// Requests handled, recording or not.
    count: AtomicUsize,
    /// Connections accepted.
    connections: AtomicUsize,
    /// The resources and the scripts that shape answers.
    state: Mutex<State>,
    /// The request log.
    log: Mutex<Vec<Recorded>>,
}

impl Inner {
    fn new(address: SocketAddr) -> Self {
        Self {
            address,
            stopping: AtomicBool::new(false),
            recording: AtomicBool::new(true),
            count: AtomicUsize::new(0),
            connections: AtomicUsize::new(0),
            state: Mutex::new(State::default()),
            log: Mutex::new(Vec::new()),
        }
    }

    fn endpoint(&self) -> String {
        format!("http://{}", self.address)
    }

    fn url(&self, path: &str) -> String {
        let slash = if path.starts_with('/') { "" } else { "/" };
        format!("http://{}{slash}{path}", self.address)
    }

    /// The store, poison ignored: a panicking connection thread must not take
    /// the fixture down with it.
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn log(&self) -> MutexGuard<'_, Vec<Recorded>> {
        self.log.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Edit the script of `path`, created empty on first use.
    fn script(&self, path: &str, edit: impl FnOnce(&mut Script)) {
        edit(self.state().scripts.entry(path.to_owned()).or_default());
    }

    /// Answer one request under the lock, then record it.
    fn handle(&self, request: &Request) -> (Response, Fault) {
        let (response, fault) = {
            let mut state = self.state();
            state.answer(request, &self.endpoint())
        };
        self.count.fetch_add(1, Ordering::Relaxed);
        if self.recording.load(Ordering::SeqCst) {
            self.log().push(Recorded {
                method: request.method.clone(),
                path: request.path.clone(),
                query: request.query.clone(),
                headers: request.headers.clone(),
                body_len: request.body.len(),
                status: if fault == Fault::Close {
                    0
                } else {
                    response.status
                },
            });
        }
        (response, fault)
    }
}

/// What a test scripted: the stored resources and, per path, how the answer
/// is shaped.
#[derive(Default)]
struct State {
    /// Stored bodies by path, as given to `put_resource` or as `PUT` spelled
    /// them (percent-decoded).
    resources: BTreeMap<String, Resource>,
    /// Per-path answer shaping.
    scripts: BTreeMap<String, Script>,
}

/// One stored body and its declared type.
struct Resource {
    bytes: Vec<u8>,
    content_type: String,
}

/// Everything a test may script on one path. Every field's default is "no
/// script", so an absent entry and an empty one answer alike.
#[derive(Default)]
struct Script {
    /// Ignore `Range` and send no `Accept-Ranges`.
    no_ranges: bool,
    /// Frame the answer body as chunks.
    chunked: bool,
    /// Serve the resource coded with this `Content-Encoding`.
    encoding: Option<String>,
    /// Status and `Location` every request answers.
    redirect: Option<(u16, String)>,
    /// Status and `Retry-After` every request answers.
    status: Option<(u16, Option<String>)>,
    /// Status, `Retry-After` and how many more requests answer them.
    failing_status: Option<(u16, Option<String>, u32)>,
    /// Answers left to sever.
    fail_after: u32,
    /// Cut the next answer after this many body bytes.
    cut_at: Option<usize>,
    /// The pages a `GET` walks.
    pages: Option<Paginated>,
    /// The `Authorization` value that passes the challenge.
    basic: Option<String>,
    /// `Set-Cookie` on every answer.
    cookie: Option<String>,
    /// Rate-limit headers on the next answer: remaining, reset seconds.
    rate_limit: Option<(u64, u64)>,
}

/// The pages of one paginated path.
struct Paginated {
    /// Each page's `items` array as JSON text.
    pages: Vec<String>,
    /// Each page's rows, split out of the array once, for the offset mode.
    rows: Vec<Vec<String>>,
    mode: PageMode,
}

impl State {
    /// The scripted answer to `request` and the fault to apply while writing
    /// it.
    fn answer(&mut self, request: &Request, endpoint: &str) -> (Response, Fault) {
        let path = percent_decode(&request.path);
        let State { resources, scripts } = self;
        let script = scripts.entry(path.clone()).or_default();
        let mut response =
            script
                .refusal(request)
                .unwrap_or_else(|| match request.method.as_str() {
                    "GET" | "HEAD" => match &script.pages {
                        Some(pages) => {
                            paginated(pages, request, &format!("{endpoint}{}", request.path))
                        }
                        None => match resources.get(&path) {
                            Some(resource) => serve_resource(resource, script, request),
                            None => Response::new(404),
                        },
                    },
                    "PUT" => {
                        let created = !resources.contains_key(&path);
                        resources.insert(
                            path.clone(),
                            Resource {
                                bytes: request.body.clone(),
                                content_type: request
                                    .header("content-type")
                                    .unwrap_or(DEFAULT_CONTENT_TYPE)
                                    .to_owned(),
                            },
                        );
                        Response::new(if created { 201 } else { 204 })
                    }
                    "DELETE" => Response::new(if resources.remove(&path).is_some() {
                        204
                    } else {
                        404
                    }),
                    "POST" | "PATCH" => {
                        let mut echo = Response::new(200).with_body(request.body.clone());
                        if let Some(content_type) = request.header("content-type") {
                            echo = echo.with_header("Content-Type", content_type);
                        }
                        echo
                    }
                    "OPTIONS" => Response::new(204).with_header("Allow", ALLOW),
                    _ => Response::new(405).with_header("Allow", ALLOW),
                });
        if let Some(cookie) = &script.cookie {
            response = response.with_header("Set-Cookie", cookie);
        }
        if let Some((remaining, reset)) = script.rate_limit.take() {
            response = response
                .with_header("RateLimit-Remaining", &remaining.to_string())
                .with_header("RateLimit-Reset", &reset.to_string())
                .with_header("X-RateLimit-Remaining", &remaining.to_string())
                .with_header("X-RateLimit-Reset", &reset.to_string());
        }
        response.chunked = script.chunked;
        let fault = if let Some(byte) = script.cut_at.take() {
            Fault::CutAt(byte)
        } else if script.fail_after > 0 {
            script.fail_after -= 1;
            if request.method != "HEAD" && response.body.len() > FAIL_AFTER_BYTES {
                Fault::CutAt(FAIL_AFTER_BYTES)
            } else {
                Fault::Close
            }
        } else {
            Fault::None
        };
        (response, fault)
    }
}

impl Script {
    /// The answer that pre-empts the method's own: the Basic challenge, a
    /// failing status, a fixed status, a redirect.
    fn refusal(&mut self, request: &Request) -> Option<Response> {
        if let Some(expected) = &self.basic {
            if request.header("authorization") != Some(expected.as_str()) {
                return Some(
                    Response::new(401)
                        .with_header("WWW-Authenticate", &format!("Basic realm=\"{REALM}\"")),
                );
            }
        }
        if let Some((status, retry_after, times)) = &mut self.failing_status {
            if *times > 0 {
                *times -= 1;
                let response = Response::new(*status);
                return Some(match retry_after {
                    Some(value) => response.with_header("Retry-After", value),
                    None => response,
                });
            }
        }
        if let Some((status, retry_after)) = &self.status {
            let response = Response::new(*status);
            return Some(match retry_after {
                Some(value) => response.with_header("Retry-After", value),
                None => response,
            });
        }
        if let Some((status, location)) = &self.redirect {
            return Some(Response::new(*status).with_header("Location", location));
        }
        None
    }
}

/// The whole resource, or the single byte range of `Range` as `206`; a range
/// starting at or past the end (any range on an empty body) is `416`, an
/// `If-Range` naming another validator `412`. An unparsable `Range` is
/// ignored, as RFC 9110 allows.
fn serve_resource(resource: &Resource, script: &Script, request: &Request) -> Response {
    let coded;
    let bytes: &[u8] = match &script.encoding {
        Some(coding) => {
            coded = encode(coding, &resource.bytes);
            &coded
        }
        None => &resource.bytes,
    };
    let etag = etag_of(bytes);
    let mut response = Response::new(200)
        .with_header("Content-Type", &resource.content_type)
        .with_header("ETag", &etag)
        .with_header("Last-Modified", LAST_MODIFIED);
    if let Some(coding) = &script.encoding {
        response = response.with_header("Content-Encoding", coding);
    }
    if script.no_ranges {
        return response.with_body(bytes.to_vec());
    }
    response = response.with_header("Accept-Ranges", "bytes");
    let Some(range) = request.header("range").and_then(ByteRange::parse) else {
        return response.with_body(bytes.to_vec());
    };
    if let Some(if_range) = request.header("if-range") {
        let validator = if_range.trim();
        if validator != etag && validator != LAST_MODIFIED {
            return Response::new(412);
        }
    }
    let size = bytes.len();
    let Some((start, end)) = range.resolve(size) else {
        return Response::new(416).with_header("Content-Range", &format!("bytes */{size}"));
    };
    response.status = 206;
    response
        .with_header("Content-Range", &format!("bytes {start}-{end}/{size}"))
        .with_body(bytes[start..=end].to_vec())
}

/// One page of a paginated path, selected by the mode's query parameter;
/// `url` is the request's own absolute URL without its query, which the
/// `Link` and `next` spellings extend.
fn paginated(pages: &Paginated, request: &Request, url: &str) -> Response {
    let count = pages.pages.len();
    let json = |body: String| {
        Response::new(200)
            .with_header("Content-Type", "application/json")
            .with_body(body.into_bytes())
    };
    let no_such_page = || {
        Response::new(404)
            .with_header("Content-Type", "application/json")
            .with_body(b"{\"error\":\"no such page\"}".to_vec())
    };
    let page_index = |name: &str| -> Option<usize> {
        request
            .query(name)
            .map_or(Some(0), |value| value.parse().ok())
    };
    match pages.mode {
        PageMode::Link => {
            let Some(index) = page_index("page").filter(|index| *index < count) else {
                return no_such_page();
            };
            let mut response = json(format!("{{\"items\":{}}}", pages.pages[index]));
            if index + 1 < count {
                response = response
                    .with_header("Link", &format!("<{url}?page={}>; rel=\"next\"", index + 1));
            }
            response
        }
        PageMode::Url => {
            let Some(index) = page_index("page").filter(|index| *index < count) else {
                return no_such_page();
            };
            let next = if index + 1 < count {
                format!("\"{url}?page={}\"", index + 1)
            } else {
                "null".to_owned()
            };
            json(format!(
                "{{\"items\":{},\"next\":{next}}}",
                pages.pages[index]
            ))
        }
        PageMode::Cursor => {
            let index = match request.query("cursor") {
                None => Some(0),
                Some(cursor) => cursor
                    .strip_prefix('c')
                    .and_then(|index| index.parse().ok()),
            };
            let Some(index) = index.filter(|index| *index < count) else {
                return no_such_page();
            };
            let next = if index + 1 < count {
                format!("\"c{}\"", index + 1)
            } else {
                "null".to_owned()
            };
            json(format!(
                "{{\"items\":{},\"next_cursor\":{next}}}",
                pages.pages[index]
            ))
        }
        PageMode::Offset => {
            let rows: Vec<&str> = pages
                .rows
                .iter()
                .flat_map(|page| page.iter().map(String::as_str))
                .collect();
            let Some(offset) = page_index("offset") else {
                return no_such_page();
            };
            let first_page = pages.rows.first().map_or(0, Vec::len);
            let Some(limit) = request
                .query("limit")
                .map_or(Some(first_page), |value| value.parse().ok())
            else {
                return no_such_page();
            };
            let start = offset.min(rows.len());
            let end = start.saturating_add(limit).min(rows.len());
            json(format!(
                "{{\"items\":[{}],\"total\":{}}}",
                rows[start..end].join(","),
                rows.len()
            ))
        }
    }
}

/// The elements of a JSON array text, each as its own text; `None` when the
/// text is not an array. Commas inside strings, objects and nested arrays do
/// not split.
fn json_array_items(text: &str) -> Option<Vec<String>> {
    let inner = text.trim().strip_prefix('[')?.strip_suffix(']')?;
    if inner.trim().is_empty() {
        return Some(Vec::new());
    }
    let mut items = Vec::new();
    let mut depth = 0_u32;
    let mut in_string = false;
    let mut escaped = false;
    let mut start = 0;
    for (index, byte) in inner.bytes().enumerate() {
        if in_string {
            match byte {
                b'\\' if !escaped => escaped = true,
                b'"' if !escaped => in_string = false,
                _ => escaped = false,
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'[' | b'{' => depth += 1,
            b']' | b'}' => depth = depth.checked_sub(1)?,
            b',' if depth == 0 => {
                items.push(inner[start..index].trim().to_owned());
                start = index + 1;
            }
            _ => {}
        }
    }
    if in_string || depth != 0 {
        return None;
    }
    items.push(inner[start..].trim().to_owned());
    Some(items)
}

/// `bytes` under one HTTP content coding.
///
/// # Panics
///
/// On a coding that is not `gzip`, `deflate` or `zstd`, which `set_encoding`
/// refused already, or when an in-memory encoder fails, which it does not.
fn encode(coding: &str, bytes: &[u8]) -> Vec<u8> {
    match coding {
        "gzip" => {
            let mut encoder =
                flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            encoder.write_all(bytes).expect("encode into memory");
            encoder.finish().expect("finish an in-memory encoder")
        }
        "deflate" => {
            let mut encoder =
                flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
            encoder.write_all(bytes).expect("encode into memory");
            encoder.finish().expect("finish an in-memory encoder")
        }
        "zstd" => zstd::stream::encode_all(bytes, 0).expect("encode into memory"),
        other => panic!("unsupported content coding {other:?}"),
    }
}

/// The `Authorization` value of Basic credentials.
pub fn basic_credential(user: &str, password: &str) -> String {
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"))
    )
}

/// One `Range` header's single byte range.
struct ByteRange {
    /// First byte; `None` for a suffix range.
    start: Option<usize>,
    /// Last byte inclusive, or the suffix length when `start` is `None`.
    end: Option<usize>,
}

impl ByteRange {
    /// `None` for anything but a single well-formed byte range.
    fn parse(header: &str) -> Option<Self> {
        let spec = header.trim().strip_prefix("bytes=")?;
        let (start, end) = spec.split_once('-')?;
        let number = |text: &str| -> Option<Option<usize>> {
            if text.trim().is_empty() {
                Some(None)
            } else {
                text.trim().parse().ok().map(Some)
            }
        };
        let (start, end) = (number(start)?, number(end)?);
        match (start, end) {
            (None, None) => None,
            (Some(start), Some(end)) if end < start => None,
            _ => Some(Self { start, end }),
        }
    }

    /// The inclusive byte bounds inside a body of `size` bytes, or `None`
    /// when unsatisfiable.
    fn resolve(&self, size: usize) -> Option<(usize, usize)> {
        match (self.start, self.end) {
            (Some(start), _) if start >= size => None,
            (Some(start), end) => Some((start, end.map_or(size - 1, |end| end.min(size - 1)))),
            (None, Some(suffix)) if suffix == 0 || size == 0 => None,
            (None, Some(suffix)) => Some((size - suffix.min(size), size - 1)),
            (None, None) => None,
        }
    }
}

/// One HTTP request as parsed off the wire.
struct Request {
    /// As sent.
    method: String,
    /// As sent, without the query.
    path: String,
    /// Percent-decoded, in wire order.
    query: Vec<(String, String)>,
    /// Lowercase names, trimmed values, in wire order.
    headers: Vec<(String, String)>,
    /// The whole body.
    body: Vec<u8>,
    /// Whether the connection stays open after the answer.
    keep_alive: bool,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header, _)| header == name)
            .map(|(_, value)| value.as_str())
    }

    fn query(&self, name: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(parameter, _)| parameter == name)
            .map(|(_, value)| value.as_str())
    }
}

/// Why a connection stops being read.
enum Refusal {
    /// The peer went away, or the request was cut short.
    Closed,
    /// The bytes were not an HTTP request.
    Malformed,
}

/// What happens to an answer on its way out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fault {
    /// Written whole.
    None,
    /// The head and this many body bytes, then the socket closes.
    CutAt(usize),
    /// Nothing written; the socket closes.
    Close,
}

/// Serve one connection until it closes, is severed, or a request is
/// malformed.
fn serve(inner: &Inner, stream: TcpStream) {
    let mut reader = BufReader::new(stream);
    loop {
        let request = match read_request(&mut reader) {
            Ok(request) => request,
            Err(Refusal::Closed) => return,
            Err(Refusal::Malformed) => {
                let response = Response::new(400)
                    .with_header("Content-Type", "text/plain")
                    .with_body(b"malformed HTTP request".to_vec());
                let _ = write_response(reader.get_mut(), false, &response, true, Fault::None);
                return;
            }
        };
        let (response, fault) = inner.handle(&request);
        let written = write_response(
            reader.get_mut(),
            request.method == "HEAD",
            &response,
            !request.keep_alive,
            fault,
        );
        if written.is_err() || !request.keep_alive || fault != Fault::None {
            return;
        }
    }
}

/// One request line, its headers, and its body; `Expect: 100-continue` is
/// acknowledged before the body is read.
fn read_request(reader: &mut BufReader<TcpStream>) -> Result<Request, Refusal> {
    let mut line = String::new();
    // RFC 9112 section 2.2: empty lines before the request line are ignored.
    while line.is_empty() {
        line = read_line(reader)?.ok_or(Refusal::Closed)?;
    }
    let parts: Vec<&str> = line.split_whitespace().collect();
    let [method, target, version] = parts[..] else {
        return Err(Refusal::Malformed);
    };
    if !version.starts_with("HTTP/1.") {
        return Err(Refusal::Malformed);
    }
    let mut headers = Vec::new();
    loop {
        let line = read_line(reader)?.ok_or(Refusal::Closed)?;
        if line.is_empty() {
            break;
        }
        let (name, value) = line.split_once(':').ok_or(Refusal::Malformed)?;
        let name = name.trim();
        if name.is_empty() || headers.len() == MAX_HEADERS {
            return Err(Refusal::Malformed);
        }
        headers.push((name.to_ascii_lowercase(), value.trim().to_owned()));
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let mut request = Request {
        method: method.to_owned(),
        path: path.to_owned(),
        query: parse_query(query),
        headers,
        body: Vec::new(),
        keep_alive: false,
    };
    request.keep_alive = keeps_alive(version, request.header("connection"));
    let expects = request
        .header("expect")
        .is_some_and(|value| value.eq_ignore_ascii_case("100-continue"));
    if expects {
        reader
            .get_mut()
            .write_all(b"HTTP/1.1 100 Continue\r\n\r\n")
            .map_err(|_| Refusal::Closed)?;
    }
    request.body = read_body(reader, &request)?;
    Ok(request)
}

/// One line without its `CRLF`; `None` at end of stream.
fn read_line(reader: &mut BufReader<TcpStream>) -> Result<Option<String>, Refusal> {
    let mut line = Vec::new();
    let read = reader
        .by_ref()
        .take(MAX_LINE)
        .read_until(b'\n', &mut line)
        .map_err(|_| Refusal::Closed)?;
    if read == 0 {
        return Ok(None);
    }
    // Cut short by the length limit or by the peer.
    if line.pop() != Some(b'\n') {
        return Err(Refusal::Malformed);
    }
    if line.last() == Some(&b'\r') {
        line.pop();
    }
    String::from_utf8(line)
        .map(Some)
        .map_err(|_| Refusal::Malformed)
}

/// HTTP/1.1 keeps the connection unless `Connection: close`; HTTP/1.0 closes
/// unless `Connection: keep-alive`.
fn keeps_alive(version: &str, connection: Option<&str>) -> bool {
    let wants = |token: &str| {
        connection.is_some_and(|value| {
            value
                .split(',')
                .any(|part| part.trim().eq_ignore_ascii_case(token))
        })
    };
    if version == "HTTP/1.0" {
        wants("keep-alive")
    } else {
        !wants("close")
    }
}

/// The body framed by `Transfer-Encoding: chunked`, else by
/// `Content-Length`, else empty.
fn read_body(reader: &mut BufReader<TcpStream>, request: &Request) -> Result<Vec<u8>, Refusal> {
    let chunked = request
        .header("transfer-encoding")
        .is_some_and(|value| value.to_ascii_lowercase().contains("chunked"));
    if chunked {
        return read_chunked(reader);
    }
    let Some(length) = request.header("content-length") else {
        return Ok(Vec::new());
    };
    let length: usize = length.trim().parse().map_err(|_| Refusal::Malformed)?;
    if length > MAX_BODY {
        return Err(Refusal::Malformed);
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).map_err(|_| Refusal::Closed)?;
    Ok(body)
}

/// Chunks up to the zero-length one, then the trailer up to its empty line.
fn read_chunked(reader: &mut BufReader<TcpStream>) -> Result<Vec<u8>, Refusal> {
    let mut body = Vec::new();
    loop {
        let line = read_line(reader)?.ok_or(Refusal::Closed)?;
        let size = line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size, 16).map_err(|_| Refusal::Malformed)?;
        if size == 0 {
            while !read_line(reader)?.ok_or(Refusal::Closed)?.is_empty() {}
            return Ok(body);
        }
        if body.len().saturating_add(size) > MAX_BODY {
            return Err(Refusal::Malformed);
        }
        let start = body.len();
        body.resize(start + size, 0);
        reader
            .read_exact(&mut body[start..])
            .map_err(|_| Refusal::Closed)?;
        if !read_line(reader)?.ok_or(Refusal::Closed)?.is_empty() {
            return Err(Refusal::Malformed);
        }
    }
}

fn parse_query(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            (percent_decode(name), percent_decode(value))
        })
        .collect()
}

/// An answer on its way to the wire.
struct Response {
    /// HTTP status.
    status: u16,
    /// Headers besides the framing ones added at write time.
    headers: Vec<(String, String)>,
    /// What follows the headers.
    body: Vec<u8>,
    /// Frame the body as chunks instead of declaring its length.
    chunked: bool,
}

impl Response {
    fn new(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: Vec::new(),
            chunked: false,
        }
    }

    fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    fn with_body(mut self, bytes: Vec<u8>) -> Self {
        self.body = bytes;
        self
    }
}

/// Status line, headers, the framing (`Content-Length`, or
/// `Transfer-Encoding: chunked`; neither on a `204`, a `304` or a `1xx`), and
/// the body unless the request was a `HEAD`. A `HEAD` declares the length its
/// `GET` would carry. A cut sends the head whole and only part of the body,
/// which is what a severed transfer looks like; a close writes nothing.
fn write_response(
    stream: &mut TcpStream,
    head: bool,
    response: &Response,
    close: bool,
    fault: Fault,
) -> io::Result<()> {
    let cut = match fault {
        Fault::None => None,
        Fault::CutAt(byte) => Some(byte),
        Fault::Close => return Ok(()),
    };
    let mut out = format!(
        "HTTP/1.1 {} {}\r\n",
        response.status,
        reason(response.status)
    );
    for (name, value) in &response.headers {
        let _ = write!(out, "{name}: {value}\r\n");
    }
    let framed = !(response.status == 204 || response.status == 304 || response.status < 200);
    if framed {
        if response.chunked {
            out.push_str("Transfer-Encoding: chunked\r\n");
        } else {
            let _ = write!(out, "Content-Length: {}\r\n", response.body.len());
        }
    }
    if close {
        out.push_str("Connection: close\r\n");
    }
    out.push_str("\r\n");
    let mut out = out.into_bytes();
    if !head && framed {
        if response.chunked {
            write_chunked(&mut out, &response.body, cut);
        } else {
            let limit = cut.unwrap_or(response.body.len()).min(response.body.len());
            out.extend_from_slice(&response.body[..limit]);
        }
    }
    stream.write_all(&out)?;
    stream.flush()
}

/// `body` as [`CHUNK_SIZE`] chunks and the terminating one; a cut stops the
/// stream once `cut` payload bytes are out, inside a chunk when the cut falls
/// there, and never writes the terminator.
fn write_chunked(out: &mut Vec<u8>, body: &[u8], cut: Option<usize>) {
    let limit = cut.unwrap_or(body.len()).min(body.len());
    let mut sent = 0;
    for chunk in body.chunks(CHUNK_SIZE) {
        if cut.is_some() && sent >= limit {
            return;
        }
        let _ = write!(out, "{:x}\r\n", chunk.len());
        if sent + chunk.len() > limit {
            out.extend_from_slice(&chunk[..limit - sent]);
            return;
        }
        out.extend_from_slice(chunk);
        out.extend_from_slice(b"\r\n");
        sent += chunk.len();
    }
    if cut.is_none() {
        out.extend_from_slice(b"0\r\n\r\n");
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        100 => "Continue",
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        206 => "Partial Content",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        409 => "Conflict",
        410 => "Gone",
        412 => "Precondition Failed",
        413 => "Content Too Large",
        416 => "Range Not Satisfiable",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Unknown",
    }
}

/// Decode percent escapes; a malformed escape or invalid UTF-8 leaves the
/// text as sent.
fn percent_decode(text: &str) -> String {
    if !text.contains('%') {
        return text.to_owned();
    }
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3])
                .ok()
                .and_then(|hex| u8::from_str_radix(hex, 16).ok());
            if let Some(byte) = hex {
                decoded.push(byte);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8(decoded).unwrap_or_else(|_| text.to_owned())
}

/// 64-bit FNV-1a: cheap, deterministic, and opaque enough for an `ETag`.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The quoted `ETag` of `bytes`, as the server sends it.
pub fn etag_of(bytes: &[u8]) -> String {
    format!("\"{:016x}\"", fnv1a(bytes))
}
