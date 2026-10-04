//! Reading who a process is to AWS out of a property map.
//!
//! A catalog, a URL's `with (...)` clause or a binding's keywords state an
//! AWS identity in names of their own: this crate's, the AWS tools'
//! `AWS_`-prefixed ones, or PyIceberg's `client.*`. [`Session::with_properties`]
//! is the one reader of them, so every consumer that signs an AWS request -
//! the S3 backend, a request signed through
//! [`Request::with_sigv4`](crate::http::Request::with_sigv4) - reads the same
//! names the same way. A name it does not know is ignored, because a property
//! bag carries a great deal that is nobody's identity; a value it cannot read
//! is refused naming the property.

use std::time::Duration;

use super::{AssumedRole, CredentialSource, Credentials, Session, Sso};
use crate::boolean::{BOOLEAN_SPELLINGS, bool_from_text};
use crate::duration::{DURATION_SPELLINGS, duration_from_text};
use crate::integer::{INTEGER_SPELLINGS, integer_from_text_as};
use crate::{Error, Result};

impl Session {
    /// The names [`Self::with_properties`] reads, in this crate's own
    /// vocabulary - the `aws_` and `client.` spellings and the aliases it
    /// also reads aside. What a binding suggests a mistyped keyword against.
    pub const PROPERTY_NAMES: [&'static str; 32] = [
        "region",
        "access_key_id",
        "secret_access_key",
        "session_token",
        "anonymous",
        "profile",
        "role_arn",
        "role_session_name",
        "external_id",
        "role_duration",
        "sts_region",
        "sts_endpoint",
        "mfa_serial",
        "source_profile",
        "credential_source",
        "web_identity_token_file",
        "sso_start_url",
        "sso_region",
        "sso_account_id",
        "sso_role_name",
        "sso_session",
        "config_file",
        "shared_credentials_file",
        "credential_process",
        "ca_bundle",
        "use_fips_endpoint",
        "use_dualstack_endpoint",
        "sts_regional_endpoints",
        "ec2_metadata_disabled",
        "ec2_metadata_service_endpoint",
        "metadata_service_timeout",
        "metadata_service_num_attempts",
    ];

    /// A session stating what `properties` name, and nothing else.
    ///
    /// # Errors
    ///
    /// As [`Self::with_properties`].
    pub fn from_properties<K, V>(properties: impl IntoIterator<Item = (K, V)>) -> Result<Self>
    where
        K: AsRef<str>,
        V: AsRef<str>,
    {
        Self::new().with_properties(properties)
    }

