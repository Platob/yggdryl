//! The digest algorithms Yggdryl computes, and how to compute them.
//!
//! One [`DigestAlgorithm`] vocabulary names every algorithm the way [`Codec`]
//! names every content coding, and [`crate::xxhash`] owns the four
//! implementations behind it. A [`Digest`] is the answer: an algorithm plus a
//! payload, canonical big-endian bytes, and one lowercase hex spelling that
//! parses back to the same value.
//!
//! [`Codec`]: crate::Codec
//!
//! ```
//! use yggdryl::{Digest, DigestAlgorithm};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let digest = DigestAlgorithm::Xxh3.digest(b"abc");
//! assert_eq!(digest.as_u64(), Some(0x78af_5f94_892f_3950));
//! assert_eq!(digest.to_string(), "xxh3-64:78af5f94892f3950");
//! assert_eq!(Digest::from_str(&digest.to_string())?, digest);
//! # Ok(())
//! # }
//! ```
//!
//! xxHash is not a cryptographic hash. A [`Digest`] detects accidental change -
//! a truncated upload, a stale cache, a duplicated row - and never withstands
//! an adversary who is allowed to choose the input.

use std::fmt;
use std::io::Read;
use std::ops::Deref;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smol_str::format_smolstr;

use crate::xxhash::{Xxh3, Xxh32, Xxh64, Xxh128};
use crate::{Error, Result, Scalar};

/// One xxHash algorithm, and the only place a name selects an implementation.
///
/// The canonical tokens are `xxh32`, `xxh64`, `xxh3-64`, and `xxh3-128`.
/// The reference-library entry-point spellings `xxh3` and `xxh128` are also
/// accepted on input and render back to their canonical protocol names.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum DigestAlgorithm {
    /// XXH32, the original 32-bit algorithm.
    Xxh32,
    /// XXH64, the original 64-bit algorithm.
    Xxh64,
    /// XXH3 answering 64 bits.
    ///
    /// This is the default because it is the algorithm every `stable_hash` in
    /// the project answers, so a caller who expresses no preference gets the
    /// value the rest of the tree already agrees on.
    #[default]
    Xxh3,
    /// XXH3 answering 128 bits.
    Xxh128,
}

impl DigestAlgorithm {
    /// Every algorithm in canonical order.
    pub const ALL: [Self; 4] = [Self::Xxh32, Self::Xxh64, Self::Xxh3, Self::Xxh128];

    /// Parse a canonical algorithm token.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] naming the accepted vocabulary and the input.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
        <Self as FromStr>::from_str(value)
    }

    /// Return the canonical lowercase token without allocating.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Xxh32 => "xxh32",
            Self::Xxh64 => "xxh64",
            Self::Xxh3 => "xxh3-64",
            Self::Xxh128 => "xxh3-128",
        }
    }

    /// Return the digest width in bytes.
    ///
    /// ```
    /// use yggdryl::DigestAlgorithm;
    ///
    /// assert_eq!(DigestAlgorithm::Xxh32.width(), 4);
    /// assert_eq!(DigestAlgorithm::Xxh128.width(), 16);
    /// ```
    pub const fn width(self) -> usize {
        match self {
            Self::Xxh32 => 4,
            Self::Xxh64 | Self::Xxh3 => 8,
            Self::Xxh128 => 16,
        }
    }

    /// Return the digest width in bits.
    pub const fn bits(self) -> u32 {
        self.width() as u32 * 8
    }

    /// Return whether this algorithm accepts a custom secret.
    ///
    /// Only the XXH3 pair does. XXH32 and XXH64 take a seed and nothing else.
    pub const fn is_secretable(self) -> bool {
        matches!(self, Self::Xxh3 | Self::Xxh128)
    }

    /// Return whether this algorithm accepts a seed.
    ///
    /// Every xxHash algorithm does. The question exists beside
    /// [`Self::is_secretable`] so a caller asks both the same way rather than
    /// remembering which of the two is the narrow one.
    pub const fn is_seedable(self) -> bool {
        true
    }

    /// Build a streaming state for this algorithm.
    ///
    /// ```
    /// use yggdryl::DigestAlgorithm;
    ///
    /// let mut digester = DigestAlgorithm::Xxh64.digester();
    /// digester.write_bytes(b"ab");
    /// digester.write_bytes(b"c");
    /// assert_eq!(digester.as_digest(), DigestAlgorithm::Xxh64.digest(b"abc"));
    /// ```
    pub fn digester(self) -> Digester {
        Digester(match self {
            Self::Xxh32 => DigesterKind::Xxh32(Xxh32::new()),
            Self::Xxh64 => DigesterKind::Xxh64(Xxh64::new()),
            Self::Xxh3 => DigesterKind::Xxh3(Xxh3::new()),
            Self::Xxh128 => DigesterKind::Xxh128(Xxh128::new()),
        })
    }

    /// Build a streaming state seeded with `seed`.
    ///
    /// The seed is a `u64` because the dispatcher answers one shape for four
    /// algorithms; XXH32, whose seed is 32 bits wide, uses its low half. A
    /// caller who knows the algorithm at compile time uses that state's own
    /// `with_seed` and passes the exact width.
    ///
    /// ```
    /// use yggdryl::{DigestAlgorithm, xxhash};
    ///
    /// let mut digester = DigestAlgorithm::Xxh32.digester_with_seed(0x1_0000_002a);
    /// digester.write_bytes(b"abc");
    /// assert_eq!(digester.as_digest().as_u32(), Some(xxhash::xxh32_with_seed(b"abc", 42)));
    /// ```
    pub fn digester_with_seed(self, seed: u64) -> Digester {
        Digester(match self {
            Self::Xxh32 => DigesterKind::Xxh32(Xxh32::with_seed(crate::xxhash::low_32(seed))),
            Self::Xxh64 => DigesterKind::Xxh64(Xxh64::with_seed(seed)),
            Self::Xxh3 => DigesterKind::Xxh3(Xxh3::with_seed(seed)),
            Self::Xxh128 => DigesterKind::Xxh128(Xxh128::with_seed(seed)),
        })
    }

    /// Digest a complete buffer.
    ///
    /// ```
    /// use yggdryl::DigestAlgorithm;
    ///
    /// assert_eq!(
    ///     DigestAlgorithm::Xxh32.digest(b"").as_u32(),
    ///     Some(0x02cc_5d05),
    /// );
    /// ```
    pub fn digest(self, input: &[u8]) -> Digest {
        match self {
            Self::Xxh32 => Digest::new(self, u128::from(crate::xxhash::xxh32(input))),
            Self::Xxh64 => Digest::new(self, u128::from(crate::xxhash::xxh64(input))),
            Self::Xxh3 => Digest::new(self, u128::from(crate::xxhash::xxh3(input))),
            Self::Xxh128 => Digest::new(self, crate::xxhash::xxh128(input)),
        }
    }
}

