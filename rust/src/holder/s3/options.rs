//! The knobs an S3 client has, and the order their values are found in.
//!
//! Nothing here touches the network. Resolution against the environment and
//! the shared AWS files happens once, on the first request, so constructing a
//! handle stays free: the options only record what a caller said explicitly.

use std::time::Duration;

use super::credentials::Credentials;
use super::encryption::Encryption;
use super::sts::AssumedRole;

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
    payload_signing: Option<bool>,
    encryption: Encryption,
    proxy: Option<String>,
    assumed_role: Option<AssumedRole>,
    bucket_creation: bool,
    bucket_deletion: bool,
    metadata: Vec<(String, String)>,
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
            payload_signing: None,
            encryption: Encryption::Default,
            proxy: None,
            assumed_role: None,
            bucket_creation: true,
            bucket_deletion: true,
            metadata: Vec::new(),
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

    /// Sign the body of every write, or send it as `UNSIGNED-PAYLOAD`.
    ///
    /// Signing hashes the whole value with SHA-256 so the store can verify
    /// what it received. That is worth paying for over a plain-HTTP endpoint,
    /// where nothing else protects the body, and it is what an unset value
    /// selects there. Over HTTPS the transport already guarantees integrity,
    /// so an unset value skips the hash - which on a large upload is the
    /// difference between hashing the value and not.
    #[must_use]
    pub const fn with_payload_signing(mut self, signing: bool) -> Self {
        self.payload_signing = Some(signing);
        self
    }

    /// Encrypt what is written, and carry what is needed to read it back.
    ///
    /// The choice belongs to the client rather than to one write because a
    /// customer-supplied key has to be presented again on every read; see
    /// [`Encryption`] for what each kind says on the wire.
    #[must_use]
    pub fn with_encryption(mut self, encryption: Encryption) -> Self {
        self.encryption = encryption;
        self
    }

    /// Reach the endpoint through the proxy at `uri`.
    ///
    /// `http://`, `https://`, `socks4://`, and `socks5://` are understood, and
    /// a proxy may carry credentials of its own as `user:password@host`.
    /// Unset, the transport reads the usual `HTTPS_PROXY` and `NO_PROXY`
    /// variables, which is what most environments already say.
    #[must_use]
    pub fn with_proxy(mut self, uri: impl Into<String>) -> Self {
        let uri: String = uri.into();
        self.proxy = (!uri.trim().is_empty()).then(|| uri.trim().to_owned());
        self
    }

    /// Sign bucket requests as `role` rather than as the keys that were found.
    ///
    /// The credential chain still answers, and what it answers is what signs
    /// the *exchange*: one STS request trades those keys for the role's, and
    /// the session it hands back is what reaches the bucket. It expires, so it
    /// is traded again shortly before it does rather than per request.
    #[must_use]
    pub fn with_assumed_role(mut self, role: AssumedRole) -> Self {
        self.assumed_role = Some(role);
        self
    }

    /// Allow, or refuse, creating a bucket through a container handle.
    ///
    /// Creating one is what [`IOFolder::create_folder`] does at a bucket root,
    /// and a process that must never make one says so here. Refusing costs no
    /// request: it is a refusal, not a probe.
    ///
    /// [`IOFolder::create_folder`]: crate::IOFolder::create_folder
    #[must_use]
    pub const fn with_bucket_creation(mut self, allowed: bool) -> Self {
        self.bucket_creation = allowed;
        self
    }

    /// Allow, or refuse, deleting a bucket through a container handle.
    #[must_use]
    pub const fn with_bucket_deletion(mut self, allowed: bool) -> Self {
        self.bucket_deletion = allowed;
        self
    }

    /// Carry `metadata` on every object this client writes.
    ///
    /// A name S3 itself defines - `content-encoding`, `cache-control`, and the
    /// rest - is sent as that header; anything else becomes user metadata,
    /// under `x-amz-meta-`. A name the write already sets for itself wins, so
    /// this never overrides the content type a handle inferred.
    #[must_use]
    pub fn with_default_metadata<K, V>(mut self, metadata: impl IntoIterator<Item = (K, V)>) -> Self
    where
        K: Into<String>,
        V: Into<String>,
    {
        self.metadata = metadata
            .into_iter()
            .map(|(name, value)| (header_name(&name.into()), value.into()))
            .collect();
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

    /// The explicit payload-signing choice.
    pub const fn payload_signing(&self) -> Option<bool> {
        self.payload_signing
    }

    /// How writes are encrypted at rest.
    pub const fn encryption(&self) -> &Encryption {
        &self.encryption
    }

    /// The proxy the endpoint is reached through, when one was named.
    pub fn proxy(&self) -> Option<&str> {
        self.proxy.as_deref()
    }

    /// The role bucket requests are signed as, when one was named.
    pub const fn assumed_role(&self) -> Option<&AssumedRole> {
        self.assumed_role.as_ref()
    }

    /// Whether a container handle may create a bucket.
    pub const fn bucket_creation(&self) -> bool {
        self.bucket_creation
    }

    /// Whether a container handle may delete a bucket.
    pub const fn bucket_deletion(&self) -> bool {
        self.bucket_deletion
    }

    /// The metadata every write carries, as the headers it goes over as.
    pub fn default_metadata(&self) -> &[(String, String)] {
        &self.metadata
    }

    /// Whether a write to an endpoint of `scheme` signs its body.
    ///
    /// Explicit wins; otherwise TLS decides, because TLS is what the hash
    /// would otherwise be duplicating.
    pub(super) fn signs_payload(&self, scheme: &str) -> bool {
        self.payload_signing
            .unwrap_or(!scheme.eq_ignore_ascii_case("https"))
    }

    /// Whether the transport differs from the process-wide default, in which
    /// case the client needs a connection pool of its own.
    pub(super) fn has_custom_transport(&self) -> bool {
        self.timeout != DEFAULT_TIMEOUT
            || self.connect_timeout != DEFAULT_CONNECT_TIMEOUT
            || self.proxy.is_some()
    }
}

/// The header a metadata name goes over as.
///
/// S3 defines a handful of names itself and treats every other as user
/// metadata, which travels under `x-amz-meta-`. A caller who spells the prefix
/// is taken at their word.
fn header_name(name: &str) -> String {
    const OWN: [&str; 6] = [
        "cache-control",
        "content-disposition",
        "content-encoding",
        "content-language",
        "content-type",
        "expires",
    ];
    let lowered = name.trim().to_ascii_lowercase();
    if OWN.contains(&lowered.as_str()) || lowered.starts_with("x-amz-") {
        lowered
    } else {
        format!("x-amz-meta-{lowered}")
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
            .field("payload_signing", &self.payload_signing)
            // `Encryption` redacts a customer key.
            .field("encryption", &self.encryption)
            .field("proxy", &self.proxy)
            .field("assumed_role", &self.assumed_role)
            .field("bucket_creation", &self.bucket_creation)
            .field("bucket_deletion", &self.bucket_deletion)
            .field("metadata", &self.metadata)
            .finish()
    }
}