    /// This session with the AWS identity `properties` name stated on it.
    ///
    /// | states | names |
    /// | --- | --- |
    /// | the region | `region` |
    /// | a credential set | `access_key_id`, `secret_access_key`, `session_token`; `anonymous` for none |
    /// | a profile | `profile` |
    /// | a role to assume | `role_arn`, with `role_session_name`, `external_id`, `role_duration`, `sts_region`, `sts_endpoint`, `mfa_serial`, `source_profile`, `credential_source`, `web_identity_token_file` |
    /// | an IAM Identity Center sign-in | `sso_start_url`, `sso_region`, `sso_account_id`, `sso_role_name`, with `sso_session` |
    /// | the shared files and a process | `config_file`, `shared_credentials_file`, `credential_process`, `ca_bundle` |
    /// | the endpoints | `use_fips_endpoint`, `use_dualstack_endpoint`, `sts_regional_endpoints`, `sts_endpoint` |
    /// | the instance metadata service | `ec2_metadata_disabled`, `ec2_metadata_service_endpoint`, `metadata_service_timeout`, `metadata_service_num_attempts` |
    ///
    /// Names are matched loosely: case, `-`, `_` and `.` are the same, and a
    /// leading `aws_` or `client.` is dropped - so `region`, `AWS_REGION`
    /// and PyIceberg's `client.region` are one name, as are
    /// `client.access-key-id`, `client.secret-access-key`,
    /// `client.session-token`, `client.profile-name` and `client.role-arn`
    /// and the bare names they fold to. A name under another prefix - `s3.`
    /// is the object store's own - is not read here, and neither is a bare
    /// `token`, which a catalog spells its own bearer token by.
    ///
    /// A name this does not know is ignored; a later pair of one name
    /// replaces an earlier one; an empty value states nothing.
    ///
    /// ```
    /// use yggdryl::aws::Session;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// // A PyIceberg catalog's properties, most of which are nobody's identity.
    /// let session = Session::new().with_environment(false).with_properties([
    ///     ("uri", "https://s3tables.eu-west-3.amazonaws.com/iceberg"),
    ///     ("client.region", "eu-west-3"),
    ///     ("client.access-key-id", "AKIAIOSFODNN7EXAMPLE"),
    ///     ("client.secret-access-key", "wJalrXUtnFEMI"),
    ///     ("token", "a catalog's own bearer token"),
    /// ])?;
    /// assert_eq!(session.region().as_deref(), Some("eu-west-3"));
    /// let keys = session.credentials(std::time::SystemTime::now())?.expect("the stated set");
    /// assert_eq!(keys.access_key_id(), "AKIAIOSFODNN7EXAMPLE");
    /// assert_eq!(keys.session_token(), None);
    /// assert!(Session::is_property("AWS_PROFILE") && !Session::is_property("token"));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// A refusal naming the property whose value does not read as what the
    /// name means; half a credential set, naming the half that is missing;
    /// an IAM Identity Center sign-in that lacks one of its four values,
    /// naming them.
    pub fn with_properties<K, V>(
        &self,
        properties: impl IntoIterator<Item = (K, V)>,
    ) -> Result<Self>
    where
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let mut identity = Identity::default();
        for (name, value) in properties {
            let (name, value) = (name.as_ref(), value.as_ref().trim());
            if !value.is_empty() {
                identity.read(&canonical(name), name, value)?;
            }
        }
        identity.apply(self)
    }

    /// Whether `name` is a property [`Self::with_properties`] reads, in any
    /// spelling it accepts, `false` being a name it ignores - answered by the
    /// reader itself, so it cannot disagree with a read.
    #[must_use]
    pub fn is_property(name: &str) -> bool {
        Identity::default()
            .read(&canonical(name), name, "x")
            .unwrap_or(true)
    }
}

/// The identity properties collected off one map, assembled onto a session
/// once all of them are in hand: a role and a sign-in are several values.
/// The S3 options' reader collects into one too, under its own wider fold,
/// keeping the names that are also the object store's - the region, the pair,
/// `anonymous`.
#[derive(Default)]
pub(crate) struct Identity {
    region: Option<String>,
    access_key: Option<String>,
    secret_key: Option<String>,
    session_token: Option<String>,
    anonymous: Option<bool>,
    profile: Option<String>,
    role_arn: Option<String>,
    role_session: Option<String>,
    external_id: Option<String>,
    role_duration: Option<Duration>,
    sts_endpoint: Option<String>,
    sts_region: Option<String>,
    mfa_serial: Option<String>,
    source_profile: Option<String>,
    credential_source: Option<CredentialSource>,
    web_identity_token_file: Option<String>,
    credential_process: Option<String>,
    config_file: Option<String>,
    credentials_file: Option<String>,
    ca_bundle: Option<String>,
    use_fips: Option<bool>,
    use_dualstack: Option<bool>,
    sts_regional: Option<bool>,
    metadata_disabled: Option<bool>,
    metadata_endpoint: Option<String>,
    metadata_timeout: Option<Duration>,
    metadata_attempts: Option<u32>,
    sso_start_url: Option<String>,
    sso_region: Option<String>,
    sso_account_id: Option<String>,
    sso_role_name: Option<String>,
    sso_session: Option<String>,
}

