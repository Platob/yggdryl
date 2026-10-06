//! The client, and the one door every request leaves through.
//!
//! [`S3Tables`] holds who signs and where the service is; the verbs over
//! table buckets, namespaces and tables live beside the values they answer,
//! and each of them hands [`S3Tables::send`] one [`Call`]. What a request
//! needs that is not its own - the region, the endpoint, the attempts, the
//! reading of a refusal - is decided here; how it is signed, and when a
//! refused key earns a second send, is [`Request::with_sigv4`]'s.
//!
//! [`Request::with_sigv4`]: crate::http::Request::with_sigv4

use std::sync::Arc;
use std::time::Duration;

use smol_str::format_smolstr;

use crate::aws::credentials::Refusal;
use crate::aws::sigv4::{canonical_query, encode_query_component};
use crate::aws::{Answer, Session};
use crate::http::{Headers, Method};
use crate::{Arn, ArnPartition, DateTime64, Error, Result, Scalar, Timezone, Url};

/// The service's name: in every credential scope, in every endpoint host,
/// and in every refusal.
pub(crate) const SERVICE: &str = "s3tables";

/// How long each phase of one request may take, botocore's own read bound.
const TIMEOUT: Duration = Duration::from_secs(60);

/// How many times a request the service marks safe to send again is
/// attempted when neither the environment nor the profile says: the AWS
/// tools' standard retry mode.
const ATTEMPTS: u32 = 3;

/// The longest text a refusal repeats, so a caller's mistake is quoted and a
/// megabyte of it is not.
const ECHO_BYTES: usize = 64;

/// The most of the service's own message a refusal repeats: with the
/// endpoint, the region, where it came from and the opt-in note after it,
/// the whole stays under the 512 bytes `Error::remote` keeps.
const MESSAGE_BYTES: usize = 160;

/// The most of the endpoint a refusal names after the service's message.
const ENDPOINT_BYTES: usize = 96;

/// The error types botocore's standard retry mode reads as throttling,
/// whatever the status they come under.
const THROTTLED: [&str; 14] = [
    "Throttling",
    "ThrottlingException",
    "ThrottledException",
    "RequestThrottledException",
    "TooManyRequestsException",
    "ProvisionedThroughputExceededException",
    "TransactionInProgressException",
    "RequestLimitExceeded",
    "BandwidthLimitExceeded",
    "LimitExceededException",
    "RequestThrottled",
    "SlowDown",
    "PriorRequestNotComplete",
    "EC2ThrottledException",
];

/// A client of the Amazon S3 Tables control plane.
///
/// It holds a [`Session`] - who signs - and the two things a caller may
/// state over it: the region and the endpoint. Building one reads nothing
/// and reaches nothing; a clone shares the session and what it has resolved.
///
/// ```
/// use yggdryl::Arn;
/// use yggdryl::aws::{Credentials, Session};
/// use yggdryl::s3tables::S3Tables;
///
/// # fn main() -> yggdryl::Result<()> {
/// // A session that states everything consults nothing outside itself.
/// let session = Session::new()
///     .with_environment(false)
///     .with_credentials(Credentials::new("AKIAIOSFODNN7EXAMPLE", "a-secret"))
///     .with_region("eu-west-3");
/// let tables = S3Tables::new(session);
///
/// // A table bucket is addressed where its ARN says it is.
/// let lake = Arn::from_str("arn:aws:s3tables:us-east-1:123456789012:bucket/lake")?;
/// assert_eq!(tables.region_of(Some(&lake))?, "us-east-1");
/// assert_eq!(tables.region_of(None)?, "eu-west-3");
/// assert_eq!(
///     tables.endpoint_url("us-east-1")?,
///     "https://s3tables.us-east-1.amazonaws.com"
/// );
///
/// // What the caller states wins over both.
/// let local = tables
///     .with_region("us-west-2")
///     .try_with_endpoint_url("http://localhost:9000/")?;
/// assert_eq!(local.region_of(Some(&lake))?, "us-west-2");
/// assert_eq!(local.endpoint_url("us-west-2")?, "http://localhost:9000");
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct S3Tables {
    session: Session,
    /// The region the caller stated, over the ARN's and the session's.
    region: Option<String>,
    /// The endpoint the caller stated, over the session's and the
    /// partition's host, read once: it holds no user information.
    endpoint: Option<Endpoint>,
}

