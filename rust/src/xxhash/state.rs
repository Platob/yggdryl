//! The four resumable streaming states.
//!
//! Each state owns one algorithm's accumulator, the seed it was constructed
//! with, and - for the XXH3 pair - the custom secret. Answering a digest reads
//! the accumulator without consuming it, so a state can be fed again and asked
//! again: that is what lets one pass over a stream answer a running digest at
//! every commit boundary rather than only at the end.

use std::fmt;
use std::hash::{BuildHasher, Hasher};
use std::io::Read;
use std::sync::Arc;

use crate::io::DEFAULT_STREAM_BATCH_SIZE;
use crate::{Digest, DigestAlgorithm, Result};

use super::secret;

/// Feed `source` to exhaustion through one reused bounded buffer.
///
/// Memory is flat in the reader's length: the window is the same batch size
/// every byte stream in the project uses, and nothing accumulates behind it.
fn feed(source: &mut impl Read, mut sink: impl FnMut(&[u8])) -> Result<u64> {
    let mut window = vec![0_u8; DEFAULT_STREAM_BATCH_SIZE];
    let mut consumed = 0_u64;
    loop {
        match source.read(&mut window) {
            Ok(0) => return Ok(consumed),
            Ok(read) => {
                sink(&window[..read]);
                consumed += read as u64;
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error.into()),
        }
    }
}

/// A resumable XXH32 state.
///
/// ```
/// use yggdryl::xxhash::{Xxh32, xxh32};
///
/// let mut state = Xxh32::new();
/// state.write_bytes(b"ab");
/// state.write_bytes(b"c");
/// assert_eq!(state.as_u32(), xxh32(b"abc"));
/// // Answering does not consume the state.
/// assert_eq!(state.as_u32(), xxh32(b"abc"));
/// ```
#[derive(Clone)]
pub struct Xxh32 {
    seed: u32,
    hasher: twox_hash::xxhash32::Hasher,
}

impl Xxh32 {
    /// Start an unseeded state.
    pub fn new() -> Self {
        Self::with_seed(0)
    }

    /// Start a state seeded with `seed`.
    pub fn with_seed(seed: u32) -> Self {
        Self {
            seed,
            hasher: twox_hash::xxhash32::Hasher::with_seed(seed),
        }
    }

    /// Return the seed this state was constructed with.
    pub const fn seed(&self) -> u32 {
        self.seed
    }

    /// Feed raw bytes.
    pub fn write_bytes(&mut self, bytes: &[u8]) {
        self.hasher.write(bytes);
    }

    /// Feed a reader to exhaustion, returning the bytes consumed.
    ///
    /// # Errors
    ///
    /// Returns the reader's failure. Bytes already fed stay fed, so a failed
    /// call leaves a partial state rather than an unchanged one.
    pub fn write_reader(&mut self, source: &mut impl Read) -> Result<u64> {
        let hasher = &mut self.hasher;
        feed(source, |chunk| hasher.write(chunk))
    }

    /// Answer the 32-bit value of everything fed so far.
    pub fn as_u32(&self) -> u32 {
        self.hasher.finish_32()
    }

    /// Answer the digest of everything fed so far.
    pub fn as_digest(&self) -> Digest {
        Digest::new(DigestAlgorithm::Xxh32, u128::from(self.as_u32()))
    }

    /// Reset to the constructed seed, not to [`Self::new`].
    pub fn clear(&mut self) {
        self.hasher = twox_hash::xxhash32::Hasher::with_seed(self.seed);
    }
}

impl Default for Xxh32 {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Xxh32 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Xxh32")
            .field("seed", &self.seed)
            .finish()
    }
}

impl Hasher for Xxh32 {
    /// Answer the 32-bit value widened into the trait's 64-bit return.
    ///
    /// [`Self::as_u32`] is the native width; nothing is lost, the upper 32
    /// bits are simply zero.
    fn finish(&self) -> u64 {
        u64::from(self.as_u32())
    }

    fn write(&mut self, bytes: &[u8]) {
        self.write_bytes(bytes);
    }
}

impl BuildHasher for Xxh32 {
    type Hasher = Self;

    /// Build a fresh state carrying this state's seed.
    fn build_hasher(&self) -> Self {
        Self::with_seed(self.seed)
    }
}

