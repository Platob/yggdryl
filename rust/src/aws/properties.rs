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
    /// also reads aside, and the `endpoint_url_<service>` family, which is
    /// one name per service id. What a binding suggests a mistyped keyword
    /// against.
    pub const PROPERTY_NAMES: [&'static str; 34] = [
        "region",
        "default_region",
        "access_key_id",
        "secret_access_key",
        "session_token",
        "anonymous",
        "profile",
        "default_profile",
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
    /// | the region | `region`, else `default_region` |
    /// | a credential set | `access_key_id`, `secret_access_key`, `session_token`; `anonymous` for none |
    /// | a profile | `profile`, else `default_profile` |
    /// | a role to assume | `role_arn`, with `role_session_name`, `external_id`, `role_duration` (or the profile key `duration_seconds`), `sts_region`, `sts_endpoint`, `mfa_serial`, one of `source_profile` and `credential_source`, `web_identity_token_file` |
    /// | an IAM Identity Center sign-in | `sso_account_id` and `sso_role_name`, with `sso_start_url` and `sso_region`, or with the `sso_session` whose `[sso-session]` section holds them |
    /// | the shared files and a process | `config_file`, `shared_credentials_file`, `credential_process`, `ca_bundle` |
    /// | the endpoints | `use_fips_endpoint`, `use_dualstack_endpoint`, `sts_regional_endpoints` (`regional` or `legacy`), `sts_endpoint`, `endpoint_url_<service>` |
    /// | the instance metadata service | `ec2_metadata_disabled`, `ec2_metadata_service_endpoint`, `metadata_service_timeout`, `metadata_service_num_attempts` |
    ///
    /// Names are matched loosely: case, `-`, `_` and `.` are the same, and a
    /// leading `aws_` or `client.` is dropped - so `region`, `AWS_REGION`
    /// and PyIceberg's `client.region` are one name, as are
    /// `client.access-key-id`, `client.secret-access-key`,
    /// `client.session-token`, `client.profile-name` and `client.role-arn`
    /// and the bare names they fold to, and `AWS_DEFAULT_REGION` and
    /// `AWS_DEFAULT_PROFILE` are `default_region` and `default_profile`. A
    /// name under another prefix - `s3.` is the object store's own - is not
    /// read here, and neither is a bare `token`, which a catalog spells its
    /// own bearer token by.
    ///
    /// `default_region` and `default_profile` are read where the bag states
    /// no `region` or `profile`, whatever their order. A name
    /// `endpoint_url_<service>`, such as `AWS_ENDPOINT_URL_STS`,
    /// `AWS_ENDPOINT_URL_S3TABLES` or `AWS_ENDPOINT_URL_SSO_OIDC`, is
    /// [`Self::with_service_endpoint_url`] for that service id, below an
    /// `sts_endpoint` for STS; `endpoint_url_s3` is the object store's
    /// reader's and is not read here.
    ///
    /// An `sso_session` stated without `sso_start_url` and `sso_region`
    /// takes them, and its `sso_registration_scopes`, from the
    /// `[sso-session]` section of that name when a walk signs in, as a
    /// profile naming the section does; this door reads no file.
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
    /// name means - a `role_arn` that is not an ARN, an
    /// `sts_regional_endpoints` that is neither `regional` nor `legacy`
    /// included; half a credential set, naming the half that is missing; a
    /// role naming both `source_profile` and `credential_source`; an IAM
    /// Identity Center sign-in that lacks its account or role, or states
    /// neither a session nor both its start URL and region, naming what is
    /// missing.
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
    /// spelling it accepts: a name it answers `false` for is one it ignores.
    ///
    /// Answered by the reader itself - a known name either takes a probe
    /// value or refuses it - so the answer cannot disagree with a read.
    #[must_use]
    pub fn is_property(name: &str) -> bool {
        Identity::default()
            .read(&canonical(name), name, "x")
            .unwrap_or(true)
    }
}

