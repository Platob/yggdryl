//! One signed, pooled HTTP client, speaking whichever store's REST API answers.
//!
//! Every remote call the backend makes goes through [`Client`], and each of its
//! operations is exactly one request unless a retry or a region discovery adds
//! another. That is the whole point of putting them here: the request count of
//! a handle operation is readable from the operation it calls, and
//! [`Client::stats`] reports what actually went out.
//!
//! What is here is what every store shares: the connection pool, the signing
//! hook, the retry budget and its jittered backoff, the streaming reader that
//! resumes a transfer the network cut, the range arithmetic, and the request
//! accounting. What a store spells for itself - its hostnames, its request
//! paths, its upload protocol, its documents - is its dialect's, reached
//! through the one [`Provider`] value that says which store this is.

use std::io::Read;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime};

use base64::Engine as _;

use super::answer::{ListPage, S3Meta};
use super::aws::xml;
use super::encryption::Encryption;
use super::options::S3Options;
use super::provider::Provider;
use super::request::Request;
use crate::auth::{is_true, variable};
use crate::aws::sigv4::{self, Signer};
use crate::aws::{Credentials, Session};
use crate::{Error, Result, Url};

/// The region assumed when nothing names one; also the signing region for the
/// `GetBucketLocation`-free discovery a redirect performs.
const DEFAULT_REGION: &str = "us-east-1";
/// Base of the exponential backoff between attempts.
const RETRY_BACKOFF: Duration = Duration::from_millis(50);
/// The longest a retry ever waits, however many attempts precede it.
const RETRY_BACKOFF_CAP: Duration = Duration::from_secs(20);
/// The longest a `Retry-After` the store sent is honoured for.
///
/// A store under load may ask for minutes. Waiting that long inside a call
/// nobody can cancel is worse than failing and letting the caller decide, so
/// anything past this is treated as "not now" rather than as an instruction.
const RETRY_AFTER_CAP: Duration = Duration::from_secs(30);
/// Tokens a client starts with, and never exceeds.
const RETRY_TOKENS: i64 = 500;
/// What one retry costs, so a client whose requests are all failing runs out.
const RETRY_COST: i64 = 5;
/// What a first-attempt success refunds.
const RETRY_REFUND: i64 = 1;

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

/// A reading of an S3 client's request counters at one instant.
///
/// ```
/// use yggdryl::s3::StatsSnapshot;
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
    /// What is left of the retry budget.
    ///
    /// A client that is only failing spends this down and then stops retrying,
    /// so a falling number is the sign of a store in trouble rather than of a
    /// slow one. A [`Default`] snapshot reports zero because it describes no
    /// client at all.
    pub retry_tokens: i64,
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
            retry_tokens: 0,
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

/// Where the store is and how containers are addressed on it.
#[derive(Clone, Debug)]
struct Endpoint {
    /// `https` unless the endpoint said otherwise.
    scheme: String,
    /// The endpoint host, without a container and without a port.
    host: String,
    /// The explicit port, when the endpoint named one.
    port: Option<u16>,
    /// Whether the container goes in the path rather than in the hostname.
    path_style: bool,
    /// The Azure storage account, when one is addressed rather than a host.
    account: Option<String>,
    /// Whether the account is a path segment ahead of the container, which is
    /// how the Azure emulators address one.
    account_in_path: bool,
}

impl Endpoint {
    /// The `Host` header for a request against `container`.
    fn host_header(&self, container: &str) -> String {
        let host = if self.path_style {
            self.host.clone()
        } else {
            format!("{container}.{}", self.host)
        };
        match self.port {
            Some(port) => format!("{host}:{port}"),
            None => host,
        }
    }