impl FromStr for DigestAlgorithm {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let normalized = value.trim();
        if normalized.eq_ignore_ascii_case("xxh3") {
            return Ok(Self::Xxh3);
        }
        if normalized.eq_ignore_ascii_case("xxh128") {
            return Ok(Self::Xxh128);
        }
        Self::ALL
            .into_iter()
            .find(|algorithm| normalized.eq_ignore_ascii_case(algorithm.as_str()))
            .ok_or_else(|| Error::Parse {
                target: "digest algorithm",
                position: 0,
                reason: format_smolstr!(
                    "expected one of {}, got {value:?}",
                    Self::ALL
                        .iter()
                        .map(|algorithm| algorithm.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            })
    }
}

impl fmt::Display for DigestAlgorithm {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for DigestAlgorithm {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for DigestAlgorithm {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let value = <&str>::deserialize(deserializer)?;
        Self::from_str(value).map_err(serde::de::Error::custom)
    }
}

/// One digest: the algorithm that produced it and the value it produced.
///
/// Two digests of different algorithms are never equal, whatever their
/// payloads, because a bare number is not an identity: `xxh64` and `xxh3-64`
/// are both 64 bits wide and answer different values for the same input.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Digest {
    algorithm: DigestAlgorithm,
    payload: u128,
}

impl Digest {
    /// Build a digest from an algorithm and its native value.
    ///
    /// The payload is masked to the algorithm's width, so a value wider than
    /// the algorithm answers cannot be stored and later read back.
    pub const fn new(algorithm: DigestAlgorithm, payload: u128) -> Self {
        let payload = match algorithm.width() {
            4 => payload & 0xffff_ffff,
            8 => payload & 0xffff_ffff_ffff_ffff,
            _ => payload,
        };
        Self { algorithm, payload }
    }

    /// Return the algorithm that produced this digest.
    pub const fn algorithm(self) -> DigestAlgorithm {
        self.algorithm
    }

    /// Return the native 32-bit value, or `None` when the width differs.
    pub const fn as_u32(self) -> Option<u32> {
        match self.algorithm {
            DigestAlgorithm::Xxh32 => Some(self.payload as u32),
            _ => None,
        }
    }

    /// Return the native 64-bit value, or `None` when the width differs.
    pub const fn as_u64(self) -> Option<u64> {
        match self.algorithm {
            DigestAlgorithm::Xxh64 | DigestAlgorithm::Xxh3 => Some(self.payload as u64),
            _ => None,
        }
    }

