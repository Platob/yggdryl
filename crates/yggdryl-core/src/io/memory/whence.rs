//! [`Whence`] — the reference point a [`seek`](super::IOCursor::seek) is measured from.

use crate::io::IoError;

/// Where a seek offset is measured from — the POSIX `lseek` `whence`: the **start** of the
/// data (`SEEK_SET`), the **current** cursor position (`SEEK_CUR`), or the **end**
/// (`SEEK_END`). A signed offset is then added, so `End` with a negative offset walks
/// backwards from the end.
///
/// It is a tiny [`Copy`] value type — hashable and equatable — so it works as a map key and
/// crosses the FFI as a plain enum.
///
/// ```
/// use yggdryl_core::io::Whence;
///
/// // 4 bytes before the end of a 10-byte object is absolute position 6.
/// assert_eq!(Whence::End.resolve(-4, 0, 10).unwrap(), 6);
/// assert_eq!(Whence::Start.resolve(2, 0, 10).unwrap(), 2);
/// assert_eq!(Whence::Current.resolve(3, 5, 10).unwrap(), 8);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Whence {
    /// From the start of the data (absolute) — POSIX `SEEK_SET`.
    Start,
    /// From the current cursor position — POSIX `SEEK_CUR`.
    Current,
    /// From the end of the data — POSIX `SEEK_END`.
    End,
}

impl Whence {
    /// Resolves this whence + a signed `offset` to an absolute position (bytes from the
    /// start), given the current cursor `position` and the total `len`.
    ///
    /// A position **past the end** is allowed (a later write fills the gap with zeros; a read
    /// there is EOF), matching POSIX `lseek`. Seeking **before** the start — or past
    /// `u64::MAX` — is an [`IoError::InvalidSeek`] naming the offending target.
    ///
    /// ```
    /// use yggdryl_core::io::Whence;
    ///
    /// assert_eq!(Whence::End.resolve(5, 0, 10).unwrap(), 15); // past the end is fine
    /// assert!(Whence::Start.resolve(-1, 0, 10).is_err());     // before the start is not
    /// ```
    pub fn resolve(self, offset: i64, position: u64, len: u64) -> Result<u64, IoError> {
        let base = match self {
            Whence::Start => 0,
            Whence::Current => position,
            Whence::End => len,
        };
        // `i128` holds every `u64` base plus every `i64` offset without wrapping, so the
        // bounds check below sees the true target rather than a truncated one.
        let target = base as i128 + offset as i128;
        if target < 0 || target > u64::MAX as i128 {
            return Err(IoError::InvalidSeek {
                whence: self,
                offset,
                position,
                len,
            });
        }
        Ok(target as u64)
    }
}