    /// The request path for `container` and a raw `key`.
    ///
    /// A dialect whose operation does not live under the container's own path -
    /// Google's JSON API, whose objects sit below `/storage/v1` - builds the
    /// whole path itself and hands it over in [`Request::target`].
    fn path(&self, container: &str, key: &str) -> String {
        let mut path = String::new();
        if self.account_in_path {
            if let Some(account) = &self.account {
                path.push('/');
                path.push_str(&sigv4::encode_key(account));
            }
        }
        if self.path_style && !container.is_empty() {
            path.push('/');
            path.push_str(&sigv4::encode_key(container));
        }
        let key = sigv4::encode_key(key);
        if !key.is_empty() {
            path.push('/');
            path.push_str(&key);
        }
        if path.is_empty() {
            path.push('/');
        }
        path
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
    /// Which of the three stores answers, and so which dialect is spoken.
    provider: Provider,
    endpoint: Endpoint,
    /// The spelling the caller's location used - `s3`, `s3a`, or `s3n` - so a
    /// refusal names the location the handle reports rather than a canonical
    /// one the caller never wrote.
    scheme: crate::Scheme,
    /// The signing region, which a redirect can correct once.
    region: RwLock<String>,
    /// Who the AWS requests sign as: the caller's session, narrowed by what
    /// the options said explicitly.
    session: Session,
    /// The signer for the current credential set and region, rebuilt when
    /// either changes.
    signer: Mutex<Option<(Credentials, String, Arc<Signer>)>>,
    /// The bearer token Google's dialect authorizes with, obtained on the first
    /// request that needs one and refreshed shortly before it lapses.
    tokens: super::google::token::TokenCache,
    /// What Azure's dialect authorizes with: a signature, a token in the query,
    /// a bearer token, or nothing.
    azure: super::azure::auth::Authorization,
    options: S3Options,
    stats: Stats,
    /// What is left to spend on retries.
    retries: RetryBudget,
    /// The counter every jitter draw is taken from.
    jitter: AtomicU64,
}

impl Client {
    /// Build the client `url` and `options` describe, touching nothing.
    ///
    /// Endpoint, region, and addressing style are settled here from what the
    /// URL, the options and the session's files say; the credential chain is
    /// not walked until the first request needs to sign one.
    ///
    /// # Errors
    ///
    /// Returns a refusal when the URL's scheme names no store, when it names no
    /// container, or when an endpoint cannot be read as a location.
    pub(super) fn new(url: &Url, options: S3Options) -> Result<Self> {
        let provider = Provider::from_scheme(url.scheme()).ok_or_else(|| {
            Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "expected a location on an object store, got the scheme {:?}",
                    url.scheme().as_str()
                ),
            ))
        })?;
        // One credential pair serves all three stores, and on Azure it is an
        // account name and a shared key - but only when a caller handed it
        // over. A pair swept out of the environment arrived under an `AWS_`
        // name, so it names an account at another store, and addressing Azure
        // with it would send a request to a host nobody asked for.
        let handed = options.credentials().cloned();
        // Everything the environment names, under whatever the caller sets
        // for it, so one vocabulary covers a property map and a process
        // environment rather than each knob being wired up separately.
        let options = match options.reads_environment() {
            true => options.from_environment().map(|ambient| {
                let explicit = options.clone();
                explicit.under(&ambient)
            })?,
            false => options,
        };
        // A shape this store does not have is refused here rather than
        // silently dropped or discovered from the store on the first write.
        options.encryption().validate(provider)?;
        let session = Self::session_of(provider, url, &options);
        let tls = session.tls_config()?;
        let options = Self::under_profile(options, &session);
        let endpoint = Self::endpoint_of(provider, url, &options, &session, handed.as_ref())?;
        // The account the endpoint settled on is what a shared-key signature
        // names, so it is read back rather than resolved a second time.
        let endpoint_account = endpoint.account.clone();
        let region = Self::region_of(url, &options, &session);
        Ok(Self {
            agent: Self::agent(&options, tls),
            provider,
            endpoint,
            scheme: url.scheme().clone(),
            region: RwLock::new(region),
            session,
            signer: Mutex::new(None),
            tokens: super::google::token::TokenCache::new(
                options.google(),
                options.anonymous(),
                options.reads_environment(),
            )?,
            azure: super::azure::auth::Authorization::new(
                options.azure(),
                endpoint_account.as_deref(),
                handed.as_ref().map(Credentials::access_key_id),
                handed.as_ref().map(Credentials::secret_access_key),
                options.anonymous(),
            )?,
            options,
            stats: Stats::default(),
            retries: RetryBudget::default(),
            jitter: AtomicU64::new(fresh_jitter()),
        })
    }

    /// The connection pool this client sends on.
    ///
    /// A client whose transport matches the defaults shares one process-wide
    /// agent, so many handles against one store share connections rather than
    /// each opening its own.
    fn agent(options: &S3Options, tls: Option<ureq::tls::TlsConfig>) -> ureq::Agent {
        if !options.has_custom_transport() && tls.is_none() {
            return shared_agent().clone();
        }
        build_agent(options, tls)
    }

    /// Bytes per part, clamped to what this store accepts.
    ///
    /// The options record what the caller asked for; the store decides what is
    /// possible, and the three stores disagree - a 5 MiB floor on S3, a
    /// 256 KiB granularity on Google, a block on Azure. Clamping here rather
    /// than in the options is what lets one options value serve all three.
    pub(super) fn part_size(&self) -> u64 {
        self.options
            .part_size()
            .clamp(self.provider.min_part_size(), self.provider.max_part_size())
    }

    /// The value size from which a chunked upload is used, clamped to the
    /// largest single write this store takes.
    pub(super) fn multipart_threshold(&self) -> u64 {
        self.options
            .multipart_threshold()
            .min(self.provider.max_single_put())
    }

    /// Entries per listing page, clamped to what this store returns.
    pub(super) fn list_page_size(&self) -> u16 {
        self.options
            .list_page_size()
            .clamp(1, self.provider.max_list_page())
    }

    /// Keys per bulk delete on this store.
    pub(super) const fn delete_batch(&self) -> usize {
        self.provider.max_delete_batch()
    }

    /// The AWS session this client signs with: the caller's, narrowed by what
    /// the options say explicitly - anonymous, a credential pair, a pair the
    /// location carries - and sealed when the options consult no environment.
    ///
    /// A session left as it was is shared with every other client built on
    /// it, so the chain is walked and a role traded once for all of them.
    fn session_of(provider: Provider, url: &Url, options: &S3Options) -> Session {
        let mut session = options.session().clone();
        if !options.reads_environment() && session.reads_environment() {
            session = session.with_environment(false);
        }
        if !matches!(provider, Provider::Aws) {
            return session;
        }
        // The region the options state is the region a role is traded in.
        if let (Some(region), None) = (options.region(), session.stated_region()) {
            session = session.with_region(region);
        }
        if options.anonymous() {
            return session.with_anonymous(true);
        }
        if let Some(explicit) = options.credentials() {
            return session.with_credentials(explicit.clone());
        }
        if let Some(from_url) = Self::url_credentials(url) {
            return session.with_credentials(from_url);
        }
        session
    }

    /// The knobs the profile states for S3 that the caller left unset: the
    /// `s3` table's `payload_signing_enabled`, and `max_attempts`.
    fn under_profile(mut options: S3Options, session: &Session) -> S3Options {
        if options.aws().payload_signing().is_none() {
            if let Some(signing) = session
                .profile()
                .and_then(|profile| profile.s3("payload_signing_enabled").map(is_true))
            {
                let aws = options.aws().clone().with_payload_signing(signing);
                options = options.with_aws(aws);
            }
        }
        if options.max_attempts() == S3Options::default().max_attempts() {
            if let Some(attempts) = session.max_attempts() {
                options = options.with_max_attempts(attempts);
            }
        }
        options
    }

    /// The Azure storage account a location addresses, if any.
    ///
    /// Three places name one, in the order a caller means them: the Azure
    /// options, the location itself, and a credential pair the caller handed
    /// over - which on Azure is an account name and a shared key. `handed` is
    /// only what the caller set, never what the environment answered, because
    /// an ambient pair arrived under an `AWS_` name and names an account at
    /// another store. Every other store leaves this empty: only Azure writes
    /// an account into its host.
    pub(super) fn azure_account(
        provider: Provider,
        url: &Url,
        options: &S3Options,
        handed: Option<&Credentials>,
    ) -> Option<String> {
        if !matches!(provider, Provider::Azure) {
            return None;
        }
        options
            .azure()
            .account()
            .map(str::to_owned)
            .or_else(|| url.account().map(str::to_owned))
            .or_else(|| handed.map(|handed| handed.access_key_id().to_owned()))
    }

    /// The endpoint the URL and options name.
    ///
    /// The order is the same for every store - an explicit endpoint, then the
    /// URL's own, then the environment, then the store's published host - and
    /// only the last two steps know which store this is.
    fn endpoint_of(
        provider: Provider,
        url: &Url,
        options: &S3Options,
        session: &Session,
        handed: Option<&Credentials>,
    ) -> Result<Endpoint> {
        // An explicitly configured endpoint wins: it is a deliberate choice
        // about where the store is, where a URL only says which object. The
        // URL's own endpoint comes next, ahead of the environment, because it
        // is the location a caller handed over rather than a default.
        let explicit = options.endpoint().map(str::to_owned);
        let from_url = url.store_endpoint().map(str::to_owned);
        let ambient = Self::ambient_endpoint(provider, options, session);
        let account = Self::azure_account(provider, url, options, handed);
        let named = explicit.or(from_url).or(ambient);
        let (scheme, host, port) = match named {
            Some(endpoint) => Self::split_endpoint(&endpoint)?,
            None => Self::published_host(provider, url, options, session, account.as_deref())?,
        };
        let lowered = host.to_ascii_lowercase();
        let path_style = match provider {
            // AWS is addressed virtual-hosted, everything else path style,
            // because an S3-compatible store on a bare host rarely resolves
            // bucket subdomains. A bucket holding a dot would break TLS
            // wildcards either way, so it stays in the path.
            Provider::Aws => {
                let bucket_has_dot = url.bucket().is_some_and(|bucket| bucket.contains('.'));
                let aws =
                    lowered.ends_with(".amazonaws.com") || lowered.ends_with(".amazonaws.com.cn");
                options
                    .path_style()
                    .or_else(|| {
                        session
                            .variable("AWS_S3_FORCE_PATH_STYLE")
                            .map(|value| is_true(&value))
                    })
                    .or_else(|| {
                        match session
                            .profile()?
                            .s3("addressing_style")?
                            .to_ascii_lowercase()
                            .as_str()
                        {
                            "path" => Some(true),
                            "virtual" => Some(false),
                            _ => None,
                        }
                    })
                    .unwrap_or(!aws || bucket_has_dot)
            }
            // Google's JSON API addresses every bucket below one host, and
            // Azure's container is always a path segment. A caller who asks for
            // the other spelling gets it; nothing else does.
            Provider::Google | Provider::Azure => options.path_style().unwrap_or(true),
        };
        // An Azure endpoint that is not one of the published account hosts -
        // an emulator, or a gateway - names the account in the path instead,
        // which is what `az://127.0.0.1:10000/devstoreaccount1/lake` spells.
        let account_in_path = matches!(provider, Provider::Azure)
            && account.is_some()
            && !lowered.starts_with(&format!(
                "{}.",
                account.as_deref().unwrap_or_default().to_ascii_lowercase()
            ));
        Ok(Endpoint {
            scheme,
            host,
            port,
            path_style,
            account,
            account_in_path,
        })
    }

    /// The endpoint the environment and a store's own files name, and the
    /// one stated on the session or the Azure options where the options
    /// consult no environment.
    fn ambient_endpoint(
        provider: Provider,
        options: &S3Options,
        session: &Session,
    ) -> Option<String> {
        let from_environment = match provider {
            // `AWS_ENDPOINT_URL_S3`, `AWS_ENDPOINT_URL`, the profile's
            // `[services]` entry, then its `endpoint_url`: the session's
            // reading, which is the AWS tools' own - and on a session that
            // consults no environment, an endpoint stated on it and nothing
            // else, so the session is asked either way.
            Provider::Aws => return session.endpoint_url("s3"),
            // `STORAGE_EMULATOR_HOST` is what every Google client reads, and
            // the value is a bare host as often as a URL.
            Provider::Google if options.reads_environment() => variable("STORAGE_EMULATOR_HOST")
                .or_else(|| variable("GOOGLE_CLOUD_STORAGE_EMULATOR_HOST"))
                .or_else(|| variable("STORAGE_API_ENDPOINT")),
            Provider::Azure if options.reads_environment() => {
                variable("AZURE_STORAGE_BLOB_ENDPOINT")
                    .or_else(|| variable("AZURE_STORAGE_ENDPOINT"))
                    .or_else(|| {
                        variable("AZURE_STORAGE_CONNECTION_STRING").and_then(|text| {
                            super::azure::options::AzureOptions::default()
                                .with_connection_string(&text)
                                .endpoint()
                                .map(str::to_owned)
                        })
                    })
            }
            Provider::Google | Provider::Azure => None,
        };
        from_environment.or_else(|| options.azure().endpoint().map(str::to_owned))
    }

    /// The host the store publishes, when nothing named another.
    fn published_host(
        provider: Provider,
        url: &Url,
        options: &S3Options,
        session: &Session,
        account: Option<&str>,
    ) -> Result<(String, String, Option<u16>)> {
        let host = match provider {
            // The regional host, or the FIPS, dual-stack or accelerate one
            // the session's configuration asks for, on the partition the
            // region belongs to.
            Provider::Aws => {
                let region = Self::region_of(url, options, session);
                let profile = session.profile();
                let table = |key: &str| {
                    profile
                        .as_ref()
                        .and_then(|profile| profile.s3(key).map(is_true))
                        .unwrap_or(false)
                };
                let fips = session.use_fips_endpoint();
                let dualstack = session.use_dualstack_endpoint() || table("use_dualstack_endpoint");
                let suffix = if region.starts_with("cn-") {
                    "amazonaws.com.cn"
                } else {
                    "amazonaws.com"
                };
                match (table("use_accelerate_endpoint"), fips, dualstack) {
                    (true, _, true) => "s3-accelerate.dualstack.amazonaws.com".to_owned(),
                    (true, _, false) => "s3-accelerate.amazonaws.com".to_owned(),
                    (false, true, true) => format!("s3-fips.dualstack.{region}.{suffix}"),
                    (false, true, false) => format!("s3-fips.{region}.{suffix}"),
                    (false, false, true) => format!("s3.dualstack.{region}.{suffix}"),
                    (false, false, false) => format!("s3.{region}.{suffix}"),
                }
            }
            Provider::Google => "storage.googleapis.com".to_owned(),
            Provider::Azure => {
                let account = account.ok_or_else(|| {
                    Error::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "expected an Azure storage account: name one in the location, \
                         in the options, or in AZURE_STORAGE_ACCOUNT_NAME",
                    ))
                })?;
                let service = if options.azure().data_lake() {
                    "dfs"
                } else {
                    "blob"
                };
                format!("{account}.{service}.core.windows.net")
            }
        };
        Ok(("https".to_owned(), host, None))
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

    /// The signing region the URL, the options and the session name.
    fn region_of(url: &Url, options: &S3Options, session: &Session) -> String {
        options
            .region()
            .map(str::to_owned)
            .or_else(|| url.region().map(str::to_owned))
            // `AWS_REGION`, `AWS_DEFAULT_REGION`, then the profile's own,
            // and none of them when the options consult no environment.
            .or_else(|| session.region())
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

    /// The options this client was built with.
    /// Every counter, plus what is left of the retry budget.
    pub(super) fn snapshot(&self) -> StatsSnapshot {
        StatsSnapshot {
            retry_tokens: self.retries.remaining(),
            ..self.stats.snapshot()
        }
    }

    /// How this client encrypts what it writes.
    pub(super) const fn encryption(&self) -> &Encryption {
        self.options.encryption()
    }

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
        let Some(credentials) = self.session.credentials(now)? else {
            return Ok(None);
        };
        let region = self.region();
        let mut slot = self.signer.lock().map_err(|_| poisoned())?;
        if let Some((signed, signed_region, signer)) = slot.as_ref() {
            // The whole set, not the key alone: a refreshed session can keep
            // its access key id and change its secret or its token.
            if *signed == credentials && *signed_region == region {
                return Ok(Some(signer.clone()));
            }
        }
        let signer = Arc::new(Signer::new(
            credentials.access_key_id(),
            credentials.secret_access_key(),
            credentials.session_token().map(str::to_owned),
            &region,
        ));
        *slot = Some((credentials, region, signer.clone()));
        Ok(Some(signer))
    }

    /// Wait before the next attempt.
    ///
    /// A `Retry-After` is an instruction and is waited out as given. Anything
    /// else is a *window*, and the wait is drawn uniformly from it: doubling
    /// alone puts every client that failed at the same instant back on the
    /// wire at the same instant, which is the herd the backoff exists to
    /// prevent. The draw is a hash of a per-client counter rather than a
    /// random number generator - no dependency, no global state, and a
    /// sequence a test can predict.
    fn pause(&self, attempt: u32, asked: Option<Duration>) {
        let delay = match asked {
            Some(asked) => asked,
            None => {
                let window = backoff(attempt);
                let span = u64::try_from(window.as_nanos()).unwrap_or(u64::MAX);
                let draw =
                    crate::xxhash::xxh3(&self.jitter.fetch_add(1, Ordering::Relaxed).to_le_bytes());
                Duration::from_nanos(draw % span.saturating_add(1))
            }
        };
        std::thread::sleep(delay);
    }

    /// Re-open a body from `offset`, for a transfer that was cut.
    fn open_resumed(
        &self,
        bucket: &str,
        key: &str,
        offset: u64,
        last: Option<u64>,
    ) -> Result<Box<dyn Read + Send>> {
        self.open_reader_range(bucket, key, offset, last)
    }

    /// Whether another attempt is allowed, and pay for it if so.
    fn may_retry(&self, attempt: u32) -> bool {
        attempt < self.options.max_attempts() && self.retries.withdraw()
    }

    /// Give back what an exchange that reached a verdict is owed.
    fn settle(&self, answer: &Answer, attempt: u32) {
        if answer.status >= 500 || answer.status == 429 {
            return;
        }
        self.retries.refund(if attempt == 1 {
            RETRY_REFUND
        } else {
            RETRY_COST
        });
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
        let mut refreshed = false;
        loop {
            attempt += 1;
            if attempt > 1 {
                self.stats.retries.fetch_add(1, Ordering::Relaxed);
            }
            // Who signs is settled before the attempt, so a credential
            // refusal is its own typed failure rather than a transport
            // failure retried with backoff.
            let (target, headers) = self.prepare(request, Some(request.body), SystemTime::now())?;
            let outcome = self.attempt(request, &target, &headers);
            let answer = match outcome {
                Ok(answer) => answer,
                Err(error) => {
                    if is_retryable_transport(&error) && self.may_retry(attempt) {
                        self.pause(attempt, None);
                        continue;
                    }
                    return Err(transport_failure(
                        self.provider.service(),
                        request,
                        self.location(request),
                        error,
                    ));
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
            if self.refresh_on_expiry(&answer, &mut refreshed)? {
                continue;
            }
            if (answer.status >= 500 || answer.status == 429) && self.may_retry(attempt) {
                self.pause(attempt, retry_after(&answer));
                continue;
            }
            self.settle(&answer, attempt);
            return Ok(answer);
        }
    }

    /// Whether a refusal says the credential set the request was signed with
    /// has lapsed - a session the store knows expired before the session
    /// thought it would - in which case the chain is walked again and the
    /// request signed once more, once.
    fn refresh_on_expiry(&self, answer: &Answer, refreshed: &mut bool) -> Result<bool> {
        if *refreshed
            || !matches!(self.provider, Provider::Aws)
            || !matches!(answer.status, 400 | 403)
        {
            return Ok(false);
        }
        let Some(code) = super::xml::parse_error(&answer.body).map(|error| error.code) else {
            return Ok(false);
        };
        if !matches!(
            code.as_str(),
            "ExpiredToken" | "ExpiredTokenException" | "InvalidToken" | "TokenRefreshRequired"
        ) {
            return Ok(false);
        }
        *refreshed = true;
        let mut signer = self.signer.lock().map_err(|_| poisoned())?;
        // Only the set this request was signed with is forgotten: another
        // client on the same session may already hold a fresh one.
        match signer.as_ref() {
            Some((signed, _, _)) => {
                self.session.invalidate_if(signed.access_key_id());
            }
            None => self.session.invalidate(),
        }
        *signer = None;
        Ok(true)
    }

    /// The wire target and the headers one attempt goes out with.
    ///
    /// Every request crosses here, so this is the one place a store's identity
    /// reaches the wire: a Signature Version 4 header, a bearer token, an Azure
    /// shared-key signature, or a token already in the query. The dialect's own
    /// headers are set first, so authorization can never be shadowed by one.
    fn prepare(
        &self,
        request: &Request<'_>,
        payload: Option<&[u8]>,
        now: SystemTime,
    ) -> Result<(String, Vec<(String, String)>)> {
        // A store that handed a whole location back owns it entirely, so it is
        // sent as it stands rather than rebuilt from the endpoint.
        if let Some(url) = &request.url {
            let (host, path) = split_url(url);
            let mut headers = request.headers.clone();
            headers.extend(self.authorize(request, &host, &path, payload, now)?);
            return Ok((url.clone(), headers));
        }
        let host = self.endpoint.host_header(&request.bucket);
        let path = request
            .target
            .clone()
            .unwrap_or_else(|| self.endpoint.path(&request.bucket, &request.key));
        let mut query = sigv4::canonical_query(&request.query);
        // A shared access signature is already a signature over the request, so
        // it rides in the query and nothing signs it again.
        if let Some(token) = self.azure.query_suffix() {
            if query.is_empty() {
                query = token.to_owned();
            } else {
                query.push('&');
                query.push_str(token);
            }
        }
        let target = if query.is_empty() {
            format!("{}://{host}{path}", self.endpoint.scheme)
        } else {
            format!("{}://{host}{path}?{query}", self.endpoint.scheme)
        };
        let mut headers = request.headers.clone();
        headers.extend(self.authorize(request, &host, &path, payload, now)?);
        Ok((target, headers))
    }

    /// The headers that say who is asking, in this store's own terms.
    fn authorize(
        &self,
        request: &Request<'_>,
        host: &str,
        path: &str,
        payload: Option<&[u8]>,
        now: SystemTime,
    ) -> Result<Vec<(String, String)>> {
        match self.provider {
            Provider::Aws => {
                let Some(signer) = self.signer(now)? else {
                    return Ok(Vec::new());
                };
                // Hashing a large body costs more than the rest of the request
                // put together, so it is only done where it buys something.
                let hash = match payload {
                    None | Some([]) => sigv4::EMPTY_PAYLOAD_SHA256.to_owned(),
                    Some(body) if self.options.signs_payload(&self.endpoint.scheme) => {
                        sigv4::sha256_hex(body)
                    }
                    Some(_) => sigv4::UNSIGNED_PAYLOAD.to_owned(),
                };
                Ok(signer.sign(
                    request.method,
                    host,
                    path,
                    &request.query,
                    &request.headers,
                    &hash,
                    now,
                ))
            }
            Provider::Google => {
                let Some(token) = self.tokens.resolve_token(&self.agent, now)? else {
                    return Ok(Vec::new());
                };
                Ok(vec![(
                    "authorization".to_owned(),
                    format!("Bearer {}", token.value()),
                )])
            }
            Provider::Azure => self.azure.headers(
                &self.agent,
                request.method,
                path,
                &request.query,
                &request.headers,
                now,
            ),
        }
    }

    /// Sign and send one attempt.
    fn attempt(
        &self,
        request: &Request<'_>,
        target: &str,
        headers: &[(String, String)],
    ) -> std::result::Result<Answer, ureq::Error> {
        self.stats.record(request.method);
        let mut wire = ureq::http::Request::builder()
            .method(request.method)
            .uri(target);
        for (name, value) in headers {
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
        let mut refreshed = false;
        loop {
            attempt += 1;
            if attempt > 1 {
                self.stats.retries.fetch_add(1, Ordering::Relaxed);
            }
            let (target, headers) = self.prepare(request, None, SystemTime::now())?;
            let opened = self.open_stream(request, &target, &headers);
            let (status, headers, mut reader) = match opened {
                Ok(opened) => opened,
                Err(error) => {
                    if is_retryable_transport(&error) && self.may_retry(attempt) {
                        self.pause(attempt, None);
                        continue;
                    }
                    return Err(transport_failure(
                        self.provider.service(),
                        request,
                        self.location(request),
                        error,
                    ));
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
                if self.refresh_on_expiry(&answer, &mut refreshed)? {
                    continue;
                }
                if (answer.status >= 500 || answer.status == 429) && self.may_retry(attempt) {
                    self.pause(attempt, retry_after(&answer));
                    continue;
                }
                self.settle(&answer, attempt);
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
    fn open_stream(
        &self,
        request: &Request<'_>,
        target: &str,
        headers: &[(String, String)],
    ) -> std::result::Result<Streamed, ureq::Error> {
        self.stats.record(request.method);
        let mut wire = ureq::http::Request::builder()
            .method(request.method)
            .uri(target);
        for (name, value) in headers {
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
        let scheme = self.scheme.as_str();
        if request.key.is_empty() {
            format!("{scheme}://{}/", request.bucket)
        } else {
            format!("{scheme}://{}/{}", request.bucket, request.key)
        }
    }

    /// Turn a non-2xx answer into the typed failure it means.
    fn failure(&self, request: &Request<'_>, answer: &Answer) -> Error {
        let path = self.location(request);
        let body = match self.provider {
            Provider::Aws | Provider::Azure => super::xml::parse_error(&answer.body),
            Provider::Google => super::google::json::parse_error(&answer.body),
        };
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
                self.provider.service(),
                request.operation,
                answer.status,
                code,
                message,
                path,
            ),
        }
    }

    /// The request that reads one object's bytes.
    ///
    /// Two of the three stores address an object by its own path; Google's JSON
    /// API puts it below `/storage/v1` and switches metadata for bytes with a
    /// query parameter, so there the path is the dialect's to build.
    fn get_request<'body>(&self, bucket: &str, key: &str) -> Request<'body> {
        let request = match self.provider {
            Provider::Aws | Provider::Azure => Request::new("GET", "GetObject", bucket, key),
            Provider::Google => super::google::dialect::get_request(bucket, key),
        };
        self.common(request).keyed(self.provider, self.encryption())
    }

    /// What every request this client sends carries, whatever the operation.
    fn common<'body>(&self, request: Request<'body>) -> Request<'body> {
        let mut request = request;
        match self.provider {
            Provider::Aws => {
                if self.options.aws().requester_pays() {
                    request = request.header("x-amz-request-payer", "requester");
                }
            }
            Provider::Google => {
                for (name, value) in super::google::dialect::common_query(&self.options) {
                    request = request.query(&name, value);
                }
            }
            // Azure versions its API by header, and a stored value is a
            // contract: every request states which version it was written for.
            Provider::Azure => {
                request = request.header("x-ms-version", self.options.azure().api_version());
            }
        }
        request
    }

    /// Read one object's metadata.
    ///
    /// One `HEAD`. Absence answers `None` rather than failing, per the
    /// laziness contract.
    ///
    /// # Errors
    ///
    /// Returns the store's refusal for anything that is not a 404.
    pub(super) fn head_object(&self, bucket: &str, key: &str) -> Result<Option<S3Meta>> {
        let request = match self.provider {
            Provider::Aws | Provider::Azure => self
                .common(Request::new("HEAD", "HeadObject", bucket, key))
                .keyed(self.provider, self.encryption()),
            Provider::Google => self.common(super::google::dialect::head_request(bucket, key)),
        };
        let answer = self.send(&request)?;
        if answer.status == 404 {
            return Ok(None);
        }
        if answer.status >= 300 {
            return Err(self.failure(&request, &answer));
        }
        // Two of the three answer the metadata in headers; Google answers a
        // JSON resource, whose `size` is a string because it is 64-bit.
        if matches!(self.provider, Provider::Google) {
            return super::google::json::parse_object(&answer.body)
                .map(Some)
                .map_err(|error| {
                    malformed(
                        self.provider.service(),
                        request.operation,
                        &self.location(&request),
                        &error.0,
                    )
                });
        }
        Ok(Some(S3Meta {
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
        let length = buffer.len() as u64;
        let Some((mut reader, window)) = self.open_range(bucket, key, offset, length)? else {
            return Ok((0, Some(0)));
        };
        if !discard(&mut reader, window.skip)? {
            return Ok((0, window.total));
        }
        let mut filled = 0;
        while filled < buffer.len() {
            let read = reader.read(&mut buffer[filled..]).map_err(Error::Io)?;
            if read == 0 {
                break;
            }
            filled += read;
        }
        drain(&mut reader);
        Ok((filled, window.total))
    }

    /// Read the window `length` bytes wide at `offset` into a fresh buffer.
    ///
    /// One ranged `GET`, allocating what the answer says is coming rather than
    /// what was asked for. That distinction is the whole point of this method:
    /// a caller may legitimately ask for more than exists - the rest of an
    /// object whose length it does not know, or a footer window larger than
    /// the object - and sizing the buffer from the request would let a
    /// four-word call allocate gigabytes.
    ///
    /// # Errors
    ///
    /// Returns the store's refusal, or a read failure part way through.
    pub(super) fn get_range_vec(
        &self,
        bucket: &str,
        key: &str,
        offset: u64,
        length: u64,
    ) -> Result<(Vec<u8>, Option<u64>)> {
        if length == 0 {
            return Ok((Vec::new(), None));
        }
        let Some((mut reader, window)) = self.open_range(bucket, key, offset, length)? else {
            return Ok((Vec::new(), Some(0)));
        };
        if !discard(&mut reader, window.skip)? {
            return Ok((Vec::new(), window.total));
        }
        let coming = window.length.map_or(length, |answered| {
            answered.saturating_sub(window.skip).min(length)
        });
        let capacity = usize::try_from(coming).map_err(|_| crate::iobase::oversized(coming))?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(capacity)
            .map_err(|_| crate::iobase::oversized(coming))?;
        (&mut reader)
            .take(coming)
            .read_to_end(&mut bytes)
            .map_err(Error::Io)?;
        drain(&mut reader);
        Ok((bytes, window.total))
    }

    /// Issue one ranged `GET` and hand back the body the window is read from.
    ///
    /// `None` is absence, which reads as emptiness. A range wholly past the
    /// end comes back as an empty body rather than as absence, because the
    /// answer still states the object's length.
    fn open_range(
        &self,
        bucket: &str,
        key: &str,
        offset: u64,
        length: u64,
    ) -> Result<Option<(Box<dyn Read + Send>, Window)>> {
        let last = offset.saturating_add(length - 1);
        let request = self
            .get_request(bucket, key)
            .header("range", format!("bytes={offset}-{last}"));
        let (status, headers, mut reader) = self.stream(&request)?;
        let answer = Answer {
            status,
            headers,
            body: Vec::new(),
        };
        match status {
            404 => return Ok(None),
            416 => {
                return Ok(Some((
                    Box::new(std::io::empty()),
                    Window {
                        total: total_of_content_range(answer.header("content-range")),
                        skip: 0,
                        length: Some(0),
                    },
                )));
            }
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
        let stated = answer
            .header("content-length")
            .and_then(|value| value.trim().parse::<u64>().ok());
        // A 200 answers the whole object, so its length is the total, and the
        // caller's window has to be found inside what arrived.
        let total = total_of_content_range(answer.header("content-range"))
            .or_else(|| (status == 200).then_some(stated).flatten());
        let skip = if status == 200 { offset } else { 0 };
        Ok(Some((
            reader,
            Window {
                total,
                skip,
                length: stated,
            },
        )))
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
        let request = self.get_request(bucket, key);
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

    /// The same, over a body that re-opens itself if the transfer dies.
    ///
    /// This is what a long read wants. The retry in [`Self::stream`] covers
    /// only the exchange up to the status: once the body is the caller's, a
    /// connection that dies half way through a gigabyte fails the whole
    /// transfer, and everything already delivered has to be read again. Here
    /// it does not: see [`Resuming`].
    ///
    /// # Errors
    ///
    /// Returns the store's refusal to open the stream at all.
    pub(super) fn open_resuming_reader(
        self: &Arc<Self>,
        bucket: &str,
        key: &str,
        offset: u64,
        last: Option<u64>,
    ) -> Result<Box<dyn Read + Send>> {
        let reader = self.open_reader_range(bucket, key, offset, last)?;
        Ok(Box::new(Resuming {
            client: Arc::clone(self),
            bucket: bucket.to_owned(),
            key: key.to_owned(),
            start: offset,
            last,
            delivered: 0,
            failures: 0,
            reader,
        }))
    }

    /// Open one object as a reader over `offset ..= last`, or to its end.
    ///
    /// One `GET` that asks for exactly the window wanted. Bounding it matters
    /// on a store: a caller hashing the first sixteen bytes of a gigabyte
    /// object should not have the store start sending the gigabyte.
    ///
    /// The reader returns its connection to the pool when it is dropped part
    /// way through, so a scan that reads a header out of each of a thousand
    /// objects reuses one connection rather than opening a thousand.
    ///
    /// # Errors
    ///
    /// Returns the store's refusal.
    pub(super) fn open_reader_range(
        &self,
        bucket: &str,
        key: &str,
        offset: u64,
        last: Option<u64>,
    ) -> Result<Box<dyn Read + Send>> {
        let mut request = self.get_request(bucket, key);
        match last {
            Some(last) => request = request.header("range", format!("bytes={offset}-{last}")),
            None if offset > 0 => request = request.header("range", format!("bytes={offset}-")),
            None => {}
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
        let stated = Answer {
            status,
            headers,
            body: Vec::new(),
        }
        .header("content-length")
        .and_then(|value| value.trim().parse::<u64>().ok());
        // A store that ignored the range answered from zero.
        let skip = if status == 200 { offset } else { 0 };
        if !discard(&mut reader, skip)? {
            return Ok(Box::new(std::io::empty()));
        }
        Ok(Box::new(Pooled {
            inner: reader,
            left: stated.map_or(u64::MAX, |stated| stated.saturating_sub(skip)),
        }))
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
        let content_type = content_type.unwrap_or("application/octet-stream");
        // Google carries the object's metadata in the same request as its
        // bytes, which is what `multipart/related` is for; the other two put
        // both in headers.
        let google = matches!(self.provider, Provider::Google);
        let (body, boundary) = if google {
            let metadata = self.google_metadata(key, content_type)?;
            let boundary = super::google::dialect::boundary(bytes);
            (
                super::google::dialect::multipart_body(&metadata, content_type, bytes, &boundary),
                boundary,
            )
        } else {
            (Vec::new(), String::new())
        };
        let mut request = if google {
            super::google::dialect::put_request(bucket, key, &body, &boundary)
        } else {
            let mut request = Request::new("PUT", "PutObject", bucket, key)
                .body(bytes)
                .header("content-type", content_type);
            if matches!(self.provider, Provider::Azure) {
                request = request
                    .header("x-ms-blob-type", self.options.azure().blob_type().as_str())
                    .header("content-length", bytes.len().to_string());
                if let Some(tier) = self.options.azure().access_tier() {
                    request = request.header("x-ms-access-tier", tier);
                }
            }
            request.with_metadata(self.provider, self.options.default_metadata())
        };
        request = self
            .common(request)
            .storing(self.provider, self.encryption());
        if let (Provider::Aws, Some(class)) = (self.provider, self.options.aws().storage_class()) {
            request = request.header("x-amz-storage-class", class);
        }
        // S3 computes the checksum the request names, verifies what it received
        // against it, and keeps it so a later read can be checked without
        // transferring the object again.
        if let (Provider::Aws, Some(checksum)) = (self.provider, self.options.aws().checksum()) {
            request = request
                .header("x-amz-sdk-checksum-algorithm", checksum.as_str())
                .header(checksum.header(), checksum.of(bytes));
        }
        if let (Provider::Google, Some(class)) =
            (self.provider, self.options.google().storage_class())
        {
            request = request.query("storageClass", class);
        }
        let answer = self.send(&request)?;
        if answer.status >= 300 {
            return Err(self.failure(&request, &answer));
        }
        Ok(answer.header("etag").map(str::to_owned))
    }

    /// The JSON resource a Google write sends beside its bytes.
    fn google_metadata(&self, key: &str, content_type: &str) -> Result<String> {
        let mut headers: Vec<(String, String)> = self
            .options
            .default_metadata()
            .iter()
            .map(|(name, value)| {
                (
                    super::request::header_name(self.provider, name),
                    value.clone(),
                )
            })
            .collect();
        headers.push(("content-type".to_owned(), content_type.to_owned()));
        super::google::json::object_metadata(key, &headers)
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
        let request = match self.provider {
            Provider::Aws | Provider::Azure => {
                self.common(Request::new("DELETE", "DeleteObject", bucket, key))
            }
            Provider::Google => self.common(super::google::dialect::delete_request(bucket, key)),
        };
        let answer = self.send(&request)?;
        if answer.status == 404 || answer.status < 300 {
            return Ok(());
        }
        Err(self.failure(&request, &answer))
    }

    /// Delete up to the store's batch maximum of objects in one request.
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
        match self.provider {
            Provider::Aws => self.delete_objects_xml(bucket, keys),
            // Google and Azure both batch whole sub-requests into one
            // `multipart/mixed` body rather than taking a list of keys.
            Provider::Google | Provider::Azure => self.delete_objects_batched(bucket, keys),
        }
    }

    /// S3's own bulk delete: one document naming every key.
    fn delete_objects_xml(&self, bucket: &str, keys: &[String]) -> Result<()> {
        let document = xml::render_delete_objects(keys, true);
        let bytes = document.as_bytes();
        let digest = base64::engine::general_purpose::STANDARD.encode(md5_of(bytes));
        let request = self
            .common(Request::new("POST", "DeleteObjects", bucket, ""))
            .query("delete", String::new())
            .header("content-md5", digest)
            .header("content-type", "application/xml")
            .body(bytes);
        let answer = self.send(&request)?;
        if answer.status >= 300 {
            return Err(self.failure(&request, &answer));
        }
        let failures = xml::parse_delete_result(&answer.body).map_err(|error| {
            malformed(
                self.provider.service(),
                request.operation,
                &self.location(&request),
                &error.0,
            )
        })?;
        match failures.first() {
            None => Ok(()),
            Some(failure) => Err(Error::remote(
                self.provider.service(),
                "DeleteObjects",
                answer.status,
                &failure.code,
                &failure.message,
                format!("{}://{bucket}/{}", self.scheme.as_str(), failure.key),
            )),
        }
    }

    /// A batch of whole sub-requests, which is how the other two spell it.
    ///
    /// The answer is a multipart document of sub-answers; a key that was not
    /// there is a success, as it is for a single delete, so only a status that
    /// says something else went wrong is reported.
    fn delete_objects_batched(&self, bucket: &str, keys: &[String]) -> Result<()> {
        let boundary = match self.provider {
            Provider::Google => super::google::dialect::boundary(bucket.as_bytes()),
            _ => super::azure::dialect::batch_boundary(bucket.as_bytes()),
        };
        let body = match self.provider {
            Provider::Google => super::google::dialect::batch_body(bucket, keys, &boundary),
            Provider::Azure => {
                let now = SystemTime::now();
                super::azure::dialect::batch_body(
                    self.endpoint.account.as_deref().unwrap_or_default(),
                    bucket,
                    keys,
                    self.options.azure().api_version(),
                    &boundary,
                    self.endpoint.account_in_path,
                    |path, headers| {
                        self.azure
                            .headers(&self.agent, "DELETE", path, &[], headers, now)
                            .unwrap_or_default()
                    },
                )
            }
            Provider::Aws => return Err(self.provider.unsupported("a batched delete")),
        };
        let request = match self.provider {
            Provider::Google => self.common(super::google::dialect::batch_request(
                bucket, &body, &boundary,
            )),
            _ => self.common(super::azure::dialect::batch_request(
                bucket, &body, &boundary,
            )),
        };
        let answer = self.send(&request)?;
        if answer.status >= 300 {
            return Err(self.failure(&request, &answer));
        }
        // A sub-answer that failed for a reason other than absence is the one
        // thing worth reporting; the batch itself succeeded.
        let text = String::from_utf8_lossy(&answer.body);
        for line in text.lines() {
            let Some(status) = line.strip_prefix("HTTP/1.1 ") else {
                continue;
            };
            let code: u16 = status
                .split_whitespace()
                .next()
                .and_then(|code| code.parse().ok())
                .unwrap_or(200);
            if code >= 300 && code != 404 {
                return Err(Error::remote(
                    self.provider.service(),
                    "DeleteObjects",
                    code,
                    status_code_name(code),
                    "a batched delete was refused",
                    self.location(&request),
                ));
            }
        }
        Ok(())
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
    ) -> Result<ListPage> {
        let request = match self.provider {
            Provider::Aws => {
                let mut request = Request::new("GET", "ListObjectsV2", bucket, "")
                    .query("list-type", "2")
                    .query("max-keys", max_keys.to_string())
                    // Keys are arbitrary UTF-8, and a control character would
                    // make the page malformed XML; asking for URL encoding
                    // keeps every key readable and the document well-formed.
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
                request
            }
            Provider::Google => super::google::dialect::list_request(
                bucket,
                prefix,
                delimiter,
                continuation,
                max_keys,
            ),
            Provider::Azure => super::azure::dialect::list_request(
                bucket,
                prefix,
                delimiter,
                continuation,
                max_keys,
            ),
        };
        let request = self.common(request);
        self.stats.lists.fetch_add(1, Ordering::Relaxed);
        let answer = self.send(&request)?;
        if answer.status >= 300 {
            return Err(self.failure(&request, &answer));
        }
        match self.provider {
            Provider::Aws => xml::parse_list_objects(&answer.body),
            Provider::Google => super::google::json::parse_list(&answer.body),
            Provider::Azure => super::azure::xml::parse_list(&answer.body),
        }
        .map_err(|error| {
            malformed(
                self.provider.service(),
                request.operation,
                &self.location(&request),
                &error.0,
            )
        })
    }

    /// Write one large object in chunks, in whatever shape this store has.
    ///
    /// Three stores, three protocols for the same thing. S3 creates an upload,
    /// sends numbered parts, and completes it from their entity tags: `parts +
    /// 2` requests. Google opens a resumable session and sends chunks into it,
    /// each stating the byte range it carries: `chunks + 1`. Azure stages
    /// blocks under ids it chooses and commits the list: `blocks + 1`. What
    /// they share is the reason for doing it at all - a failure re-sends one
    /// chunk rather than the whole value.
    ///
    /// # Errors
    ///
    /// Returns the store's refusal. A failure part way through abandons what
    /// was staged where the store has a way to, so parts are not billed
    /// forever; the original failure is what the caller hears about.
    pub(super) fn put_chunked(
        &self,
        bucket: &str,
        key: &str,
        bytes: &[u8],
        content_type: &str,
    ) -> Result<Option<String>> {
        let mut source = std::io::Cursor::new(bytes);
        self.put_streamed(bucket, key, &mut source, bytes.len() as u64, content_type)
    }

    /// Write one large object in chunks read from `source` as they go out.
    ///
    /// [`Self::put_chunked`] with the value streamed rather than held: one
    /// part-sized buffer is filled from the source and sent before the next
    /// is read, so a value of any length costs one part of memory. `length`
    /// is what the source holds, and a source that ends before it is
    /// refused - with the upload abandoned where the store has a way to.
    ///
    /// # Errors
    ///
    /// Returns the source's read failure, a short source, or the store's
    /// refusal.
    pub(super) fn put_streamed(
        &self,
        bucket: &str,
        key: &str,
        source: &mut dyn Read,
        length: u64,
        content_type: &str,
    ) -> Result<Option<String>> {
        let part_size = usize::try_from(self.part_size())
            .map_err(|_| crate::iobase::oversized(self.part_size()))?;
        match self.provider {
            Provider::Aws => {
                self.put_multipart(bucket, key, source, length, content_type, part_size)
            }
            Provider::Google => {
                self.put_resumable(bucket, key, source, length, content_type, part_size)
            }
            Provider::Azure => {
                self.put_blocks(bucket, key, source, length, content_type, part_size)
            }
        }
    }

    /// S3's shape: create, send numbered parts, complete from their tags.
    fn put_multipart(
        &self,
        bucket: &str,
        key: &str,
        source: &mut dyn Read,
        length: u64,
        content_type: &str,
        part_size: usize,
    ) -> Result<Option<String>> {
        let upload = self.create_multipart(bucket, key, Some(content_type))?;
        let mut pending = Parts::new(source, length, part_size);
        let mut buffer = Vec::new();
        let mut parts = Vec::new();
        let mut number = 0_u32;
        let sent = loop {
            match pending.next(&mut buffer) {
                Ok(true) => {}
                Ok(false) => break Ok(()),
                Err(error) => break Err(error),
            }
            let Some(next) = number.checked_add(1) else {
                break Err(too_many_parts());
            };
            number = next;
            match self.upload_part(bucket, key, &upload, number, &buffer) {
                Ok(etag) => parts.push((number, etag)),
                Err(error) => break Err(error),
            }
        };
        if let Err(error) = sent {
            let _ = self.abort_multipart(bucket, key, &upload);
            return Err(error);
        }
        match self.complete_multipart(bucket, key, &upload, &parts) {
            Ok(etag) => Ok(etag),
            Err(error) => {
                let _ = self.abort_multipart(bucket, key, &upload);
                Err(error)
            }
        }
    }

    /// Google's shape: one session, then chunks that state their own range.
    ///
    /// Every chunk but the last is a multiple of 256 KiB, which is the one
    /// framing rule the protocol has; the answer to the last one is the object.
    fn put_resumable(
        &self,
        bucket: &str,
        key: &str,
        source: &mut dyn Read,
        length: u64,
        content_type: &str,
        part_size: usize,
    ) -> Result<Option<String>> {
        let granularity =
            usize::try_from(super::google::dialect::CHUNK_GRANULARITY).unwrap_or(usize::MAX);
        let part_size = (part_size / granularity).max(1) * granularity;
        let total = length;
        let metadata = self.google_metadata(key, content_type)?;
        let request = self.common(super::google::dialect::initiate_request(
            bucket,
            key,
            metadata.as_bytes(),
            content_type,
            total,
        ));
        let answer = self.send(&request)?;
        if answer.status >= 300 {
            return Err(self.failure(&request, &answer));
        }
        let session = answer
            .header("location")
            .map(str::to_owned)
            .ok_or_else(|| {
                malformed(
                    self.provider.service(),
                    request.operation,
                    &self.location(&request),
                    &super::google::json::missing_session().0,
                )
            })?;
        let mut pending = Parts::new(source, length, part_size);
        let mut buffer = Vec::new();
        let mut start = 0_u64;
        loop {
            match pending.next(&mut buffer) {
                Ok(true) => {}
                Ok(false) => break,
                Err(error) => {
                    let _ = self.send(&super::google::dialect::cancel_request(
                        &session, bucket, key,
                    ));
                    return Err(error);
                }
            }
            let request =
                super::google::dialect::chunk_request(&session, bucket, key, &buffer, start, total);
            let answer = self.send(&request)?;
            // 308 is the store saying the chunk landed and more is expected;
            // it is the protocol's own use of the code, not a redirect.
            if answer.status != 308 && answer.status >= 300 {
                let _ = self.send(&super::google::dialect::cancel_request(
                    &session, bucket, key,
                ));
                return Err(self.failure(&request, &answer));
            }
            start += buffer.len() as u64;
        }
        Ok(None)
    }

    /// Azure's shape: stage blocks under ids of one width, then commit them.
    fn put_blocks(
        &self,
        bucket: &str,
        key: &str,
        source: &mut dyn Read,
        length: u64,
        content_type: &str,
        part_size: usize,
    ) -> Result<Option<String>> {
        let mut pending = Parts::new(source, length, part_size);
        let mut buffer = Vec::new();
        let mut ids = Vec::new();
        let mut number = 0_u32;
        while pending.next(&mut buffer)? {
            let id = super::azure::dialect::block_id(number);
            let request = self
                .common(super::azure::dialect::put_block_request(
                    bucket, key, &id, &buffer,
                ))
                .keyed(self.provider, self.encryption());
            let answer = self.send(&request)?;
            if answer.status >= 300 {
                return Err(self.failure(&request, &answer));
            }
            ids.push(id);
            number = number.checked_add(1).ok_or_else(too_many_parts)?;
        }
        // Nothing is committed until the list is, so an abandoned upload leaves
        // uncommitted blocks the account's own rule expires; there is no abort.
        let document = super::azure::xml::render_block_list(&ids);
        let mut request = self
            .common(super::azure::dialect::put_block_list_request(
                bucket,
                key,
                document.as_bytes(),
            ))
            .storing(self.provider, self.encryption())
            .header("x-ms-blob-content-type", content_type)
            .with_metadata(self.provider, self.options.default_metadata());
        if let Some(tier) = self.options.azure().access_tier() {
            request = request.header("x-ms-access-tier", tier);
        }
        let answer = self.send(&request)?;
        if answer.status >= 300 {
            return Err(self.failure(&request, &answer));
        }
        Ok(answer.header("etag").map(str::to_owned))
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
            .storing(self.provider, self.encryption())
            .query("uploads", String::new());
        if let Some(content_type) = content_type {
            request = request.header("content-type", content_type);
        }
        request = request.with_metadata(self.provider, self.options.default_metadata());
        let answer = self.send(&request)?;
        if answer.status >= 300 {
            return Err(self.failure(&request, &answer));
        }
        xml::parse_upload_id(&answer.body).map_err(|error| {
            malformed(
                self.provider.service(),
                request.operation,
                &self.location(&request),
                &error.0,
            )
        })
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
        // A part inherits how the upload was created, so it says nothing
        // about that - but a customer key is not kept, so it says that.
        let request = Request::new("PUT", "UploadPart", bucket, key)
            .keyed(self.provider, self.encryption())
            .query("partNumber", part.to_string())
            .query("uploadId", upload)
            .body(bytes);
        let answer = self.send(&request)?;
        if answer.status >= 300 {
            return Err(self.failure(&request, &answer));
        }
        answer.header("etag").map(str::to_owned).ok_or_else(|| {
            malformed(
                self.provider.service(),
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
                self.provider.service(),
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
        let request = match self.provider {
            Provider::Aws => self.common(Request::new("HEAD", "HeadBucket", bucket, "")),
            Provider::Google => self.common(super::google::dialect::head_bucket_request(bucket)),
            Provider::Azure => self.common(super::azure::dialect::head_container_request(bucket)),
        };
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
        let document = match self.provider {
            Provider::Aws if region == DEFAULT_REGION => String::new(),
            Provider::Aws => xml::render_create_bucket(&region),
            // Google's bucket resource states its own location; Azure's
            // container has none to state.
            Provider::Google => {
                let mut entries = vec![("name", crate::Scalar::from(bucket))];
                let location = region.to_ascii_uppercase();
                entries.push(("location", crate::Scalar::from(location.as_str())));
                if let Some(class) = self.options.google().storage_class() {
                    entries.push(("storageClass", crate::Scalar::from(class)));
                }
                crate::json::into_utf8(&crate::Scalar::from_struct(entries)?)?
            }
            Provider::Azure => String::new(),
        };
        let request = match self.provider {
            Provider::Aws => self
                .common(Request::new("PUT", "CreateBucket", bucket, ""))
                .body(document.as_bytes()),
            Provider::Google => {
                // A bucket lives in a project, and there is no default one.
                let project = self.options.google().project().ok_or_else(|| {
                    Error::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "expected a Google project to own the bucket: name one with \
                         GoogleOptions::with_project or the project_id property",
                    ))
                })?;
                self.common(super::google::dialect::create_bucket_request(
                    bucket,
                    project,
                    document.as_bytes(),
                ))
            }
            Provider::Azure => self.common(super::azure::dialect::create_container_request(bucket)),
        };
        let answer = self.send(&request)?;
        // Owning it already is what a repeated create means, not a conflict.
        if answer.status < 300 {
            return Ok(());
        }
        let code = match self.provider {
            Provider::Aws | Provider::Azure => super::xml::parse_error(&answer.body),
            Provider::Google => super::google::json::parse_error(&answer.body),
        }
        .map(|error| error.code);
        if matches!(
            code.as_deref(),
            Some("BucketAlreadyOwnedByYou" | "ContainerAlreadyExists" | "conflict")
        ) {
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
        let request = match self.provider {
            Provider::Aws => self.common(Request::new("DELETE", "DeleteBucket", bucket, "")),
            Provider::Google => self.common(super::google::dialect::delete_bucket_request(bucket)),
            Provider::Azure => self.common(super::azure::dialect::delete_container_request(bucket)),
        };
        let answer = self.send(&request)?;
        if answer.status == 404 || answer.status < 300 {
            return Ok(());
        }
        Err(self.failure(&request, &answer))
    }
}

/// The parts of one upload, read from its source one at a time.
///
/// What bounds an upload's memory: a part-sized buffer is filled from the
/// source and sent before the next is read, whatever the value's length. The
/// source declared its length up front, so a source that ends early is a
/// refusal rather than a shorter object.
struct Parts<'source> {
    source: &'source mut dyn Read,
    part_size: usize,
    total: u64,
    remaining: u64,
}

impl<'source> Parts<'source> {
    fn new(source: &'source mut dyn Read, length: u64, part_size: usize) -> Self {
        Self {
            source,
            part_size: part_size.max(1),
            total: length,
            remaining: length,
        }
    }

    /// Fill `buffer` with the next part, answering `false` once every byte
    /// is out.
    fn next(&mut self, buffer: &mut Vec<u8>) -> Result<bool> {
        if self.remaining == 0 {
            return Ok(false);
        }
        let length = usize::try_from(self.remaining)
            .map_or(self.part_size, |remaining| remaining.min(self.part_size));
        buffer.clear();
        buffer
            .try_reserve(length)
            .map_err(|_| crate::iobase::oversized(length as u64))?;
        buffer.resize(length, 0);
        let mut filled = 0;
        while filled < length {
            match self.source.read(&mut buffer[filled..]) {
                Ok(0) => {
                    return Err(short_upload(
                        self.total,
                        self.total - self.remaining + filled as u64,
                    ));
                }
                Ok(read) => filled += read,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => return Err(Error::Io(error)),
            }
        }
        self.remaining -= length as u64;
        Ok(true)
    }
}

/// Report a source that ended before the length it declared.
pub(super) fn short_upload(expected: u64, got: u64) -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::UnexpectedEof,
        format!("expected {expected} bytes to upload, got {got}"),
    ))
}

/// Refuse a value too large for the store's part count.
fn too_many_parts() -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        "expected a value small enough to upload in the parts this store accepts",
    ))
}

/// The host and the path-with-query of a whole URL.
///
/// A store that hands a location back states the host it is to be reached at,
/// and a signature covers the path as sent, so both are taken from the URL
/// rather than from the endpoint the client was built with.
fn split_url(url: &str) -> (String, String) {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    match rest.split_once('/') {
        Some((host, path)) => (host.to_owned(), format!("/{path}")),
        None => (rest.to_owned(), "/".to_owned()),
    }
}

/// Read a response body to its end so its connection can be pooled.
///
/// A client that stops at the byte count it wanted leaves the stream mid-body,
/// and the connection cannot be reused - which on a real store is a fresh TCP
/// and TLS handshake for every ranged read, the exact cost this backend is
/// built to avoid. A ranged answer is already the size that was asked for, so
/// this normally reads the one zero that says so; a store that answered with
/// more than was asked for is abandoned instead of drained, because reading
/// past what a caller wanted is the larger waste.
/// What a ranged answer says about the window it carries.
struct Window {
    /// The object's whole length, when the answer stated it.
    total: Option<u64>,
    /// Bytes to discard before the window, when the store ignored the range.
    skip: u64,
    /// The bytes the answer says are coming, including anything skipped.
    length: Option<u64>,
}

/// A body that gives its connection back when it is dropped part way through.
///
/// Bytes left unread on the wire leave a connection unusable for the next
/// request, so the pool discards it - and reconnecting costs a round trip and,
/// over TLS, a handshake. Reading the remainder costs a copy of bytes already
/// in flight, so the remainder wins up to [`DRAIN_LIMIT`], past which the
/// connection is not worth what it would take to save it.
struct Pooled {
    inner: Box<dyn Read + Send>,
    /// What is left before the body ends, or [`u64::MAX`] when unstated.
    left: u64,
}

impl Read for Pooled {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buffer)?;
        self.left = self.left.saturating_sub(read as u64);
        Ok(read)
    }
}

impl Drop for Pooled {
    fn drop(&mut self) {
        if self.left == 0 || self.left > POOLED_DRAIN_LIMIT as u64 {
            return;
        }
        drain_upto(&mut self.inner, POOLED_DRAIN_LIMIT);
    }
}

/// Read and throw away `count` bytes, reporting whether they were all there.
fn discard(reader: &mut (impl Read + ?Sized), count: u64) -> Result<bool> {
    let mut discarded = 0_u64;
    let mut sink = [0_u8; 8192];
    while discarded < count {
        let want = usize::try_from((count - discarded).min(sink.len() as u64)).unwrap_or(0);
        let read = reader.read(&mut sink[..want]).map_err(Error::Io)?;
        if read == 0 {
            return Ok(false);
        }
        discarded += read as u64;
    }
    Ok(true)
}

fn drain(reader: &mut (impl Read + ?Sized)) {
    drain_upto(reader, DRAIN_LIMIT);
}

/// Read and throw away up to `limit` bytes, to keep a connection reusable.
fn drain_upto(reader: &mut (impl Read + ?Sized), limit: usize) {
    let mut sink = [0_u8; 4096];
    let mut discarded = 0_usize;
    while discarded < limit {
        match reader.read(&mut sink) {
            Ok(0) | Err(_) => return,
            Ok(read) => discarded += read,
        }
    }
}

/// How much of an over-long body is drained before the connection is dropped.
const DRAIN_LIMIT: usize = 64 * 1024;

/// How much of an abandoned stream is drained to keep its connection.
///
/// Larger than [`DRAIN_LIMIT`] because the bytes were asked for: a caller that
/// opens an object and stops after its header has the rest already on the way,
/// and copying it out of the socket beats a fresh connection and, over TLS, a
/// fresh handshake. Past this the transfer is the larger cost and the
/// connection is let go.
const POOLED_DRAIN_LIMIT: usize = 1024 * 1024;

/// The largest document read into memory from a non-object answer.
///
/// A listing page of a thousand keys is tens of kilobytes; this bound exists so
/// a store answering something unexpected cannot make the client hold it.
const MAX_DOCUMENT: u64 = 32 * 1024 * 1024;

/// The process-wide connection pool, shared by every default-configured client.
fn shared_agent() -> &'static ureq::Agent {
    static AGENT: std::sync::OnceLock<ureq::Agent> = std::sync::OnceLock::new();
    AGENT.get_or_init(|| build_agent(&S3Options::default(), None))
}

