//! The [`Whence`] reference point for positioned I/O.

/// The reference point a positioned-I/O `position` is measured from, mirroring the
/// POSIX `lseek` whence values.
///
/// ```
/// use yggdryl_core::Whence;
///
/// assert_eq!(Whence::default(), Whence::Start);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Whence {
    /// From the beginning: `position` is an absolute offset.
    #[default]
    Start,
    /// From the resource's current position (for stateful resources).
    Current,
    /// From the end: `position` counts forward from the current length, so `0` is
    /// the append point.
    End,
}
