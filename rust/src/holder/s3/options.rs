//! The knobs an S3 client has, and the order their values are found in.
//!
//! Nothing here touches the network. Resolution against the environment and
//! the shared AWS files happens once, on the first request, so constructing a
//! handle stays free: the options only record what a caller said explicitly.

use std::time::Duration;

use super::credentials::Credentials;

/// Part size when nothing else is said: 16 MiB, well above S3's 5 MiB floor.
const DEFAULT_PART_SIZE: u64 = 16 * 1024 * 1024;
/// The smallest part S3 accepts for every part but the last.
const MIN_PART_SIZE: u64 = 5 * 1024 * 1024;
/// The largest single `PutObject` S3 accepts; multipart is mandatory above it.
const MAX_SINGLE_PUT: u64 = 5 * 1024 * 1024 * 1024;
/// Multipart threshold when nothing else is said: 64 MiB.
///
/// A single `PutObject` is one request where multipart is `parts + 2`, so the
/// threshold sits high: below it one request wins, above it bounded retry
/// granularity does.
const DEFAULT_MULTIPART_THRESHOLD: u64 = 64 * 1024 * 1024;
/// Keys per listing page when nothing else is said: S3's own maximum.
const DEFAULT_LIST_PAGE_SIZE: u16 = 1000;
/// Attempts per request when nothing else is said: the first plus two retries.
const DEFAULT_MAX_ATTEMPTS: u32 = 3;
/// Per-request time budget when nothing else is said.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);
/// Connection establishment budget when nothing else is said.
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// How an S3 client reaches the store and signs what it sends.
///
/// Every knob is optional, and an unset one is resolved from the same places
/// the AWS tools read, in this order: the URL, the environment, the shared
/// configuration files, then a default. Explicit values always win.
///
/// | knob | URL | environment | `~/.aws/config` | default |
/// | --- | --- | --- | --- | --- |
/// | endpoint | hostname (with port) | `AWS_ENDPOINT_URL_S3`, `AWS_ENDPOINT_URL` | `endpoint_url` | `https://s3.{region}.amazonaws.com` |
/// | region | recognized AWS hostname | `AWS_REGION`, `AWS_DEFAULT_REGION` | `region` | `us-east-1` |
/// | credentials | `s3://key:secret@bucket/` | `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_SESSION_TOKEN` | `~/.aws/credentials` | container, then instance metadata, then anonymous |
/// | profile | | `AWS_PROFILE` | | `default` |
/// | path style | | `AWS_S3_FORCE_PATH_STYLE` | | virtual-hosted on AWS, path style elsewhere |
///
/// ```
/// use std::time::Duration;
///
/// use yggdryl::holder::s3::{Credentials, S3Options};
///
/// let options = S3Options::default()
///     .with_endpoint("http://localhost:9000")
///     .with_region("us-east-1")
///     .with_credentials(Credentials::new("minioadmin", "minioadmin"))
///     .with_path_style(true)
///     .with_timeout(Duration::from_secs(30));
///
/// assert_eq!(options.endpoint(), Some("http://localhost:9000"));
/// assert_eq!(options.region(), Some("us-east-1"));
/// assert_eq!(options.path_style(), Some(true));
/// // Part sizes are clamped to what S3 accepts rather than refused.
/// assert_eq!(
///     S3Options::default().with_part_size(1).part_size(),
///     5 * 1024 * 1024
/// );
/// ```
#[derive(Clone)]
pub struct S3Options {
    endpoint: Option<String>,
    region: Option<String>,
    credentials: Option<Credentials>,
    anonymous: bool,
    profile: Option<String>,
    path_style: Option<bool>,
    part_size: u64,
    multipart_threshold: u64,
    list_page_size: u16,
    max_attempts: u32,
    timeout: Duration,
    connect_timeout: Duration,
    read_environment: bool,
}

impl Default for S3Options {
    fn default() -> Self {
        Self {
            endpoint: None,
            region: None,
            credentials: None,
            anonymous: false,
            profile: None,
            path_style: None,
            part_size: DEFAULT_PART_SIZE,
            multipart_threshold: DEFAULT_MULTIPART_THRESHOLD,
            list_page_size: DEFAULT_LIST_PAGE_SIZE,
            max_attempts: DEFAULT_MAX_ATTEMPTS,
            timeout: DEFAULT_TIMEOUT,
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            read_environment: true,
        }
    }
}