impl Identity {
    /// Collect `value` under `key`, a name already folded; `name` is the
    /// property as written, for a refusal to name. Answers whether `key` is
    /// an identity property at all.
    ///
    /// # Errors
    ///
    /// A refusal naming `name` when `value` does not read as what it means.
    pub(crate) fn read(&mut self, key: &str, name: &str, value: &str) -> Result<bool> {
        let text = || Some(value.to_owned());
        match key {
            "region" => self.region = text(),
            "access_key" | "access_key_id" => self.access_key = text(),
            "secret_key" | "secret_access_key" => self.secret_key = text(),
            "session_token" => self.session_token = text(),
            "anonymous" | "no_sign_request" => self.anonymous = Some(flag(name, value)?),
            "profile" | "profile_name" => self.profile = text(),
            "role_arn" => self.role_arn = text(),
            "session_name" | "role_session_name" => self.role_session = text(),
            "external_id" | "role_external_id" => self.external_id = text(),
            "role_duration" | "role_session_duration" | "assume_role_duration_seconds" => {
                self.role_duration = Some(seconds(name, value)?);
            }
            key if EndpointName::of(key) == Some(EndpointName::Sts) => self.sts_endpoint = text(),
            "sts_region" | "role_region" => self.sts_region = text(),
            "mfa_serial" | "role_mfa_serial" => self.mfa_serial = text(),
            "source_profile" | "role_source_profile" => self.source_profile = text(),
            "credential_source" | "role_credential_source" => {
                self.credential_source = Some(value.parse::<CredentialSource>()?);
            }
            "web_identity_token_file" | "role_web_identity_token_file" => {
                self.web_identity_token_file = text();
            }
            "credential_process" => self.credential_process = text(),
            "config_file" => self.config_file = text(),
            "shared_credentials_file" => self.credentials_file = text(),
            "ca_bundle" => self.ca_bundle = text(),
            "use_fips_endpoint" | "fips_endpoint" => self.use_fips = Some(flag(name, value)?),
            "use_dualstack_endpoint" | "dualstack_endpoint" => {
                self.use_dualstack = Some(flag(name, value)?);
            }
            "sts_regional_endpoints" => self.sts_regional = Some(regional(name, value)?),
            "ec2_metadata_disabled" | "metadata_disabled" => {
                self.metadata_disabled = Some(flag(name, value)?);
            }
            key if EndpointName::of(key) == Some(EndpointName::Metadata) => {
                self.metadata_endpoint = text();
            }
            "metadata_service_timeout" | "ec2_metadata_service_timeout" => {
                self.metadata_timeout = Some(seconds(name, value)?);
            }
            "metadata_service_num_attempts" | "ec2_metadata_service_num_attempts" => {
                self.metadata_attempts = Some(count(name, value)?);
            }
            "sso_start_url" => self.sso_start_url = text(),
            "sso_region" => self.sso_region = text(),
            "sso_account_id" => self.sso_account_id = text(),
            "sso_role_name" => self.sso_role_name = text(),
            "sso_session" | "sso_session_name" => self.sso_session = text(),
            // Never a bare `token`: a catalog spells its own bearer token
            // that way, and it authorizes the catalog rather than AWS.
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// `session` with what was collected stated on it.
    ///
    /// # Errors
    ///
    /// Half a credential set, or a sign-in that lacks one of its four
    /// values, each naming what is missing.
    pub(crate) fn apply(&self, session: &Session) -> Result<Session> {
        let mut session = session.clone();
        if let Some(region) = &self.region {
            session = session.with_region(region);
        }
        match (&self.access_key, &self.secret_key) {
            (Some(access_key), Some(secret_key)) => {
                let mut credentials = Credentials::new(access_key, secret_key);
                if let Some(token) = &self.session_token {
                    credentials = credentials.with_session_token(token);
                }
                session = session.with_credentials(credentials);
            }
            (Some(_), None) => {
                return Err(refusal(
                    "a credential set is an access_key_id and a secret_access_key: \
                     the secret_access_key is missing",
                ));
            }
            (None, Some(_)) => {
                return Err(refusal(
                    "a credential set is an access_key_id and a secret_access_key: \
                     the access_key_id is missing",
                ));
            }
            (None, None) if self.session_token.is_some() => {
                return Err(refusal(
                    "a session_token is part of a credential set: \
                     the access_key_id and the secret_access_key are missing",
                ));
            }
            (None, None) => {}
        }
        if let Some(anonymous) = self.anonymous {
            session = session.with_anonymous(anonymous);
        }
        if let Some(profile) = &self.profile {
            session = session.with_profile(profile);
        }
        if let Some(command) = &self.credential_process {
            session = session.with_credential_process(command);
        }
        if let Some(path) = &self.config_file {
            session = session.with_config_file(path);
        }
        if let Some(path) = &self.credentials_file {
            session = session.with_credentials_file(path);
        }
        if let Some(path) = &self.ca_bundle {
            session = session.with_ca_bundle(path);
        }
        if let Some(fips) = self.use_fips {
            session = session.with_use_fips_endpoint(fips);
        }
        if let Some(dualstack) = self.use_dualstack {
            session = session.with_use_dualstack_endpoint(dualstack);
        }
        if let Some(regional) = self.sts_regional {
            session = session.with_sts_regional_endpoints(regional);
        }
        if let Some(disabled) = self.metadata_disabled {
            session = session.with_metadata_disabled(disabled);
        }
        if let Some(endpoint) = &self.metadata_endpoint {
            session = session.with_metadata_endpoint(endpoint);
        }
        if let Some(timeout) = self.metadata_timeout {
            session = session.with_metadata_timeout(timeout);
        }
        if let Some(attempts) = self.metadata_attempts {
            session = session.with_metadata_attempts(attempts);
        }
        // An STS endpoint stated without a role still says where STS is.
        if let (Some(endpoint), None) = (&self.sts_endpoint, &self.role_arn) {
            session = session.with_service_endpoint_url("sts", endpoint);
        }
        if let Some(role_arn) = &self.role_arn {
            let mut role = AssumedRole::new(role_arn);
            if let Some(name) = &self.role_session {
                role = role.with_session_name(name);
            }
            if let Some(external_id) = &self.external_id {
                role = role.with_external_id(external_id);
            }
            if let Some(duration) = self.role_duration {
                role = role.with_duration(duration);
            }
            if let Some(region) = &self.sts_region {
                role = role.with_region(region);
            }
            if let Some(endpoint) = &self.sts_endpoint {
                role = role.with_endpoint(endpoint);
            }
            if let Some(serial) = &self.mfa_serial {
                role = role.with_mfa_serial(serial);
            }
            if let Some(source) = &self.source_profile {
                role = role.with_source_profile(source);
            }
            if let Some(source) = self.credential_source {
                role = role.with_credential_source(source);
            }
            if let Some(path) = &self.web_identity_token_file {
                role = role.with_web_identity_token_file(path);
            }
            session = session.with_assumed_role(role);
        }
        // A sign-in is four values or none of them, so it is assembled here
        // rather than one property at a time.
        let named = [
            ("sso_start_url", &self.sso_start_url),
            ("sso_region", &self.sso_region),
            ("sso_account_id", &self.sso_account_id),
            ("sso_role_name", &self.sso_role_name),
        ];
        if self.sso_session.is_some() || named.iter().any(|(_, value)| value.is_some()) {
            let missing: Vec<&str> = named
                .iter()
                .filter(|(_, value)| value.is_none())
                .map(|(key, _)| *key)
                .collect();
            if !missing.is_empty() {
                return Err(refusal(&format!(
                    "an IAM Identity Center sign-in needs {}",
                    missing.join(", ")
                )));
            }
            let mut sso = Sso::new(
                self.sso_start_url.as_deref().unwrap_or_default(),
                self.sso_region.as_deref().unwrap_or_default(),
                self.sso_account_id.as_deref().unwrap_or_default(),
                self.sso_role_name.as_deref().unwrap_or_default(),
            );
            if let Some(name) = &self.sso_session {
                sso = sso.with_session_name(name);
            }
            session = session.with_sso(sso);
        }
        Ok(session)
    }
}

/// What a property naming an endpoint says where it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EndpointName {
    /// An object store, whichever store answers: the S3 options' reader.
    Store,
    /// Amazon S3, over a [`Self::Store`] name: the S3 options' reader.
    S3,
    /// STS: this reader.
    Sts,
    /// The instance metadata service: this reader.
    Metadata,
}

impl EndpointName {
    /// Every name an endpoint is stated under, once folded, and what it
    /// places: the one list both property readers match an endpoint by, and
    /// whose names the S3 options' environment sweep never turns into a knob -
    /// a stated endpoint outranks the environment and the profile, which a
    /// variable merely held must not.
    const ALL: [(&'static str, Self); 14] = [
        ("endpoint", Self::Store),
        ("endpoint_url", Self::Store),
        ("endpoint_override", Self::Store),
        ("blob_endpoint", Self::Store),
        ("storage_blob_endpoint", Self::Store),
        ("storage_endpoint", Self::Store),
        ("service_host", Self::Store),
        ("host", Self::Store),
        ("endpoint_url_s3", Self::S3),
        ("sts_endpoint", Self::Sts),
        ("role_sts_endpoint", Self::Sts),
        ("ec2_metadata_service_endpoint", Self::Metadata),
        ("metadata_service_endpoint", Self::Metadata),
        ("ec2_metadata_endpoint", Self::Metadata),
    ];

    /// What the folded name `key` places, when it names an endpoint.
    pub(crate) fn of(key: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, placed)| *placed)
    }
}

