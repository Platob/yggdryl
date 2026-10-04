#![allow(dead_code)]
//! `rust/tests/support/s3tables.rs`: the in-process fake of the Amazon S3
//! Tables control plane the `s3tables` suites run against.
//!
//! One loopback listener keeps table buckets, namespaces and tables and
//! answers the fifteen operations in botocore's shapes and error types -
//! `GetTable` by name or by `tableArn` alone; a table's version token moves
//! on every change, and a stale token or a deletion of a level that still
//! holds one is a `409 ConflictException`. It is its own socket because
//! `http::Server` refuses a path segment that decodes to a separator, and
//! every label below a table bucket is an ARN. It names nothing of the
//! crate, so nothing it checks is checked by the code under test: the
//! SHA-256 and the HMAC are `ring`'s, the percent-coding and the canonical
//! request follow the SigV4 specification.
//!
//! Every request is checked before it is answered - the credential scope
//! `<date>/<region>/s3tables/aws4_request` signing at least `host` and
//! `x-amz-date` (botocore signs no more for this service), a stated
//! `x-amz-content-sha256` equal to the body's, the signature recomputed over
//! the path normalized and encoded again and the query decoded, encoded
//! again and sorted, each segment encoded once, a well-formed table bucket
//! ARN, `application/json` on a body, a temporary set's token sent and
//! signed, and the model's bounds: a version token of at least one
//! character, a rename that names a target, a partition spec or write order
//! whose fields name columns of the schema beside them. A failure is
//! refused as the service refuses it and recorded as a violation; dropping
//! a fake that holds one fails the test.

use std::collections::{BTreeMap, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use serde_json::{Value, json};

/// The region the fake is in until a test moves it.
pub const REGION: &str = "us-east-1";
/// The account every table bucket belongs to.
pub const ACCOUNT: &str = "123456789012";
/// The long-lived pair the fake accepts from the start.
pub const ACCESS_KEY: &str = "AKIAIOSFODNN7EXAMPLE";
pub const SECRET_KEY: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";
/// The service's own cap on a page.
const MAX_PAGE: usize = 1000;

/// One request the fake handled, as a test inspects it.
#[derive(Clone, Debug)]
pub struct Recorded {
    pub method: String,
    /// The request target exactly as sent, query included.
    pub target: String,
    /// The query pairs, percent-decoded, in wire order.
    pub query: Vec<(String, String)>,
    /// Headers with lowercase names, in wire order.
    pub headers: Vec<(String, String)>,
    /// The body as text.
    pub body: String,
    pub status: u16,
}

impl Recorded {
    /// One header.
    pub fn header(&self, name: &str) -> Option<&str> {
        lookup(&self.headers, name)
    }

    /// One query parameter, decoded.
    pub fn query(&self, name: &str) -> Option<&str> {
        lookup(&self.query, name)
    }

    /// The body as the JSON document it is, `null` for none.
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }

    /// The access key the request was signed with.
    pub fn signer(&self) -> String {
        self.header("authorization")
            .and_then(|value| value.split("Credential=").nth(1))
            .and_then(|rest| rest.split('/').next())
            .unwrap_or_default()
            .to_owned()
    }

    /// `METHOD target`, the line a request-count assertion compares.
    pub fn line(&self) -> String {
        format!("{} {}", self.method, self.target)
    }
}

/// One table as a test reads it out of the fake.
#[derive(Clone, Debug)]
pub struct TableState {
    pub arn: String,
    pub version_token: String,
    pub metadata_location: Option<String>,
    pub warehouse_location: String,
    /// The `schemaV2` document the table was created with, when it had one.
    pub schema: Option<Value>,
    /// The `partitionSpec` it was created with, when it stated one.
    pub partition_spec: Option<Value>,
    /// The `writeOrder` it was created with, when it stated one.
    pub write_order: Option<Value>,
}

struct Table {
    id: String,
    version: u64,
    created: u64,
    modified: u64,
    metadata_location: Option<String>,
    schema: Option<Value>,
    partition_spec: Option<Value>,
    write_order: Option<Value>,
}

impl Table {
    fn warehouse(&self) -> String {
        format!("s3://{}--table-s3", self.id)
    }

    fn token(&self) -> String {
        version_token(self.version)
    }
}

struct Namespace {
    created: u64,
    tables: BTreeMap<String, Table>,
}

struct Bucket {
    created: u64,
    namespaces: BTreeMap<String, Namespace>,
}

/// One scripted answer, given instead of acting.
enum Scripted {
    /// The status, the error type in `x-amzn-ErrorType` and the message.
    Typed(u16, String, String),
    /// The status and a body, under no `x-amzn-ErrorType` at all.
    Body(u16, Value),
}

struct State {
    region: String,
    buckets: BTreeMap<String, Bucket>,
    /// The access keys the fake accepts: each one's secret, and the session
    /// token a temporary set must send.
    keys: BTreeMap<String, (String, Option<String>)>,
    /// The next requests answer these instead of acting, once checked.
    refusals: VecDeque<Scripted>,
    /// The most entries a page holds, whatever the request asks for.
    page_size: usize,
    /// One tick per change: every stamp and every version is one.
    clock: u64,
    recorded: Vec<Recorded>,
    violations: Vec<String>,
}

impl State {
    fn tick(&mut self) -> u64 {
        self.clock += 1;
        self.clock
    }

    fn bucket_arn(&self, name: &str) -> String {
        format!("arn:aws:s3tables:{}:{ACCOUNT}:bucket/{name}", self.region)
    }

    /// The bucket a table bucket ARN names here, when it is one of this
    /// region and account.
    fn bucket_of(&self, arn: &str) -> Option<String> {
        arn.strip_prefix(&format!(
            "arn:aws:s3tables:{}:{ACCOUNT}:bucket/",
            self.region
        ))
        .filter(|name| !name.is_empty() && !name.contains('/'))
        .map(str::to_owned)
    }

    fn table_arn(&self, bucket: &str, table: &Table) -> String {
        format!("{}/table/{}", self.bucket_arn(bucket), table.id)
    }

    /// Record that `request` broke a rule the service holds it to.
    fn violation(&mut self, request: &Request, what: impl std::fmt::Display) {
        self.violations
            .push(format!("{} {}: {what}", request.method, request.target));
    }
}

struct Inner {
    address: String,
    state: Mutex<State>,
    stopping: AtomicBool,
}