/// A resumable XXH64 state.
///
/// ```
/// use yggdryl::xxhash::{Xxh64, xxh64, xxh64_with_seed};
///
/// let mut state = Xxh64::with_seed(7);
/// state.write_bytes(b"abc");
/// assert_eq!(state.as_u64(), xxh64_with_seed(b"abc", 7));
/// state.clear();
/// // `clear` returns to the constructed seed, not to an unseeded state.
/// assert_eq!(state.as_u64(), xxh64_with_seed(b"", 7));
/// assert_ne!(state.as_u64(), xxh64(b""));
/// ```
#[derive(Clone)]
pub struct Xxh64 {
    seed: u64,
    hasher: twox_hash::xxhash64::Hasher,
}

impl Xxh64 {
    /// Start an unseeded state.
    pub fn new() -> Self {
        Self::with_seed(0)
    }

    /// Start a state seeded with `seed`.
    pub fn with_seed(seed: u64) -> Self {
        Self {
            seed,
            hasher: twox_hash::xxhash64::Hasher::with_seed(seed),
        }
    }

    /// Return the seed this state was constructed with.
    pub const fn seed(&self) -> u64 {
        self.seed
    }

    /// Feed raw bytes.
    pub fn write_bytes(&mut self, bytes: &[u8]) {
        self.hasher.write(bytes);
    }

    /// Feed a reader to exhaustion, returning the bytes consumed.
    ///
    /// # Errors
    ///
    /// Returns the reader's failure. Bytes already fed stay fed.
    pub fn write_reader(&mut self, source: &mut impl Read) -> Result<u64> {
        let hasher = &mut self.hasher;
        feed(source, |chunk| hasher.write(chunk))
    }

    /// Answer the 64-bit value of everything fed so far.
    pub fn as_u64(&self) -> u64 {
        self.hasher.finish()
    }

    /// Answer the digest of everything fed so far.
    pub fn as_digest(&self) -> Digest {
        Digest::new(DigestAlgorithm::Xxh64, u128::from(self.as_u64()))
    }

    /// Reset to the constructed seed, not to [`Self::new`].
    pub fn clear(&mut self) {
        self.hasher = twox_hash::xxhash64::Hasher::with_seed(self.seed);
    }
}

impl Default for Xxh64 {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Xxh64 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Xxh64")
            .field("seed", &self.seed)
            .finish()
    }
}

impl Hasher for Xxh64 {
    fn finish(&self) -> u64 {
        self.as_u64()
    }

    fn write(&mut self, bytes: &[u8]) {
        self.write_bytes(bytes);
    }
}

impl BuildHasher for Xxh64 {
    type Hasher = Self;

    /// Build a fresh state carrying this state's seed.
    fn build_hasher(&self) -> Self {
        Self::with_seed(self.seed)
    }
}

/// A resumable XXH3 state answering 64 bits.
///
/// ```
/// use yggdryl::xxhash::{Xxh3_64, xxh3_64};
///
/// let mut state = Xxh3_64::new();
/// for chunk in [b"sym".as_slice(), b"bol"] {
///     state.write_bytes(chunk);
/// }
/// assert_eq!(state.as_u64(), xxh3_64(b"symbol"));
/// ```
#[derive(Clone)]
pub struct Xxh3_64 {
    seed: u64,
    secret: Option<Arc<[u8]>>,
    hasher: twox_hash::xxhash3_64::Hasher,
}

impl Xxh3_64 {
    /// Start a state with the default seed and secret.
    pub fn new() -> Self {
        Self {
            seed: 0,
            secret: None,
            hasher: twox_hash::xxhash3_64::Hasher::new(),
        }
    }

    /// Start a state seeded with `seed` and the secret derived from it.
    pub fn with_seed(seed: u64) -> Self {
        Self {
            seed,
            secret: None,
            hasher: twox_hash::xxhash3_64::Hasher::with_seed(seed),
        }
    }

    /// Start a state with a custom secret and the default seed.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::InvalidSecret`] when `secret` is shorter than
    /// [`super::SECRET_MINIMUM_LENGTH`].
    pub fn from_secret(secret: &[u8]) -> Result<Self> {
        Self::from_seed_and_secret(0, secret)
    }