/// The prefixes an AWS identity property is written under: the AWS tools'
/// own, and PyIceberg's.
const PREFIXES: [&str; 2] = ["client_", "aws_"];

/// The name a property is matched by: case, separators and prefix removed.
fn canonical(name: &str) -> String {
    let mut key = name.trim().to_ascii_lowercase().replace(['-', '.'], "_");
    while let Some(rest) = PREFIXES.iter().find_map(|prefix| key.strip_prefix(prefix)) {
        key = rest.to_owned();
    }
    key
}

/// A boolean, read through the one table every flag in the crate reads.
pub(crate) fn flag(name: &str, value: &str) -> Result<bool> {
    bool_from_text(value).ok_or_else(|| {
        refusal(&format!(
            "expected {BOOLEAN_SPELLINGS} for {name}, got {value}"
        ))
    })
}

/// Whether STS is reached in the region: `regional`, `legacy`, or a boolean.
fn regional(name: &str, value: &str) -> Result<bool> {
    match value.to_ascii_lowercase().as_str() {
        "regional" => Ok(true),
        "legacy" => Ok(false),
        _ => flag(name, value),
    }
}

/// A duration, which is how every vocabulary spells seconds.
pub(crate) fn seconds(name: &str, value: &str) -> Result<Duration> {
    duration_from_text(value).ok_or_else(|| {
        refusal(&format!(
            "expected {DURATION_SPELLINGS} for {name}, got {value}"
        ))
    })
}

/// A count.
pub(crate) fn count(name: &str, value: &str) -> Result<u32> {
    integer_from_text_as(value).ok_or_else(|| {
        refusal(&format!(
            "expected {INTEGER_SPELLINGS} for {name}, got {value}"
        ))
    })
}

/// Refuse a property value.
pub(crate) fn refusal(message: &str) -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        message.to_owned(),
    ))
}
