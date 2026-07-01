//! The [`IoSlice`] — a bounded window over an inner [`Io`].

use crate::io::{Io, IoError};
use crate::whence::Whence;

/// A fixed-length window `[offset, offset + len)` over an inner [`Io`], re-based so
/// its own positions run `0..len`. Reads and writes translate into the inner source
/// and clamp to the window, so a caller can neither see nor touch elements outside
/// it. Every operation delegates to the inner source, keeping its bulk paths.
///
/// ```
/// use yggdryl_core::{Io, IoSlice, Whence};
///
/// let io = IoSlice::new(vec![1u8, 2, 3, 4, 5], 1, 3); // the window [2, 3, 4]
/// assert_eq!(io.len().unwrap(), 3);
/// assert_eq!(io.pread_one(0, Whence::Start).unwrap(), 2);
/// assert_eq!(io.pread_array(0, Whence::Start, 10).unwrap(), vec![2, 3, 4]);
/// ```
#[derive(Clone, Debug, Default)]
pub struct IoSlice<I> {
    io: I,
    offset: usize,
    len: usize,
}

impl<I> IoSlice<I> {
    /// A window of `len` elements starting at `offset` in `io`.
    pub fn new(io: I, offset: usize, len: usize) -> Self {
        Self { io, offset, len }
    }

    /// The window's start offset in the inner io.
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// A shared reference to the inner io.
    pub fn get_ref(&self) -> &I {
        &self.io
    }

    /// Consumes the window, returning the inner io.
    pub fn into_inner(self) -> I {
        self.io
    }
}

/// Delegates every positional operation to the inner io — translating the
/// window-relative index to `offset + index` and clamping to the window — so nothing
/// outside `[offset, offset + len)` is reachable.
impl<T, I: Io<T>> Io<T> for IoSlice<I> {
    fn len(&self) -> Result<usize, IoError> {
        Ok(self.len)
    }

    fn default(&self) -> T
    where
        T: Default,
    {
        self.io.default()
    }

    fn pread_one(&self, position: usize, whence: Whence) -> Result<T, IoError> {
        let at = self.resolve(position, whence)?;
        if at >= self.len {
            return Err(IoError::OutOfBounds);
        }
        self.io.pread_one(self.offset + at, Whence::Start)
    }

    fn pread_array(&self, position: usize, whence: Whence, len: usize) -> Result<Vec<T>, IoError> {
        let at = self.resolve(position, whence)?;
        let count = len.min(self.len - at);
        self.io.pread_array(self.offset + at, Whence::Start, count)
    }

    fn pwrite_one(&mut self, position: usize, whence: Whence, value: T) -> Result<(), IoError> {
        let at = self.resolve(position, whence)?;
        if at >= self.len {
            return Err(IoError::OutOfBounds);
        }
        self.io.pwrite_one(self.offset + at, Whence::Start, value)
    }

    fn pwrite_array(
        &mut self,
        position: usize,
        whence: Whence,
        values: &[T],
    ) -> Result<usize, IoError>
    where
        T: Clone,
    {
        let at = self.resolve(position, whence)?;
        let count = values.len().min(self.len - at);
        if count < values.len() {
            crate::log_event!(
                warn,
                "IoSlice::pwrite_array clamping {} to {count} at the window end",
                values.len()
            );
        }
        self.io
            .pwrite_array(self.offset + at, Whence::Start, &values[..count])
    }

    fn resize(&mut self, len: usize) -> Result<(), IoError>
    where
        T: Default + Clone,
    {
        if len != self.len {
            crate::log_event!(
                warn,
                "IoSlice::resize ignoring {} -> {len}; a slice is a fixed window",
                self.len
            );
        }
        Ok(())
    }
}
