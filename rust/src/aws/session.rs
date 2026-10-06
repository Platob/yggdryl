//! What a caller states about AWS, and everything else resolved lazily.
//!
//! A [`Session`] is the one door the crate reaches AWS's configuration
//! through: the profile, the region, the endpoint a service is reached at,
//! and the credential set every request signs with. It records what the
//! caller stated, reads the environment and the shared files once on the
//! first question that needs them, walks the credential chain once on the
//! first request that signs, and refreshes a temporary set before it lapses.
//! Cloning one shares that work, so a session built once serves every handle
//! a process opens.
//!
//! A credential source the caller configured - a named profile, a profile's
//! role, sign-in or console session, a `credential_process`, a key pair in
//! the environment or in a shared file, half of one included - is the
//! identity the caller meant: when it fails, the walk ends with a refusal
//! naming it and why, and no later source signs in its place. Only a source
//! with nothing configured lets the walk go on to the next.

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, SystemTime};

use super::container;
use super::credentials::{Credentials, Refusal};
use super::login;
use super::metadata::{self, Imds};
use super::process;
use super::profile::{self, Files, Profile};
use super::sigv4::Signer;
use super::sso::{self, Sso, SsoLogin};
use super::sts::{self, AssumedRole, CredentialSource};
use crate::auth::{Environment, Expiring, Lease, Report, instant, iso8601};
use crate::duration::duration_from_text;
use crate::integer::integer_from_text_as;
use crate::{Arn, ArnPartition, Charset, Error, Result};
use smol_str::SmolStr;

/// The profile read when nothing names another.
const DEFAULT_PROFILE: &str = "default";
/// The region STS is reached in when nothing names one.
const DEFAULT_REGION: &str = "us-east-1";
/// Replace a temporary set this long before it lapses; a walk that fails
/// inside the window keeps the set, which still signs.
const REFRESH_WINDOW: Duration = Duration::from_secs(15 * 60);
/// Inside this long before a temporary set lapses, a walk that fails is the
/// answer rather than the set in hand, as botocore's mandatory refresh
/// window: a set about to lapse mid-request is not one to keep signing with.
const MANDATORY_WINDOW: Duration = Duration::from_secs(10 * 60);
/// How long a walk that failed is held before the chain is walked again, so
/// a set that nobody can obtain costs a request its answer rather than every
/// request a probe of every source.
const RETRY_PAUSE: Duration = Duration::from_secs(30);
/// How long a walk that found nothing configured is held before the chain is
/// walked again: unsigned requests stay unsigned for a while, and a source
/// that appears - a metadata service that answers late - is found then.
const ANONYMOUS_HOLD: Duration = Duration::from_secs(5 * 60);
/// The bound on establishing a connection to any identity service.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// The largest answer an identity service gives that is read: every one is
/// a document of a few kilobytes.
const MAX_IDENTITY_ANSWER: u64 = 256 * 1024;
/// How many `source_profile` steps a chain of profiles may take.
const MAX_PROFILE_DEPTH: usize = 8;
/// How many access key ids a store refused a session remembers. A
/// temporary set's key id is issued once, so a refused one never signs
/// again; the bound keeps a process refused over and over from growing.
const REFUSED_KEYS: usize = 8;
/// How many signers a session keeps: one per region and service the set in
/// hand signs for, each holding its day's derived key. A process signs for a
/// handful; the bound keeps one that names regions without end from growing.
const SIGNERS: usize = 8;
/// The regions the global `sts.amazonaws.com` still serves under the legacy
/// endpoint mode; every other region is regional in both modes.
const LEGACY_STS_REGIONS: [&str; 15] = [
    "ap-northeast-1",
    "ap-south-1",
    "ap-southeast-1",
    "ap-southeast-2",
    "ca-central-1",
    "eu-central-1",
    "eu-north-1",
    "eu-west-1",
    "eu-west-2",
    "eu-west-3",
    "sa-east-1",
    "us-east-1",
    "us-east-2",
    "us-west-1",
    "us-west-2",
];

/// What a session asks for the code an MFA device shows, given the device's
/// serial; `None` is a person who declined.
pub type MfaPrompt = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// What the caller stated.
#[derive(Clone)]
struct Knobs {
    profile: Option<String>,
    region: Option<String>,
    credentials: Option<Credentials>,
    anonymous: bool,
    role: Option<AssumedRole>,
    sso: Option<Sso>,
    credential_process: Option<String>,
    endpoint_url: Option<String>,
    service_endpoints: BTreeMap<String, String>,
    config_file: Option<PathBuf>,
    credentials_file: Option<PathBuf>,
    config_text: Option<String>,
    credentials_text: Option<String>,
    directory: Option<PathBuf>,
    environment: Environment,
    reads_environment: bool,
    metadata_endpoint: Option<String>,
    metadata_disabled: Option<bool>,
    metadata_timeout: Option<Duration>,
    metadata_attempts: Option<u32>,
    sso_login: SsoLogin,
    mfa_prompt: Option<MfaPrompt>,
    use_fips_endpoint: Option<bool>,
    use_dualstack_endpoint: Option<bool>,
    sts_regional_endpoints: Option<bool>,
    ca_bundle: Option<PathBuf>,
}

impl Default for Knobs {
    fn default() -> Self {
        Self {
            profile: None,
            region: None,
            credentials: None,
            anonymous: false,
            role: None,
            sso: None,
            credential_process: None,
            endpoint_url: None,
            service_endpoints: BTreeMap::new(),
            config_file: None,
            credentials_file: None,
            config_text: None,
            credentials_text: None,
            directory: None,
            environment: Environment::Process,
            reads_environment: true,
            metadata_endpoint: None,
            metadata_disabled: None,
            metadata_timeout: None,
            metadata_attempts: None,
            sso_login: SsoLogin::Never,
            mfa_prompt: None,
            use_fips_endpoint: None,
            use_dualstack_endpoint: None,
            sts_regional_endpoints: None,
            ca_bundle: None,
        }
    }
}

/// The set the chain found, and which source answered it.
#[derive(Clone)]
struct Found {
    credentials: Credentials,
    source: &'static str,
}

impl Expiring for Found {
    fn expires_at(&self) -> Option<SystemTime> {
        self.credentials.expires_at()
    }

    fn refresh_window(&self) -> Option<Duration> {
        (self.source == "login").then_some(login::REFRESH_WINDOW)
    }
}

struct Inner {
    knobs: Knobs,
    /// The HTTP session the identity services are reached through, built
    /// once; its refusal - a CA bundle that cannot be read - is every
    /// identity request's.
    http: OnceLock<std::result::Result<crate::http::Session, String>>,
    /// Both shared files as last read; read again by a walk that finds
    /// either moved on disk, and by the first question after `invalidate`.
    files: Mutex<Option<SharedFiles>>,
    /// The set in hand, refreshed before it lapses and kept while a walk
    /// that would replace it fails.
    lease: Lease<Found>,
    /// The next walk passes the CLI caches by: a store said the set they
    /// hold has lapsed, whatever their expiry says.
    skip_caches: AtomicBool,
    /// The access key ids a store refused, oldest first, at most
    /// `REFUSED_KEYS`: a source answering one held in every region is
    /// passed over, and an exchange in a region one is held in is not sent.
    refused: Mutex<VecDeque<Refused>>,
    /// The signers of the set in hand, oldest first, at most `SIGNERS`.
    signers: Mutex<Vec<Held>>,
}

/// One signer, and what it was made for: the whole set rather than its key
/// alone, because a refreshed session can keep its access key id and change
/// its secret or its token.
struct Held {
    credentials: Credentials,
    region: String,
    service: SmolStr,
    signer: Arc<Signer>,
}

/// A key a store refused, where, by whom, and until when: for good when its
/// set lapsed, until the end of a pause when the store did not recognize it.
///
/// A key an opt-in region did not recognize is held in that region alone:
/// such a region refuses every key until the account enables it, so the
/// refusal says nothing of the key anywhere else.
struct Refused {
    access_key_id: String,
    /// The region the refusal holds in; `None` is every region.
    region: Option<String>,
    /// Who refused it, as the walk names them: `the service at {endpoint}
    /// in {region}`, or `a store` for a refusal a caller reported.
    by: String,
    until: Option<SystemTime>,
}

/// Both shared files as one read of them found them.
struct SharedFiles {
    /// The version of the configuration file and of the credentials file
    /// the read saw, `None` for one stated as text or not on disk.
    versions: [Option<Version>; 2],
    parsed: Arc<Files>,
    /// Why a file that is there could not be read, for the walk to name.
    unreadable: Vec<String>,
}

/// What the file system says of one shared file: rewriting it in place - a
/// set dumped anew, a profile edited - moves its length or the instant it
/// was written.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Version {
    length: u64,
    modified: Option<SystemTime>,
}

