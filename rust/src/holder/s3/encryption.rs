//! How the store encrypts what it holds, and what a request has to say.
//!
//! S3 encrypts at rest three ways, and they differ in *who holds the key* -
//! which is the same thing as differing in what goes on the wire.

use base64::Engine as _;

use crate::{Error, Result};

/// The length of an `AES256` key, in bytes.
const AES256_KEY_LENGTH: usize = 32;

/// Server-side encryption, and the key material a request carries for it.
///
/// | | how a write says it | what a read carries |
/// | --- | --- | --- |
/// | [`Encryption::Default`] | nothing; the bucket's own rule applies | nothing |
/// | [`Encryption::Managed`] | `AES256` | nothing |
/// | [`Encryption::Kms`] | `aws:kms`, and which key | nothing |
/// | [`Encryption::Customer`] | the key itself | the key itself |
///
/// The last row is what shapes this type. A key the store never keeps has to
/// be presented again on every read, so the choice belongs to the client
/// rather than to one write - which is why it is an [`S3Options`] knob and not
/// an argument.
///
/// [`S3Options`]: super::S3Options
///
/// ```
/// use yggdryl::holder::s3::{Encryption, KmsKey, S3Options};
///
/// // The bucket's own default, which is what most callers want.
/// assert!(matches!(S3Options::default().encryption(), Encryption::Default));
///
/// // A named KMS key, with an encryption context and S3 Bucket Keys on.
/// let key = KmsKey::new("arn:aws:kms:eu-west-1:1234:key/abcd")
///     .with_context(r#"{"desk":"power"}"#)
///     .with_bucket_key(true);
/// let options = S3Options::default().with_encryption(Encryption::Kms(key));
/// assert!(matches!(options.encryption(), Encryption::Kms(_)));
/// ```
#[derive(Clone, Default)]
pub enum Encryption {
    /// Say nothing, and let the bucket's own rule decide.
    #[default]
    Default,
    /// `SSE-S3`: keys the store manages, named `AES256` on the wire.
    Managed,
    /// `SSE-KMS`: a key AWS KMS manages, named `aws:kms` on the wire.
    Kms(KmsKey),
    /// `SSE-C`: a key the caller holds and the store forgets.
    Customer(CustomerKey),
}

/// Which KMS key encrypts a write, and how.
///
/// Every field is optional except the choice of KMS itself: a bucket with a
/// default key needs only [`KmsKey::default`], and naming a key overrides it.
#[derive(Clone, Debug, Default)]
pub struct KmsKey {
    key_id: Option<String>,
    context: Option<String>,
    bucket_key: Option<bool>,
    dual_layer: bool,
}

/// A key the caller holds, which the store uses and does not keep.
///
/// Built once and carried on every request that touches the object's bytes,
/// so the base64 spellings are computed here rather than per request.
#[derive(Clone)]
pub struct CustomerKey {
    /// Base64 of the raw key, which is how the header spells it.
    encoded: String,
    /// Base64 of the key's MD5, which S3 uses to catch a mangled header.
    checksum: String,
}

impl Encryption {
    /// Encrypt with keys the store manages, which is `SSE-S3`.
    #[must_use]
    pub const fn managed() -> Self {
        Self::Managed
    }

    /// Encrypt with the KMS key `key_id`, which is `SSE-KMS`.
    #[must_use]
    pub fn kms(key_id: impl Into<String>) -> Self {
        Self::Kms(KmsKey::new(key_id))
    }

    /// Encrypt with the bucket's own default KMS key.
    #[must_use]
    pub fn kms_default() -> Self {
        Self::Kms(KmsKey::default())
    }

    /// Encrypt with the caller's own 32-byte key, which is `SSE-C`.
    ///
    /// # Errors
    ///
    /// Returns a refusal when `key` is not the 32 bytes `AES256` takes.
    pub fn customer(key: &[u8]) -> Result<Self> {
        CustomerKey::new(key).map(Self::Customer)
    }

    /// Whether anything is said at all, which is what a default does not.
    pub const fn is_default(&self) -> bool {
        matches!(self, Self::Default)
    }

    /// The headers a request deciding how an object is *stored* carries.
    ///
    /// `PutObject` and `CreateMultipartUpload` are the two that decide it;
    /// `UploadPart` inherits what the upload was created with, and carries
    /// only [`Self::read_headers`].
    ///
    /// Public because it is exactly what another client's headers are compared
    /// against: the interop driver puts these beside `botocore`'s for the same
    /// key, which is how a spelling that only this crate agrees with is
    /// caught.
    pub fn write_headers(&self) -> Vec<(&'static str, String)> {
        match self {
            Self::Default => Vec::new(),
            Self::Managed => vec![("x-amz-server-side-encryption", "AES256".to_owned())],
            Self::Kms(key) => key.headers(),
            Self::Customer(key) => key.headers(),
        }
    }

    /// The headers a request touching an object's *bytes* carries.
    ///
    /// Only a customer key needs any: the store remembers which of its own
    /// keys it used, and a key it never kept has to be presented again - on
    /// reads as much as on writes, which is the whole cost of `SSE-C`.
    pub fn read_headers(&self) -> Vec<(&'static str, String)> {
        match self {
            Self::Customer(key) => key.headers(),
            _ => Vec::new(),
        }
    }
}

impl KmsKey {
    /// Name `key_id` - an ARN, a key id, or an alias - as the key to use.
    #[must_use]
    pub fn new(key_id: impl Into<String>) -> Self {
        Self {
            key_id: Some(key_id.into()),
            ..Self::default()
        }
    }

