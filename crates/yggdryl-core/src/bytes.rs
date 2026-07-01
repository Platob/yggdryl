//! The [`Bytes`] byte-serialization trait, layered on [`Io`].

use crate::io::{Io, IoError};
use crate::whence::Whence;

/// A value that serializes to and from bytes through a byte [`Io`].
///
/// The two primitives read/write the value against any `Io<u8>` sink or source:
/// [`pwrite_bytes`](Bytes::pwrite_bytes) writes `self` into a byte `Io` and
/// [`pread_bytes`](Bytes::pread_bytes) reads one back (with the number of bytes
/// consumed, so values compose sequentially). [`to_bytes`](Bytes::to_bytes) /
/// [`from_bytes`](Bytes::from_bytes) are the whole-value conveniences over a
/// `Vec<u8>`.
///
/// The integer primitives are implemented in little-endian.
///
/// ```
/// use yggdryl_core::Bytes;
///
/// let n: u32 = 0x0A0B_0C0D;
/// let bytes = n.to_bytes();
/// assert_eq!(bytes, vec![0x0D, 0x0C, 0x0B, 0x0A]); // little-endian
/// assert_eq!(u32::from_bytes(&bytes).unwrap(), n);
/// ```
pub trait Bytes: Sized {
    /// Writes `self` as bytes into `io` at `position` measured from `whence`,
    /// returning the number of bytes written.
    fn pwrite_bytes<W: Io<u8>>(
        &self,
        io: &mut W,
        position: usize,
        whence: Whence,
    ) -> Result<usize, IoError>;

    /// Reads a `Self` from `io` at `position` measured from `whence`, returning the
    /// value and the number of bytes consumed. Errors
    /// [`OutOfBounds`](IoError::OutOfBounds) if the input ends early.
    fn pread_bytes<R: Io<u8>>(
        io: &R,
        position: usize,
        whence: Whence,
    ) -> Result<(Self, usize), IoError>;

    /// Serializes `self` to a fresh byte vector.
    fn to_bytes(&self) -> Vec<u8> {
        let mut buf: Vec<u8> = Vec::new();
        self.pwrite_bytes(&mut buf, 0, Whence::Start)
            .expect("writing to a Vec<u8> never fails");
        buf
    }

    /// Deserializes a `Self` from `bytes`.
    fn from_bytes(bytes: &[u8]) -> Result<Self, IoError> {
        Self::pread_bytes(&bytes.to_vec(), 0, Whence::Start).map(|(value, _)| value)
    }
}

/// Implements [`Bytes`] for a fixed-width integer as `$n` little-endian bytes.
macro_rules! impl_bytes_le {
    ($ty:ty, $n:literal) => {
        impl Bytes for $ty {
            fn pwrite_bytes<W: Io<u8>>(
                &self,
                io: &mut W,
                position: usize,
                whence: Whence,
            ) -> Result<usize, IoError> {
                io.pwrite_array(position, whence, &self.to_le_bytes())
            }

            fn pread_bytes<R: Io<u8>>(
                io: &R,
                position: usize,
                whence: Whence,
            ) -> Result<(Self, usize), IoError> {
                let bytes = io.pread_array(position, whence, $n)?;
                let array: [u8; $n] = bytes
                    .as_slice()
                    .try_into()
                    .map_err(|_| IoError::OutOfBounds)?;
                Ok((<$ty>::from_le_bytes(array), $n))
            }
        }
    };
}

impl_bytes_le!(u8, 1);
impl_bytes_le!(u16, 2);
impl_bytes_le!(u32, 4);
impl_bytes_le!(u64, 8);

// The custom 256-bit integers serialize as their 32 little-endian bytes.
impl_bytes_le!(crate::int256::I256, 32);
impl_bytes_le!(crate::int256::U256, 32);
