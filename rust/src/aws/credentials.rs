//! The keys that sign a request, and when they stop being accepted.
//!
//! One credential set is a value: an access key, its secret, the session
//! token a temporary set carries, and when that set lapses. Where a set comes
//! from is the chain [`Session`](super::Session) walks; this file owns only
//! what one set is and the document every metadata service and a
//! `credential_process` answer one in. When a set is replaced is the
//! session's lease.

use std::time::SystemTime;

use crate::auth::{Expiring, Secret, instant};
use crate::{Error, Result};

/// An access key, its secret, and the session token a temporary set carries.
///
/// The secret never appears in `Debug` output or in an error.
///
/// ```
/// use yggdryl::aws::Credentials;
///
/// let keys = Credentials::new("AKIAIOSFODNN7EXAMPLE", "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY");
/// assert_eq!(keys.access_key_id(), "AKIAIOSFODNN7EXAMPLE");
/// assert_eq!(keys.session_token(), None);
/// assert!(!keys.is_temporary());
/// assert!(!format!("{keys:?}").contains("wJalrXUtnFEMI"));
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Credentials {
    access_key_id: String,
    secret_access_key: Secret,
    session_token: Option<Secret>,
    /// When a temporary set stops being accepted; a long-lived set has none.
    expires_at: Option<SystemTime>,
    /// The account the keys belong to, when the source said.
    account_id: Option<String>,
}

impl Credentials {
    /// A long-lived key pair.
    pub fn new(access_key_id: impl Into<String>, secret_access_key: impl Into<String>) -> Self {
        Self {
            access_key_id: access_key_id.into(),
            secret_access_key: Secret::new(secret_access_key),
            session_token: None,
            expires_at: None,
            account_id: None,
        }
    }

    /// Carry the session token a temporary set signs with.
    #[must_use]
    pub fn with_session_token(mut self, token: impl Into<String>) -> Self {
        let token: String = token.into();
        self.session_token = (!token.is_empty()).then(|| Secret::new(token));
        self
    }

    /// Record when a temporary set lapses.
    #[must_use]
    pub const fn with_expiry(mut self, expires_at: SystemTime) -> Self {
        self.expires_at = Some(expires_at);
        self
    }

    /// Record which account the keys belong to.
    #[must_use]
    pub fn with_account_id(mut self, account_id: impl Into<String>) -> Self {
        let account_id: String = account_id.into();
        self.account_id = (!account_id.is_empty()).then_some(account_id);
        self
    }

    /// The access key id.
    pub fn access_key_id(&self) -> &str {
        &self.access_key_id
    }

    /// The secret, for the signer alone.
    pub(crate) fn secret_access_key(&self) -> &str {
        self.secret_access_key.expose()
    }

    /// The session token, when the set is temporary.
    pub fn session_token(&self) -> Option<&str> {
        self.session_token.as_ref().map(Secret::expose)
    }

    /// When the set lapses, when it does.
    pub const fn expires_at(&self) -> Option<SystemTime> {
        self.expires_at
    }

    /// The account the keys belong to, when the source said.
    pub fn account_id(&self) -> Option<&str> {
        self.account_id.as_deref()
    }

    /// Whether the set carries a session token or an expiry - a set that
    /// was issued rather than created.
    pub const fn is_temporary(&self) -> bool {
        self.session_token.is_some() || self.expires_at.is_some()
    }
}

impl Expiring for Credentials {
    fn expires_at(&self) -> Option<SystemTime> {
        self.expires_at
    }
}

/// The JSON document the metadata services, the container endpoint and a
/// `credential_process` all answer with.
///
/// The keys are `AccessKeyId`, `SecretAccessKey`, `Token` or `SessionToken`,
/// `Expiration` and `AccountId`; a process states `Version: 1` beside them,
/// which is checked when present.
///
/// # Errors
///
/// A body that is not JSON, or one without an access key id and a secret.
pub(crate) fn parse_document(body: &[u8], source: &str) -> Result<Credentials> {
    let document: serde_json::Value = serde_json::from_slice(body).map_err(|error| {
        Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("expected a JSON credential document from {source}, got {error}"),
        ))
    })?;
    let text = |name: &str| {
        document
            .get(name)
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    if let Some(version) = document.get("Version") {
        if version.as_i64() != Some(1) {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("expected Version 1 from {source}, got {version}"),
            )));
        }
    }
    let (Some(access_key_id), Some(secret_access_key)) =
        (text("AccessKeyId"), text("SecretAccessKey"))
    else {
        return Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("expected AccessKeyId and SecretAccessKey from {source}, got neither"),
        )));
    };
    let mut credentials = Credentials::new(access_key_id, secret_access_key);
    if let Some(token) = text("Token").or_else(|| text("SessionToken")) {
        credentials = credentials.with_session_token(token);
    }
    if let Some(account_id) = text("AccountId") {
        credentials = credentials.with_account_id(account_id);
    }
    if let Some(expiry) = text("Expiration").as_deref().and_then(instant) {
        credentials = credentials.with_expiry(expiry);
    }
    Ok(credentials)
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/aws/credentials.rs` pins and a caller cannot reach.
    //!
    //! What a credential set is read out of - one document - is pinned here,
    //! and the edges a set crosses are pinned through the lease that holds
    //! it, in `rust/tests/auth/lease.rs`. Each item forwards.
    use crate::Result;
    use crate::aws::Credentials;

    /// The credential set one JSON document states.
    ///
    /// # Errors
    ///
    /// A document without an access key id and a secret, or with a version
    /// that is not 1.
    pub fn parse_document(body: &[u8], source: &str) -> Result<Credentials> {
        super::parse_document(body, source)
    }
}