    /// Start a state with a custom seed and secret.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::InvalidSecret`] when `secret` is shorter than
    /// [`super::SECRET_MINIMUM_LENGTH`].
    pub fn from_seed_and_secret(seed: u64, secret: &[u8]) -> Result<Self> {
        secret::validate(DigestAlgorithm::Xxh3_64, secret)?;
        let secret: Arc<[u8]> = Arc::from(secret);
        Ok(Self {
            seed,
            hasher: build_64(seed, &secret),
            secret: Some(secret),
        })
    }

    /// Return the seed this state was constructed with.
    pub const fn seed(&self) -> u64 {
        self.seed
    }

    /// Return the custom secret this state was constructed with, if any.
    pub fn secret(&self) -> Option<&[u8]> {
        self.secret.as_deref()
    }

    /// Feed raw bytes.
    pub fn write_bytes(&mut self, bytes: &[u8]) {
        self.hasher.write(bytes);
    }

    /// Feed a reader to exhaustion, returning the bytes consumed.
    ///
    /// # Errors
    ///
    /// Returns the reader's failure. Bytes already fed stay fed.
    pub fn write_reader(&mut self, source: &mut impl Read) -> Result<u64> {
        let hasher = &mut self.hasher;
        feed(source, |chunk| hasher.write(chunk))
    }

    /// Answer the 64-bit value of everything fed so far.
    pub fn as_u64(&self) -> u64 {
        self.hasher.finish()
    }

    /// Answer the digest of everything fed so far.
    pub fn as_digest(&self) -> Digest {
        Digest::new(DigestAlgorithm::Xxh3_64, u128::from(self.as_u64()))
    }

    /// Reset to the constructed seed and secret, not to [`Self::new`].
    pub fn clear(&mut self) {
        self.hasher = match &self.secret {
            Some(secret) => build_64(self.seed, secret),
            None if self.seed == 0 => twox_hash::xxhash3_64::Hasher::new(),
            None => twox_hash::xxhash3_64::Hasher::with_seed(self.seed),
        };
    }
}

/// Build an XXH3-64 accumulator over an already validated secret.
fn build_64(seed: u64, secret: &[u8]) -> twox_hash::xxhash3_64::Hasher {
    match twox_hash::xxhash3_64::Hasher::with_seed_and_secret(seed, secret) {
        Ok(hasher) => hasher,
        // The only failure is a short secret, and no secret reaches here
        // without passing `secret::validate` first.
        Err(_) => unreachable!("the secret was validated before it was stored"),
    }
}

impl Default for Xxh3_64 {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Xxh3_64 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Xxh3_64")
            .field("seed", &self.seed)
            .field("secret", &self.secret.as_ref().map(|secret| secret.len()))
            .finish()
    }
}

impl Hasher for Xxh3_64 {
    fn finish(&self) -> u64 {
        self.as_u64()
    }

    fn write(&mut self, bytes: &[u8]) {
        self.write_bytes(bytes);
    }
}

impl BuildHasher for Xxh3_64 {
    type Hasher = Self;

    /// Build a fresh state carrying this state's seed and secret.
    ///
    /// XXH3 keeps its secret on the heap, so each build allocates one secret
    /// buffer; that cost is the algorithm's, not this wrapper's.
    fn build_hasher(&self) -> Self {
        let mut state = self.clone();
        state.clear();
        state
    }
}

/// A resumable XXH3 state answering 128 bits.
///
/// ```
/// use yggdryl::xxhash::{Xxh3_128, xxh3_128, xxh3_64};
///
/// let mut state = Xxh3_128::new();
/// state.write_bytes(b"abc");
/// assert_eq!(state.as_u128(), xxh3_128(b"abc"));
/// // The low 64 bits are XXH3-64 of the same input.
/// assert_eq!(u64::try_from(state.as_u128() & u128::from(u64::MAX))?, xxh3_64(b"abc"));
/// # Ok::<(), std::num::TryFromIntError>(())
/// ```
#[derive(Clone)]
pub struct Xxh3_128 {
    seed: u64,
    secret: Option<Arc<[u8]>>,
    hasher: twox_hash::xxhash3_128::Hasher,
}

impl Xxh3_128 {
    /// Start a state with the default seed and secret.
    pub fn new() -> Self {
        Self {
            seed: 0,
            secret: None,
            hasher: twox_hash::xxhash3_128::Hasher::new(),
        }
    }

    /// Start a state seeded with `seed` and the secret derived from it.
    pub fn with_seed(seed: u64) -> Self {
        Self {
            seed,
            secret: None,
            hasher: twox_hash::xxhash3_128::Hasher::with_seed(seed),
        }
    }

