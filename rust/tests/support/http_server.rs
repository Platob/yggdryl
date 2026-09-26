#![allow(dead_code)]
//! `rust/tests/support/http_server.rs`: the in-process HTTP/1.1 server the
//! `http` suites and benchmarks run against, a thin adapter over the crate's
//! own [`yggdryl::http::Server`].
//!
//! [`HttpServer`] keeps the fixture vocabulary the suites were written
//! against and implements every word of it through the server's `mount`,
//! `route`, `inject` and request log: resources live in one
//! `Holder::FsFolder` over a `MemoryFileSystem` mounted at `/`, so an
//! unscripted path is served by the crate's own mount - byte ranges,
//! validators, conditionals, `PUT`, `DELETE` - and a scripted path is one
//! composite route handler answering exactly as the script says: no ranges,
//! chunked or coded bodies, redirects, fixed statuses with `Retry-After`,
//! pagination in four shapes, Basic authentication, cookies and rate-limit
//! headers; a severed transfer is an injected [`Fault`].
//!
//! Every answer is deterministic: `ETag` is the quoted lower-case hex of the
//! XXH3-64 digest of the bytes served, exactly as the mount spells it,
//! `Last-Modified` is what the mount answers for its leaf and
//! [`LAST_MODIFIED`] on a scripted path, and a pagination cursor is the
//! index of the page it names. A suite relies on the method contract here,
//! never on a validator's spelling.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use base64::Engine as _;
use yggdryl::fs::{FsFolder, MemoryFileSystem};
use yggdryl::holder::Holder;
pub use yggdryl::http::Recorded;
use yggdryl::http::{Fault, Headers, Method, Request, Response, Server, Status};
use yggdryl::{DEFAULT_STREAM_BATCH_SIZE, DigestAlgorithm, IOBase, MediaType, Result};

/// `Last-Modified` of every scripted resource, as RFC 9110 spells an
/// HTTP-date.
pub const LAST_MODIFIED: &str = "Wed, 01 Jan 2020 00:00:00 GMT";
/// `Content-Type` of a resource stored without one.
pub const DEFAULT_CONTENT_TYPE: &str = "application/octet-stream";
/// Body bytes a [`HttpServer::fail_after`] answer sends before the socket is
/// cut; a shorter body is refused before any byte of the answer.
pub const FAIL_AFTER_BYTES: usize = 1024;
/// Payload bytes per chunk of a [`HttpServer::set_chunked`] answer.
pub const CHUNK_SIZE: usize = DEFAULT_STREAM_BATCH_SIZE;
/// The realm every `WWW-Authenticate` challenge names.
pub const REALM: &str = "test";

/// The fixture readers the suites use on a recorded request, over the
/// crate's [`Recorded`].
pub trait RecordedExt {
    /// The request header named `name`, case-insensitively.
    fn header(&self, name: &str) -> Option<&str>;
    /// The first query parameter named `name`, percent-decoded.
    fn query(&self, name: &str) -> Option<&str>;
}