/// Build an agent for `options`.
///
/// The request budget is applied per phase rather than as one global deadline.
/// Both bound the same hazard - a store that accepts a connection and then
/// stops answering - but a global deadline is re-checked around every read and
/// write, which costs more per request than the whole of signing one. Per
/// phase, the bound is free.
fn build_agent(options: &S3Options, tls: Option<ureq::tls::TlsConfig>) -> ureq::Agent {
    let mut builder = ureq::Agent::config_builder()
            // Statuses are read, never raised: a store says what it means in
            // the status and a document, and this client maps both itself.
            .http_status_as_error(false)
            // Nor is a 3xx a redirect to follow. The one redirect that matters
            // - a bucket answering with the region it is in - is handled here,
            // where the signing region can be corrected; and Google's resumable
            // upload answers 308 for its own reasons, which is not a redirect
            // at all and has no location to follow.
            .max_redirects(0)
            .max_redirects_will_error(false)
            .timeout_connect(Some(options.connect_timeout()))
            .timeout_send_request(Some(options.timeout()))
            .timeout_recv_response(Some(options.timeout()))
            .timeout_recv_body(Some(options.timeout()))
            .user_agent(concat!("yggdryl/", env!("CARGO_PKG_VERSION")));
    // A named proxy replaces what the environment says; `.proxy` is left
    // untouched otherwise so `HTTPS_PROXY` and `NO_PROXY` keep deciding.
    if let Some(proxy) = options.proxy().and_then(|uri| ureq::Proxy::new(uri).ok()) {
        builder = builder.proxy(Some(proxy));
    }
    // The bundle `AWS_CA_BUNDLE` names is trusted in place of the platform's
    // roots, which is what reaching a private endpoint through a private
    // authority needs.
    if let Some(tls) = tls {
        builder = builder.tls_config(tls);
    }
    ureq::Agent::new_with_config(builder.build())
}

