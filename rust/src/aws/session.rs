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

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime};

use super::container;
use super::credentials::Credentials;
use super::metadata::{self, Imds};
use super::process;
use super::profile::{self, Files, Profile};
use super::sso::{self, Sso, SsoLogin};
use super::sts::{self, AssumedRole, CredentialSource};
use crate::auth::{Environment, Expiring, Lease, Report, instant};
use crate::{Error, Result};

/// The profile read when nothing names another.
const DEFAULT_PROFILE: &str = "default";
/// The region STS is reached in when nothing names one.
const DEFAULT_REGION: &str = "us-east-1";
/// Replace a temporary set this long before it lapses; a walk that fails
/// inside the window keeps the set, which still signs.
const REFRESH_WINDOW: Duration = Duration::from_secs(15 * 60);
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
/// How many `source_profile` steps a chain of profiles may take.
const MAX_PROFILE_DEPTH: usize = 8;
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
}

struct Inner {
    knobs: Knobs,
    agent: OnceLock<ureq::Agent>,
    files: OnceLock<Files>,
    /// The set in hand, refreshed before it lapses and kept while a walk
    /// that would replace it fails.
    lease: Lease<Found>,
    /// The next walk passes the CLI caches by: a store said the set they
    /// hold has lapsed, whatever their expiry says.
    skip_caches: AtomicBool,
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
/// | endpoint of a service | `with_endpoint_url` | `AWS_ENDPOINT_URL_<SERVICE>`, `AWS_ENDPOINT_URL` | `[services]`, `endpoint_url` | the published host |
/// | credentials | `with_credentials`, `with_assumed_role`, `with_sso`, `with_credential_process` | the chain | the chain | none, unsigned |
///
/// The credential chain is walked in the order botocore walks it, and every
/// source that is configured and broken is recorded and passed over rather
/// than failing the walk; the refusal, when nothing answers, names each.
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
/// assert_eq!(session.sts_endpoint("eu-west-3"), "https://sts.eu-west-3.amazonaws.com");
///
/// // A session told the environment answers from it, and nothing else.
/// let session = Session::new()
///     .with_variables([("AWS_REGION", "ap-southeast-1"), ("AWS_ENDPOINT_URL_S3", "http://localhost:9000")])
///     .with_directory("/nonexistent/.aws");
/// assert_eq!(session.region().as_deref(), Some("ap-southeast-1"));
/// assert_eq!(session.endpoint_url("s3").as_deref(), Some("http://localhost:9000"));
/// assert_eq!(session.endpoint_url("sts"), None);
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
                agent: OnceLock::new(),
                files: OnceLock::new(),
                lease: Lease::new(
                    "AWS credential set",
                    REFRESH_WINDOW,
                    RETRY_PAUSE,
                    ANONYMOUS_HOLD,
                ),
                skip_caches: AtomicBool::new(false),
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
    /// environment also takes precedence over the environment's own keys:
    /// the profile is what the caller meant.
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

    /// Reach every service at `url`, as `AWS_ENDPOINT_URL` does.
    #[must_use]
    pub fn with_endpoint_url(&self, url: impl Into<String>) -> Self {
        let url: String = url.into();
        self.modified(|knobs| knobs.endpoint_url = endpoint(&url))
    }

    /// Reach `service` - `s3`, `sts`, `sso-oidc` - at `url`, as
    /// `AWS_ENDPOINT_URL_<SERVICE>` and a `[services]` entry do.
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

    /// Whether the session states who it is - a set, a role, a sign-in or a
    /// process - rather than leaving it to the chain.
    #[cfg(feature = "s3")]
    pub(crate) fn states_identity(&self) -> bool {
        let knobs = &self.inner.knobs;
        knobs.credentials.is_some()
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
        self.inner
            .knobs
            .profile
            .clone()
            .or_else(|| self.variable("AWS_DEFAULT_PROFILE"))
            .or_else(|| self.variable("AWS_PROFILE"))
            .unwrap_or_else(|| DEFAULT_PROFILE.to_owned())
    }

    /// The directory the shared files and the caches live under: what was
    /// stated, else `~/.aws` under the home the environment names; `None`
    /// when no home is known, and when the session consults no environment.
    pub fn directory(&self) -> Option<PathBuf> {
        self.inner
            .knobs
            .directory
            .clone()
            .or_else(|| Some(self.home()?.join(".aws")))
    }

