//! [`Whence`] — the origin a positioned offset is measured from.

use core::fmt;

use crate::IoError;

/// The origin an [`IOBase`](crate::IOBase) offset is measured from — the seek
/// reference point, mirroring C `SEEK_SET` / `SEEK_CUR` / `SEEK_END`.
///
/// ```
/// use yggdryl_core::Whence;
///
/// // 4 bytes back from the end of a 10-byte resource is absolute position 6.
/// assert_eq!(Whence::End.resolve(-4, 0, 10).unwrap(), 6);
/// assert_eq!(Whence::Current.resolve(2, 5, 10).unwrap(), 7);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Whence {
    /// From the start of the resource — the offset is the absolute position and
    /// must be non-negative.
    Start,
    /// From the current cursor position.
    Current,
    /// From the end of the resource.
    End,
}

impl Whence {
    /// Resolves this origin and `offset` against the `current` cursor and the
    /// resource `len` into an absolute position.
    ///
    /// # Errors
    /// Returns [`IoError::InvalidSeek`] if the result is before the start of the
    /// resource or beyond the addressable (`u64`) range.
    pub fn resolve(self, offset: i64, current: u64, len: u64) -> Result<u64, IoError> {
        let base = match self {
            Self::Start => 0_i128,
            Self::Current => i128::from(current),
            Self::End => i128::from(len),
        };
        let absolute = base + i128::from(offset);
        if (0..=i128::from(u64::MAX)).contains(&absolute) {
            Ok(absolute as u64)
        } else {
            Err(IoError::InvalidSeek {
                offset,
                whence: self,
            })
        }
    }
}

impl fmt::Display for Whence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Start => "start",
            Self::Current => "current",
            Self::End => "end",
        })
    }
}