/// The window attempt `attempt + 1` is drawn from: doubling, and capped.
fn backoff(attempt: u32) -> Duration {
    let steps = attempt.saturating_sub(1).min(6);
    RETRY_BACKOFF
        .saturating_mul(1_u32 << steps)
        .min(RETRY_BACKOFF_CAP)
}

/// How long a client may spend on retries before it stops making them.
///
/// Doubling spreads one client's own attempts, and does nothing about the
/// other hundred that failed at the same instant: when a store is refusing
/// broadly, every client retrying every request turns a partial outage into a
/// worse one. A budget is what makes the client's total retry load bounded
/// rather than proportional to its failure rate. Each retry costs
/// [`RETRY_COST`] tokens, a request that succeeds without one refunds
/// [`RETRY_REFUND`], and a retry that succeeds gives its cost back, so a
/// healthy client always has budget and a client that is only failing runs out
/// and fails fast.
struct RetryBudget {
    tokens: std::sync::atomic::AtomicI64,
}

impl Default for RetryBudget {
    fn default() -> Self {
        Self {
            tokens: std::sync::atomic::AtomicI64::new(RETRY_TOKENS),
        }
    }
}

impl RetryBudget {
    /// Take the price of one retry, or refuse it.
    fn withdraw(&self) -> bool {
        self.tokens
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |held| {
                (held >= RETRY_COST).then_some(held - RETRY_COST)
            })
            .is_ok()
    }

    /// Put `tokens` back, never above where the budget started.
    fn refund(&self, tokens: i64) {
        let _ = self
            .tokens
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |held| {
                Some((held + tokens).min(RETRY_TOKENS))
            });
    }

    /// What is left, for the counters to report.
    fn remaining(&self) -> i64 {
        self.tokens.load(Ordering::Relaxed)
    }
}

