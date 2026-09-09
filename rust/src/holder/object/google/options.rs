//! The knobs that are Google Cloud Storage's own.
//!
//! Google authenticates with a bearer token rather than with a signature over
//! the request, so most of what is here is about *where that token comes from*:
//! a service-account key, the credentials `gcloud` wrote, a token the caller
//! already holds, an identity to impersonate, or the metadata server every
//! instance has. The rest are the per-object facts the JSON API takes that no
//! other store has - the project that is billed, a predefined ACL, a storage
//! class.

use std::path::{Path, PathBuf};

/// The OAuth 2.0 scope a read-write storage client asks for.
///
/// `devstorage.full_control` is what the official clients request, because
/// listing, reading, writing, and ACLs are all in scope for a filesystem-shaped
/// client; a caller who wants less says so.
pub const DEFAULT_SCOPE: &str = "https://www.googleapis.com/auth/devstorage.full_control";

/// How this backend reaches Google Cloud Storage in particular.
///
/// ```
/// use yggdryl::holder::object::{GoogleOptions, ObjectOptions};
///
/// let options = ObjectOptions::default().with_google(
///     GoogleOptions::default()
///         .with_project("trading-analytics")
///         .with_storage_class("NEARLINE"),
/// );
/// assert_eq!(options.google().project(), Some("trading-analytics"));
/// ```
#[derive(Clone, Debug, Default)]
pub struct GoogleOptions {
    project: Option<String>,
    user_project: Option<String>,
    quota_project: Option<String>,
    credentials_file: Option<PathBuf>,
    credentials_json: Option<String>,
    access_token: Option<String>,
    impersonate: Option<String>,
    scope: Option<String>,
    storage_class: Option<String>,
    predefined_acl: Option<String>,
    metadata_host: Option<String>,
}

impl GoogleOptions {
    /// Own new buckets under `project`.
    ///
    /// Creating a bucket is the one operation that needs it: an object lives in
    /// a bucket, and a bucket lives in a project. Nothing else reads it.
    #[must_use]
    pub fn with_project(mut self, project: impl Into<String>) -> Self {
        self.project = Some(project.into());
        self
    }

    /// Bill requests to `project` rather than to the bucket's owner.
    ///
    /// This is the `userProject` a requester-pays bucket demands, and the same
    /// parameter a caller uses to attribute usage.
    #[must_use]
    pub fn with_user_project(mut self, project: impl Into<String>) -> Self {
        self.user_project = Some(project.into());
        self
    }

    /// Count requests against `project`'s quota.
    #[must_use]
    pub fn with_quota_project(mut self, project: impl Into<String>) -> Self {
        self.quota_project = Some(project.into());
        self
    }

    /// Authenticate with the credentials in the file at `path`.
    ///
    /// The file is either a service-account key or the authorized-user document
    /// `gcloud auth application-default login` writes; which one it is, is read
    /// from its own `type`, exactly as `GOOGLE_APPLICATION_CREDENTIALS` is.
    #[must_use]
    pub fn with_credentials_file(mut self, path: impl AsRef<Path>) -> Self {
        self.credentials_file = Some(path.as_ref().to_path_buf());
        self
    }

    /// Authenticate with the credentials `json` spells, read from memory.
    ///
    /// A deployment that hands a key over in an environment variable or a
    /// secret manager never writes it to a file, so this is the same intake
    /// without the file.
    #[must_use]
    pub fn with_credentials_json(mut self, json: impl Into<String>) -> Self {
        self.credentials_json = Some(json.into());
        self
    }

    /// Authenticate with a bearer token the caller already holds.
    ///
    /// Nothing is exchanged and nothing is refreshed: the token is sent as it
    /// stands until it expires, which is what a caller who obtained one from
    /// their own identity layer wants.
    #[must_use]
    pub fn with_access_token(mut self, token: impl Into<String>) -> Self {
        self.access_token = Some(token.into());
        self
    }

    /// Reach the store as the service account `email`, impersonating it.
    ///
    /// Whatever the credential chain answers signs one call to
    /// `iamcredentials.googleapis.com`, and the token that call returns is what
    /// reaches the store - the same shape as an assumed role on AWS.
    #[must_use]
    pub fn with_impersonation(mut self, email: impl Into<String>) -> Self {
        self.impersonate = Some(email.into());
        self
    }