impl S3Tables {
    /// A client that signs as `session` answers, in the region and at the
    /// endpoint it resolves.
    pub fn new(session: Session) -> Self {
        Self {
            session,
            region: None,
            endpoint: None,
        }
    }

    /// Sign for, and address, `region`, whatever a table bucket's ARN or the
    /// session says; a blank region states none.
    ///
    /// The region names the host a request is sent to, so a region that is
    /// not a host label is refused when a request resolves it
    /// ([`Self::region_of`]).
    #[must_use]
    pub fn with_region(mut self, region: impl Into<String>) -> Self {
        let region: String = region.into();
        self.region = (!region.trim().is_empty()).then(|| region.trim().to_owned());
        self
    }

    /// Reach the service at `url`, whatever the session or the region says;
    /// a blank URL states none.
    ///
    /// The URL is read here, once: an `http` or `https` origin, and the path
    /// a gateway mounts the service under. Every request goes there whatever
    /// the session's FIPS and dual-stack switches say, as botocore sends one
    /// given an endpoint.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] for a URL that is not `http` or `https`,
    /// names no host, carries a query or a fragment, or carries user
    /// information - a request is signed with the session's credentials, so
    /// a pair written into the endpoint would be read by nothing. A refusal
    /// never repeats the user information it found.
    pub fn try_with_endpoint_url(mut self, url: impl AsRef<str>) -> Result<Self> {
        let url = url.as_ref().trim();
        self.endpoint = if url.is_empty() {
            None
        } else {
            Some(Endpoint::from_url(url)?)
        };
        Ok(self)
    }

    /// The session every request signs with.
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// The region a request is signed for and sent to: what was stated on
    /// the client, else the region of the table bucket `bucket` names, else
    /// the session's.
    ///
    /// # Errors
    ///
    /// Returns a refusal naming every way to state a region when none of
    /// the three answers, and [`Error::Parse`] targeting `region` and naming
    /// where the region came from - `S3Tables::with_region`, the table
    /// bucket's ARN, the session - when it is not a host label: 1 to 63 of
    /// the ASCII letters, the digits and `-`, not at either end and not
    /// digits alone, the one rule every AWS host is built under, because
    /// the region names the host the signed request is sent to.
    pub fn region_of(&self, bucket: Option<&Arn>) -> Result<String> {
        self.located_region(bucket).map(|(region, _)| region)
    }

    /// [`Self::region_of`], with where the region came from, for a refusal
    /// to name.
    fn located_region(&self, bucket: Option<&Arn>) -> Result<(String, &'static str)> {
        let (region, source) = if let Some(region) = &self.region {
            (region.clone(), "S3Tables::with_region")
        } else if let Some(region) = bucket.and_then(Arn::region) {
            (region.to_owned(), "the table bucket's ARN")
        } else if let Some(region) = self.session.region() {
            (region, "the session")
        } else {
            return Err(invalid_input(
                "expected a region for Amazon S3 Tables, got none: address a table bucket by its \
                 ARN, state one with S3Tables::with_region or Session::with_region, set \
                 AWS_REGION, or name one in the profile"
                    .to_owned(),
            ));
        };
        ArnPartition::check_region(&region, source)?;
        Ok((region, source))
    }

    /// The endpoint requests for `region` go to: what was stated on the
    /// client, else [`Session::service_endpoint`] for `s3tables` - the
    /// endpoint the session states or configures for the service
    /// (`AWS_ENDPOINT_URL_S3TABLES`, `AWS_ENDPOINT_URL`, the profile's
    /// `[services]` entry `s3tables`, its `endpoint_url`), else the published
    /// host of the region's partition, `https://s3tables[-fips].{region}.{suffix}`
    /// with the dual-stack suffix as the session's switches say.
    ///
    /// A stated or configured endpoint is used whatever the FIPS and
    /// dual-stack switches say: they choose among the published hosts, and
    /// botocore turns both off when an endpoint is given.
    ///
    /// # Errors
    ///
    /// What the session refuses of the configured endpoint
    /// ([`Session::endpoint_url`]), and a configured endpoint the client
    /// cannot send to, refused as [`Self::try_with_endpoint_url`] refuses
    /// one.
    pub fn endpoint_url(&self, region: &str) -> Result<String> {
        self.endpoint(region).map(|endpoint| endpoint.0)
    }