impl Inner {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The fake: a loopback listener, its accept thread, and the state every
/// handled request reads and writes. Dropping it stops the listener and
/// fails the test over any violation it still holds.
pub struct S3TablesFake {
    inner: Arc<Inner>,
    accept: Option<JoinHandle<()>>,
}

impl S3TablesFake {
    /// Bind `127.0.0.1:0` and start answering.
    ///
    /// # Panics
    ///
    /// When the loopback listener cannot be bound.
    pub fn start() -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind a loopback listener");
        let address = listener
            .local_addr()
            .expect("a bound listener has an address");
        let inner = Arc::new(Inner {
            address: address.to_string(),
            state: Mutex::new(State {
                region: REGION.to_owned(),
                buckets: BTreeMap::new(),
                keys: BTreeMap::from([(ACCESS_KEY.to_owned(), (SECRET_KEY.to_owned(), None))]),
                refusals: VecDeque::new(),
                page_size: MAX_PAGE,
                clock: 0,
                recorded: Vec::new(),
                violations: Vec::new(),
            }),
            stopping: AtomicBool::new(false),
        });
        let shared = Arc::clone(&inner);
        let accept = std::thread::spawn(move || {
            for connection in listener.incoming() {
                if shared.stopping.load(Ordering::SeqCst) {
                    break;
                }
                if let Ok(stream) = connection {
                    let inner = Arc::clone(&shared);
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

    /// `127.0.0.1:PORT`, the `Host` a request to the fake is signed under.
    pub fn host(&self) -> String {
        self.inner.address.clone()
    }

    /// Answer as the service of `region`: the ARNs minted and the scope a
    /// signature must name both follow it.
    pub fn set_region(&self, region: &str) {
        self.inner.state().region = region.to_owned();
    }

    /// The ARN a table bucket called `name` has here, created or not.
    pub fn bucket_arn(&self, name: &str) -> String {
        self.inner.state().bucket_arn(name)
    }

    /// Accept requests signed by `access_key` with `secret`; a `token` is
    /// the session token such a request must then send and sign.
    pub fn accept_key(&self, access_key: &str, secret: &str, token: Option<&str>) {
        self.inner.state().keys.insert(
            access_key.to_owned(),
            (secret.to_owned(), token.map(str::to_owned)),
        );
    }

    /// Hold at most `size` entries per page, whatever a request asks for.
    pub fn set_page_size(&self, size: usize) {
        self.inner.state().page_size = size.clamp(1, MAX_PAGE);
    }

    /// Answer the next `times` requests - once each has passed its checks -
    /// with `status`, `error_type` and `message` instead of acting. An empty
    /// `error_type` answers with no `x-amzn-ErrorType` at all, as a gateway
    /// in front of the service would.
    pub fn refuse_next(&self, status: u16, error_type: &str, message: &str, times: usize) {
        let mut state = self.inner.state();
        for _ in 0..times {
            state.refusals.push_back(Scripted::Typed(
                status,
                error_type.to_owned(),
                message.to_owned(),
            ));
        }
    }

    /// Answer the next request - once it has passed its checks - with
    /// `status` and `body` and no `x-amzn-ErrorType`: the other two places
    /// the protocol lets a service state an error's type are in the body.
    pub fn answer_next(&self, status: u16, body: Value) {
        self.inner
            .state()
            .refusals
            .push_back(Scripted::Body(status, body));
    }

    /// Put a table bucket in place without a request, and answer its ARN.
    pub fn seed_bucket(&self, name: &str) -> String {
        let mut state = self.inner.state();
        let created = state.tick();
        state.buckets.entry(name.to_owned()).or_insert(Bucket {
            created,
            namespaces: BTreeMap::new(),
        });
        state.bucket_arn(name)
    }

    /// Put a namespace in place without a request, its bucket included.
    pub fn seed_namespace(&self, bucket: &str, namespace: &str) {
        self.seed_bucket(bucket);
        let mut state = self.inner.state();
        let created = state.tick();
        if let Some(held) = state.buckets.get_mut(bucket) {
            held.namespaces
                .entry(namespace.to_owned())
                .or_insert(Namespace {
                    created,
                    tables: BTreeMap::new(),
                });
        }
    }

    /// Put a table in place without a request, its namespace included, and
    /// answer what it holds.
    pub fn seed_table(&self, bucket: &str, namespace: &str, name: &str) -> TableState {
        self.seed_namespace(bucket, namespace);
        let mut state = self.inner.state();
        let table = new_table(&mut state, Some(json!({"type": "struct", "fields": []})));
        if let Some(held) = state
            .buckets
            .get_mut(bucket)
            .and_then(|held| held.namespaces.get_mut(namespace))
        {
            held.tables.entry(name.to_owned()).or_insert(table);
        }
        drop(state);
        self.table(bucket, namespace, name)
            .expect("the table just seeded")
    }

    /// What the table `name` holds now, when the fake holds it.
    pub fn table(&self, bucket: &str, namespace: &str, name: &str) -> Option<TableState> {
        let state = self.inner.state();
        let table = state
            .buckets
            .get(bucket)?
            .namespaces
            .get(namespace)?
            .tables
            .get(name)?;
        Some(TableState {
            arn: state.table_arn(bucket, table),
            version_token: table.token(),
            metadata_location: table.metadata_location.clone(),
            warehouse_location: table.warehouse(),
            schema: table.schema.clone(),
            partition_spec: table.partition_spec.clone(),
            write_order: table.write_order.clone(),
        })
    }

    /// Whether the fake holds the table bucket `name`.
    pub fn has_bucket(&self, name: &str) -> bool {
        self.inner.state().buckets.contains_key(name)
    }

    /// Whether the fake holds the namespace `namespace` of `bucket`.
    pub fn has_namespace(&self, bucket: &str, namespace: &str) -> bool {
        self.inner
            .state()
            .buckets
            .get(bucket)
            .is_some_and(|held| held.namespaces.contains_key(namespace))
    }

    /// The requests handled so far, in arrival order.
    pub fn requests(&self) -> Vec<Recorded> {
        self.inner.state().recorded.clone()
    }

    /// `METHOD target` of every request handled so far.
    pub fn lines(&self) -> Vec<String> {
        self.requests().iter().map(Recorded::line).collect()
    }

    /// `METHOD target` and the status of every request handled so far.
    pub fn answered(&self) -> Vec<(String, u16)> {
        self.requests()
            .iter()
            .map(|request| (request.line(), request.status))
            .collect()
    }

    pub fn request_count(&self) -> usize {
        self.inner.state().recorded.len()
    }

    pub fn clear_requests(&self) {
        self.inner.state().recorded.clear();
    }

    /// Every check a request failed so far, and forget them: what a test
    /// that sends a broken request on purpose reads.
    pub fn take_violations(&self) -> Vec<String> {
        std::mem::take(&mut self.inner.state().violations)
    }

    /// The `authorization` header of a request built by hand, signed with
    /// the fake's own reading of Signature Version 4 over `canonical_uri`:
    /// what a test uses to show which canonical URI the fake accepts.
    ///
    /// `headers` are the signed ones, lowercase names, with `host`,
    /// `x-amz-date` and `x-amz-content-sha256` among them.
    pub fn authorization(
        &self,
        method: &str,
        canonical_uri: &str,
        query: &str,
        headers: &[(String, String)],
        payload_hash: &str,
    ) -> String {
        let region = self.inner.state().region.clone();
        let mut signed: Vec<(&str, &str)> = headers
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        signed.sort_unstable();
        let datetime = signed
            .iter()
            .find(|(name, _)| *name == "x-amz-date")
            .map(|(_, value)| *value)
            .expect("an x-amz-date among the signed headers");
        let query = canonical_query(query).expect("a query that decodes");
        let request = canonical_request(method, canonical_uri, &query, &signed, payload_hash);
        let names: Vec<&str> = signed.iter().map(|(name, _)| *name).collect();
        format!(
            "AWS4-HMAC-SHA256 Credential={ACCESS_KEY}/{}/{region}/s3tables/aws4_request, \
             SignedHeaders={}, Signature={}",
            &datetime[..8],
            names.join(";"),
            signature(SECRET_KEY, datetime, &region, &request)
        )
    }
}

impl Drop for S3TablesFake {
    fn drop(&mut self) {
        self.inner.stopping.store(true, Ordering::SeqCst);
        // Wake the accept loop so it sees the flag.
        let _ = TcpStream::connect(&self.inner.address);
        if let Some(accept) = self.accept.take() {
            let _ = accept.join();
        }
        let violations = self.take_violations();
        if !std::thread::panicking() {
            assert!(
                violations.is_empty(),
                "the fake refused requests the client should never send: {violations:#?}"
            );
        }
    }
}

/// One parsed request.
struct Request {
    method: String,
    /// The target exactly as sent.
    target: String,
    /// The path exactly as sent.
    path: String,
    /// The query exactly as sent, without its `?`.
    raw_query: String,
    query: Vec<(String, String)>,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        lookup(&self.headers, name)
    }

    fn query(&self, name: &str) -> Option<&str> {
        lookup(&self.query, name)
    }

    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
}

/// One answer.
struct Response {
    status: u16,
    error_type: Option<String>,
    body: String,
}

impl Response {
    fn json(status: u16, body: &Value) -> Self {
        Self {
            status,
            error_type: None,
            body: body.to_string(),
        }
    }

    fn empty(status: u16) -> Self {
        Self {
            status,
            error_type: None,
            body: String::new(),
        }
    }

    fn error(status: u16, error_type: &str, message: &str) -> Self {
        Self {
            status,
            error_type: Some(error_type.to_owned()),
            body: json!({ "message": message }).to_string(),
        }
    }

    fn not_found(what: &str) -> Self {
        Self::error(
            404,
            "NotFoundException",
            &format!("The specified {what} does not exist."),
        )
    }

    fn conflict(message: &str) -> Self {
        Self::error(409, "ConflictException", message)
    }

    fn bad_request(message: &str) -> Self {
        Self::error(400, "BadRequestException", message)
    }
}

/// Serve one connection: one request, one answer, close.
fn serve(inner: &Inner, mut stream: TcpStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let Some(request) = read_request(&mut stream) else {
        return;
    };
    let response = {
        let mut state = inner.state();
        let response = answer(&mut state, &request);
        state.recorded.push(Recorded {
            method: request.method.clone(),
            target: request.target.clone(),
            query: request.query.clone(),
            headers: request.headers.clone(),
            body: String::from_utf8_lossy(&request.body).into_owned(),
            status: response.status,
        });
        response
    };
    let mut head = format!(
        "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nConnection: close\r\nx-amzn-RequestId: fake\r\n",
        response.status,
        reason(response.status),
        response.body.len()
    );
    if !response.body.is_empty() {
        head.push_str("Content-Type: application/json\r\n");
    }
    if let Some(error_type) = response
        .error_type
        .as_ref()
        .filter(|named| !named.is_empty())
    {
        // The service qualifies the type after a colon; a client reads what
        // precedes it.
        head.push_str(&format!(
            "x-amzn-ErrorType: {error_type}:http://internal.amazon.com/coral/com.amazonaws.s3tables/\r\n"
        ));
    }
    head.push_str("\r\n");
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(response.body.as_bytes());
    let _ = stream.flush();
}

fn read_request(stream: &mut TcpStream) -> Option<Request> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let mut parts = line.trim_end().splitn(3, ' ');
    let method = parts.next()?.to_owned();
    let target = parts.next()?.to_owned();
    let mut headers = Vec::new();
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).ok()?;
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        let (name, value) = header.split_once(':')?;
        headers.push((name.trim().to_ascii_lowercase(), value.trim().to_owned()));
    }
    let length: usize = lookup(&headers, "content-length")
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0; length];
    if length > 0 {
        reader.read_exact(&mut body).ok()?;
    }
    let (path, raw_query) = match target.split_once('?') {
        Some((path, query)) => (path.to_owned(), query.to_owned()),
        None => (target.clone(), String::new()),
    };
    let query = raw_query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            (
                decode(name).unwrap_or_else(|| name.to_owned()),
                decode(value).unwrap_or_else(|| value.to_owned()),
            )
        })
        .collect();
    Some(Request {
        method,
        target,
        path,
        raw_query,
        query,
        headers,
        body,
    })
}