impl RecordedExt for Recorded {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name)
    }

    fn query(&self, name: &str) -> Option<&str> {
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

/// The test server: the crate's [`Server`] on a loopback port, its resource
/// store, and the scripts that shape answers. Dropping it stops the
/// listener.
pub struct HttpServer {
    server: Server,
    shared: Arc<Shared>,
}

impl HttpServer {
    /// Bind `127.0.0.1:0` and start accepting.
    ///
    /// # Panics
    ///
    /// When the loopback listener cannot be bound.
    pub fn start() -> Self {
        let server = Server::bind("127.0.0.1:0").expect("bind a loopback listener");
        let memory = Arc::new(MemoryFileSystem::new());
        let root = FsFolder::from_path(memory, "", None).expect("a memory root");
        server
            .mount("/", Holder::from(root.clone()))
            .expect("mount the memory root");
        let shared = Arc::new(Shared {
            root,
            endpoint: format!("http://{}", server.address()),
            state: Mutex::new(State::default()),
        });
        Self { server, shared }
    }

    /// The crate's server underneath, for what the fixture does not spell.
    pub fn server(&self) -> &Server {
        &self.server
    }

    /// `http://127.0.0.1:PORT`.
    pub fn endpoint(&self) -> String {
        self.shared.endpoint.clone()
    }

    /// `http://127.0.0.1:PORT/path`; a path without its leading slash gets
    /// one.
    pub fn url(&self, path: &str) -> String {
        let slash = if path.starts_with('/') { "" } else { "/" };
        format!("{}{slash}{path}", self.shared.endpoint)
    }

    /// The bound port.
    pub fn port(&self) -> u16 {
        self.server.port()
    }

    /// Connections accepted so far, so a test can see whether a client pools
    /// them.
    pub fn connections(&self) -> usize {
        usize::try_from(self.server.connections()).unwrap_or(usize::MAX)
    }

    /// Store `bytes` under `path` with `content_type`, as a `PUT` would;
    /// `None` is [`DEFAULT_CONTENT_TYPE`].
    pub fn put_resource(&self, path: &str, bytes: &[u8], content_type: Option<&str>) {
        let content_type = content_type.unwrap_or(DEFAULT_CONTENT_TYPE);
        self.shared
            .store(path, bytes, content_type)
            .expect("write into the memory root");
        self.server
            .set_media_type(path, media_type_of(content_type))
            .expect("declare the media type on the root mount");
    }

    /// What a `PUT` or [`put_resource`](Self::put_resource) stored under
    /// `path`: the bytes and the `Content-Type` recorded with them - the one
    /// given here or through a scripted path, else the one the name infers.
    pub fn resource(&self, path: &str) -> Option<(Vec<u8>, String)> {
        self.shared.resource(path)
    }

    /// Whether `Range` is honoured on `path` (`true`, the default: `206` with
    /// `Content-Range`, `Accept-Ranges: bytes`, `ETag`, `Last-Modified`;
    /// `If-Range` honoured with `412` on a mismatch; `416` past the end). With
    /// `false` a `Range` is ignored, the whole body answers `200` and no
    /// `Accept-Ranges` is sent.
    pub fn set_ranges(&self, path: &str, ranges: bool) {
        self.script(path, |script| script.no_ranges = !ranges);
    }

    /// Script `path` with nothing changed: a `GET` still serves the resource,
    /// and a `POST` or `PATCH` echoes its body under its `Content-Type`,
    /// which the mount alone answers `405` to.
    pub fn echo(&self, path: &str) {
        self.script(path, |_| {});
    }

    /// Answer `path` with `Transfer-Encoding: chunked` and no
    /// `Content-Length`, in [`CHUNK_SIZE`] chunks.
    pub fn set_chunked(&self, path: &str, chunked: bool) {
        self.script(path, |script| script.chunked = chunked);
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
        self.script(path, |script| script.encoding = Some(coding.to_owned()));
    }

    /// Answer every request of `path` with `status` and `Location: location`.
    pub fn redirect(&self, path: &str, status: u16, location: &str) {
        self.script(path, |script| {
            script.redirect = Some((status, location.to_owned()))
        });
    }

    /// Answer every request of `path` with `status` and an empty body,
    /// `Retry-After: retry_after` when given, until [`reset`](Self::reset).
    pub fn set_status(&self, path: &str, status: u16, retry_after: Option<&str>) {
        self.script(path, |script| {
            script.status = Some((status, retry_after.map(str::to_owned)))
        });
    }

    /// Answer the next `times` requests of `path` with `status` (and
    /// `Retry-After: retry_after` when given), then serve it as scripted.
    pub fn fail_status(&self, path: &str, status: u16, retry_after: Option<&str>, times: u32) {
        let retry_after = retry_after.map(str::to_owned);
        self.script(path, |script| {
            script.failing_status = Some((status, retry_after, times))
        });
    }

    /// Sever the next `count` answers of `path`: a body longer than
    /// [`FAIL_AFTER_BYTES`] is cut after that many bytes, a shorter one is
    /// refused before any byte of the answer. The body's length is read when
    /// this is called.
    pub fn fail_after(&self, path: &str, count: u32) {
        let long = self
            .shared
            .resource(path)
            .is_some_and(|(bytes, _)| bytes.len() > FAIL_AFTER_BYTES);
        let fault = if long {
            Fault::CutBodyAt(FAIL_AFTER_BYTES as u64)
        } else {
            Fault::CloseBeforeAnswer
        };
        self.server.inject(path, fault, count);
    }

    /// Cut the next answer of `path` after `byte` body bytes: the head goes
    /// out whole, declaring the full length, and the socket closes there.
    pub fn cut_body_at(&self, path: &str, byte: usize) {
        self.server.inject(path, Fault::CutBodyAt(byte as u64), 1);
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
        self.script(path, |script| {
            script.pages = Some(Arc::new(Paginated { pages, rows, mode }))
        });
    }

    /// Answer `path` with `401` and `WWW-Authenticate: Basic realm="test"`
    /// unless the `Authorization` header is the Basic credential of `user`
    /// and `password`.
    pub fn require_basic(&self, path: &str, user: &str, password: &str) {
        self.script(path, |script| {
            script.basic = Some(basic_credential(user, password))
        });
    }

    /// Carry `Set-Cookie: header` on every answer of `path`. Every request's
    /// `Cookie` header is recorded ([`cookies`](Self::cookies)).
    pub fn set_cookie(&self, path: &str, header: &str) {
        self.script(path, |script| script.cookie = Some(header.to_owned()));
    }

    /// Carry `RateLimit-Remaining`, `RateLimit-Reset`,
    /// `X-RateLimit-Remaining` and `X-RateLimit-Reset` on the next answer
    /// of `path`, the reset in delta seconds.
    pub fn rate_limit(&self, path: &str, remaining: u64, reset_seconds: u64) {
        self.script(path, |script| {
            script.rate_limit = Some((remaining, reset_seconds))
        });
    }

    /// Forget every script of `path`: the resource stays and the crate's
    /// mount serves it again - ranges honoured, nothing redirected, refused,
    /// paginated, challenged or decorated - and every injected fault is
    /// dropped (the server clears faults for every path at once).
    pub fn reset(&self, path: &str) {
        let path = normalized(path);
        self.shared.state().scripts.remove(&path);
        self.server.unroute(None, &path);
        self.server.clear_faults();
    }

    /// The `Cookie` header of every recorded request that carried one, in
    /// order.
    pub fn cookies(&self) -> Vec<String> {
        self.server
            .requests()
            .iter()
            .filter_map(|recorded| recorded.headers.get("cookie").map(str::to_owned))
            .collect()
    }

    /// Every request handled since the last [`clear_requests`](Self::clear_requests),
    /// in order, while recording was on.
    pub fn requests(&self) -> Vec<Recorded> {
        self.server.requests()
    }

    /// Requests handled since the last [`clear_requests`](Self::clear_requests),
    /// recording or not.
    pub fn request_count(&self) -> usize {
        self.server.request_count()
    }

    /// Forget the recorded requests and zero the count.
    pub fn clear_requests(&self) {
        self.server.clear_requests();
    }

    /// Record handled requests (`true`, the default) or only count them.
    pub fn set_recording(&self, recording: bool) {
        self.server.set_recording(recording);
    }

    /// Edit the script of `path`, created empty on first use, and route the
    /// path to the composite handler that reads it.
    fn script(&self, path: &str, edit: impl FnOnce(&mut Script)) {
        let path = normalized(path);
        {
            let mut state = self.shared.state();
            edit(state.scripts.entry(path.clone()).or_default());
        }
        let shared = Arc::clone(&self.shared);
        let scripted = path.clone();
        self.server.route(None, &path, move |request| {
            shared.answer(&scripted, request)
        });
    }
}

/// `path` as the server spells it: one leading slash, no trailing one.
fn normalized(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return "/".to_owned();
    }
    if trimmed.starts_with('/') {
        trimmed.to_owned()
    } else {
        format!("/{trimmed}")
    }
}

