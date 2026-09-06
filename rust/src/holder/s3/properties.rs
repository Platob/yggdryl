//! Reading the knobs out of a property map someone else's names.
//!
//! A caller reaching S3 through this crate rarely starts from this crate's
//! spellings. They start from a PyIceberg catalog's properties, or from the
//! arguments PyArrow's `S3FileSystem` takes, or from the environment variable
//! names the AWS tools use - three vocabularies for one set of knobs. Rather
//! than making every caller translate, this reads all three.
//!
//! Two rules make that safe. A name this crate does not know is *ignored*,
//! because a catalog's properties carry a great deal that has nothing to do
//! with a store. A name it knows but cannot honor is *refused*, because a
//! caller who asked for a signing service and silently did not get one would
//! find out from the store, at the worst moment.

use std::time::Duration;

use super::credentials::Credentials;
use super::encryption::{CustomerKey, Encryption, KmsKey};
use super::options::S3Options;
use super::sts::AssumedRole;
use crate::{Error, Result};

impl S3Options {
    /// Read the knobs `properties` names, in whichever vocabulary it uses.
    ///
    /// Explicit calls still win where they say the same thing, because this
    /// applies what it finds to `self` and a later `with_` call replaces it.
    ///
    /// | knob | this crate | PyIceberg | PyArrow |
    /// | --- | --- | --- | --- |
    /// | endpoint | `endpoint` | `s3.endpoint` | `endpoint_override`, `scheme` |
    /// | region | `region` | `s3.region` | `region` |
    /// | credentials | `access_key_id`, `secret_access_key`, `session_token` | `s3.access-key-id`, `s3.secret-access-key`, `s3.session-token` | `access_key`, `secret_key`, `session_token` |
    /// | anonymous | `anonymous` | | `anonymous` |
    /// | addressing | `path_style` | `s3.force-virtual-addressing` | `force_virtual_addressing` |
    /// | timeouts | `timeout`, `connect_timeout` | `s3.request-timeout`, `s3.connect-timeout` | `request_timeout`, `connect_timeout` |
    /// | proxy | `proxy` | `s3.proxy-uri` | `proxy_options` |
    /// | role | `role_arn`, `role_session_name`, `external_id` | `s3.role-arn`, `s3.role-session-name` | `role_arn`, `session_name`, `external_id` |
    /// | encryption | `sse_type`, `sse_key`, `sse_md5` | `s3.sse.type`, `s3.sse.key`, `s3.sse.md5` | |
    /// | buckets | `allow_bucket_creation`, `allow_bucket_deletion` | | the same |
    /// | metadata | `default_metadata` is a map, so it is not read here | | |
    ///
    /// Names are matched loosely: case, `-`, `_`, and `.` are the same, and a
    /// leading `s3.`, `client.`, or `aws_` is dropped. So `s3.access-key-id`,
    /// `AWS_ACCESS_KEY_ID`, and `access_key` are one knob.
    ///
    /// # Errors
    ///
    /// Returns a refusal when a value will not parse as what its name means,
    /// when a customer key is unusable, or when a name asks for something this
    /// client does not do.
    ///
    /// ```
    /// use yggdryl::holder::s3::S3Options;
    ///
    /// // A PyIceberg catalog's properties, most of which are not about S3.
    /// let options = S3Options::default().with_properties([
    ///     ("warehouse", "s3://trades/lake"),
    ///     ("s3.endpoint", "http://localhost:9000"),
    ///     ("s3.access-key-id", "minioadmin"),
    ///     ("s3.secret-access-key", "minioadmin"),
    ///     ("s3.region", "eu-west-1"),
    ///     ("s3.force-virtual-addressing", "false"),
    ///     ("s3.request-timeout", "30"),
    /// ])?;
    /// assert_eq!(options.endpoint(), Some("http://localhost:9000"));
    /// assert_eq!(options.region(), Some("eu-west-1"));
    /// assert_eq!(options.path_style(), Some(true));
    ///
    /// // PyArrow's argument names, for the same knobs.
    /// let options = S3Options::default().with_properties([
    ///     ("endpoint_override", "localhost:9000"),
    ///     ("scheme", "http"),
    ///     ("access_key", "minioadmin"),
    ///     ("secret_key", "minioadmin"),
    ///     ("allow_bucket_creation", "true"),
    /// ])?;
    /// assert_eq!(options.endpoint(), Some("http://localhost:9000"));
    /// assert!(options.bucket_creation());
    /// ```
    pub fn with_properties<K, V>(
        mut self,
        properties: impl IntoIterator<Item = (K, V)>,
    ) -> Result<Self>
    where
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let mut parts = Parts::default();
        for (name, value) in properties {
            let (name, value) = (name.as_ref(), value.as_ref().trim());
            if value.is_empty() {
                continue;
            }
            self = self.read(&canonical(name), name, value, &mut parts)?;
        }
        parts.apply(self)
    }

    /// The knobs the process environment names, under this one's prefixes.
    ///
    /// The whole environment is swept rather than a list of variables being
    /// looked up, so every name [`Self::with_properties`] accepts is also an
    /// environment variable: `AWS_REGION` and `AWS_ENDPOINT_URL_S3` because
    /// they are the AWS spellings, `AWS_S3_SSE_TYPE` and `YGGDRYL_S3_ROLE_ARN`
    /// because they are this crate's knobs said the same way. Names it does
    /// not know are ignored, which is most of an environment.
    ///
    /// Answers nothing when [`Self::with_environment`] is off.
    #[must_use]
    pub fn environment_properties(&self) -> Vec<(String, String)> {
        if !self.reads_environment() {
            return Vec::new();
        }
        let mut found: Vec<(String, String)> = std::env::vars()
            .filter_map(|(name, value)| {
                let named = self
                    .environment_prefixes()
                    .iter()
                    .filter_map(|prefix| strip_prefix_ignoring_case(&name, prefix))
                    .max_by_key(|rest| name.len() - rest.len())?;
                (!value.trim().is_empty()).then(|| (named.to_owned(), value))
            })
            .collect();
        // The sweep order is the environment's, which is nobody's; sorting
        // makes what an ambiguous pair resolves to a fact rather than a race.
        found.sort();
        found
    }

    /// The knobs the environment names, as options.
    ///
    /// # Errors
    ///
    /// As [`Self::with_properties`]: a value that will not parse is a refusal
    /// rather than a silently ignored variable.
    pub fn from_environment(&self) -> Result<Self> {
        Self::default()
            .with_environment_prefixes(self.environment_prefixes().to_vec())
            .with_properties(self.environment_properties())
    }

    /// Fill from `ambient` every knob this one does not set for itself.
    ///
    /// Explicit wins, which is what makes "explicit, then the URL, then the
    /// environment" an order rather than a special case per knob. A knob left
    /// at its default takes the ambient answer; one the caller set keeps
    /// theirs.
    #[must_use]
    pub fn under(mut self, ambient: &Self) -> Self {
        let fallback = Self::default();
        if self.endpoint().is_none() {
            if let Some(endpoint) = ambient.endpoint() {
                self = self.with_endpoint(endpoint);
            }
        }
        if self.region().is_none() {
            if let Some(region) = ambient.region() {
                self = self.with_region(region);
            }
        }
        if self.credentials().is_none() && !self.anonymous() {
            if let Some(credentials) = ambient.credentials() {
                self = self.with_credentials(credentials.clone());
            } else if ambient.anonymous() {
                self = self.with_anonymous(true);
            }
        }
        if self.profile().is_none() {
            if let Some(profile) = ambient.profile() {
                self = self.with_profile(profile);
            }
        }
        if self.path_style().is_none() {
            if let Some(path_style) = ambient.path_style() {
                self = self.with_path_style(path_style);
            }
        }
        if self.payload_signing().is_none() {
            if let Some(signing) = ambient.payload_signing() {
                self = self.with_payload_signing(signing);
            }
        }
        if self.proxy().is_none() {
            if let Some(proxy) = ambient.proxy() {
                self = self.with_proxy(proxy);
            }
        }
        if self.encryption().is_default() && !ambient.encryption().is_default() {
            self = self.with_encryption(ambient.encryption().clone());
        }
        if self.assumed_role().is_none() {
            if let Some(role) = ambient.assumed_role() {
                self = self.with_assumed_role(role.clone());
            }
        }
        if self.default_metadata().is_empty() && !ambient.default_metadata().is_empty() {
            self = self.with_default_metadata(ambient.default_metadata().to_vec());
        }
        if self.part_size() == fallback.part_size() {
            self = self.with_part_size(ambient.part_size());
        }
        if self.multipart_threshold() == fallback.multipart_threshold() {
            self = self.with_multipart_threshold(ambient.multipart_threshold());
        }
        if self.list_page_size() == fallback.list_page_size() {
            self = self.with_list_page_size(ambient.list_page_size());
        }
        if self.max_attempts() == fallback.max_attempts() {
            self = self.with_max_attempts(ambient.max_attempts());
        }
        if self.timeout() == fallback.timeout() {
            self = self.with_timeout(ambient.timeout());
        }
        if self.connect_timeout() == fallback.connect_timeout() {
            self = self.with_connect_timeout(ambient.connect_timeout());
        }
        if self.bucket_creation() == fallback.bucket_creation() {
            self = self.with_bucket_creation(ambient.bucket_creation());
        }
        if self.bucket_deletion() == fallback.bucket_deletion() {
            self = self.with_bucket_deletion(ambient.bucket_deletion());
        }
        self
    }

    /// The same, starting from the defaults.
    ///
    /// # Errors
    ///
    /// As [`Self::with_properties`].
    pub fn from_properties<K, V>(properties: impl IntoIterator<Item = (K, V)>) -> Result<Self>
    where
        K: AsRef<str>,
        V: AsRef<str>,
    {
        Self::default().with_properties(properties)
    }

    /// Apply one property, or collect it for something assembled from several.
    fn read(self, key: &str, name: &str, value: &str, parts: &mut Parts) -> Result<Self> {
        let mut options = self;
        match key {
            "endpoint" | "endpoint_url" | "endpoint_override" => {
                parts.endpoint = Some(value.to_owned());
            }
            // The service-specific spelling wins over the generic one, which
            // is what `AWS_ENDPOINT_URL_S3` beside `AWS_ENDPOINT_URL` means.
            "endpoint_url_s3" | "s3_endpoint_url" => {
                parts.service_endpoint = Some(value.to_owned());
            }
            "scheme" => parts.scheme = Some(value.to_owned()),
            "region" => parts.region = Some(value.to_owned()),
            "default_region" => parts.default_region = Some(value.to_owned()),
            "access_key" | "access_key_id" => parts.access_key = Some(value.to_owned()),
            "secret_key" | "secret_access_key" => parts.secret_key = Some(value.to_owned()),
            "session_token" => parts.session_token = Some(value.to_owned()),
            "anonymous" | "allow_anonymous" | "no_sign_request" => {
                options = options.with_anonymous(flag(name, value)?);
            }
            "profile" | "profile_name" => options = options.with_profile(value),
            "path_style" | "force_path_style" | "use_path_style" => {
                options = options.with_path_style(flag(name, value)?);
            }
            // The same knob, said the other way round.
            "force_virtual_addressing" | "virtual_addressing" | "virtual_hosted_style_request" => {
                options = options.with_path_style(!flag(name, value)?);
            }
            "timeout" | "request_timeout" => options = options.with_timeout(seconds(name, value)?),
            "connect_timeout" | "connection_timeout" => {
                options = options.with_connect_timeout(seconds(name, value)?);
            }
            "max_attempts" => options = options.with_max_attempts(count(name, value)?),
            "max_retries" | "num_retries" | "retry_num_retries" => {
                options = options.with_max_attempts(count(name, value)?.saturating_add(1));
            }
            "part_size" | "multipart_part_size" | "multipart_part_size_bytes" => {
                options = options.with_part_size(size(name, value)?);
            }
            "multipart_threshold" | "multipart_threshold_bytes" => {
                options = options.with_multipart_threshold(size(name, value)?);
            }
            "list_page_size" | "page_size" => {
                options =
                    options.with_list_page_size(count(name, value)?.try_into().unwrap_or(u16::MAX));
            }
            "payload_signing" | "sign_payload" => {
                options = options.with_payload_signing(flag(name, value)?);
            }
            "proxy" | "proxy_uri" | "proxy_url" | "proxy_options" => {
                options = options.with_proxy(value);
            }
            "allow_bucket_creation" => options = options.with_bucket_creation(flag(name, value)?),
            "allow_bucket_deletion" => options = options.with_bucket_deletion(flag(name, value)?),
            "role_arn" => parts.role_arn = Some(value.to_owned()),
            "session_name" | "role_session_name" => parts.role_session = Some(value.to_owned()),
            "external_id" | "role_external_id" => parts.external_id = Some(value.to_owned()),
            "role_duration" | "role_session_duration" | "assume_role_duration_seconds" => {
                parts.role_duration = Some(seconds(name, value)?);
            }
            "sts_endpoint" | "role_sts_endpoint" => parts.sts_endpoint = Some(value.to_owned()),
            "sts_region" | "role_region" => parts.sts_region = Some(value.to_owned()),
            "sse" | "sse_type" | "server_side_encryption" => {
                parts.sse_type = Some(value.to_owned());
            }
            "sse_key" => parts.sse_key = Some(value.to_owned()),
            "sse_md5" | "sse_key_md5" => parts.sse_md5 = Some(value.to_owned()),
            "sse_kms_key_id" | "sse_kms_key" | "kms_key_id" => {
                parts.kms_key_id = Some(value.to_owned());
            }
            "sse_context" | "sse_encryption_context" | "encryption_context" => {
                parts.sse_context = Some(value.to_owned());
            }
            "sse_bucket_key" | "sse_bucket_key_enabled" | "bucket_key_enabled" => {
                parts.bucket_key = Some(flag(name, value)?);
            }
            // Named, understood, and not something this client can do. A
            // silent omission here would be found out at the store.
            "signer" | "signer_uri" | "signer_endpoint" => return Err(unsupported(name)),
            // Everything else belongs to something that is not this store.
            _ => {}
        }
        Ok(options)
    }
}