    /// Return the native 128-bit value, or `None` when the width differs.
    pub const fn as_u128(self) -> Option<u128> {
        match self.algorithm {
            DigestAlgorithm::Xxh128 => Some(self.payload),
            _ => None,
        }
    }

    /// Return the canonical big-endian bytes at the algorithm's exact width.
    ///
    /// This is the representation the xxHash reference calls
    /// `XXH*_canonicalFromHash`: the value as it is written to a wire or a
    /// file, most significant byte first, so two machines of different
    /// endianness store the same digest as the same bytes.
    ///
    /// ```
    /// use yggdryl::DigestAlgorithm;
    ///
    /// let digest = DigestAlgorithm::Xxh32.digest(b"");
    /// assert_eq!(&*digest.into_bytes(), &[0x02, 0xcc, 0x5d, 0x05]);
    /// ```
    pub const fn into_bytes(self) -> DigestBytes {
        DigestBytes {
            bytes: self.payload.to_be_bytes(),
            length: self.algorithm.width() as u8,
        }
    }

    /// Rebuild a digest from its canonical big-endian bytes.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] when `bytes` is not exactly the algorithm's
    /// width.
    pub fn from_bytes(algorithm: DigestAlgorithm, bytes: &[u8]) -> Result<Self> {
        let width = algorithm.width();
        if bytes.len() != width {
            return Err(Error::Parse {
                target: "digest",
                position: 0,
                reason: format_smolstr!("expected {width} {algorithm} bytes, got {}", bytes.len()),
            });
        }
        let mut payload = [0_u8; 16];
        payload[16 - width..].copy_from_slice(bytes);
        Ok(Self {
            algorithm,
            payload: u128::from_be_bytes(payload),
        })
    }

    /// Parse the canonical `<algorithm>:<hex>` spelling.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] naming the failure.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
        <Self as FromStr>::from_str(value)
    }

    /// Return the deterministic 64-bit hash used by every binding.
    pub fn stable_hash(&self) -> u64 {
        crate::stable_hash_of(self)
    }
}

/// The canonical big-endian bytes of one [`Digest`].
///
/// The value is inline, so reading a digest's bytes never allocates. It
/// dereferences to the exact-width slice and compares as that slice.
#[derive(Clone, Copy)]
pub struct DigestBytes {
    bytes: [u8; 16],
    length: u8,
}

impl Deref for DigestBytes {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.bytes[16 - self.length as usize..]
    }
}

impl AsRef<[u8]> for DigestBytes {
    fn as_ref(&self) -> &[u8] {
        self
    }
}

impl fmt::Debug for DigestBytes {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, formatter)
    }
}

impl PartialEq for DigestBytes {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

impl Eq for DigestBytes {}

impl std::hash::Hash for DigestBytes {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        (**self).hash(state);
    }
}

impl fmt::Display for Digest {
    /// Render `<algorithm>:<lowercase hex zero-padded to the width>`.
    ///
    /// The algorithm is part of the rendering because it is part of the value:
    /// without it `xxh64` and `xxh3-64` would share one spelling and
    /// [`FromStr`] could not be the exact inverse this contract requires.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}:{:0width$x}",
            self.algorithm,
            self.payload,
            width = self.algorithm.width() * 2
        )
    }
}

impl FromStr for Digest {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let normalized = value.trim();
        let (name, hex) = normalized.split_once(':').ok_or_else(|| Error::Parse {
            target: "digest",
            position: 0,
            reason: format_smolstr!(
                "expected <algorithm>:<hex>, got {}",
                crate::text::elide_to(normalized, crate::text::ERROR_TEXT_LIMIT)
            ),
        })?;
        let algorithm = DigestAlgorithm::from_str(name)?;
        let expected = algorithm.width() * 2;
        if hex.len() != expected {
            return Err(Error::Parse {
                target: "digest",
                position: name.len() + 1,
                reason: format_smolstr!(
                    "expected {expected} {algorithm} hex digits, got {}",
                    hex.len()
                ),
            });
        }
        let payload = u128::from_str_radix(hex, 16).map_err(|error| Error::Parse {
            target: "digest",
            position: name.len() + 1,
            reason: format_smolstr!("expected hexadecimal digits, got {error}"),
        })?;
        Ok(Self { algorithm, payload })
    }
}

impl Serialize for Digest {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Digest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let value = <&str>::deserialize(deserializer)?;
        Self::from_str(value).map_err(serde::de::Error::custom)
    }
}

