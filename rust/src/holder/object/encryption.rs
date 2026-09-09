//! How a store encrypts what it holds, and what a request has to say.
//!
//! All three stores encrypt at rest, and the interesting question in each is
//! the same one: *who holds the key* - which is the same thing as asking what
//! goes on the wire. So the choice is one value across the three, and each
//! store spells it its own way or says it does not have it.
//!
//! | | Amazon S3 | Google Cloud Storage | Azure Blob Storage |
//! | --- | --- | --- | --- |
//! | [`Encryption::Default`] | the bucket's rule | the bucket's rule | the account's rule |
//! | [`Encryption::Managed`] | `SSE-S3` | already the default | already the default |
//! | [`Encryption::Kms`] | `SSE-KMS`, `aws:kms:dsse` | a CMEK `kmsKeyName` | not this shape; use a scope |
//! | [`Encryption::Customer`] | `SSE-C` | a customer-supplied key | a customer-provided key |
//! | [`Encryption::Scope`] | not this shape | not this shape | an encryption scope |
//!
//! A combination a store does not have is refused once, when the client is
//! built, rather than silently dropped or discovered from the store.

use base64::Engine as _;
use sha2::{Digest as _, Sha256};

use super::provider::Provider;
use crate::{Error, Result};

/// The length of an `AES256` key, in bytes.
const AES256_KEY_LENGTH: usize = 32;

/// Server-side encryption, and the key material a request carries for it.
///
/// | | how a write says it | what a read carries |
/// | --- | --- | --- |
/// | [`Encryption::Default`] | nothing; the container's own rule applies | nothing |
/// | [`Encryption::Managed`] | the store's own keys | nothing |
/// | [`Encryption::Kms`] | which managed key | nothing |
/// | [`Encryption::Customer`] | the key itself | the key itself |
/// | [`Encryption::Scope`] | which account scope | nothing |
///
/// The last row is what shapes this type. A key the store never keeps has to
/// be presented again on every read, so the choice belongs to the client
/// rather than to one write - which is why it is an [`ObjectOptions`] knob and not
/// an argument.
///
/// [`ObjectOptions`]: super::ObjectOptions
///
/// ```
/// use yggdryl::holder::object::{Encryption, KmsKey, ObjectOptions};
///
/// // The bucket's own default, which is what most callers want.
/// assert!(matches!(ObjectOptions::default().encryption(), Encryption::Default));
///
/// // A named KMS key, with an encryption context and S3 Bucket Keys on.
/// let key = KmsKey::new("arn:aws:kms:eu-west-1:1234:key/abcd")
///     .with_context(r#"{"desk":"power"}"#)
///     .with_bucket_key(true);
/// let options = ObjectOptions::default().with_encryption(Encryption::Kms(key));
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
    /// An Azure encryption scope: a key configured on the account, named per
    /// request. Azure's shape for a customer-managed key, and only Azure's.
    Scope(String),
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
/// so the base64 spellings are computed here rather than per request. Both
/// digests are kept because the three stores ask for different ones: S3 wants
/// the key's MD5, Google and Azure want its SHA-256.
#[derive(Clone)]
pub struct CustomerKey {
    /// Base64 of the raw key, which is how the header spells it.
    encoded: String,
    /// Base64 of the key's MD5, which S3 uses to catch a mangled header.
    checksum: String,
    /// Base64 of the key's SHA-256, which Google and Azure use for the same.
    digest: String,
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

    /// Encrypt under the account encryption scope `scope`, which is Azure's.
    #[must_use]
    pub fn scope(scope: impl Into<String>) -> Self {
        Self::Scope(scope.into())
    }

    /// Whether anything is said at all, which is what a default does not.
    pub const fn is_default(&self) -> bool {
        matches!(self, Self::Default)
    }