/// The properties that only mean something once several are in hand.
#[derive(Default)]
struct Parts {
    endpoint: Option<String>,
    service_endpoint: Option<String>,
    scheme: Option<String>,
    region: Option<String>,
    default_region: Option<String>,
    access_key: Option<String>,
    secret_key: Option<String>,
    session_token: Option<String>,
    role_arn: Option<String>,
    role_session: Option<String>,
    external_id: Option<String>,
    role_duration: Option<Duration>,
    sts_endpoint: Option<String>,
    sts_region: Option<String>,
    sse_type: Option<String>,
    sse_key: Option<String>,
    sse_md5: Option<String>,
    kms_key_id: Option<String>,
    sse_context: Option<String>,
    bucket_key: Option<bool>,
}

impl Parts {
    /// Assemble what was collected onto `options`.
    fn apply(self, mut options: S3Options) -> Result<S3Options> {
        if let Some(region) = self.region.as_ref().or(self.default_region.as_ref()) {
            options = options.with_region(region);
        }
        let endpoint = self.service_endpoint.as_ref().or(self.endpoint.as_ref());
        if let Some(endpoint) = endpoint {
            // PyArrow splits the endpoint from its scheme; PyIceberg does not.
            options = match (&self.scheme, endpoint.contains("://")) {
                (Some(scheme), false) => options.with_endpoint(format!("{scheme}://{endpoint}")),
                _ => options.with_endpoint(endpoint),
            };
        }
        if let (Some(access_key), Some(secret_key)) = (&self.access_key, &self.secret_key) {
            let mut credentials = Credentials::new(access_key, secret_key);
            if let Some(token) = &self.session_token {
                credentials = credentials.with_session_token(token);
            }
            options = options.with_credentials(credentials);
        }
        if let Some(role_arn) = &self.role_arn {
            let mut role = AssumedRole::new(role_arn);
            if let Some(session) = &self.role_session {
                role = role.with_session_name(session);
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
            options = options.with_assumed_role(role);
        }
        if let Some(encryption) = self.encryption()? {
            options = options.with_encryption(encryption);
        }
        Ok(options)
    }

    /// The encryption the `sse.*` properties describe.
    ///
    /// `AES256` is the ambiguous one: it is what `SSE-S3` is called, and it is
    /// also the algorithm of a customer key. A key alongside it is what tells
    /// them apart, which is the reading the AWS tools take.
    fn encryption(&self) -> Result<Option<Encryption>> {
        let named = self.sse_type.as_deref().map(str::trim);
        let customer = |key: &str| -> Result<Encryption> {
            let held = match &self.sse_md5 {
                Some(md5) => CustomerKey::from_base64_checked(key, md5)?,
                None => CustomerKey::from_base64(key)?,
            };
            Ok(Encryption::Customer(held))
        };
        let kms = || {
            let key_id = self.kms_key_id.as_ref().or(self.sse_key.as_ref());
            let mut key = key_id.map_or_else(KmsKey::default, KmsKey::new);
            if let Some(context) = &self.sse_context {
                key = key.with_context(context);
            }
            if let Some(enabled) = self.bucket_key {
                key = key.with_bucket_key(enabled);
            }
            key
        };
        match named.map(str::to_ascii_lowercase).as_deref() {
            None | Some("none" | "disabled" | "false") => {
                // A key with no type is still a key: `sse.key` alone means the
                // customer key, because nothing else takes one.
                match (&self.sse_key, &self.kms_key_id) {
                    (Some(key), None) => match customer(key) {
                        Ok(encryption) => Ok(Some(encryption)),
                        // A key with no type could have been either; say so
                        // rather than only that it is not a customer key.
                        Err(error) if self.sse_md5.is_none() => Err(refusal(&format!(
                            "{error}; name an sse type of aws:kms if the key is a KMS key"
                        ))),
                        Err(error) => Err(error),
                    },
                    (_, Some(_)) => Ok(Some(Encryption::Kms(kms()))),
                    (None, None) => Ok(None),
                }
            }
            Some("aes256" | "sse-s3" | "sse_s3" | "s3" | "managed") => match &self.sse_key {
                Some(key) => customer(key).map(Some),
                None => Ok(Some(Encryption::Managed)),
            },
            Some("aws:kms" | "sse-kms" | "sse_kms" | "kms") => Ok(Some(Encryption::Kms(kms()))),
            Some("aws:kms:dsse" | "dsse-kms" | "dsse") => {
                Ok(Some(Encryption::Kms(kms().with_dual_layer(true))))
            }
            Some("sse-c" | "sse_c" | "custom" | "customer") => match &self.sse_key {
                Some(key) => customer(key).map(Some),
                None => Err(refusal(
                    "expected a customer key alongside an sse type of sse-c",
                )),
            },
            Some(other) => Err(refusal(&format!(
                "expected an sse type of none, AES256, aws:kms, aws:kms:dsse, or sse-c, got {other}"
            ))),
        }
    }
}

/// The rest of `name` after `prefix`, matched without regard to case.
fn strip_prefix_ignoring_case<'name>(name: &'name str, prefix: &str) -> Option<&'name str> {
    let head = name.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &name[prefix.len()..])
        .filter(|rest| !rest.is_empty())
}

