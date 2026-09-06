#![allow(dead_code)]
//! An in-process fake S3 server: the substrate the S3 backend's unit tests
//! and benchmarks run on.
//!
//! [`FakeS3`] speaks enough HTTP/1.1 and enough of the S3 REST API - buckets,
//! objects, ranged reads, `ListObjectsV2`, multipart uploads, bulk deletes,
//! `SigV4` header checks - for the client to be exercised end to end over a
//! real socket, and it records every request so a test can count and inspect
//! what went on the wire. It is a leaf file included with `#[path]` from two
//! places, so it depends on `std` alone and names nothing of the crate.
//!
//! Every answer is deterministic: `ETag` is a quoted FNV-1a hash of the bytes
//! (multipart: `"{hash}-{parts}"`), `Last-Modified` is one fixed instant, and
//! a continuation token is the last key or prefix a page emitted.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::ops::Bound;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;

/// The namespace every S3 result document declares on its root.
const XMLNS: &str = "http://s3.amazonaws.com/doc/2006-03-01/";
/// `Last-Modified` of every object, as RFC 7231 spells it.
const LAST_MODIFIED_HTTP: &str = "Wed, 01 Jan 2020 00:00:00 GMT";
/// The same instant as a listing's `<LastModified>` (ISO 8601).
const LAST_MODIFIED_ISO: &str = "2020-01-01T00:00:00.000Z";
/// `Content-Type` reported for an object stored without one.
const DEFAULT_CONTENT_TYPE: &str = "binary/octet-stream";
/// Longest request or header line accepted; a longer one is malformed.
const MAX_LINE: u64 = 8192;
/// Most header lines accepted on one request.
const MAX_HEADERS: usize = 256;
/// Most keys one `DeleteObjects` accepts, as on AWS.
const MAX_DELETE_KEYS: usize = 1000;
/// `max-keys` when the listing request names none.
const DEFAULT_MAX_KEYS: usize = 1000;

/// One request the server handled, as a test inspects it.
#[derive(Clone, Debug)]
pub struct Recorded {
    /// The HTTP method as sent.
    pub method: String,
    /// The bucket the request addressed, from the path or the `Host` header.
    pub bucket: Option<String>,
    /// The object key, percent-decoded; `None` for bucket-level requests.
    pub key: Option<String>,
    /// The request path exactly as sent (percent escapes retained), without
    /// the query; `key` holds the decoded spelling.
    pub path: String,
    /// The query, percent-decoded, in wire order.
    pub query: Vec<(String, String)>,
    /// The request headers with lowercase names, in wire order.
    pub headers: Vec<(String, String)>,
    /// Bytes in the request body; the body itself is not retained.
    pub body_len: usize,
    /// The status the server answered.
    pub status: u16,
}

/// The fake server: a loopback listener, its accept thread, and the store
/// every handled request operates on. Dropping it stops the listener.
pub struct FakeS3 {
    /// Shared with the accept thread and every connection thread.
    inner: Arc<Inner>,
    /// The accept thread, joined on drop.
    accept: Option<JoinHandle<()>>,
}

