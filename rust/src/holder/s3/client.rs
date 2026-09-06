//! One signed, pooled HTTP client speaking the S3 REST API.
//!
//! Every remote call the backend makes goes through [`Client`], and each of its
//! operations is exactly one request unless a retry or a region discovery adds
//! another. That is the whole point of putting them here: the request count of
//! a handle operation is readable from the operation it calls, and
//! [`Client::stats`] reports what actually went out.

use std::io::Read;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime};

use base64::Engine as _;

use super::credentials::{CredentialCache, CredentialSource, Credentials, variable};
use super::options::S3Options;
use super::sign::{self, Signer};
use super::xml;
use crate::{Error, Result, Url};

/// The region assumed when nothing names one; also the signing region for the
/// `GetBucketLocation`-free discovery a redirect performs.
const DEFAULT_REGION: &str = "us-east-1";
/// Keys per `DeleteObjects` request, the maximum S3 accepts.
pub(super) const DELETE_BATCH: usize = 1000;
/// Base of the exponential backoff between attempts.
const RETRY_BACKOFF: Duration = Duration::from_millis(50);
/// The service name in every credential scope.
const SERVICE: &str = "s3";

/// What one S3 request answered about an object.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct ObjectMeta {
    /// The object's byte length.
    pub(super) size: u64,
    /// The entity tag, quotes included, when the store gave one.
    pub(super) etag: Option<String>,
    /// The stored `Content-Type`, when the store gave one.
    pub(super) content_type: Option<String>,
}

/// How many requests of each shape have gone out.
///
/// Counted rather than timed, because the number of round trips is the thing
/// this backend is designed around and the thing a regression test can assert.
#[derive(Debug, Default)]
pub(super) struct Stats {
    requests: AtomicU64,
    heads: AtomicU64,
    gets: AtomicU64,
    puts: AtomicU64,
    posts: AtomicU64,
    deletes: AtomicU64,
    lists: AtomicU64,
    retries: AtomicU64,
}

/// A reading of [`Stats`] at one instant.
///
/// ```
/// use yggdryl::holder::s3::StatsSnapshot;
///
/// // Nothing has gone out, so every count is zero.
/// assert_eq!(StatsSnapshot::default().requests, 0);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct StatsSnapshot {
    /// Every request, retries included.
    pub requests: u64,
    /// `HEAD` requests, which read metadata without bytes.
    pub heads: u64,
    /// `GET` requests, whole or ranged.
    pub gets: u64,
    /// `PUT` requests, whole objects and multipart parts.
    pub puts: u64,
    /// `POST` requests: multipart lifecycle and bulk delete.
    pub posts: u64,
    /// `DELETE` requests.
    pub deletes: u64,
    /// Listing requests, which are `GET`s and counted in both.
    pub lists: u64,
    /// Attempts beyond the first, from a retried failure or a region redirect.
    pub retries: u64,
}

impl Stats {
    /// Read every counter.
    pub(super) fn snapshot(&self) -> StatsSnapshot {
        StatsSnapshot {
            requests: self.requests.load(Ordering::Relaxed),
            heads: self.heads.load(Ordering::Relaxed),
            gets: self.gets.load(Ordering::Relaxed),
            puts: self.puts.load(Ordering::Relaxed),
            posts: self.posts.load(Ordering::Relaxed),
            deletes: self.deletes.load(Ordering::Relaxed),
            lists: self.lists.load(Ordering::Relaxed),
            retries: self.retries.load(Ordering::Relaxed),
        }
    }

    /// Count one request of `method`.
    fn record(&self, method: &str) {
        self.requests.fetch_add(1, Ordering::Relaxed);
        let counter = match method {
            "HEAD" => &self.heads,
            "GET" => &self.gets,
            "PUT" => &self.puts,
            "POST" => &self.posts,
            "DELETE" => &self.deletes,
            _ => return,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }
}

/// Where the store is and how buckets are addressed on it.
#[derive(Clone, Debug)]
struct Endpoint {
    /// `https` unless the endpoint said otherwise.
    scheme: String,
    /// The endpoint host, without a bucket and without a port.
    host: String,
    /// The explicit port, when the endpoint named one.
    port: Option<u16>,
    /// Whether the bucket goes in the path rather than in the hostname.
    path_style: bool,
}

impl Endpoint {
    /// The `Host` header for a request against `bucket`.
    fn host_header(&self, bucket: &str) -> String {
        let host = if self.path_style {
            self.host.clone()
        } else {
            format!("{bucket}.{}", self.host)
        };
        match self.port {
            Some(port) => format!("{host}:{port}"),
            None => host,
        }
    }

    /// The request path for `bucket` and a raw `key`.
    fn path(&self, bucket: &str, key: &str) -> String {
        let key = sign::encode_key(key);
        if self.path_style {
            let bucket = sign::encode_key(bucket);
            if key.is_empty() {
                format!("/{bucket}")
            } else {
                format!("/{bucket}/{key}")
            }
        } else {
            format!("/{key}")
        }
    }
}

/// One request, before it is signed and sent.
struct Request<'body> {
    method: &'static str,
    /// What the store calls this operation, for the error it may answer with.
    operation: &'static str,
    bucket: String,
    /// The raw object key, unencoded.
    key: String,
    /// Raw query pairs; the canonical form is built once, at signing.
    query: Vec<(String, String)>,
    /// Headers beyond the ones signing always adds.
    headers: Vec<(String, String)>,
    /// The body, which is re-sent verbatim on a retry.
    body: &'body [u8],
}

impl<'body> Request<'body> {
    fn new(method: &'static str, operation: &'static str, bucket: &str, key: &str) -> Self {
        Self {
            method,
            operation,
            bucket: bucket.to_owned(),
            key: key.to_owned(),
            query: Vec::new(),
            headers: Vec::new(),
            body: &[],
        }
    }

    fn query(mut self, name: &str, value: impl Into<String>) -> Self {
        self.query.push((name.to_owned(), value.into()));
        self
    }

    fn header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.push((name.to_owned(), value.into()));
        self
    }

    fn body(mut self, body: &'body [u8]) -> Self {
        self.body = body;
        self
    }
}

