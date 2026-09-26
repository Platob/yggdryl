//! Every knob a session or a request reads, with its default in the
//! signature and one property door for a caller who starts from a
//! configuration file, a catalog's property bag or the environment.

use std::num::NonZero;
use std::path::{Path, PathBuf};
use std::time::Duration;

use smol_str::format_smolstr;

use super::{Authorization, Headers, Pagination};
use crate::{Codec, DEFAULT_STREAM_BATCH_SIZE, Error, FieldPath, Result, Url};

/// How a session reaches a server and reads what it answers.
///
/// Every knob has a `with_` setter and a getter of the same name; the
/// defaults are the constants beside them, and [`Self::from_properties`] and
/// [`Self::with_properties`] read the same knobs out of `(name, value)` text
/// pairs.
///
/// ```
/// use std::time::Duration;
///
/// use yggdryl::http::HttpOptions;
///
/// # fn main() -> yggdryl::Result<()> {
/// let options = HttpOptions::from_properties([
///     ("timeout", "30"),
///     ("connect-timeout", "2500ms"),
///     ("max_attempts", "5"),
///     ("follow_redirects", "no"),
///     ("header.X-Api-Key", "k-123"),
///     ("warehouse", "s3://lake"),  // not an HTTP knob: ignored
/// ])?;
/// assert_eq!(options.timeout(), Duration::from_secs(30));
/// assert_eq!(options.connect_timeout(), Duration::from_millis(2500));
/// assert_eq!(options.max_attempts(), 5);
/// assert!(!options.follow_redirects());
/// assert_eq!(options.headers().get("x-api-key"), Some("k-123"));
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct HttpOptions {
    base_url: Option<Url>,
    headers: Headers,
    authorization: Option<Authorization>,
    timeout: Duration,
    connect_timeout: Duration,
    max_attempts: u32,
    max_redirects: u32,
    follow_redirects: bool,
    user_agent: String,
    proxy: Option<String>,
    ca_bundle: Option<PathBuf>,
    accept_encodings: Vec<Codec>,
    read_environment: bool,
    max_body_size: u64,
    stream_batch_size: usize,
    concurrency: usize,
    pagination: Pagination,
    records: Option<FieldPath>,
    page_limit: Option<usize>,
    cookies: bool,
    max_pause: Duration,
}