/// `path` below the mounted root.
fn relative(path: &str) -> &str {
    path.trim_start_matches('/')
}

/// The media type a `Content-Type` value declares.
fn media_type_of(content_type: &str) -> MediaType {
    MediaType::from_content_headers(Some(content_type), None).unwrap_or_else(|_| {
        MediaType::from_content_headers(Some(DEFAULT_CONTENT_TYPE), None).expect("octet-stream")
    })
}

/// What the fixture and every route handler share.
struct Shared {
    /// The mounted memory root, holding every resource.
    root: FsFolder,
    /// `http://127.0.0.1:PORT`.
    endpoint: String,
    state: Mutex<State>,
}

/// The scripts and the content types the fixture recorded.
#[derive(Default)]
struct State {
    /// Per-path answer shaping.
    scripts: BTreeMap<String, Script>,
    /// `Content-Type` per path, as `put_resource` or a scripted `PUT` said.
    content_types: BTreeMap<String, String>,
}

impl Shared {
    /// The state, poison ignored: a panicking handler must not take the
    /// fixture down with it.
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The leaf of `path` in the memory root.
    fn leaf(&self, path: &str) -> Result<Holder> {
        Holder::from(self.root.clone()).child_by_path(relative(&normalized(path)))
    }

    /// Store `bytes` under `path` and record `content_type` for it.
    fn store(&self, path: &str, bytes: &[u8], content_type: &str) -> Result<()> {
        let mut leaf = self.leaf(path)?;
        leaf.write_all_bytes(bytes)?;
        self.state()
            .content_types
            .insert(normalized(path), content_type.to_owned());
        Ok(())
    }