/// A live response: its status, its headers, and its body still on the wire.
type Streamed = (u16, Vec<(String, String)>, Box<dyn Read + Send>);

/// What a request answered.
struct Answer {
    status: u16,
    headers: Vec<(String, String)>,
    /// The body, read whole. Ranged and whole object reads take the streaming
    /// path instead, so nothing here holds an object twice.
    body: Vec<u8>,
}

impl Answer {
    /// One header's value, matched without regard to case.
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header, _)| header.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// A signed, pooled client for one endpoint.
pub(super) struct Client {
    agent: ureq::Agent,
    endpoint: Endpoint,
    /// The signing region, which a redirect can correct once.
    region: RwLock<String>,
    credentials: CredentialCache,
    /// The signer for the current key and region, rebuilt when either changes.
    signer: Mutex<Option<(String, String, Arc<Signer>)>>,
    options: S3Options,
    stats: Stats,
}

impl Client {
    /// Build the client `url` and `options` describe, touching nothing.
    ///
    /// Endpoint, region, and addressing style are settled here from what the
    /// URL and the options say; the credential chain is not walked until the
    /// first request needs to sign one.
    ///
    /// # Errors
    ///
    /// Returns a refusal when the URL names no bucket, or when an endpoint
    /// cannot be read as a location.
    pub(super) fn new(url: &Url, options: S3Options) -> Result<Self> {
        let endpoint = Self::endpoint_of(url, &options)?;
        let region = Self::region_of(url, &options);
        let credentials = if options.anonymous() {
            CredentialSource::Anonymous
        } else if let Some(explicit) = options.credentials() {
            CredentialSource::Fixed(explicit.clone())
        } else if let Some(from_url) = Self::url_credentials(url) {
            CredentialSource::Fixed(from_url)
        } else if options.reads_environment() {
            CredentialSource::Chain {
                profile: options.profile().map(str::to_owned),
            }
        } else {
            CredentialSource::Anonymous
        };
        Ok(Self {
            agent: Self::agent(&options),
            endpoint,
            region: RwLock::new(region),
            credentials: CredentialCache::new(credentials),
            signer: Mutex::new(None),
            options,
            stats: Stats::default(),
        })
    }

    /// The connection pool this client sends on.
    ///
    /// A client whose transport matches the defaults shares one process-wide
    /// agent, so many handles against one store share connections rather than
    /// each opening its own.
    fn agent(options: &S3Options) -> ureq::Agent {
        if !options.has_custom_transport() {
            return shared_agent().clone();
        }
        build_agent(options)
    }

    /// The endpoint the URL and options name.
    fn endpoint_of(url: &Url, options: &S3Options) -> Result<Endpoint> {
        let configured = options.endpoint().map(str::to_owned).or_else(|| {
            options
                .reads_environment()
                .then(|| {
                    variable("AWS_ENDPOINT_URL_S3")
                        .or_else(|| variable("AWS_ENDPOINT_URL"))
                        .or_else(|| super::profile::load(options.profile()).endpoint_url)
                })
                .flatten()
        });
        // An explicitly configured endpoint wins: it is a deliberate choice
        // about where the store is, where a URL only says which object. The
        // URL's own endpoint comes next, ahead of the environment, because it
        // is the location a caller handed over rather than a default.
        let explicit = options.endpoint().map(str::to_owned);
        let from_url = url.s3_endpoint().map(str::to_owned);
        let (scheme, host, port) = match explicit.or(from_url).or(configured) {
            Some(endpoint) => Self::split_endpoint(&endpoint)?,
            None => {
                let region = Self::region_of(url, options);
                (
                    "https".to_owned(),
                    format!("s3.{region}.amazonaws.com"),
                    None,
                )
            }
        };
        // AWS is addressed virtual-hosted, everything else path style, because
        // an S3-compatible store on a bare host rarely resolves bucket
        // subdomains. A bucket holding a dot would break TLS wildcards
        // either way, so it stays in the path.
        let bucket_has_dot = url.bucket().is_some_and(|bucket| bucket.contains('.'));
        let aws = host.to_ascii_lowercase().ends_with(".amazonaws.com")
            || host.to_ascii_lowercase().ends_with(".amazonaws.com.cn");
        let path_style = options
            .path_style()
            .or_else(|| {
                options
                    .reads_environment()
                    .then(|| variable("AWS_S3_FORCE_PATH_STYLE"))
                    .flatten()
                    .map(|value| matches!(value.to_ascii_lowercase().as_str(), "true" | "1"))
            })
            .unwrap_or(!aws || bucket_has_dot);
        Ok(Endpoint {
            scheme,
            host,
            port,
            path_style,
        })
    }