/// Check one request, then answer it: a scripted refusal, or the operation
/// its method and path name.
fn answer(state: &mut State, request: &Request) -> Response {
    if let Err((refusal, violation)) = verify(state, request) {
        if let Some(violation) = violation {
            state.violation(request, violation);
        }
        return refusal;
    }
    if let Some(violation) = out_of_bounds(request) {
        state.violation(request, violation);
        return Response::bad_request("1 validation error detected.");
    }
    match state.refusals.pop_front() {
        Some(Scripted::Typed(status, error_type, message)) => {
            return Response::error(status, &error_type, &message);
        }
        Some(Scripted::Body(status, body)) => return Response::json(status, &body),
        None => {}
    }
    let mut labels = Vec::new();
    for segment in request.path.trim_start_matches('/').split('/') {
        // A label encoded once decodes, and encodes back to what was sent.
        match decode(segment) {
            Some(label) if encode(&label, false) == segment => labels.push(label),
            _ => {
                state.violation(
                    request,
                    format_args!("the path segment {segment:?} is not percent-encoded once"),
                );
                return Response::bad_request("The request path is malformed.");
            }
        }
    }
    let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
    match (request.method.as_str(), labels.as_slice()) {
        ("PUT", ["buckets"]) => create_bucket(state, request),
        ("GET", ["buckets"]) => list_buckets(state, request),
        ("GET", ["get-table"]) => get_table(state, request),
        (
            method,
            [
                collection @ ("buckets" | "namespaces" | "tables"),
                arn,
                rest @ ..,
            ],
        ) => {
            let bucket = match bucket_named(state, request, arn) {
                Ok(bucket) => bucket,
                Err(refusal) => return refusal,
            };
            match (method, *collection, rest) {
                ("GET", "buckets", []) => get_bucket(state, &bucket),
                ("DELETE", "buckets", []) => delete_bucket(state, &bucket),
                ("PUT", "namespaces", []) => create_namespace(state, request, &bucket),
                ("GET", "namespaces", []) => list_namespaces(state, request, &bucket),
                ("GET", "namespaces", [namespace]) => get_namespace(state, &bucket, namespace),
                ("DELETE", "namespaces", [namespace]) => {
                    delete_namespace(state, &bucket, namespace)
                }
                ("GET", "tables", []) => list_tables(state, request, &bucket),
                ("PUT", "tables", [namespace]) => create_table(state, request, &bucket, namespace),
                ("DELETE", "tables", [namespace, name]) => {
                    delete_table(state, request, &bucket, namespace, name)
                }
                ("PUT", "tables", [namespace, name, "rename"]) => {
                    rename_table(state, request, &bucket, namespace, name)
                }
                ("GET", "tables", [namespace, name, "metadata-location"]) => {
                    get_metadata_location(state, &bucket, namespace, name)
                }
                ("PUT", "tables", [namespace, name, "metadata-location"]) => {
                    update_metadata_location(state, request, &bucket, namespace, name)
                }
                _ => unknown(state, request),
            }
        }
        _ => unknown(state, request),
    }
}