/// Who this process is to AWS, and where AWS is.
///
/// Every knob is optional. An unset one is resolved the way the AWS tools
/// resolve it - the environment, then the profile in `~/.aws/config` and
/// `~/.aws/credentials`, then the metadata services, then a default - once,
/// on the first question that needs it. Explicit values always win.
///
/// | question | explicit | environment | profile | default |
/// | --- | --- | --- | --- | --- |
/// | profile | `with_profile` | `AWS_DEFAULT_PROFILE`, `AWS_PROFILE` | | `default` |
/// | region | `with_region` | `AWS_REGION`, `AWS_DEFAULT_REGION` | `region` | none |
/// | endpoint of a service | `with_service_endpoint_url`, then `with_endpoint_url` | `AWS_ENDPOINT_URL_<SERVICE>`, then `AWS_ENDPOINT_URL` | its `[services]` entry, then `endpoint_url` | the published host |
/// | credentials | `with_credentials`, `with_assumed_role`, `with_sso`, `with_credential_process` | the chain | the chain | none, unsigned |
///
/// A service's endpoint is botocore's configured endpoint
/// ([`Self::endpoint_url`]): `<SERVICE>` is its service id - `S3`, `STS`,
/// `SSO`, `SSO_OIDC`, `SIGNIN`, `S3TABLES` for the services this crate
/// calls - and `AWS_IGNORE_CONFIGURED_ENDPOINT_URLS`, else the profile's
/// `ignore_configured_endpoint_urls`, switches the environment and profile
/// columns off and never what was stated.
///
/// The credential chain is walked in the order botocore walks it. A source
/// that is configured and broken ends the walk with a refusal naming it and
/// why - a profile somebody named and nobody wrote, a role that cannot be
/// assumed, a lapsed sign-in, half a key pair - rather than letting another
/// identity sign; a source with nothing configured is recorded as absent and
/// the walk goes on. A set a source answers that has lapsed, or whose key a
/// store refused, is passed over by name. The refusal names every failure
/// and every absence the walk met.
///
/// ```
/// use yggdryl::aws::{Credentials, Session};
///
/// # fn main() -> yggdryl::Result<()> {
/// // A sealed session consults nothing outside what it was told.
/// let session = Session::new()
///     .with_environment(false)
///     .with_region("eu-west-3")
///     .with_credentials(Credentials::new("AKIAIOSFODNN7EXAMPLE", "wJalrXUtnFEMI"));
/// let keys = session.credentials(std::time::SystemTime::now())?.expect("the explicit set");
/// assert_eq!(keys.access_key_id(), "AKIAIOSFODNN7EXAMPLE");
/// assert_eq!(session.credential_source(), Some("explicit credentials"));
/// assert_eq!(session.region().as_deref(), Some("eu-west-3"));
/// assert_eq!(session.sts_endpoint("eu-west-3")?, "https://sts.eu-west-3.amazonaws.com");
///
/// // A session told the environment answers from it, and nothing else.
/// let session = Session::new()
///     .with_variables([("AWS_REGION", "ap-southeast-1"), ("AWS_ENDPOINT_URL_S3", "http://localhost:9000")])
///     .with_directory("/nonexistent/.aws");
/// assert_eq!(session.region().as_deref(), Some("ap-southeast-1"));
/// assert_eq!(session.endpoint_url("s3")?.as_deref(), Some("http://localhost:9000"));
/// assert_eq!(session.endpoint_url("sts")?, None);
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct Session {
    inner: Arc<Inner>,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    /// A session that states nothing and resolves everything.
    pub fn new() -> Self {
        Self::from_knobs(Knobs::default())
    }

    fn from_knobs(knobs: Knobs) -> Self {
        Self {
            inner: Arc::new(Inner {
                knobs,
                http: OnceLock::new(),
                files: Mutex::new(None),
                lease: Lease::new(
                    "AWS credential set",
                    REFRESH_WINDOW,
                    MANDATORY_WINDOW,
                    RETRY_PAUSE,
                    ANONYMOUS_HOLD,
                ),
                skip_caches: AtomicBool::new(false),
                refused: Mutex::new(VecDeque::new()),
                signers: Mutex::new(Vec::new()),
            }),
        }
    }

    /// A session with one knob changed, and nothing resolved yet.
    fn modified(&self, change: impl FnOnce(&mut Knobs)) -> Self {
        let mut knobs = self.inner.knobs.clone();
        change(&mut knobs);
        Self::from_knobs(knobs)
    }

    // --- what the caller states ---------------------------------------------

    /// Read `profile` from the shared files instead of the one
    /// `AWS_DEFAULT_PROFILE` or `AWS_PROFILE` names.
    ///
    /// As with the AWS tools, a profile named here rather than by the
    /// environment also takes precedence over the environment's own keys and
    /// its web identity (`AWS_ROLE_ARN`, `AWS_WEB_IDENTITY_TOKEN_FILE`,
    /// `AWS_ROLE_SESSION_NAME`): the profile is what the caller meant. A
    /// profile named here that neither file holds is a refusal.
    #[must_use]
    pub fn with_profile(&self, profile: impl Into<String>) -> Self {
        let profile: String = profile.into();
        self.modified(|knobs| {
            knobs.profile = (!profile.trim().is_empty()).then(|| profile.trim().to_owned());
        })
    }

    /// Sign for, and address, `region`.
    #[must_use]
    pub fn with_region(&self, region: impl Into<String>) -> Self {
        let region: String = region.into();
        self.modified(|knobs| {
            knobs.region = (!region.trim().is_empty()).then(|| region.trim().to_owned());
        })
    }

    /// Sign with exactly `credentials`, consulting no other source - though a
    /// role named with [`Self::with_assumed_role`] is still traded for.
    #[must_use]
    pub fn with_credentials(&self, credentials: Credentials) -> Self {
        self.modified(|knobs| {
            knobs.credentials = Some(credentials);
            knobs.anonymous = false;
        })
    }

    /// Sign nothing, for public resources.
    #[must_use]
    pub fn with_anonymous(&self, anonymous: bool) -> Self {
        self.modified(|knobs| {
            knobs.anonymous = anonymous;
            if anonymous {
                knobs.credentials = None;
            }
        })
    }

    /// Sign as `role` rather than as whatever the chain answers.
    ///
    /// The chain still answers, and what it answers is what signs the
    /// *exchange*: one STS request trades those keys for the role's, and the
    /// session it hands back is what signs everything after. It expires, so
    /// it is traded again shortly before it does rather than per request. A
    /// role named here that cannot be assumed is a refusal, never a request
    /// signed as somebody else.
    #[must_use]
    pub fn with_assumed_role(&self, role: AssumedRole) -> Self {
        self.modified(|knobs| knobs.role = Some(role))
    }

    /// Obtain keys through the IAM Identity Center sign-in `sso` describes,
    /// as a profile with `sso_session` would.
    #[must_use]
    pub fn with_sso(&self, sso: Sso) -> Self {
        self.modified(|knobs| knobs.sso = Some(sso))
    }

    /// Obtain keys by running `command`, as a profile's `credential_process`
    /// would.
    #[must_use]
    pub fn with_credential_process(&self, command: impl Into<String>) -> Self {
        let command: String = command.into();
        self.modified(|knobs| {
            knobs.credential_process =
                (!command.trim().is_empty()).then(|| command.trim().to_owned());
        })
    }

    /// Reach every service at `url`, the identity services a walk calls
    /// included, as `AWS_ENDPOINT_URL` does - but stated, so it beats every
    /// configured endpoint and is never ignored;
    /// [`Self::with_service_endpoint_url`] beats it for one service.
    #[must_use]
    pub fn with_endpoint_url(&self, url: impl Into<String>) -> Self {
        let url: String = url.into();
        self.modified(|knobs| knobs.endpoint_url = endpoint(&url))
    }

    /// Reach `service` - its service id, `s3`, `sts`, `sso-oidc`, read as
    /// [`Self::endpoint_url`] reads it - at `url`, as
    /// `AWS_ENDPOINT_URL_<SERVICE>` and a `[services]` entry do, over
    /// [`Self::with_endpoint_url`] and every configured endpoint.
    #[must_use]
    pub fn with_service_endpoint_url(&self, service: &str, url: impl Into<String>) -> Self {
        let url: String = url.into();
        let key = profile::service_key(service);
        self.modified(|knobs| match endpoint(&url) {
            Some(url) => {
                knobs.service_endpoints.insert(key, url);
            }
            None => {
                knobs.service_endpoints.remove(&key);
            }
        })
    }

    /// Read the configuration file at `path` instead of `AWS_CONFIG_FILE` or
    /// `~/.aws/config`.
    #[must_use]
    pub fn with_config_file(&self, path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        self.modified(|knobs| knobs.config_file = Some(path))
    }

    /// Read the credentials file at `path` instead of
    /// `AWS_SHARED_CREDENTIALS_FILE` or `~/.aws/credentials`.
    #[must_use]
    pub fn with_credentials_file(&self, path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        self.modified(|knobs| knobs.credentials_file = Some(path))
    }

    /// Read `text` as the configuration file, reading no file for it.
    #[must_use]
    pub fn with_config_text(&self, text: impl Into<String>) -> Self {
        let text = text.into();
        self.modified(|knobs| knobs.config_text = Some(text))
    }

    /// Read `text` as the credentials file, reading no file for it.
    #[must_use]
    pub fn with_credentials_text(&self, text: impl Into<String>) -> Self {
        let text = text.into();
        self.modified(|knobs| knobs.credentials_text = Some(text))
    }

    /// Keep the shared files and the caches under `path` instead of `~/.aws`.
    #[must_use]
    pub fn with_directory(&self, path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        self.modified(|knobs| knobs.directory = Some(path))
    }

    /// Read `variables` in place of the process environment.
    #[must_use]
    pub fn with_variables<K, V>(&self, variables: impl IntoIterator<Item = (K, V)>) -> Self
    where
        K: Into<String>,
        V: Into<String>,
    {
        let given: BTreeMap<String, String> = variables
            .into_iter()
            .map(|(name, value)| (name.into(), value.into()))
            .collect();
        self.modified(|knobs| knobs.environment = Environment::Given(given))
    }

    /// Consult, or ignore, the environment, the shared files and the two
    /// metadata services.
    ///
    /// Off, only what was stated on the session decides, which is what a
    /// test wants and what a sandboxed process may need.
    #[must_use]
    pub fn with_environment(&self, reads_environment: bool) -> Self {
        self.modified(|knobs| knobs.reads_environment = reads_environment)
    }

    /// Reach the instance metadata service at `url`, as
    /// `AWS_EC2_METADATA_SERVICE_ENDPOINT` does.
    #[must_use]
    pub fn with_metadata_endpoint(&self, url: impl Into<String>) -> Self {
        let url: String = url.into();
        self.modified(|knobs| knobs.metadata_endpoint = endpoint(&url))
    }

    /// Never ask the instance metadata service, as
    /// `AWS_EC2_METADATA_DISABLED` does.
    #[must_use]
    pub fn with_metadata_disabled(&self, disabled: bool) -> Self {
        self.modified(|knobs| knobs.metadata_disabled = Some(disabled))
    }

    /// Bound each request to the instance metadata service by `timeout`, as
    /// `AWS_METADATA_SERVICE_TIMEOUT` does.
    #[must_use]
    pub fn with_metadata_timeout(&self, timeout: Duration) -> Self {
        self.modified(|knobs| knobs.metadata_timeout = Some(timeout))
    }

    /// Ask the instance metadata service `attempts` times before taking it
    /// to be absent, as `AWS_METADATA_SERVICE_NUM_ATTEMPTS` does.
    #[must_use]
    pub fn with_metadata_attempts(&self, attempts: u32) -> Self {
        self.modified(|knobs| knobs.metadata_attempts = Some(attempts.max(1)))
    }

    /// How a sign-in is shown to a person when IAM Identity Center needs one.
    #[must_use]
    pub fn with_sso_login(&self, login: SsoLogin) -> Self {
        self.modified(|knobs| knobs.sso_login = login)
    }

    /// How a code is asked of a person when a role names an MFA device.
    #[must_use]
    pub fn with_mfa_prompt(&self, prompt: MfaPrompt) -> Self {
        self.modified(|knobs| knobs.mfa_prompt = Some(prompt))
    }

    /// Reach the FIPS endpoints, as `AWS_USE_FIPS_ENDPOINT` does.
    #[must_use]
    pub fn with_use_fips_endpoint(&self, fips: bool) -> Self {
        self.modified(|knobs| knobs.use_fips_endpoint = Some(fips))
    }

    /// Reach the dual-stack endpoints, as `AWS_USE_DUALSTACK_ENDPOINT` does.
    #[must_use]
    pub fn with_use_dualstack_endpoint(&self, dualstack: bool) -> Self {
        self.modified(|knobs| knobs.use_dualstack_endpoint = Some(dualstack))
    }

    /// Reach STS in the region (`true`) or, for the regions that have one,
    /// at the global endpoint (`false`), as `AWS_STS_REGIONAL_ENDPOINTS`
    /// spells `regional` and `legacy`.
    #[must_use]
    pub fn with_sts_regional_endpoints(&self, regional: bool) -> Self {
        self.modified(|knobs| knobs.sts_regional_endpoints = Some(regional))
    }

    /// Trust the certificate authorities in the PEM bundle at `path`, as
    /// `AWS_CA_BUNDLE` does.
    #[must_use]
    pub fn with_ca_bundle(&self, path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        self.modified(|knobs| knobs.ca_bundle = Some(path))
    }

    /// Fill from `ambient` every knob this session does not set for itself.
    ///
    /// Explicit wins, knob by knob; a session that gains nothing is the same
    /// session, with what it has already resolved.
    #[cfg(feature = "s3")]
    pub(crate) fn under(&self, ambient: &Self) -> Self {
        let mine = &self.inner.knobs;
        let theirs = &ambient.inner.knobs;
        let mut merged = mine.clone();
        let mut changed = false;
        macro_rules! fill {
            ($field:ident) => {
                if merged.$field.is_none() && theirs.$field.is_some() {
                    merged.$field = theirs.$field.clone();
                    changed = true;
                }
            };
        }
        fill!(profile);
        fill!(region);
        fill!(role);
        fill!(sso);
        fill!(credential_process);
        fill!(endpoint_url);
        for (service, url) in &theirs.service_endpoints {
            if !merged.service_endpoints.contains_key(service) {
                merged
                    .service_endpoints
                    .insert(service.clone(), url.clone());
                changed = true;
            }
        }
        fill!(config_file);
        fill!(credentials_file);
        fill!(config_text);
        fill!(credentials_text);
        fill!(directory);
        fill!(metadata_endpoint);
        fill!(metadata_disabled);
        fill!(metadata_timeout);
        fill!(metadata_attempts);
        fill!(mfa_prompt);
        fill!(use_fips_endpoint);
        fill!(use_dualstack_endpoint);
        fill!(sts_regional_endpoints);
        fill!(ca_bundle);
        if merged.credentials.is_none() && !merged.anonymous {
            if let Some(credentials) = &theirs.credentials {
                merged.credentials = Some(credentials.clone());
                changed = true;
            } else if theirs.anonymous {
                merged.anonymous = true;
                changed = true;
            }
        }
        if matches!(merged.environment, Environment::Process)
            && matches!(theirs.environment, Environment::Given(_))
        {
            merged.environment = theirs.environment.clone();
            changed = true;
        }
        if matches!(merged.sso_login, SsoLogin::Never)
            && !matches!(theirs.sso_login, SsoLogin::Never)
        {
            merged.sso_login = theirs.sso_login.clone();
            changed = true;
        }
        if changed {
            Self::from_knobs(merged)
        } else {
            self.clone()
        }
    }

    // --- what was stated, read back ------------------------------------------

    /// The role every request signs as, when one was named.
    pub fn assumed_role(&self) -> Option<&AssumedRole> {
        self.inner.knobs.role.as_ref()
    }

    /// The sign-in keys are obtained through, when one was stated.
    pub fn sso(&self) -> Option<&Sso> {
        self.inner.knobs.sso.as_ref()
    }

    /// Whether requests go unsigned.
    pub fn anonymous(&self) -> bool {
        self.inner.knobs.anonymous
    }

    /// The region stated on the session, before anything is resolved.
    #[cfg(feature = "s3")]
    pub(crate) fn stated_region(&self) -> Option<&str> {
        self.inner.knobs.region.as_deref()
    }

    /// Whether the session states who it is - a set, a role, a sign-in, a
    /// process or a profile - rather than leaving it to the chain.
    #[cfg(feature = "s3")]
    pub(crate) fn states_identity(&self) -> bool {
        let knobs = &self.inner.knobs;
        knobs.profile.is_some()
            || knobs.credentials.is_some()
            || knobs.role.is_some()
            || knobs.sso.is_some()
            || knobs.credential_process.is_some()
    }

    /// Whether the environment, the shared files and the metadata services
    /// are consulted.
    pub fn reads_environment(&self) -> bool {
        self.inner.knobs.reads_environment
    }

    /// How a sign-in is shown to a person.
    pub fn sso_login(&self) -> &SsoLogin {
        &self.inner.knobs.sso_login
    }

    // --- what is resolved ------------------------------------------------------

    /// The sign-in stated on the session, resolved: one stated by its
    /// section's name alone - `sso_session` with the account and the role
    /// beside it - takes its start URL, region and scopes from the
    /// `[sso-session]` section the shared files hold; `None` where none is
    /// stated.
    ///
    /// # Errors
    ///
    /// Returns the files' refusal of a section they do not hold, or one
    /// lacking its start URL or region.
    fn stated_sso(&self) -> Result<Option<Sso>> {
        let Some(stated) = &self.inner.knobs.sso else {
            return Ok(None);
        };
        match super::properties::sso_section(stated) {
            Some(name) => self
                .files()
                .sso_session(name, stated.account_id(), stated.role_name())
                .map(Some),
            None => Ok(Some(stated.clone())),
        }
    }

    /// The variables a session built with `with_variables` reads as its
    /// whole environment, as given; `None` for a session reading the
    /// process's own.
    pub(crate) fn given_variables(&self) -> Option<&BTreeMap<String, String>> {
        match &self.inner.knobs.environment {
            Environment::Given(given) => Some(given),
            Environment::Process => None,
        }
    }

    /// A variable as the session reads it: from what it was given, or from
    /// the process; trimmed, and absent when empty or when the session
    /// consults no environment.
    pub fn variable(&self, name: &str) -> Option<String> {
        self.reads_environment()
            .then(|| self.inner.knobs.environment.get(name))
            .flatten()
    }

    /// The profile read: what was stated, else `AWS_DEFAULT_PROFILE`, else
    /// `AWS_PROFILE`, else `default` - the older variable first, which is
    /// the order botocore reads the pair in.
    pub fn profile_name(&self) -> String {
        self.named_profile()
            .map_or_else(|| DEFAULT_PROFILE.to_owned(), |(name, _)| name)
    }

    /// The profile somebody named, and who named it: what was stated, else
    /// `AWS_DEFAULT_PROFILE`, else `AWS_PROFILE`; `None` when nobody did.
    fn named_profile(&self) -> Option<(String, &'static str)> {
        if let Some(name) = &self.inner.knobs.profile {
            return Some((name.clone(), "Session::with_profile"));
        }
        ["AWS_DEFAULT_PROFILE", "AWS_PROFILE"]
            .into_iter()
            .find_map(|variable| Some((self.variable(variable)?, variable)))
    }

    /// The directory the shared files and the caches live under: what was
    /// stated, its variables and a leading `~` expanded, else `~/.aws` under
    /// the home the environment names; `None` when no home is known, and
    /// when the session consults no environment.
    pub fn directory(&self) -> Option<PathBuf> {
        match &self.inner.knobs.directory {
            Some(stated) => Some(self.expanded(stated)),
            None => Some(self.home()?.join(".aws")),
        }
    }

    /// The home directory the AWS tools read on this platform: on Windows
    /// `USERPROFILE`, then `HOME` - a `HOME` an MSYS2 or Cygwin shell
    /// exported names another directory than the one `aws configure` wrote
    /// to - elsewhere `HOME`, then `USERPROFILE`; read from the process or
    /// from the variables a caller handed over, and none for a session that
    /// consults no environment.
    fn home(&self) -> Option<PathBuf> {
        const ORDER: [&str; 2] = if cfg!(windows) {
            ["USERPROFILE", "HOME"]
        } else {
            ["HOME", "USERPROFILE"]
        };
        if !self.reads_environment() {
            return None;
        }
        ORDER
            .into_iter()
            .find_map(|name| self.inner.knobs.environment.get(name))
            .map(PathBuf::from)
    }

    /// `path` with its variables and a leading `~` expanded as botocore
    /// expands a configured path (`os.path.expandvars`, then
    /// `os.path.expanduser`), against the session's own environment.
    fn expand(&self, path: &str) -> PathBuf {
        profile::expand_path(
            path,
            &|name: &str| self.variable(name),
            self.home().as_deref(),
        )
    }

    /// A stated path, expanded as [`Self::expand`] does when it is text; a
    /// path that is not is kept as given.
    fn expanded(&self, path: &Path) -> PathBuf {
        path.to_str()
            .map_or_else(|| path.to_owned(), |text| self.expand(text))
    }

    /// The configuration file read: what was stated, else `AWS_CONFIG_FILE`,
    /// else `config` under [`Self::directory`] - each expanded as
    /// `expand` does. An empty `AWS_CONFIG_FILE` names no file at
    /// all, so none is read, rather than the one under the directory.
    pub fn config_file(&self) -> Option<PathBuf> {
        self.shared_file(
            self.inner.knobs.config_file.as_deref(),
            "AWS_CONFIG_FILE",
            "config",
        )
    }

    /// The credentials file read: what was stated, else
    /// `AWS_SHARED_CREDENTIALS_FILE`, else `credentials` under
    /// [`Self::directory`] - each expanded as `expand` does. An
    /// empty `AWS_SHARED_CREDENTIALS_FILE` names no file at all, so none is
    /// read, rather than the one under the directory.
    pub fn credentials_file(&self) -> Option<PathBuf> {
        self.shared_file(
            self.inner.knobs.credentials_file.as_deref(),
            "AWS_SHARED_CREDENTIALS_FILE",
            "credentials",
        )
    }

    /// One shared file: `stated`, else what `variable` names - its empty
    /// value naming none - else `name` under [`Self::directory`].
    fn shared_file(&self, stated: Option<&Path>, variable: &str, name: &str) -> Option<PathBuf> {
        if let Some(stated) = stated {
            return Some(self.expanded(stated));
        }
        if self.reads_environment() {
            match self.inner.knobs.environment.raw(variable) {
                Some(value) if value.trim().is_empty() => return None,
                Some(value) => return Some(self.expand(value.trim())),
                None => {}
            }
        }
        self.directory().map(|directory| directory.join(name))
    }

    /// Both shared files, as last read.
    fn files(&self) -> Arc<Files> {
        let mut held = lock(&self.inner.files);
        Arc::clone(&held.get_or_insert_with(|| self.read_files()).parsed)
    }

    /// Read both shared files again when either moved on disk since they
    /// were read, so a session living across an edit - a set dumped anew,
    /// a profile changed - signs with what the files say now. A walk calls
    /// it, never a request: two `metadata` calls a walk.
    fn reread_files(&self) {
        let versions = self.file_versions();
        let mut held = lock(&self.inner.files);
        if held.as_ref().is_none_or(|read| read.versions != versions) {
            let moved = held.is_some();
            *held = Some(self.read_files());
            if moved {
                // A key a store did not recognize may be one the edit fixed.
                lock(&self.inner.refused).retain(|refused| refused.until.is_none());
            }
        }
    }

    /// Whether either shared file moved on disk since it was last read.
    fn files_moved(&self) -> bool {
        let versions = self.file_versions();
        lock(&self.inner.files)
            .as_ref()
            .is_some_and(|read| read.versions != versions)
    }

    /// What the file system says of the shared files the session reads
    /// from disk.
    fn file_versions(&self) -> [Option<Version>; 2] {
        let knobs = &self.inner.knobs;
        let version = |text: Option<&String>, path: Option<PathBuf>| {
            if text.is_some() || !self.reads_environment() {
                return None;
            }
            let metadata = std::fs::metadata(path?).ok()?;
            Some(Version {
                length: metadata.len(),
                modified: metadata.modified().ok(),
            })
        };
        [
            version(knobs.config_text.as_ref(), self.config_file()),
            version(knobs.credentials_text.as_ref(), self.credentials_file()),
        ]
    }

    /// Both shared files, read now. The versions are taken before the
    /// bytes, so a write landing between the two is read again by the next
    /// walk rather than missed.
    fn read_files(&self) -> SharedFiles {
        let knobs = &self.inner.knobs;
        let versions = self.file_versions();
        let mut unreadable = Vec::new();
        let mut read = |text: Option<&String>, path: Option<PathBuf>, what: &str| {
            if let Some(text) = text {
                return Some(text.clone());
            }
            if !self.reads_environment() {
                return None;
            }
            let path = path?;
            match std::fs::read(&path) {
                Ok(bytes) => Some(decode_shared_file(&bytes)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => {
                    unreadable.push(format!(
                        "the {what} {} could not be read: {error}",
                        path.display()
                    ));
                    None
                }
            }
        };
        let config = read(
            knobs.config_text.as_ref(),
            self.config_file(),
            "configuration file",
        );
        let credentials = read(
            knobs.credentials_text.as_ref(),
            self.credentials_file(),
            "credentials file",
        );
        SharedFiles {
            versions,
            parsed: Arc::new(Files::parse(config.as_deref(), credentials.as_deref())),
            unreadable,
        }
    }

    /// The profile [`Self::profile_name`] names, when either file holds it.
    pub fn profile(&self) -> Option<Profile> {
        self.files().profile(&self.profile_name())
    }

    /// Every profile either file holds, in name order.
    pub fn available_profiles(&self) -> Vec<String> {
        self.files().names()
    }

    /// The region: what was stated, else `AWS_REGION`, else
    /// `AWS_DEFAULT_REGION`, else the profile's.
    ///
    /// The instance metadata service also knows one, and
    /// [`Self::instance_region`] asks it; that costs a request, so it is a
    /// question of its own rather than the last step of this one.
    pub fn region(&self) -> Option<String> {
        self.region_and_source().map(|(region, _)| region)
    }

    /// [`Self::region`], and where it came from, for a refusal of a region
    /// that names no host to say which setting to fix.
    fn region_and_source(&self) -> Option<(String, &'static str)> {
        if let Some(region) = &self.inner.knobs.region {
            return Some((region.clone(), "Session::with_region"));
        }
        ["AWS_REGION", "AWS_DEFAULT_REGION"]
            .into_iter()
            .find_map(|variable| Some((self.variable(variable)?, variable)))
            .or_else(|| {
                Some((
                    self.profile()?.region()?.to_owned(),
                    "the region of the profile",
                ))
            })
    }

    /// The region the instance metadata service reports, asked now.
    ///
    /// `None` off an instance, or when the service is disabled.
    pub fn instance_region(&self) -> Option<String> {
        self.imds()?.region(self.http().ok()?)
    }

    /// The endpoint `service` is reached at, when one was stated or
    /// configured, in botocore's order:
    ///
    /// 1. what was stated for it ([`Self::with_service_endpoint_url`]), then
    ///    for every service ([`Self::with_endpoint_url`]);
    /// 2. `AWS_ENDPOINT_URL_<SERVICE>`, then `AWS_ENDPOINT_URL`;
    /// 3. the `endpoint_url` of the service's entry in the `[services]`
    ///    section the profile names, then the profile's own `endpoint_url`.
    ///
    /// Steps 2 and 3 are skipped under `AWS_IGNORE_CONFIGURED_ENDPOINT_URLS`,
    /// else the profile's `ignore_configured_endpoint_urls`, each read
    /// through the crate's one boolean table - wider than botocore's, which
    /// takes only `true` - and a blank variable read as unset, so the profile
    /// is read next; step 1 is never skipped. A blank variable or value is
    /// no source. The endpoint is kept as given, path included, its trailing
    /// `/` dropped.
    ///
    /// `service` is the service id botocore names the variable and the
    /// entry after - `S3`, `STS`, `SSO`, `SSO OIDC`, `Signin`, `S3Tables`,
    /// `Secrets Manager` - in any case, a hyphen or a space read as `_`:
    /// `sso-oidc` reads `AWS_ENDPOINT_URL_SSO_OIDC` and the `[services]` key
    /// `sso_oidc`. It is never the endpoint prefix (`oidc`, `portal.sso`) or
    /// the signing name. `Ok(None)` means the service's published host.
    ///
    /// ```
    /// use yggdryl::aws::Session;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let session = Session::new()
    ///     .with_variables([
    ///         ("AWS_ENDPOINT_URL_SSO_OIDC", "http://localhost:4566/oidc/"),
    ///         ("AWS_ENDPOINT_URL", "http://localhost:4566"),
    ///     ])
    ///     .with_directory("/nonexistent/.aws");
    /// assert_eq!(
    ///     session.endpoint_url("SSO OIDC")?.as_deref(),
    ///     Some("http://localhost:4566/oidc")
    /// );
    /// assert_eq!(session.endpoint_url("sts")?.as_deref(), Some("http://localhost:4566"));
    ///
    /// // A profile naming a [services] section nobody wrote is refused.
    /// let misspelt = session
    ///     .with_variables::<&str, &str>([])
    ///     .with_config_text("[default]\nservices = locl\n\n[services local]\ns3 =\n  endpoint_url = http://localhost:9000\n");
    /// assert!(misspelt.endpoint_url("s3").is_err());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// When the lookup reaches it: a configured value that names no endpoint
    /// (`/`), and a profile whose `services` names a section that defines
    /// nothing - each named, as botocore refuses them, rather than leaving
    /// the published host a typo would.
    pub fn endpoint_url(&self, service: &str) -> Result<Option<String>> {
        let key = profile::service_key(service);
        if let Some(url) = self.inner.knobs.service_endpoints.get(&key) {
            return Ok(Some(url.clone()));
        }
        if let Some(url) = &self.inner.knobs.endpoint_url {
            return Ok(Some(url.clone()));
        }
        let profile = self.profile();
        let ignored = self
            .inner
            .knobs
            .environment
            .flag("AWS_IGNORE_CONFIGURED_ENDPOINT_URLS")
            .filter(|_| self.reads_environment())
            .or_else(|| profile.as_ref()?.flag("ignore_configured_endpoint_urls"))
            .unwrap_or(false);
        if ignored {
            return Ok(None);
        }
        let variable = format!("AWS_ENDPOINT_URL_{}", key.to_ascii_uppercase());
        for name in [variable.as_str(), "AWS_ENDPOINT_URL"] {
            if let Some(url) = self.variable(name) {
                return configured(&url, || name.to_owned()).map(Some);
            }
        }
        let Some(profile) = profile else {
            return Ok(None);
        };
        if let Some(url) = profile.service_endpoint_url(&key)? {
            return configured(url, || {
                format!(
                    "the endpoint_url of {key} in the [services] section the profile {} names",
                    profile.name()
                )
            })
            .map(Some);
        }
        profile
            .endpoint_url()
            .map(|url| {
                configured(url, || {
                    format!("the endpoint_url of the profile {}", profile.name())
                })
            })
            .transpose()
    }

    /// Whether the FIPS endpoints are used.
    pub fn use_fips_endpoint(&self) -> bool {
        self.inner
            .knobs
            .use_fips_endpoint
            .or_else(|| self.flag("AWS_USE_FIPS_ENDPOINT", "use_fips_endpoint"))
            .unwrap_or(false)
    }

    /// Whether the dual-stack endpoints are used.
    pub fn use_dualstack_endpoint(&self) -> bool {
        self.inner
            .knobs
            .use_dualstack_endpoint
            .or_else(|| self.flag("AWS_USE_DUALSTACK_ENDPOINT", "use_dualstack_endpoint"))
            .unwrap_or(false)
    }

    /// Whether STS is reached in the region rather than at the global
    /// endpoint the legacy mode keeps for the older regions: what was
    /// stated, else `AWS_STS_REGIONAL_ENDPOINTS`, else the profile's
    /// `sts_regional_endpoints`, else regional - each read through the one
    /// reading the property door reads: `regional` or `legacy`, in any case.
    ///
    /// # Errors
    ///
    /// A mode that is neither, named by the variable or the profile key it
    /// was read from, as botocore refuses it
    /// (`InvalidSTSRegionalEndpointsConfigError`) rather than reading a typo
    /// as regional.
    pub fn sts_regional_endpoints(&self) -> Result<bool> {
        if let Some(regional) = self.inner.knobs.sts_regional_endpoints {
            return Ok(regional);
        }
        let (mode, source) = match self.variable("AWS_STS_REGIONAL_ENDPOINTS") {
            Some(mode) => (mode, "AWS_STS_REGIONAL_ENDPOINTS".to_owned()),
            None => match self.profile() {
                Some(profile) => match profile.get("sts_regional_endpoints") {
                    Some(mode) => (
                        mode.to_owned(),
                        format!("sts_regional_endpoints of the profile {}", profile.name()),
                    ),
                    None => return Ok(true),
                },
                None => return Ok(true),
            },
        };
        super::properties::regional(&mode).ok_or_else(|| {
            Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("expected legacy or regional for {source}, got {mode:?}"),
            ))
        })
    }

    /// The STS endpoint an exchange for `region` goes to.
    ///
    /// # Errors
    ///
    /// As [`Self::endpoint_url`] and [`Self::sts_regional_endpoints`], and a
    /// region that is not one host label where a host is built from it.
    pub fn sts_endpoint(&self, region: &str) -> Result<String> {
        Ok(self.sts_target(region, "the region asked for")?.0)
    }

    /// The STS endpoint an exchange for `region` goes to, and the region it
    /// is signed for: [`Self::service_endpoint`], under the one rule that is
    /// STS's own - the global endpoint the legacy mode keeps for the older
    /// regions, signed for `us-east-1` whatever region the caller is in.
    /// `source` names where `region` came from, for the refusal of one that
    /// names no host.
    pub(crate) fn sts_target(
        &self,
        region: &str,
        source: &'static str,
    ) -> Result<(String, String)> {
        let configured = self.endpoint_url("sts")?;
        let regional = self.sts_regional_endpoints()?;
        if configured.is_none()
            && !regional
            && !self.use_fips_endpoint()
            && !self.use_dualstack_endpoint()
            && LEGACY_STS_REGIONS.contains(&region)
        {
            return Ok((
                "https://sts.amazonaws.com".to_owned(),
                DEFAULT_REGION.to_owned(),
            ));
        }
        let endpoint = match configured {
            Some(endpoint) => endpoint,
            None => self.published_endpoint("sts", region, source)?,
        };
        Ok((endpoint, region.to_owned()))
    }

    /// The endpoint `service` is reached at in `region`: the one configured
    /// for it ([`Self::endpoint_url`]), else the host the partition
    /// publishes - `https://{service}[-fips].{region}.{suffix}`, the suffix
    /// the region's partition's, the dual-stack one under
    /// [`Self::use_dualstack_endpoint`], `-fips` under
    /// [`Self::use_fips_endpoint`].
    ///
    /// `service` names the service twice: as the service id the configured
    /// endpoint is looked up by (`AWS_ENDPOINT_URL_S3TABLES`, the
    /// `[services]` key `s3tables`), and as the endpoint prefix its published
    /// host is built from. The door fits a service whose two names agree
    /// once folded: `s3tables`, `sts`, `glue`. One whose names differ, such
    /// as SSO OIDC (`oidc`), SSO (`portal.sso`) or Secrets Manager
    /// (`secretsmanager`), asks [`Self::endpoint_url`] by its service id and
    /// builds its own host, as does a service whose hosts follow another
    /// shape: S3, the Sign-In service. Neither name is always the SigV4
    /// signing name.
    ///
    /// ```
    /// use yggdryl::aws::Session;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let session = Session::new().with_environment(false);
    /// assert_eq!(
    ///     session.service_endpoint("s3tables", "eu-west-3")?,
    ///     "https://s3tables.eu-west-3.amazonaws.com"
    /// );
    /// assert_eq!(
    ///     session.with_use_fips_endpoint(true).service_endpoint("s3tables", "us-gov-west-1")?,
    ///     "https://s3tables-fips.us-gov-west-1.amazonaws.com"
    /// );
    /// assert_eq!(
    ///     session
    ///         .with_service_endpoint_url("s3tables", "http://localhost:4566/")
    ///         .service_endpoint("s3tables", "eu-west-3")?,
    ///     "http://localhost:4566"
    /// );
    ///
    /// // `AWS_ENDPOINT_URL_S3TABLES`, named after the service id `S3Tables`.
    /// let configured = Session::new()
    ///     .with_variables([("AWS_ENDPOINT_URL_S3TABLES", "http://localhost:4566/tables/")])
    ///     .with_directory("/nonexistent/.aws");
    /// assert_eq!(
    ///     configured.service_endpoint("s3tables", "eu-west-3")?,
    ///     "http://localhost:4566/tables"
    /// );
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// As [`Self::endpoint_url`], and a `region` that is not one host label
    /// where the published host is built from it: a `/`, a `.` or an `@`
    /// in it would send the request - its signature and its session token -
    /// to another host.
    pub fn service_endpoint(&self, service: &str, region: &str) -> Result<String> {
        match self.endpoint_url(service)? {
            Some(url) => Ok(url),
            None => self.published_endpoint(service, region, "the region asked for"),
        }
    }

    /// The host the region's partition publishes for `service`, as a URL;
    /// `region` refused naming `source` unless it is one host label.
    fn published_endpoint(
        &self,
        service: &str,
        region: &str,
        source: &'static str,
    ) -> Result<String> {
        ArnPartition::check_region(region, source)?;
        let host = ArnPartition::from_region(region).service_host(
            service,
            region,
            self.use_fips_endpoint(),
            self.use_dualstack_endpoint(),
        );
        Ok(format!("https://{host}"))
    }

    /// The attempts per request the environment or the profile name, as
    /// `AWS_MAX_ATTEMPTS` and `max_attempts` do.
    pub fn max_attempts(&self) -> Option<u32> {
        self.variable("AWS_MAX_ATTEMPTS")
            .or_else(|| self.profile()?.get("max_attempts").map(str::to_owned))
            .and_then(|value| integer_from_text_as::<u32>(&value))
            .filter(|attempts| *attempts > 0)
    }

    /// The PEM bundle of trusted certificate authorities: what was stated,
    /// else `AWS_CA_BUNDLE`, else the profile's `ca_bundle`; a configured
    /// one expanded as `expand` does.
    pub fn ca_bundle(&self) -> Option<PathBuf> {
        self.inner.knobs.ca_bundle.clone().or_else(|| {
            self.variable("AWS_CA_BUNDLE")
                .or_else(|| self.profile()?.get("ca_bundle").map(str::to_owned))
                .map(|path| self.expand(&path))
        })
    }

    /// The TLS configuration the bundle asks for, when one is named.
    ///
    /// # Errors
    ///
    /// A bundle that cannot be read, or holds no certificate: trust is never
    /// widened to the platform's roots by a bundle nobody could read.
    #[cfg(feature = "s3")]
    pub(crate) fn tls_config(&self) -> Result<Option<ureq::tls::TlsConfig>> {
        let Some(path) = self.ca_bundle() else {
            return Ok(None);
        };
        let pem = std::fs::read(&path).map_err(|error| {
            refusal(format!(
                "could not read the certificate bundle {}: {error}",
                path.display()
            ))
        })?;
        let certificates: Vec<ureq::tls::Certificate<'static>> = ureq::tls::parse_pem(&pem)
            .filter_map(|item| match item {
                Ok(ureq::tls::PemItem::Certificate(certificate)) => Some(certificate),
                _ => None,
            })
            .collect();
        if certificates.is_empty() {
            return Err(refusal(format!(
                "the certificate bundle {} holds no certificate",
                path.display()
            )));
        }
        Ok(Some(
            ureq::tls::TlsConfig::builder()
                .root_certs(ureq::tls::RootCerts::Specific(Arc::new(certificates)))
                .build(),
        ))
    }

    /// A boolean the environment or the profile states.
    fn flag(&self, variable: &str, key: &str) -> Option<bool> {
        self.reads_environment()
            .then(|| self.inner.knobs.environment.flag(variable))
            .flatten()
            .or_else(|| self.profile()?.flag(key))
    }

    /// How the instance metadata service is reached, or `None` when it is
    /// disabled or the session consults no environment.
    pub(crate) fn imds(&self) -> Option<Imds> {
        let knobs = &self.inner.knobs;
        if !self.reads_environment() {
            return None;
        }
        let disabled = knobs
            .metadata_disabled
            .or_else(|| self.flag("AWS_EC2_METADATA_DISABLED", "ec2_metadata_disabled"))
            .unwrap_or(false);
        if disabled {
            return None;
        }
        let profile = self.profile();
        let setting = |variable: &str, key: &str| {
            self.variable(variable)
                .or_else(|| profile.as_ref()?.get(key).map(str::to_owned))
        };
        let endpoint = knobs
            .metadata_endpoint
            .clone()
            .or_else(|| {
                setting(
                    "AWS_EC2_METADATA_SERVICE_ENDPOINT",
                    "ec2_metadata_service_endpoint",
                )
                .and_then(|url| endpoint(&url))
            })
            .unwrap_or_else(|| {
                // The mode names the family; the older `imds_use_ipv6`
                // switch is read only where no mode is stated, as the AWS
                // tools read it.
                let ipv6 = match setting(
                    "AWS_EC2_METADATA_SERVICE_ENDPOINT_MODE",
                    "ec2_metadata_service_endpoint_mode",
                ) {
                    Some(mode) => mode.eq_ignore_ascii_case("ipv6"),
                    None => self
                        .flag("AWS_IMDS_USE_IPV6", "imds_use_ipv6")
                        .unwrap_or(false),
                };
                if ipv6 {
                    metadata::IPV6_ENDPOINT
                } else {
                    metadata::IPV4_ENDPOINT
                }
                .to_owned()
            });
        let defaults = Imds::default();
        Some(Imds {
            endpoint,
            timeout: knobs
                .metadata_timeout
                .or_else(|| {
                    setting("AWS_METADATA_SERVICE_TIMEOUT", "metadata_service_timeout")
                        .and_then(|seconds| duration_from_text(&seconds))
                        .filter(|timeout| !timeout.is_zero())
                })
                .unwrap_or(defaults.timeout),
            attempts: knobs
                .metadata_attempts
                .or_else(|| {
                    setting(
                        "AWS_METADATA_SERVICE_NUM_ATTEMPTS",
                        "metadata_service_num_attempts",
                    )
                    .and_then(|count| integer_from_text_as::<u32>(&count))
                })
                .unwrap_or(defaults.attempts)
                .max(1),
            v1_allowed: !self
                .flag("AWS_EC2_METADATA_V1_DISABLED", "ec2_metadata_v1_disabled")
                .unwrap_or(false),
        })
    }

    /// The HTTP session the identity services are reached through, built
    /// once: no redirect followed - a credential header never reaches
    /// another host - no cookie kept, no `.netrc` credential added, the
    /// process's proxy settings read only by a session reading the process
    /// environment, and the CA bundle the session names trusted alone.
    ///
    /// # Errors
    ///
    /// A CA bundle that cannot be read or holds no certificate: every
    /// identity request refuses naming it, rather than trusting the
    /// platform's roots in its place.
    pub(crate) fn http(&self) -> Result<&crate::http::Session> {
        self.inner
            .http
            .get_or_init(|| {
                let process = matches!(self.inner.knobs.environment, Environment::Process);
                let mut options = crate::http::HttpOptions::default()
                    .with_read_environment(self.reads_environment() && process)
                    .with_netrc(false)
                    .with_cookies(false)
                    .with_follow_redirects(false)
                    .with_connect_timeout(CONNECT_TIMEOUT)
                    .with_max_body_size(MAX_IDENTITY_ANSWER);
                if let Some(bundle) = self.ca_bundle() {
                    options = options.with_ca_bundle(bundle);
                }
                crate::http::Session::with_options(options)
                    .map_err(|error| format!("the identity services cannot be reached: {error}"))
            })
            .as_ref()
            .map_err(|message| refusal(message.clone()))
    }

    // --- credentials --------------------------------------------------------

    /// The credential set to sign with at `now`, or `None` for unsigned
    /// requests.
    ///
    /// The first call walks the chain; later calls answer from the set in
    /// hand until a temporary one nears its expiry, when the chain is walked
    /// again - and a walk that fails while the set in hand still stands keeps
    /// that set, tries again after a pause, and fails only once the set has
    /// lapsed. A chain that finds nothing configured is anonymous rather than
    /// an error, because a public bucket is a valid destination.
    ///
    /// # Errors
    ///
    /// Returns a refusal naming the configured source that failed and why -
    /// beside every set the walk passed over and every source it found
    /// nothing configured in - when that source ended the walk, or when
    /// every set a source answered was passed over.
    pub fn credentials(&self, now: SystemTime) -> Result<Option<Credentials>> {
        // A hold - nothing configured, or a failure inside its pause - is a
        // promise about the files as they stood; one edited since - a set
        // dumped anew, a profile fixed - breaks it, so this ask walks again.
        // Two `metadata` calls, paid only while a hold stands.
        if self.inner.lease.is_holding(now) && self.files_moved() {
            self.inner.lease.invalidate();
        }
        let found = self.inner.lease.get(now, || self.walk(now))?;
        Ok(found.map(|found| found.credentials))
    }

    /// Which source answered the set in hand - `environment`, `sso`,
    /// `instance metadata` and the rest - when one has.
    pub fn credential_source(&self) -> Option<&'static str> {
        self.inner.lease.peek().map(|found| found.source)
    }

    /// Forget the set in hand and the shared files as read, so the next
    /// request reads the files again and walks the chain again.
    ///
    /// What a client does when a store answers that the set it was handed has
    /// expired before the session thought it would.
    pub fn invalidate(&self) {
        self.inner.skip_caches.store(true, Ordering::Relaxed);
        *lock(&self.inner.files) = None;
        self.inner.lease.invalidate();
    }

    /// Record that a store refused the key `access_key_id` - lapsed,
    /// revoked or unknown - and [`Self::invalidate`] when that key is the
    /// one in hand, so a client refused with one set does not discard the
    /// fresh one another client already obtained.
    ///
    /// A refused key never signs again on this session: the walk passes
    /// over a source that answers it, naming it, and asks the sources
    /// after it - a set dumped anew under another key, a sign-in, the
    /// container or the instance. A set stated with
    /// [`Self::with_credentials`] is what the caller said and is still
    /// answered.
    ///
    /// Answers whether anything was forgotten.
    pub fn invalidate_if(&self, access_key_id: &str) -> bool {
        self.refuse(access_key_id, None, "a store".to_owned(), None);
        self.forget_if(access_key_id, true)
    }

    /// Remember what a refusal by the service at `endpoint`, signed for
    /// `region`, says of `access_key_id`: for good for a set that lapsed,
    /// for the pause for a key the service does not recognize. Answers
    /// whether the key is now held in every region.
    ///
    /// A key an opt-in region does not recognize ([`ArnPartition::is_opt_in`])
    /// is held in that region alone: such a region refuses every key until
    /// the account enables it, so the refusal says nothing of the key
    /// anywhere else.
    fn hold_refusal(
        &self,
        access_key_id: &str,
        refusal: Refusal,
        endpoint: &str,
        region: &str,
        now: SystemTime,
    ) -> bool {
        let until = match refusal {
            Refusal::Lapsed => None,
            Refusal::Unrecognized => Some(now + RETRY_PAUSE),
        };
        let by = format!("the service at {endpoint} in {region}");
        if refusal == Refusal::Unrecognized && ArnPartition::is_opt_in(region) {
            self.refuse(access_key_id, Some(region), by, until);
            return false;
        }
        self.refuse(access_key_id, None, by, until);
        true
    }

    /// Whether a request `signed` by this access key, sent to `endpoint`
    /// and signed for `region`, and refused, goes out once more: the
    /// session is told what the refusal says of the key, and answers whether
    /// it now signs with another one - the same set would be refused the
    /// same way.
    ///
    /// `code` is the error code the answer named: one that refuses the
    /// request rather than its key answers `false` and changes nothing, and
    /// so does a key an opt-in region did not recognize, which is held in
    /// that region alone - the service's own answer is the caller's. `None`
    /// is a refusal that could name none - a `HEAD` answered with no body
    /// while it carried a session token - for which the sources are read
    /// again with nothing held against the key, which may be fine.
    /// `Ok(true)` with no set left is a session that now signs nothing.
    ///
    /// # Errors
    ///
    /// The session's own refusal - every source and why - when nothing
    /// answers any more: that, rather than the store's verdict, is what the
    /// caller can act on.
    pub(crate) fn answers_another(
        &self,
        signed: &str,
        code: Option<&str>,
        endpoint: &str,
        region: &str,
        now: SystemTime,
    ) -> Result<bool> {
        match code {
            Some(code) => {
                let Some(refusal) = Refusal::from_code(code) else {
                    return Ok(false);
                };
                if !self.hold_refusal(signed, refusal, endpoint, region, now) {
                    return Ok(false);
                }
                self.forget_if(signed, true);
            }
            None => {
                self.forget_if(signed, false);
            }
        }
        let fresh = self.credentials(now)?;
        Ok(fresh.is_none_or(|fresh| fresh.access_key_id() != signed))
    }

    /// The signer of the set in hand at `now` for `service`, the SigV4
    /// signing name, in `region`; `None` for unsigned requests.
    ///
    /// One signer is kept per credential set, region and service, so the
    /// day's signing key is derived once for all the requests that share
    /// them; a set that was replaced takes its signers with it.
    ///
    /// # Errors
    ///
    /// As [`Self::credentials`].
    pub(crate) fn signer(
        &self,
        service: &str,
        region: &str,
        now: SystemTime,
    ) -> Result<Option<Arc<Signer>>> {
        let Some(credentials) = self.credentials(now)? else {
            return Ok(None);
        };
        let mut signers = lock(&self.inner.signers);
        if let Some(held) = signers.iter().find(|held| {
            held.region == region && held.service == service && held.credentials == credentials
        }) {
            return Ok(Some(Arc::clone(&held.signer)));
        }
        // Only the set in hand signs: one it replaced leaves nothing behind.
        signers.retain(|held| held.credentials == credentials);
        if signers.len() >= SIGNERS {
            signers.remove(0);
        }
        let signer = Arc::new(Signer::for_service(
            service,
            credentials.access_key_id(),
            credentials.secret_access_key(),
            credentials.session_token().map(str::to_owned),
            region,
        ));
        signers.push(Held {
            credentials,
            region: region.to_owned(),
            service: service.into(),
            signer: Arc::clone(&signer),
        });
        Ok(Some(signer))
    }

    /// Forget the set in hand and the files as read when the set in hand
    /// is `access_key_id`'s, or nothing is held; `skip_caches` says whether
    /// the next walk passes the CLI caches by too. Answers whether anything
    /// was forgotten.
    pub(crate) fn forget_if(&self, access_key_id: &str, skip_caches: bool) -> bool {
        let forget = || {
            if skip_caches {
                self.inner.skip_caches.store(true, Ordering::Relaxed);
            }
            *lock(&self.inner.files) = None;
            self.inner.lease.invalidate();
        };
        match self.inner.lease.peek() {
            Some(found) if found.credentials.access_key_id() != access_key_id => false,
            Some(_) | None => {
                forget();
                true
            }
        }
    }

    /// Sign in through IAM Identity Center now, for the sign-in stated on
    /// the session or the one its profile names, and file the token where
    /// the AWS CLI reads it.
    ///
    /// # Errors
    ///
    /// No sign-in configured, [`SsoLogin::Never`], or the service's refusal.
    pub fn login(&self) -> Result<()> {
        let Some(sso) = self.stated_sso()?.or_else(|| self.profile()?.sso().ok()?) else {
            return Err(refusal(format!(
                "neither the session nor the profile {} names an IAM Identity Center sign-in",
                self.profile_name()
            )));
        };
        let Some(directory) = self.directory() else {
            return Err(refusal(
                "no home directory to file the sign-in under: set Session::with_directory",
            ));
        };
        let oidc = match self.endpoint_url("sso-oidc")? {
            Some(url) => url,
            None => sso.oidc_endpoint(),
        };
        let token = sso::login(
            self.http()?,
            &oidc,
            &sso,
            &self.inner.knobs.sso_login,
            SystemTime::now(),
        )?;
        let path = token_cache_path(&directory, &sso);
        token.write(&path).map_err(|error| {
            refusal(format!(
                "signed in, and could not file the token at {}: {error}",
                path.display()
            ))
        })
    }

    // --- the chain ----------------------------------------------------------

    /// Walk the chain once: the first source that answers, or the refusal
    /// of the first configured source that could not, beside every set
    /// passed over and every source found absent before it.
    ///
    /// The shared files are read again first when either moved on disk,
    /// and what the walk found is logged under `yggdryl.aws.session`: each
    /// source asked at `DEBUG`, the source that answered at `INFO`, and a
    /// set passed over, or an answer found only after passing sets over,
    /// at `WARNING`. A key id is logged as its first and last four
    /// characters; nothing secret is.
    fn walk(&self, now: SystemTime) -> Result<Option<Found>> {
        self.reread_files();
        let mut report = Report::new("AWS credentials", module_path!());
        let Some((credentials, source)) = self.walk_into(&mut report, now, false) else {
            return report.conclude();
        };
        if !report.failures().is_empty() {
            log::warn!(
                "signing with the AWS credential set {source} answered, after passing over: {}",
                report.failures().join("; ")
            );
        }
        log::info!(
            "AWS credentials from {source}: key {}{}",
            credentials.key_id_hint(),
            credentials
                .expires_at()
                .map(|at| format!(", lapsing at {}", iso8601(at)))
                .unwrap_or_default()
        );
        self.inner.skip_caches.store(false, Ordering::Relaxed);
        Ok(Some(Found {
            credentials,
            source,
        }))
    }

    /// `found`, which `source` answered, when it may sign at `now`; a set
    /// that has lapsed, or whose key a store refused in every region, is
    /// recorded against `source` - with `hint`, the way out - and passed
    /// over, so the walk asks the sources after it.
    fn admit(
        &self,
        report: &mut Report,
        source: &'static str,
        found: Credentials,
        now: SystemTime,
        hint: impl FnOnce() -> String,
    ) -> Option<(Credentials, &'static str)> {
        let Some(reason) = self.unusable(&found, None, now) else {
            return Some((found, source));
        };
        let hint = hint();
        let reason = if hint.is_empty() {
            reason
        } else {
            format!("{reason}: {hint}")
        };
        log::warn!("passing over the AWS credential set {source} answered: {reason}");
        report.failed(source, reason);
        None
    }

    /// Why `found` cannot sign at `now` in `region` - `None` being every
    /// region - when it cannot: a store refused its key there, naming who
    /// and where, or it has lapsed.
    fn unusable(
        &self,
        found: &Credentials,
        region: Option<&str>,
        now: SystemTime,
    ) -> Option<String> {
        let key = found.key_id_hint();
        match self.refused_by(found.access_key_id(), region, now) {
            Some((by, None)) => {
                return Some(format!(
                    "{by} refused its key {key} as expired, revoked or unknown"
                ));
            }
            Some((by, Some(region))) => {
                return Some(format!(
                    "{by} refused its key {key}: {region} is a region the account may not have \
                     enabled, which refuses every key until it is"
                ));
            }
            None => {}
        }
        let expiry = found.expires_at().filter(|expiry| *expiry <= now)?;
        Some(format!(
            "its key {key} lapsed at {}, and this machine's clock reads {}",
            iso8601(expiry),
            iso8601(now)
        ))
    }

    /// Remember that `by` refused `access_key_id` in `region` - `None`
    /// being every region - `until` an instant or for good; a refusal for
    /// good is never shortened by a later one.
    fn refuse(
        &self,
        access_key_id: &str,
        region: Option<&str>,
        by: String,
        until: Option<SystemTime>,
    ) {
        let mut refused = lock(&self.inner.refused);
        if let Some(held) = refused
            .iter_mut()
            .find(|held| held.access_key_id == access_key_id && held.region.as_deref() == region)
        {
            held.until = held.until.zip(until).map(|(held, until)| held.max(until));
            held.by = by;
            return;
        }
        if refused.len() == REFUSED_KEYS {
            refused.pop_front();
        }
        refused.push_back(Refused {
            access_key_id: access_key_id.to_owned(),
            region: region.map(str::to_owned),
            by,
            until,
        });
    }

    /// Who refused `access_key_id` in a way that still stands at `now` in
    /// `region`, and the region the refusal is held in - `None` being every
    /// region: a refusal held in every region, or - asked for one region -
    /// one held in that region.
    fn refused_by(
        &self,
        access_key_id: &str,
        region: Option<&str>,
        now: SystemTime,
    ) -> Option<(String, Option<String>)> {
        lock(&self.inner.refused)
            .iter()
            .find(|held| {
                held.access_key_id == access_key_id
                    && held.until.is_none_or(|until| now < until)
                    && (held.region.is_none() || held.region.as_deref() == region)
            })
            .map(|held| (held.by.clone(), held.region.clone()))
    }

    /// The steps of the chain, in botocore's order.
    ///
    /// `skip_role` is set when the walk is the base of the explicit role,
    /// which wraps everything after it. A source that is configured and
    /// fails ends the walk: its failure is recorded and `None` answered, so
    /// the report concludes with it. A source with nothing configured is
    /// recorded as absent and the walk goes on. Every set a source answers
    /// passes [`Self::admit`] - a lapsed one, or one whose key a store
    /// refused, is passed over by name - except a set the caller stated,
    /// which is what the caller said.
    fn walk_into(
        &self,
        report: &mut Report,
        now: SystemTime,
        skip_role: bool,
    ) -> Option<(Credentials, &'static str)> {
        let knobs = &self.inner.knobs;
        if knobs.anonymous {
            return None;
        }
        if let (false, Some(role)) = (skip_role, &knobs.role) {
            if role.web_identity_token_file().is_some() {
                return match self.trade_web_identity(role, now) {
                    Ok(traded) => self.admit(report, "web identity", traded, now, String::new),
                    Err(error) => {
                        report.failed("web identity", error);
                        None
                    }
                };
            }
            // A source the role names - its `source_profile`, its
            // `credential_source` - signs the exchange; a role naming none
            // is signed by whatever the rest of the chain answers.
            let base = match self.source_base(role, None, now, 0) {
                Ok(Some(base)) => base,
                Ok(None) => match self.walk_into(report, now, true) {
                    Some((base, _)) => base,
                    None => {
                        report.failed(
                            "assumed role",
                            format!("found no keys to assume {} with", role.role_arn()),
                        );
                        return None;
                    }
                },
                Err(error) => {
                    report.failed("assumed role", error);
                    return None;
                }
            };
            // A role the caller named is the identity they meant; the keys
            // beneath it are not a fallback, whatever the exchange answers.
            return match self.trade(role, &base, now) {
                Ok(traded) => self.admit(report, "assumed role", traded, now, String::new),
                Err(error) => {
                    report.failed("assumed role", error);
                    None
                }
            };
        }
        if let Some(explicit) = &knobs.credentials {
            return Some((explicit.clone(), "explicit credentials"));
        }
        // A sign-in or a process the caller stated is the identity they
        // meant: its failure is the answer - a section the files do not
        // hold included.
        if knobs.sso.is_some() {
            let sso = match self.stated_sso() {
                Ok(Some(sso)) => sso,
                Ok(None) => unreachable!("a stated sign-in resolves to one"),
                Err(error) => return ended(report, "sso", error),
            };
            match self.sso_credentials(&sso, now) {
                Ok(found) => {
                    if let Some(admitted) = self.admit(report, "sso", found, now, String::new) {
                        return Some(admitted);
                    }
                }
                Err(error) => return ended(report, "sso", error),
            }
        }
        if let Some(command) = &knobs.credential_process {
            match process::run(command) {
                Ok(found) => {
                    if let Some(admitted) =
                        self.admit(report, "credential process", found, now, String::new)
                    {
                        return Some(admitted);
                    }
                }
                Err(error) => return ended(report, "credential process", error),
            }
        }
        if !self.reads_environment() {
            return None;
        }
        let env = &knobs.environment;
        let name = self.profile_name();
        for problem in self.unreadable_files() {
            report.failed("shared files", problem);
        }
        let profile = self.files().profile(&name);
        if profile.is_none()
            && let Some((_, by)) = self.named_profile()
        {
            // A profile somebody named and nobody wrote ends the walk before
            // any other source - the environment's keys included - signs in
            // its place, as botocore's `ProfileNotFound` does.
            return ended(
                report,
                "profile",
                format!(
                    "the profile {name} that {by} names is not in {} or {}",
                    describe(self.config_file()),
                    describe(self.credentials_file())
                ),
            );
        }
        if knobs.profile.is_none() {
            match environment_credentials(env) {
                Ok(Some(found)) => {
                    let hint = || {
                        "set AWS_ACCESS_KEY_ID and its companions anew, or unset them".to_owned()
                    };
                    if let Some(admitted) = self.admit(report, "environment", found, now, hint) {
                        return Some(admitted);
                    }
                }
                Ok(None) => report.absent("environment"),
                Err(error) => return ended(report, "environment", error),
            }
        }

        let Some(profile) = profile else {
            report.absent(&format!(
                "profile {name} (not in {} or {})",
                describe(self.config_file()),
                describe(self.credentials_file())
            ));
            if let Some(step) = self.walk_web_identity(report, env, None, now) {
                return step;
            }
            return self.walk_machine(report, env, now);
        };
        // A profile that names a role means that role: when the exchange
        // fails, the profile's own keys are not a fallback, because they are
        // another identity - often the very one that was to be traded away.
        match profile.assumed_role() {
            Ok(Some(role)) => {
                return match self.trade_profile_role(&role, &profile, now, 0) {
                    Ok(found) => self.admit(report, "assumed role", found, now, String::new),
                    Err(error) => ended(report, "assumed role", error),
                };
            }
            Ok(None) => {}
            Err(error) => return ended(report, "assumed role", error),
        }
        self.walk_profile(report, env, &profile, now)
    }

    /// The steps of the chain from web identity on, for the `profile` the
    /// session reads: web identity, then the profile's sign-in, keys,
    /// console session, process and configuration keys, then what the
    /// machine answers ([`Self::walk_machine`]).
    fn walk_profile(
        &self,
        report: &mut Report,
        env: &Environment,
        profile: &Profile,
        now: SystemTime,
    ) -> Option<(Credentials, &'static str)> {
        if let Some(step) = self.walk_web_identity(report, env, Some(profile), now) {
            return step;
        }
        let mut configured = false;
        match profile.sso() {
            Ok(Some(sso)) => {
                configured = true;
                match self.sso_credentials(&sso, now) {
                    Ok(found) => {
                        if let Some(admitted) = self.admit(report, "sso", found, now, String::new) {
                            return Some(admitted);
                        }
                    }
                    Err(error) => return ended(report, "sso", error),
                }
            }
            Ok(None) => {}
            Err(error) => return ended(report, "sso", error),
        }
        let dump_anew = |file: Option<PathBuf>| {
            move || {
                format!(
                    "write a fresh set under [{}] in {}, or sign in again",
                    profile.name(),
                    describe(file)
                )
            }
        };
        match profile.credential_file_credentials() {
            Ok(Some(found)) => {
                configured = true;
                let hint = dump_anew(self.credentials_file());
                if let Some(admitted) =
                    self.admit(report, "shared credentials file", found, now, hint)
                {
                    return Some(admitted);
                }
            }
            Ok(None) => {}
            Err(error) => {
                return ended(
                    report,
                    "shared credentials file",
                    partial(error, profile, self.credentials_file()),
                );
            }
        }
        if let Some(login_session) = profile.login_session() {
            configured = true;
            match self.login_credentials(profile, login_session, now) {
                Ok(found) => {
                    let hint =
                        || format!("sign in again: `aws login --profile {}`", profile.name());
                    if let Some(admitted) = self.admit(report, "login", found, now, hint) {
                        return Some(admitted);
                    }
                }
                Err(error) => return ended(report, "login", error),
            }
        }
        if let Some(command) = profile.credential_process() {
            configured = true;
            match process::run(command) {
                Ok(found) => {
                    if let Some(admitted) =
                        self.admit(report, "credential process", found, now, String::new)
                    {
                        return Some(admitted);
                    }
                }
                Err(error) => return ended(report, "credential process", error),
            }
        }
        match profile.config_file_credentials() {
            Ok(Some(found)) => {
                configured = true;
                let hint = dump_anew(self.config_file());
                if let Some(admitted) = self.admit(report, "config file", found, now, hint) {
                    return Some(admitted);
                }
            }
            Ok(None) => {}
            Err(error) => {
                return ended(
                    report,
                    "config file",
                    partial(error, profile, self.config_file()),
                );
            }
        }
        if !configured {
            report.absent(&format!(
                "profile {} (no keys, role, sign-in or process)",
                profile.name()
            ));
        }
        self.walk_machine(report, env, now)
    }

    /// The web identity step: `AWS_WEB_IDENTITY_TOKEN_FILE`, `AWS_ROLE_ARN`
    /// and `AWS_ROLE_SESSION_NAME`, each read from the environment then from
    /// `profile`'s `web_identity_token_file`, `role_arn` and
    /// `role_session_name` - the environment skipped under a profile stated
    /// on the session, as botocore disables it under an explicit profile.
    ///
    /// `None` when no token file is configured, recorded as absent, or when
    /// the exchange answered a set that was passed over; `Some` with the
    /// walk's answer otherwise - the traded set, or the end of the walk.
    fn walk_web_identity(
        &self,
        report: &mut Report,
        env: &Environment,
        profile: Option<&Profile>,
        now: SystemTime,
    ) -> Option<Option<(Credentials, &'static str)>> {
        let stated = self.inner.knobs.profile.is_some();
        let read = |variable: &str, key: &str| {
            (!stated)
                .then(|| env.get(variable))
                .flatten()
                .map(|value| (value, variable.to_owned()))
                .or_else(|| {
                    let profile = profile?;
                    Some((
                        profile.get(key)?.to_owned(),
                        format!("{key} of the profile {}", profile.name()),
                    ))
                })
        };
        let Some((file, file_source)) =
            read("AWS_WEB_IDENTITY_TOKEN_FILE", "web_identity_token_file")
        else {
            report.absent("web identity");
            return None;
        };
        let Some((arn, _)) = read("AWS_ROLE_ARN", "role_arn") else {
            return Some(ended(
                report,
                "web identity",
                format!(
                    "{file_source} names the token file {file} and nothing names the role to \
                     assume with it: set AWS_ROLE_ARN, or role_arn in the profile"
                ),
            ));
        };
        let mut role = AssumedRole::new(arn).with_web_identity_token_file(file);
        if let Some((name, _)) = read("AWS_ROLE_SESSION_NAME", "role_session_name") {
            role = role.with_session_name(name);
        }
        match self.trade_web_identity(&role, now) {
            Ok(found) => self
                .admit(report, "web identity", found, now, String::new)
                .map(Some),
            Err(error) => Some(ended(report, "web identity", error)),
        }
    }

    /// The steps of the chain no profile configures: the legacy boto files,
    /// the container endpoint and the instance metadata service.
    fn walk_machine(
        &self,
        report: &mut Report,
        env: &Environment,
        now: SystemTime,
    ) -> Option<(Credentials, &'static str)> {
        if let Some(found) = self.legacy_credentials(env)
            && let Some(admitted) = self.admit(report, "boto config", found, now, String::new)
        {
            return Some(admitted);
        }
        match self
            .http()
            .and_then(|http| container::credentials(http, env))
        {
            Ok(Some(found)) => {
                if let Some(admitted) = self.admit(report, "container", found, now, String::new) {
                    return Some(admitted);
                }
            }
            Ok(None) => report.absent("container"),
            Err(error) => return ended(report, "container", error),
        }
        match self.imds() {
            Some(imds) => match self.http().and_then(|http| imds.credentials(http, now)) {
                Ok(Some(found)) => {
                    if let Some(admitted) =
                        self.admit(report, "instance metadata", found, now, String::new)
                    {
                        return Some(admitted);
                    }
                }
                Ok(None) => report.absent("instance metadata"),
                Err(error) => report.failed("instance metadata", error),
            },
            None => report.absent("instance metadata (disabled)"),
        }
        None
    }

    /// Why a shared file that is there could not be read, as last read.
    fn unreadable_files(&self) -> Vec<String> {
        lock(&self.inner.files)
            .as_ref()
            .map(|read| read.unreadable.clone())
            .unwrap_or_default()
    }

    /// The key pair the legacy boto files spell, when the environment names
    /// one or the machine has one.
    ///
    /// The machine's own `/etc/boto.cfg` and `~/.boto` are read only under
    /// the process environment: an environment a caller handed over names
    /// its files itself, through `BOTO_CONFIG` and `AWS_CREDENTIAL_FILE`.
    fn legacy_credentials(&self, env: &Environment) -> Option<Credentials> {
        if let Some(path) = env.get("AWS_CREDENTIAL_FILE")
            && let Some(found) = std::fs::read_to_string(path)
                .ok()
                .and_then(|text| profile::ec2_credential_file(&text))
        {
            return Some(found);
        }
        let paths: Vec<PathBuf> = match (env.get("BOTO_CONFIG"), env) {
            (Some(path), _) => vec![self.expand(&path)],
            (None, Environment::Given(_)) => Vec::new(),
            (None, Environment::Process) => {
                let mut paths = vec![PathBuf::from("/etc/boto.cfg")];
                if let Some(home) = self.home() {
                    paths.push(home.join(".boto"));
                }
                paths
            }
        };
        paths.into_iter().find_map(|path| {
            std::fs::read_to_string(path)
                .ok()
                .and_then(|text| profile::boto_config(&text))
        })
    }

    /// Trade for the role a profile names, finding the keys that sign the
    /// exchange where the profile says.
    fn trade_profile_role(
        &self,
        role: &AssumedRole,
        profile: &Profile,
        now: SystemTime,
        depth: usize,
    ) -> Result<Credentials> {
        if depth > MAX_PROFILE_DEPTH {
            return Err(refusal(format!(
                "the source_profile chain through {} is longer than {MAX_PROFILE_DEPTH}, which is a cycle",
                profile.name()
            )));
        }
        if role.web_identity_token_file().is_some() {
            return self.trade_web_identity(role, now);
        }
        let base = self
            .source_base(role, Some(profile), now, depth)?
            .ok_or_else(|| {
                refusal(format!(
                    "the profile {} names role_arn without a source",
                    profile.name()
                ))
            })?;
        self.trade(role, &base, now)
    }

    /// The keys `role`'s `source_profile` or `credential_source` names,
    /// `owner` being the profile that named the role; `None` when it names
    /// neither.
    fn source_base(
        &self,
        role: &AssumedRole,
        owner: Option<&Profile>,
        now: SystemTime,
        depth: usize,
    ) -> Result<Option<Credentials>> {
        let env = &self.inner.knobs.environment;
        let who = || match owner {
            Some(owner) => format!("the profile {}", owner.name()),
            None => format!("the role {}", role.role_arn()),
        };
        let base = match (role.source_profile(), role.credential_source()) {
            (Some(source), _) => match owner.filter(|owner| owner.name() == source) {
                // A profile naming itself signs with its own keys.
                Some(owner) => owner.credentials()?.ok_or_else(|| {
                    refusal(format!(
                        "the profile {source} names itself as source_profile and holds no keys"
                    ))
                })?,
                None => {
                    let source_profile = self.files().profile(source).ok_or_else(|| {
                        refusal(format!(
                            "{} names source_profile {source}, which is in neither file",
                            who()
                        ))
                    })?;
                    self.profile_base(&source_profile, now, depth + 1)?
                }
            },
            // The three sources read the environment or the machine, which
            // a session told to consult neither does not.
            (None, Some(_)) if !self.reads_environment() => {
                return Err(refusal(format!(
                    "{} names credential_source, and the session consults no environment",
                    who()
                )));
            }
            (None, Some(CredentialSource::Environment)) => environment_credentials(env)?
                .ok_or_else(|| {
                    refusal("credential_source Environment names no AWS_ACCESS_KEY_ID")
                })?,
            (None, Some(CredentialSource::Ec2InstanceMetadata)) => self
                .imds()
                .ok_or_else(|| {
                    refusal("credential_source Ec2InstanceMetadata, and the service is disabled")
                })?
                .credentials(self.http()?, now)?
                .ok_or_else(|| {
                    refusal("credential_source Ec2InstanceMetadata, and the service did not answer")
                })?,
            (None, Some(CredentialSource::EcsContainer)) => {
                container::credentials(self.http()?, env)?.ok_or_else(|| {
                    refusal(
                        "credential_source EcsContainer, and no container endpoint is configured",
                    )
                })?
            }
            (None, None) => return Ok(None),
        };
        Ok(Some(base))
    }

    /// The keys a source profile lends the exchange it is named for: its own
    /// keys, else its role, its sign-in, its console session or its process.
    ///
    /// Keys come first, even beside a `role_arn`, as botocore's
    /// `_resolve_credentials_from_profile` takes a source profile's static
    /// keys before its role: one exchange, signed by the keys the file holds,
    /// rather than a second principal traded for first. The profile the
    /// session reads trades its role first ([`Self::walk_into`]).
    fn profile_base(
        &self,
        profile: &Profile,
        now: SystemTime,
        depth: usize,
    ) -> Result<Credentials> {
        if let Some(found) = profile.credentials()? {
            return Ok(found);
        }
        if let Some(role) = profile.assumed_role()? {
            return self.trade_profile_role(&role, profile, now, depth);
        }
        if let Some(sso) = profile.sso()? {
            return self.sso_credentials(&sso, now);
        }
        if let Some(login_session) = profile.login_session() {
            return self.login_credentials(profile, login_session, now);
        }
        if let Some(command) = profile.credential_process() {
            return process::run(command);
        }
        Err(refusal(format!(
            "the profile {} holds no keys, role, sign-in or process",
            profile.name()
        )))
    }

    /// The set the console sign-in `login_session` names, which `aws login`
    /// filed for `profile`: read from its cache, and refreshed at the Sign-In
    /// service of the session's region while it nears its end - or at once,
    /// whatever its expiry says, when a store refused the set in hand.
    fn login_credentials(
        &self,
        profile: &Profile,
        login_session: &str,
        now: SystemTime,
    ) -> Result<Credentials> {
        let cache = self
            .variable("AWS_LOGIN_CACHE_DIRECTORY")
            .map(|path| self.expand(&path))
            .or_else(|| {
                self.directory()
                    .map(|directory| directory.join("login").join("cache"))
            })
            .ok_or_else(|| {
                refusal(
                    "no home directory to read the `aws login` sign-in from: set \
                     Session::with_directory or AWS_LOGIN_CACHE_DIRECTORY",
                )
            })?;
        let arn = Arn::from_str(login_session.trim()).map_err(|error| {
            refusal(format!(
                "the profile {} names login_session {login_session:?}, which is not an ARN: {error}",
                profile.name()
            ))
        })?;
        let region = self.region().unwrap_or_else(|| {
            ArnPartition::from_arn(&arn)
                .map_or(DEFAULT_REGION, ArnPartition::global_region)
                .to_owned()
        });
        let endpoint = match self.endpoint_url("signin")? {
            Some(endpoint) => endpoint,
            None => login::endpoint(
                &region,
                self.use_fips_endpoint(),
                self.use_dualstack_endpoint(),
            )
            .map_err(refusal)?,
        };
        login::credentials(
            self.http()?,
            &endpoint,
            &cache,
            login_session,
            profile.name(),
            &|key| self.refused_by(key, None, now).is_some(),
            now,
        )
    }

    /// The region an exchange for `role` is made in, and where it came
    /// from: the role's own, else [`Self::region`], else the region its
    /// partition's global services answer in.
    fn exchange_region(&self, role: &AssumedRole) -> (String, &'static str) {
        if let Some(region) = role.region() {
            return (region.to_owned(), "the region of the role");
        }
        self.region_and_source().unwrap_or_else(|| {
            (
                default_region_of(role.role_arn()).to_owned(),
                "the role's partition",
            )
        })
    }

    /// Trade `base` for `role`'s session, through the CLI cache when it holds
    /// one that lasts.
    ///
    /// A key STS refuses is held as a store's refusal is
    /// ([`Self::hold_refusal`]), and an exchange whose keys are held refused
    /// in its region is not sent: the refusal names who refused them, where.
    fn trade(
        &self,
        role: &AssumedRole,
        base: &Credentials,
        now: SystemTime,
    ) -> Result<Credentials> {
        let key = role.cache_key();
        let cache = self.cli_cache();
        if let Some(cached) = cache
            .as_deref()
            .filter(|_| !self.skips_caches())
            .and_then(|directory| sts::read_cache(directory, &key, now))
        {
            return Ok(cached);
        }
        let (region, source) = self.exchange_region(role);
        let (endpoint, signing_region) = match role.endpoint() {
            Some(endpoint) => (endpoint.to_owned(), region),
            None => self.sts_target(&region, source)?,
        };
        if let Some(reason) = self.unusable(base, Some(&signing_region), now) {
            return Err(refusal(format!(
                "the keys that would sign the exchange for {} cannot: {reason}",
                role.role_arn()
            )));
        }
        let token_code = match role.mfa_serial() {
            Some(serial) => Some(self.mfa_code(serial, role)?),
            None => None,
        };
        let traded = sts::assume(
            self.http()?,
            base,
            role,
            &role.session_name_at(now),
            token_code.as_deref(),
            &endpoint,
            &signing_region,
            now,
        )
        .inspect_err(|error| {
            // STS refusing the keys themselves - not the role - is the same
            // news a store's refusal is.
            if let Error::Remote { code, .. } = error
                && let Some(refusal) = Refusal::from_code(code)
            {
                self.hold_refusal(
                    base.access_key_id(),
                    refusal,
                    &endpoint,
                    &signing_region,
                    now,
                );
            }
        })?;
        if let Some(directory) = cache {
            sts::write_cache(&directory, &key, &traded);
        }
        Ok(traded)
    }

    /// Trade the token `role`'s file holds for `role`'s session.
    fn trade_web_identity(&self, role: &AssumedRole, now: SystemTime) -> Result<Credentials> {
        let path = role
            .web_identity_token_file()
            .ok_or_else(|| refusal("the role names no web identity token file"))?;
        let token = std::fs::read_to_string(path).map_err(|error| {
            refusal(format!(
                "could not read the web identity token file {}: {error}",
                path.display()
            ))
        })?;
        let (region, source) = self.exchange_region(role);
        let endpoint = match role.endpoint() {
            Some(endpoint) => endpoint.to_owned(),
            None => self.sts_target(&region, source)?.0,
        };
        sts::assume_with_web_identity(
            self.http()?,
            role,
            token.trim(),
            &role.session_name_at(now),
            &endpoint,
            now,
        )
    }

    /// The code the MFA device `serial` shows, asked of the session's prompt.
    fn mfa_code(&self, serial: &str, role: &AssumedRole) -> Result<String> {
        let Some(prompt) = &self.inner.knobs.mfa_prompt else {
            return Err(refusal(format!(
                "the role {} needs a code from the MFA device {serial}, and the session has no \
                 prompt to ask for one: set Session::with_mfa_prompt",
                role.role_arn()
            )));
        };
        prompt(serial).ok_or_else(|| {
            refusal(format!(
                "no code was given for the MFA device {serial}, which the role {} needs",
                role.role_arn()
            ))
        })
    }

    /// The keys the sign-in `sso` describes, through the caches the CLI
    /// shares, refreshing or signing in as the session allows.
    fn sso_credentials(&self, sso: &Sso, now: SystemTime) -> Result<Credentials> {
        let Some(directory) = self.directory() else {
            return Err(refusal(
                "no home directory to read the IAM Identity Center sign-in from: set \
                 Session::with_directory",
            ));
        };
        let cli_cache = directory.join("cli").join("cache");
        let credentials_key = sso.credentials_cache_key();
        if !self.skips_caches()
            && let Some(cached) = sts::read_cache(&cli_cache, &credentials_key, now)
        {
            return Ok(cached);
        }
        let oidc = match self.endpoint_url("sso-oidc")? {
            Some(url) => url,
            None => sso.oidc_endpoint(),
        };
        let portal = match self.endpoint_url("sso")? {
            Some(url) => url,
            None => sso.portal_endpoint(),
        };
        let token_path = token_cache_path(&directory, sso);
        let mut token = sso::Token::read(&token_path);
        if let Some(held) = token.clone() {
            if held.is_stale(now) && held.can_refresh(now) {
                match sso::refresh(self.http()?, &oidc, &held, now) {
                    Ok(fresh) => {
                        if let Err(error) = fresh.write(&token_path) {
                            log::warn!(
                                "the refreshed IAM Identity Center sign-in could not be filed at {}: {error}",
                                token_path.display()
                            );
                        }
                        token = Some(fresh);
                    }
                    Err(error) if held.is_expired(now) => {
                        log::warn!(
                            "the IAM Identity Center sign-in lapsed and could not be refreshed: {error}"
                        );
                        token = None;
                    }
                    Err(error) => {
                        log::warn!(
                            "keeping the IAM Identity Center sign-in in hand, which still stands: refreshing it failed: {error}"
                        );
                    }
                }
            } else if held.is_expired(now) {
                token = None;
            }
        }
        let token = match token {
            Some(token) => token,
            None => {
                let signed_in =
                    sso::login(self.http()?, &oidc, sso, &self.inner.knobs.sso_login, now)?;
                if let Err(error) = signed_in.write(&token_path) {
                    log::warn!(
                        "the IAM Identity Center sign-in could not be filed at {}: {error}",
                        token_path.display()
                    );
                }
                signed_in
            }
        };
        let found = sso::role_credentials(self.http()?, &portal, sso, &token)?;
        sts::write_cache(&cli_cache, &credentials_key, &found);
        Ok(found)
    }

    /// Whether the next walk passes the CLI caches by.
    fn skips_caches(&self) -> bool {
        self.inner.skip_caches.load(Ordering::Relaxed)
    }

    /// The directory the CLI caches assumed-role and SSO sessions under.
    fn cli_cache(&self) -> Option<PathBuf> {
        self.directory()
            .map(|directory| directory.join("cli").join("cache"))
    }
}