/// A streaming digest state selected at runtime.
///
/// This is to [`DigestAlgorithm`] what [`crate::Encoder`] is to
/// [`crate::Codec`]: the one place a runtime algorithm becomes a concrete
/// implementation. A caller who knows the algorithm at compile time uses the
/// concrete state in [`crate::xxhash`] instead and pays no dispatch.
#[derive(Clone, Debug)]
pub struct Digester(DigesterKind);

#[derive(Clone, Debug)]
enum DigesterKind {
    Xxh32(Xxh32),
    Xxh64(Xxh64),
    Xxh3(Xxh3),
    Xxh128(Xxh128),
}

impl Digester {
    /// Return the algorithm this state computes.
    pub const fn algorithm(&self) -> DigestAlgorithm {
        match self.0 {
            DigesterKind::Xxh32(_) => DigestAlgorithm::Xxh32,
            DigesterKind::Xxh64(_) => DigestAlgorithm::Xxh64,
            DigesterKind::Xxh3(_) => DigestAlgorithm::Xxh3,
            DigesterKind::Xxh128(_) => DigestAlgorithm::Xxh128,
        }
    }

    /// Feed raw bytes.
    pub fn write_bytes(&mut self, bytes: &[u8]) {
        match &mut self.0 {
            DigesterKind::Xxh32(state) => state.write_bytes(bytes),
            DigesterKind::Xxh64(state) => state.write_bytes(bytes),
            DigesterKind::Xxh3(state) => state.write_bytes(bytes),
            DigesterKind::Xxh128(state) => state.write_bytes(bytes),
        }
    }

    /// Feed one value's canonical byte representation.
    ///
    /// See [`Scalar::write_bytes`] for the encoding and what it guarantees.
    pub fn write_scalar(&mut self, value: &Scalar) {
        match &mut self.0 {
            DigesterKind::Xxh32(state) => state.write_scalar(value),
            DigesterKind::Xxh64(state) => state.write_scalar(value),
            DigesterKind::Xxh3(state) => state.write_scalar(value),
            DigesterKind::Xxh128(state) => state.write_scalar(value),
        }
    }

    /// Feed a reader to exhaustion, returning the bytes consumed.
    ///
    /// # Errors
    ///
    /// Returns the reader's failure. Bytes already fed stay fed.
    pub fn write_reader(&mut self, source: &mut impl Read) -> Result<u64> {
        match &mut self.0 {
            DigesterKind::Xxh32(state) => state.write_reader(source),
            DigesterKind::Xxh64(state) => state.write_reader(source),
            DigesterKind::Xxh3(state) => state.write_reader(source),
            DigesterKind::Xxh128(state) => state.write_reader(source),
        }
    }

    /// Answer the digest of everything fed so far, without consuming the state.
    pub fn as_digest(&self) -> Digest {
        match &self.0 {
            DigesterKind::Xxh32(state) => state.as_digest(),
            DigesterKind::Xxh64(state) => state.as_digest(),
            DigesterKind::Xxh3(state) => state.as_digest(),
            DigesterKind::Xxh128(state) => state.as_digest(),
        }
    }

    /// Reset to the constructed seed and secret.
    pub fn clear(&mut self) {
        match &mut self.0 {
            DigesterKind::Xxh32(state) => state.clear(),
            DigesterKind::Xxh64(state) => state.clear(),
            DigesterKind::Xxh3(state) => state.clear(),
            DigesterKind::Xxh128(state) => state.clear(),
        }
    }

    /// Fill the digest holders in one Arrow batch under `root`.
    ///
    /// This state is a configuration prototype: its algorithm, seed, and
    /// secret are used, but bytes already written to it are ignored and the
    /// state remains unchanged. With `force` false, populated holder cells are
    /// preserved and only canonical defaults are computed; with it true,
    /// every visible holder is recomputed.
    ///
    /// # Errors
    ///
    /// Returns an error when `root` is not a usable Struct schema, a holder
    /// has the wrong width, a digest path cannot be resolved, or the batch
    /// cannot be cast to `root`.
    #[cfg(feature = "arrow")]
    pub fn fill_arrow_batch(
        &self,
        root: &crate::Field,
        batch: arrow_array::RecordBatch,
        force: bool,
    ) -> crate::arrow::Result<arrow_array::RecordBatch> {
        crate::xxhash::arrow::fill_arrow_batch_with(self, root, batch, force)
    }
}

impl std::hash::Hasher for Digester {
    fn finish(&self) -> u64 {
        let digest = self.as_digest();
        digest
            .as_u64()
            .or_else(|| digest.as_u32().map(u64::from))
            .unwrap_or_else(|| crate::xxhash::low_64(digest.payload))
    }

    fn write(&mut self, bytes: &[u8]) {
        self.write_bytes(bytes);
    }
}

#[cfg(test)]
mod tests;
