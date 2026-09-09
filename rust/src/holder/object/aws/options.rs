//! The knobs that are Amazon S3's own.
//!
//! Everything here is a fact about S3 or about the AWS tools' configuration,
//! and so has no counterpart on the other two stores: the shared configuration
//! profile, whether a payload is hashed for the signature, the role a request
//! is signed as, the storage class an object lands in, and who pays for a
//! request. A knob all three stores have lives on
//! [`ObjectOptions`](super::super::ObjectOptions) instead.

use super::sts::AssumedRole;

/// How this backend reaches Amazon S3 in particular.
///
/// ```
/// use yggdryl::holder::object::{AwsOptions, ObjectOptions};
///
/// let options = ObjectOptions::default().with_aws(
///     AwsOptions::default()
///         .with_profile("trading")
///         .with_storage_class("INTELLIGENT_TIERING")
///         .with_requester_pays(true),
/// );
/// assert_eq!(options.aws().profile(), Some("trading"));
/// assert_eq!(options.aws().storage_class(), Some("INTELLIGENT_TIERING"));
/// ```
#[derive(Clone, Debug, Default)]
pub struct AwsOptions {
    profile: Option<String>,
    payload_signing: Option<bool>,
    assumed_role: Option<AssumedRole>,
    storage_class: Option<String>,
    requester_pays: bool,
    checksum: Option<Checksum>,
}

/// The checksum algorithm S3 computes and stores beside an object.
///
/// S3 verifies what it received against the value the request states, and keeps
/// it so a later read can be checked without transferring the object again.
/// `CRC32` is what the AWS tools default to; `SHA256` is the strong one.
///
/// The two S3 also accepts - `CRC32C` and `CRC64NVME` - are refused by name
/// rather than silently ignored: each is a second polynomial this crate would
/// carry for a value nothing else here needs.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Checksum {
    /// CRC-32, the AWS tools' default.
    Crc32,
    /// SHA-256.
    Sha256,
}

impl Checksum {
    /// The header this algorithm's value travels in.
    pub(crate) const fn header(&self) -> &'static str {
        match self {
            Self::Crc32 => "x-amz-checksum-crc32",
            Self::Sha256 => "x-amz-checksum-sha256",
        }
    }

    /// The name `x-amz-sdk-checksum-algorithm` states.
    pub(crate) const fn as_str(&self) -> &'static str {
        match self {
            Self::Crc32 => "CRC32",
            Self::Sha256 => "SHA256",
        }
    }

    /// This checksum over `bytes`, base64 as the header spells it.
    pub(crate) fn of(&self, bytes: &[u8]) -> String {
        use base64::Engine as _;
        let engine = base64::engine::general_purpose::STANDARD;
        match self {
            Self::Crc32 => engine.encode(crc32(bytes).to_be_bytes()),
            Self::Sha256 => {
                use sha2::Digest as _;
                engine.encode(sha2::Sha256::digest(bytes))
            }
        }
    }
}

/// CRC-32 over `bytes`, the reflected IEEE 802.3 polynomial every tool means
/// by `CRC32`.
///
/// Computed a bit at a time rather than from a table: a checksum is taken once
/// per write of a whole value, so the table would be state held for nothing.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let carry = crc & 1;
            crc >>= 1;
            if carry != 0 {
                crc ^= 0xEDB8_8320;
            }
        }
    }
    !crc
}

impl std::str::FromStr for Checksum {
    type Err = crate::Error;

    fn from_str(value: &str) -> crate::Result<Self> {
        match value
            .trim()
            .to_ascii_lowercase()
            .replace(['-', '_'], "")
            .as_str()
        {
            "crc32" => Ok(Self::Crc32),
            "sha256" => Ok(Self::Sha256),
            _ => Err(crate::Error::Parse {
                target: "s3 checksum algorithm",
                position: 0,
                reason: format!("expected CRC32 or SHA256, got {value:?}").into(),
            }),
        }
    }
}

