//! The [`Seekable`] cursor trait.

use super::{IOError, Whence};

/// A resource with a cursor that can be queried and moved.
///
/// [`tell`](Seekable::tell) reports the current position (in bytes from the start);
/// [`seek`](Seekable::seek) moves it relative to a [`Whence`] and returns the new
/// position. It is implemented by the [`RawIOCursor`](super::RawIOCursor) and
/// [`IOCursor`](super::IOCursor) adapters, which measure [`Whence::Current`] from
/// this cursor and advance it on every read and write. A bare
/// [`RawIOBase`](super::RawIOBase) resource keeps no cursor of its own; wrap it in
/// one of those adapters for positioned streaming.
///
/// ```
/// use yggdryl_core::{IOError, Seekable, Whence};
///
/// #[derive(Default)]
/// struct Cursor {
///     position: usize,
///     len: usize,
/// }
///
/// impl Seekable for Cursor {
///     fn tell(&self) -> usize {
///         self.position
///     }
///
///     fn seek(&mut self, position: usize, whence: Whence) -> Result<usize, IOError> {
///         let base = match whence {
///             Whence::Start => 0,
///             Whence::Current => self.position,
///             Whence::End => self.len,
///             _ => 0,
///         };
///         self.position = base + position;
///         Ok(self.position)
///     }
/// }
///
/// let mut c = Cursor { position: 0, len: 10 };
/// assert_eq!(c.seek(3, Whence::Start)?, 3);
/// assert_eq!(c.tell(), 3);
/// assert_eq!(c.seek(2, Whence::Current)?, 5); // relative to the cursor
/// assert_eq!(c.seek(0, Whence::End)?, 10); // the end (append point)
/// # Ok::<(), yggdryl_core::IOError>(())
/// ```
pub trait Seekable {
    /// The current cursor position, in bytes from the start.
    fn tell(&self) -> usize;

    /// Move the cursor to `position` relative to `whence`, returning the new
    /// position, in bytes from the start.
    fn seek(&mut self, position: usize, whence: Whence) -> Result<usize, IOError>;
}