    /// [`Self::endpoint_url`], read.
    fn endpoint(&self, region: &str) -> Result<Endpoint> {
        match &self.endpoint {
            Some(endpoint) => Ok(endpoint.clone()),
            None => Endpoint::from_url(&self.session.service_endpoint(SERVICE, region)?),
        }
    }

    /// Send `call` and read its answer: one request, signed with Signature
    /// Version 4 for `s3tables` as the session answers, plus the ones the
    /// HTTP client repeats for a call the service marks safe to send again,
    /// plus one more when the service says the key that signed it is no
    /// longer accepted and the session then answers another.
    ///
    /// # Errors
    ///
    /// Returns the region's or the endpoint's refusal, and the session's
    /// when it answers no credential set, each naming the operation;
    /// [`Error::Absent`] or [`Error::Conflict`] where the call says the
    /// service's `NotFoundException` or `ConflictException` means one;
    /// [`Error::Remote`] for every other refusal of the service, its
    /// message the service's own followed by the endpoint the request went
    /// to and the region it was signed for with where that region came from
    /// ([`Call::refusal`]); and [`Error::Io`] when nothing answered.
    pub(crate) fn send(&self, call: &Call<'_>) -> Result<Reply> {
        let (region, source) = self
            .located_region(call.bucket)
            .map_err(|error| call.located(error))?;
        let endpoint = self
            .endpoint(&region)
            .map_err(|error| call.located(error))?;
        let mut request = self
            .session
            .http()?
            .request(call.method, &endpoint.url_of(&call.path, &call.query))?
            .with_timeout(TIMEOUT)
            .with_max_attempts(self.session.max_attempts().unwrap_or(ATTEMPTS))
            .with_idempotent(call.repeatable)
            .with_sigv4(&self.session, SERVICE, region.as_str());
        if call.repeatable {
            request = request.with_retry_on(|_, headers, body| is_throttled(headers, body));
        }
        if let Some(body) = &call.body {
            request = request
                .with_header("content-type", "application/json")?
                .with_body(Arc::clone(body));
        }
        let answer = Answer::of(&request).map_err(|error| call.unanswered(&endpoint, error))?;
        if (200..300).contains(&answer.status) {
            return Reply::read(call, &answer);
        }
        Err(call.refusal(&Refused::read(&answer), &endpoint, &region, source))
    }
}

/// Where requests go: `scheme://host[:port]`, then the path a gateway
/// mounts the service under, without a trailing slash - read once, holding
/// no user information, so nothing that renders it can leak a credential.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Endpoint(String);

impl Endpoint {
    /// Read an endpoint URL.
    fn from_url(text: &str) -> Result<Self> {
        let text = text.trim().trim_end_matches('/');
        // An endpoint may be written with credentials in it; a refusal
        // masks them before it cuts the text short, so no part of one is
        // ever repeated.
        let refuse = |position: usize, expected: &str| Error::Parse {
            target: "s3tables endpoint",
            position,
            reason: format_smolstr!(
                "expected {expected}, got {:?}",
                echo(&crate::fs::mask_uri(text))
            ),
        };
        let parsed = Url::from_str(text).map_err(|error| {
            let position = match error {
                Error::Parse { position, .. } => position,
                _ => 0,
            };
            refuse(position, "an http or https URL")
        })?;
        let scheme = parsed.scheme().as_str().to_ascii_lowercase();
        if !matches!(scheme.as_str(), "http" | "https") {
            return Err(refuse(0, "an http or https URL"));
        }
        let authority = parsed.authority();
        let at = scheme.len() + "://".len();
        if authority.user().is_some() {
            return Err(refuse(
                at,
                "an endpoint with no user information: a request is signed with the session's \
                 credentials",
            ));
        }
        if authority.host().is_empty() {
            return Err(refuse(at, "an http or https URL naming a host"));
        }
        if parsed.query(false)?.is_some() || parsed.fragment(false)?.is_some() {
            return Err(refuse(
                text.find(['?', '#']).unwrap_or(0),
                "an endpoint with no query and no fragment",
            ));
        }
        let prefix = parsed.path_text(false)?;
        Ok(Self(format!(
            "{scheme}://{}{}",
            authority.host_port(),
            prefix.trim_end_matches('/')
        )))
    }