impl std::fmt::Debug for Session {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let knobs = &self.inner.knobs;
        formatter
            .debug_struct("Session")
            .field("profile", &knobs.profile)
            .field("region", &knobs.region)
            // `Credentials` redacts its own secret.
            .field("credentials", &knobs.credentials)
            .field("anonymous", &knobs.anonymous)
            .field("assumed_role", &knobs.role)
            .field("sso", &knobs.sso)
            .field("credential_process", &knobs.credential_process)
            // An endpoint may carry user information; it is never rendered.
            .field(
                "endpoint_url",
                &knobs.endpoint_url.as_deref().map(crate::fs::mask_uri),
            )
            .field("config_file", &knobs.config_file)
            .field("credentials_file", &knobs.credentials_file)
            .field("directory", &knobs.directory)
            .field("reads_environment", &knobs.reads_environment)
            .field("sso_login", &knobs.sso_login)
            .field("mfa_prompt", &knobs.mfa_prompt.as_ref().map(|_| "<set>"))
            .finish_non_exhaustive()
    }
}

/// The region a request about `arn` names when nothing names another: the
/// region its partition's global services answer in - `cn-northwest-1` for
/// a role in China - and `us-east-1` for text that names no partition.
fn default_region_of(arn: &str) -> &'static str {
    Arn::from_str(arn)
        .ok()
        .and_then(|arn| ArnPartition::from_arn(&arn))
        .map_or(DEFAULT_REGION, ArnPartition::global_region)
}

