//! Positioned byte- and bit-I/O: the [`IOBase`] trait and its [`Whence`] reference
//! point.

mod error;
mod whence;

pub use error::IOError;
pub use whence::Whence;

/// Positioned reads and writes over a resource, one or many `u8` bytes or `bool`
/// bits at a time.
///
/// Every access names a `position` and the [`Whence`] it is measured from —
/// counted in **bytes** for the `*_byte_*` methods and in **bits** (MSB-first, so
/// bit `0` of a byte is its most significant bit) for the `*_bit_*` methods.
/// Implementors provide the four array primitives
/// ([`pread_byte_array`](IOBase::pread_byte_array),
/// [`pwrite_byte_array`](IOBase::pwrite_byte_array),
/// [`pread_bit_array`](IOBase::pread_bit_array),
/// [`pwrite_bit_array`](IOBase::pwrite_bit_array)); the single-element `*_one`
/// methods come for free from their default implementations.
///
/// ```
/// use yggdryl_core::{IOBase, IOError, Whence};
///
/// // A byte buffer; this example addresses from the start, and bits are MSB-first.
/// #[derive(Default)]
/// struct Mem {
///     data: Vec<u8>,
/// }
///
/// impl IOBase for Mem {
///     fn pread_byte_array(&self, position: usize, _whence: Whence, size: usize) -> Result<Vec<u8>, IOError> {
///         let end = position + size;
///         if end > self.data.len() {
///             return Err(IOError::OutOfBounds { offset: end, len: self.data.len() });
///         }
///         Ok(self.data[position..end].to_vec())
///     }
///
///     fn pwrite_byte_array(&mut self, position: usize, _whence: Whence, values: &[u8]) -> Result<(), IOError> {
///         let end = position + values.len();
///         if end > self.data.len() {
///             self.data.resize(end, 0);
///         }
///         self.data[position..end].copy_from_slice(values);
///         Ok(())
///     }
///
///     fn pread_bit_array(&self, position: usize, _whence: Whence, size: usize) -> Result<Vec<bool>, IOError> {
///         if position + size > self.data.len() * 8 {
///             return Err(IOError::OutOfBounds { offset: position + size, len: self.data.len() * 8 });
///         }
///         Ok((0..size)
///             .map(|i| {
///                 let idx = position + i;
///                 (self.data[idx / 8] >> (7 - idx % 8)) & 1 == 1
///             })
///             .collect())
///     }
///
///     fn pwrite_bit_array(&mut self, position: usize, _whence: Whence, values: &[bool]) -> Result<(), IOError> {
///         let needed = (position + values.len()).div_ceil(8);
///         if needed > self.data.len() {
///             self.data.resize(needed, 0);
///         }
///         for (i, &bit) in values.iter().enumerate() {
///             let idx = position + i;
///             let mask = 1u8 << (7 - idx % 8);
///             if bit {
///                 self.data[idx / 8] |= mask;
///             } else {
///                 self.data[idx / 8] &= !mask;
///             }
///         }
///         Ok(())
///     }
/// }
///
/// let mut mem = Mem::default();
/// mem.pwrite_byte_array(0, Whence::Start, &[0b1010_0000])?;
/// assert_eq!(mem.pread_byte_one(0, Whence::Start)?, 0b1010_0000);
/// assert!(mem.pread_bit_one(0, Whence::Start)?); // the most significant bit
/// assert!(!mem.pread_bit_one(1, Whence::Start)?);
/// mem.pwrite_bit_one(1, Whence::Start, true)?; // flip bit 1 on
/// assert_eq!(mem.pread_byte_one(0, Whence::Start)?, 0b1110_0000);
/// # Ok::<(), yggdryl_core::IOError>(())
/// ```
pub trait IOBase {
    /// Read one byte at `position` (in bytes) relative to `whence`.
    fn pread_byte_one(&self, position: usize, whence: Whence) -> Result<u8, IOError> {
        crate::log_event!(
            trace,
            "IOBase::pread_byte_one position={position} whence={whence:?}"
        );
        self.pread_byte_array(position, whence, 1)?
            .into_iter()
            .next()
            .ok_or(IOError::UnexpectedEof {
                requested: 1,
                available: 0,
            })
    }

    /// Write one byte at `position` (in bytes) relative to `whence`.
    fn pwrite_byte_one(
        &mut self,
        position: usize,
        whence: Whence,
        value: u8,
    ) -> Result<(), IOError> {
        crate::log_event!(
            trace,
            "IOBase::pwrite_byte_one position={position} whence={whence:?}"
        );
        self.pwrite_byte_array(position, whence, std::slice::from_ref(&value))
    }

    /// Read `size` bytes starting at `position` (in bytes) relative to `whence`.
    fn pread_byte_array(
        &self,
        position: usize,
        whence: Whence,
        size: usize,
    ) -> Result<Vec<u8>, IOError>;

    /// Write `values` starting at `position` (in bytes) relative to `whence`.
    fn pwrite_byte_array(
        &mut self,
        position: usize,
        whence: Whence,
        values: &[u8],
    ) -> Result<(), IOError>;

    /// Read one bit at `position` (in bits) relative to `whence`.
    fn pread_bit_one(&self, position: usize, whence: Whence) -> Result<bool, IOError> {
        crate::log_event!(
            trace,
            "IOBase::pread_bit_one position={position} whence={whence:?}"
        );
        self.pread_bit_array(position, whence, 1)?
            .into_iter()
            .next()
            .ok_or(IOError::UnexpectedEof {
                requested: 1,
                available: 0,
            })
    }

    /// Write one bit at `position` (in bits) relative to `whence`.
    fn pwrite_bit_one(
        &mut self,
        position: usize,
        whence: Whence,
        value: bool,
    ) -> Result<(), IOError> {
        crate::log_event!(
            trace,
            "IOBase::pwrite_bit_one position={position} whence={whence:?}"
        );
        self.pwrite_bit_array(position, whence, std::slice::from_ref(&value))
    }

    /// Read `size` bits starting at `position` (in bits) relative to `whence`.
    fn pread_bit_array(
        &self,
        position: usize,
        whence: Whence,
        size: usize,
    ) -> Result<Vec<bool>, IOError>;

    /// Write `values` starting at `position` (in bits) relative to `whence`.
    fn pwrite_bit_array(
        &mut self,
        position: usize,
        whence: Whence,
        values: &[bool],
    ) -> Result<(), IOError>;
}