    /// The bytes and content type stored under `path`.
    fn resource(&self, path: &str) -> Option<(Vec<u8>, String)> {
        let leaf = self.leaf(path).ok()?;
        if leaf.kind() == yggdryl::IOKind::Unknown {
            return None;
        }
        let bytes = leaf.read_all_bytes().ok()?;
        let content_type = self
            .state()
            .content_types
            .get(&normalized(path))
            .cloned()
            .unwrap_or_else(|| leaf.media_type().base().to_string());
        Some((bytes, content_type))
    }

    /// Remove what is under `path`; whether something was.
    fn remove(&self, path: &str) -> Result<bool> {
        let mut leaf = self.leaf(path)?;
        let existed = leaf.kind() != yggdryl::IOKind::Unknown;
        leaf.remove(false)?;
        self.state().content_types.remove(&normalized(path));
        Ok(existed)
    }

    /// The scripted answer of `path` to `request`.
    fn answer(&self, path: &str, request: &Request) -> Result<Response> {
        let (refusal, script) = {
            let mut state = self.state();
            let script = state.scripts.entry(path.to_owned()).or_default();
            let refusal = script.refusal(request)?;
            let rate_limit = script.rate_limit.take();
            (refusal, script.snapshot(rate_limit))
        };
        let mut response = match refusal {
            Some(response) => response,
            None => match request.method() {
                Method::Get | Method::Head => match &script.pages {
                    Some(pages) => paginated(pages, request, &self.page_url(request))?,
                    None => match self.resource(path) {
                        Some((bytes, content_type)) => {
                            serve_resource(&bytes, &content_type, &script, request)?
                        }
                        None => Response::new(Status::NOT_FOUND),
                    },
                },
                Method::Put => {
                    let created = self.resource(path).is_none();
                    let content_type = request
                        .headers()
                        .content_type()
                        .unwrap_or(DEFAULT_CONTENT_TYPE)
                        .to_owned();
                    self.store(path, request.body().as_bytes(), &content_type)?;
                    Response::new(if created {
                        Status::CREATED
                    } else {
                        Status::NO_CONTENT
                    })
                }
                Method::Delete => Response::new(if self.remove(path)? {
                    Status::NO_CONTENT
                } else {
                    Status::NOT_FOUND
                }),
                Method::Post | Method::Patch => {
                    let mut echo = Response::new(Status::OK).with_body(request.body().clone());
                    if let Some(content_type) = request.headers().content_type() {
                        echo = echo.with_header("Content-Type", content_type)?;
                    }
                    echo
                }
                Method::Options => Response::new(Status::NO_CONTENT).with_header("Allow", ALLOW)?,
                _ => Response::new(Status::METHOD_NOT_ALLOWED).with_header("Allow", ALLOW)?,
            },
        };
        if let Some(cookie) = &script.cookie {
            response = response.with_header("Set-Cookie", cookie)?;
        }
        if let Some((remaining, reset)) = script.rate_limit {
            response = response
                .with_header("RateLimit-Remaining", &remaining.to_string())?
                .with_header("RateLimit-Reset", &reset.to_string())?
                .with_header("X-RateLimit-Remaining", &remaining.to_string())?
                .with_header("X-RateLimit-Reset", &reset.to_string())?;
        }
        if script.chunked && !matches!(response.status(), Status::NO_CONTENT | Status::NOT_MODIFIED)
        {
            response = response.with_header("Transfer-Encoding", "chunked")?;
        }
        Ok(response)
    }