/// The cache file `sso`'s token is filed under `directory`.
fn token_cache_path(directory: &Path, sso: &Sso) -> PathBuf {
    directory
        .join("sso")
        .join("cache")
        .join(format!("{}.json", sso.token_cache_key()))
}

/// `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY`, with `AWS_SESSION_TOKEN`
/// (or the older `AWS_SECURITY_TOKEN`), `AWS_CREDENTIAL_EXPIRATION` and
/// `AWS_ACCOUNT_ID`.
///
/// # Errors
///
/// A key without its secret, or an expiry that is not an instant.
fn environment_credentials(env: &Environment) -> Result<Option<Credentials>> {
    let Some(access_key_id) = env.get("AWS_ACCESS_KEY_ID") else {
        return Ok(None);
    };
    let Some(secret_access_key) = env.get("AWS_SECRET_ACCESS_KEY") else {
        return Err(refusal(
            "AWS_ACCESS_KEY_ID is set without AWS_SECRET_ACCESS_KEY",
        ));
    };
    let mut found = Credentials::new(access_key_id, secret_access_key);
    if let Some(token) = env
        .get("AWS_SESSION_TOKEN")
        .or_else(|| env.get("AWS_SECURITY_TOKEN"))
    {
        found = found.with_session_token(token);
    }
    if let Some(account) = env.get("AWS_ACCOUNT_ID") {
        found = found.with_account_id(account);
    }
    if let Some(expiry) = env.get("AWS_CREDENTIAL_EXPIRATION") {
        let expiry = instant(&expiry).ok_or_else(|| {
            refusal(format!(
                "AWS_CREDENTIAL_EXPIRATION is not an ISO 8601 instant: {expiry:?}"
            ))
        })?;
        found = found.with_expiry(expiry);
    }
    Ok(Some(found))
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/aws/session.rs` pins and a caller cannot reach.

    /// The metadata endpoint the session would reach, or `None` when the
    /// service is disabled or the session consults no environment.
    pub fn imds_endpoint(session: &super::Session) -> Option<String> {
        session.imds().map(|imds| imds.endpoint)
    }

    /// The bound on each metadata request the session would make, or `None`
    /// when the service is disabled or the session consults no environment.
    pub fn imds_timeout(session: &super::Session) -> Option<std::time::Duration> {
        session.imds().map(|imds| imds.timeout)
    }

    /// The STS endpoint an exchange for `region` goes to and the region it
    /// is signed for.
    pub fn sts_target(session: &super::Session, region: &str) -> crate::Result<(String, String)> {
        session.sts_target(region, "the region asked for")
    }

    /// Who refused `access_key_id` in a way that still stands at `now` in
    /// `region` - `None` being every region - when someone did, and the
    /// region the refusal is held in, `None` being every region.
    pub fn refused_by(
        session: &super::Session,
        access_key_id: &str,
        region: Option<&str>,
        now: std::time::SystemTime,
    ) -> Option<(String, Option<String>)> {
        session.refused_by(access_key_id, region, now)
    }
}

/// One endpoint URL as a session keeps it: trimmed, no trailing slash, and
/// absent when empty.
fn endpoint(url: &str) -> Option<String> {
    let url = url.trim().trim_end_matches('/');
    (!url.is_empty()).then(|| url.to_owned())
}

/// A configured endpoint as a session keeps it, refused naming `source`
/// when it names none - `/` - as botocore refuses it, rather than the
/// lookup ending at the published host.
fn configured(url: &str, source: impl FnOnce() -> String) -> Result<String> {
    endpoint(url).ok_or_else(|| {
        Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("{} is {url:?}, which names no endpoint", source()),
        ))
    })
}

/// A path for a refusal, or the word for none.
fn describe(path: Option<PathBuf>) -> String {
    path.map_or_else(|| "no file".to_owned(), |path| path.display().to_string())
}

/// End a walk at `source`, which was configured and failed: its failure
/// recorded, so the report concludes with it, and nothing answered.
fn ended<T>(report: &mut Report, source: &str, error: impl std::fmt::Display) -> Option<T> {
    report.failed(source, error);
    None
}

/// A shared file's refusal of `profile`'s keys, worded for the walk: half a
/// key pair ([`profile::is_partial_set`]) says how to mend it in `file`.
fn partial(error: Error, profile: &Profile, file: Option<PathBuf>) -> String {
    if profile::is_partial_set(&error) {
        format!(
            "{error}: write the missing key beside it under the profile {} in {}, or remove \
             the half that is there",
            profile.name(),
            describe(file)
        )
    } else {
        error.to_string()
    }
}

fn refusal(message: impl Into<String>) -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        message.into(),
    ))
}

/// A shared file's text, in the charset its byte-order mark names - what
/// PowerShell writes is UTF-16 or UTF-8 behind one - else UTF-8. A stray
/// byte in one comment is no reason to read no profile at all, so what the
/// charset cannot read is replaced.
fn decode_shared_file(bytes: &[u8]) -> String {
    let (charset, mark) = Charset::from_bom(bytes).unwrap_or((Charset::Utf8, 0));
    charset.decode_lossy(&bytes[mark..]).into_owned()
}

/// The guard of a lock the session holds, whatever a panicking holder left.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