/// A body that re-opens itself when the transfer dies part way through.
///
/// A long read over a network dies for reasons that have nothing to do with
/// the object - a reset connection, an idle timeout, a load balancer being
/// recycled - and failing the whole transfer for one of them means re-reading
/// everything already delivered. On a multi-gigabyte scan that is the
/// difference between finishing and not. This asks for the rest, from the byte
/// it stopped at, and carries on; the caller sees one uninterrupted stream and
/// the resumed request is counted like any other.
///
/// Only a transport failure resumes, and only while *consecutive* failures
/// stay under the client's attempt limit - a byte arriving resets that, so a
/// transfer which keeps moving survives any number of interruptions while one
/// that cannot deliver a byte stops rather than looping. A store refusing the
/// request answers with a status instead, which never reaches here.
struct Resuming {
    client: Arc<Client>,
    bucket: String,
    key: String,
    /// Where the caller's window starts.
    start: u64,
    /// The last byte the window covers, when it is bounded.
    last: Option<u64>,
    /// How much of it has reached the caller.
    delivered: u64,
    /// Failures since the last byte arrived.
    failures: u32,
    reader: Box<dyn Read + Send>,
}

impl Read for Resuming {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        loop {
            match self.reader.read(buffer) {
                Ok(read) => {
                    self.delivered = self.delivered.saturating_add(read as u64);
                    if read > 0 {
                        self.failures = 0;
                    }
                    return Ok(read);
                }
                Err(error) => {
                    if !is_resumable(&error)
                        || self.failures.saturating_add(1) >= self.client.options().max_attempts()
                    {
                        return Err(error);
                    }
                    self.failures += 1;
                    self.client.pause(self.failures, None);
                    let from = self.start.saturating_add(self.delivered);
                    if self.last.is_some_and(|last| from > last) {
                        // Everything asked for arrived; the failure was the
                        // end of the body announcing itself badly.
                        return Ok(0);
                    }
                    match self
                        .client
                        .open_resumed(&self.bucket, &self.key, from, self.last)
                    {
                        Ok(reader) => self.reader = reader,
                        // The re-open failed too, so the original failure is
                        // what the caller hears about.
                        Err(_) => return Err(error),
                    }
                }
            }
        }
    }
}

