//! The knobs an object-store client has, and the order their values are found
//! in.
//!
//! Nothing here touches the network. Resolution against the environment and a
//! store's own configuration files happens once, on the first request, so
//! constructing a handle stays free: the options only record what a caller said
//! explicitly.
//!
//! A knob all three stores have is here. A knob one store has is on that
//! store's own options - [`AwsOptions`](super::AwsOptions),
//! [`GoogleOptions`](super::GoogleOptions), and
//! [`AzureOptions`](super::AzureOptions), reached through
//! [`ObjectOptions::with_aws`] and its two siblings. A knob therefore has
//! exactly one owner, and nothing pretends the three stores are one store.

use std::time::Duration;

use super::aws::credentials::Credentials;
use super::aws::options::AwsOptions;
use super::azure::options::AzureOptions;
use super::encryption::Encryption;
use super::google::options::GoogleOptions;

/// Part size when nothing else is said: 16 MiB, well above S3's 5 MiB floor
/// and a multiple of Google's 256 KiB one.
const DEFAULT_PART_SIZE: u64 = 16 * 1024 * 1024;
/// Multipart threshold when nothing else is said: 64 MiB.
///
/// A single write is one request where a chunked upload is several, so the
/// threshold sits high: below it one request wins, above it bounded retry
/// granularity does.
const DEFAULT_MULTIPART_THRESHOLD: u64 = 64 * 1024 * 1024;
/// Entries per listing page when nothing else is said.
const DEFAULT_LIST_PAGE_SIZE: u16 = 1000;
/// Attempts per request when nothing else is said: the first plus two retries.
const DEFAULT_MAX_ATTEMPTS: u32 = 3;
/// Per-request time budget when nothing else is said.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);
/// Connection establishment budget when nothing else is said.
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Which environment variables are read when nothing else is said.
///
/// One prefix per store, because that is what each store's own tools set and
/// what every deployment already has, plus `YGGDRYL_` so a process can
/// configure this client without pretending to be configuring theirs.
const DEFAULT_ENVIRONMENT_PREFIXES: [&str; 4] = ["AWS_", "GOOGLE_", "AZURE_", "YGGDRYL_"];

/// How an object-store client reaches a store and authorizes what it sends.
///
/// Every knob is optional, and an unset one is resolved from the same places
/// each store's own tools read, in this order: the URL, the environment, the
/// store's configuration files, then a default. Explicit values always win.
///
/// | knob | URL | environment | file | default |
/// | --- | --- | --- | --- | --- |
/// | endpoint | hostname (with port) | `AWS_ENDPOINT_URL_S3`, `STORAGE_EMULATOR_HOST`, `AZURE_STORAGE_BLOB_ENDPOINT` | `~/.aws/config` | the store's published host |
/// | region | recognized hostname | `AWS_REGION`, `AWS_DEFAULT_REGION` | `~/.aws/config` | `us-east-1` |
/// | credentials | `s3://key:secret@bucket/` | each store's own names | `~/.aws/credentials`, a Google credentials document | the instance's own identity, then anonymous |
///
/// ```
/// use std::time::Duration;
///
/// use yggdryl::holder::object::{Credentials, ObjectOptions};
///
/// let options = ObjectOptions::default()
///     .with_endpoint("http://localhost:9000")
///     .with_region("us-east-1")
///     .with_credentials(Credentials::new("minioadmin", "minioadmin"))
///     .with_path_style(true)
///     .with_timeout(Duration::from_secs(30));
///
/// assert_eq!(options.endpoint(), Some("http://localhost:9000"));
/// assert_eq!(options.region(), Some("us-east-1"));
/// assert_eq!(options.path_style(), Some(true));
/// ```
#[derive(Clone)]
pub struct ObjectOptions {
    endpoint: Option<String>,
    region: Option<String>,
    credentials: Option<Credentials>,
    anonymous: bool,
    path_style: Option<bool>,
    part_size: u64,
    multipart_threshold: u64,
    list_page_size: u16,
    max_attempts: u32,
    timeout: Duration,
    connect_timeout: Duration,
    read_environment: bool,
    encryption: Encryption,
    proxy: Option<String>,
    environment_prefixes: Vec<String>,
    container_creation: bool,
    container_deletion: bool,
    metadata: Vec<(String, String)>,
    aws: AwsOptions,
    google: GoogleOptions,
    azure: AzureOptions,
}

impl Default for ObjectOptions {
    fn default() -> Self {
        Self {
            endpoint: None,
            region: None,
            credentials: None,
            anonymous: false,
            path_style: None,
            part_size: DEFAULT_PART_SIZE,
            multipart_threshold: DEFAULT_MULTIPART_THRESHOLD,
            list_page_size: DEFAULT_LIST_PAGE_SIZE,
            max_attempts: DEFAULT_MAX_ATTEMPTS,
            timeout: DEFAULT_TIMEOUT,
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            read_environment: true,
            encryption: Encryption::Default,
            proxy: None,
            environment_prefixes: DEFAULT_ENVIRONMENT_PREFIXES
                .iter()
                .map(|prefix| (*prefix).to_owned())
                .collect(),
            container_creation: true,
            container_deletion: true,
            metadata: Vec::new(),
            aws: AwsOptions::default(),
            google: GoogleOptions::default(),
            azure: AzureOptions::default(),
        }
    }
}