    /// The home directory the environment names: `HOME`, then
    /// `USERPROFILE`, read from the process or from the variables a caller
    /// handed over, and none for a session that consults no environment.
    fn home(&self) -> Option<PathBuf> {
        if !self.reads_environment() {
            return None;
        }
        match &self.inner.knobs.environment {
            Environment::Process => crate::local::LocalFolder::home().ok()?.path().ok(),
            given @ Environment::Given(_) => given
                .get("HOME")
                .or_else(|| given.get("USERPROFILE"))
                .map(PathBuf::from),
        }
    }

    /// The configuration file read: what was stated, else `AWS_CONFIG_FILE`,
    /// else `config` under [`Self::directory`].
    pub fn config_file(&self) -> Option<PathBuf> {
        self.inner
            .knobs
            .config_file
            .clone()
            .or_else(|| {
                self.variable("AWS_CONFIG_FILE")
                    .map(|path| profile::expand_user(&path, self.home().as_deref()))
            })
            .or_else(|| self.directory().map(|directory| directory.join("config")))
    }

    /// The credentials file read: what was stated, else
    /// `AWS_SHARED_CREDENTIALS_FILE`, else `credentials` under
    /// [`Self::directory`].
    pub fn credentials_file(&self) -> Option<PathBuf> {
        self.inner
            .knobs
            .credentials_file
            .clone()
            .or_else(|| {
                self.variable("AWS_SHARED_CREDENTIALS_FILE")
                    .map(|path| profile::expand_user(&path, self.home().as_deref()))
            })
            .or_else(|| {
                self.directory()
                    .map(|directory| directory.join("credentials"))
            })
    }

    /// Both shared files, read once.
    fn files(&self) -> &Files {
        self.inner.files.get_or_init(|| {
            let knobs = &self.inner.knobs;
            let read = |text: Option<&String>, path: fn(&Self) -> Option<PathBuf>| match text {
                Some(text) => Some(text.clone()),
                None if self.reads_environment() => {
                    // A stray byte in one comment is no reason to read no
                    // profile at all: what is not UTF-8 is replaced.
                    path(self)
                        .and_then(|path| std::fs::read(path).ok())
                        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                }
                None => None,
            };
            let config = read(knobs.config_text.as_ref(), Self::config_file);
            let credentials = read(knobs.credentials_text.as_ref(), Self::credentials_file);
            Files::parse(config.as_deref(), credentials.as_deref())
        })
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
        self.inner
            .knobs
            .region
            .clone()
            .or_else(|| self.variable("AWS_REGION"))
            .or_else(|| self.variable("AWS_DEFAULT_REGION"))
            .or_else(|| self.profile()?.region().map(str::to_owned))
    }

    /// The region the instance metadata service reports, asked now.
    ///
    /// `None` off an instance, or when the service is disabled.
    pub fn instance_region(&self) -> Option<String> {
        self.imds()?.region(self.agent())
    }