impl HttpOptions {
    /// The whole-request timeout: two minutes.
    pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);
    /// The connection timeout: ten seconds.
    pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
    /// How many times one request is attempted: three.
    pub const DEFAULT_MAX_ATTEMPTS: u32 = 3;
    /// How many redirect hops are followed: ten.
    pub const DEFAULT_MAX_REDIRECTS: u32 = 10;
    /// The `User-Agent` sent unless the caller set one.
    pub const DEFAULT_USER_AGENT: &'static str = concat!("yggdryl/", env!("CARGO_PKG_VERSION"));
    /// The content codings a request says it decodes: gzip, deflate, zstd.
    pub const DEFAULT_ACCEPT_ENCODINGS: [Codec; 3] = [Codec::Gzip, Codec::Deflate, Codec::Zstd];
    /// The most a whole-body read holds: 256 MiB.
    pub const DEFAULT_MAX_BODY_SIZE: u64 = 256 * 1024 * 1024;
    /// The most threads that send requests side by side: eight.
    pub const MAX_DEFAULT_CONCURRENCY: usize = 8;
    /// The longest a `Retry-After` or a rate limit is waited for: thirty
    /// seconds.
    pub const DEFAULT_MAX_PAUSE: Duration = Duration::from_secs(30);

    /// The defaults, in a `const` context.
    fn defaults() -> Self {
        Self {
            base_url: None,
            headers: Headers::new(),
            authorization: None,
            timeout: Self::DEFAULT_TIMEOUT,
            connect_timeout: Self::DEFAULT_CONNECT_TIMEOUT,
            max_attempts: Self::DEFAULT_MAX_ATTEMPTS,
            max_redirects: Self::DEFAULT_MAX_REDIRECTS,
            follow_redirects: true,
            user_agent: Self::DEFAULT_USER_AGENT.to_owned(),
            proxy: None,
            ca_bundle: None,
            accept_encodings: Self::DEFAULT_ACCEPT_ENCODINGS.to_vec(),
            read_environment: true,
            max_body_size: Self::DEFAULT_MAX_BODY_SIZE,
            stream_batch_size: DEFAULT_STREAM_BATCH_SIZE,
            concurrency: default_concurrency(),
            pagination: Pagination::Auto,
            records: None,
            page_limit: None,
            cookies: true,
            max_pause: Self::DEFAULT_MAX_PAUSE,
        }
    }

    /// The defaults with the knobs `properties` names applied.
    ///
    /// # Errors
    ///
    /// As [`Self::with_properties`].
    pub fn from_properties<I, K, V>(properties: I) -> Result<Self>
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        Self::default().with_properties(properties)
    }

    /// Apply the knobs `properties` names, in snake or kebab case.
    ///
    /// | property | reads |
    /// | --- | --- |
    /// | `timeout`, `connect_timeout`, `max_pause` | seconds, decimal allowed, with an optional `s` or `ms` suffix |
    /// | `max_attempts`, `max_redirects`, `concurrency`, `page_limit`, `stream_batch_size` | a whole number; `page_limit` `0` clears the limit |
    /// | `follow_redirects`, `read_environment`, `cookies` | `true`/`false`, `1`/`0`, `yes`/`no` |
    /// | `max_body_size` | a byte count with an optional `KiB`, `MiB`, `GiB` (or `KB`, `MB`, `GB`) suffix |
    /// | `accept_encoding` | comma-separated coding names this crate decodes |
    /// | `user_agent`, `proxy`, `ca_bundle` | text as given |
    /// | `base_url` | a URL |
    /// | `pagination` | a [`Pagination`] spelling |
    /// | `records` | a [`FieldPath`] |
    /// | `bearer_token` | a `Bearer` [`Authorization`] |
    /// | `basic_auth` | `user:password`, a `Basic` [`Authorization`] |
    /// | `header.<name>`, `headers.<name>` | one default header, the name kept as spelled |
    ///
    /// A name this table does not have is ignored, so a catalog's whole
    /// property bag can be handed over; a value that is empty once trimmed
    /// is an absent property.
    ///
    /// # Errors
    ///
    /// [`Error::Parse`] naming the property whose value does not read as what
    /// its name means.
    pub fn with_properties<I, K, V>(mut self, properties: I) -> Result<Self>
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        for (name, value) in properties {
            let (name, value) = (name.as_ref(), value.as_ref().trim());
            if value.is_empty() {
                continue;
            }
            self = self.read(name, value)?;
        }
        Ok(self)
    }

    /// Apply one property.
    fn read(self, name: &str, value: &str) -> Result<Self> {
        let lowered = name.to_ascii_lowercase();
        if let Some(header) = lowered
            .strip_prefix("header.")
            .or_else(|| lowered.strip_prefix("headers."))
        {
            if header.is_empty() {
                return Err(refusal(name, value, "a header name after the dot"));
            }
            // The name is kept as the caller spelled it, so it is read off
            // `name` rather than the lowered copy.
            return self.with_header(&name[name.len() - header.len()..], value);
        }
        let key = lowered.replace('-', "_");
        Ok(match key.as_str() {
            "timeout" | "request_timeout" => self.with_timeout(seconds(name, value)?),
            "connect_timeout" | "connection_timeout" => {
                self.with_connect_timeout(seconds(name, value)?)
            }
            "max_pause" => self.with_max_pause(seconds(name, value)?),
            "max_attempts" => self.with_max_attempts(count(name, value)?),
            "max_redirects" => self.with_max_redirects(count(name, value)?),
            "concurrency" => self.with_concurrency(
                usize::try_from(count(name, value)?)
                    .map_err(|_| refusal(name, value, "a thread count"))?,
            ),
            "stream_batch_size" => self.with_stream_batch_size(
                usize::try_from(size(name, value)?)
                    .map_err(|_| refusal(name, value, "a batch size that fits memory"))?,
            ),
            "page_limit" => {
                let limit = usize::try_from(count(name, value)?)
                    .map_err(|_| refusal(name, value, "a page count"))?;
                self.with_page_limit((limit > 0).then_some(limit))
            }
            "follow_redirects" => self.with_follow_redirects(flag(name, value)?),
            "read_environment" => self.with_read_environment(flag(name, value)?),
            "cookies" => self.with_cookies(flag(name, value)?),
            "max_body_size" => self.with_max_body_size(size(name, value)?),
            "accept_encoding" | "accept_encodings" => {
                let mut codecs = Vec::new();
                for member in value.split(',') {
                    let member = member.trim();
                    if member.is_empty() {
                        continue;
                    }
                    codecs.push(
                        Codec::from_str(member).map_err(|_| {
                            refusal(name, value, "content codings this crate decodes")
                        })?,
                    );
                }
                self.with_accept_encodings(codecs)
            }
            "user_agent" => self.with_user_agent(value),
            "proxy" | "proxy_url" | "proxy_uri" => self.with_proxy(value),
            "ca_bundle" => self.with_ca_bundle(value),
            "base_url" => {
                self.with_base_url(Url::from_str(value).map_err(|_| refusal(name, value, "a URL"))?)
            }
            "pagination" => self.with_pagination(
                Pagination::from_str(value)
                    .map_err(|_| refusal(name, value, "a pagination spelling"))?,
            ),
            "records" => self.with_records(
                FieldPath::from_str(value).map_err(|_| refusal(name, value, "a field path"))?,
            ),
            "bearer_token" => self.with_authorization(Authorization::bearer(value)),
            "basic_auth" => {
                let (user, password) = value
                    .split_once(':')
                    .ok_or_else(|| refusal(name, value, "`user:password`"))?;
                self.with_authorization(Authorization::basic(user, password))
            }
            _ => self,
        })
    }

    /// The URL a relative request joins onto.
    #[must_use]
    pub fn with_base_url(mut self, base_url: Url) -> Self {
        self.base_url = Some(base_url);
        self
    }

    /// The headers every request carries unless it sets its own.
    #[must_use]
    pub fn with_headers(mut self, headers: Headers) -> Self {
        self.headers = headers;
        self
    }

    /// One more default header, replacing the value the name had.
    ///
    /// # Errors
    ///
    /// As [`Headers::insert`].
    pub fn with_header(mut self, name: &str, value: &str) -> Result<Self> {
        self.headers.insert(name, value)?;
        Ok(self)
    }

    /// The credential every request sends unless it names its own.
    #[must_use]
    pub fn with_authorization(mut self, authorization: Authorization) -> Self {
        self.authorization = Some(authorization);
        self
    }

    /// The whole-request timeout.
    #[must_use]
    pub const fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// The connection timeout.
    #[must_use]
    pub const fn with_connect_timeout(mut self, connect_timeout: Duration) -> Self {
        self.connect_timeout = connect_timeout;
        self
    }

    /// How many times one request is attempted; zero is one.
    #[must_use]
    pub const fn with_max_attempts(mut self, max_attempts: u32) -> Self {
        self.max_attempts = if max_attempts == 0 { 1 } else { max_attempts };
        self
    }

    /// How many redirect hops are followed.
    #[must_use]
    pub const fn with_max_redirects(mut self, max_redirects: u32) -> Self {
        self.max_redirects = max_redirects;
        self
    }

    /// Whether a 3xx is followed at all.
    #[must_use]
    pub const fn with_follow_redirects(mut self, follow_redirects: bool) -> Self {
        self.follow_redirects = follow_redirects;
        self
    }

    /// The `User-Agent` sent unless a request sets one.
    #[must_use]
    pub fn with_user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = user_agent.into();
        self
    }

    /// The proxy URL every request goes through.
    #[must_use]
    pub fn with_proxy(mut self, proxy: impl Into<String>) -> Self {
        self.proxy = Some(proxy.into());
        self
    }

    /// The CA bundle TLS trusts instead of the platform's.
    #[must_use]
    pub fn with_ca_bundle(mut self, ca_bundle: impl Into<PathBuf>) -> Self {
        self.ca_bundle = Some(ca_bundle.into());
        self
    }

    /// The content codings a request says it decodes, in preference order.
    #[must_use]
    pub fn with_accept_encodings(mut self, codecs: impl IntoIterator<Item = Codec>) -> Self {
        self.accept_encodings = codecs.into_iter().collect();
        self
    }

    /// Whether the proxy and CA bundle variables of the environment are read
    /// where the options name none.
    #[must_use]
    pub const fn with_read_environment(mut self, read_environment: bool) -> Self {
        self.read_environment = read_environment;
        self
    }

    /// The most a whole-body read holds.
    #[must_use]
    pub const fn with_max_body_size(mut self, max_body_size: u64) -> Self {
        self.max_body_size = max_body_size;
        self
    }

    /// The chunk a streamed body is read in; zero is the crate's default.
    #[must_use]
    pub const fn with_stream_batch_size(mut self, stream_batch_size: usize) -> Self {
        self.stream_batch_size = if stream_batch_size == 0 {
            DEFAULT_STREAM_BATCH_SIZE
        } else {
            stream_batch_size
        };
        self
    }

    /// How many requests go out side by side; zero is one.
    #[must_use]
    pub const fn with_concurrency(mut self, concurrency: usize) -> Self {
        self.concurrency = if concurrency == 0 { 1 } else { concurrency };
        self
    }

    /// How the next page of a paginated resource is found.
    #[must_use]
    pub fn with_pagination(mut self, pagination: Pagination) -> Self {
        self.pagination = pagination;
        self
    }

    /// Where a page's rows are in its document.
    #[must_use]
    pub fn with_records(mut self, records: FieldPath) -> Self {
        self.records = Some(records);
        self
    }

    /// The most pages a walk reads; `None` reads them all.
    #[must_use]
    pub const fn with_page_limit(mut self, page_limit: Option<usize>) -> Self {
        self.page_limit = page_limit;
        self
    }

    /// Whether `Set-Cookie` is kept and `Cookie` sent back.
    #[must_use]
    pub const fn with_cookies(mut self, cookies: bool) -> Self {
        self.cookies = cookies;
        self
    }

    /// The longest a `Retry-After` or a rate limit is waited for.
    #[must_use]
    pub const fn with_max_pause(mut self, max_pause: Duration) -> Self {
        self.max_pause = max_pause;
        self
    }

    /// The URL a relative request joins onto.
    #[must_use]
    pub fn base_url(&self) -> Option<&Url> {
        self.base_url.as_ref()
    }

    /// The headers every request carries unless it sets its own.
    #[must_use]
    pub fn headers(&self) -> &Headers {
        &self.headers
    }

    /// The credential every request sends unless it names its own.
    #[must_use]
    pub fn authorization(&self) -> Option<&Authorization> {
        self.authorization.as_ref()
    }

    /// The whole-request timeout.
    #[must_use]
    pub const fn timeout(&self) -> Duration {
        self.timeout
    }

    /// The connection timeout.
    #[must_use]
    pub const fn connect_timeout(&self) -> Duration {
        self.connect_timeout
    }

    /// How many times one request is attempted.
    #[must_use]
    pub const fn max_attempts(&self) -> u32 {
        self.max_attempts
    }

    /// How many redirect hops are followed.
    #[must_use]
    pub const fn max_redirects(&self) -> u32 {
        self.max_redirects
    }

    /// Whether a 3xx is followed at all.
    #[must_use]
    pub const fn follow_redirects(&self) -> bool {
        self.follow_redirects
    }

    /// The `User-Agent` sent unless a request sets one.
    #[must_use]
    pub fn user_agent(&self) -> &str {
        &self.user_agent
    }

    /// The proxy URL every request goes through, when one is named.
    #[must_use]
    pub fn proxy(&self) -> Option<&str> {
        self.proxy.as_deref()
    }

    /// The CA bundle TLS trusts instead of the platform's, when one is named.
    #[must_use]
    pub fn ca_bundle(&self) -> Option<&Path> {
        self.ca_bundle.as_deref()
    }

    /// The content codings a request says it decodes, in preference order.
    #[must_use]
    pub fn accept_encodings(&self) -> &[Codec] {
        &self.accept_encodings
    }

    /// Whether the environment's proxy and CA bundle variables are read.
    #[must_use]
    pub const fn read_environment(&self) -> bool {
        self.read_environment
    }

    /// The most a whole-body read holds.
    #[must_use]
    pub const fn max_body_size(&self) -> u64 {
        self.max_body_size
    }

    /// The chunk a streamed body is read in.
    #[must_use]
    pub const fn stream_batch_size(&self) -> usize {
        self.stream_batch_size
    }

    /// How many requests go out side by side.
    #[must_use]
    pub const fn concurrency(&self) -> usize {
        self.concurrency
    }

    /// How the next page of a paginated resource is found.
    #[must_use]
    pub const fn pagination(&self) -> &Pagination {
        &self.pagination
    }

    /// Where a page's rows are in its document, when declared.
    #[must_use]
    pub fn records(&self) -> Option<&FieldPath> {
        self.records.as_ref()
    }

    /// The most pages a walk reads, when bounded.
    #[must_use]
    pub const fn page_limit(&self) -> Option<usize> {
        self.page_limit
    }

    /// Whether `Set-Cookie` is kept and `Cookie` sent back.
    #[must_use]
    pub const fn cookies(&self) -> bool {
        self.cookies
    }

    /// The longest a `Retry-After` or a rate limit is waited for.
    #[must_use]
    pub const fn max_pause(&self) -> Duration {
        self.max_pause
    }
}