    /// Split `https://host:port` into its parts, defaulting the scheme.
    fn split_endpoint(endpoint: &str) -> Result<(String, String, Option<u16>)> {
        let (scheme, rest) = match endpoint.split_once("://") {
            Some((scheme, rest)) => (scheme.to_ascii_lowercase(), rest),
            None => ("https".to_owned(), endpoint),
        };
        let rest = rest.trim_end_matches('/');
        // An IPv6 literal's own colons belong to the address, so the port is
        // whatever follows the closing bracket - and elsewhere, whatever
        // follows the last colon of a host that has no colons of its own.
        let after_host = if rest.starts_with('[') {
            rest.find(']').map(|close| close + 1)
        } else {
            (rest.matches(':').count() == 1)
                .then(|| rest.rfind(':'))
                .flatten()
        };
        let (host, port) = match after_host {
            Some(split) if rest[split..].starts_with(':') => {
                let port = &rest[split + 1..];
                let port = port.parse::<u16>().map_err(|_| {
                    Error::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        format!(
                            "expected a port number in the S3 endpoint {endpoint:?}, got {port:?}"
                        ),
                    ))
                })?;
                (rest[..split].to_owned(), Some(port))
            }
            _ => (rest.to_owned(), None),
        };
        if host.is_empty() {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("expected a host in the S3 endpoint {endpoint:?}, got none"),
            )));
        }
        Ok((scheme, host, port))
    }

    /// The signing region the URL and options name.
    fn region_of(url: &Url, options: &S3Options) -> String {
        options
            .region()
            .map(str::to_owned)
            .or_else(|| url.region().map(str::to_owned))
            .or_else(|| {
                options
                    .reads_environment()
                    .then(|| {
                        variable("AWS_REGION")
                            .or_else(|| variable("AWS_DEFAULT_REGION"))
                            .or_else(|| super::profile::load(options.profile()).region)
                    })
                    .flatten()
            })
            .unwrap_or_else(|| DEFAULT_REGION.to_owned())
    }

    /// The credentials a URL spells in its user information, if any.
    ///
    /// A URL is a location a caller may have written down, so the keys it
    /// carries are used and then never rendered again: the handle's own URL
    /// has them stripped.
    fn url_credentials(url: &Url) -> Option<Credentials> {
        let user = url.user().filter(|user| !user.is_empty())?;
        let secret = url.password().filter(|secret| !secret.is_empty())?;
        // A key or secret may spell a reserved character as an escape; the
        // one URI parser decodes it, and a value that will not decode is used
        // as it stands rather than silently mangled.
        let decode = |value: &str| {
            crate::uri::percent_decode(value, "s3 credentials")
                .map_or_else(|_| value.to_owned(), |decoded| decoded.into_owned())
        };
        Some(Credentials::new(decode(user), decode(secret)))
    }

    /// The counters of what has gone out.
    pub(super) const fn stats(&self) -> &Stats {
        &self.stats
    }

    /// The options this client was built with.
    pub(super) const fn options(&self) -> &S3Options {
        &self.options
    }

    /// The region requests are currently signed for.
    pub(super) fn region(&self) -> String {
        self.region
            .read()
            .map(|region| region.clone())
            .unwrap_or_else(|_| DEFAULT_REGION.to_owned())
    }

    /// The signer for the current credentials and region.
    ///
    /// `None` when the client is anonymous, which is a valid way to reach a
    /// public bucket rather than a failure.
    fn signer(&self, now: SystemTime) -> Result<Option<Arc<Signer>>> {
        let Some(credentials) = self.credentials.resolve(&self.agent, now)? else {
            return Ok(None);
        };
        let region = self.region();
        let mut slot = self.signer.lock().map_err(|_| poisoned())?;
        if let Some((key, signed_region, signer)) = slot.as_ref() {
            if key == credentials.access_key_id() && *signed_region == region {
                return Ok(Some(signer.clone()));
            }
        }
        let signer = Arc::new(Signer::new(
            credentials.access_key_id(),
            credentials.secret_access_key(),
            credentials.session_token().map(str::to_owned),
            &region,
        ));
        *slot = Some((
            credentials.access_key_id().to_owned(),
            region,
            signer.clone(),
        ));
        Ok(Some(signer))
    }

    /// Send `request`, retrying what is worth retrying.
    ///
    /// One attempt is the rule; a retry happens only for a transport failure,
    /// a throttle, a server-side error, or the one region redirect that
    /// corrects the signing region. Every attempt is counted, so a test reads
    /// the true number of round trips rather than the intended one.
    fn send(&self, request: &Request<'_>) -> Result<Answer> {
        let mut attempt = 0;
        let mut redirected = false;
        loop {
            attempt += 1;
            if attempt > 1 {
                self.stats.retries.fetch_add(1, Ordering::Relaxed);
            }
            let outcome = self.attempt(request);
            let answer = match outcome {
                Ok(answer) => answer,
                Err(error) => {
                    if attempt < self.options.max_attempts() && is_retryable_transport(&error) {
                        std::thread::sleep(backoff(attempt));
                        continue;
                    }
                    return Err(transport_failure(request, self.location(request), error));
                }
            };
            // A bucket in another region answers with the region it is in, so
            // the correction costs one redirect rather than a lookup per client.
            if !redirected {
                if let Some(region) = bucket_region_of(&answer) {
                    if region != self.region() {
                        redirected = true;
                        self.adopt_region(region)?;
                        continue;
                    }
                }
            }
            if (answer.status >= 500 || answer.status == 429)
                && attempt < self.options.max_attempts()
            {
                std::thread::sleep(backoff(attempt));
                continue;
            }
            return Ok(answer);
        }
    }

    /// Sign and send one attempt.
    fn attempt(&self, request: &Request<'_>) -> std::result::Result<Answer, ureq::Error> {
        let now = SystemTime::now();
        let host = self.endpoint.host_header(&request.bucket);
        let path = self.endpoint.path(&request.bucket, &request.key);
        let query = sign::canonical_query(&request.query);
        let target = if query.is_empty() {
            format!("{}://{host}{path}", self.endpoint.scheme)
        } else {
            format!("{}://{host}{path}?{query}", self.endpoint.scheme)
        };

        let mut headers = request.headers.clone();
        // The signer is asked for last so its own headers cannot be shadowed.
        match self.signer(now) {
            Ok(Some(signer)) => {
                let payload = sign::sha256_hex(request.body);
                headers.extend(signer.sign(
                    request.method,
                    &host,
                    &path,
                    &request.query,
                    &request.headers,
                    &payload,
                    now,
                ));
            }
            Ok(None) => {}
            // A credential chain that failed is reported as a transport
            // failure, which is what it is from the request's point of view.
            Err(error) => {
                return Err(ureq::Error::Io(std::io::Error::other(error.to_string())));
            }
        }

        self.stats.record(request.method);
        let mut wire = ureq::http::Request::builder()
            .method(request.method)
            .uri(&target);
        for (name, value) in &headers {
            wire = wire.header(name.as_str(), value.as_str());
        }
        let wire = wire.body(request.body).map_err(ureq::Error::Http)?;
        let mut response = self.agent.run(wire)?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.as_str().to_owned(), value.to_owned()))
            })
            .collect();
        // Only small documents come back here - listings, errors, multipart
        // results. Object bytes never do: they take the streaming path.
        let body = response
            .body_mut()
            .with_config()
            .limit(MAX_DOCUMENT)
            .read_to_vec()?;
        Ok(Answer {
            status,
            headers,
            body,
        })
    }

    /// Sign a request and hand back its live response, body unread.
    ///
    /// This is the one path object bytes take, so a whole-value read or a
    /// ranged read transfers straight into the caller's buffer rather than
    /// through an intermediate copy. It retries and follows a region redirect
    /// exactly as [`Self::send`] does - the body is untouched until the status
    /// says the answer is the one to keep - and stops retrying the moment the
    /// caller starts reading it.
    fn stream(&self, request: &Request<'_>) -> Result<Streamed> {
        let mut attempt = 0;
        let mut redirected = false;
        loop {
            attempt += 1;
            if attempt > 1 {
                self.stats.retries.fetch_add(1, Ordering::Relaxed);
            }
            let opened = self.open_stream(request);
            let (status, headers, mut reader) = match opened {
                Ok(opened) => opened,
                Err(error) => {
                    if attempt < self.options.max_attempts() && is_retryable_transport(&error) {
                        std::thread::sleep(backoff(attempt));
                        continue;
                    }
                    return Err(transport_failure(request, self.location(request), error));
                }
            };
            // Only a failing answer is read here, and only to decide: a
            // successful body belongs to the caller, untouched.
            if status >= 300 {
                let mut body = Vec::new();
                reader
                    .by_ref()
                    .take(MAX_DOCUMENT)
                    .read_to_end(&mut body)
                    .ok();
                let answer = Answer {
                    status,
                    headers,
                    body,
                };
                if !redirected {
                    if let Some(region) = bucket_region_of(&answer) {
                        if region != self.region() {
                            redirected = true;
                            self.adopt_region(region)?;
                            continue;
                        }
                    }
                }
                if (answer.status >= 500 || answer.status == 429)
                    && attempt < self.options.max_attempts()
                {
                    std::thread::sleep(backoff(attempt));
                    continue;
                }
                return Ok((
                    answer.status,
                    answer.headers,
                    Box::new(std::io::Cursor::new(answer.body)),
                ));
            }
            return Ok((status, headers, reader));
        }
    }

    /// One attempt at opening a streamed response.
    fn open_stream(&self, request: &Request<'_>) -> std::result::Result<Streamed, ureq::Error> {
        let now = SystemTime::now();
        let host = self.endpoint.host_header(&request.bucket);
        let path = self.endpoint.path(&request.bucket, &request.key);
        let query = sign::canonical_query(&request.query);
        let target = if query.is_empty() {
            format!("{}://{host}{path}", self.endpoint.scheme)
        } else {
            format!("{}://{host}{path}?{query}", self.endpoint.scheme)
        };
        let mut headers = request.headers.clone();
        match self.signer(now) {
            Ok(Some(signer)) => headers.extend(signer.sign(
                request.method,
                &host,
                &path,
                &request.query,
                &request.headers,
                sign::EMPTY_PAYLOAD_SHA256,
                now,
            )),
            Ok(None) => {}
            Err(error) => return Err(ureq::Error::Io(std::io::Error::other(error.to_string()))),
        }
        self.stats.record(request.method);
        let mut wire = ureq::http::Request::builder()
            .method(request.method)
            .uri(&target);
        for (name, value) in &headers {
            wire = wire.header(name.as_str(), value.as_str());
        }
        let wire = wire.body(()).map_err(ureq::Error::Http)?;
        let response = self.agent.run(wire)?;
        let status = response.status().as_u16();
        let headers: Vec<(String, String)> = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.as_str().to_owned(), value.to_owned()))
            })
            .collect();
        let reader = response
            .into_body()
            .into_with_config()
            .limit(u64::MAX)
            .reader();
        Ok((status, headers, Box::new(reader)))
    }

    /// Sign for `region` from here on, dropping the signer bound to the old one.
    fn adopt_region(&self, region: String) -> Result<()> {
        {
            let mut current = self.region.write().map_err(|_| poisoned())?;
            *current = region;
        }
        let mut signer = self.signer.lock().map_err(|_| poisoned())?;
        *signer = None;
        Ok(())
    }

    /// The canonical location a request addresses, for its error.
    fn location(&self, request: &Request<'_>) -> String {
        if request.key.is_empty() {
            format!("s3://{}/", request.bucket)
        } else {
            format!("s3://{}/{}", request.bucket, request.key)
        }
    }

    /// Turn a non-2xx answer into the typed failure it means.
    fn failure(&self, request: &Request<'_>, answer: &Answer) -> Error {
        let path = self.location(request);
        let body = xml::parse_error(&answer.body);
        let code = body.as_ref().map_or_else(
            || status_code_name(answer.status),
            |error| error.code.clone(),
        );
        let message = body.as_ref().map_or_else(
            || format!("the store answered {}", answer.status),
            |error| error.message.clone(),
        );
        match answer.status {
            404 => Error::absent(absent_kind(request), path),
            // A conditional write lost, which is what a create reports.
            409 | 412 => Error::conflict(absent_kind(request), absent_kind(request), path),
            _ => Error::remote(
                SERVICE,
                request.operation,
                answer.status,
                code,
                message,
                path,
            ),
        }
    }

    /// Read one object's metadata.
    ///
    /// One `HEAD`. Absence answers `None` rather than failing, per the
    /// laziness contract.
    ///
    /// # Errors
    ///
    /// Returns the store's refusal for anything that is not a 404.
    pub(super) fn head_object(&self, bucket: &str, key: &str) -> Result<Option<ObjectMeta>> {
        let request = Request::new("HEAD", "HeadObject", bucket, key);
        let answer = self.send(&request)?;
        if answer.status == 404 {
            return Ok(None);
        }
        if answer.status >= 300 {
            return Err(self.failure(&request, &answer));
        }
        Ok(Some(ObjectMeta {
            size: answer
                .header("content-length")
                .and_then(|value| value.trim().parse().ok())
                .unwrap_or(0),
            etag: answer.header("etag").map(str::to_owned),
            content_type: answer.header("content-type").map(str::to_owned),
        }))
    }

    /// Fill `buffer` from `offset`, reporting the bytes read and the object's
    /// total length when the answer stated it.
    ///
    /// One ranged `GET`: the range asked for is the range transferred, never
    /// the whole object. A missing object and a range past the end both read
    /// zero bytes, because absence is emptiness and so is reading past the
    /// end.
    ///
    /// # Errors
    ///
    /// Returns the store's refusal, or a read failure part way through.
    pub(super) fn get_range(
        &self,
        bucket: &str,
        key: &str,
        offset: u64,
        buffer: &mut [u8],
    ) -> Result<(usize, Option<u64>)> {
        if buffer.is_empty() {
            return Ok((0, None));
        }
        let last = offset.saturating_add(buffer.len() as u64 - 1);
        let request = Request::new("GET", "GetObject", bucket, key)
            .header("range", format!("bytes={offset}-{last}"));
        let (status, headers, mut reader) = self.stream(&request)?;
        let answer = Answer {
            status,
            headers,
            body: Vec::new(),
        };
        match status {
            // Absence is emptiness, and so is a range wholly past the end.
            404 => return Ok((0, Some(0))),
            416 => return Ok((0, total_of_content_range(answer.header("content-range")))),
            200..=299 => {}
            _ => {
                // The body is the error document, which was not read above.
                let mut body = Vec::new();
                reader.read_to_end(&mut body).ok();
                let answer = Answer {
                    status,
                    headers: answer.headers,
                    body,
                };
                return Err(self.failure(&request, &answer));
            }
        }
        let total = total_of_content_range(answer.header("content-range")).or_else(|| {
            // A 200 answers the whole object, so its length is the total.
            (status == 200)
                .then(|| {
                    answer
                        .header("content-length")
                        .and_then(|value| value.trim().parse::<u64>().ok())
                })
                .flatten()
        });
        // A store that ignored the range answered from zero, so the caller's
        // window has to be found inside what arrived.
        let skip = if status == 200 { offset } else { 0 };
        let mut discarded = 0_u64;
        let mut sink = [0_u8; 8192];
        while discarded < skip {
            let want = usize::try_from((skip - discarded).min(sink.len() as u64)).unwrap_or(0);
            let read = reader.read(&mut sink[..want]).map_err(Error::Io)?;
            if read == 0 {
                return Ok((0, total));
            }
            discarded += read as u64;
        }
        let mut filled = 0;
        while filled < buffer.len() {
            let read = reader.read(&mut buffer[filled..]).map_err(Error::Io)?;
            if read == 0 {
                break;
            }
            filled += read;
        }
        Ok((filled, total))
    }

    /// Read one object whole.
    ///
    /// One `GET`, streamed into one allocation of the answered size. A missing
    /// object reads empty.
    ///
    /// # Errors
    ///
    /// Returns the store's refusal, or a read failure part way through.
    pub(super) fn get_all(&self, bucket: &str, key: &str) -> Result<Vec<u8>> {
        let request = Request::new("GET", "GetObject", bucket, key);
        let (status, headers, mut reader) = self.stream(&request)?;
        let answer = Answer {
            status,
            headers,
            body: Vec::new(),
        };
        if status == 404 {
            return Ok(Vec::new());
        }
        if !(200..300).contains(&status) {
            let mut body = Vec::new();
            reader.read_to_end(&mut body).ok();
            let answer = Answer {
                status,
                headers: answer.headers,
                body,
            };
            return Err(self.failure(&request, &answer));
        }
        let mut bytes = Vec::new();
        if let Some(size) = answer
            .header("content-length")
            .and_then(|value| value.trim().parse::<usize>().ok())
        {
            bytes
                .try_reserve_exact(size)
                .map_err(|_| crate::iobase::oversized(size as u64))?;
        }
        reader.read_to_end(&mut bytes).map_err(Error::Io)?;
        Ok(bytes)
    }

    /// Open one object as a reader positioned at `offset`.
    ///
    /// One `GET` for the whole drain, which is what a stream is for: reading a
    /// value in chunks must not become one request per chunk. A missing object
    /// opens as empty.
    ///
    /// # Errors
    ///
    /// Returns the store's refusal.
    pub(super) fn open_reader(
        &self,
        bucket: &str,
        key: &str,
        offset: u64,
    ) -> Result<Box<dyn Read + Send>> {
        let mut request = Request::new("GET", "GetObject", bucket, key);
        if offset > 0 {
            request = request.header("range", format!("bytes={offset}-"));
        }
        let (status, headers, mut reader) = self.stream(&request)?;
        match status {
            404 | 416 => return Ok(Box::new(std::io::empty())),
            200..=299 => {}
            _ => {
                let mut body = Vec::new();
                reader.read_to_end(&mut body).ok();
                let answer = Answer {
                    status,
                    headers,
                    body,
                };
                return Err(self.failure(&request, &answer));
            }
        }
        // A store that ignored the range answered from zero.
        if status == 200 && offset > 0 {
            let mut discarded = 0_u64;
            let mut sink = [0_u8; 8192];
            while discarded < offset {
                let want =
                    usize::try_from((offset - discarded).min(sink.len() as u64)).unwrap_or(0);
                let read = reader.read(&mut sink[..want]).map_err(Error::Io)?;
                if read == 0 {
                    break;
                }
                discarded += read as u64;
            }
        }
        Ok(reader)
    }

    /// Replace one object with `bytes`.
    ///
    /// One `PUT`, and the entity tag the store answers with.
    ///
    /// # Errors
    ///
    /// Returns the store's refusal.
    pub(super) fn put_object(
        &self,
        bucket: &str,
        key: &str,
        bytes: &[u8],
        content_type: Option<&str>,
    ) -> Result<Option<String>> {
        let mut request = Request::new("PUT", "PutObject", bucket, key).body(bytes);
        if let Some(content_type) = content_type {
            request = request.header("content-type", content_type);
        }
        let answer = self.send(&request)?;
        if answer.status >= 300 {
            return Err(self.failure(&request, &answer));
        }
        Ok(answer.header("etag").map(str::to_owned))
    }

    /// Delete one object.
    ///
    /// One `DELETE`. S3 answers 204 whether or not the key was there, which is
    /// exactly the removal contract's "absence is a no-op success".
    ///
    /// # Errors
    ///
    /// Returns the store's refusal.
    pub(super) fn delete_object(&self, bucket: &str, key: &str) -> Result<()> {
        let request = Request::new("DELETE", "DeleteObject", bucket, key);
        let answer = self.send(&request)?;
        if answer.status == 404 || answer.status < 300 {
            return Ok(());
        }
        Err(self.failure(&request, &answer))
    }

    /// Delete up to [`DELETE_BATCH`] objects in one request.
    ///
    /// One `POST`, which is what keeps a recursive removal from costing one
    /// request per key.
    ///
    /// # Errors
    ///
    /// Returns the store's refusal, or the first per-key failure it reports.
    pub(super) fn delete_objects(&self, bucket: &str, keys: &[String]) -> Result<()> {
        if keys.is_empty() {
            return Ok(());
        }
        let document = xml::render_delete_objects(keys, true);
        let bytes = document.as_bytes();
        let digest = base64::engine::general_purpose::STANDARD.encode(md5_of(bytes));
        let request = Request::new("POST", "DeleteObjects", bucket, "")
            .query("delete", String::new())
            .header("content-md5", digest)
            .header("content-type", "application/xml")
            .body(bytes);
        let answer = self.send(&request)?;
        if answer.status >= 300 {
            return Err(self.failure(&request, &answer));
        }
        let failures = xml::parse_delete_result(&answer.body)
            .map_err(|error| malformed(request.operation, &self.location(&request), &error.0))?;
        match failures.first() {
            None => Ok(()),
            Some(failure) => Err(Error::remote(
                SERVICE,
                "DeleteObjects",
                answer.status,
                &failure.code,
                &failure.message,
                format!("s3://{bucket}/{}", failure.key),
            )),
        }
    }

    /// Read one page of a listing.
    ///
    /// One `GET` per page, holding at most `max_keys` entries. A delimiter
    /// rolls sub-prefixes up, which is what makes one level of a tree one
    /// request rather than a walk.
    ///
    /// # Errors
    ///
    /// Returns the store's refusal or a malformed page.
    pub(super) fn list_objects(
        &self,
        bucket: &str,
        prefix: &str,
        delimiter: Option<&str>,
        continuation: Option<&str>,
        max_keys: u16,
    ) -> Result<xml::ListPage> {
        let mut request = Request::new("GET", "ListObjectsV2", bucket, "")
            .query("list-type", "2")
            .query("max-keys", max_keys.to_string())
            // Keys are arbitrary UTF-8, and a control character would make the
            // page malformed XML; asking for URL encoding keeps every key
            // readable and the document well-formed.
            .query("encoding-type", "url");
        if !prefix.is_empty() {
            request = request.query("prefix", prefix);
        }
        if let Some(delimiter) = delimiter {
            request = request.query("delimiter", delimiter);
        }
        if let Some(token) = continuation {
            request = request.query("continuation-token", token);
        }
        self.stats.lists.fetch_add(1, Ordering::Relaxed);
        let answer = self.send(&request)?;
        if answer.status >= 300 {
            return Err(self.failure(&request, &answer));
        }
        xml::parse_list_objects(&answer.body)
            .map_err(|error| malformed(request.operation, &self.location(&request), &error.0))
    }

    /// Begin a multipart upload, answering its identifier.
    ///
    /// # Errors
    ///
    /// Returns the store's refusal.
    pub(super) fn create_multipart(
        &self,
        bucket: &str,
        key: &str,
        content_type: Option<&str>,
    ) -> Result<String> {
        let mut request = Request::new("POST", "CreateMultipartUpload", bucket, key)
            .query("uploads", String::new());
        if let Some(content_type) = content_type {
            request = request.header("content-type", content_type);
        }
        let answer = self.send(&request)?;
        if answer.status >= 300 {
            return Err(self.failure(&request, &answer));
        }
        xml::parse_upload_id(&answer.body)
            .map_err(|error| malformed(request.operation, &self.location(&request), &error.0))
    }

    /// Upload one part, answering its entity tag.
    ///
    /// # Errors
    ///
    /// Returns the store's refusal, or a part the store accepted without a tag.
    pub(super) fn upload_part(
        &self,
        bucket: &str,
        key: &str,
        upload: &str,
        part: u32,
        bytes: &[u8],
    ) -> Result<String> {
        let request = Request::new("PUT", "UploadPart", bucket, key)
            .query("partNumber", part.to_string())
            .query("uploadId", upload)
            .body(bytes);
        let answer = self.send(&request)?;
        if answer.status >= 300 {
            return Err(self.failure(&request, &answer));
        }
        answer.header("etag").map(str::to_owned).ok_or_else(|| {
            malformed(
                request.operation,
                &self.location(&request),
                "the store accepted a part without an ETag",
            )
        })
    }

    /// Complete a multipart upload from the parts it accepted.
    ///
    /// # Errors
    ///
    /// Returns the store's refusal, including the late failure it can report
    /// inside a 200 answer.
    pub(super) fn complete_multipart(
        &self,
        bucket: &str,
        key: &str,
        upload: &str,
        parts: &[(u32, String)],
    ) -> Result<Option<String>> {
        let document = xml::render_complete_multipart(parts);
        let request = Request::new("POST", "CompleteMultipartUpload", bucket, key)
            .query("uploadId", upload)
            .header("content-type", "application/xml")
            .body(document.as_bytes());
        let answer = self.send(&request)?;
        if answer.status >= 300 {
            return Err(self.failure(&request, &answer));
        }
        xml::parse_complete_multipart(&answer.body).map_err(|error| {
            Error::remote(
                SERVICE,
                "CompleteMultipartUpload",
                answer.status,
                "InternalError",
                &error.0,
                self.location(&request),
            )
        })
    }

    /// Abandon a multipart upload, releasing the parts it holds.
    ///
    /// # Errors
    ///
    /// Returns the store's refusal; an upload already gone is success.
    pub(super) fn abort_multipart(&self, bucket: &str, key: &str, upload: &str) -> Result<()> {
        let request =
            Request::new("DELETE", "AbortMultipartUpload", bucket, key).query("uploadId", upload);
        let answer = self.send(&request)?;
        if answer.status == 404 || answer.status < 300 {
            return Ok(());
        }
        Err(self.failure(&request, &answer))
    }

    /// Return whether the bucket is there.
    ///
    /// # Errors
    ///
    /// Returns the store's refusal for anything that is not a 404.
    pub(super) fn head_bucket(&self, bucket: &str) -> Result<bool> {
        let request = Request::new("HEAD", "HeadBucket", bucket, "");
        let answer = self.send(&request)?;
        if answer.status == 404 {
            return Ok(false);
        }
        if answer.status >= 300 {
            return Err(self.failure(&request, &answer));
        }
        Ok(true)
    }

    /// Create the bucket, absorbing the answer that says it is already ours.
    ///
    /// # Errors
    ///
    /// Returns the store's refusal.
    pub(super) fn create_bucket(&self, bucket: &str) -> Result<()> {
        let region = self.region();
        // `us-east-1` is the only region a location constraint must not name.
        let document = if region == DEFAULT_REGION {
            String::new()
        } else {
            xml::render_create_bucket(&region)
        };
        let request = Request::new("PUT", "CreateBucket", bucket, "").body(document.as_bytes());
        let answer = self.send(&request)?;
        // Owning it already is what a repeated create means, not a conflict.
        if answer.status < 300 {
            return Ok(());
        }
        let code = xml::parse_error(&answer.body).map(|error| error.code);
        if code.as_deref() == Some("BucketAlreadyOwnedByYou") {
            return Ok(());
        }
        Err(self.failure(&request, &answer))
    }

    /// Delete the bucket itself.
    ///
    /// # Errors
    ///
    /// Returns the store's refusal, including the one naming it non-empty.
    pub(super) fn delete_bucket(&self, bucket: &str) -> Result<()> {
        let request = Request::new("DELETE", "DeleteBucket", bucket, "");
        let answer = self.send(&request)?;
        if answer.status == 404 || answer.status < 300 {
            return Ok(());
        }
        Err(self.failure(&request, &answer))
    }
}