/// The name a property is matched by: case, separators, and prefix removed.
fn canonical(name: &str) -> String {
    let mut key = name.trim().to_ascii_lowercase().replace(['-', '.'], "_");
    // A prefix may stack - `AWS_S3_FORCE_PATH_STYLE` carries two - so this
    // peels rather than strips once.
    loop {
        let peeled = ["s3_", "client_", "aws_"]
            .iter()
            .find_map(|prefix| key.strip_prefix(prefix));
        match peeled {
            Some(rest) => key = rest.to_owned(),
            None => return key,
        }
    }
}

/// A boolean, in any of the spellings a configuration file uses.
fn flag(name: &str, value: &str) -> Result<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "t" | "yes" | "y" | "on" | "1" => Ok(true),
        "false" | "f" | "no" | "n" | "off" | "0" => Ok(false),
        _ => Err(refusal(&format!(
            "expected a boolean for {name}, got {value}"
        ))),
    }
}

/// A duration in seconds, which is how both vocabularies spell one.
fn seconds(name: &str, value: &str) -> Result<Duration> {
    let seconds: f64 = value
        .parse()
        .map_err(|_| refusal(&format!("expected seconds for {name}, got {value}")))?;
    if !seconds.is_finite() || seconds < 0.0 {
        return Err(refusal(&format!(
            "expected a duration of at least zero seconds for {name}, got {value}"
        )));
    }
    Ok(Duration::from_secs_f64(seconds))
}