impl FakeS3 {
    /// Bind `127.0.0.1:0` and start accepting. Signatures are required until
    /// [`allow_anonymous`](Self::allow_anonymous) says otherwise.
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
        format!("http://{}", self.inner.address)
    }

    /// The bound port.
    pub fn port(&self) -> u16 {
        self.inner.address.port()
    }

    /// Skip every signature check (`true`) or require one (`false`).
    pub fn allow_anonymous(&self, allowed: bool) {
        self.inner.anonymous.store(allowed, Ordering::SeqCst);
    }

    /// Answer `403 InvalidAccessKeyId` to any other access key; `None`
    /// accepts every key.
    pub fn require_access_key(&self, access_key: Option<&str>) {
        self.inner.store().required_access_key = access_key.map(str::to_owned);
    }

    /// Pin the region of `bucket`: a credential scope naming another region
    /// is answered `400 AuthorizationHeaderMalformed` with
    /// `x-amz-bucket-region`. `None` unpins.
    pub fn set_bucket_region(&self, bucket: &str, region: Option<&str>) {
        let mut store = self.inner.store();
        match region {
            Some(region) => {
                store.regions.insert(bucket.to_owned(), region.to_owned());
            }
            None => {
                store.regions.remove(bucket);
            }
        }
    }

    /// Create `bucket`; existing is kept.
    pub fn create_bucket(&self, bucket: &str) {
        self.inner
            .store()
            .buckets
            .entry(bucket.to_owned())
            .or_default();
    }

    /// Seed one object, creating the bucket.
    pub fn put(&self, bucket: &str, key: &str, bytes: &[u8]) {
        self.inner
            .store()
            .buckets
            .entry(bucket.to_owned())
            .or_default()
            .insert(
                key.to_owned(),
                Object::new(bytes.to_vec(), None, Encryption::None),
            );
    }

    /// The bytes of one object.
    pub fn get(&self, bucket: &str, key: &str) -> Option<Vec<u8>> {
        self.inner
            .store()
            .buckets
            .get(bucket)?
            .get(key)
            .map(|object| object.bytes.clone())
    }

    /// How `key` was encrypted, as the store recorded it.
    ///
    /// `"none"`, `"AES256"`, `"aws:kms"` with the key it named, or
    /// `"customer"` with the MD5 the write presented - which is all a store
    /// keeps of a key it was handed.
    pub fn stored_encryption(&self, bucket: &str, key: &str) -> Option<String> {
        Some(
            self.inner
                .store()
                .buckets
                .get(bucket)?
                .get(key)?
                .encryption
                .described(),
        )
    }

    /// Every key of `bucket` in byte order; empty for a missing bucket.
    pub fn keys(&self, bucket: &str) -> Vec<String> {
        self.inner
            .store()
            .buckets
            .get(bucket)
            .map(|objects| objects.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// Every bucket in byte order.
    pub fn buckets(&self) -> Vec<String> {
        self.inner.store().buckets.keys().cloned().collect()
    }

    /// Answer the next `times` requests with `status` and error `code`
    /// before looking at them, e.g. `503 SlowDown` to exercise retries.
    pub fn fail_next(&self, status: u16, code: &str, times: usize) {
        self.inner.store().failure = (times > 0).then(|| Injected {
            status,
            code: code.to_owned(),
            remaining: times,
        });
    }

    /// Keep (`true`) or stop keeping (`false`) the request log; the count
    /// keeps going either way.
    pub fn set_recording(&self, recording: bool) {
        self.inner.recording.store(recording, Ordering::SeqCst);
    }

    /// The requests handled while recording, in arrival order.
    pub fn requests(&self) -> Vec<Recorded> {
        self.inner.log().clone()
    }

    /// Requests handled since the last [`clear_requests`](Self::clear_requests),
    /// recording or not.
    /// Connections accepted since the server started.
    ///
    /// A client that pools keeps this far below [`Self::request_count`]; one
    /// that reconnects per request drives the two together, which on a real
    /// store is a TLS handshake each time.
    pub fn connection_count(&self) -> usize {
        self.inner.connections.load(Ordering::Relaxed)
    }

    pub fn request_count(&self) -> usize {
        self.inner.count.load(Ordering::SeqCst)
    }

    /// Empty the log and reset the count.
    pub fn clear_requests(&self) {
        self.inner.log().clear();
        self.inner.count.store(0, Ordering::SeqCst);
    }

    /// Multipart uploads initiated and neither completed nor aborted.
    pub fn open_uploads(&self) -> usize {
        self.inner.store().uploads.len()
    }
}

impl Drop for FakeS3 {
    fn drop(&mut self) {
        self.inner.stopping.store(true, Ordering::SeqCst);
        // A connection is the only thing that returns from `accept`.
        drop(TcpStream::connect(self.inner.address));
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
    /// Skip every signature check.
    anonymous: AtomicBool,
    /// Append handled requests to `log`.
    recording: AtomicBool,
    /// Requests handled, recording or not.
    count: AtomicUsize,
    /// Connections accepted, so a test can see whether a client pools them.
    connections: AtomicUsize,
    /// Source of `x-amz-request-id` values.
    request_ids: AtomicUsize,
    /// Buckets, uploads, regions, and the knobs that shape answers.
    store: Mutex<Store>,
    /// The request log.
    log: Mutex<Vec<Recorded>>,
}

impl Inner {
    fn new(address: SocketAddr) -> Self {
        Self {
            address,
            stopping: AtomicBool::new(false),
            anonymous: AtomicBool::new(false),
            recording: AtomicBool::new(true),
            connections: AtomicUsize::new(0),
            count: AtomicUsize::new(0),
            request_ids: AtomicUsize::new(0),
            store: Mutex::new(Store::default()),
            log: Mutex::new(Vec::new()),
        }
    }

    /// The store, poison-tolerant: a panicking test must not hide the next
    /// test's failure behind a lock error.
    fn store(&self) -> MutexGuard<'_, Store> {
        self.store.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn log(&self) -> MutexGuard<'_, Vec<Recorded>> {
        self.log.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn next_request_id(&self) -> String {
        format!(
            "{:016X}",
            self.request_ids.fetch_add(1, Ordering::SeqCst) + 1
        )
    }

    /// Answer one parsed request and record it.
    fn handle(&self, request: &mut Request) -> Response {
        let body_len = request.body.len();
        let (bucket, key) = self.resolve(request);
        let mut response = self.answer(request, bucket.as_deref(), key.as_deref());
        let region = bucket
            .as_deref()
            .and_then(|bucket| self.store().regions.get(bucket).cloned());
        if let Some(region) = region {
            response
                .headers
                .push(("x-amz-bucket-region".to_owned(), region));
        }
        response.request_id = self.next_request_id();
        self.record(request, bucket, key, body_len, response.status);
        response
    }

    /// Injected failure, then signature checks, then the operation.
    fn answer(&self, request: &mut Request, bucket: Option<&str>, key: Option<&str>) -> Response {
        if let Some(failure) = self.injected_failure() {
            return failure;
        }
        if let Some(refusal) = self.refusal(request, bucket) {
            return refusal;
        }
        self.store().dispatch(request, bucket, key)
    }

    /// The bucket and key the request addresses: virtual-hosted when the
    /// `Host` header is `{bucket}.{endpoint host}`, path-style otherwise.
    fn resolve(&self, request: &Request) -> (Option<String>, Option<String>) {
        let path = percent_decode(&request.path);
        let path = path.strip_prefix('/').unwrap_or(&path);
        let host = request.header("host").unwrap_or("");
        let hosted = host
            .strip_suffix(&format!(".{}", self.address))
            .or_else(|| host.strip_suffix(&format!(".{}", self.address.ip())));
        let (bucket, key) = match hosted {
            Some(bucket) => (bucket, path),
            None => path.split_once('/').unwrap_or((path, "")),
        };
        (non_empty(bucket), non_empty(key))
    }

    fn injected_failure(&self) -> Option<Response> {
        let mut store = self.store();
        let failure = store.failure.as_mut()?;
        failure.remaining -= 1;
        let response = Response::error(failure.status, &failure.code, "Injected failure.", &[]);
        if failure.remaining == 0 {
            store.failure = None;
        }
        Some(response)
    }

    /// The `SigV4` header checks: shape of `Authorization`, presence of the
    /// date and payload-hash headers and of every signed header, the required
    /// access key, and the bucket's pinned region. Signatures are never
    /// verified - the server holds no secret. `Some` is the refusal.
    fn refusal(&self, request: &Request, bucket: Option<&str>) -> Option<Response> {
        if self.anonymous.load(Ordering::SeqCst) {
            return None;
        }
        let denied = || Some(Response::error(403, "AccessDenied", "Access Denied", &[]));
        let credential = request
            .header("authorization")
            .and_then(parse_authorization);
        let Some(credential) = credential else {
            return denied();
        };
        let present = |name: &str| request.header(name).is_some();
        if !present("x-amz-date") || !present("x-amz-content-sha256") {
            return denied();
        }
        let signed = |name: &str| credential.signed.iter().any(|header| header == name);
        if !signed("host") || !signed("x-amz-date") || !signed("x-amz-content-sha256") {
            return denied();
        }
        if !credential.signed.iter().all(|header| present(header)) {
            return denied();
        }
        let store = self.store();
        if store
            .required_access_key
            .as_deref()
            .is_some_and(|required| required != credential.access_key)
        {
            return Some(Response::error(
                403,
                "InvalidAccessKeyId",
                "The AWS Access Key Id you provided does not exist in our records.",
                &[],
            ));
        }
        let expected = store.regions.get(bucket?)?;
        if expected == &credential.region {
            return None;
        }
        let message = format!(
            "The authorization header is malformed; the region '{}' is wrong; expecting '{expected}'",
            credential.region
        );
        Some(
            Response::error(
                400,
                "AuthorizationHeaderMalformed",
                &message,
                &[("Region", expected)],
            )
            .with_header("x-amz-bucket-region", expected),
        )
    }

    fn record(
        &self,
        request: &Request,
        bucket: Option<String>,
        key: Option<String>,
        body_len: usize,
        status: u16,
    ) {
        self.count.fetch_add(1, Ordering::SeqCst);
        if !self.recording.load(Ordering::SeqCst) {
            return;
        }
        self.log().push(Recorded {
            method: request.method.clone(),
            bucket,
            key,
            path: request.path.clone(),
            query: request.query.clone(),
            headers: request.headers.clone(),
            body_len,
            status,
        });
    }
}

/// The parsed parts of one `Authorization: AWS4-HMAC-SHA256 ...` header.
struct Credential {
    /// The access key id opening the credential scope.
    access_key: String,
    /// The region named in the credential scope.
    region: String,
    /// `SignedHeaders`, lowercase.
    signed: Vec<String>,
}

/// `AWS4-HMAC-SHA256 Credential=<ak>/<date>/<region>/s3/aws4_request,
/// SignedHeaders=<h;h>, Signature=<64 hex>`; anything else is `None`.
fn parse_authorization(value: &str) -> Option<Credential> {
    let rest = value.strip_prefix("AWS4-HMAC-SHA256 ")?;
    let (mut credential, mut signed, mut signature) = (None, None, None);
    for part in rest.split(',') {
        let (name, value) = part.trim().split_once('=')?;
        match name {
            "Credential" => credential = Some(value),
            "SignedHeaders" => signed = Some(value),
            "Signature" => signature = Some(value),
            _ => return None,
        }
    }
    let scope: Vec<&str> = credential?.split('/').collect();
    let [access_key, _date, region, "s3", "aws4_request"] = scope[..] else {
        return None;
    };
    let signature = signature?;
    if signature.len() != 64 || !signature.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    Some(Credential {
        access_key: access_key.to_owned(),
        region: region.to_owned(),
        signed: signed?
            .split(';')
            .map(|header| header.trim().to_ascii_lowercase())
            .collect(),
    })
}

/// Everything the handled requests read and write.
#[derive(Default)]
struct Store {
    /// Bucket name to its objects, both in byte order.
    buckets: BTreeMap<String, BTreeMap<String, Object>>,
    /// Multipart uploads in flight, by upload id.
    uploads: HashMap<String, Upload>,
    /// Pinned bucket regions.
    regions: HashMap<String, String>,
    /// The only access key accepted, when set.
    required_access_key: Option<String>,
    /// The failure injected by `fail_next`, while requests remain.
    failure: Option<Injected>,
    /// Upload ids handed out so far.
    next_upload: usize,
}

/// One stored object.
struct Object {
    /// The whole value.
    bytes: Vec<u8>,
    /// Quoted, as it goes on the wire.
    etag: String,
    /// `Content-Type` as stored; `None` answers the default.
    content_type: Option<String>,
    /// How the object was encrypted, as the store remembers it.
    encryption: Encryption,
}

impl Object {
    fn new(bytes: Vec<u8>, content_type: Option<String>, encryption: Encryption) -> Self {
        let etag = etag_of(&bytes);
        Self {
            bytes,
            etag,
            content_type,
            encryption,
        }
    }
}

/// What a request said about encrypting an object, as the store keeps it.
///
/// The store keeps everything about its own keys and nothing about a
/// caller's, which is exactly why `SSE-C` has to be presented again on a read.
#[derive(Clone, Default, PartialEq, Eq)]
enum Encryption {
    /// Nothing was said, so nothing is remembered.
    #[default]
    None,
    /// `AES256`, keys the store manages.
    Managed,
    /// `aws:kms` or `aws:kms:dsse`, with whatever the write named.
    Kms {
        algorithm: String,
        key_id: Option<String>,
        context: Option<String>,
        bucket_key: Option<String>,
    },
    /// A key the caller holds: only its stated MD5 is kept.
    Customer { md5: String },
}

impl Encryption {
    /// Read what a write says about encryption, or refuse an unusable one.
    ///
    /// A customer key is checked the way S3 checks it: the algorithm has to be
    /// `AES256`, the key has to be base64 of 32 bytes, and its MD5 has to come
    /// with it.
    fn of(request: &Request) -> std::result::Result<Self, Box<Response>> {
        if let Some(algorithm) = request.header("x-amz-server-side-encryption-customer-algorithm") {
            if algorithm != "AES256" {
                return Err(Box::new(invalid_encryption(
                    "Requests specifying Server Side Encryption with Customer \
                     provided keys must provide a valid encryption algorithm.",
                )));
            }
            let key = request
                .header("x-amz-server-side-encryption-customer-key")
                .and_then(|key| {
                    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, key).ok()
                })
                .filter(|key| key.len() == 32);
            if key.is_none() {
                return Err(Box::new(invalid_encryption(
                    "Requests specifying Server Side Encryption with Customer \
                     provided keys must provide an appropriate secret key.",
                )));
            }
            let Some(md5) = request.header("x-amz-server-side-encryption-customer-key-md5") else {
                return Err(Box::new(invalid_encryption(
                    "Requests specifying Server Side Encryption with Customer \
                     provided keys must provide the client calculated MD5 of \
                     the secret key.",
                )));
            };
            return Ok(Self::Customer {
                md5: md5.to_owned(),
            });
        }
        match request.header("x-amz-server-side-encryption") {
            None => Ok(Self::None),
            Some("AES256") => Ok(Self::Managed),
            Some(algorithm @ ("aws:kms" | "aws:kms:dsse")) => Ok(Self::Kms {
                algorithm: algorithm.to_owned(),
                key_id: request
                    .header("x-amz-server-side-encryption-aws-kms-key-id")
                    .map(str::to_owned),
                context: request
                    .header("x-amz-server-side-encryption-context")
                    .map(str::to_owned),
                bucket_key: request
                    .header("x-amz-server-side-encryption-bucket-key-enabled")
                    .map(str::to_owned),
            }),
            Some(_) => Err(Box::new(invalid_encryption(
                "The encryption method specified is not supported",
            ))),
        }
    }

    /// Refuse a read that does not present the key the object needs.
    ///
    /// S3 refuses both ways round: a customer-encrypted object read without
    /// the key, and any other object read with one.
    fn admits(&self, request: &Request) -> Option<Response> {
        let presented = request.header("x-amz-server-side-encryption-customer-key-md5");
        match (self, presented) {
            (Self::Customer { md5 }, Some(given)) if md5 == given => None,
            (Self::Customer { .. }, _) => Some(invalid_encryption(
                "The object was stored using a form of Server Side Encryption. \
                 The correct parameters must be provided to retrieve the object.",
            )),
            (_, Some(_)) => Some(invalid_encryption(
                "The object was not stored using a form of Server Side \
                 Encryption with customer provided keys.",
            )),
            (_, None) => None,
        }
    }

    /// What an answer says about how the object is encrypted.
    fn headers(&self) -> Vec<(&'static str, String)> {
        match self {
            Self::None => Vec::new(),
            Self::Managed => vec![("x-amz-server-side-encryption", "AES256".to_owned())],
            Self::Kms {
                algorithm,
                key_id,
                context,
                bucket_key,
            } => {
                let mut headers = vec![("x-amz-server-side-encryption", algorithm.clone())];
                if let Some(key_id) = key_id {
                    headers.push((
                        "x-amz-server-side-encryption-aws-kms-key-id",
                        key_id.clone(),
                    ));
                }
                if let Some(context) = context {
                    headers.push(("x-amz-server-side-encryption-context", context.clone()));
                }
                if let Some(enabled) = bucket_key {
                    headers.push((
                        "x-amz-server-side-encryption-bucket-key-enabled",
                        enabled.clone(),
                    ));
                }
                headers
            }
            Self::Customer { md5 } => vec![
                (
                    "x-amz-server-side-encryption-customer-algorithm",
                    "AES256".to_owned(),
                ),
                ("x-amz-server-side-encryption-customer-key-MD5", md5.clone()),
            ],
        }
    }

    /// How a test names what was recorded.
    fn described(&self) -> String {
        match self {
            Self::None => "none".to_owned(),
            Self::Managed => "AES256".to_owned(),
            Self::Kms {
                algorithm, key_id, ..
            } => match key_id {
                Some(key_id) => format!("{algorithm} {key_id}"),
                None => algorithm.clone(),
            },
            Self::Customer { md5 } => format!("customer {md5}"),
        }
    }
}

/// The refusal S3 answers a malformed or mismatched encryption request with.
fn invalid_encryption(message: &str) -> Response {
    Response::error(400, "InvalidArgument", message, &[])
}

/// One multipart upload in flight.
struct Upload {
    /// The bucket the object lands in.
    bucket: String,
    /// The key the object lands at.
    key: String,
    /// `Content-Type` given at initiation, applied to the assembled object.
    content_type: Option<String>,
    /// How the initiation said the assembled object is to be encrypted.
    encryption: Encryption,
    /// Part number to its bytes and quoted `ETag`.
    parts: BTreeMap<u32, (Vec<u8>, String)>,
}

/// The error `fail_next` answers with, and how many more times.
struct Injected {
    /// HTTP status of the answer.
    status: u16,
    /// S3 error code inside it.
    code: String,
    /// Requests still to fail.
    remaining: usize,
}

impl Store {
    /// Route one authorized request to its operation.
    fn dispatch(
        &mut self,
        request: &mut Request,
        bucket: Option<&str>,
        key: Option<&str>,
    ) -> Response {
        let Some(bucket) = bucket else {
            return invalid_request();
        };
        match (request.method.as_str(), key) {
            ("PUT", None) => self.create_bucket(bucket),
            ("HEAD", None) => self.head_bucket(bucket),
            ("DELETE", None) => self.delete_bucket(bucket),
            ("GET", None) if request.query("list-type") == Some("2") => self.list(bucket, request),
            ("POST", None) if request.has_query("delete") => self.delete_objects(bucket, request),
            ("PUT", Some(key)) if request.has_query("uploadId") => {
                self.upload_part(bucket, key, request)
            }
            ("PUT", Some(key)) => self.put_object(bucket, key, request),
            ("POST", Some(key)) if request.has_query("uploads") => {
                self.initiate_upload(bucket, key, request)
            }
            ("POST", Some(key)) if request.has_query("uploadId") => {
                self.complete_upload(bucket, key, request)
            }
            ("DELETE", Some(key)) if request.has_query("uploadId") => {
                self.abort_upload(bucket, key, request)
            }
            ("DELETE", Some(key)) => self.delete_object(bucket, key),
            ("GET" | "HEAD", Some(key)) => self.get_object(bucket, key, request),
            _ => invalid_request(),
        }
    }

    fn create_bucket(&mut self, bucket: &str) -> Response {
        if self.buckets.contains_key(bucket) {
            return Response::error(
                409,
                "BucketAlreadyOwnedByYou",
                "Your previous request to create the named bucket succeeded and you already own it.",
                &[],
            );
        }
        self.buckets.insert(bucket.to_owned(), BTreeMap::new());
        Response::new(200)
    }

    fn head_bucket(&self, bucket: &str) -> Response {
        if self.buckets.contains_key(bucket) {
            Response::new(200)
        } else {
            no_such_bucket()
        }
    }

    fn delete_bucket(&mut self, bucket: &str) -> Response {
        match self.buckets.get(bucket) {
            None => no_such_bucket(),
            Some(objects) if !objects.is_empty() => Response::error(
                409,
                "BucketNotEmpty",
                "The bucket you tried to delete is not empty",
                &[],
            ),
            Some(_) => {
                self.buckets.remove(bucket);
                Response::new(204)
            }
        }
    }

    fn list(&self, bucket: &str, request: &Request) -> Response {
        let Some(objects) = self.buckets.get(bucket) else {
            return no_such_bucket();
        };
        let query = match ListQuery::from_request(request) {
            Ok(query) => query,
            Err(message) => return Response::error(400, "InvalidArgument", &message, &[]),
        };
        let page = scan(objects, &query);
        Response::xml(200, &render_listing(bucket, &query, &page))
    }

    /// `If-None-Match: *` refuses to overwrite; `If-Match` refuses a
    /// different or missing object. Both answer 412.
    fn put_object(&mut self, bucket: &str, key: &str, request: &mut Request) -> Response {
        let Some(objects) = self.buckets.get_mut(bucket) else {
            return no_such_bucket();
        };
        let current = objects.get(key);
        if request.header("if-none-match") == Some("*") && current.is_some() {
            return precondition_failed();
        }
        if let Some(expected) = request.header("if-match") {
            let matches = current.is_some_and(|object| {
                expected == "*" || trim_quotes(expected) == trim_quotes(&object.etag)
            });
            if !matches {
                return precondition_failed();
            }
        }
        let encryption = match Encryption::of(request) {
            Ok(encryption) => encryption,
            Err(refusal) => return *refusal,
        };
        let content_type = request.header("content-type").map(str::to_owned);
        let object = Object::new(std::mem::take(&mut request.body), content_type, encryption);
        let etag = object.etag.clone();
        let answered = object.encryption.headers();
        objects.insert(key.to_owned(), object);
        let mut response = Response::new(200).with_header("ETag", &etag);
        for (name, value) in answered {
            response = response.with_header(name, &value);
        }
        response
    }

    fn upload_part(&mut self, bucket: &str, key: &str, request: &mut Request) -> Response {
        let number = request
            .query("partNumber")
            .and_then(|text| text.parse::<u32>().ok())
            .filter(|number| (1..=10_000).contains(number));
        let Some(number) = number else {
            return Response::error(
                400,
                "InvalidArgument",
                "Part number must be an integer between 1 and 10000, inclusive",
                &[],
            );
        };
        let id = request.query("uploadId").unwrap_or("");
        let upload = self
            .uploads
            .get_mut(id)
            .filter(|upload| upload.bucket == bucket && upload.key == key);
        let Some(upload) = upload else {
            return no_such_upload();
        };
        if let Some(refusal) = upload.encryption.admits(request) {
            return refusal;
        }
        let bytes = std::mem::take(&mut request.body);
        let etag = etag_of(&bytes);
        upload.parts.insert(number, (bytes, etag.clone()));
        Response::new(200).with_header("ETag", &etag)
    }

    fn initiate_upload(&mut self, bucket: &str, key: &str, request: &Request) -> Response {
        if !self.buckets.contains_key(bucket) {
            return no_such_bucket();
        }
        let encryption = match Encryption::of(request) {
            Ok(encryption) => encryption,
            Err(refusal) => return *refusal,
        };
        self.next_upload += 1;
        let id = format!("upload-{}", self.next_upload);
        self.uploads.insert(
            id.clone(),
            Upload {
                bucket: bucket.to_owned(),
                key: key.to_owned(),
                content_type: request.header("content-type").map(str::to_owned),
                encryption,
                parts: BTreeMap::new(),
            },
        );
        let mut xml = document("InitiateMultipartUploadResult");
        element(&mut xml, "Bucket", bucket);
        element(&mut xml, "Key", key);
        element(&mut xml, "UploadId", &id);
        xml.push_str("</InitiateMultipartUploadResult>");
        Response::xml(200, &xml)
    }

    /// Assemble the listed parts in part-number order. An unknown part or a
    /// wrong `ETag` is `400 InvalidPart` and leaves the upload open.
    fn complete_upload(&mut self, bucket: &str, key: &str, request: &Request) -> Response {
        let id = request.query("uploadId").unwrap_or("");
        let upload = self
            .uploads
            .get(id)
            .filter(|upload| upload.bucket == bucket && upload.key == key);
        let Some(upload) = upload else {
            return no_such_upload();
        };
        let mut listed = parse_complete_parts(&request.body).unwrap_or_default();
        if listed.is_empty() {
            return malformed_xml();
        }
        listed.sort_unstable_by_key(|(number, _)| *number);
        let mut bytes = Vec::new();
        for (number, etag) in &listed {
            match upload.parts.get(number) {
                Some((part, stored)) if trim_quotes(stored) == trim_quotes(etag) => {
                    bytes.extend_from_slice(part);
                }
                _ => {
                    return Response::error(
                        400,
                        "InvalidPart",
                        "One or more of the specified parts could not be found. The part may not have been uploaded, or the specified entity tag may not match the part's entity tag.",
                        &[],
                    );
                }
            }
        }
        let Some(objects) = self.buckets.get_mut(bucket) else {
            return no_such_bucket();
        };
        let Some(upload) = self.uploads.remove(id) else {
            return no_such_upload();
        };
        let mut object = Object::new(bytes, upload.content_type, upload.encryption);
        object.etag = format!("\"{}-{}\"", trim_quotes(&object.etag), listed.len());
        let etag = object.etag.clone();
        objects.insert(key.to_owned(), object);
        let host = request.header("host").unwrap_or("");
        let mut xml = document("CompleteMultipartUploadResult");
        element(
            &mut xml,
            "Location",
            &format!("http://{host}/{bucket}/{key}"),
        );
        element(&mut xml, "Bucket", bucket);
        element(&mut xml, "Key", key);
        element(&mut xml, "ETag", &etag);
        xml.push_str("</CompleteMultipartUploadResult>");
        Response::xml(200, &xml)
    }

    fn abort_upload(&mut self, bucket: &str, key: &str, request: &Request) -> Response {
        let id = request.query("uploadId").unwrap_or("");
        let known = self
            .uploads
            .get(id)
            .is_some_and(|upload| upload.bucket == bucket && upload.key == key);
        if !known {
            return no_such_upload();
        }
        self.uploads.remove(id);
        Response::new(204)
    }

    /// Whole object, or the single byte range of `Range` as 206; a range
    /// starting at or past the end (any range on an empty object) is 416.
    /// An unparsable `Range` is ignored, as RFC 7233 allows.
    fn get_object(&self, bucket: &str, key: &str, request: &Request) -> Response {
        let Some(objects) = self.buckets.get(bucket) else {
            return no_such_bucket();
        };
        let Some(object) = objects.get(key) else {
            return Response::error(
                404,
                "NoSuchKey",
                "The specified key does not exist.",
                &[("Key", key)],
            );
        };
        if let Some(refusal) = object.encryption.admits(request) {
            return refusal;
        }
        let size = object.bytes.len();
        let range = request.header("range").and_then(ByteRange::parse);
        let Some(range) = range else {
            return object_response(200, object).with_body(object.bytes.clone());
        };
        let Some((start, end)) = range.resolve(size) else {
            let requested = request.header("range").unwrap_or("");
            return Response::error(
                416,
                "InvalidRange",
                "The requested range is not satisfiable",
                &[
                    ("RangeRequested", requested),
                    ("ActualObjectSize", &size.to_string()),
                ],
            )
            .with_header("Content-Range", &format!("bytes */{size}"));
        };
        object_response(206, object)
            .with_header("Content-Range", &format!("bytes {start}-{end}/{size}"))
            .with_body(object.bytes[start..=end].to_vec())
    }

    fn delete_object(&mut self, bucket: &str, key: &str) -> Response {
        let Some(objects) = self.buckets.get_mut(bucket) else {
            return no_such_bucket();
        };
        objects.remove(key);
        Response::new(204)
    }

    /// `<Delete>` with up to 1000 `<Object><Key>`; `<Quiet>true</Quiet>`
    /// drops the per-key `<Deleted>` entries from the answer.
    fn delete_objects(&mut self, bucket: &str, request: &Request) -> Response {
        let Some(objects) = self.buckets.get_mut(bucket) else {
            return no_such_bucket();
        };
        let Some((keys, quiet)) = parse_delete(&request.body) else {
            return malformed_xml();
        };
        if keys.len() > MAX_DELETE_KEYS {
            return malformed_xml();
        }
        let mut xml = document("DeleteResult");
        for key in keys {
            objects.remove(&key);
            if !quiet {
                xml.push_str("<Deleted>");
                element(&mut xml, "Key", &key);
                xml.push_str("</Deleted>");
            }
        }
        xml.push_str("</DeleteResult>");
        Response::xml(200, &xml)
    }
}

/// The headers every successful object answer carries.
fn object_response(status: u16, object: &Object) -> Response {
    let mut response = Response::new(status)
        .with_header("ETag", &object.etag)
        .with_header("Last-Modified", LAST_MODIFIED_HTTP)
        .with_header(
            "Content-Type",
            object
                .content_type
                .as_deref()
                .unwrap_or(DEFAULT_CONTENT_TYPE),
        )
        .with_header("Accept-Ranges", "bytes");
    for (name, value) in object.encryption.headers() {
        response = response.with_header(name, &value);
    }
    response
}

/// One `Range: bytes=` spec: `a-b`, `a-`, or `-n`.
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
            if text.is_empty() {
                Some(None)
            } else {
                text.parse().ok().map(Some)
            }
        };
        let (start, end) = (number(start)?, number(end)?);
        match (start, end) {
            (None, None) => None,
            (Some(start), Some(end)) if end < start => None,
            _ => Some(Self { start, end }),
        }
    }

    /// The inclusive byte bounds inside an object of `size` bytes, or `None`
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