/// A method and path no operation of the model has.
fn unknown(state: &mut State, request: &Request) -> Response {
    state.violation(
        request,
        "no operation of the model has this method and path",
    );
    Response::error(404, "UnknownOperationException", "Unknown operation.")
}

/// The bucket a table bucket ARN names here: a malformed ARN is a
/// violation, and a well-formed one of another region or account names no
/// bucket.
fn bucket_named(state: &mut State, request: &Request, arn: &str) -> Result<String, Response> {
    if let Some(bucket) = state.bucket_of(arn) {
        return Ok(bucket);
    }
    if !is_table_bucket_arn(arn) {
        state.violation(request, format_args!("{arn:?} is not a table bucket ARN"));
        return Err(Response::bad_request("The table bucket ARN is malformed."));
    }
    Err(Response::not_found("table bucket"))
}

/// What a failed check answers, and the violation it records - none for a
/// refusal that says nothing about the client, such as a key the fake was
/// never told of.
type Refused = (Response, Option<String>);

/// Check everything a request must carry whatever it asks for.
fn verify(state: &State, request: &Request) -> Result<(), Refused> {
    let forbidden = |error_type: &str, message: &str, violation: String| {
        (Response::error(403, error_type, message), Some(violation))
    };
    let Some(authorization) = request.header("authorization") else {
        return Err(forbidden(
            "MissingAuthenticationTokenException",
            "Missing Authentication Token",
            "no authorization header".to_owned(),
        ));
    };
    let invalid = |violation: String| {
        forbidden(
            "InvalidSignatureException",
            "The request signature we calculated does not match the signature you provided.",
            violation,
        )
    };
    let Some(fields) = authorization.strip_prefix("AWS4-HMAC-SHA256 ") else {
        return Err(invalid(format!(
            "authorization is not AWS4-HMAC-SHA256: {authorization}"
        )));
    };
    let field = |name: &str| {
        fields
            .split(',')
            .map(str::trim)
            .find_map(|field| field.strip_prefix(name))
    };
    let (Some(credential), Some(signed), Some(presented)) = (
        field("Credential="),
        field("SignedHeaders="),
        field("Signature="),
    ) else {
        return Err(invalid(format!(
            "authorization lacks a credential, signed headers or a signature: {authorization}"
        )));
    };
    let Some(datetime) = request.header("x-amz-date") else {
        return Err(invalid("no x-amz-date header".to_owned()));
    };
    let scope: Vec<&str> = credential.split('/').collect();
    let &[access_key, date, region, service, terminator] = scope.as_slice() else {
        return Err(invalid(format!(
            "the credential scope has not five parts: {credential}"
        )));
    };
    if service != "s3tables" || terminator != "aws4_request" {
        return Err(invalid(format!(
            "the credential scope does not end /s3tables/aws4_request: {credential}"
        )));
    }
    if region != state.region {
        return Err(invalid(format!(
            "the credential scope names the region {region}, the service is in {}",
            state.region
        )));
    }
    if !datetime.starts_with(date) {
        return Err(invalid(format!(
            "the credential scope is dated {date}, x-amz-date says {datetime}"
        )));
    }
    let names: Vec<&str> = signed.split(';').collect();
    for required in ["host", "x-amz-date"] {
        if !names.contains(&required) {
            return Err(invalid(format!("{required} is not signed: {signed}")));
        }
    }
    // The service hashes the body it received; a hash the request states
    // beside it has to be that one.
    let payload_hash = sha256_hex(&request.body);
    if let Some(stated) = request.header("x-amz-content-sha256")
        && stated != payload_hash
    {
        return Err(invalid(format!(
            "x-amz-content-sha256 is {stated}, the body received hashes to {payload_hash}"
        )));
    }
    if !request.body.is_empty() && request.header("content-type") != Some("application/json") {
        return Err(invalid(format!(
            "a body is sent under content-type {:?}, not application/json",
            request.header("content-type")
        )));
    }
    let Some((secret, token)) = state.keys.get(access_key) else {
        return Err((
            Response::error(
                403,
                "UnrecognizedClientException",
                "The security token included in the request is invalid.",
            ),
            None,
        ));
    };
    if let Some(token) = token
        && (request.header("x-amz-security-token") != Some(token.as_str())
            || !names.contains(&"x-amz-security-token"))
    {
        return Err(invalid(format!(
            "the session token of {access_key} is not sent and signed: {signed}"
        )));
    }
    let Some(query) = canonical_query(&request.raw_query) else {
        return Err(invalid(format!(
            "the query {:?} holds an escape that is not two hex digits of UTF-8",
            request.raw_query
        )));
    };
    let mut headers = Vec::with_capacity(names.len());
    for name in &names {
        let Some(value) = request.header(name) else {
            return Err(invalid(format!("{name} is signed and not sent")));
        };
        headers.push((*name, value));
    }
    // Every service but Amazon S3: the path as sent, its empty and dot
    // segments removed, encoded again.
    let uri = encode(&remove_dot_segments(&request.path), true);
    let canonical = canonical_request(&request.method, &uri, &query, &headers, &payload_hash);
    let expected = signature(secret, datetime, region, &canonical);
    if expected != presented {
        return Err(invalid(format!(
            "the signature is {presented}, this canonical request signs to {expected}:\n{canonical}"
        )));
    }
    Ok(())
}