impl ObjectOptions {
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

    /// Sign for, or address, `region`.
    #[must_use]
    pub fn with_region(mut self, region: impl Into<String>) -> Self {
        self.region = Some(region.into());
        self
    }

    /// Authorize with exactly these credentials, consulting nothing else.
    ///
    /// One pair serves all three stores, because all three have one: an access
    /// key and its secret on Amazon S3, an HMAC key on Google Cloud Storage,
    /// and an account name with its shared key on Azure.
    #[must_use]
    pub fn with_credentials(mut self, credentials: Credentials) -> Self {
        self.credentials = Some(credentials);
        self.anonymous = false;
        self
    }

    /// Send unauthorized requests, for public containers.
    #[must_use]
    pub fn with_anonymous(mut self, anonymous: bool) -> Self {
        self.anonymous = anonymous;
        if anonymous {
            self.credentials = None;
        }
        self
    }

    /// Address containers as `/{container}/key` on the endpoint rather than as
    /// `{container}.{endpoint}`.
    ///
    /// Only Amazon S3 and Google's XML API offer the choice; Azure's container
    /// is always a path segment and says so by refusing the other spelling.
    #[must_use]
    pub fn with_path_style(mut self, path_style: bool) -> Self {
        self.path_style = Some(path_style);
        self
    }

    /// Upload `part_size` bytes per part.
    ///
    /// The value is recorded as given and clamped to what the store the handle
    /// reaches actually accepts, because the three floors and ceilings differ:
    /// 5 MiB on S3, a multiple of 256 KiB on Google, a block on Azure.
    #[must_use]
    pub const fn with_part_size(mut self, part_size: u64) -> Self {
        self.part_size = part_size;
        self
    }

    /// Upload values of at least `threshold` bytes in parts.
    #[must_use]
    pub const fn with_multipart_threshold(mut self, threshold: u64) -> Self {
        self.multipart_threshold = threshold;
        self
    }

    /// Ask for `page_size` entries per listing request.
    #[must_use]
    pub const fn with_list_page_size(mut self, page_size: u16) -> Self {
        self.list_page_size = page_size;
        self
    }