    /// Bind the ciphertext to `context`, a JSON object KMS records and
    /// requires again to decrypt.
    #[must_use]
    pub fn with_context(mut self, context: impl Into<String>) -> Self {
        self.context = Some(context.into());
        self
    }

    /// Ask for, or refuse, an S3 Bucket Key.
    ///
    /// A bucket key is one KMS call per bucket and time window rather than one
    /// per object, which is what makes writing a lake of small parts under
    /// `SSE-KMS` affordable.
    #[must_use]
    pub const fn with_bucket_key(mut self, enabled: bool) -> Self {
        self.bucket_key = Some(enabled);
        self
    }

    /// Encrypt twice, which is what `aws:kms:dsse` names.
    #[must_use]
    pub const fn with_dual_layer(mut self, dual_layer: bool) -> Self {
        self.dual_layer = dual_layer;
        self
    }

    /// The named key, when one was named.
    pub fn key_id(&self) -> Option<&str> {
        self.key_id.as_deref()
    }

    /// The encryption context, as the JSON it was given as.
    pub fn context(&self) -> Option<&str> {
        self.context.as_deref()
    }

    /// The bucket-key choice, when one was made.
    pub const fn bucket_key(&self) -> Option<bool> {
        self.bucket_key
    }

    /// Whether this asks for dual-layer encryption.
    pub const fn dual_layer(&self) -> bool {
        self.dual_layer
    }

    /// What a write says about this key.
    fn headers(&self) -> Vec<(&'static str, String)> {
        let algorithm = if self.dual_layer {
            "aws:kms:dsse"
        } else {
            "aws:kms"
        };
        let mut headers = vec![("x-amz-server-side-encryption", algorithm.to_owned())];
        if let Some(key_id) = &self.key_id {
            headers.push((
                "x-amz-server-side-encryption-aws-kms-key-id",
                key_id.clone(),
            ));
        }
        if let Some(context) = &self.context {
            // The context goes over as base64 of the JSON, not as the JSON.
            headers.push((
                "x-amz-server-side-encryption-context",
                base64::engine::general_purpose::STANDARD.encode(context),
            ));
        }
        if let Some(enabled) = self.bucket_key {
            headers.push((
                "x-amz-server-side-encryption-bucket-key-enabled",
                enabled.to_string(),
            ));
        }
        headers
    }
}

impl CustomerKey {
    /// Hold the 32-byte `AES256` key `key`.
    ///
    /// # Errors
    ///
    /// Returns a refusal when `key` is not 32 bytes long.
    pub fn new(key: &[u8]) -> Result<Self> {
        if key.len() != AES256_KEY_LENGTH {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "expected an AES256 key of {AES256_KEY_LENGTH} bytes, got {}",
                    key.len()
                ),
            )));
        }
        let engine = base64::engine::general_purpose::STANDARD;
        Ok(Self {
            encoded: engine.encode(key),
            checksum: engine.encode(super::client::md5_of(key)),
        })
    }

    /// Hold the key `key` spells in base64, which is how AWS tools carry it.
    ///
    /// # Errors
    ///
    /// Returns a refusal when `key` is not base64, or does not decode to the
    /// 32 bytes `AES256` takes.
    pub fn from_base64(key: &str) -> Result<Self> {
        let raw = base64::engine::general_purpose::STANDARD
            .decode(key.trim())
            .map_err(|error| {
                Error::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("expected a base64 AES256 key: {error}"),
                ))
            })?;
        Self::new(&raw)
    }

    /// The same, checked against a `checksum` the caller was given.
    ///
    /// PyIceberg and the AWS tools carry the MD5 alongside the key, and a
    /// mismatch means one of the two was copied wrong - which is worth
    /// hearing here rather than as a `400` from the store.
    ///
    /// # Errors
    ///
    /// Returns a refusal when the key is unusable, or when `checksum` is not
    /// the key's.
    pub fn from_base64_checked(key: &str, checksum: &str) -> Result<Self> {
        let held = Self::from_base64(key)?;
        if held.checksum != checksum.trim() {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "expected the customer key's MD5 to match the key it came with",
            )));
        }
        Ok(held)
    }

    /// The key, base64 as the header spells it.
    pub fn encoded(&self) -> &str {
        &self.encoded
    }

    /// Base64 of the key's MD5, as the header spells it.
    pub fn checksum(&self) -> &str {
        &self.checksum
    }

    /// What every request touching this object's bytes carries.
    fn headers(&self) -> Vec<(&'static str, String)> {
        vec![
            (
                "x-amz-server-side-encryption-customer-algorithm",
                "AES256".to_owned(),
            ),
            (
                "x-amz-server-side-encryption-customer-key",
                self.encoded.clone(),
            ),
            (
                "x-amz-server-side-encryption-customer-key-MD5",
                self.checksum.clone(),
            ),
        ]
    }
}

/// The key is a secret, so it is named rather than rendered.
impl std::fmt::Debug for CustomerKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CustomerKey")
            .field("key", &"<redacted>")
            .field("md5", &self.checksum)
            .finish()
    }
}

impl std::fmt::Debug for Encryption {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Default => formatter.write_str("Default"),
            Self::Managed => formatter.write_str("Managed"),
            Self::Kms(key) => formatter.debug_tuple("Kms").field(key).finish(),
            Self::Customer(key) => formatter.debug_tuple("Customer").field(key).finish(),
        }
    }
}
