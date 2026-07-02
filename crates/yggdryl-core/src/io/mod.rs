//! Positioned byte-I/O: the [`IOBase`] trait and its [`Whence`] reference point.

mod error;
mod whence;

pub use error::IOError;
pub use whence::Whence;

/// Positioned reads and writes over a resource of `T` elements.
///
/// Every access names a `position` and the [`Whence`] it is measured from, so no
/// cursor state is implied by the call itself. Implementors provide the two array
/// primitives [`pread_array`](IOBase::pread_array) and
/// [`pwrite_array`](IOBase::pwrite_array); the single-element
/// [`pread_one`](IOBase::pread_one) / [`pwrite_one`](IOBase::pwrite_one) come for
/// free from their default implementations.
///
/// ```
/// use yggdryl_core::{IOBase, IOError, Whence};
///
/// // A tiny in-memory byte store: implement the two array primitives and the
/// // single-element methods come for free.
/// #[derive(Default)]
/// struct Mem {
///     data: Vec<u8>,
/// }
///
/// impl Mem {
///     fn offset(&self, position: usize, whence: Whence) -> usize {
///         match whence {
///             Whence::End => self.data.len() + position,
///             _ => position,
///         }
///     }
/// }
///
/// impl IOBase<u8> for Mem {
///     fn pread_array(&self, position: usize, whence: Whence, size: usize) -> Result<Vec<u8>, IOError> {
///         let start = self.offset(position, whence);
///         let end = start + size;
///         if end > self.data.len() {
///             return Err(IOError::OutOfBounds { offset: end, len: self.data.len() });
///         }
///         Ok(self.data[start..end].to_vec())
///     }
///
///     fn pwrite_array(&mut self, position: usize, whence: Whence, values: &[u8]) -> Result<(), IOError> {
///         let start = self.offset(position, whence);
///         let end = start + values.len();
///         if end > self.data.len() {
///             self.data.resize(end, 0);
///         }
///         self.data[start..end].copy_from_slice(values);
///         Ok(())
///     }
/// }
///
/// let mut mem = Mem::default();
/// mem.pwrite_array(0, Whence::Start, &[1, 2, 3])?; // write three bytes
/// mem.pwrite_one(0, Whence::End, 4)?;              // append one byte
/// assert_eq!(mem.pread_array(0, Whence::Start, 4)?, vec![1, 2, 3, 4]);
/// assert_eq!(mem.pread_one(2, Whence::Start)?, 3);
/// # Ok::<(), yggdryl_core::IOError>(())
/// ```
pub trait IOBase<T> {
    /// Read one element at `position` relative to `whence`.
    fn pread_one(&self, position: usize, whence: Whence) -> Result<T, IOError> {
        crate::log_event!(
            trace,
            "IOBase::pread_one position={position} whence={whence:?}"
        );
        self.pread_array(position, whence, 1)?
            .into_iter()
            .next()
            .ok_or(IOError::UnexpectedEof {
                requested: 1,
                available: 0,
            })
    }

    /// Write one element at `position` relative to `whence`.
    fn pwrite_one(&mut self, position: usize, whence: Whence, value: T) -> Result<(), IOError> {
        crate::log_event!(
            trace,
            "IOBase::pwrite_one position={position} whence={whence:?}"
        );
        self.pwrite_array(position, whence, std::slice::from_ref(&value))
    }

    /// Read `size` elements starting at `position` relative to `whence`.
    fn pread_array(&self, position: usize, whence: Whence, size: usize) -> Result<Vec<T>, IOError>;

    /// Write `values` starting at `position` relative to `whence`.
    fn pwrite_array(
        &mut self,
        position: usize,
        whence: Whence,
        values: &[T],
    ) -> Result<(), IOError>;
}
