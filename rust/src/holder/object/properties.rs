//! Reading the knobs out of a property map in someone else's names.
//!
//! A caller reaching an object store through this crate rarely starts from this
//! crate's spellings. They start from a PyIceberg catalog's properties, or from
//! the arguments PyArrow's `S3FileSystem`, `GcsFileSystem`, or
//! `AzureFileSystem` take, or from the environment variable names each store's
//! own tools use - a vocabulary per store per tool, for one set of knobs.
//! Rather than making every caller translate, this reads all of them.
//!
//! Three rules make that safe. A name this crate does not know is *ignored*,
//! because a catalog's properties carry a great deal that has nothing to do
//! with a store. A name it knows but cannot honor is *refused*, because a
//! caller who asked for a signing service and silently did not get one would
//! find out from the store, at the worst moment. And a name two stores both
//! have - `storage_class`, `client_id` - is applied to *both*, because only the
//! store that answers ever reads its own options, so nothing is ambiguous by
//! the time it matters.

use std::time::Duration;

use super::aws::credentials::Credentials;
use super::aws::options::{AwsOptions, Checksum};
use super::aws::sts::AssumedRole;
use super::azure::options::{AzureOptions, BlobType};
use super::encryption::Encryption;
use super::google::options::GoogleOptions;
use super::options::ObjectOptions;
use crate::{Error, Result};

