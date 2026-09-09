//! The knobs that are Azure Blob Storage's own.
//!
//! Azure differs from the other two in what a location even is: the account is
//! part of the endpoint rather than of the path, so a bare `az://container/blob`
//! says nothing about which account holds it and something here has to. It also
//! authenticates four ways - a shared key, a token appended to the query, an
//! Entra ID bearer token, or nothing at all - and the choice belongs to the
//! client rather than to a request.

use std::path::{Path, PathBuf};

/// The REST API version every request states.
///
/// Azure versions its API by request header rather than by URL, and a stored
/// value is a contract: a newer version can change how a header is read, so it
/// is pinned here and moved deliberately.
pub const DEFAULT_API_VERSION: &str = "2025-05-05";

/// The account name Azurite serves, and the key every installation of it has.
///
/// Published by Microsoft as the development account, identical in every
/// emulator, and therefore a constant rather than a secret.
pub const DEVELOPMENT_ACCOUNT: &str = "devstoreaccount1";
/// The development account's key, as published.
pub const DEVELOPMENT_KEY: &str =
    "Eby8vdM02xNOcqFlqUwJPLlmEtlCDXJ1OUzFT50uSRZ6IFsuFq2UVErCz4I6tq/K1SZFPTOtr/KBHBeksoGMGw==";

/// Which kind of blob a write creates.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum BlobType {
    /// A block blob: staged blocks committed as a list. What a file is.
    #[default]
    Block,
    /// An append blob: bytes only ever added at the end.
    Append,
    /// A page blob: fixed-size pages, written at 512-byte boundaries.
    Page,
}

impl BlobType {
    /// The `x-ms-blob-type` this kind states.
    pub(crate) const fn as_str(&self) -> &'static str {
        match self {
            Self::Block => "BlockBlob",
            Self::Append => "AppendBlob",
            Self::Page => "PageBlob",
        }
    }
}

impl std::str::FromStr for BlobType {
    type Err = crate::Error;

    fn from_str(value: &str) -> crate::Result<Self> {
        match value
            .trim()
            .to_ascii_lowercase()
            .replace(['-', '_'], "")
            .as_str()
        {
            "block" | "blockblob" => Ok(Self::Block),
            "append" | "appendblob" => Ok(Self::Append),
            "page" | "pageblob" => Ok(Self::Page),
            _ => Err(crate::Error::Parse {
                target: "azure blob type",
                position: 0,
                reason: format!("expected block, append, or page, got {value:?}").into(),
            }),
        }
    }
}

/// How this backend reaches Azure Blob Storage in particular.
///
/// ```
/// use yggdryl::holder::object::{AzureOptions, ObjectOptions};
///
/// let options = ObjectOptions::default().with_azure(
///     AzureOptions::default()
///         .with_account("trades")
///         .with_sas_token("sv=2025-05-05&sig=..."),
/// );
/// assert_eq!(options.azure().account(), Some("trades"));
/// ```
#[derive(Clone, Default)]
pub struct AzureOptions {
    account: Option<String>,
    account_key: Option<String>,
    sas_token: Option<String>,
    bearer_token: Option<String>,
    tenant_id: Option<String>,
    client_id: Option<String>,
    client_secret: Option<String>,
    federated_token_file: Option<PathBuf>,
    managed_identity: bool,
    api_version: Option<String>,
    blob_type: BlobType,
    access_tier: Option<String>,
    data_lake: bool,
    authority_host: Option<String>,
    endpoint: Option<String>,
}

impl AzureOptions {
    /// Everything a connection string spells, read from `text`.
    ///
    /// This is the one intake most Azure configuration arrives as, so the whole
    /// document is read rather than a caller taking it apart: `AccountName`,
    /// `AccountKey`, `SharedAccessSignature`, `BlobEndpoint`,
    /// `DefaultEndpointsProtocol`, `EndpointSuffix`, and
    /// `UseDevelopmentStorage`. A key it does not know is ignored, and the
    /// endpoint it names is answered by [`Self::endpoint`].
    ///
    /// ```
    /// use yggdryl::holder::object::AzureOptions;
    ///
    /// let options = AzureOptions::default().with_connection_string(
    ///     "DefaultEndpointsProtocol=https;AccountName=trades;AccountKey=a2V5;EndpointSuffix=core.windows.net",
    /// );
    /// assert_eq!(options.account(), Some("trades"));
    /// assert_eq!(options.endpoint(), Some("https://trades.blob.core.windows.net"));
    /// ```
    #[must_use]
    pub fn with_connection_string(mut self, text: &str) -> Self {
        let mut protocol = None;
        let mut suffix = None;
        let mut endpoint = None;
        let mut development = false;
        for pair in text.split(';') {
            let Some((name, value)) = pair.split_once('=') else {
                continue;
            };
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            match name.trim().to_ascii_lowercase().as_str() {
                "accountname" => self.account = Some(value.to_owned()),
                // A key is base64 and may end in `=`, which the split above
                // would have cut; the value is taken from the first `=` on.
                "accountkey" => {
                    self.account_key = pair.split_once('=').map(|(_, key)| key.trim().to_owned());
                }
                "sharedaccesssignature" => {
                    self.sas_token = Some(value.trim_start_matches('?').to_owned());
                }
                "blobendpoint" => endpoint = Some(value.trim_end_matches('/').to_owned()),
                "defaultendpointsprotocol" => protocol = Some(value.to_ascii_lowercase()),
                "endpointsuffix" => suffix = Some(value.to_owned()),
                "usedevelopmentstorage" => {
                    development = matches!(value.to_ascii_lowercase().as_str(), "true" | "1");
                }
                _ => {}
            }
        }
        if development {
            self.account = Some(DEVELOPMENT_ACCOUNT.to_owned());
            self.account_key = Some(DEVELOPMENT_KEY.to_owned());
            self.endpoint = Some(format!("http://127.0.0.1:10000/{DEVELOPMENT_ACCOUNT}"));
            return self;
        }
        self.endpoint = endpoint.or_else(|| {
            let account = self.account.as_ref()?;
            let protocol = protocol.as_deref().unwrap_or("https");
            let suffix = suffix.as_deref().unwrap_or("core.windows.net");
            Some(format!("{protocol}://{account}.blob.{suffix}"))
        });
        self
    }

