//! The [`IoCursor`] — a stateful cursor over an inner [`Io`].

use crate::io::{Io, IoError};
use crate::whence::Whence;

/// A read/write cursor over an inner [`Io`]. It adds the one thing the positional
/// [`Io`] trait leaves out — a retained cursor — so [`position`](Io::position) reads
/// it, [`seek`](Io::seek) moves it, and [`Whence::Current`] on a read or write
/// addresses it. Every other operation delegates straight to the inner source, so a
/// memory-resident inner keeps its bulk paths.
///
/// Reads and writes are positional: they resolve [`Whence::Current`] against the
/// cursor but do not move it — [`seek`](Io::seek) is the only thing that does.
///
/// ```
/// use yggdryl_core::{Io, IoCursor, Whence};
///
/// let mut io = IoCursor::new(vec![1u8, 2, 3, 4]);
/// io.seek(2, Whence::Start).unwrap();
/// assert_eq!(io.position().unwrap(), 2);
/// assert_eq!(io.pread(0, Whence::Current).unwrap(), 3); // reads at the cursor
/// ```
#[derive(Clone, Debug, Default)]
pub struct IoCursor<I> {
    io: I,
    cursor: usize,
}

impl<I> IoCursor<I> {
    /// A cursor over `io`, positioned at the start.
    pub fn new(io: I) -> Self {
        Self { io, cursor: 0 }
    }

    /// A shared reference to the inner io.
    pub fn get_ref(&self) -> &I {
        &self.io
    }

    /// Consumes the cursor, returning the inner io.
    pub fn into_inner(self) -> I {
        self.io
    }

    /// Moves the cursor to the absolute `position` without bounds-checking it (an
    /// out-of-range read or write errors when it runs). Use [`seek`](Io::seek) for a
    /// validated move.
    pub fn set_position(&mut self, position: usize) {
        self.cursor = position;
    }
}

/// Delegates every positional operation to the inner io — resolving against the
/// retained cursor first — so the cursor only adds stateful [`position`](Io::position)
/// / [`seek`](Io::seek) on top of the inner source.
impl<T, I: Io<T>> Io<T> for IoCursor<I> {
    fn len(&self) -> Result<usize, IoError> {
        self.io.len()
    }

    fn capacity(&self) -> Result<usize, IoError> {
        self.io.capacity()
    }

    fn with_capacity(&mut self, capacity: usize) -> Result<(), IoError> {
        self.io.with_capacity(capacity)
    }

    fn default(&self) -> T
    where
        T: Default,
    {
        self.io.default()
    }

    fn position(&self) -> Result<usize, IoError> {
        Ok(self.cursor)
    }

    fn seek(&mut self, position: usize, whence: Whence) -> Result<usize, IoError> {
        let target = self.resolve(position, whence)?;
        crate::log_event!(trace, "IoCursor::seek -> {target}");
        self.cursor = target;
        Ok(target)
    }

    fn pread(&self, position: usize, whence: Whence) -> Result<T, IoError> {
        self.io
            .pread(self.resolve(position, whence)?, Whence::Start)
    }

    fn pread_array(&self, position: usize, whence: Whence, len: usize) -> Result<Vec<T>, IoError> {
        self.io
            .pread_array(self.resolve(position, whence)?, Whence::Start, len)
    }

    fn pwrite(&mut self, position: usize, whence: Whence, value: T) -> Result<(), IoError> {
        let at = self.resolve(position, whence)?;
        self.io.pwrite(at, Whence::Start, value)
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
        self.io.pwrite_array(at, Whence::Start, values)
    }

    fn pwrite_io<S: Io<T> + ?Sized>(
        &mut self,
        position: usize,
        whence: Whence,
        source: &S,
    ) -> Result<usize, IoError>
    where
        T: Clone,
    {
        let at = self.resolve(position, whence)?;
        self.io.pwrite_io(at, Whence::Start, source)
    }

    fn resize(&mut self, len: usize) -> Result<(), IoError>
    where
        T: Default + Clone,
    {
        self.io.resize(len)
    }
}