    /// The request's own absolute URL without its query, which the `Link`
    /// and `next` spellings extend.
    fn page_url(&self, request: &Request) -> String {
        let path = request
            .url()
            .path_text(false)
            .map(|path| path.into_owned())
            .unwrap_or_else(|_| "/".to_owned());
        format!("{}{path}", self.endpoint)
    }
}

/// The methods a scripted resource answers, as `Allow` lists them.
const ALLOW: &str = "GET, HEAD, PUT, POST, PATCH, DELETE, OPTIONS";

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
    /// The pages a `GET` walks.
    pages: Option<Arc<Paginated>>,
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

impl Script {
    /// What a handler needs outside the lock: the flags, the pages, the
    /// decorations, with the one-shot rate limit already taken.
    fn snapshot(&self, rate_limit: Option<(u64, u64)>) -> Self {
        Self {
            no_ranges: self.no_ranges,
            chunked: self.chunked,
            encoding: self.encoding.clone(),
            redirect: None,
            status: None,
            failing_status: None,
            pages: self.pages.clone(),
            basic: None,
            cookie: self.cookie.clone(),
            rate_limit,
        }
    }

    /// The answer that pre-empts the method's own: the Basic challenge, a
    /// failing status, a fixed status, a redirect.
    fn refusal(&mut self, request: &Request) -> Result<Option<Response>> {
        if let Some(expected) = &self.basic {
            if request.headers().get("authorization") != Some(expected.as_str()) {
                return Response::new(Status::UNAUTHORIZED)
                    .with_header("WWW-Authenticate", &format!("Basic realm=\"{REALM}\""))
                    .map(Some);
            }
        }
        if let Some((status, retry_after, times)) = &mut self.failing_status {
            if *times > 0 {
                *times -= 1;
                return status_response(*status, retry_after.as_deref()).map(Some);
            }
        }
        if let Some((status, retry_after)) = &self.status {
            return status_response(*status, retry_after.as_deref()).map(Some);
        }
        if let Some((status, location)) = &self.redirect {
            return Response::new(Status::new(*status)?)
                .with_header("Location", location)
                .map(Some);
        }
        Ok(None)
    }
}

/// An empty answer of `status` with `Retry-After` when given.
fn status_response(status: u16, retry_after: Option<&str>) -> Result<Response> {
    let response = Response::new(Status::new(status)?);
    match retry_after {
        Some(value) => response.with_header("Retry-After", value),
        None => Ok(response),
    }
}