/// The largest document read into memory from a non-object answer.
///
/// A listing page of a thousand keys is tens of kilobytes; this bound exists so
/// a store answering something unexpected cannot make the client hold it.
const MAX_DOCUMENT: u64 = 32 * 1024 * 1024;

/// The process-wide connection pool, shared by every default-configured client.
fn shared_agent() -> &'static ureq::Agent {
    static AGENT: std::sync::OnceLock<ureq::Agent> = std::sync::OnceLock::new();
    AGENT.get_or_init(|| build_agent(&S3Options::default()))
}

/// Build an agent for `options`.
fn build_agent(options: &S3Options) -> ureq::Agent {
    ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            // Statuses are read, never raised: S3 says what it means in the
            // status and an XML body, and this client maps both itself.
            .http_status_as_error(false)
            .timeout_global(Some(options.timeout()))
            .timeout_connect(Some(options.connect_timeout()))
            .user_agent(concat!("yggdryl/", env!("CARGO_PKG_VERSION")))
            .build(),
    )
}

/// The delay before attempt `attempt + 1`, doubling and capped.
fn backoff(attempt: u32) -> Duration {
    let steps = attempt.saturating_sub(1).min(6);
    RETRY_BACKOFF.saturating_mul(1_u32 << steps)
}

/// Whether a transport failure is worth another attempt.
///
/// A connection that never established, timed out, or was cut is; a request
/// the client itself could not form is not.
fn is_retryable_transport(error: &ureq::Error) -> bool {
    matches!(
        error,
        ureq::Error::Io(_)
            | ureq::Error::Timeout(_)
            | ureq::Error::ConnectionFailed
            | ureq::Error::HostNotFound
    )
}