impl Default for HttpOptions {
    fn default() -> Self {
        Self::defaults()
    }
}

/// The parallelism the machine has, at most [`HttpOptions::MAX_DEFAULT_CONCURRENCY`].
fn default_concurrency() -> usize {
    std::thread::available_parallelism()
        .map_or(1, NonZero::get)
        .min(HttpOptions::MAX_DEFAULT_CONCURRENCY)
}

/// A duration in seconds, decimal allowed, with an optional `s` or `ms`
/// suffix.
fn seconds(name: &str, value: &str) -> Result<Duration> {
    let lowered = value.to_ascii_lowercase();
    let (digits, scale) = if let Some(digits) = lowered.strip_suffix("ms") {
        (digits, 0.001)
    } else if let Some(digits) = lowered.strip_suffix('s') {
        (digits, 1.0)
    } else {
        (lowered.as_str(), 1.0)
    };
    let count: f64 = digits
        .trim()
        .parse()
        .map_err(|_| refusal(name, value, "seconds, with an optional `s` or `ms` suffix"))?;
    if !count.is_finite() || count < 0.0 {
        return Err(refusal(name, value, "a duration of at least zero"));
    }
    Ok(Duration::from_secs_f64(count * scale))
}

/// A whole number.
fn count(name: &str, value: &str) -> Result<u32> {
    value
        .parse()
        .map_err(|_| refusal(name, value, "a whole number"))
}