/// The whole resource, or the single byte range of `Range` as `206`; a range
/// starting at or past the end (any range on an empty body) is `416`, an
/// `If-Range` naming another validator `412`. An unparsable `Range` is
/// ignored, as RFC 9110 allows.
fn serve_resource(
    stored: &[u8],
    content_type: &str,
    script: &Script,
    request: &Request,
) -> Result<Response> {
    let coded;
    let bytes: &[u8] = match &script.encoding {
        Some(coding) => {
            coded = encode(coding, stored)?;
            &coded
        }
        None => stored,
    };
    let etag = etag_of(bytes);
    let mut response = Response::new(Status::OK)
        .with_header("Content-Type", content_type)?
        .with_header("ETag", &etag)?
        .with_header("Last-Modified", LAST_MODIFIED)?;
    if let Some(coding) = &script.encoding {
        response = response.with_header("Content-Encoding", coding)?;
    }
    if script.no_ranges {
        return Ok(response.with_body(bytes));
    }
    response = response.with_header("Accept-Ranges", "bytes")?;
    let Some(range) = request.headers().get("range").and_then(ByteRange::parse) else {
        return Ok(response.with_body(bytes));
    };
    if let Some(if_range) = request.headers().get("if-range") {
        let validator = if_range.trim();
        if validator != etag && validator != LAST_MODIFIED {
            return Ok(Response::new(Status::PRECONDITION_FAILED));
        }
    }
    let size = bytes.len();
    let Some((start, end)) = range.resolve(size) else {
        return Response::new(Status::RANGE_NOT_SATISFIABLE)
            .with_header("Content-Range", &format!("bytes */{size}"));
    };
    response
        .with_status(Status::PARTIAL_CONTENT)
        .with_header("Content-Range", &format!("bytes {start}-{end}/{size}"))
        .map(|response| response.with_body(&bytes[start..=end]))
}

/// One page of a paginated path, selected by the mode's query parameter;
/// `url` is the request's own absolute URL without its query, which the
/// `Link` and `next` spellings extend.
fn paginated(pages: &Paginated, request: &Request, url: &str) -> Result<Response> {
    let count = pages.pages.len();
    let json = |body: String| -> Result<Response> {
        Response::new(Status::OK)
            .with_header("Content-Type", "application/json")
            .map(|response| response.with_body(body))
    };
    let no_such_page = || -> Result<Response> {
        Response::new(Status::NOT_FOUND)
            .with_header("Content-Type", "application/json")
            .map(|response| response.with_body("{\"error\":\"no such page\"}"))
    };
    let parameters = request.url().parameters(true)?;
    let query = |name: &str| parameters.get(name);
    let page_index =
        |name: &str| -> Option<usize> { query(name).map_or(Some(0), |value| value.parse().ok()) };
    match pages.mode {
        PageMode::Link => {
            let Some(index) = page_index("page").filter(|index| *index < count) else {
                return no_such_page();
            };
            let mut response = json(format!("{{\"items\":{}}}", pages.pages[index]))?;
            if index + 1 < count {
                response = response
                    .with_header("Link", &format!("<{url}?page={}>; rel=\"next\"", index + 1))?;
            }
            Ok(response)
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
            let index = match query("cursor") {
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
            let Some(limit) = query("limit").map_or(Some(first_page), |value| value.parse().ok())
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

/// `bytes` under one HTTP content coding, through the crate's own codecs.
fn encode(coding: &str, bytes: &[u8]) -> Result<Vec<u8>> {
    match coding {
        "gzip" => yggdryl::gzip::dump(bytes),
        "deflate" => yggdryl::zlib::dump(bytes),
        "zstd" => yggdryl::zstd::dump(bytes),
        other => Err(yggdryl::Error::unsupported(
            "a content coding the fixture does not spell",
            other,
        )),
    }
}

/// The `Authorization` value of Basic credentials.
pub fn basic_credential(user: &str, password: &str) -> String {
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"))
    )
}

/// The quoted `ETag` of `bytes`, as the server sends it: the lower-case hex
/// of the XXH3-64 digest.
pub fn etag_of(bytes: &[u8]) -> String {
    let digest = DigestAlgorithm::Xxh3.digest(bytes);
    format!("\"{:016x}\"", digest.as_u64().unwrap_or_default())
}

/// The `Headers` of a `Recorded`, by name, for suites that read several.
pub fn header<'a>(recorded: &'a Recorded, name: &str) -> Option<&'a str> {
    recorded.headers.get(name)
}

/// Every header of a recorded request as `(name, value)` pairs, the way the
/// earlier fixture listed them.
pub fn header_pairs(recorded: &Recorded) -> Vec<(String, String)> {
    recorded
        .headers
        .iter()
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect()
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

/// A `Headers` value from `(name, value)` pairs, for suites building
/// request headers.
pub fn headers<const N: usize>(pairs: [(&str, &str); N]) -> Headers {
    Headers::from_entries(pairs).expect("valid header pairs")
}