    /// The URL of `path` - its labels already encoded once - with `query`,
    /// raw pairs encoded here as the signature reads them back.
    fn url_of(&self, path: &str, query: &[(String, String)]) -> String {
        let query = canonical_query(query);
        if query.is_empty() {
            format!("{}{path}", self.0)
        } else {
            format!("{}{path}?{query}", self.0)
        }
    }
}

/// Whether a refusal is the service throttling the caller, by the error type
/// it states rather than its status: botocore's standard retry mode retries
/// a `400 ThrottlingException` as it does a `429`.
fn is_throttled(headers: &Headers, body: &[u8]) -> bool {
    crate::aws::error_code(headers, body).is_some_and(|code| THROTTLED.contains(&shape_name(&code)))
}

/// One request of the service: the operation the model names, the wire
/// path and query that address it, and what its refusals mean.
pub(crate) struct Call<'a> {
    /// The operation as the model names it, for a refusal.
    operation: &'static str,
    method: Method,
    /// Whether a second send can do no harm: a read, or an operation the
    /// model marks idempotent. A `PUT` that creates, renames or commits is
    /// not, so the HTTP client never repeats it once the service may have
    /// acted on it.
    repeatable: bool,
    /// The path as sent: every label percent-encoded once.
    path: String,
    /// Query pairs, raw; the wire and the signature both spell them encoded.
    query: Vec<(String, String)>,
    /// The JSON document a `PUT` carries.
    body: Option<Arc<[u8]>>,
    /// The table bucket addressed, whose ARN names the region.
    bucket: Option<&'a Arn>,
    /// The ARN or the name a refusal reports.
    addressed: String,
    /// What the service's `NotFoundException` says is not there, for a call
    /// that reads or deletes one thing.
    absent: Option<&'static str>,
    /// What the service's `ConflictException` says is already there, for a
    /// call that creates one thing.
    conflict: Option<&'static str>,
}

impl<'a> Call<'a> {
    /// A read: `GET`, sent again when a `5xx`, a throttle or the transport
    /// fails it.
    pub(crate) fn get(operation: &'static str, path: String, addressed: String) -> Self {
        Self::new(operation, Method::Get, true, path, addressed)
    }

    /// A `PUT` that creates, renames or commits: sent once.
    pub(crate) fn put(
        operation: &'static str,
        path: String,
        addressed: String,
        body: Vec<u8>,
    ) -> Self {
        let mut call = Self::new(operation, Method::Put, false, path, addressed);
        call.body = Some(Arc::from(body));
        call
    }

    /// A `DELETE`, which the model marks idempotent: sent again like a read.
    pub(crate) fn delete(operation: &'static str, path: String, addressed: String) -> Self {
        Self::new(operation, Method::Delete, true, path, addressed)
    }

    fn new(
        operation: &'static str,
        method: Method,
        repeatable: bool,
        path: String,
        addressed: String,
    ) -> Self {
        Self {
            operation,
            method,
            repeatable,
            path,
            query: Vec::new(),
            body: None,
            bucket: None,
            addressed,
            absent: None,
            conflict: None,
        }
    }

    /// Address the table bucket `bucket`, whose region the request is
    /// signed for.
    pub(crate) fn in_bucket(mut self, bucket: &'a Arn) -> Self {
        self.bucket = Some(bucket);
        self
    }

    /// Carry one query pair, raw.
    pub(crate) fn with_query(mut self, name: &str, value: &str) -> Self {
        self.query.push((name.to_owned(), value.to_owned()));
        self
    }

    /// Read the service's `NotFoundException` as the absence of one `kind`.
    pub(crate) fn absent_as(mut self, kind: &'static str) -> Self {
        self.absent = Some(kind);
        self
    }

    /// Read the service's `ConflictException` as one `kind` already being
    /// there.
    pub(crate) fn conflict_as(mut self, kind: &'static str) -> Self {
        self.conflict = Some(kind);
        self
    }