impl ObjectOptions {
    /// Read the knobs `properties` names, in whichever vocabulary it uses.
    ///
    /// Explicit calls still win where they say the same thing, because this
    /// applies what it finds to `self` and a later `with_` call replaces it.
    ///
    /// | knob | this crate | PyIceberg | PyArrow | the store's own tools |
    /// | --- | --- | --- | --- | --- |
    /// | endpoint | `endpoint` | `s3.endpoint`, `gcs.service.host`, `adls.endpoint` | `endpoint_override`, `scheme` | `AWS_ENDPOINT_URL`, `STORAGE_EMULATOR_HOST`, `AZURE_STORAGE_BLOB_ENDPOINT` |
    /// | region | `region` | `s3.region` | `region` | `AWS_REGION` |
    /// | keys | `access_key_id`, `secret_access_key` | `s3.access-key-id`, `s3.secret-access-key` | `access_key`, `secret_key` | `AWS_ACCESS_KEY_ID` |
    /// | anonymous | `anonymous` | | `anonymous` | `AWS_NO_SIGN_REQUEST` |
    /// | addressing | `path_style` | `s3.force-virtual-addressing` | `force_virtual_addressing` | `AWS_S3_FORCE_PATH_STYLE` |
    /// | timeouts | `timeout`, `connect_timeout` | `s3.request-timeout`, `s3.connect-timeout` | `request_timeout`, `connect_timeout` | |
    /// | role | `role_arn`, `role_session_name`, `external_id` | `s3.role-arn` | `role_arn` | |
    /// | encryption | `sse_type`, `sse_key`, `sse_md5` | `s3.sse.type`, `s3.sse.key` | | |
    /// | project | `project`, `user_project` | `gcs.project-id` | `project_id` | `GOOGLE_CLOUD_PROJECT` |
    /// | Google identity | `credentials_file`, `access_token` | `gcs.oauth2.token` | `credentials_file` | `GOOGLE_APPLICATION_CREDENTIALS` |
    /// | Azure account | `account_name`, `account_key`, `sas_token` | `adls.account-name`, `adls.account-key`, `adls.sas-token` | `account_name`, `account_key` | `AZURE_STORAGE_ACCOUNT_NAME` |
    /// | Azure identity | `tenant_id`, `client_id`, `client_secret` | `adls.tenant-id`, `adls.client-id` | the same | `AZURE_TENANT_ID` |
    ///
    /// Names are matched loosely: case, `-`, `_`, and `.` are the same, and a
    /// leading `s3.`, `gcs.`, `gs.`, `google.`, `azure.`, `adls.`, `abfs.`,
    /// `client.`, `storage.`, `aws_`, or `blob.` is dropped. So
    /// `s3.access-key-id`, `AWS_ACCESS_KEY_ID`, and `access_key` are one knob.
    ///
    /// # Errors
    ///
    /// Returns a refusal when a value will not parse as what its name means,
    /// when a customer key is unusable, or when a name asks for something this
    /// client does not do.
    ///
    /// ```
    /// use yggdryl::holder::object::ObjectOptions;
    ///
    /// // A PyIceberg catalog's properties, most of which are not about a store.
    /// let options = ObjectOptions::default().with_properties([
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
    /// // The same catalog's Azure and Google properties, in their own names.
    /// let options = ObjectOptions::default().with_properties([
    ///     ("adls.account-name", "trades"),
    ///     ("adls.sas-token", "sv=2025-05-05&sig=x"),
    ///     ("gcs.project-id", "trading-analytics"),
    /// ])?;
    /// assert_eq!(options.azure().account(), Some("trades"));
    /// assert!(options.azure().has_sas_token());
    /// assert_eq!(options.google().project(), Some("trading-analytics"));
    /// # Ok::<(), yggdryl::Error>(())
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
    /// environment variable: `AWS_REGION` and `AZURE_STORAGE_ACCOUNT_NAME`
    /// because they are the stores' own spellings, `AWS_SSE_TYPE` and
    /// `YGGDRYL_ROLE_ARN` because they are this crate's knobs said the same
    /// way. Names it does not know are ignored, which is most of an
    /// environment.
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
        if self.path_style().is_none() {
            if let Some(path_style) = ambient.path_style() {
                self = self.with_path_style(path_style);
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
        if self.container_creation() == fallback.container_creation() {
            self = self.with_container_creation(ambient.container_creation());
        }
        if self.container_deletion() == fallback.container_deletion() {
            self = self.with_container_deletion(ambient.container_deletion());
        }
        let aws = self.aws().clone().under(ambient.aws());
        let google = self.google().clone().under(ambient.google());
        let azure = self.azure().clone().under(ambient.azure());
        self.with_aws(aws).with_google(google).with_azure(azure)
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
    #[allow(clippy::too_many_lines)]
    fn read(self, key: &str, name: &str, value: &str, parts: &mut Parts) -> Result<Self> {
        let mut options = self;
        match key {
            // --- where the store is -----------------------------------------
            "endpoint" | "endpoint_url" | "endpoint_override" | "blob_endpoint"
            | "service_host" | "host" => {
                parts.endpoint = Some(value.to_owned());
            }
            // The service-specific spelling wins over the generic one, which
            // is what `AWS_ENDPOINT_URL_S3` beside `AWS_ENDPOINT_URL` means.
            "endpoint_url_s3" | "s3_endpoint_url" => {
                parts.service_endpoint = Some(value.to_owned());
            }
            "scheme" => parts.scheme = Some(value.to_owned()),
            "region" | "location" => parts.region = Some(value.to_owned()),
            "default_region" => parts.default_region = Some(value.to_owned()),

            // --- who is asking ----------------------------------------------
            "access_key" | "access_key_id" | "hmac_access_key" | "hmac_key_id" => {
                parts.access_key = Some(value.to_owned());
            }
            "secret_key" | "secret_access_key" | "hmac_secret" | "hmac_secret_key" => {
                parts.secret_key = Some(value.to_owned());
            }
            "session_token" => parts.session_token = Some(value.to_owned()),
            "anonymous" | "allow_anonymous" | "no_sign_request" => {
                options = options.with_anonymous(flag(name, value)?);
            }

            // --- how much, how often, how long ------------------------------
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
            "part_size"
            | "multipart_part_size"
            | "multipart_part_size_bytes"
            | "chunk_size"
            | "upload_chunk_size"
            | "blocksize"
            | "block_size" => {
                options = options.with_part_size(size(name, value)?);
            }
            "multipart_threshold" | "multipart_threshold_bytes" => {
                options = options.with_multipart_threshold(size(name, value)?);
            }
            "list_page_size" | "page_size" | "max_keys" | "max_results" => {
                options =
                    options.with_list_page_size(count(name, value)?.try_into().unwrap_or(u16::MAX));
            }
            "proxy" | "proxy_uri" | "proxy_url" | "proxy_options" => {
                options = options.with_proxy(value);
            }
            "allow_bucket_creation" | "allow_container_creation" => {
                options = options.with_container_creation(flag(name, value)?);
            }
            "allow_bucket_deletion" | "allow_container_deletion" => {
                options = options.with_container_deletion(flag(name, value)?);
            }

            // --- encryption, which every store has and each spells its own way
            "sse" | "sse_type" | "server_side_encryption" | "encryption_type" => {
                parts.sse_type = Some(value.to_owned());
            }
            "sse_key" | "encryption_key" | "customer_key" => {
                parts.sse_key = Some(value.to_owned());
            }
            "sse_md5" | "sse_key_md5" | "encryption_key_sha256" | "customer_key_sha256" => {
                parts.sse_md5 = Some(value.to_owned());
            }
            "sse_kms_key_id" | "sse_kms_key" | "kms_key_id" | "kms_key_name" => {
                parts.kms_key_id = Some(value.to_owned());
            }
            "sse_context" | "sse_encryption_context" | "encryption_context" => {
                parts.sse_context = Some(value.to_owned());
            }
            "sse_bucket_key" | "sse_bucket_key_enabled" | "bucket_key_enabled" => {
                parts.bucket_key = Some(flag(name, value)?);
            }
            "encryption_scope" => parts.encryption_scope = Some(value.to_owned()),

            // --- Amazon S3's own --------------------------------------------
            "profile" | "profile_name" => {
                parts.aws = parts.aws.clone().with_profile(value);
            }
            "payload_signing" | "sign_payload" | "payload_signing_enabled" => {
                parts.aws = parts.aws.clone().with_payload_signing(flag(name, value)?);
            }
            "requester_pays" | "request_payer" | "requester_pays_enabled" => {
                parts.aws = parts
                    .aws
                    .clone()
                    .with_requester_pays(requester(name, value)?);
            }
            "checksum" | "checksum_algorithm" | "request_checksum_calculation" => {
                parts.aws = parts.aws.clone().with_checksum(value.parse::<Checksum>()?);
            }
            "role_arn" => parts.role_arn = Some(value.to_owned()),
            "session_name" | "role_session_name" => parts.role_session = Some(value.to_owned()),
            "external_id" | "role_external_id" => parts.external_id = Some(value.to_owned()),
            "role_duration" | "role_session_duration" | "assume_role_duration_seconds" => {
                parts.role_duration = Some(seconds(name, value)?);
            }
            "sts_endpoint" | "role_sts_endpoint" => parts.sts_endpoint = Some(value.to_owned()),
            "sts_region" | "role_region" => parts.sts_region = Some(value.to_owned()),

            // --- Google Cloud Storage's own ---------------------------------
            "project" | "project_id" | "cloud_project" => {
                parts.google = parts.google.clone().with_project(value);
            }
            "user_project" | "requester_pays_project" | "billing_project" => {
                parts.google = parts.google.clone().with_user_project(value);
            }
            "quota_project" | "quota_project_id" => {
                parts.google = parts.google.clone().with_quota_project(value);
            }
            "credentials_file"
            | "service_account_file"
            | "service_account"
            | "application_credentials"
            | "application_default_credentials" => {
                parts.google = parts.google.clone().with_credentials_file(value);
            }
            "credentials_json" | "service_account_json" | "service_account_info" => {
                parts.google = parts.google.clone().with_credentials_json(value);
            }
            "impersonate_service_account" | "impersonate" | "target_principal" => {
                parts.google = parts.google.clone().with_impersonation(value);
            }
            "oauth2_token" | "google_access_token" => {
                parts.google = parts.google.clone().with_access_token(value);
            }
            "scope" | "scopes" | "oauth_scope" => {
                parts.google = parts.google.clone().with_scope(value);
            }
            "predefined_acl" | "acl" | "canned_acl" => {
                parts.google = parts.google.clone().with_predefined_acl(value);
            }
            "metadata_host" | "gce_metadata_host" => {
                parts.google = parts.google.clone().with_metadata_host(value);
            }

            // --- Azure Blob Storage's own -----------------------------------
            "connection_string" => {
                parts.azure = parts.azure.clone().with_connection_string(value);
            }
            "account" | "account_name" => {
                parts.azure = parts.azure.clone().with_account(value);
            }
            "account_key" | "shared_key" | "azure_storage_key" => {
                parts.azure = parts.azure.clone().with_account_key(value);
            }
            "sas_token" | "sas" | "shared_access_signature" => {
                parts.azure = parts.azure.clone().with_sas_token(value);
            }
            "tenant_id" | "tenant" => {
                parts.tenant_id = Some(value.to_owned());
            }
            "client_secret" => parts.client_secret = Some(value.to_owned()),
            "federated_token_file" | "workload_identity_token_file" => {
                parts.azure = parts.azure.clone().with_federated_token_file(value);
            }
            "managed_identity" | "use_managed_identity" | "msi" => {
                parts.azure = parts
                    .azure
                    .clone()
                    .with_managed_identity(flag(name, value)?);
            }
            "authority_host" => {
                parts.azure = parts.azure.clone().with_authority_host(value);
            }
            "api_version" | "blob_api_version" => {
                parts.azure = parts.azure.clone().with_api_version(value);
            }
            "blob_type" => {
                parts.azure = parts
                    .azure
                    .clone()
                    .with_blob_type(value.parse::<BlobType>()?);
            }
            "access_tier" | "tier" | "blob_tier" => {
                parts.azure = parts.azure.clone().with_access_tier(value);
            }
            "data_lake" | "use_data_lake" | "hierarchical_namespace" | "adls_gen2" => {
                parts.azure = parts.azure.clone().with_data_lake(flag(name, value)?);
            }

            // --- names two stores both have ---------------------------------
            // Applied to each, because only the store that answers reads its
            // own options; nothing is ambiguous by the time it matters.
            "storage_class" => {
                parts.aws = parts.aws.clone().with_storage_class(value);
                parts.google = parts.google.clone().with_storage_class(value);
            }
            "client_id" => {
                parts.client_id = Some(value.to_owned());
            }
            "token" | "access_token" | "bearer_token" => {
                parts.google = parts.google.clone().with_access_token(value);
                parts.azure = parts.azure.clone().with_bearer_token(value);
            }

            // Named, understood, and not something this client can do. A
            // silent omission here would be found out at the store.
            "signer" | "signer_uri" | "signer_endpoint" => return Err(unsupported(name)),
            // Everything else belongs to something that is not a store.
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
    encryption_scope: Option<String>,
    tenant_id: Option<String>,
    client_id: Option<String>,
    client_secret: Option<String>,
    aws: AwsOptions,
    google: GoogleOptions,
    azure: AzureOptions,
}

impl Parts {
    /// Assemble what was collected onto `options`.
    fn apply(self, mut options: ObjectOptions) -> Result<ObjectOptions> {
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
        // An Azure account name and key are a credential pair like any other,
        // and the pair is what a shared-key signature is built from.
        let access_key = self
            .access_key
            .clone()
            .or_else(|| self.azure.account().map(str::to_owned));
        let secret_key = self.secret_key.clone().or_else(|| azure_key(&self.azure));
        if let (Some(access_key), Some(secret_key)) = (&access_key, &secret_key) {
            let mut credentials = Credentials::new(access_key, secret_key);
            if let Some(token) = &self.session_token {
                credentials = credentials.with_session_token(token);
            }
            options = options.with_credentials(credentials);
        }
        let mut aws = self.aws;
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
            aws = aws.with_assumed_role(role);
        }
        // An Entra ID application is three values or none of them, so it is
        // assembled here rather than one property at a time.
        let mut azure = self.azure;
        if let Some(client_id) = &self.client_id {
            azure = match (&self.tenant_id, &self.client_secret) {
                (Some(tenant), Some(secret)) => azure.with_client_secret(tenant, client_id, secret),
                _ => azure.with_client_id(client_id),
            };
        }
        if let Some(scope) = &self.encryption_scope {
            azure = azure.with_encryption_scope(scope);
        }
        let encryption = Encryption::from_parts(
            self.sse_type.as_deref(),
            self.sse_key.as_deref(),
            self.sse_md5.as_deref(),
            self.kms_key_id.as_deref(),
            self.sse_context.as_deref(),
            self.bucket_key,
        )?;
        if let Some(encryption) = encryption {
            options = options.with_encryption(encryption);
        }
        Ok(options
            .with_aws(aws)
            .with_google(self.google)
            .with_azure(azure))
    }
}

/// The shared key an Azure options block carries, when it carries one.
fn azure_key(options: &AzureOptions) -> Option<String> {
    options.account_key().map(str::to_owned)
}

/// The rest of `name` after `prefix`, matched without regard to case.
fn strip_prefix_ignoring_case<'name>(name: &'name str, prefix: &str) -> Option<&'name str> {
    let head = name.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &name[prefix.len()..])
        .filter(|rest| !rest.is_empty())
}