    /// Refuse a choice `provider` does not have, once, when a client is built.
    ///
    /// A store that simply *is* encrypted - Google and Azure both are, with
    /// their own keys, whatever a request says - takes [`Self::Managed`] as
    /// saying nothing rather than as an error: it is already true.
    ///
    /// # Errors
    ///
    /// Returns a refusal naming the store and the shape it does not have.
    pub(super) fn validate(&self, provider: Provider) -> Result<()> {
        let refused = match (self, provider) {
            (Self::Kms(_), Provider::Azure) => Some(
                "a managed encryption key on Azure is configured as an account \
                 encryption scope; name one with Encryption::scope",
            ),
            (Self::Scope(_), Provider::Aws | Provider::Google) => Some(
                "an encryption scope is Azure's shape for a managed key; on this \
                 store name the key itself with Encryption::kms",
            ),
            (Self::Kms(key), Provider::Google) if key.dual_layer() => Some(
                "dual-layer encryption is Amazon S3's; Google Cloud Storage \
                 encrypts a CMEK object once",
            ),
            _ => None,
        };
        match refused {
            Some(reason) => Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("{}: {reason}", provider.described()),
            ))),
            None => Ok(()),
        }
    }

    /// The encryption the `sse.*` properties describe.
    ///
    /// `AES256` is the ambiguous one: it is what `SSE-S3` is called, and it is
    /// also the algorithm of a customer key. A key alongside it is what tells
    /// them apart, which is the reading every store's tools take.
    ///
    /// # Errors
    ///
    /// Returns a refusal when a key is unusable, or when the named type is not
    /// one any of the three stores has.
    pub(super) fn from_parts(
        named: Option<&str>,
        key: Option<&str>,
        checksum: Option<&str>,
        kms_key_id: Option<&str>,
        context: Option<&str>,
        bucket_key: Option<bool>,
    ) -> Result<Option<Self>> {
        let customer = |key: &str| -> Result<Self> {
            let held = match checksum {
                Some(checksum) => CustomerKey::from_base64_checked(key, checksum)?,
                None => CustomerKey::from_base64(key)?,
            };
            Ok(Self::Customer(held))
        };
        let kms = || {
            let key_id = kms_key_id.or(key);
            let mut held = key_id.map_or_else(KmsKey::default, KmsKey::new);
            if let Some(context) = context {
                held = held.with_context(context);
            }
            if let Some(enabled) = bucket_key {
                held = held.with_bucket_key(enabled);
            }
            held
        };
        match named.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
            None | Some("none" | "disabled" | "false") => {
                // A key with no type is still a key: `sse.key` alone means the
                // customer key, because nothing else takes one.
                match (key, kms_key_id) {
                    (Some(key), None) => match customer(key) {
                        Ok(encryption) => Ok(Some(encryption)),
                        // A key with no type could have been either; say so
                        // rather than only that it is not a customer key.
                        Err(error) if checksum.is_none() => Err(Error::Io(std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            format!(
                                "{error}; name an sse type of aws:kms if the key is a managed key"
                            ),
                        ))),
                        Err(error) => Err(error),
                    },
                    (_, Some(_)) => Ok(Some(Self::Kms(kms()))),
                    (None, None) => Ok(None),
                }
            }
            Some("aes256" | "sse-s3" | "sse_s3" | "s3" | "managed") => match key {
                Some(key) => customer(key).map(Some),
                None => Ok(Some(Self::Managed)),
            },
            Some("aws:kms" | "sse-kms" | "sse_kms" | "kms" | "cmek") => Ok(Some(Self::Kms(kms()))),
            Some("aws:kms:dsse" | "dsse-kms" | "dsse") => {
                Ok(Some(Self::Kms(kms().with_dual_layer(true))))
            }
            Some("sse-c" | "sse_c" | "custom" | "customer" | "cpk") => match key {
                Some(key) => customer(key).map(Some),
                None => Err(Error::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "expected a customer key alongside an sse type of sse-c",
                ))),
            },
            Some("scope") => match key {
                Some(scope) => Ok(Some(Self::Scope(scope.to_owned()))),
                None => Err(Error::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "expected an encryption scope name alongside an sse type of scope",
                ))),
            },
            Some(other) => Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "expected an sse type of none, AES256, aws:kms, aws:kms:dsse, sse-c, \
                     or scope, got {other}"
                ),
            ))),
        }
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
    pub fn write_headers(&self, provider: Provider) -> Vec<(&'static str, String)> {
        match (self, provider) {
            (Self::Managed, Provider::Aws) => {
                vec![("x-amz-server-side-encryption", "AES256".to_owned())]
            }
            // Google and Azure encrypt every object with their own keys
            // already, so asking for it says nothing new.
            (Self::Managed, _) => Vec::new(),
            (Self::Kms(key), _) => key.headers(provider),
            (Self::Customer(key), _) => key.headers(provider),
            (Self::Scope(scope), Provider::Azure) => {
                vec![("x-ms-encryption-scope", scope.clone())]
            }
            (Self::Default | Self::Scope(_), _) => Vec::new(),
        }
    }

    /// The query a request deciding how an object is stored carries.
    ///
    /// Google's JSON API takes the managed key as a parameter rather than as a
    /// header, which is the one place this is not a header at all.
    pub(super) fn write_query(&self, provider: Provider) -> Vec<(String, String)> {
        match (self, provider) {
            (Self::Kms(key), Provider::Google) => key
                .key_id()
                .map(|key_id| vec![("kmsKeyName".to_owned(), key_id.to_owned())])
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    /// The headers a request touching an object's *bytes* carries.
    ///
    /// Only a customer key needs any: the store remembers which of its own
    /// keys it used, and a key it never kept has to be presented again - on
    /// reads as much as on writes, which is the whole cost of `SSE-C`.
    pub fn read_headers(&self, provider: Provider) -> Vec<(&'static str, String)> {
        match self {
            Self::Customer(key) => key.headers(provider),
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
    fn headers(&self, provider: Provider) -> Vec<(&'static str, String)> {
        if matches!(provider, Provider::Google) {
            // The JSON API takes the key as a query parameter; the XML API
            // takes this header, and both name the same key.
            return self
                .key_id
                .clone()
                .map(|key_id| vec![("x-goog-encryption-kms-key-name", key_id)])
                .unwrap_or_default();
        }
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
            digest: engine.encode(Sha256::digest(key)),
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

    /// Base64 of the key's MD5, as the S3 header spells it.
    pub fn checksum(&self) -> &str {
        &self.checksum
    }

    /// Base64 of the key's SHA-256, as the Google and Azure headers spell it.
    pub fn digest(&self) -> &str {
        &self.digest
    }

    /// What every request touching this object's bytes carries.
    ///
    /// The same key, three spellings, and two different digests of it: S3 asks
    /// for the MD5 and the other two for the SHA-256.
    fn headers(&self, provider: Provider) -> Vec<(&'static str, String)> {
        match provider {
            Provider::Aws => vec![
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
            ],
            Provider::Google => vec![
                ("x-goog-encryption-algorithm", "AES256".to_owned()),
                ("x-goog-encryption-key", self.encoded.clone()),
                ("x-goog-encryption-key-sha256", self.digest.clone()),
            ],
            Provider::Azure => vec![
                ("x-ms-encryption-algorithm", "AES256".to_owned()),
                ("x-ms-encryption-key", self.encoded.clone()),
                ("x-ms-encryption-key-sha256", self.digest.clone()),
            ],
        }
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
            Self::Scope(scope) => formatter.debug_tuple("Scope").field(scope).finish(),
        }
    }
}