/// The knobs of one `ListObjectsV2` request.
struct ListQuery<'a> {
    /// Keys must start with it; empty lists the bucket.
    prefix: &'a str,
    /// Only when given and non-empty.
    delimiter: Option<&'a str>,
    /// `start-after` as sent, echoed back.
    start_after: Option<&'a str>,
    /// The continuation token as sent, echoed back.
    token: Option<&'a str>,
    /// Where the scan starts: the token when given, else `start-after`. A
    /// token is the last key or prefix the previous page emitted, so both
    /// position the scan the same way.
    after: Option<&'a str>,
    /// Most keys and prefixes on the page.
    max_keys: usize,
    /// Percent-encode names in the answer (`encoding-type=url`).
    encode: bool,
}

impl<'a> ListQuery<'a> {
    /// `Err` carries the `InvalidArgument` message.
    fn from_request(request: &'a Request) -> Result<Self, String> {
        let max_keys = match request.query("max-keys") {
            None => DEFAULT_MAX_KEYS,
            Some(text) => text
                .parse()
                .map_err(|_| "max-keys must be a non-negative integer".to_owned())?,
        };
        let encode = match request.query("encoding-type") {
            None => false,
            Some("url") => true,
            Some(_) => return Err("Invalid Encoding Method specified in Request".to_owned()),
        };
        let token = request.query("continuation-token");
        let start_after = request.query("start-after");
        Ok(Self {
            prefix: request.query("prefix").unwrap_or(""),
            delimiter: request
                .query("delimiter")
                .filter(|delimiter| !delimiter.is_empty()),
            start_after,
            token,
            after: token.or(start_after),
            max_keys,
            encode,
        })
    }
}