    /// A failure met before or while sending, named by the operation and
    /// what it addressed: a region, an endpoint or a credential set the
    /// request could not be sent without. A typed refusal keeps its type.
    fn located(&self, error: Error) -> Error {
        match error {
            Error::Io(error) => Error::Io(std::io::Error::new(
                error.kind(),
                format!(
                    "{SERVICE} {} at {:?}: {error}",
                    self.operation,
                    echo(&self.addressed)
                ),
            )),
            other => other,
        }
    }

    /// A request nothing answered, named by what it was for and where it
    /// went; any other failure of the send - the session's refusal to sign
    /// - is [`Self::located`].
    fn unanswered(&self, endpoint: &Endpoint, error: Error) -> Error {
        if !crate::http::is_unanswered(&error) {
            return self.located(error);
        }
        match error {
            Error::Io(error) => Error::Io(std::io::Error::new(
                error.kind(),
                format!(
                    "{SERVICE} {} at {:?} got no answer from {}: {error}",
                    self.operation,
                    echo(&self.addressed),
                    endpoint.0
                ),
            )),
            other => other,
        }
    }

    /// The typed failure a refusing answer means.
    ///
    /// The error type decides, never the status alone: a `404` that is not
    /// the service's own `NotFoundException` - a gateway's, a wrong
    /// endpoint's - says nothing about what the service holds, and reading
    /// it as an absence would make a delete that reached no service a
    /// success.
    ///
    /// Any other refusal is [`Error::Remote`] at what the call addressed,
    /// its code the service's, its message the first line of the service's
    /// own - at most [`MESSAGE_BYTES`] of it - then the `endpoint` the
    /// request was sent to, the `region` it was signed for and the `source`
    /// that region came from. A code that refuses the key that signed
    /// (`UnrecognizedClientException`, `InvalidClientTokenId`) in an opt-in
    /// region adds that AWS answers it for every key there until the
    /// account enables the region, since the key may be sound.
    fn refusal(&self, refused: &Refused, endpoint: &Endpoint, region: &str, source: &str) -> Error {
        match (refused.error_type.as_deref(), self.absent, self.conflict) {
            (Some("NotFoundException"), Some(kind), _) => Error::absent(kind, &self.addressed),
            (Some("ConflictException"), _, Some(kind)) => {
                Error::conflict(kind, kind, &self.addressed)
            }
            (error_type, _, _) => {
                let said = refused.message.lines().next().unwrap_or_default();
                let sent = format!(
                    "{} (sent to {}, signed for the region {region} {source} states)",
                    clip(said, MESSAGE_BYTES),
                    clip(&endpoint.0, ENDPOINT_BYTES)
                );
                let refuses_key = matches!(
                    error_type.and_then(Refusal::from_code),
                    Some(Refusal::Unrecognized)
                );
                let message = if refuses_key && ArnPartition::is_opt_in(region) {
                    format!(
                        "{sent}; {region} is an opt-in region: AWS refuses every key there with \
                         this code until the account enables the region, so check the region \
                         before the key"
                    )
                } else {
                    sent
                };
                Error::remote(
                    SERVICE,
                    self.operation,
                    refused.status,
                    error_type.unwrap_or_else(|| status_code_name(refused.status)),
                    message,
                    &self.addressed,
                )
            }
        }
    }

    /// Report an answer this client cannot read.
    fn malformed(&self, status: u16, reason: impl AsRef<str>) -> Error {
        Error::remote(
            SERVICE,
            self.operation,
            status,
            "MalformedAnswer",
            reason,
            &self.addressed,
        )
    }
}

/// One path label as the wire spells it: percent-encoded once, the `:` and
/// the `/` of an ARN included.
pub(crate) fn label(text: &str) -> String {
    encode_query_component(text)
}

/// A refusing answer, read once: its status, the error type it states and
/// its message.
struct Refused {
    status: u16,
    /// The error's own name, or `None` when the answer states no type.
    error_type: Option<String>,
    /// The message the body states, or the status when it states none.
    message: String,
}