    /// Attempt each request at most `attempts` times, at least once.
    #[must_use]
    pub const fn with_max_attempts(mut self, attempts: u32) -> Self {
        self.max_attempts = if attempts == 0 { 1 } else { attempts };
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

    /// Encrypt what is written, and carry what is needed to read it back.
    ///
    /// The choice belongs to the client rather than to one write because a
    /// customer-supplied key has to be presented again on every read; see
    /// [`Encryption`] for what each kind says on the wire, and which of the
    /// three stores has it.
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

    /// Allow, or refuse, creating a container through a container handle.
    ///
    /// Creating one is what [`IOFolder::create_folder`] does at a container
    /// root, and a process that must never make one says so here. Refusing
    /// costs no request: it is a refusal, not a probe.
    ///
    /// [`IOFolder::create_folder`]: crate::IOFolder::create_folder
    #[must_use]
    pub const fn with_container_creation(mut self, allowed: bool) -> Self {
        self.container_creation = allowed;
        self
    }

    /// Allow, or refuse, deleting a container through a container handle.
    #[must_use]
    pub const fn with_container_deletion(mut self, allowed: bool) -> Self {
        self.container_deletion = allowed;
        self
    }

    /// Carry `metadata` on every object this client writes.
    ///
    /// A name the store defines itself - `content-encoding`, `cache-control`,
    /// and the rest - is sent as that header; anything else becomes user
    /// metadata under the store's own prefix. A name the write already sets for
    /// itself wins, so this never overrides the content type a handle inferred.
    #[must_use]
    pub fn with_default_metadata<K, V>(mut self, metadata: impl IntoIterator<Item = (K, V)>) -> Self
    where
        K: Into<String>,
        V: Into<String>,
    {
        self.metadata = metadata
            .into_iter()
            .map(|(name, value)| (name.into().trim().to_ascii_lowercase(), value.into()))
            .collect();
        self
    }

    /// Read environment variables under `prefix` as well as the usual ones.
    ///
    /// The name after the prefix is matched the way
    /// [`Self::with_properties`] matches one, so a deployment that spells its
    /// configuration `TRADING_ENDPOINT` and `TRADING_SSE_TYPE` gets every knob
    /// rather than the handful someone remembered to wire up.
    ///
    /// [`Self::with_properties`]: Self::with_properties
    #[must_use]
    pub fn with_environment_prefix(mut self, prefix: impl Into<String>) -> Self {
        let prefix: String = prefix.into();
        if !prefix.trim().is_empty() {
            self.environment_prefixes.push(prefix.trim().to_owned());
        }
        self
    }

    /// Read environment variables under exactly these prefixes.
    ///
    /// Replaces the defaults rather than adding to them, which is what a
    /// process that must not pick up an ambient `AWS_` needs. An empty list
    /// reads nothing from the environment by name, though
    /// [`Self::with_environment`] is the switch for reading none of it at all.
    #[must_use]
    pub fn with_environment_prefixes<P>(mut self, prefixes: impl IntoIterator<Item = P>) -> Self
    where
        P: Into<String>,
    {
        self.environment_prefixes = prefixes
            .into_iter()
            .map(Into::into)
            .filter(|prefix| !prefix.trim().is_empty())
            .map(|prefix| prefix.trim().to_owned())
            .collect();
        self
    }

    /// Consult, or ignore, the process environment and a store's own files.
    ///
    /// Off, only explicit values and the URL decide, which is what a test wants
    /// and what a sandboxed process may need.
    #[must_use]
    pub const fn with_environment(mut self, read_environment: bool) -> Self {
        self.read_environment = read_environment;
        self
    }

    /// Set the knobs that are Amazon S3's own.
    #[must_use]
    pub fn with_aws(mut self, options: AwsOptions) -> Self {
        self.aws = options;
        self
    }

    /// Set the knobs that are Google Cloud Storage's own.
    #[must_use]
    pub fn with_google(mut self, options: GoogleOptions) -> Self {
        self.google = options;
        self
    }

    /// Set the knobs that are Azure Blob Storage's own.
    #[must_use]
    pub fn with_azure(mut self, options: AzureOptions) -> Self {
        self.azure = options;
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

    /// Whether requests go unauthorized.
    pub const fn anonymous(&self) -> bool {
        self.anonymous
    }

    /// The explicit addressing style.
    pub const fn path_style(&self) -> Option<bool> {
        self.path_style
    }

    /// Bytes per part, as the caller asked for them.
    pub const fn part_size(&self) -> u64 {
        self.part_size
    }

    /// The value size from which a chunked upload is used.
    pub const fn multipart_threshold(&self) -> u64 {
        self.multipart_threshold
    }

    /// Entries per listing request, as the caller asked for them.
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

    /// Whether the environment and a store's own files are consulted.
    pub const fn reads_environment(&self) -> bool {
        self.read_environment
    }

    /// How writes are encrypted at rest.
    pub const fn encryption(&self) -> &Encryption {
        &self.encryption
    }

    /// The prefixes environment variables are read under.
    pub fn environment_prefixes(&self) -> &[String] {
        &self.environment_prefixes
    }

    /// The proxy the endpoint is reached through, when one was named.
    pub fn proxy(&self) -> Option<&str> {
        self.proxy.as_deref()
    }

    /// Whether a container handle may create a container.
    pub const fn container_creation(&self) -> bool {
        self.container_creation
    }

    /// Whether a container handle may delete a container.
    pub const fn container_deletion(&self) -> bool {
        self.container_deletion
    }

    /// The metadata every write carries.
    pub fn default_metadata(&self) -> &[(String, String)] {
        &self.metadata
    }

    /// The knobs that are Amazon S3's own.
    pub const fn aws(&self) -> &AwsOptions {
        &self.aws
    }

    /// The knobs that are Google Cloud Storage's own.
    pub const fn google(&self) -> &GoogleOptions {
        &self.google
    }

    /// The knobs that are Azure Blob Storage's own.
    pub const fn azure(&self) -> &AzureOptions {
        &self.azure
    }

    /// Whether a write to an endpoint of `scheme` signs its body.
    ///
    /// Explicit wins; otherwise TLS decides, because TLS is what the hash would
    /// otherwise be duplicating.
    pub(super) fn signs_payload(&self, scheme: &str) -> bool {
        self.aws
            .payload_signing()
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

impl std::fmt::Debug for ObjectOptions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ObjectOptions")
            .field("endpoint", &self.endpoint)
            .field("region", &self.region)
            // `Credentials` redacts its own secret.
            .field("credentials", &self.credentials)
            .field("anonymous", &self.anonymous)
            .field("path_style", &self.path_style)
            .field("part_size", &self.part_size)
            .field("multipart_threshold", &self.multipart_threshold)
            .field("list_page_size", &self.list_page_size)
            .field("max_attempts", &self.max_attempts)
            .field("timeout", &self.timeout)
            .field("connect_timeout", &self.connect_timeout)
            .field("read_environment", &self.read_environment)
            // `Encryption` redacts a customer key.
            .field("encryption", &self.encryption)
            .field("proxy", &self.proxy)
            .field("environment_prefixes", &self.environment_prefixes)
            .field("container_creation", &self.container_creation)
            .field("container_deletion", &self.container_deletion)
            .field("metadata", &self.metadata)
            .field("aws", &self.aws)
            .field("google", &self.google)
            // `AzureOptions` redacts its own secrets.
            .field("azure", &self.azure)
            .finish()
    }
}