/// One listing page: keys and rolled-up prefixes in scan order, plus the
/// last one emitted when the page was cut short - the next page's token.
struct Page<'a> {
    /// Keys emitted, with their objects.
    contents: Vec<(&'a str, &'a Object)>,
    /// Rolled-up prefixes emitted.
    prefixes: Vec<String>,
    /// The key or prefix emitted last; `None` when this page ended the scan.
    next: Option<String>,
}

/// Walk the keys under `prefix` in byte order, rolling up those with the
/// delimiter after the prefix; keys and prefixes count alike against
/// `max-keys`, each prefix once. Only keys and prefixes greater than the
/// start position are emitted - AWS documents the prefix half of that rule,
/// and it is what stops a page from re-emitting the prefix it ended on.
fn scan<'a>(objects: &'a BTreeMap<String, Object>, query: &ListQuery<'_>) -> Page<'a> {
    let mut page = Page {
        contents: Vec::new(),
        prefixes: Vec::new(),
        next: None,
    };
    // S3 answers `max-keys=0` with an empty page that is not truncated.
    if query.max_keys == 0 {
        return page;
    }
    let mut last = None;
    let from = (Bound::Included(query.prefix), Bound::Unbounded);
    for (key, object) in objects.range::<str, _>(from) {
        if !key.starts_with(query.prefix) {
            break;
        }
        let before_start = |name: &str| query.after.is_some_and(|after| name <= after);
        if before_start(key) {
            continue;
        }
        let rest = &key[query.prefix.len()..];
        let group = query.delimiter.and_then(|delimiter| {
            rest.find(delimiter)
                .map(|at| key[..query.prefix.len() + at + delimiter.len()].to_owned())
        });
        // Byte order keeps a prefix's keys contiguous, so only the prefix
        // emitted last can repeat.
        if group.as_deref().is_some_and(|group| {
            before_start(group) || page.prefixes.last().is_some_and(|last| last == group)
        }) {
            continue;
        }
        if page.contents.len() + page.prefixes.len() == query.max_keys {
            page.next = last;
            return page;
        }
        if let Some(group) = group {
            last = Some(group.clone());
            page.prefixes.push(group);
        } else {
            last = Some(key.clone());
            page.contents.push((key, object));
        }
    }
    page
}

/// `<ListBucketResult>`: `Contents` first, then `CommonPrefixes`. With
/// `encoding-type=url` keys, prefixes and `StartAfter` keep `/` as AWS does
/// while `Delimiter` is fully encoded.
fn render_listing(bucket: &str, query: &ListQuery<'_>, page: &Page<'_>) -> String {
    let name = |text: &str, keep_slash: bool| {
        if query.encode {
            percent_encode(text, keep_slash)
        } else {
            text.to_owned()
        }
    };
    let mut xml = document("ListBucketResult");
    element(&mut xml, "Name", bucket);
    element(&mut xml, "Prefix", &name(query.prefix, true));
    if let Some(after) = query.start_after {
        element(&mut xml, "StartAfter", &name(after, true));
    }
    if let Some(token) = query.token {
        element(&mut xml, "ContinuationToken", token);
    }
    if let Some(next) = &page.next {
        element(&mut xml, "NextContinuationToken", next);
    }
    let count = page.contents.len() + page.prefixes.len();
    element(&mut xml, "KeyCount", &count.to_string());
    element(&mut xml, "MaxKeys", &query.max_keys.to_string());
    if let Some(delimiter) = query.delimiter {
        element(&mut xml, "Delimiter", &name(delimiter, false));
    }
    if query.encode {
        element(&mut xml, "EncodingType", "url");
    }
    let truncated = if page.next.is_some() { "true" } else { "false" };
    element(&mut xml, "IsTruncated", truncated);
    for (key, object) in &page.contents {
        xml.push_str("<Contents>");
        element(&mut xml, "Key", &name(key, true));
        element(&mut xml, "LastModified", LAST_MODIFIED_ISO);
        element(&mut xml, "ETag", &object.etag);
        element(&mut xml, "Size", &object.bytes.len().to_string());
        element(&mut xml, "StorageClass", "STANDARD");
        xml.push_str("</Contents>");
    }
    for prefix in &page.prefixes {
        xml.push_str("<CommonPrefixes>");
        element(&mut xml, "Prefix", &name(prefix, true));
        xml.push_str("</CommonPrefixes>");
    }
    xml.push_str("</ListBucketResult>");
    xml
}

/// The keys of a `<Delete>` body and its `<Quiet>` flag; `None` when the
/// body is not one.
fn parse_delete(body: &[u8]) -> Option<(Vec<String>, bool)> {
    let xml = std::str::from_utf8(body).ok()?;
    let delete = elements(xml, "Delete").into_iter().next()?;
    let quiet = elements(delete, "Quiet")
        .first()
        .is_some_and(|quiet| quiet.trim() == "true");
    let keys = elements(delete, "Object")
        .into_iter()
        .map(|object| {
            elements(object, "Key")
                .first()
                .map(|key| unescape_text(key))
        })
        .collect::<Option<Vec<_>>>()?;
    Some((keys, quiet))
}

/// The `(PartNumber, ETag)` pairs of a `<CompleteMultipartUpload>` body.
fn parse_complete_parts(body: &[u8]) -> Option<Vec<(u32, String)>> {
    let xml = std::str::from_utf8(body).ok()?;
    let root = elements(xml, "CompleteMultipartUpload")
        .into_iter()
        .next()?;
    elements(root, "Part")
        .into_iter()
        .map(|part| {
            let number = elements(part, "PartNumber").first()?.trim().parse().ok()?;
            let etag = unescape_text(elements(part, "ETag").first()?.trim());
            Some((number, etag))
        })
        .collect()
}

/// The inner text of every `<name>...</name>` element in document order,
/// scanning rather than parsing: the request bodies this server reads escape
/// every `<` in text, so tag search is exact.
fn elements<'a>(xml: &'a str, name: &str) -> Vec<&'a str> {
    let open = format!("<{name}");
    let close = format!("</{name}>");
    let mut found = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find(&open) {
        let after = &rest[start + open.len()..];
        let Some(end_of_tag) = after.find('>') else {
            break;
        };
        // `<Key` also opens `<KeyCount>`: the name must end at the `>` or at
        // an attribute.
        let attributes = &after[..end_of_tag];
        if !attributes.is_empty() && !attributes.starts_with(char::is_whitespace) {
            rest = after;
            continue;
        }
        let inner = &after[end_of_tag + 1..];
        let Some(end) = inner.find(&close) else {
            break;
        };
        found.push(&inner[..end]);
        rest = &inner[end + close.len()..];
    }
    found
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
    /// The whole body, taken by the operation that stores it.
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

    fn has_query(&self, name: &str) -> bool {
        self.query(name).is_some()
    }
}

/// Why a connection stops being read.
enum Refusal {
    /// The peer went away, or the request was cut short.
    Closed,
    /// The bytes were not an HTTP request.
    Malformed,
}

/// Serve one connection until it closes or a request is malformed.
fn serve(inner: &Inner, stream: TcpStream) {
    let mut reader = BufReader::new(stream);
    loop {
        let mut request = match read_request(&mut reader) {
            Ok(request) => request,
            Err(Refusal::Closed) => return,
            Err(Refusal::Malformed) => {
                let mut response = Response::error(
                    400,
                    "InvalidRequest",
                    "The HTTP request could not be parsed.",
                    &[],
                );
                response.request_id = inner.next_request_id();
                let _ = write_response(reader.get_mut(), "GET", &response, true);
                return;
            }
        };
        let response = inner.handle(&mut request);
        let written = write_response(
            reader.get_mut(),
            &request.method,
            &response,
            !request.keep_alive,
        );
        if written.is_err() || !request.keep_alive {
            return;
        }
    }
}

/// One request line, its headers, and its body; `Expect: 100-continue` is
/// acknowledged before the body is read.
fn read_request(reader: &mut BufReader<TcpStream>) -> Result<Request, Refusal> {
    let mut line = String::new();
    // RFC 7230 section 3.5: empty lines before the request line are ignored.
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
    body: Body,
    /// Set once per handled request; the header and the error document
    /// carry it.
    request_id: String,
}

/// What an answer carries: bytes, or an error document rendered at write
/// time with the request id.
enum Body {
    /// Bytes sent as they are.
    Bytes(Vec<u8>),
    /// An `<Error>` document.
    Error {
        /// S3 error code.
        code: String,
        /// Human-readable text.
        message: String,
        /// Extra elements after `<Message>`, e.g. `<Key>` or `<Region>`.
        extra: Vec<(String, String)>,
    },
}