impl Refused {
    /// Read a refusal the way the REST-JSON protocol spells one: the error
    /// type in the `x-amzn-ErrorType` header, else in the body's `code`,
    /// else in its `__type`; the message in the body's `message`.
    fn read(answer: &Answer) -> Self {
        let document = crate::json::from_bytes(&answer.body).ok();
        let member = |keys: &[&str]| {
            let document = document.as_ref()?;
            keys.iter()
                .find_map(|key| document.get_key_str(key).and_then(Scalar::as_str))
        };
        let error_type = answer
            .error_type
            .as_deref()
            .or_else(|| member(&["code", "Code"]))
            .or_else(|| member(&["__type"]))
            .map(shape_name)
            .filter(|name| !name.is_empty())
            .map(str::to_owned);
        let message = member(&["message", "Message"]).map_or_else(
            || format!("the service answered {}", answer.status),
            str::to_owned,
        );
        Self {
            status: answer.status,
            error_type,
            message,
        }
    }
}

/// An error type's own name: what precedes the `:` a service qualifies it
/// after, and what follows the `#` of a namespace written before it.
fn shape_name(stated: &str) -> &str {
    let name = stated.split(':').next().unwrap_or(stated);
    name.rsplit('#').next().unwrap_or(name).trim()
}

/// A stable name for a status with no error type behind it - a gateway's
/// answer, a proxy's: the status's own name, never one of the service's
/// `...Exception` types, which only the service can say.
fn status_code_name(status: u16) -> &'static str {
    match status {
        400 => "BadRequest",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "NotFound",
        409 => "Conflict",
        429 => "TooManyRequests",
        500 => "InternalServerError",
        502 => "BadGateway",
        503 => "ServiceUnavailable",
        504 => "GatewayTimeout",
        _ => "Unknown",
    }
}

/// A refusal of what the caller stated, before anything is sent.
pub(crate) fn invalid_input(message: String) -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        message,
    ))
}

/// One successful answer: its JSON document, and the call a field it lacks
/// is reported against.
pub(crate) struct Reply {
    document: Scalar,
    status: u16,
    operation: &'static str,
    addressed: String,
}

impl Reply {
    /// Read a `2xx` answer: its document, or nothing for a body-less one.
    fn read(call: &Call<'_>, answer: &Answer) -> Result<Self> {
        let document = if answer.body.is_empty() {
            Scalar::Null
        } else {
            crate::json::from_bytes(&answer.body).map_err(|error| {
                call.malformed(
                    answer.status,
                    format!("expected a JSON document, got {error}"),
                )
            })?
        };
        Ok(Self {
            document,
            status: answer.status,
            operation: call.operation,
            addressed: call.addressed.clone(),
        })
    }

    /// The answer's document, to be read by the names the model states.
    pub(crate) fn reader(&self) -> Reader<'_> {
        Reader {
            document: &self.document,
            reply: self,
        }
    }
}

/// One JSON object of an answer - the whole document, or one entry of a
/// listing - read by the names the model states.
#[derive(Clone, Copy)]
pub(crate) struct Reader<'a> {
    document: &'a Scalar,
    reply: &'a Reply,
}