/// Every prefix a store's own vocabulary puts in front of a knob's name.
///
/// Peeled rather than stripped once, because a name may carry two:
/// `AWS_S3_FORCE_PATH_STYLE` and `azure.storage.account-name` both do.
const PREFIXES: [&str; 12] = [
    "s3_", "gcs_", "gs_", "google_", "gcp_", "azure_", "adls_", "abfs_", "blob_", "client_",
    "storage_", "aws_",
];

/// The name a property is matched by: case, separators, and prefix removed.
fn canonical(name: &str) -> String {
    let mut key = name.trim().to_ascii_lowercase().replace(['-', '.'], "_");
    loop {
        // `client_id` and `client_secret` are knobs of their own on Azure, so
        // the `client_` prefix is not peeled off them.
        if key == "client_id" || key == "client_secret" {
            return key;
        }
        let peeled = PREFIXES.iter().find_map(|prefix| key.strip_prefix(prefix));
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

/// Whether a request accepts requester-pays charges.
///
/// The AWS tools spell it `requester`, and everyone else spells it a boolean.
fn requester(name: &str, value: &str) -> Result<bool> {
    if value.eq_ignore_ascii_case("requester") {
        return Ok(true);
    }
    flag(name, value)
}

/// A duration in seconds, which is how every vocabulary spells one.
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
        "{name} asks for something this object-store client does not do; \
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