    /// Start a state with a custom secret and the default seed.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::InvalidSecret`] when `secret` is shorter than
    /// [`super::SECRET_MINIMUM_LENGTH`].
    pub fn from_secret(secret: &[u8]) -> Result<Self> {
        Self::from_seed_and_secret(0, secret)
    }

    /// Start a state with a custom seed and secret.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::InvalidSecret`] when `secret` is shorter than
    /// [`super::SECRET_MINIMUM_LENGTH`].
    pub fn from_seed_and_secret(seed: u64, secret: &[u8]) -> Result<Self> {
        secret::validate(DigestAlgorithm::Xxh3_128, secret)?;
        let secret: Arc<[u8]> = Arc::from(secret);
        Ok(Self {
            seed,
            hasher: build_128(seed, &secret),
            secret: Some(secret),
        })
    }

    /// Return the seed this state was constructed with.
    pub const fn seed(&self) -> u64 {
        self.seed
    }

    /// Return the custom secret this state was constructed with, if any.
    pub fn secret(&self) -> Option<&[u8]> {
        self.secret.as_deref()
    }

    /// Feed raw bytes.
    pub fn write_bytes(&mut self, bytes: &[u8]) {
        self.hasher.write(bytes);
    }

    /// Feed a reader to exhaustion, returning the bytes consumed.
    ///
    /// # Errors
    ///
    /// Returns the reader's failure. Bytes already fed stay fed.
    pub fn write_reader(&mut self, source: &mut impl Read) -> Result<u64> {
        let hasher = &mut self.hasher;
        feed(source, |chunk| hasher.write(chunk))
    }

    /// Answer the 128-bit value of everything fed so far.
    pub fn as_u128(&self) -> u128 {
        self.hasher.finish_128()
    }

    /// Answer the digest of everything fed so far.
    pub fn as_digest(&self) -> Digest {
        Digest::new(DigestAlgorithm::Xxh3_128, self.as_u128())
    }

    /// Reset to the constructed seed and secret, not to [`Self::new`].
    pub fn clear(&mut self) {
        self.hasher = match &self.secret {
            Some(secret) => build_128(self.seed, secret),
            None if self.seed == 0 => twox_hash::xxhash3_128::Hasher::new(),
            None => twox_hash::xxhash3_128::Hasher::with_seed(self.seed),
        };
    }
}

/// Build an XXH3-128 accumulator over an already validated secret.
fn build_128(seed: u64, secret: &[u8]) -> twox_hash::xxhash3_128::Hasher {
    match twox_hash::xxhash3_128::Hasher::with_seed_and_secret(seed, secret) {
        Ok(hasher) => hasher,
        Err(_) => unreachable!("the secret was validated before it was stored"),
    }
}

impl Default for Xxh3_128 {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Xxh3_128 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Xxh3_128")
            .field("seed", &self.seed)
            .field("secret", &self.secret.as_ref().map(|secret| secret.len()))
            .finish()
    }
}

impl Hasher for Xxh3_128 {
    /// Answer the low 64 bits of the 128-bit value.
    ///
    /// [`Hasher::finish`] returns a `u64` and cannot carry more.
    /// [`Self::as_u128`] is the full value, and the low half returned here is
    /// exactly XXH3-64 of the same input.
    fn finish(&self) -> u64 {
        low_64(self.as_u128())
    }

    fn write(&mut self, bytes: &[u8]) {
        self.write_bytes(bytes);
    }
}

impl BuildHasher for Xxh3_128 {
    type Hasher = Self;

    /// Build a fresh state carrying this state's seed and secret.
    fn build_hasher(&self) -> Self {
        let mut state = self.clone();
        state.clear();
        state
    }
}

/// Return the low 64 bits of a 128-bit digest without an unchecked cast.
pub(crate) fn low_64(value: u128) -> u64 {
    u64::try_from(value & u128::from(u64::MAX))
        .unwrap_or_else(|_| unreachable!("a masked u128 always fits a u64"))
}

/// Return the low 32 bits of a 64-bit seed without an unchecked cast.
pub(crate) fn low_32(value: u64) -> u32 {
    u32::try_from(value & u64::from(u32::MAX))
        .unwrap_or_else(|_| unreachable!("a masked u64 always fits a u32"))
}