fn create_bucket(state: &mut State, request: &Request) -> Response {
    let document = request.json();
    let Some(name) = document.get("name").and_then(Value::as_str) else {
        return Response::bad_request("The request names no table bucket.");
    };
    if !is_name(name, 3, 63, '-') {
        return Response::bad_request("The table bucket name is not valid.");
    }
    if state.buckets.contains_key(name) {
        return Response::conflict("A table bucket with that name already exists.");
    }
    let created = state.tick();
    state.buckets.insert(
        name.to_owned(),
        Bucket {
            created,
            namespaces: BTreeMap::new(),
        },
    );
    Response::json(200, &json!({ "arn": state.bucket_arn(name) }))
}

fn bucket_document(state: &State, name: &str, bucket: &Bucket) -> Value {
    json!({
        "arn": state.bucket_arn(name),
        "name": name,
        "ownerAccountId": ACCOUNT,
        "createdAt": stamp(bucket.created),
        "tableBucketId": format!("bucket-{}", bucket.created),
        "type": "customer",
    })
}

fn get_bucket(state: &State, name: &str) -> Response {
    match state.buckets.get(name) {
        Some(bucket) => Response::json(200, &bucket_document(state, name, bucket)),
        None => Response::not_found("table bucket"),
    }
}

fn list_buckets(state: &State, request: &Request) -> Response {
    let entries: Vec<(String, Value)> = state
        .buckets
        .iter()
        .map(|(name, bucket)| (name.clone(), bucket_document(state, name, bucket)))
        .collect();
    page(state, request, "tableBuckets", "maxBuckets", entries)
}

fn delete_bucket(state: &mut State, name: &str) -> Response {
    match state.buckets.get(name) {
        None => Response::not_found("table bucket"),
        Some(bucket) if !bucket.namespaces.is_empty() => {
            Response::conflict("The bucket that you tried to delete is not empty.")
        }
        Some(_) => {
            state.buckets.remove(name);
            Response::empty(204)
        }
    }
}