/// Whether a read failure is the transport's rather than the store's verdict.
///
/// The generic kind is included deliberately: a client library reports a
/// severed connection in more than one shape, and mistaking one for a decoding
/// failure costs the whole transfer where mistaking it the other way costs one
/// bounded re-open.
fn is_resumable(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::UnexpectedEof
            | std::io::ErrorKind::TimedOut
            | std::io::ErrorKind::Interrupted
            | std::io::ErrorKind::Other
    )
}

/// A starting point for one client's jitter, different from every other's.
///
/// Two clients in one process that fail at the same instant should not draw
/// the same delays, so each starts its counter somewhere of its own.
fn fresh_jitter() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let ordinal = NEXT.fetch_add(1, Ordering::Relaxed);
    crate::xxhash::xxh3(
        &[u64::from(std::process::id()), ordinal]
            .map(u64::to_le_bytes)
            .concat(),
    )
}

/// The `Retry-After` an answer asks for, when it asks for one this will wait.
///
/// Seconds only: the HTTP-date spelling is legal and no S3 implementation
/// sends it, and reading a date needs a clock this has no reason to trust.
fn retry_after(answer: &Answer) -> Option<Duration> {
    let seconds: u64 = answer.header("retry-after")?.trim().parse().ok()?;
    let asked = Duration::from_secs(seconds);
    (asked <= RETRY_AFTER_CAP).then_some(asked)
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
            super::xml::parse_error(&answer.body)
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
fn malformed(service: &'static str, operation: &'static str, path: &str, reason: &str) -> Error {
    Error::remote(service, operation, 200, "MalformedResponse", reason, path)
}

/// A request that never reached the store, or whose connection failed.
fn transport_failure(
    service: &'static str,
    request: &Request<'_>,
    path: String,
    error: ureq::Error,
) -> Error {
    Error::Io(std::io::Error::other(format!(
        "{service} {} at {path:?} failed: {error}",
        request.operation
    )))
}

/// MD5 of `bytes`, which `DeleteObjects` still requires as `Content-MD5`.
pub(super) fn md5_of(bytes: &[u8]) -> [u8; 16] {
    use md5::Digest as _;
    md5::Md5::digest(bytes).into()
}

/// Report a poisoned client lock without panicking a caller.
fn poisoned() -> Error {
    Error::Io(std::io::Error::other(
        "an S3 client lock was poisoned by a panicking writer",
    ))
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/s3/client.rs` and `rust/tests/s3/properties.rs`
    //! pin and a caller cannot reach.
    //!
    //! Addressing, the retry schedule, and what a redirect or a `Content-Range`
    //! says are settled before a single request goes out, so they are pinned
    //! without a store to answer. Every item forwards to the real one, so the
    //! client stays exactly as private as it was.
    use std::time::{Duration, SystemTime};

    use crate::Result;
    use crate::Url;
    use crate::s3::{Credentials, Provider, S3Options};

    /// The region a location that names no endpoint is signed for.
    pub const DEFAULT_REGION: &str = super::DEFAULT_REGION;

    /// The pause before the first retry; each further one doubles it.
    pub const RETRY_BACKOFF: Duration = super::RETRY_BACKOFF;

    /// How long the client waits before attempt `attempt`.
    pub fn backoff(attempt: u32) -> Duration {
        super::backoff(attempt)
    }

    /// The region a redirect names, for an answer of `status` and `headers`.
    ///
    /// The body plays no part, so this builds the answer the reader reads
    /// rather than handing one out.
    pub fn bucket_region_of(status: u16, headers: &[(String, String)]) -> Option<String> {
        super::bucket_region_of(&super::Answer {
            status,
            headers: headers.to_vec(),
            body: Vec::new(),
        })
    }

    /// The whole length a `Content-Range` states, when it states one.
    pub fn total_of_content_range(header: Option<&str>) -> Option<u64> {
        super::total_of_content_range(header)
    }

    /// How one endpoint addresses a container and a key.
    pub struct Endpoint(super::Endpoint);

    impl Endpoint {
        /// The endpoint its parts spell.
        pub fn new(
            scheme: &str,
            host: &str,
            port: Option<u16>,
            path_style: bool,
            account: Option<&str>,
            account_in_path: bool,
        ) -> Self {
            Self(super::Endpoint {
                scheme: scheme.to_owned(),
                host: host.to_owned(),
                port,
                path_style,
                account: account.map(str::to_owned),
                account_in_path,
            })
        }

        /// The `Host` header a request against `container` carries.
        pub fn host_header(&self, container: &str) -> String {
            self.0.host_header(container)
        }

        /// The request path `key` in `container` is sent as.
        pub fn path(&self, container: &str, key: &str) -> String {
            self.0.path(container, key)
        }
    }

    /// One configured client, as much of it as a test can observe.
    pub struct Client(super::Client);

    impl Client {
        /// Build the client `url` and `options` describe, touching nothing.
        ///
        /// # Errors
        ///
        /// Returns a refusal when the URL names no store, no container, or an
        /// endpoint that cannot be read as a location.
        pub fn new(url: &Url, options: S3Options) -> Result<Self> {
            super::Client::new(url, options).map(Self)
        }

        /// The region requests are currently signed for.
        pub fn region(&self) -> String {
            self.0.region()
        }

        /// The scheme the endpoint is reached over.
        pub fn scheme(&self) -> &str {
            &self.0.endpoint.scheme
        }

        /// The `Host` header a request against `container` carries.
        pub fn host_header(&self, container: &str) -> String {
            self.0.endpoint.host_header(container)
        }

        /// The request path `key` in `container` is sent as.
        pub fn path(&self, container: &str, key: &str) -> String {
            self.0.endpoint.path(container, key)
        }

        /// The Azure storage account a request against `url` addresses.
        pub fn azure_account(
            provider: Provider,
            url: &Url,
            options: &S3Options,
            handed: Option<&Credentials>,
        ) -> Option<String> {
            super::Client::azure_account(provider, url, options, handed)
        }

        /// The credentials a location carries, when it carries a pair.
        pub fn url_credentials(url: &Url) -> Option<Credentials> {
            super::Client::url_credentials(url)
        }

        /// The scheme, host and port one endpoint spelling splits into.
        ///
        /// # Errors
        ///
        /// Returns a refusal when the endpoint names a port that is not one.
        pub fn split_endpoint(endpoint: &str) -> Result<(String, String, Option<u16>)> {
            super::Client::split_endpoint(endpoint)
        }

        /// The access key id the client would sign with at `now`.
        ///
        /// `None` is an anonymous client, which is a way to reach a public
        /// bucket rather than a failure.
        ///
        /// # Errors
        ///
        /// Whatever walking the credential chain refuses.
        pub fn signer_access_key_id(&self, now: SystemTime) -> Result<Option<String>> {
            Ok(self
                .0
                .signer(now)?
                .map(|signer| signer.access_key_id().to_owned()))
        }
    }
}