    /// Ask for `scope` rather than [`DEFAULT_SCOPE`].
    #[must_use]
    pub fn with_scope(mut self, scope: impl Into<String>) -> Self {
        self.scope = Some(scope.into());
        self
    }

    /// Store what is written in `class` rather than in the bucket's default.
    ///
    /// `STANDARD`, `NEARLINE`, `COLDLINE`, `ARCHIVE`: passed through, because
    /// the list is Google's.
    #[must_use]
    pub fn with_storage_class(mut self, class: impl Into<String>) -> Self {
        self.storage_class = Some(class.into());
        self
    }

    /// Apply the predefined ACL `acl` to what is written.
    ///
    /// `private`, `publicRead`, `bucketOwnerFullControl`, and the rest. A
    /// bucket with uniform bucket-level access refuses any of them, which is
    /// why this is unset by default.
    #[must_use]
    pub fn with_predefined_acl(mut self, acl: impl Into<String>) -> Self {
        self.predefined_acl = Some(acl.into());
        self
    }

    /// Ask `host` for an instance token instead of `metadata.google.internal`.
    ///
    /// `GCE_METADATA_HOST` is the variable the official clients read, and a test
    /// that stands a fake metadata server up needs the same switch.
    #[must_use]
    pub fn with_metadata_host(mut self, host: impl Into<String>) -> Self {
        self.metadata_host = Some(host.into());
        self
    }

    /// The project new buckets are owned by, when one was named.
    pub fn project(&self) -> Option<&str> {
        self.project.as_deref()
    }

    /// The project requests are billed to, when one was named.
    pub fn user_project(&self) -> Option<&str> {
        self.user_project.as_deref()
    }

    /// The project whose quota requests count against, when one was named.
    pub fn quota_project(&self) -> Option<&str> {
        self.quota_project.as_deref()
    }

    /// The credentials file, when one was named.
    pub fn credentials_file(&self) -> Option<&Path> {
        self.credentials_file.as_deref()
    }

    /// The credentials document, when one was given directly.
    pub fn credentials_json(&self) -> Option<&str> {
        self.credentials_json.as_deref()
    }

    /// The bearer token the caller supplied, when they supplied one.
    pub fn access_token(&self) -> Option<&str> {
        self.access_token.as_deref()
    }

    /// The service account being impersonated, when one was named.
    pub fn impersonation(&self) -> Option<&str> {
        self.impersonate.as_deref()
    }

    /// The scope tokens are asked for with.
    pub fn scope(&self) -> &str {
        self.scope.as_deref().unwrap_or(DEFAULT_SCOPE)
    }

    /// The storage class writes ask for, when one was named.
    pub fn storage_class(&self) -> Option<&str> {
        self.storage_class.as_deref()
    }

    /// The predefined ACL writes ask for, when one was named.
    pub fn predefined_acl(&self) -> Option<&str> {
        self.predefined_acl.as_deref()
    }

    /// The metadata host instance tokens are asked of, when one was named.
    pub fn metadata_host(&self) -> Option<&str> {
        self.metadata_host.as_deref()
    }

    /// Fill from `ambient` every knob this one does not set for itself.
    pub(crate) fn under(mut self, ambient: &Self) -> Self {
        if self.project.is_none() {
            self.project = ambient.project.clone();
        }
        if self.user_project.is_none() {
            self.user_project = ambient.user_project.clone();
        }
        if self.quota_project.is_none() {
            self.quota_project = ambient.quota_project.clone();
        }
        if self.credentials_file.is_none() && self.credentials_json.is_none() {
            self.credentials_file = ambient.credentials_file.clone();
            self.credentials_json = ambient.credentials_json.clone();
        }
        if self.access_token.is_none() {
            self.access_token = ambient.access_token.clone();
        }
        if self.impersonate.is_none() {
            self.impersonate = ambient.impersonate.clone();
        }
        if self.scope.is_none() {
            self.scope = ambient.scope.clone();
        }
        if self.storage_class.is_none() {
            self.storage_class = ambient.storage_class.clone();
        }
        if self.predefined_acl.is_none() {
            self.predefined_acl = ambient.predefined_acl.clone();
        }
        if self.metadata_host.is_none() {
            self.metadata_host = ambient.metadata_host.clone();
        }
        self
    }
}
