//! Structural and canonical-display adapters for the stable XXH3-64 sink.

use std::fmt;
use std::hash::{Hash, Hasher};

use crate::DigestAlgorithm;
use crate::xxhash::Feed;

/// Hash canonical display output with the stable Yggdryl XXH3-64 contract.
///
/// The rendering is written into the sink rather than assembled: a short
/// one - a name, a term, a plan - is staged on the stack and hashed in one
/// shot, and a long one streams from the byte it outgrows the stage, so a
/// value whose canonical text is large costs no copy of it.
pub(crate) fn stable_hash_display(value: &impl fmt::Display) -> u64 {
    let mut hasher = StableHash::new();
    let result = fmt::write(&mut hasher, format_args!("{value}"));
    debug_assert!(result.is_ok(), "the stable hash sink is infallible");
    hasher.finish()
}

/// Hash a native structural [`Hash`] implementation with the stable sink.
pub(crate) fn stable_hash_of(value: &impl Hash) -> u64 {
    let mut hasher = StableHash::new();
    value.hash(&mut hasher);
    hasher.finish()
}

/// XXH3-64 behind explicit little-endian integer writes.
///
/// [`Hasher`]'s default `write_u8` through `write_usize` bodies use
/// native-endian bytes, so handing a bare XXH3 state to a [`Hash`]
/// implementation would make a stored hash disagree between a big-endian and a
/// little-endian machine. Overriding them is the whole reason this stays a
/// named type rather than an inline call.
struct StableHash(Feed);

impl StableHash {
    const fn new() -> Self {
        Self(Feed::new(DigestAlgorithm::Xxh3))
    }
}

impl fmt::Write for StableHash {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        self.write(value.as_bytes());
        Ok(())
    }
}

impl Hasher for StableHash {
    fn finish(&self) -> u64 {
        self.0.as_u64()
    }

    fn write(&mut self, bytes: &[u8]) {
        self.0.write(bytes);
    }

    fn write_u8(&mut self, value: u8) {
        self.write(&value.to_le_bytes());
    }

    fn write_u16(&mut self, value: u16) {
        self.write(&value.to_le_bytes());
    }

    fn write_u32(&mut self, value: u32) {
        self.write(&value.to_le_bytes());
    }

    fn write_u64(&mut self, value: u64) {
        self.write(&value.to_le_bytes());
    }

    fn write_u128(&mut self, value: u128) {
        self.write(&value.to_le_bytes());
    }

    fn write_usize(&mut self, value: usize) {
        self.write_u64(value as u64);
    }

    fn write_i8(&mut self, value: i8) {
        self.write(&value.to_le_bytes());
    }

    fn write_i16(&mut self, value: i16) {
        self.write(&value.to_le_bytes());
    }

    fn write_i32(&mut self, value: i32) {
        self.write(&value.to_le_bytes());
    }

    fn write_i64(&mut self, value: i64) {
        self.write(&value.to_le_bytes());
    }

    fn write_i128(&mut self, value: i128) {
        self.write(&value.to_le_bytes());
    }

    fn write_isize(&mut self, value: isize) {
        self.write_i64(value as i64);
    }
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/hashing/stable.rs` pins and a caller cannot reach.
    //!
    //! The stable sink is the one hash a stored value is compared by, so its
    //! little-endian integer writes and its agreement with `xxhash::xxh3` are
    //! the contract. Each door is a forwarder, so nothing here is more public
    //! than it was: the `StableHash` sink stays private and is handed out
    //! only as the traits it implements.

    use std::fmt;
    use std::hash::{Hash, Hasher};

    /// Hash canonical display output with the stable XXH3-64 contract.
    pub fn stable_hash_display(value: &impl fmt::Display) -> u64 {
        super::stable_hash_display(value)
    }

    /// Hash a native structural [`Hash`] implementation with the stable sink.
    pub fn stable_hash_of(value: &impl Hash) -> u64 {
        super::stable_hash_of(value)
    }

    /// A fresh stable sink, to write into directly and finish.
    pub fn stable_hash_sink() -> impl Hasher {
        super::StableHash::new()
    }
}