    /// Address the storage account named `account`.
    #[must_use]
    pub fn with_account(mut self, account: impl Into<String>) -> Self {
        self.account = Some(account.into());
        self
    }

    /// Sign with the account's shared key, which is base64 as Azure gives it.
    #[must_use]
    pub fn with_account_key(mut self, key: impl Into<String>) -> Self {
        self.account_key = Some(key.into());
        self
    }

    /// Authorize with a shared access signature rather than by signing.
    ///
    /// The token is a query string, and a leading `?` is dropped: it is what a
    /// caller copies out of the portal. Nothing else is signed when one is set,
    /// because the token already carries the signature.
    #[must_use]
    pub fn with_sas_token(mut self, token: impl Into<String>) -> Self {
        let token: String = token.into();
        self.sas_token = Some(token.trim().trim_start_matches('?').to_owned());
        self
    }

    /// Authorize with a bearer token the caller already holds.
    #[must_use]
    pub fn with_bearer_token(mut self, token: impl Into<String>) -> Self {
        self.bearer_token = Some(token.into());
        self
    }

    /// Obtain a bearer token as the Entra ID application `client_id`.
    #[must_use]
    pub fn with_client_secret(
        mut self,
        tenant_id: impl Into<String>,
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
    ) -> Self {
        self.tenant_id = Some(tenant_id.into());
        self.client_id = Some(client_id.into());
        self.client_secret = Some(client_secret.into());
        self
    }

    /// Exchange the federated token in `path` for one of the account's.
    ///
    /// This is what workload identity gives a pod: a projected file holding a
    /// signed assertion, traded at the same endpoint a client secret is.
    #[must_use]
    pub fn with_federated_token_file(mut self, path: impl AsRef<Path>) -> Self {
        self.federated_token_file = Some(path.as_ref().to_path_buf());
        self
    }

    /// Ask the instance metadata service for a token.
    ///
    /// With a `client_id` set, the user-assigned identity of that name; without
    /// one, the system-assigned identity.
    #[must_use]
    pub const fn with_managed_identity(mut self, enabled: bool) -> Self {
        self.managed_identity = enabled;
        self
    }

    /// Name the Entra ID application whose identity is used.
    #[must_use]
    pub fn with_client_id(mut self, client_id: impl Into<String>) -> Self {
        self.client_id = Some(client_id.into());
        self
    }

    /// Obtain tokens from `host` rather than `https://login.microsoftonline.com`.
    #[must_use]
    pub fn with_authority_host(mut self, host: impl Into<String>) -> Self {
        let host: String = host.into();
        self.authority_host = Some(host.trim().trim_end_matches('/').to_owned());
        self
    }

    /// State `version` as the REST API version rather than
    /// [`DEFAULT_API_VERSION`].
    #[must_use]
    pub fn with_api_version(mut self, version: impl Into<String>) -> Self {
        self.api_version = Some(version.into());
        self
    }

    /// Create blobs of `kind` rather than block blobs.
    #[must_use]
    pub const fn with_blob_type(mut self, kind: BlobType) -> Self {
        self.blob_type = kind;
        self
    }

    /// Write into the access tier `tier`: `Hot`, `Cool`, `Cold`, or `Archive`.
    #[must_use]
    pub fn with_access_tier(mut self, tier: impl Into<String>) -> Self {
        self.access_tier = Some(tier.into());
        self
    }

    /// Address the Data Lake Storage Gen2 endpoint rather than the blob one.
    ///
    /// An account with a hierarchical namespace answers both; `dfs` is the one
    /// that has real directories, and `abfss` is the scheme that names it.
    #[must_use]
    pub const fn with_data_lake(mut self, enabled: bool) -> Self {
        self.data_lake = enabled;
        self
    }

    /// The account addressed, when one was named.
    pub fn account(&self) -> Option<&str> {
        self.account.as_deref()
    }