fn create_namespace(state: &mut State, request: &Request, bucket: &str) -> Response {
    let document = request.json();
    let names: Vec<&str> = document
        .get("namespace")
        .and_then(Value::as_array)
        .map(|names| names.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let &[namespace] = names.as_slice() else {
        return Response::bad_request("The request names not exactly one namespace.");
    };
    if !is_name(namespace, 1, 255, '_') {
        return Response::bad_request("The namespace name is not valid.");
    }
    let created = state.tick();
    let arn = state.bucket_arn(bucket);
    let Some(held) = state.buckets.get_mut(bucket) else {
        return Response::not_found("table bucket");
    };
    if held.namespaces.contains_key(namespace) {
        return Response::conflict("A namespace with that name already exists.");
    }
    held.namespaces.insert(
        namespace.to_owned(),
        Namespace {
            created,
            tables: BTreeMap::new(),
        },
    );
    Response::json(
        200,
        &json!({ "tableBucketARN": arn, "namespace": [namespace] }),
    )
}

fn namespace_document(name: &str, namespace: &Namespace) -> Value {
    json!({
        "namespace": [name],
        "createdAt": stamp(namespace.created),
        "createdBy": ACCOUNT,
        "ownerAccountId": ACCOUNT,
        "namespaceId": format!("namespace-{}", namespace.created),
    })
}

fn get_namespace(state: &State, bucket: &str, namespace: &str) -> Response {
    match state
        .buckets
        .get(bucket)
        .and_then(|held| held.namespaces.get(namespace))
    {
        Some(held) => Response::json(200, &namespace_document(namespace, held)),
        None => Response::not_found("namespace"),
    }
}

fn list_namespaces(state: &State, request: &Request, bucket: &str) -> Response {
    let Some(held) = state.buckets.get(bucket) else {
        return Response::not_found("table bucket");
    };
    let entries = held
        .namespaces
        .iter()
        .map(|(name, namespace)| (name.clone(), namespace_document(name, namespace)))
        .collect();
    page(state, request, "namespaces", "maxNamespaces", entries)
}

fn delete_namespace(state: &mut State, bucket: &str, namespace: &str) -> Response {
    let Some(held) = state.buckets.get_mut(bucket) else {
        return Response::not_found("table bucket");
    };
    match held.namespaces.get(namespace) {
        None => Response::not_found("namespace"),
        Some(found) if !found.tables.is_empty() => {
            Response::conflict("The namespace that you tried to delete is not empty.")
        }
        Some(_) => {
            held.namespaces.remove(namespace);
            Response::empty(204)
        }
    }
}

/// A table that nothing holds yet, with a first metadata file when it is
/// created with a schema.
fn new_table(state: &mut State, schema: Option<Value>) -> Table {
    let created = state.tick();
    let id = format!("{created:08x}-0000-4000-8000-{created:012x}");
    let metadata_location = schema
        .is_some()
        .then(|| format!("s3://{id}--table-s3/metadata/00000-{id}.metadata.json"));
    Table {
        id,
        version: created,
        created,
        modified: created,
        metadata_location,
        schema,
        partition_spec: None,
        write_order: None,
    }
}

/// What a creation's `iceberg` member lays a table out as: its schema, its
/// partition spec and its write order, each where it states one.
type Layout = (Option<Value>, Option<Value>, Option<Value>);

/// The schema, the partition spec and the write order an `iceberg` member
/// of a creation states, each checked against the model: a partition field
/// and a sort field name a column of the schema by its id, a partition
/// field carries a transform and a name, a sort field a direction and a
/// null order the model lists.
fn iceberg_layout(iceberg: &Value) -> Result<Layout, String> {
    let Some(members) = iceberg.as_object() else {
        return Err("The iceberg metadata is not an object.".to_owned());
    };
    if let Some(unknown) = members
        .keys()
        .find(|key| !matches!(key.as_str(), "schemaV2" | "partitionSpec" | "writeOrder"))
    {
        return Err(format!(
            "The iceberg metadata member {unknown:?} is not one this fake models."
        ));
    }
    let Some(schema) = iceberg
        .get("schemaV2")
        .filter(|schema| is_schema_v2(schema))
    else {
        return Err("The table metadata states no valid schemaV2.".to_owned());
    };
    let mut ids = Vec::new();
    collect_ids(schema, &mut ids);
    let names_a_column = |field: &Value| {
        field
            .get("source-id")
            .and_then(Value::as_i64)
            .is_some_and(|id| ids.contains(&id))
    };
    let text =
        |field: &Value, member: &str| field.get(member).and_then(Value::as_str).map(str::to_owned);
    let partition_spec = match iceberg.get("partitionSpec") {
        None => None,
        Some(spec) => {
            let fields = spec.get("fields").and_then(Value::as_array);
            let valid = fields.is_some_and(|fields| {
                fields.iter().all(|field| {
                    names_a_column(field)
                        && text(field, "transform").is_some()
                        && text(field, "name").is_some()
                        && field.get("field-id").is_none_or(Value::is_i64)
                })
            }) && spec.get("spec-id").is_none_or(Value::is_i64);
            if !valid {
                return Err("The partition spec is not valid for the schema.".to_owned());
            }
            Some(spec.clone())
        }
    };
    let write_order = match iceberg.get("writeOrder") {
        None => None,
        Some(order) => {
            let fields = order.get("fields").and_then(Value::as_array);
            let valid = order.get("order-id").is_some_and(Value::is_i64)
                && fields.is_some_and(|fields| {
                    fields.iter().all(|field| {
                        names_a_column(field)
                            && text(field, "transform").is_some()
                            && matches!(text(field, "direction").as_deref(), Some("asc" | "desc"))
                            && matches!(
                                text(field, "null-order").as_deref(),
                                Some("nulls-first" | "nulls-last")
                            )
                    })
                });
            if !valid {
                return Err("The write order is not valid for the schema.".to_owned());
            }
            Some(order.clone())
        }
    };
    Ok((Some(schema.clone()), partition_spec, write_order))
}

/// Every field id a `schemaV2` document states, nested ones included.
fn collect_ids(value: &Value, ids: &mut Vec<i64>) {
    match value {
        Value::Object(members) => {
            for (key, member) in members {
                if matches!(key.as_str(), "id" | "element-id" | "key-id" | "value-id")
                    && let Some(id) = member.as_i64()
                {
                    ids.push(id);
                }
                collect_ids(member, ids);
            }
        }
        Value::Array(items) => items.iter().for_each(|item| collect_ids(item, ids)),
        _ => {}
    }
}

/// Whether `arn` is a well-formed table bucket ARN, of any partition,
/// region or account.
fn is_table_bucket_arn(arn: &str) -> bool {
    let parts: Vec<&str> = arn.splitn(6, ':').collect();
    matches!(
        parts.as_slice(),
        ["arn", partition, "s3tables", region, account, resource]
            if !partition.is_empty()
                && !region.is_empty()
                && account.len() == 12
                && account.bytes().all(|byte| byte.is_ascii_digit())
                && resource
                    .strip_prefix("bucket/")
                    .is_some_and(|name| is_name(name, 3, 63, '-'))
    )
}

/// What a request states past the bounds the model sets - a check botocore
/// makes before it sends anything, so the service never sees it from that
/// client and a request of this one that does is a defect.
fn out_of_bounds(request: &Request) -> Option<String> {
    let document = request.json();
    let token = request
        .query("versionToken")
        .or_else(|| document.get("versionToken").and_then(Value::as_str));
    if token == Some("") {
        return Some("an empty versionToken, which the model bounds at one character".to_owned());
    }
    if request.method == "PUT"
        && request.path.ends_with("/rename")
        && document.get("newNamespaceName").is_none()
        && document.get("newName").is_none()
    {
        return Some("a rename that names no new namespace and no new name".to_owned());
    }
    None
}

/// Whether `schema` is the `schemaV2` structure the model states: a
/// `struct` whose every field has an integer `id`, a `name`, a `type` and
/// a boolean `required`.
fn is_schema_v2(schema: &Value) -> bool {
    schema.get("type").and_then(Value::as_str) == Some("struct")
        && schema
            .get("fields")
            .and_then(Value::as_array)
            .is_some_and(|fields| {
                fields.iter().all(|field| {
                    field.get("id").is_some_and(Value::is_i64)
                        && field.get("name").is_some_and(Value::is_string)
                        && field.get("type").is_some()
                        && field.get("required").is_some_and(Value::is_boolean)
                })
            })
}

fn create_table(state: &mut State, request: &Request, bucket: &str, namespace: &str) -> Response {
    let document = request.json();
    let Some(name) = document.get("name").and_then(Value::as_str) else {
        return Response::bad_request("The request names no table.");
    };
    if !is_name(name, 1, 255, '_') {
        return Response::bad_request("The table name is not valid.");
    }
    if document.get("format").and_then(Value::as_str) != Some("ICEBERG") {
        return Response::bad_request("The table format is not ICEBERG.");
    }
    let (schema, partition_spec, write_order) = match document.pointer("/metadata/iceberg") {
        None if document.get("metadata").is_none() => (None, None, None),
        None => return Response::bad_request("The table metadata states no iceberg member."),
        Some(iceberg) => match iceberg_layout(iceberg) {
            Ok(layout) => layout,
            Err(message) => return Response::bad_request(&message),
        },
    };
    let exists = match state
        .buckets
        .get(bucket)
        .and_then(|held| held.namespaces.get(namespace))
    {
        None => return Response::not_found("namespace"),
        Some(held) => held.tables.contains_key(name),
    };
    if exists {
        return Response::conflict("A table with that name already exists in the namespace.");
    }
    let mut table = new_table(state, schema);
    table.partition_spec = partition_spec;
    table.write_order = write_order;
    let answer = json!({
        "tableARN": state.table_arn(bucket, &table),
        "versionToken": table.token(),
    });
    if let Some(held) = state
        .buckets
        .get_mut(bucket)
        .and_then(|held| held.namespaces.get_mut(namespace))
    {
        held.tables.insert(name.to_owned(), table);
    }
    Response::json(200, &answer)
}

/// The table a bucket, a namespace and a name address, or the model's
/// refusal of the level that is missing.
fn table_of<'a>(
    state: &'a State,
    bucket: &str,
    namespace: &str,
    name: &str,
) -> Result<&'a Table, Response> {
    state
        .buckets
        .get(bucket)
        .and_then(|held| held.namespaces.get(namespace))
        .and_then(|held| held.tables.get(name))
        .ok_or_else(|| Response::not_found("table"))
}

/// What `GetTable` describes the table `name` of `bucket.namespace` as.
fn table_document(
    state: &State,
    bucket: &str,
    namespace: &str,
    name: &str,
    table: &Table,
) -> Value {
    let mut document = json!({
        "name": name,
        "type": "customer",
        "tableARN": state.table_arn(bucket, table),
        "namespace": [namespace],
        "versionToken": table.token(),
        "warehouseLocation": table.warehouse(),
        "createdAt": stamp(table.created),
        "createdBy": ACCOUNT,
        "modifiedAt": stamp(table.modified),
        "modifiedBy": ACCOUNT,
        "ownerAccountId": ACCOUNT,
        "format": "ICEBERG",
    });
    if let Some(location) = &table.metadata_location {
        document["metadataLocation"] = json!(location);
    }
    document
}

