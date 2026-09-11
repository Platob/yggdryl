//! The two targets a decode appends UTF-8 to.

use crate::Result;

/// A target one decode appends UTF-8 to.
///
/// Every decode in this module emits exactly two kinds of output: a run of
/// bytes that is already UTF-8 because it is US-ASCII, and one scalar a table
/// or a sequence answered. A [`String`] wants that scalar and a byte target
/// wants its pre-encoded bytes, and both want the run copied whole, so each
/// codec is written once against this pair and each target spells the two
/// operations the cheapest way its representation allows.
pub(super) trait Utf8Sink {
    /// Append a run of bytes the caller proved to be UTF-8 already.
    ///
    /// # Errors
    ///
    /// Returns the refusal of a target that must re-check the run, which a
    /// run the caller's own scan produced never trips.
    fn push_utf8(&mut self, run: &[u8]) -> Result<()>;

    /// Append one scalar, `encoded` carrying its UTF-8 bytes.
    fn push_scalar(&mut self, scalar: char, encoded: &[u8]);

    /// Reserve room for `additional` more bytes of output.
    fn reserve(&mut self, additional: usize);
}

impl Utf8Sink for String {
    fn push_utf8(&mut self, run: &[u8]) -> Result<()> {
        self.push_str(super::ascii::text(run)?);
        Ok(())
    }

    fn push_scalar(&mut self, scalar: char, _encoded: &[u8]) {
        self.push(scalar);
    }

    fn reserve(&mut self, additional: usize) {
        Self::reserve(self, additional);
    }
}

impl Utf8Sink for Vec<u8> {
    fn push_utf8(&mut self, run: &[u8]) -> Result<()> {
        self.extend_from_slice(run);
        Ok(())
    }

    fn push_scalar(&mut self, _scalar: char, encoded: &[u8]) {
        self.extend_from_slice(encoded);
    }

    fn reserve(&mut self, additional: usize) {
        Self::reserve(self, additional);
    }
}