    /// The shared key, when one was given.
    pub(crate) fn account_key(&self) -> Option<&str> {
        self.account_key.as_deref()
    }

    /// Whether a shared key was given, without rendering it.
    pub fn has_account_key(&self) -> bool {
        self.account_key.is_some()
    }

    /// The shared access signature, when one was given.
    pub(crate) fn sas_token(&self) -> Option<&str> {
        self.sas_token.as_deref()
    }

    /// Whether a shared access signature was given, without rendering it.
    pub fn has_sas_token(&self) -> bool {
        self.sas_token.is_some()
    }

    /// The bearer token the caller supplied, when they supplied one.
    pub(crate) fn bearer_token(&self) -> Option<&str> {
        self.bearer_token.as_deref()
    }

    /// The Entra ID tenant, when one was named.
    pub fn tenant_id(&self) -> Option<&str> {
        self.tenant_id.as_deref()
    }

    /// The Entra ID application, when one was named.
    pub fn client_id(&self) -> Option<&str> {
        self.client_id.as_deref()
    }

    /// The application secret, when one was given.
    pub(crate) fn client_secret(&self) -> Option<&str> {
        self.client_secret.as_deref()
    }

    /// The federated token file, when one was named.
    pub fn federated_token_file(&self) -> Option<&Path> {
        self.federated_token_file.as_deref()
    }

    /// Whether the instance metadata service is asked for a token.
    pub const fn managed_identity(&self) -> bool {
        self.managed_identity
    }

    /// Where tokens are obtained from.
    pub fn authority_host(&self) -> &str {
        self.authority_host
            .as_deref()
            .unwrap_or("https://login.microsoftonline.com")
    }

    /// The REST API version every request states.
    pub fn api_version(&self) -> &str {
        self.api_version.as_deref().unwrap_or(DEFAULT_API_VERSION)
    }

    /// The kind of blob a write creates.
    pub const fn blob_type(&self) -> BlobType {
        self.blob_type
    }

    /// The access tier writes ask for, when one was named.
    pub fn access_tier(&self) -> Option<&str> {
        self.access_tier.as_deref()
    }

    /// Whether the Data Lake endpoint is addressed.
    pub const fn data_lake(&self) -> bool {
        self.data_lake
    }

    /// The endpoint a connection string named, when one did.
    pub fn endpoint(&self) -> Option<&str> {
        self.endpoint.as_deref()
    }

    /// Fill from `ambient` every knob this one does not set for itself.
    pub(crate) fn under(mut self, ambient: &Self) -> Self {
        if self.account.is_none() {
            self.account = ambient.account.clone();
        }
        if self.account_key.is_none() && self.sas_token.is_none() {
            self.account_key = ambient.account_key.clone();
            self.sas_token = ambient.sas_token.clone();
        }
        if self.bearer_token.is_none() {
            self.bearer_token = ambient.bearer_token.clone();
        }
        if self.tenant_id.is_none() {
            self.tenant_id = ambient.tenant_id.clone();
        }
        if self.client_id.is_none() {
            self.client_id = ambient.client_id.clone();
        }
        if self.client_secret.is_none() {
            self.client_secret = ambient.client_secret.clone();
        }
        if self.federated_token_file.is_none() {
            self.federated_token_file = ambient.federated_token_file.clone();
        }
        if !self.managed_identity {
            self.managed_identity = ambient.managed_identity;
        }
        if self.api_version.is_none() {
            self.api_version = ambient.api_version.clone();
        }
        if self.blob_type == BlobType::Block {
            self.blob_type = ambient.blob_type;
        }
        if self.access_tier.is_none() {
            self.access_tier = ambient.access_tier.clone();
        }
        if !self.data_lake {
            self.data_lake = ambient.data_lake;
        }
        if self.authority_host.is_none() {
            self.authority_host = ambient.authority_host.clone();
        }
        if self.endpoint.is_none() {
            self.endpoint = ambient.endpoint.clone();
        }
        self
    }
}

impl std::fmt::Debug for AzureOptions {
    /// Every secret answers as whether it is there, never as what it is.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AzureOptions")
            .field("account", &self.account)
            .field(
                "account_key",
                &self.account_key.as_ref().map(|_| "<redacted>"),
            )
            .field("sas_token", &self.sas_token.as_ref().map(|_| "<redacted>"))
            .field(
                "bearer_token",
                &self.bearer_token.as_ref().map(|_| "<redacted>"),
            )
            .field("tenant_id", &self.tenant_id)
            .field("client_id", &self.client_id)
            .field(
                "client_secret",
                &self.client_secret.as_ref().map(|_| "<redacted>"),
            )
            .field("federated_token_file", &self.federated_token_file)
            .field("managed_identity", &self.managed_identity)
            .field("api_version", &self.api_version)
            .field("blob_type", &self.blob_type)
            .field("access_tier", &self.access_tier)
            .field("data_lake", &self.data_lake)
            .field("authority_host", &self.authority_host)
            .field("endpoint", &self.endpoint)
            .finish()
    }
}