/// The region a redirect names, when it names one.
fn bucket_region_of(answer: &Answer) -> Option<String> {
    if !matches!(answer.status, 301 | 307 | 400 | 403) {
        return None;
    }
    answer
        .header("x-amz-bucket-region")
        .map(str::to_owned)
        .or_else(|| {
            xml::parse_error(&answer.body)
                .filter(|error| error.code == "AuthorizationHeaderMalformed")
                .and_then(|error| {
                    // The message names the region when the header does not.
                    let start = error.message.find("'")? + 1;
                    let rest = error.message.get(start..)?;
                    let end = rest.find('\'')?;
                    Some(rest[..end].to_owned())
                })
        })
        .filter(|region| !region.is_empty())
}

/// The total object length a `Content-Range` states.
fn total_of_content_range(header: Option<&str>) -> Option<u64> {
    let total = header?.rsplit_once('/')?.1.trim();
    total.parse().ok()
}

/// What an operation was addressing, for a typed absence or conflict.
fn absent_kind(request: &Request<'_>) -> &'static str {
    if request.key.is_empty() {
        "bucket"
    } else {
        "object"
    }
}

/// A stable name for a status with no error document behind it.
fn status_code_name(status: u16) -> String {
    match status {
        400 => "BadRequest",
        401 => "Unauthorized",
        403 => "AccessDenied",
        405 => "MethodNotAllowed",
        429 => "SlowDown",
        500 => "InternalError",
        501 => "NotImplemented",
        503 => "ServiceUnavailable",
        _ => "Unknown",
    }
    .to_owned()
}

/// A store answered something this client cannot read.
fn malformed(operation: &'static str, path: &str, reason: &str) -> Error {
    Error::remote(SERVICE, operation, 200, "MalformedResponse", reason, path)
}

/// A request that never reached the store, or whose connection failed.
fn transport_failure(request: &Request<'_>, path: String, error: ureq::Error) -> Error {
    Error::Io(std::io::Error::other(format!(
        "{SERVICE} {} at {path:?} failed: {error}",
        request.operation
    )))
}

/// MD5 of `bytes`, which `DeleteObjects` still requires as `Content-MD5`.
fn md5_of(bytes: &[u8]) -> [u8; 16] {
    use md5::Digest as _;
    md5::Md5::digest(bytes).into()
}

/// Report a poisoned client lock without panicking a caller.
fn poisoned() -> Error {
    Error::Io(std::io::Error::other(
        "an S3 client lock was poisoned by a panicking writer",
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        Answer, Client, DEFAULT_REGION, Endpoint, backoff, bucket_region_of, total_of_content_range,
    };
    use crate::Url;
    use crate::holder::s3::S3Options;

    fn url(text: &str) -> Url {
        Url::from_str(text).expect("a valid location")
    }

    /// Options that consult nothing outside the test.
    fn sealed() -> S3Options {
        S3Options::default().with_environment(false)
    }

    #[test]
    fn an_aws_location_is_addressed_virtual_hosted_and_signed_for_its_region() {
        let client = Client::new(
            &url("s3://trades.s3.eu-west-3.amazonaws.com/lake/part.parquet"),
            sealed(),
        )
        .expect("a client");
        assert_eq!(client.region(), "eu-west-3");
        assert_eq!(
            client.endpoint.host_header("trades"),
            "trades.s3.eu-west-3.amazonaws.com"
        );
        assert_eq!(
            client.endpoint.path("trades", "lake/part.parquet"),
            "/lake/part.parquet"
        );
    }

    #[test]
    fn a_bucket_only_location_defaults_its_endpoint_from_the_region() {
        let client =
            Client::new(&url("s3://trades/lake/part.parquet"), sealed()).expect("a client");
        assert_eq!(client.region(), DEFAULT_REGION);
        assert_eq!(
            client.endpoint.host_header("trades"),
            "trades.s3.us-east-1.amazonaws.com"
        );

        // A dot in the bucket would break a TLS wildcard, so it stays in the path.
        let dotted = Client::new(&url("s3://my.trades/part.parquet"), sealed()).expect("a client");
        assert_eq!(
            dotted.endpoint.host_header("my.trades"),
            "s3.us-east-1.amazonaws.com"
        );
        assert_eq!(
            dotted.endpoint.path("my.trades", "part.parquet"),
            "/my.trades/part.parquet"
        );
    }

    #[test]
    fn a_local_endpoint_is_addressed_path_style_over_its_own_scheme_and_port() {
        let client = Client::new(
            &url("s3://localhost:9000/trades/lake/part.parquet"),
            sealed().with_endpoint("http://localhost:9000"),
        )
        .expect("a client");
        assert_eq!(client.endpoint.scheme, "http");
        assert_eq!(client.endpoint.host_header("trades"), "localhost:9000");
        assert_eq!(
            client.endpoint.path("trades", "lake/part.parquet"),
            "/trades/lake/part.parquet"
        );
    }

    #[test]
    fn a_key_is_encoded_once_and_its_separators_survive() {
        let endpoint = Endpoint {
            scheme: "https".to_owned(),
            host: "s3.example.io".to_owned(),
            port: None,
            path_style: true,
        };
        assert_eq!(
            endpoint.path("trades", "year=2026/a b/c+d.parquet"),
            "/trades/year%3D2026/a%20b/c%2Bd.parquet"
        );
        // The bucket root has no trailing separator to sign over.
        assert_eq!(endpoint.path("trades", ""), "/trades");
    }

    #[test]
    fn explicit_credentials_beat_a_url_that_carries_its_own() {
        let carried = Client::url_credentials(&url("s3://key:s3cr3t@trades/part.parquet"))
            .expect("credentials off the URL");
        assert_eq!(carried.access_key_id(), "key");

        let explicit = Client::new(
            &url("s3://key:s3cr3t@trades/part.parquet"),
            sealed().with_credentials(crate::holder::s3::Credentials::new("other", "secret")),
        )
        .expect("a client");
        let signer = explicit
            .signer(std::time::SystemTime::UNIX_EPOCH)
            .expect("a signer")
            .expect("credentials");
        assert_eq!(signer.access_key_id(), "other");
    }

    #[test]
    fn a_content_range_states_the_total_and_a_redirect_states_the_region() {
        assert_eq!(total_of_content_range(Some("bytes 0-9/1024")), Some(1024));
        assert_eq!(total_of_content_range(Some("bytes */1024")), Some(1024));
        assert_eq!(total_of_content_range(Some("bytes 0-9/*")), None);
        assert_eq!(total_of_content_range(None), None);

        let redirect = Answer {
            status: 301,
            headers: vec![("x-amz-bucket-region".to_owned(), "eu-west-3".to_owned())],
            body: Vec::new(),
        };
        assert_eq!(bucket_region_of(&redirect), Some("eu-west-3".to_owned()));

        // A 200 is never a redirect, whatever headers it carries.
        let fine = Answer {
            status: 200,
            headers: vec![("x-amz-bucket-region".to_owned(), "eu-west-3".to_owned())],
            body: Vec::new(),
        };
        assert_eq!(bucket_region_of(&fine), None);
    }

    #[test]
    fn the_backoff_doubles_and_stops_doubling() {
        assert_eq!(backoff(1), super::RETRY_BACKOFF);
        assert_eq!(backoff(2), super::RETRY_BACKOFF * 2);
        assert_eq!(backoff(3), super::RETRY_BACKOFF * 4);
        assert_eq!(backoff(20), super::RETRY_BACKOFF * 64);
    }

    #[test]
    fn an_endpoint_splits_into_scheme_host_and_port() {
        assert_eq!(
            Client::split_endpoint("http://localhost:9000").expect("a split endpoint"),
            ("http".to_owned(), "localhost".to_owned(), Some(9000))
        );
        assert_eq!(
            Client::split_endpoint("s3.example.io").expect("a split endpoint"),
            ("https".to_owned(), "s3.example.io".to_owned(), None)
        );
        assert_eq!(
            Client::split_endpoint("https://[::1]:9000").expect("a split endpoint"),
            ("https".to_owned(), "[::1]".to_owned(), Some(9000))
        );
        Client::split_endpoint("https://host:notaport").expect_err("a refused port");
    }
}