/// The identity properties collected off one map, assembled onto a session
/// once all of them are in hand: a role and a sign-in are several values.
///
/// The S3 options' reader collects into one too, under the keys its own,
/// wider fold answers, and keeps for itself the names that are also the
/// object store's - the region, the pair, `anonymous` - so one reader reads
/// every name however it was reached.
#[derive(Default)]
pub(crate) struct Identity {
    region: Option<String>,
    /// Read only where `region` is not stated.
    default_region: Option<String>,
    access_key: Option<String>,
    secret_key: Option<String>,
    session_token: Option<String>,
    anonymous: Option<bool>,
    profile: Option<String>,
    /// Read only where `profile` is not stated.
    default_profile: Option<String>,
    /// `endpoint_url_<service>`: the service id as folded, and the endpoint,
    /// in the order stated, so a later pair for one service replaces it.
    service_endpoints: Vec<(String, String)>,
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
            "default_region" => self.default_region = text(),
            "access_key" | "access_key_id" => self.access_key = text(),
            "secret_key" | "secret_access_key" => self.secret_key = text(),
            "session_token" => self.session_token = text(),
            "anonymous" | "no_sign_request" => self.anonymous = Some(flag(name, value)?),
            "profile" | "profile_name" => self.profile = text(),
            "default_profile" => self.default_profile = text(),
            "role_arn" => {
                // Read once where it is stated, as a profile's is, so a typo
                // is this property's refusal rather than one STS words.
                value.parse::<crate::Arn>().map_err(|error| {
                    refusal(&format!(
                        "{name} {value:?} is not an ARN, which a role is named by: {error}"
                    ))
                })?;
                self.role_arn = text();
            }
            "session_name" | "role_session_name" => self.role_session = text(),
            "external_id" | "role_external_id" => self.external_id = text(),
            "role_duration"
            | "role_session_duration"
            | "assume_role_duration_seconds"
            | "duration_seconds" => {
                self.role_duration = Some(seconds(name, value)?);
            }
            key if EndpointName::of(key) == Some(EndpointName::Sts) => self.sts_endpoint = text(),
            key if EndpointName::of(key) == Some(EndpointName::Service) => {
                let service = key.strip_prefix(SERVICE_ENDPOINT).unwrap_or(key);
                self.service_endpoints
                    .push((service.to_owned(), value.to_owned()));
            }
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
            "sts_regional_endpoints" => {
                self.sts_regional = Some(regional(value).ok_or_else(|| {
                    refusal(&format!(
                        "expected {REGIONAL_SPELLINGS} for {name}, got {value}"
                    ))
                })?);
            }
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
    /// Half a credential set, a role naming two sources, or a sign-in
    /// lacking its account, its role or where it signs in, each naming what
    /// is wrong.
    pub(crate) fn apply(&self, session: &Session) -> Result<Session> {
        let mut session = session.clone();
        if let Some(region) = self.region.as_ref().or(self.default_region.as_ref()) {
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
        if let Some(profile) = self.profile.as_ref().or(self.default_profile.as_ref()) {
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
        for (service, endpoint) in &self.service_endpoints {
            session = session.with_service_endpoint_url(service, endpoint);
        }
        // An STS endpoint stated without a role still says where STS is,
        // over an `endpoint_url_sts`.
        if let (Some(endpoint), None) = (&self.sts_endpoint, &self.role_arn) {
            session = session.with_service_endpoint_url("sts", endpoint);
        }
        if let Some(role_arn) = &self.role_arn {
            // A role is traded with the keys of one source, as a profile's is.
            if self.source_profile.is_some() && self.credential_source.is_some() {
                return Err(refusal(
                    "the role names both source_profile and credential_source, \
                     and a role has one source",
                ));
            }
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
        if let Some(sso) = self.sso()? {
            session = session.with_sso(sso);
        }
        Ok(session)
    }

    /// The sign-in the `sso_*` properties state, assembled once all are in
    /// hand: an account and a role, signed in for at a start URL in a
    /// region - stated, or held by the `[sso-session]` section
    /// `sso_session` names, which the walk reads ([`sso_section`]).
    ///
    /// # Errors
    ///
    /// A sign-in lacking its account or its role, or stating one of its
    /// start URL and region without the other, or neither without a
    /// session, naming what is missing.
    fn sso(&self) -> Result<Option<Sso>> {
        let place = [
            ("sso_start_url", &self.sso_start_url),
            ("sso_region", &self.sso_region),
        ];
        let named = [
            ("sso_account_id", &self.sso_account_id),
            ("sso_role_name", &self.sso_role_name),
        ];
        if self.sso_session.is_none()
            && !place.iter().chain(&named).any(|(_, value)| value.is_some())
        {
            return Ok(None);
        }
        // The section holds the start URL and the region together, so a
        // session leaves both to it or states both over it.
        let from_section =
            self.sso_session.is_some() && place.iter().all(|(_, value)| value.is_none());
        let needed: &[(&str, &Option<String>)] = if from_section { &[] } else { &place };
        let missing_place = needed.iter().any(|(_, value)| value.is_none());
        let missing: Vec<&str> = needed
            .iter()
            .chain(&named)
            .filter(|(_, value)| value.is_none())
            .map(|(key, _)| *key)
            .collect();
        if !missing.is_empty() {
            return Err(refusal(&format!(
                "an IAM Identity Center sign-in needs {}{}",
                missing.join(", "),
                match &self.sso_session {
                    Some(name) if missing_place => format!(
                        ", or neither sso_start_url nor sso_region for the \
                         [sso-session {name}] section to hold them"
                    ),
                    _ => String::new(),
                }
            )));
        }
        // Left empty, the start URL and the region are the section's to
        // fill when a walk signs in.
        let mut sso = Sso::new(
            self.sso_start_url.as_deref().unwrap_or_default(),
            self.sso_region.as_deref().unwrap_or_default(),
            self.sso_account_id.as_deref().unwrap_or_default(),
            self.sso_role_name.as_deref().unwrap_or_default(),
        );
        if let Some(name) = &self.sso_session {
            sso = sso.with_session_name(name);
        }
        Ok(Some(sso))
    }
}

/// The `[sso-session]` section a sign-in stated as properties still takes
/// its start URL, region and scopes from: its name, where the properties
/// named the section and stated neither the start URL nor the region; `None`
/// for a sign-in that states where it signs in.
///
/// The walk reads the section when it signs in, as [`Profile::sso`]
/// reads the one a profile names, and refuses a section no file defines;
/// the property door reads no file.
///
/// [`Profile::sso`]: super::Profile::sso
pub(crate) fn sso_section(sso: &Sso) -> Option<&str> {
    if sso.start_url().is_empty() && sso.region().is_empty() {
        sso.session_name()
    } else {
        None
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
    /// Any other service, by the service id after [`SERVICE_ENDPOINT`]
    /// (`endpoint_url_sts`, `endpoint_url_s3tables`): this reader.
    Service,
}

/// What a service's own endpoint name is the service id after, once folded:
/// `AWS_ENDPOINT_URL_<SERVICE>` without its `AWS_`.
const SERVICE_ENDPOINT: &str = "endpoint_url_";

impl EndpointName {
    /// Every name an endpoint is stated under, once folded, and what it
    /// places - beside [`SERVICE_ENDPOINT`] before any service id, which is
    /// [`Self::Service`]: the one list the two property readers match an
    /// endpoint by, and the S3 options' environment sweep turns no name of
    /// into a knob - a stated endpoint outranks `AWS_ENDPOINT_URL_<SERVICE>`
    /// and the profile and survives `AWS_IGNORE_CONFIGURED_ENDPOINT_URLS`,
    /// which a variable the environment merely holds must not.
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
            .or_else(|| {
                key.strip_prefix(SERVICE_ENDPOINT)
                    .is_some_and(|service| !service.is_empty())
                    .then_some(Self::Service)
            })
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

/// The spellings [`regional`] reads, for a refusal to list.
pub(crate) const REGIONAL_SPELLINGS: &str = "regional/legacy, or true/false";

/// Whether STS is reached in the region, read off an
/// `sts_regional_endpoints` setting: `regional` is `true`, `legacy` is
/// `false`, in any case and trimmed; anything else is `None`, which each
/// reader - this property, `AWS_STS_REGIONAL_ENDPOINTS`, the profile key -
/// refuses naming itself, as botocore refuses a value outside the two.
pub(crate) fn regional(text: &str) -> Option<bool> {
    let text = text.trim();
    if text.eq_ignore_ascii_case("regional") {
        Some(true)
    } else if text.eq_ignore_ascii_case("legacy") {
        Some(false)
    } else {
        // A flag's spellings say the same two things, through the one
        // boolean table every setting reads.
        crate::boolean::bool_from_text(text)
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