/// A count.
fn count(name: &str, value: &str) -> Result<u32> {
    value
        .parse()
        .map_err(|_| refusal(&format!("expected a whole number for {name}, got {value}")))
}

/// A byte count, which may carry a `KiB`, `MiB`, or `GiB` suffix.
fn size(name: &str, value: &str) -> Result<u64> {
    let lowered = value.to_ascii_lowercase();
    let (digits, scale) = ["gib", "mib", "kib", "gb", "mb", "kb", "g", "m", "k", "b"]
        .iter()
        .find_map(|suffix| {
            lowered
                .strip_suffix(suffix)
                .map(|digits| (digits, unit(suffix)))
        })
        .unwrap_or((lowered.as_str(), 1));
    let count: u64 = digits
        .trim()
        .parse()
        .map_err(|_| refusal(&format!("expected a byte count for {name}, got {value}")))?;
    Ok(count.saturating_mul(scale))
}

/// The multiplier a size suffix names.
const fn unit(suffix: &str) -> u64 {
    match suffix.as_bytes() {
        [b'g', ..] => 1024 * 1024 * 1024,
        [b'm', ..] => 1024 * 1024,
        [b'k', ..] => 1024,
        _ => 1,
    }
}

/// Refuse a property this client understands and cannot honor.
fn unsupported(name: &str) -> Error {
    refusal(&format!(
        "{name} asks for something this S3 client does not do; \
         remove it rather than have it silently not happen"
    ))
}

/// Refuse a property value.
fn refusal(message: &str) -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        message.to_owned(),
    ))
}