/// Whether `arn` is a well-formed table ARN: a table bucket's, then
/// `/table/` and the identifier the service gave the table.
fn is_table_arn(arn: &str) -> bool {
    arn.split_once("/table/").is_some_and(|(bucket, id)| {
        is_table_bucket_arn(bucket)
            && !id.is_empty()
            && id
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "-_".contains(character))
    })
}

/// `GetTable` addressed by the table's own ARN, the one query parameter the
/// model lets stand alone: the table that identifier is, wherever a rename
/// has moved it.
fn get_table_by_arn(state: &mut State, request: &Request, arn: &str) -> Response {
    if !is_table_arn(arn) {
        state.violation(
            request,
            format_args!("the query's tableArn {arn:?} is not a table ARN"),
        );
        return Response::bad_request("The table ARN is malformed.");
    }
    if ["tableBucketARN", "namespace", "name"]
        .iter()
        .any(|other| request.query(other).is_some())
    {
        state.violation(
            request,
            "a table addressed by its ARN is addressed by nothing else",
        );
        return Response::bad_request("The request addresses a table two ways.");
    }
    let state = &*state;
    for (bucket, held) in &state.buckets {
        for (namespace, found) in &held.namespaces {
            for (name, table) in &found.tables {
                if state.table_arn(bucket, table) == arn {
                    return Response::json(
                        200,
                        &table_document(state, bucket, namespace, name, table),
                    );
                }
            }
        }
    }
    Response::not_found("table")
}

fn get_table(state: &mut State, request: &Request) -> Response {
    if let Some(arn) = request.query("tableArn") {
        return get_table_by_arn(state, request, arn);
    }
    let (Some(arn), Some(namespace), Some(name)) = (
        request.query("tableBucketARN"),
        request.query("namespace"),
        request.query("name"),
    ) else {
        return Response::bad_request("The request names no table.");
    };
    let bucket = match bucket_named(state, request, arn) {
        Ok(bucket) => bucket,
        Err(refusal) => return refusal,
    };
    let state = &*state;
    match table_of(state, &bucket, namespace, name) {
        Ok(table) => Response::json(200, &table_document(state, &bucket, namespace, name, table)),
        Err(refusal) => refusal,
    }
}

fn list_tables(state: &State, request: &Request, bucket: &str) -> Response {
    let Some(held) = state.buckets.get(bucket) else {
        return Response::not_found("table bucket");
    };
    let only = request.query("namespace");
    if let Some(only) = only
        && !held.namespaces.contains_key(only)
    {
        return Response::not_found("namespace");
    }
    let entries = held
        .namespaces
        .iter()
        .filter(|(namespace, _)| only.is_none_or(|only| only == namespace.as_str()))
        .flat_map(|(namespace, found)| {
            found.tables.iter().map(move |(name, table)| {
                (
                    format!("{namespace}.{name}"),
                    json!({
                        "namespace": [namespace],
                        "name": name,
                        "type": "customer",
                        "tableARN": state.table_arn(bucket, table),
                        "createdAt": stamp(table.created),
                        "modifiedAt": stamp(table.modified),
                    }),
                )
            })
        })
        .collect();
    page(state, request, "tables", "maxTables", entries)
}

/// The table to change, once the version token a request states - where it
/// states one - is the table's own.
fn table_under<'a>(
    state: &'a mut State,
    bucket: &str,
    namespace: &str,
    name: &str,
    token: Option<&str>,
) -> Result<&'a mut Table, Response> {
    let table = state
        .buckets
        .get_mut(bucket)
        .and_then(|held| held.namespaces.get_mut(namespace))
        .and_then(|held| held.tables.get_mut(name))
        .ok_or_else(|| Response::not_found("table"))?;
    match token {
        Some(token) if token != table.token() => Err(Response::conflict(
            "The version token does not match the table's current version.",
        )),
        _ => Ok(table),
    }
}

fn delete_table(
    state: &mut State,
    request: &Request,
    bucket: &str,
    namespace: &str,
    name: &str,
) -> Response {
    if let Err(refusal) = table_under(
        state,
        bucket,
        namespace,
        name,
        request.query("versionToken"),
    ) {
        return refusal;
    }
    if let Some(held) = state
        .buckets
        .get_mut(bucket)
        .and_then(|held| held.namespaces.get_mut(namespace))
    {
        held.tables.remove(name);
    }
    Response::empty(204)
}

fn rename_table(
    state: &mut State,
    request: &Request,
    bucket: &str,
    namespace: &str,
    name: &str,
) -> Response {
    let document = request.json();
    let text = |member: &str| document.get(member).and_then(Value::as_str);
    let target_namespace = text("newNamespaceName").unwrap_or(namespace).to_owned();
    let target_name = text("newName").unwrap_or(name).to_owned();
    if let Err(refusal) = table_under(state, bucket, namespace, name, text("versionToken")) {
        return refusal;
    }
    let moved = state.tick();
    let Some(held) = state.buckets.get_mut(bucket) else {
        return Response::not_found("table bucket");
    };
    match held.namespaces.get(&target_namespace) {
        None => return Response::not_found("namespace"),
        Some(target) if target.tables.contains_key(&target_name) => {
            return Response::conflict("A table with that name already exists in the namespace.");
        }
        Some(_) => {}
    }
    let Some(mut table) = held
        .namespaces
        .get_mut(namespace)
        .and_then(|found| found.tables.remove(name))
    else {
        return Response::not_found("table");
    };
    table.version = moved;
    table.modified = moved;
    if let Some(target) = held.namespaces.get_mut(&target_namespace) {
        target.tables.insert(target_name, table);
    }
    Response::empty(204)
}

fn get_metadata_location(state: &State, bucket: &str, namespace: &str, name: &str) -> Response {
    match table_of(state, bucket, namespace, name) {
        Ok(table) => {
            let mut document = json!({
                "versionToken": table.token(),
                "warehouseLocation": table.warehouse(),
            });
            if let Some(location) = &table.metadata_location {
                document["metadataLocation"] = json!(location);
            }
            Response::json(200, &document)
        }
        Err(refusal) => refusal,
    }
}