/// A boolean: `true`/`false`, `1`/`0`, `yes`/`no`, in any case.
fn flag(name: &str, value: &str) -> Result<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" => Ok(true),
        "false" | "0" | "no" => Ok(false),
        _ => Err(refusal(name, value, "true/false, 1/0 or yes/no")),
    }
}

/// A byte count with an optional binary or decimal unit suffix, both read
/// as powers of 1024 the way every configuration file means them.
fn size(name: &str, value: &str) -> Result<u64> {
    const UNITS: [(&str, u64); 10] = [
        ("gib", 1 << 30),
        ("mib", 1 << 20),
        ("kib", 1 << 10),
        ("gb", 1 << 30),
        ("mb", 1 << 20),
        ("kb", 1 << 10),
        ("g", 1 << 30),
        ("m", 1 << 20),
        ("k", 1 << 10),
        ("b", 1),
    ];
    let lowered = value.to_ascii_lowercase();
    let (digits, scale) = UNITS
        .iter()
        .find_map(|(suffix, scale)| lowered.strip_suffix(suffix).map(|digits| (digits, *scale)))
        .unwrap_or((lowered.as_str(), 1));
    let count: u64 = digits.trim().parse().map_err(|_| {
        refusal(
            name,
            value,
            "a byte count, with an optional KiB/MiB/GiB suffix",
        )
    })?;
    Ok(count.saturating_mul(scale))
}

/// Refuse the value of a property this door knows.
fn refusal(name: &str, value: &str, expected: &str) -> Error {
    Error::Parse {
        target: "http option",
        position: 0,
        reason: format_smolstr!("{name}: expected {expected}, got {value:?}"),
    }
}