impl Response {
    fn new(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: Body::Bytes(Vec::new()),
            request_id: String::new(),
        }
    }

    fn xml(status: u16, document: &str) -> Self {
        Self::new(status)
            .with_header("Content-Type", "application/xml")
            .with_body(document.as_bytes().to_vec())
    }

    fn error(status: u16, code: &str, message: &str, extra: &[(&str, &str)]) -> Self {
        let mut response = Self::new(status);
        response.body = Body::Error {
            code: code.to_owned(),
            message: message.to_owned(),
            extra: extra
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
        };
        response
    }

    fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    fn with_body(mut self, bytes: Vec<u8>) -> Self {
        self.body = Body::Bytes(bytes);
        self
    }
}

/// Status line, headers, `x-amz-request-id`, `Content-Length`, and the body
/// unless the request was a `HEAD`. A `HEAD` refusal reports length 0, as
/// AWS sends no document with it.
fn write_response(
    stream: &mut TcpStream,
    method: &str,
    response: &Response,
    close: bool,
) -> io::Result<()> {
    let head = method == "HEAD";
    let rendered;
    let body: &[u8] = match &response.body {
        Body::Bytes(bytes) => bytes,
        Body::Error { .. } if head => &[],
        Body::Error {
            code,
            message,
            extra,
        } => {
            rendered = render_error(code, message, extra, &response.request_id);
            rendered.as_bytes()
        }
    };
    let mut out = format!(
        "HTTP/1.1 {} {}\r\n",
        response.status,
        reason(response.status)
    );
    for (name, value) in &response.headers {
        let _ = write!(out, "{name}: {value}\r\n");
    }
    if !head && matches!(response.body, Body::Error { .. }) {
        out.push_str("Content-Type: application/xml\r\n");
    }
    let _ = write!(
        out,
        "x-amz-request-id: {}\r\nContent-Length: {}\r\n",
        response.request_id,
        body.len()
    );
    if close {
        out.push_str("Connection: close\r\n");
    }
    out.push_str("\r\n");
    let mut out = out.into_bytes();
    if !head {
        out.extend_from_slice(body);
    }
    stream.write_all(&out)?;
    stream.flush()
}

fn reason(status: u16) -> &'static str {
    match status {
        100 => "Continue",
        200 => "OK",
        204 => "No Content",
        206 => "Partial Content",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        412 => "Precondition Failed",
        416 => "Range Not Satisfiable",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Unknown",
    }
}

/// `<Error>` as AWS shapes it: no namespace, `Code`, `Message`, the
/// operation's extra elements, `RequestId`, `HostId`.
fn render_error(code: &str, message: &str, extra: &[(String, String)], request_id: &str) -> String {
    let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Error>");
    element(&mut xml, "Code", code);
    element(&mut xml, "Message", message);
    for (name, value) in extra {
        element(&mut xml, name, value);
    }
    element(&mut xml, "RequestId", request_id);
    element(&mut xml, "HostId", "fake-s3");
    xml.push_str("</Error>");
    xml
}

fn invalid_request() -> Response {
    Response::error(
        400,
        "InvalidRequest",
        "The server does not implement this request.",
        &[],
    )
}

fn no_such_bucket() -> Response {
    Response::error(
        404,
        "NoSuchBucket",
        "The specified bucket does not exist.",
        &[],
    )
}

fn no_such_upload() -> Response {
    Response::error(
        404,
        "NoSuchUpload",
        "The specified upload does not exist. The upload ID may be invalid, or the upload may have been aborted or completed.",
        &[],
    )
}

fn precondition_failed() -> Response {
    Response::error(
        412,
        "PreconditionFailed",
        "At least one of the pre-conditions you specified did not hold",
        &[],
    )
}

fn malformed_xml() -> Response {
    Response::error(
        400,
        "MalformedXML",
        "The XML you provided was not well-formed or did not validate against our published schema",
        &[],
    )
}

/// The prologue and the open root tag of one result document.
fn document(root: &str) -> String {
    format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<{root} xmlns=\"{XMLNS}\">")
}

/// Append `<name>text</name>` with `text` escaped.
fn element(xml: &mut String, name: &str, text: &str) {
    let _ = write!(xml, "<{name}>{}</{name}>", escape_text(text));
}

/// Escape `& < > " '` for element text; AWS escapes the quotes of an `ETag`
/// this way.
fn escape_text(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            other => escaped.push(other),
        }
    }
    escaped
}

/// Resolve the five named entities and numeric references; an unknown
/// reference is kept verbatim.
fn unescape_text(text: &str) -> String {
    let mut unescaped = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        unescaped.push_str(&rest[..at]);
        rest = &rest[at..];
        let Some(end) = rest.find(';') else {
            break;
        };
        let entity = &rest[1..end];
        let resolved = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => entity
                .strip_prefix('#')
                .and_then(|number| match number.strip_prefix('x') {
                    Some(hex) => u32::from_str_radix(hex, 16).ok(),
                    None => number.parse().ok(),
                })
                .and_then(char::from_u32),
        };
        match resolved {
            Some(character) => unescaped.push(character),
            None => unescaped.push_str(&rest[..=end]),
        }
        rest = &rest[end + 1..];
    }
    unescaped.push_str(rest);
    unescaped
}