    /// The endpoint `service` is reached at, when one was configured: what
    /// was stated, else `AWS_ENDPOINT_URL_<SERVICE>`, else `AWS_ENDPOINT_URL`,
    /// else the profile's `[services]` entry, else the profile's
    /// `endpoint_url` - the last four skipped under
    /// `AWS_IGNORE_CONFIGURED_ENDPOINT_URLS` or the profile's
    /// `ignore_configured_endpoint_urls`.
    ///
    /// `service` is the service id - `s3`, `sts`, `sso-oidc` - spelled any
    /// way; `None` means the service's published host.
    pub fn endpoint_url(&self, service: &str) -> Option<String> {
        let key = profile::service_key(service);
        if let Some(url) = self.inner.knobs.service_endpoints.get(&key) {
            return Some(url.clone());
        }
        if let Some(url) = &self.inner.knobs.endpoint_url {
            return Some(url.clone());
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
            return None;
        }
        self.variable(&format!("AWS_ENDPOINT_URL_{}", key.to_ascii_uppercase()))
            .or_else(|| self.variable("AWS_ENDPOINT_URL"))
            .or_else(|| {
                profile
                    .as_ref()?
                    .service_endpoint_url(&key)
                    .map(str::to_owned)
            })
            .or_else(|| profile.as_ref()?.endpoint_url().map(str::to_owned))
            .and_then(|url| endpoint(&url))
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
    /// endpoint the legacy mode keeps for the older regions.
    pub fn sts_regional_endpoints(&self) -> bool {
        self.inner.knobs.sts_regional_endpoints.unwrap_or_else(|| {
            self.variable("AWS_STS_REGIONAL_ENDPOINTS")
                .or_else(|| {
                    self.profile()?
                        .get("sts_regional_endpoints")
                        .map(str::to_owned)
                })
                .is_none_or(|mode| !mode.eq_ignore_ascii_case("legacy"))
        })
    }

    /// The STS endpoint an exchange for `region` goes to.
    pub fn sts_endpoint(&self, region: &str) -> String {
        self.sts_target(region).0
    }

    /// The STS endpoint an exchange for `region` goes to, and the region it
    /// is signed for: the global endpoint the legacy mode keeps is signed for
    /// `us-east-1`, whatever region the caller is in.
    pub(crate) fn sts_target(&self, region: &str) -> (String, String) {
        if let Some(url) = self.endpoint_url("sts") {
            return (url, region.to_owned());
        }
        let fips = self.use_fips_endpoint();
        let dualstack = self.use_dualstack_endpoint();
        if !self.sts_regional_endpoints()
            && !fips
            && !dualstack
            && LEGACY_STS_REGIONS.contains(&region)
        {
            return (
                "https://sts.amazonaws.com".to_owned(),
                DEFAULT_REGION.to_owned(),
            );
        }
        let service = if fips { "sts-fips" } else { "sts" };
        let suffix = match (region.starts_with("cn-"), dualstack) {
            (true, _) => "amazonaws.com.cn",
            (false, true) => "api.aws",
            (false, false) => "amazonaws.com",
        };
        (
            format!("https://{service}.{region}.{suffix}"),
            region.to_owned(),
        )
    }

    /// The attempts per request the environment or the profile name, as
    /// `AWS_MAX_ATTEMPTS` and `max_attempts` do.
    pub fn max_attempts(&self) -> Option<u32> {
        self.variable("AWS_MAX_ATTEMPTS")
            .or_else(|| self.profile()?.get("max_attempts").map(str::to_owned))
            .and_then(|value| value.parse().ok())
            .filter(|attempts| *attempts > 0)
    }

    /// The PEM bundle of trusted certificate authorities: what was stated,
    /// else `AWS_CA_BUNDLE`, else the profile's `ca_bundle`.
    pub fn ca_bundle(&self) -> Option<PathBuf> {
        self.inner.knobs.ca_bundle.clone().or_else(|| {
            self.variable("AWS_CA_BUNDLE")
                .or_else(|| self.profile()?.get("ca_bundle").map(str::to_owned))
                .map(|path| profile::expand_user(&path, self.home().as_deref()))
        })
    }

    /// The TLS configuration the bundle asks for, when one is named.
    ///
    /// # Errors
    ///
    /// A bundle that cannot be read, or holds no certificate: trust is never
    /// widened to the platform's roots by a bundle nobody could read.
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
                        .and_then(|seconds| seconds.parse::<f64>().ok())
                        .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
                        .map(Duration::from_secs_f64)
                })
                .unwrap_or(defaults.timeout),
            attempts: knobs
                .metadata_attempts
                .or_else(|| {
                    setting(
                        "AWS_METADATA_SERVICE_NUM_ATTEMPTS",
                        "metadata_service_num_attempts",
                    )
                    .and_then(|count| count.parse().ok())
                })
                .unwrap_or(defaults.attempts)
                .max(1),
            v1_allowed: !self
                .flag("AWS_EC2_METADATA_V1_DISABLED", "ec2_metadata_v1_disabled")
                .unwrap_or(false),
        })
    }

    /// The HTTP client the identity services are reached through, built
    /// once.
    pub(crate) fn agent(&self) -> &ureq::Agent {
        self.inner.agent.get_or_init(|| {
            let mut builder = ureq::Agent::config_builder()
                .http_status_as_error(false)
                .max_redirects(0)
                .max_redirects_will_error(false)
                .timeout_connect(Some(CONNECT_TIMEOUT))
                .user_agent(concat!("yggdryl/", env!("CARGO_PKG_VERSION")));
            match self.tls_config() {
                Ok(Some(tls)) => builder = builder.tls_config(tls),
                Ok(None) => {}
                Err(error) => log::warn!(
                    "{error}; the identity services are reached with the platform's roots"
                ),
            }
            ureq::Agent::new_with_config(builder.build())
        })
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
    /// Returns a refusal naming every source that was configured and could
    /// not answer, when no source answered.
    pub fn credentials(&self, now: SystemTime) -> Result<Option<Credentials>> {
        let found = self.inner.lease.get(now, || {
            self.walk(now).map(|found| {
                found.map(|(credentials, source)| Found {
                    credentials,
                    source,
                })
            })
        })?;
        if found.is_some() {
            self.inner.skip_caches.store(false, Ordering::Relaxed);
        }
        Ok(found.map(|found| found.credentials))
    }

    /// Which source answered the set in hand - `environment`, `sso`,
    /// `instance metadata` and the rest - when one has.
    pub fn credential_source(&self) -> Option<&'static str> {
        self.inner.lease.peek().map(|found| found.source)
    }

    /// Forget the set in hand, so the next request walks the chain again.
    ///
    /// What a client does when a store answers that the set it was handed has
    /// expired before the session thought it would.
    pub fn invalidate(&self) {
        self.inner.skip_caches.store(true, Ordering::Relaxed);
        self.inner.lease.invalidate();
    }

    /// [`Self::invalidate`], only when the set in hand is the one whose
    /// access key id is `access_key_id` - so a client refused with one set
    /// does not discard the fresh one another client already obtained.
    ///
    /// Answers whether anything was forgotten.
    pub fn invalidate_if(&self, access_key_id: &str) -> bool {
        match self.inner.lease.peek() {
            Some(found) if found.credentials.access_key_id() == access_key_id => {
                self.invalidate();
                true
            }
            Some(_) => false,
            None => {
                self.invalidate();
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
        let Some(sso) = self
            .inner
            .knobs
            .sso
            .clone()
            .or_else(|| self.profile()?.sso().ok()?)
        else {
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
        let oidc = self
            .endpoint_url("sso-oidc")
            .unwrap_or_else(|| sso.oidc_endpoint());
        let token = sso::login(
            self.agent(),
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

    /// Walk the chain once: the first source that answers, or every source
    /// that was configured and could not.
    fn walk(&self, now: SystemTime) -> Result<Option<(Credentials, &'static str)>> {
        let mut report = Report::new("AWS credentials");
        if let Some(found) = self.walk_into(&mut report, now, false) {
            return Ok(Some(found));
        }
        report.conclude()
    }

    /// The steps of the chain, in botocore's order.
    ///
    /// `skip_role` is set when the walk is the base of the explicit role,
    /// which wraps everything after it.
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
                    Ok(traded) => Some((traded, "web identity")),
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
            return match self.trade(role, &base, now) {
                Ok(traded) => Some((traded, "assumed role")),
                Err(error) => {
                    // A role the caller named is the identity they meant; the
                    // keys beneath it are not a fallback.
                    report.failed("assumed role", error);
                    None
                }
            };
        }
        if let Some(explicit) = &knobs.credentials {
            return Some((explicit.clone(), "explicit credentials"));
        }
        if let Some(sso) = &knobs.sso {
            match self.sso_credentials(sso, now) {
                Ok(found) => return Some((found, "sso")),
                Err(error) => report.failed("sso", error),
            }
        }
        if let Some(command) = &knobs.credential_process {
            match process::run(command) {
                Ok(found) => return Some((found, "credential process")),
                Err(error) => report.failed("credential process", error),
            }
        }
        if !self.reads_environment() {
            return None;
        }
        let env = &knobs.environment;
        if knobs.profile.is_none() {
            match environment_credentials(env) {
                Ok(Some(found)) => return Some((found, "environment")),
                Ok(None) => report.absent("environment"),
                Err(error) => report.failed("environment", error),
            }
        }

        let name = self.profile_name();
        let profile = self.files().profile(&name);
        // A profile that names a role means that role: when the exchange
        // fails, the profile's own keys are not a fallback, because they are
        // another identity - often the very one that was to be traded away.
        let mut role_named = false;
        match &profile {
            None => {
                let where_ = format!(
                    "profile {name} (not in {} or {})",
                    describe(self.config_file()),
                    describe(self.credentials_file())
                );
                // A profile somebody named is a source that failed; the
                // default nobody wrote is simply not there.
                let named = knobs.profile.is_some()
                    || self.variable("AWS_DEFAULT_PROFILE").is_some()
                    || self.variable("AWS_PROFILE").is_some();
                if named {
                    report.failed("profile", format!("no {where_}"));
                } else {
                    report.absent(&where_);
                }
            }
            Some(profile) => match profile.assumed_role() {
                Ok(Some(role)) => {
                    role_named = true;
                    match self.trade_profile_role(&role, profile, now, 0) {
                        Ok(found) => return Some((found, "assumed role")),
                        Err(error) => report.failed("assumed role", error),
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    role_named = true;
                    report.failed("assumed role", error);
                }
            },
        }
        if let (Some(arn), Some(file)) = (
            env.get("AWS_ROLE_ARN"),
            env.get("AWS_WEB_IDENTITY_TOKEN_FILE"),
        ) {
            let mut role = AssumedRole::new(arn).with_web_identity_token_file(file);
            if let Some(name) = env.get("AWS_ROLE_SESSION_NAME") {
                role = role.with_session_name(name);
            }
            match self.trade_web_identity(&role, now) {
                Ok(found) => return Some((found, "web identity")),
                Err(error) => report.failed("web identity", error),
            }
        }
        if let Some(profile) = profile.as_ref().filter(|_| !role_named) {
            match profile.sso() {
                Ok(Some(sso)) => match self.sso_credentials(&sso, now) {
                    Ok(found) => return Some((found, "sso")),
                    Err(error) => report.failed("sso", error),
                },
                Ok(None) => {}
                Err(error) => report.failed("sso", error),
            }
            if let Some(found) = profile.credential_file_credentials() {
                return Some((found, "shared credentials file"));
            }
            if let Some(command) = profile.credential_process() {
                match process::run(command) {
                    Ok(found) => return Some((found, "credential process")),
                    Err(error) => report.failed("credential process", error),
                }
            }
            if let Some(found) = profile.config_file_credentials() {
                return Some((found, "config file"));
            }
        }
        if let Some(found) = self.legacy_credentials(env) {
            return Some((found, "boto config"));
        }
        match container::credentials(self.agent(), env) {
            Ok(Some(found)) => return Some((found, "container")),
            Ok(None) => report.absent("container"),
            Err(error) => report.failed("container", error),
        }
        match self.imds() {
            Some(imds) => match imds.credentials(self.agent()) {
                Ok(Some(found)) => return Some((found, "instance metadata")),
                Ok(None) => report.absent("instance metadata"),
                Err(error) => report.failed("instance metadata", error),
            },
            None => report.absent("instance metadata (disabled)"),
        }
        None
    }

    /// The key pair the legacy boto files spell, when the environment names
    /// one or the machine has one.
    ///
    /// The machine's own `/etc/boto.cfg` and `~/.boto` are read only under
    /// the process environment: an environment a caller handed over names
    /// its files itself, through `BOTO_CONFIG` and `AWS_CREDENTIAL_FILE`.
    fn legacy_credentials(&self, env: &Environment) -> Option<Credentials> {
        if let Some(path) = env.get("AWS_CREDENTIAL_FILE") {
            if let Some(found) = std::fs::read_to_string(path)
                .ok()
                .and_then(|text| profile::ec2_credential_file(&text))
            {
                return Some(found);
            }
        }
        let paths: Vec<PathBuf> = match (env.get("BOTO_CONFIG"), env) {
            (Some(path), _) => vec![profile::expand_user(&path, self.home().as_deref())],
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
                Some(owner) => owner.credentials().ok_or_else(|| {
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
                .credentials(self.agent())?
                .ok_or_else(|| {
                    refusal("credential_source Ec2InstanceMetadata, and the service did not answer")
                })?,
            (None, Some(CredentialSource::EcsContainer)) => {
                container::credentials(self.agent(), env)?.ok_or_else(|| {
                    refusal(
                        "credential_source EcsContainer, and no container endpoint is configured",
                    )
                })?
            }
            (None, None) => return Ok(None),
        };
        Ok(Some(base))
    }

    /// The keys a profile answers on its own: its role, its sign-in, its
    /// keys, or its process.
    fn profile_base(
        &self,
        profile: &Profile,
        now: SystemTime,
        depth: usize,
    ) -> Result<Credentials> {
        if let Some(role) = profile.assumed_role()? {
            return self.trade_profile_role(&role, profile, now, depth);
        }
        if let Some(sso) = profile.sso()? {
            return self.sso_credentials(&sso, now);
        }
        if let Some(found) = profile.credentials() {
            return Ok(found);
        }
        if let Some(command) = profile.credential_process() {
            return process::run(command);
        }
        Err(refusal(format!(
            "the profile {} holds no keys, role, sign-in or process",
            profile.name()
        )))
    }

    /// Trade `base` for `role`'s session, through the CLI cache when it holds
    /// one that lasts.
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
        let token_code = match role.mfa_serial() {
            Some(serial) => Some(self.mfa_code(serial, role)?),
            None => None,
        };
        let region = role
            .region()
            .map(str::to_owned)
            .or_else(|| self.region())
            .unwrap_or_else(|| DEFAULT_REGION.to_owned());
        let (endpoint, signing_region) = match role.endpoint() {
            Some(endpoint) => (endpoint.to_owned(), region),
            None => self.sts_target(&region),
        };
        let traded = sts::assume(
            self.agent(),
            base,
            role,
            &role.session_name_at(now),
            token_code.as_deref(),
            &endpoint,
            &signing_region,
            now,
        )?;
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
        let region = role
            .region()
            .map(str::to_owned)
            .or_else(|| self.region())
            .unwrap_or_else(|| DEFAULT_REGION.to_owned());
        let endpoint = role
            .endpoint()
            .map(str::to_owned)
            .unwrap_or_else(|| self.sts_endpoint(&region));
        sts::assume_with_web_identity(
            self.agent(),
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
        if !self.skips_caches() {
            if let Some(cached) = sts::read_cache(&cli_cache, &credentials_key, now) {
                return Ok(cached);
            }
        }
        let oidc = self
            .endpoint_url("sso-oidc")
            .unwrap_or_else(|| sso.oidc_endpoint());
        let portal = self
            .endpoint_url("sso")
            .unwrap_or_else(|| sso.portal_endpoint());
        let token_path = token_cache_path(&directory, sso);
        let mut token = sso::Token::read(&token_path);
        if let Some(held) = token.clone() {
            if held.is_stale(now) && held.can_refresh(now) {
                match sso::refresh(self.agent(), &oidc, &held, now) {
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
                    sso::login(self.agent(), &oidc, sso, &self.inner.knobs.sso_login, now)?;
                if let Err(error) = signed_in.write(&token_path) {
                    log::warn!(
                        "the IAM Identity Center sign-in could not be filed at {}: {error}",
                        token_path.display()
                    );
                }
                signed_in
            }
        };
        let found = sso::role_credentials(self.agent(), &portal, sso, &token)?;
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
            .field("endpoint_url", &knobs.endpoint_url)
            .field("config_file", &knobs.config_file)
            .field("credentials_file", &knobs.credentials_file)
            .field("directory", &knobs.directory)
            .field("reads_environment", &knobs.reads_environment)
            .field("sso_login", &knobs.sso_login)
            .field("mfa_prompt", &knobs.mfa_prompt.as_ref().map(|_| "<set>"))
            .finish_non_exhaustive()
    }
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
}

/// One endpoint URL as a session keeps it: trimmed, no trailing slash, and
/// absent when empty.
fn endpoint(url: &str) -> Option<String> {
    let url = url.trim().trim_end_matches('/');
    (!url.is_empty()).then(|| url.to_owned())
}

/// A path for a refusal, or the word for none.
fn describe(path: Option<PathBuf>) -> String {
    path.map_or_else(|| "no file".to_owned(), |path| path.display().to_string())
}

fn refusal(message: impl Into<String>) -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        message.into(),
    ))
}