fn update_metadata_location(
    state: &mut State,
    request: &Request,
    bucket: &str,
    namespace: &str,
    name: &str,
) -> Response {
    let document = request.json();
    let text = |member: &str| document.get(member).and_then(Value::as_str);
    let (Some(token), Some(location)) = (text("versionToken"), text("metadataLocation")) else {
        return Response::bad_request("The request states no version token or no location.");
    };
    let moved = state.tick();
    let arn = match table_of(state, bucket, namespace, name) {
        Ok(table) => state.table_arn(bucket, table),
        Err(refusal) => return refusal,
    };
    let table = match table_under(state, bucket, namespace, name, Some(token)) {
        Ok(table) => table,
        Err(refusal) => return refusal,
    };
    if !location.starts_with(&format!("{}/", table.warehouse())) {
        return Response::bad_request("The metadata location is not in the table's warehouse.");
    }
    table.version = moved;
    table.modified = moved;
    table.metadata_location = Some(location.to_owned());
    Response::json(
        200,
        &json!({
            "name": name,
            "tableARN": arn,
            "namespace": [namespace],
            "versionToken": table.token(),
            "metadataLocation": location,
        }),
    )
}

/// One page of `entries`, each keyed by the name it sorts under: the ones
/// after the continuation token, as many as the request and the fake allow,
/// and the token of the page after it when there is one.
fn page(
    state: &State,
    request: &Request,
    member: &str,
    limit: &str,
    entries: Vec<(String, Value)>,
) -> Response {
    let after = match request.query("continuationToken") {
        None => None,
        Some(token) => match token
            .strip_prefix("after/")
            .and_then(|rest| rest.strip_suffix("+="))
        {
            Some(after) => Some(after.to_owned()),
            None => return Response::bad_request("The continuation token is not valid."),
        },
    };
    let asked = match request.query(limit).map(str::parse::<usize>) {
        None => MAX_PAGE,
        Some(Ok(asked)) if (1..=MAX_PAGE).contains(&asked) => asked,
        Some(_) => return Response::bad_request("The page size is not between 1 and 1000."),
    };
    let size = asked.min(state.page_size);
    let mut rest = entries
        .into_iter()
        .filter(|(key, _)| after.as_ref().is_none_or(|after| key > after));
    let held: Vec<(String, Value)> = rest.by_ref().take(size).collect();
    let mut document = json!({});
    if rest.next().is_some()
        && let Some((last, _)) = held.last()
    {
        // The service's tokens are opaque and not URL-safe; this one carries
        // the three characters a query value has to escape.
        document["continuationToken"] = json!(format!("after/{last}+="));
    }
    document[member] = Value::Array(held.into_iter().map(|(_, entry)| entry).collect());
    Response::json(200, &document)
}

/// Whether `name` is `min` to `max` of the digits, the lower-case letters
/// and `extra`.
fn is_name(name: &str, min: usize, max: usize, extra: char) -> bool {
    (min..=max).contains(&name.len())
        && name.chars().all(|character| {
            character.is_ascii_digit() || character.is_ascii_lowercase() || character == extra
        })
}

/// The version token of the `version`-th change: opaque, and different for
/// every change.
fn version_token(version: u64) -> String {
    format!("{:016x}", version.wrapping_mul(0x9E37_79B9_7F4A_7C15))
}

/// The ISO 8601 instant of tick `tick`, to the microsecond as the service
/// writes one.
fn stamp(tick: u64) -> String {
    format!(
        "2026-01-01T{:02}:{:02}:{:02}.{:06}Z",
        tick / 3600 % 24,
        tick / 60 % 60,
        tick % 60,
        tick % 1_000_000
    )
}

/// Percent-encode `text` with the unreserved set of RFC 3986 kept, and `/`
/// too when it separates segments.
pub fn encode(text: &str, slash_kept: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric()
            || matches!(byte, b'-' | b'_' | b'.' | b'~')
            || (slash_kept && byte == b'/')
        {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Percent-decode `text`, or `None` when an escape is not two hex digits or
/// the bytes are not UTF-8.
fn decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = std::str::from_utf8(bytes.get(index + 1..index + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The value of the first of `pairs` named `name`.
fn lookup<'a>(pairs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    pairs
        .iter()
        .find(|(held, _)| held == name)
        .map(|(_, value)| value.as_str())
}

/// The canonical request Signature Version 4 signs, `headers` in the order
/// they are signed.
fn canonical_request(
    method: &str,
    uri: &str,
    query: &str,
    headers: &[(&str, &str)],
    payload_hash: &str,
) -> String {
    let mut request = format!("{method}\n{uri}\n{query}\n");
    for (name, value) in headers {
        request.push_str(&format!("{name}:{}\n", value.trim()));
    }
    let names: Vec<&str> = headers.iter().map(|(name, _)| *name).collect();
    request.push_str(&format!("\n{}\n{payload_hash}", names.join(";")));
    request
}

/// The canonical query of a query string as sent, as the service computes
/// it: each name and value decoded, then encoded again with the unreserved
/// set kept, the pairs sorted by name and then by value - so a pair the
/// client encoded another way signs the way the service reads it. `None`
/// for an escape that does not decode.
fn canonical_query(raw: &str) -> Option<String> {
    let mut pairs = Vec::new();
    for pair in raw.split('&').filter(|pair| !pair.is_empty()) {
        let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
        pairs.push((
            encode(&decode(name)?, false),
            encode(&decode(value)?, false),
        ));
    }
    pairs.sort_unstable();
    let pairs: Vec<String> = pairs
        .into_iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect();
    Some(pairs.join("&"))
}

/// The path the service signs: RFC 3986's dot segments removed, and the
/// empty segments too, as every AWS service but Amazon S3 removes them.
fn remove_dot_segments(path: &str) -> String {
    let mut kept: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                kept.pop();
            }
            segment => kept.push(segment),
        }
    }
    let first = if path.starts_with('/') { "/" } else { "" };
    let last = if path.ends_with('/') && !kept.is_empty() {
        "/"
    } else {
        ""
    };
    format!("{first}{}{last}", kept.join("/"))
}

/// The Signature Version 4 signature of `canonical_request` for the
/// `s3tables` service in `region`, at the instant `datetime`
/// (`YYYYMMDDTHHMMSSZ`), under `secret`.
fn signature(secret: &str, datetime: &str, region: &str, canonical_request: &str) -> String {
    let date = datetime.get(..8).unwrap_or(datetime);
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{datetime}\n{date}/{region}/s3tables/aws4_request\n{}",
        sha256_hex(canonical_request.as_bytes())
    );
    let mut key = hmac_sha256(format!("AWS4{secret}").as_bytes(), date.as_bytes());
    for part in [region, "s3tables", "aws4_request"] {
        key = hmac_sha256(&key, part.as_bytes());
    }
    hex(&hmac_sha256(&key, string_to_sign.as_bytes()))
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, key);
    ring::hmac::sign(&key, data).as_ref().to_vec()
}

/// The lowercase hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(ring::digest::digest(&ring::digest::SHA256, bytes).as_ref())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Unknown",
    }
}