impl S3Options {
    /// Address the store at `endpoint`, a URL such as `https://s3.example.io`
    /// or `http://localhost:9000`.
    ///
    /// A bare host is taken as `https`; a trailing slash is dropped.
    #[must_use]
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        let endpoint: String = endpoint.into();
        let endpoint = endpoint.trim().trim_end_matches('/');
        self.endpoint = Some(if endpoint.contains("://") {
            endpoint.to_owned()
        } else {
            format!("https://{endpoint}")
        });
        self
    }

    /// Sign for `region`.
    #[must_use]
    pub fn with_region(mut self, region: impl Into<String>) -> Self {
        self.region = Some(region.into());
        self
    }

    /// Sign with exactly these credentials, consulting nothing else.
    #[must_use]
    pub fn with_credentials(mut self, credentials: Credentials) -> Self {
        self.credentials = Some(credentials);
        self.anonymous = false;
        self
    }

    /// Send unsigned requests, for public buckets.
    #[must_use]
    pub fn with_anonymous(mut self, anonymous: bool) -> Self {
        self.anonymous = anonymous;
        if anonymous {
            self.credentials = None;
        }
        self
    }

    /// Read `profile` from the shared AWS files instead of `AWS_PROFILE`.
    #[must_use]
    pub fn with_profile(mut self, profile: impl Into<String>) -> Self {
        self.profile = Some(profile.into());
        self
    }

    /// Address buckets as `/{bucket}/key` on the endpoint rather than as
    /// `{bucket}.{endpoint}`.
    #[must_use]
    pub fn with_path_style(mut self, path_style: bool) -> Self {
        self.path_style = Some(path_style);
        self
    }

    /// Upload `part_size` bytes per multipart part, clamped to S3's 5 MiB
    /// floor and 5 GiB ceiling.
    #[must_use]
    pub fn with_part_size(mut self, part_size: u64) -> Self {
        self.part_size = part_size.clamp(MIN_PART_SIZE, MAX_SINGLE_PUT);
        self
    }

    /// Upload values of at least `threshold` bytes in parts, clamped to S3's
    /// 5 GiB single-put ceiling.
    #[must_use]
    pub fn with_multipart_threshold(mut self, threshold: u64) -> Self {
        self.multipart_threshold = threshold.min(MAX_SINGLE_PUT);
        self
    }

    /// Ask for `page_size` keys per listing request, clamped to `1..=1000`.
    #[must_use]
    pub fn with_list_page_size(mut self, page_size: u16) -> Self {
        self.list_page_size = page_size.clamp(1, DEFAULT_LIST_PAGE_SIZE);
        self
    }

    /// Attempt each request at most `attempts` times, at least once.
    #[must_use]
    pub fn with_max_attempts(mut self, attempts: u32) -> Self {
        self.max_attempts = attempts.max(1);
        self
    }

    /// Bound each request, headers to last body byte, by `timeout`.
    #[must_use]
    pub const fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Bound connection establishment by `timeout`.
    #[must_use]
    pub const fn with_connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = timeout;
        self
    }

    /// Consult, or ignore, the process environment and the shared AWS files.
    ///
    /// Off, only explicit values and the URL decide, which is what a test
    /// wants and what a sandboxed process may need.
    #[must_use]
    pub const fn with_environment(mut self, read_environment: bool) -> Self {
        self.read_environment = read_environment;
        self
    }

    /// The explicit endpoint URL.
    pub fn endpoint(&self) -> Option<&str> {
        self.endpoint.as_deref()
    }

    /// The explicit region.
    pub fn region(&self) -> Option<&str> {
        self.region.as_deref()
    }

    /// The explicit credentials.
    pub const fn credentials(&self) -> Option<&Credentials> {
        self.credentials.as_ref()
    }

    /// Whether requests go unsigned.
    pub const fn anonymous(&self) -> bool {
        self.anonymous
    }

    /// The explicit profile name.
    pub fn profile(&self) -> Option<&str> {
        self.profile.as_deref()
    }

    /// The explicit addressing style.
    pub const fn path_style(&self) -> Option<bool> {
        self.path_style
    }

    /// Bytes per multipart part.
    pub const fn part_size(&self) -> u64 {
        self.part_size
    }

    /// The value size from which multipart upload is used.
    pub const fn multipart_threshold(&self) -> u64 {
        self.multipart_threshold
    }

    /// Keys per listing request.
    pub const fn list_page_size(&self) -> u16 {
        self.list_page_size
    }

    /// Attempts per request.
    pub const fn max_attempts(&self) -> u32 {
        self.max_attempts
    }

    /// The per-request time budget.
    pub const fn timeout(&self) -> Duration {
        self.timeout
    }

    /// The connection establishment budget.
    pub const fn connect_timeout(&self) -> Duration {
        self.connect_timeout
    }

    /// Whether the environment and the shared AWS files are consulted.
    pub const fn reads_environment(&self) -> bool {
        self.read_environment
    }

    /// Whether the transport differs from the process-wide default, in which
    /// case the client needs a connection pool of its own.
    pub(super) fn has_custom_transport(&self) -> bool {
        self.timeout != DEFAULT_TIMEOUT || self.connect_timeout != DEFAULT_CONNECT_TIMEOUT
    }
}

impl std::fmt::Debug for S3Options {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("S3Options")
            .field("endpoint", &self.endpoint)
            .field("region", &self.region)
            // `Credentials` redacts its own secret.
            .field("credentials", &self.credentials)
            .field("anonymous", &self.anonymous)
            .field("profile", &self.profile)
            .field("path_style", &self.path_style)
            .field("part_size", &self.part_size)
            .field("multipart_threshold", &self.multipart_threshold)
            .field("list_page_size", &self.list_page_size)
            .field("max_attempts", &self.max_attempts)
            .field("timeout", &self.timeout)
            .field("connect_timeout", &self.connect_timeout)
            .field("read_environment", &self.read_environment)
            .finish()
    }
}