/// Percent-encode with the unreserved set (`A-Z a-z 0-9 - _ . ~`), keeping
/// `/` when asked.
fn percent_encode(text: &str, keep_slash: bool) -> String {
    let mut encoded = String::with_capacity(text.len());
    for byte in text.bytes() {
        let kept = byte.is_ascii_alphanumeric()
            || matches!(byte, b'-' | b'_' | b'.' | b'~')
            || (keep_slash && byte == b'/');
        if kept {
            encoded.push(byte as char);
        } else {
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
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

/// The quoted `ETag` of `bytes`.
fn etag_of(bytes: &[u8]) -> String {
    format!("\"{:016x}\"", fnv1a(bytes))
}

fn trim_quotes(etag: &str) -> &str {
    etag.trim().trim_matches('"')
}

fn non_empty(text: &str) -> Option<String> {
    (!text.is_empty()).then(|| text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One parsed answer.
    struct Reply {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    }

    impl Reply {
        fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .iter()
                .find(|(header, _)| header == name)
                .map(|(_, value)| value.as_str())
        }

        fn text(&self) -> &str {
            std::str::from_utf8(&self.body).unwrap()
        }
    }

    fn anonymous() -> FakeS3 {
        let server = FakeS3::start();
        server.allow_anonymous(true);
        server
    }

    fn connect(server: &FakeS3) -> BufReader<TcpStream> {
        BufReader::new(TcpStream::connect(("127.0.0.1", server.port())).unwrap())
    }

    /// A raw request; `Host` and `Content-Length` are added unless given.
    fn raw(method: &str, target: &str, headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
        let has = |name: &str| {
            headers
                .iter()
                .any(|(header, _)| header.eq_ignore_ascii_case(name))
        };
        let mut text = format!("{method} {target} HTTP/1.1\r\n");
        if !has("host") {
            text.push_str("Host: 127.0.0.1\r\n");
        }
        if !has("content-length") && !has("transfer-encoding") {
            let _ = write!(text, "Content-Length: {}\r\n", body.len());
        }
        for (name, value) in headers {
            let _ = write!(text, "{name}: {value}\r\n");
        }
        text.push_str("\r\n");
        let mut bytes = text.into_bytes();
        bytes.extend_from_slice(body);
        bytes
    }

    /// `None` when the server closed the connection instead of answering.
    fn read_reply(reader: &mut BufReader<TcpStream>, head: bool) -> Option<Reply> {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap() == 0 {
            return None;
        }
        let status = line.split_whitespace().nth(1).unwrap().parse().unwrap();
        let mut headers = Vec::new();
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            let (name, value) = line.split_once(':').unwrap();
            headers.push((name.to_ascii_lowercase(), value.trim().to_owned()));
        }
        let length = headers
            .iter()
            .find(|(name, _)| name == "content-length")
            .map_or(0, |(_, value)| value.parse::<usize>().unwrap());
        let mut body = vec![0; if head { 0 } else { length }];
        reader.read_exact(&mut body).unwrap();
        Some(Reply {
            status,
            headers,
            body,
        })
    }

    fn exchange(reader: &mut BufReader<TcpStream>, request: &[u8]) -> Reply {
        reader.get_mut().write_all(request).unwrap();
        read_reply(reader, request.starts_with(b"HEAD ")).unwrap()
    }

    /// One request on its own connection.
    fn call(
        server: &FakeS3,
        method: &str,
        target: &str,
        headers: &[(&str, &str)],
        body: &[u8],
    ) -> Reply {
        exchange(&mut connect(server), &raw(method, target, headers, body))
    }

    fn status(server: &FakeS3, method: &str, target: &str, body: &[u8]) -> u16 {
        call(server, method, target, &[], body).status
    }

    fn list(server: &FakeS3, bucket: &str, query: &str) -> String {
        let reply = call(
            server,
            "GET",
            &format!("/{bucket}?list-type=2&{query}"),
            &[],
            b"",
        );
        assert_eq!(reply.status, 200, "{}", reply.text());
        reply.text().to_owned()
    }

    fn keys_in(xml: &str) -> Vec<String> {
        elements(xml, "Contents")
            .iter()
            .map(|entry| elements(entry, "Key")[0].to_owned())
            .collect()
    }

    fn prefixes_in(xml: &str) -> Vec<String> {
        elements(xml, "CommonPrefixes")
            .iter()
            .map(|entry| elements(entry, "Prefix")[0].to_owned())
            .collect()
    }

    fn first(xml: &str, name: &str) -> Option<String> {
        elements(xml, name).first().map(|text| (*text).to_owned())
    }

    #[test]
    fn a_put_object_round_trips_through_get_and_head() {
        let server = anonymous();
        server.create_bucket("b");
        let put = call(
            &server,
            "PUT",
            "/b/hello.txt",
            &[("Content-Type", "text/plain")],
            b"hello",
        );
        assert_eq!(put.status, 200);
        let etag = put.header("etag").unwrap().to_owned();
        assert!(etag.starts_with('"') && etag.ends_with('"') && etag.len() == 18);
        assert_eq!(server.get("b", "hello.txt").unwrap(), b"hello");
        assert_eq!(server.keys("b"), ["hello.txt"]);

        let get = call(&server, "GET", "/b/hello.txt", &[], b"");
        assert_eq!(get.status, 200);
        assert_eq!(get.body, b"hello");
        assert_eq!(get.header("content-length"), Some("5"));
        assert_eq!(get.header("etag"), Some(etag.as_str()));
        assert_eq!(get.header("last-modified"), Some(LAST_MODIFIED_HTTP));
        assert_eq!(get.header("content-type"), Some("text/plain"));
        assert_eq!(get.header("accept-ranges"), Some("bytes"));
        assert!(get.header("x-amz-request-id").is_some());

        let head = call(&server, "HEAD", "/b/hello.txt", &[], b"");
        assert_eq!(head.status, 200);
        assert_eq!(head.header("content-length"), Some("5"));
        assert_eq!(head.header("etag"), Some(etag.as_str()));
        assert!(head.body.is_empty());

        server.put("b", "raw", b"xyz");
        let raw = call(&server, "GET", "/b/raw", &[], b"");
        assert_eq!(raw.header("content-type"), Some(DEFAULT_CONTENT_TYPE));
    }

    #[test]
    fn ranges_answer_206_and_unsatisfiable_ranges_416() {
        let server = anonymous();
        server.put("b", "digits", b"0123456789");
        server.put("b", "empty", b"");
        let range = |key: &str, range: &str| {
            call(
                &server,
                "GET",
                &format!("/b/{key}"),
                &[("Range", range)],
                b"",
            )
        };

        let middle = range("digits", "bytes=2-4");
        assert_eq!(middle.status, 206);
        assert_eq!(middle.body, b"234");
        assert_eq!(middle.header("content-range"), Some("bytes 2-4/10"));
        assert_eq!(middle.header("content-length"), Some("3"));

        let tail = range("digits", "bytes=7-");
        assert_eq!((tail.status, tail.body.as_slice()), (206, &b"789"[..]));
        assert_eq!(tail.header("content-range"), Some("bytes 7-9/10"));

        let suffix = range("digits", "bytes=-3");
        assert_eq!((suffix.status, suffix.body.as_slice()), (206, &b"789"[..]));
        assert_eq!(suffix.header("content-range"), Some("bytes 7-9/10"));

        let clipped = range("digits", "bytes=2-100");
        assert_eq!(clipped.body, b"23456789");
        assert_eq!(clipped.header("content-range"), Some("bytes 2-9/10"));

        let long_suffix = range("digits", "bytes=-100");
        assert_eq!(long_suffix.header("content-range"), Some("bytes 0-9/10"));

        let past = range("digits", "bytes=10-");
        assert_eq!(past.status, 416);
        assert_eq!(past.header("content-range"), Some("bytes */10"));
        assert!(past.text().contains("<Code>InvalidRange</Code>"));
        assert!(
            past.text()
                .contains("<ActualObjectSize>10</ActualObjectSize>")
        );

        let empty = range("empty", "bytes=0-");
        assert_eq!(empty.status, 416);
        assert_eq!(empty.header("content-range"), Some("bytes */0"));

        let garbage = range("digits", "bytes=x-y");
        assert_eq!(
            (garbage.status, garbage.body.as_slice()),
            (200, &b"0123456789"[..])
        );

        let head = call(&server, "HEAD", "/b/digits", &[("Range", "bytes=2-4")], b"");
        assert_eq!(head.status, 206);
        assert_eq!(head.header("content-length"), Some("3"));
        assert!(head.body.is_empty());
    }

    #[test]
    fn absent_objects_and_buckets_answer_404_documents() {
        let server = anonymous();
        server.create_bucket("b");
        let key = call(&server, "GET", "/b/missing", &[], b"");
        assert_eq!(key.status, 404);
        assert!(key.text().contains("<Code>NoSuchKey</Code>"));
        assert!(key.text().contains("<Key>missing</Key>"));
        assert!(
            key.text()
                .starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Error>")
        );
        assert_eq!(key.header("content-type"), Some("application/xml"));

        let bucket = call(&server, "GET", "/nobucket/k", &[], b"");
        assert_eq!(bucket.status, 404);
        assert!(bucket.text().contains("<Code>NoSuchBucket</Code>"));

        let head = call(&server, "HEAD", "/b/missing", &[], b"");
        assert_eq!(head.status, 404);
        assert_eq!(head.header("content-length"), Some("0"));
        assert!(head.body.is_empty());

        let delete = call(&server, "DELETE", "/b/missing", &[], b"");
        assert_eq!(delete.status, 204);
        let delete = call(&server, "DELETE", "/nobucket/missing", &[], b"");
        assert_eq!(delete.status, 404);
    }

    #[test]
    fn bucket_lifecycle_follows_s3_status_codes() {
        let server = anonymous();
        assert_eq!(call(&server, "HEAD", "/b", &[], b"").status, 404);
        assert_eq!(call(&server, "PUT", "/b", &[], b"").status, 200);
        let again = call(&server, "PUT", "/b", &[], b"");
        assert_eq!(again.status, 409);
        assert!(
            again
                .text()
                .contains("<Code>BucketAlreadyOwnedByYou</Code>")
        );
        assert_eq!(call(&server, "HEAD", "/b", &[], b"").status, 200);
        assert_eq!(server.buckets(), ["b"]);

        assert_eq!(call(&server, "PUT", "/b/k", &[], b"x").status, 200);
        let full = call(&server, "DELETE", "/b", &[], b"");
        assert_eq!(full.status, 409);
        assert!(full.text().contains("<Code>BucketNotEmpty</Code>"));
        assert_eq!(call(&server, "DELETE", "/b/k", &[], b"").status, 204);
        assert_eq!(call(&server, "DELETE", "/b", &[], b"").status, 204);
        assert_eq!(call(&server, "HEAD", "/b", &[], b"").status, 404);
        assert_eq!(call(&server, "DELETE", "/b", &[], b"").status, 404);
        assert!(server.buckets().is_empty());
    }

    #[test]
    fn conditional_puts_answer_412() {
        let server = anonymous();
        server.create_bucket("b");
        let put = |headers: &[(&str, &str)]| call(&server, "PUT", "/b/k", headers, b"v");
        assert_eq!(put(&[("If-Match", "\"anything\"")]).status, 412);
        let created = put(&[("If-None-Match", "*")]);
        assert_eq!(created.status, 200);
        let etag = created.header("etag").unwrap().to_owned();
        let refused = put(&[("If-None-Match", "*")]);
        assert_eq!(refused.status, 412);
        assert!(refused.text().contains("<Code>PreconditionFailed</Code>"));
        assert_eq!(put(&[("If-Match", "\"0000000000000000\"")]).status, 412);
        assert_eq!(put(&[("If-Match", &etag)]).status, 200);
        assert_eq!(put(&[("If-Match", etag.trim_matches('"'))]).status, 200);
        assert_eq!(put(&[("If-Match", "*")]).status, 200);
        assert_eq!(call(&server, "PUT", "/nobucket/k", &[], b"v").status, 404);
    }

    #[test]
    fn listing_rolls_keys_into_common_prefixes_in_byte_order() {
        let server = anonymous();
        for key in ["d", "b/2", "a.txt", "c/x/y", "b/1", "B"] {
            server.put("b", key, key.as_bytes());
        }
        let flat = list(&server, "b", "");
        assert_eq!(keys_in(&flat), ["B", "a.txt", "b/1", "b/2", "c/x/y", "d"]);
        assert!(prefixes_in(&flat).is_empty());
        assert_eq!(first(&flat, "KeyCount").as_deref(), Some("6"));
        assert_eq!(first(&flat, "MaxKeys").as_deref(), Some("1000"));
        assert_eq!(first(&flat, "IsTruncated").as_deref(), Some("false"));
        assert_eq!(first(&flat, "Name").as_deref(), Some("b"));
        assert!(flat.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<ListBucketResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">"));
        assert!(flat.contains("<Size>5</Size>"));
        assert!(flat.contains("<ETag>&quot;"));
        assert!(flat.contains(&format!("<LastModified>{LAST_MODIFIED_ISO}</LastModified>")));
        assert!(!flat.contains("<Delimiter>"));
        assert!(!flat.contains("<EncodingType>"));

        let rolled = list(&server, "b", "delimiter=/");
        assert_eq!(keys_in(&rolled), ["B", "a.txt", "d"]);
        assert_eq!(prefixes_in(&rolled), ["b/", "c/"]);
        assert_eq!(first(&rolled, "KeyCount").as_deref(), Some("5"));
        assert_eq!(first(&rolled, "Delimiter").as_deref(), Some("/"));
        assert!(rolled.rfind("<Contents>").unwrap() < rolled.find("<CommonPrefixes>").unwrap());

        let under = list(&server, "b", "prefix=b/&delimiter=/");
        assert_eq!(keys_in(&under), ["b/1", "b/2"]);
        assert_eq!(first(&under, "Prefix").as_deref(), Some("b/"));

        let deep = list(&server, "b", "prefix=c/&delimiter=/");
        assert_eq!(keys_in(&deep), Vec::<String>::new());
        assert_eq!(prefixes_in(&deep), ["c/x/"]);

        let nothing = list(&server, "b", "prefix=zzz");
        assert_eq!(first(&nothing, "KeyCount").as_deref(), Some("0"));
        assert_eq!(
            call(&server, "GET", "/nobucket?list-type=2", &[], b"").status,
            404
        );
    }

    #[test]
    fn listing_pages_through_keys_and_prefixes_with_continuation_tokens() {
        let server = anonymous();
        for key in ["a", "b/1", "b/2", "b/3", "c", "d/1"] {
            server.put("b", key, b"");
        }
        let mut pages = Vec::new();
        let mut token = None::<String>;
        loop {
            let query = match &token {
                Some(token) => format!(
                    "delimiter=/&max-keys=2&continuation-token={}",
                    percent_encode(token, false)
                ),
                None => "delimiter=/&max-keys=2".to_owned(),
            };
            let page = list(&server, "b", &query);
            let mut entries = keys_in(&page);
            entries.extend(prefixes_in(&page));
            pages.push(entries);
            if first(&page, "IsTruncated").as_deref() == Some("false") {
                assert!(first(&page, "NextContinuationToken").is_none());
                break;
            }
            let next = first(&page, "NextContinuationToken").unwrap();
            assert_eq!(first(&page, "KeyCount").as_deref(), Some("2"));
            token = Some(next);
        }
        assert_eq!(pages, [vec!["a", "b/"], vec!["c", "d/"]]);

        let one = list(&server, "b", "delimiter=/&max-keys=1");
        assert_eq!(prefixes_in(&one), Vec::<String>::new());
        assert_eq!(keys_in(&one), ["a"]);
        let two = list(&server, "b", "delimiter=/&max-keys=1&start-after=a");
        assert_eq!(prefixes_in(&two), ["b/"]);
        assert_eq!(first(&two, "StartAfter").as_deref(), Some("a"));
        assert_eq!(first(&two, "IsTruncated").as_deref(), Some("true"));
        let token = first(&two, "NextContinuationToken").unwrap();
        let three = list(
            &server,
            "b",
            &format!("delimiter=/&max-keys=1&continuation-token={token}"),
        );
        assert_eq!(keys_in(&three), ["c"]);
        assert_eq!(
            first(&three, "ContinuationToken").as_deref(),
            Some(token.as_str())
        );

        let inside = list(&server, "b", "prefix=b/&max-keys=2");
        assert_eq!(keys_in(&inside), ["b/1", "b/2"]);
        let rest = list(
            &server,
            "b",
            &format!(
                "prefix=b/&max-keys=2&continuation-token={}",
                first(&inside, "NextContinuationToken").unwrap()
            ),
        );
        assert_eq!(keys_in(&rest), ["b/3"]);
        assert_eq!(first(&rest, "IsTruncated").as_deref(), Some("false"));

        let none = list(&server, "b", "max-keys=0");
        assert_eq!(first(&none, "KeyCount").as_deref(), Some("0"));
        assert_eq!(first(&none, "IsTruncated").as_deref(), Some("false"));
        assert_eq!(
            call(&server, "GET", "/b?list-type=2&max-keys=x", &[], b"").status,
            400
        );
    }

    #[test]
    fn a_prefix_at_the_start_position_is_not_emitted_again() {
        let server = anonymous();
        for key in ["photos/", "photos/a", "photos/b/c", "zoo"] {
            server.put("b", key, b"");
        }
        let after = list(&server, "b", "delimiter=/&start-after=photos/");
        assert_eq!(keys_in(&after), ["zoo"]);
        assert!(prefixes_in(&after).is_empty());

        let marker = list(&server, "b", "prefix=photos/&delimiter=/&max-keys=1");
        assert_eq!(keys_in(&marker), ["photos/"]);
        let token = first(&marker, "NextContinuationToken").unwrap();
        assert_eq!(token, "photos/");
        let query = format!("prefix=photos/&delimiter=/&continuation-token={token}");
        let rest = list(&server, "b", &query);
        assert_eq!(keys_in(&rest), ["photos/a"]);
        assert_eq!(prefixes_in(&rest), ["photos/b/"]);
        assert_eq!(first(&rest, "IsTruncated").as_deref(), Some("false"));
    }

    #[test]
    fn listing_percent_encodes_names_when_asked() {
        let server = anonymous();
        server.put("b", "sp ace/\u{e9}.txt", b"");
        server.put("b", "plain", b"");
        let plain = list(&server, "b", "delimiter=/");
        assert_eq!(prefixes_in(&plain), ["sp ace/"]);
        assert_eq!(keys_in(&plain), ["plain"]);

        let encoded = list(
            &server,
            "b",
            "delimiter=/&encoding-type=url&start-after=pl%20a",
        );
        assert_eq!(prefixes_in(&encoded), ["sp%20ace/"]);
        assert_eq!(first(&encoded, "EncodingType").as_deref(), Some("url"));
        assert_eq!(first(&encoded, "Delimiter").as_deref(), Some("%2F"));
        assert_eq!(first(&encoded, "StartAfter").as_deref(), Some("pl%20a"));

        let inside = list(&server, "b", "encoding-type=url&prefix=sp%20ace/");
        assert_eq!(keys_in(&inside), ["sp%20ace/%C3%A9.txt"]);
        assert_eq!(first(&inside, "Prefix").as_deref(), Some("sp%20ace/"));
        let raw = list(&server, "b", "prefix=sp%20ace/");
        assert_eq!(keys_in(&raw), ["sp ace/\u{e9}.txt"]);
        assert_eq!(
            call(
                &server,
                "GET",
                "/b?list-type=2&encoding-type=base64",
                &[],
                b""
            )
            .status,
            400
        );
    }

    /// Initiate an upload of `key` in bucket `b` and answer its id.
    fn initiate(server: &FakeS3, key: &str, headers: &[(&str, &str)]) -> String {
        let reply = call(server, "POST", &format!("/b/{key}?uploads"), headers, b"");
        assert_eq!(reply.status, 200, "{}", reply.text());
        assert!(
            reply
                .text()
                .contains("<InitiateMultipartUploadResult xmlns=")
        );
        first(reply.text(), "UploadId").unwrap()
    }

    #[test]
    fn multipart_uploads_assemble_parts_in_number_order() {
        let server = anonymous();
        server.create_bucket("b");
        let id = initiate(&server, "big", &[("Content-Type", "text/plain")]);
        assert_eq!(server.open_uploads(), 1);
        let part = |number: u32, bytes: &[u8]| {
            let target = format!("/b/big?partNumber={number}&uploadId={id}");
            let reply = call(&server, "PUT", &target, &[], bytes);
            assert_eq!(reply.status, 200);
            reply.header("etag").unwrap().to_owned()
        };
        let etag2 = part(2, b"world");
        let etag1 = part(1, b"hello ");
        let complete = |body: String| {
            let target = format!("/b/big?uploadId={id}");
            call(&server, "POST", &target, &[], body.as_bytes())
        };
        let wrong = complete(format!(
            "<CompleteMultipartUpload><Part><PartNumber>1</PartNumber><ETag>\"bad\"</ETag></Part><Part><PartNumber>2</PartNumber><ETag>{etag2}</ETag></Part></CompleteMultipartUpload>"
        ));
        assert_eq!(wrong.status, 400);
        assert!(wrong.text().contains("<Code>InvalidPart</Code>"));
        assert_eq!(server.open_uploads(), 1);
        let empty = "<CompleteMultipartUpload></CompleteMultipartUpload>";
        assert_eq!(complete(empty.to_owned()).status, 400);

        let done = complete(format!(
            "<CompleteMultipartUpload><Part><PartNumber>2</PartNumber><ETag>{etag2}</ETag></Part><Part><PartNumber>1</PartNumber><ETag>&quot;{}&quot;</ETag></Part></CompleteMultipartUpload>",
            etag1.trim_matches('"')
        ));
        assert_eq!(done.status, 200, "{}", done.text());
        let etag = first(done.text(), "ETag").unwrap();
        assert!(etag.ends_with("-2&quot;"), "{etag}");
        assert_eq!(server.get("b", "big").unwrap(), b"hello world");
        assert_eq!(server.open_uploads(), 0);
        let get = call(&server, "GET", "/b/big", &[], b"");
        assert_eq!(get.header("content-type"), Some("text/plain"));
        assert!(get.header("etag").unwrap().ends_with("-2\""));
        assert_eq!(complete(String::new()).status, 404);
    }

    #[test]
    fn multipart_uploads_abort_and_refuse_unknown_ids() {
        let server = anonymous();
        server.create_bucket("b");
        let id = initiate(&server, "k", &[]);
        let other = initiate(&server, "other", &[]);
        assert_ne!(other, id);
        assert_eq!(server.open_uploads(), 2);
        let unknown = "/b/k?partNumber=1&uploadId=nope";
        assert_eq!(status(&server, "PUT", unknown, b"x"), 404);
        let zero = format!("/b/k?partNumber=0&uploadId={id}");
        assert_eq!(status(&server, "PUT", &zero, b"x"), 400);
        let elsewhere = format!("/b/moved?partNumber=1&uploadId={id}");
        assert_eq!(status(&server, "PUT", &elsewhere, b"x"), 404);
        let abort = format!("/b/other?uploadId={other}");
        assert_eq!(status(&server, "DELETE", &abort, b""), 204);
        assert_eq!(status(&server, "DELETE", &abort, b""), 404);
        assert_eq!(server.open_uploads(), 1);
        assert_eq!(status(&server, "POST", "/nobucket/k?uploads", b""), 404);
    }

    #[test]
    fn bulk_delete_removes_listed_keys_and_reports_each() {
        let server = anonymous();
        server.put("b", "one", b"1");
        server.put("b", "two&more", b"2");
        server.put("b", "keep", b"3");
        let body = "<Delete xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Object><Key>one</Key></Object><Object><Key>two&amp;more</Key></Object><Object><Key>absent</Key></Object></Delete>";
        let reply = call(&server, "POST", "/b?delete", &[], body.as_bytes());
        assert_eq!(reply.status, 200);
        assert!(reply.text().contains("<DeleteResult xmlns="));
        assert_eq!(elements(reply.text(), "Deleted").len(), 3);
        assert!(
            reply
                .text()
                .contains("<Deleted><Key>two&amp;more</Key></Deleted>")
        );
        assert_eq!(server.keys("b"), ["keep"]);

        let quiet = "<Delete><Quiet>true</Quiet><Object><Key>keep</Key></Object></Delete>";
        let reply = call(&server, "POST", "/b?delete", &[], quiet.as_bytes());
        assert_eq!(reply.status, 200);
        assert!(!reply.text().contains("<Deleted>"));
        assert!(server.keys("b").is_empty());

        let mut many = String::from("<Delete>");
        for index in 0..=MAX_DELETE_KEYS {
            let _ = write!(many, "<Object><Key>k{index}</Key></Object>");
        }
        many.push_str("</Delete>");
        let reply = call(&server, "POST", "/b?delete", &[], many.as_bytes());
        assert_eq!(reply.status, 400);
        assert!(reply.text().contains("<Code>MalformedXML</Code>"));
        assert_eq!(
            call(&server, "POST", "/b?delete", &[], b"<Nope/>").status,
            400
        );
        assert_eq!(
            call(&server, "POST", "/nobucket?delete", &[], quiet.as_bytes()).status,
            404
        );
    }

    #[test]
    fn a_100_continue_put_is_acknowledged_before_the_body() {
        let server = anonymous();
        server.create_bucket("b");
        let mut reader = connect(&server);
        let head = raw("PUT", "/b/k", &[("Expect", "100-continue")], b"12345");
        let split = head.len() - 5;
        reader.get_mut().write_all(&head[..split]).unwrap();
        let interim = read_reply(&mut reader, true).unwrap();
        assert_eq!(interim.status, 100);
        assert!(interim.headers.is_empty());
        reader.get_mut().write_all(&head[split..]).unwrap();
        let reply = read_reply(&mut reader, false).unwrap();
        assert_eq!(reply.status, 200);
        assert_eq!(server.get("b", "k").unwrap(), b"12345");
    }

    #[test]
    fn a_chunked_put_body_is_reassembled() {
        let server = anonymous();
        server.create_bucket("b");
        let body = b"7;ext=1\r\nchunked\r\n6\r\n bytes\r\n0\r\nTrailer: x\r\n\r\n";
        let reply = call(
            &server,
            "PUT",
            "/b/k",
            &[("Transfer-Encoding", "chunked")],
            body,
        );
        assert_eq!(reply.status, 200);
        assert_eq!(server.get("b", "k").unwrap(), b"chunked bytes");
        assert_eq!(server.requests()[0].body_len, 13);
    }

    #[test]
    fn two_requests_on_one_connection_both_succeed() {
        let server = anonymous();
        server.create_bucket("b");
        let mut reader = connect(&server);
        let put = exchange(&mut reader, &raw("PUT", "/b/k", &[], b"first"));
        assert_eq!(put.status, 200);
        assert_eq!(put.header("connection"), None);
        let get = exchange(&mut reader, &raw("GET", "/b/k", &[], b""));
        assert_eq!((get.status, get.body.as_slice()), (200, &b"first"[..]));
        let last = exchange(
            &mut reader,
            &raw("HEAD", "/b/k", &[("Connection", "close")], b""),
        );
        assert_eq!(last.status, 200);
        assert_eq!(last.header("connection"), Some("close"));
        assert!(read_reply(&mut reader, false).is_none());
        assert_eq!(server.request_count(), 3);
    }

    #[test]
    fn requests_without_a_signature_are_denied_unless_anonymous() {
        let server = FakeS3::start();
        server.put("b", "k", b"v");
        let signed = |headers: &[(&str, &str)]| call(&server, "GET", "/b/k", headers, b"");
        let denied = signed(&[]);
        assert_eq!(denied.status, 403);
        assert!(denied.text().contains("<Code>AccessDenied</Code>"));

        let authorization = "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20130524/us-east-1/s3/aws4_request, SignedHeaders=host;x-amz-content-sha256;x-amz-date, Signature=fe5f80f77d5fa3beca038a248ff027d0445342fe2855ddc963176630326f1024";
        let stamped = [
            ("Authorization", authorization),
            ("x-amz-date", "20130524T000000Z"),
            ("x-amz-content-sha256", "UNSIGNED-PAYLOAD"),
        ];
        assert_eq!(signed(&stamped).status, 200);
        assert_eq!(signed(&stamped[..2]).status, 403);
        assert_eq!(signed(&[stamped[0], stamped[2]]).status, 403);
        let unsigned_date = authorization.replace(
            "host;x-amz-content-sha256;x-amz-date",
            "host;x-amz-content-sha256",
        );
        assert_eq!(
            signed(&[("Authorization", &unsigned_date), stamped[1], stamped[2]]).status,
            403
        );
        let extra = authorization.replace("x-amz-date,", "x-amz-date;x-amz-security-token,");
        assert_eq!(
            signed(&[("Authorization", &extra), stamped[1], stamped[2]]).status,
            403
        );
        assert_eq!(
            signed(&[
                ("Authorization", &extra),
                stamped[1],
                stamped[2],
                ("x-amz-security-token", "t")
            ])
            .status,
            200
        );
        let short = authorization.replace("Signature=fe5f", "Signature=ff");
        assert_eq!(
            signed(&[("Authorization", &short), stamped[1], stamped[2]]).status,
            403
        );
        let scope = authorization.replace("/s3/aws4_request", "/sqs/aws4_request");
        assert_eq!(
            signed(&[("Authorization", &scope), stamped[1], stamped[2]]).status,
            403
        );

        server.require_access_key(Some("AKIDEXAMPLE"));
        assert_eq!(signed(&stamped).status, 200);
        server.require_access_key(Some("AKIDOTHER"));
        let wrong = signed(&stamped);
        assert_eq!(wrong.status, 403);
        assert!(wrong.text().contains("<Code>InvalidAccessKeyId</Code>"));
        server.require_access_key(None);
        assert_eq!(signed(&stamped).status, 200);

        server.allow_anonymous(true);
        assert_eq!(signed(&[]).status, 200);
    }

    #[test]
    fn a_region_mismatch_answers_400_with_the_bucket_region() {
        let server = FakeS3::start();
        server.put("b", "k", b"v");
        server.set_bucket_region("b", Some("eu-west-1"));
        let signed = |region: &str| {
            let authorization = format!(
                "AWS4-HMAC-SHA256 Credential=AK/20130524/{region}/s3/aws4_request, SignedHeaders=host;x-amz-content-sha256;x-amz-date, Signature={}",
                "0".repeat(64)
            );
            call(
                &server,
                "GET",
                "/b/k",
                &[
                    ("Authorization", &authorization),
                    ("x-amz-date", "20130524T000000Z"),
                    ("x-amz-content-sha256", "UNSIGNED-PAYLOAD"),
                ],
                b"",
            )
        };
        let wrong = signed("us-east-1");
        assert_eq!(wrong.status, 400);
        assert_eq!(wrong.header("x-amz-bucket-region"), Some("eu-west-1"));
        assert!(
            wrong
                .text()
                .contains("<Code>AuthorizationHeaderMalformed</Code>")
        );
        assert!(wrong.text().contains("<Region>eu-west-1</Region>"));
        assert!(wrong.text().contains("expecting &apos;eu-west-1&apos;"));
        let right = signed("eu-west-1");
        assert_eq!(right.status, 200);
        assert_eq!(right.header("x-amz-bucket-region"), Some("eu-west-1"));
        server.set_bucket_region("b", None);
        assert_eq!(signed("us-east-1").status, 200);
        assert_eq!(server.requests().last().unwrap().status, 200);
    }

    #[test]
    fn injected_failures_answer_the_next_requests() {
        let server = anonymous();
        server.put("b", "k", b"v");
        server.fail_next(503, "SlowDown", 2);
        let first_try = call(&server, "GET", "/b/k", &[], b"");
        assert_eq!(first_try.status, 503);
        assert!(first_try.text().contains("<Code>SlowDown</Code>"));
        assert_eq!(call(&server, "GET", "/b/k", &[], b"").status, 503);
        assert_eq!(call(&server, "GET", "/b/k", &[], b"").status, 200);
        assert_eq!(server.request_count(), 3);
        let statuses: Vec<u16> = server
            .requests()
            .iter()
            .map(|recorded| recorded.status)
            .collect();
        assert_eq!(statuses, [503, 503, 200]);
        server.fail_next(500, "InternalError", 0);
        assert_eq!(call(&server, "GET", "/b/k", &[], b"").status, 200);
    }

    #[test]
    fn unknown_operations_answer_400_and_are_recorded() {
        let server = anonymous();
        server.create_bucket("b");
        let reply = call(
            &server,
            "GET",
            "/b?versions&prefix=a%20b",
            &[("X-Custom", " v ")],
            b"",
        );
        assert_eq!(reply.status, 400);
        assert!(reply.text().contains("<Code>InvalidRequest</Code>"));
        assert_eq!(call(&server, "GET", "/", &[], b"").status, 400);
        assert_eq!(call(&server, "PATCH", "/b/k", &[], b"").status, 400);
        let recorded = server.requests();
        assert_eq!(recorded.len(), 3);
        assert_eq!(recorded[0].method, "GET");
        assert_eq!(recorded[0].bucket.as_deref(), Some("b"));
        assert_eq!(recorded[0].key, None);
        assert_eq!(recorded[0].path, "/b");
        assert_eq!(
            recorded[0].query,
            [
                ("versions".to_owned(), String::new()),
                ("prefix".to_owned(), "a b".to_owned())
            ]
        );
        assert!(
            recorded[0]
                .headers
                .contains(&("x-custom".to_owned(), "v".to_owned()))
        );
        assert_eq!(recorded[0].status, 400);
        assert_eq!(recorded[1].bucket, None);
        assert_eq!(recorded[2].key.as_deref(), Some("k"));
    }

    #[test]
    fn a_malformed_request_line_answers_400_and_closes() {
        let server = anonymous();
        let mut reader = connect(&server);
        reader.get_mut().write_all(b"NOT HTTP\r\n\r\n").unwrap();
        let reply = read_reply(&mut reader, false).unwrap();
        assert_eq!(reply.status, 400);
        assert_eq!(reply.header("connection"), Some("close"));
        assert!(read_reply(&mut reader, false).is_none());
        let mut reader = connect(&server);
        reader
            .get_mut()
            .write_all(b"GET /b/k HTTP/1.1\r\nno colon\r\n\r\n")
            .unwrap();
        assert_eq!(read_reply(&mut reader, false).unwrap().status, 400);
        assert_eq!(server.request_count(), 0);
    }

    #[test]
    fn dropping_the_server_returns_promptly_and_closes_the_port() {
        use std::time::{Duration, Instant};

        let server = anonymous();
        let port = server.port();
        let _idle = connect(&server);
        let started = Instant::now();
        drop(server);
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(TcpStream::connect(("127.0.0.1", port)).is_err());
    }

    #[test]
    fn the_host_header_selects_a_virtual_hosted_bucket() {
        let server = anonymous();
        server.create_bucket("hosted");
        let host = format!("hosted.127.0.0.1:{}", server.port());
        assert_eq!(
            call(&server, "PUT", "/k", &[("Host", &host)], b"v").status,
            200
        );
        assert_eq!(server.keys("hosted"), ["k"]);
        let list = call(
            &server,
            "GET",
            "/?list-type=2",
            &[("Host", "hosted.127.0.0.1")],
            b"",
        );
        assert_eq!(keys_in(list.text()), ["k"]);
        let recorded = server.requests();
        assert_eq!(recorded[0].bucket.as_deref(), Some("hosted"));
        assert_eq!(recorded[0].key.as_deref(), Some("k"));
        assert_eq!(recorded[1].key, None);
        assert_eq!(
            server.endpoint(),
            format!("http://127.0.0.1:{}", server.port())
        );
    }

    #[test]
    fn keys_are_percent_decoded_from_the_target() {
        let server = anonymous();
        server.create_bucket("b");
        assert_eq!(
            call(&server, "PUT", "/b/a%20b%2Fc%C3%A9?x=1%202", &[], b"v").status,
            200
        );
        assert_eq!(server.keys("b"), ["a b/c\u{e9}"]);
        let recorded = &server.requests()[0];
        assert_eq!(recorded.path, "/b/a%20b%2Fc%C3%A9");
        assert_eq!(recorded.key.as_deref(), Some("a b/c\u{e9}"));
        assert_eq!(recorded.query, [("x".to_owned(), "1 2".to_owned())]);
        let get = call(&server, "GET", "/b/a%20b/c%C3%A9", &[], b"");
        assert_eq!((get.status, get.body.as_slice()), (200, &b"v"[..]));
    }

    #[test]
    fn recording_can_be_disabled_while_counting_continues() {
        let server = anonymous();
        server.put("b", "k", b"v");
        server.set_recording(false);
        call(&server, "GET", "/b/k", &[], b"");
        call(&server, "GET", "/b/k", &[], b"");
        assert!(server.requests().is_empty());
        assert_eq!(server.request_count(), 2);
        server.set_recording(true);
        call(&server, "GET", "/b/k", &[], b"");
        assert_eq!(server.requests().len(), 1);
        assert_eq!(server.request_count(), 3);
        server.clear_requests();
        assert!(server.requests().is_empty());
        assert_eq!(server.request_count(), 0);
    }

    #[test]
    fn text_helpers_round_trip() {
        assert_eq!(escape_text("a<b>&\"c'"), "a&lt;b&gt;&amp;&quot;c&apos;");
        assert_eq!(
            unescape_text("a&lt;b&gt;&amp;&quot;c&apos;&#65;&#x42;&bogus;&"),
            "a<b>&\"c'AB&bogus;&"
        );
        assert_eq!(
            percent_encode("a b/c~d.e-f_g\u{e9}", true),
            "a%20b/c~d.e-f_g%C3%A9"
        );
        assert_eq!(percent_encode("a/b", false), "a%2Fb");
        assert_eq!(percent_decode("a%20b%2"), "a b%2");
        assert_eq!(percent_decode("%FF"), "%FF");
        assert_eq!(etag_of(b""), "\"cbf29ce484222325\"");
        assert_eq!(
            elements(
                "<Key>1</Key><KeyCount>9</KeyCount><Key attr=\"x\">2</Key>",
                "Key"
            ),
            ["1", "2"]
        );
    }
}