impl AwsOptions {
    /// Read `profile` from the shared AWS files instead of `AWS_PROFILE`.
    #[must_use]
    pub fn with_profile(mut self, profile: impl Into<String>) -> Self {
        self.profile = Some(profile.into());
        self
    }

    /// Sign the body of every write, or send it as `UNSIGNED-PAYLOAD`.
    ///
    /// Signing hashes the whole value with SHA-256 so the store can verify
    /// what it received. That is worth paying for over a plain-HTTP endpoint,
    /// where nothing else protects the body, and it is what an unset value
    /// selects there. Over HTTPS the transport already guarantees integrity,
    /// so an unset value skips the hash - which on a large upload is the
    /// difference between hashing the value and not.
    #[must_use]
    pub const fn with_payload_signing(mut self, signing: bool) -> Self {
        self.payload_signing = Some(signing);
        self
    }

    /// Sign requests as `role` rather than as the keys that were found.
    ///
    /// The credential chain still answers, and what it answers is what signs
    /// the *exchange*: one STS request trades those keys for the role's, and
    /// the session it hands back is what reaches the container. It expires, so
    /// it is traded again shortly before it does rather than per request.
    #[must_use]
    pub fn with_assumed_role(mut self, role: AssumedRole) -> Self {
        self.assumed_role = Some(role);
        self
    }

    /// Store what is written in `class` rather than in the bucket's default.
    ///
    /// `STANDARD`, `STANDARD_IA`, `INTELLIGENT_TIERING`, `GLACIER_IR`, and the
    /// rest, sent verbatim as `x-amz-storage-class`: the list is S3's and grows
    /// without this crate, so a name is passed through rather than enumerated.
    #[must_use]
    pub fn with_storage_class(mut self, class: impl Into<String>) -> Self {
        self.storage_class = Some(class.into());
        self
    }

    /// Accept the charges for a request against a requester-pays bucket.
    ///
    /// A bucket configured that way refuses a request that does not say this,
    /// so it is a declaration rather than an optimization.
    #[must_use]
    pub const fn with_requester_pays(mut self, pays: bool) -> Self {
        self.requester_pays = pays;
        self
    }

    /// Compute and store `checksum` beside every object written.
    #[must_use]
    pub const fn with_checksum(mut self, checksum: Checksum) -> Self {
        self.checksum = Some(checksum);
        self
    }

    /// The explicit profile name.
    pub fn profile(&self) -> Option<&str> {
        self.profile.as_deref()
    }

    /// The explicit payload-signing choice.
    pub const fn payload_signing(&self) -> Option<bool> {
        self.payload_signing
    }

    /// The role requests are signed as, when one was named.
    pub const fn assumed_role(&self) -> Option<&AssumedRole> {
        self.assumed_role.as_ref()
    }

    /// The storage class writes ask for, when one was named.
    pub fn storage_class(&self) -> Option<&str> {
        self.storage_class.as_deref()
    }

    /// Whether requests accept requester-pays charges.
    pub const fn requester_pays(&self) -> bool {
        self.requester_pays
    }

    /// The checksum algorithm writes carry, when one was named.
    pub const fn checksum(&self) -> Option<Checksum> {
        self.checksum
    }

    /// Fill from `ambient` every knob this one does not set for itself.
    pub(crate) fn under(mut self, ambient: &Self) -> Self {
        if self.profile.is_none() {
            self.profile = ambient.profile.clone();
        }
        if self.payload_signing.is_none() {
            self.payload_signing = ambient.payload_signing;
        }
        if self.assumed_role.is_none() {
            self.assumed_role = ambient.assumed_role.clone();
        }
        if self.storage_class.is_none() {
            self.storage_class = ambient.storage_class.clone();
        }
        if !self.requester_pays {
            self.requester_pays = ambient.requester_pays;
        }
        if self.checksum.is_none() {
            self.checksum = ambient.checksum;
        }
        self
    }
}