impl<'a> Reader<'a> {
    /// A text the model requires.
    pub(crate) fn text(&self, key: &str) -> Result<&'a str> {
        self.optional_text(key)
            .ok_or_else(|| self.malformed(format!("expected {key:?} in the answer, got none")))
    }

    /// A text the model leaves optional.
    pub(crate) fn optional_text(&self, key: &str) -> Option<&'a str> {
        self.document
            .get_key_str(key)
            .and_then(Scalar::as_str)
            .filter(|text| !text.is_empty())
    }

    /// An ISO 8601 date-time, read as the instant it is; one that states no
    /// zone is UTC.
    pub(crate) fn instant(&self, key: &str) -> Result<DateTime64> {
        let text = self.text(key)?;
        DateTime64::from_text(text, Timezone::UTC)
            .map_err(|error| self.unreadable(key, "an ISO 8601 date-time", text, &error))
    }

    /// An ARN the model requires.
    pub(crate) fn arn(&self, key: &str) -> Result<Arn> {
        let text = self.text(key)?;
        Arn::from_str(text).map_err(|error| self.unreadable(key, "an ARN", text, &error))
    }

    /// A location the model requires.
    pub(crate) fn url(&self, key: &str) -> Result<Url> {
        let text = self.text(key)?;
        self.location(key, text)
    }

    /// A location the model leaves optional.
    pub(crate) fn optional_url(&self, key: &str) -> Result<Option<Url>> {
        self.optional_text(key)
            .map(|text| self.location(key, text))
            .transpose()
    }

    /// The one namespace name the model spells as a list of names: its
    /// levels joined by a dot, which today is the one name a table bucket's
    /// namespace has.
    pub(crate) fn namespace(&self) -> Result<String> {
        let levels: Vec<&str> = self
            .document
            .get_key_str("namespace")
            .and_then(Scalar::as_sequence)
            .map(|levels| levels.iter().filter_map(Scalar::as_str).collect())
            .unwrap_or_default();
        if levels.is_empty() {
            return Err(self.malformed("expected \"namespace\" in the answer, got none"));
        }
        Ok(levels.join("."))
    }

    /// The entries of the list `key` names, each read like the document.
    pub(crate) fn entries(&self, key: &str) -> Result<impl Iterator<Item = Reader<'a>> + use<'a>> {
        let entries = self
            .document
            .get_key_str(key)
            .and_then(Scalar::as_sequence)
            .ok_or_else(|| self.malformed(format!("expected {key:?} in the answer, got none")))?;
        let reply = self.reply;
        Ok(entries
            .iter()
            .map(move |document| Reader { document, reply }))
    }

    fn location(&self, key: &str, text: &str) -> Result<Url> {
        Url::from_str(text).map_err(|error| self.unreadable(key, "a location", text, &error))
    }

    fn unreadable(&self, key: &str, expected: &str, text: &str, error: &Error) -> Error {
        self.malformed(format!(
            "expected {expected} under {key:?}, got {:?}: {error}",
            echo(text)
        ))
    }

    /// Report an answer this client cannot read, against the call it
    /// answered.
    pub(crate) fn malformed(&self, reason: impl AsRef<str>) -> Error {
        Error::remote(
            SERVICE,
            self.reply.operation,
            self.reply.status,
            "MalformedAnswer",
            reason,
            &self.reply.addressed,
        )
    }
}

/// The JSON document of a request body, its members named as the model
/// names them.
///
/// # Errors
///
/// Returns the JSON codec's refusal of a value it cannot write.
pub(crate) fn body<const N: usize>(members: [(&'static str, Scalar); N]) -> Result<Vec<u8>> {
    crate::json::into_bytes(&Scalar::from_struct(members)?)
}

/// Refuse `name` unless it is what the model lets a `kind` be called:
/// `min` to `max` of the digits, the lower-case letters and `extra`.
///
/// # Errors
///
/// Returns [`Error::Parse`] naming the rule, at the first byte that breaks
/// it.
pub(crate) fn check_name(
    kind: &'static str,
    name: &str,
    min: usize,
    max: usize,
    extra: u8,
) -> Result<()> {
    let allowed = |byte: u8| byte.is_ascii_digit() || byte.is_ascii_lowercase() || byte == extra;
    let position = match name.bytes().position(|byte| !allowed(byte)) {
        Some(position) => position,
        None if name.len() < min => name.len(),
        None if name.len() > max => max,
        None => return Ok(()),
    };
    Err(Error::Parse {
        target: kind,
        position,
        reason: format_smolstr!(
            "expected {min} to {max} of 0-9, a-z and {:?}, got {:?}",
            char::from(extra),
            echo(name)
        ),
    })
}

/// Refuse a version token the model would refuse: it is at least one
/// character, and an empty one is a token nobody read.
///
/// # Errors
///
/// Returns [`Error::Parse`] at the token's start.
pub(crate) fn check_version_token(token: &str) -> Result<()> {
    if token.is_empty() {
        return Err(Error::Parse {
            target: "version token",
            position: 0,
            reason: smol_str::SmolStr::new_static(
                "expected the version token a reading of the table answered, got an empty one",
            ),
        });
    }
    Ok(())
}

/// At most [`ECHO_BYTES`] of `text`, for a refusal to quote.
pub(crate) fn echo(text: &str) -> &str {
    clip(text, ECHO_BYTES)
}

/// At most `bytes` of `text`, cut on a character boundary.
fn clip(text: &str, bytes: usize) -> &str {
    &text[..text.floor_char_boundary(bytes)]
}
