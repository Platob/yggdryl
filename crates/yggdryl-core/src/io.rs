//! The positional [`Io`] array abstraction, its [`IoError`], and the in-memory
//! [`Vec`] leaf implementation.

use crate::whence::Whence;

/// An error raised by an [`Io`] operation.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum IoError {
    /// A resolved position fell outside the valid `0..=len` range.
    OutOfBounds,
    /// The source is read-only and cannot be written.
    ReadOnly,
}

impl std::fmt::Display for IoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IoError::OutOfBounds => {
                f.write_str("position out of bounds — expected an index within `0..=len`")
            }
            IoError::ReadOnly => f.write_str("source is read-only — writing is not supported"),
        }
    }
}

impl std::error::Error for IoError {}

/// A random-access array of `T` values addressed by an absolute element position
/// resolved from a [`Whence`] origin — the one abstraction every byte/array source
/// (an in-memory [`Vec`], a local file, a cloud object, an HTTP body) implements.
///
/// An implementor supplies [`len`](Io::len) and the two positional primitives
/// [`pread`](Io::pread) / [`pwrite`](Io::pwrite); the default
/// [`resolve`](Io::resolve) turns a `(position, whence)` pair into an absolute
/// index for them. A stateful source overrides [`position`](Io::position) so that
/// [`Whence::Current`] addresses its cursor.
///
/// ```
/// use yggdryl_core::{Io, Whence};
///
/// let mut io = vec![1u8, 2, 3, 4];
/// assert_eq!(io.pread(1, Whence::Start, 2).unwrap(), vec![2, 3]);
/// io.pwrite(0, Whence::End, &[5]).unwrap(); // append at the end
/// assert_eq!(io, vec![1, 2, 3, 4, 5]);
/// ```
pub trait Io<T> {
    /// The total number of `T` elements in the source.
    ///
    /// Named `len` for the element count; a leaf reaches through to its own storage
    /// (e.g. [`Vec::as_slice`]`().len()`) to answer it.
    fn len(&self) -> Result<u64, IoError>;

    /// Whether the source holds no elements.
    fn is_empty(&self) -> Result<bool, IoError> {
        Ok(self.len()? == 0)
    }

    /// The current cursor position (an element index), against which
    /// [`Whence::Current`] is resolved. Defaults to `0` for a cursorless source; a
    /// stateful source overrides it.
    fn position(&self) -> Result<u64, IoError> {
        Ok(0)
    }

    /// Resolves `position` measured from `whence` to an absolute element index in
    /// `0..=len`. A [`Whence::Start`] / [`Whence::Current`] offset counts forward
    /// (from the start / the cursor); a [`Whence::End`] offset counts back from the
    /// end. Errors [`OutOfBounds`](IoError::OutOfBounds) when it falls outside the
    /// source.
    fn resolve(&self, position: u64, whence: Whence) -> Result<u64, IoError> {
        let len = self.len()?;
        let index = match whence {
            Whence::Start => position,
            Whence::Current => self
                .position()?
                .checked_add(position)
                .ok_or(IoError::OutOfBounds)?,
            Whence::End => len.checked_sub(position).ok_or(IoError::OutOfBounds)?,
        };
        if index > len {
            return Err(IoError::OutOfBounds);
        }
        Ok(index)
    }

    /// Reads up to `len` elements starting at `position` measured from `whence`,
    /// returning those actually available there (fewer than `len` near the end,
    /// empty at the end). Errors [`OutOfBounds`](IoError::OutOfBounds) if the
    /// resolved position is past the end.
    fn pread(&self, position: u64, whence: Whence, len: usize) -> Result<Vec<T>, IoError>;

    /// Writes `values` at `position` measured from `whence` — overwriting, and
    /// extending the source when the write runs past the end — and returns the
    /// number of elements written. Errors [`ReadOnly`](IoError::ReadOnly) on a
    /// read-only source or [`OutOfBounds`](IoError::OutOfBounds) if the resolved
    /// position is past the end.
    fn pwrite(&mut self, position: u64, whence: Whence, values: &[T]) -> Result<usize, IoError>;
}

/// Narrows a resolved `u64` position to a `usize` index, erroring
/// [`OutOfBounds`](IoError::OutOfBounds) when it does not fit the address space.
fn index(position: u64) -> Result<usize, IoError> {
    usize::try_from(position).map_err(|_| IoError::OutOfBounds)
}

/// [`Vec`] is the in-memory array leaf: a read clones the requested window; a write
/// overwrites the overlapping elements and appends any that run past the end (the
/// resolved position is `<= len`, so a write is always contiguous — it never opens
/// a gap).
impl<T: Clone> Io<T> for Vec<T> {
    fn len(&self) -> Result<u64, IoError> {
        Ok(self.as_slice().len() as u64)
    }

    fn pread(&self, position: u64, whence: Whence, len: usize) -> Result<Vec<T>, IoError> {
        let start = index(self.resolve(position, whence)?)?;
        crate::log_event!(trace, "Vec::pread start={start} len={len}");
        let end = start.saturating_add(len).min(self.as_slice().len());
        Ok(self[start..end].to_vec())
    }

    fn pwrite(&mut self, position: u64, whence: Whence, values: &[T]) -> Result<usize, IoError> {
        let start = index(self.resolve(position, whence)?)?;
        crate::log_event!(trace, "Vec::pwrite start={start} len={}", values.len());
        let overlap = values.len().min(self.as_slice().len() - start);
        self[start..start + overlap].clone_from_slice(&values[..overlap]);
        self.extend_from_slice(&values[overlap..]);
        Ok(values.len())
    }
}
