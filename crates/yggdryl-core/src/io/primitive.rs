//! [`IoPrimitive`] — the little-endian element codec backing [`TypedCursor`].
//!
//! Mirrors the `buffer` layer's `primitive` module: one trait, stamped out once per
//! fixed-width type, so the generic typed cursor
//! ([`TypedCursor<T>`](crate::TypedCursor)) can serialise any `T` without knowing the
//! concrete type. Covers the native integers and floats plus the wide integers
//! [`i96`](crate::i96) / native `i128` / [`i256`](crate::i256).

use crate::{i256, i96};

/// A fixed-width primitive a [`TypedCursor`](crate::TypedCursor) reads and writes
/// little-endian. Implemented for every native integer and float (`i8` … `u64`,
/// `f32`, `f64`) and the wide integers ([`i96`](crate::i96), `i128`,
/// [`i256`](crate::i256)); `u8` is the byte case.
///
/// [`ZERO`](IoPrimitive::ZERO) is the value a typed write uses to fill any gap it
/// opens past the end of the resource (see
/// [`default_byte_array`](crate::TypedIOBase::default_byte_array)).
///
/// ```
/// use yggdryl_core::IoPrimitive;
///
/// assert_eq!(<i32 as IoPrimitive>::WIDTH, 4);
/// assert_eq!(i32::from_le_slice(&[0x04, 0x03, 0x02, 0x01]), 0x0102_0304);
/// assert_eq!(1_i16.to_le_vec(), vec![1, 0]);
/// ```
pub trait IoPrimitive: Copy {
    /// The width of one value in bytes.
    const WIDTH: usize;

    /// The zero value — the gap-fill pattern on a grow.
    const ZERO: Self;

    /// Whether a value's **in-memory** bytes equal its little-endian wire form on a
    /// little-endian target (so a `&[Self]` can be reinterpreted as `&[u8]`
    /// zero-copy). True for the native integers/floats (`WIDTH == size_of`); false
    /// for the wide integers whose storage width differs from the wire width (`i96`)
    /// or whose layout is not guaranteed (`i256`).
    const REINTERPRET_LE: bool;

    /// Appends this value's little-endian bytes to `out`.
    fn write_le(self, out: &mut Vec<u8>);

    /// Decodes one value from exactly [`WIDTH`](IoPrimitive::WIDTH) little-endian
    /// bytes.
    ///
    /// # Panics
    /// If `bytes.len() != WIDTH`. Callers pass whole-`WIDTH` chunks, so this cannot
    /// fire on the public IO surface.
    fn from_le_slice(bytes: &[u8]) -> Self;

    /// This value's little-endian bytes as a fresh `Vec` (a convenience over
    /// [`write_le`](IoPrimitive::write_le)).
    fn to_le_vec(self) -> Vec<u8> {
        let mut out = Vec::with_capacity(Self::WIDTH);
        self.write_le(&mut out);
        out
    }
}

/// Implements [`IoPrimitive`] for each native type via its inherent little-endian
/// conversions (their in-memory width equals the wire width).
macro_rules! io_primitive {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl IoPrimitive for $ty {
                const WIDTH: usize = core::mem::size_of::<$ty>();
                const ZERO: Self = 0 as $ty;
                const REINTERPRET_LE: bool = true;

                fn write_le(self, out: &mut Vec<u8>) {
                    out.extend_from_slice(&self.to_le_bytes());
                }

                fn from_le_slice(bytes: &[u8]) -> Self {
                    <$ty>::from_le_bytes(
                        bytes.try_into().expect("from_le_slice expects WIDTH bytes"),
                    )
                }
            }
        )+
    };
}

io_primitive!(i8, u8, i16, u16, i32, u32, i64, u64, i128, f32, f64);

// The wide integers whose wire width differs from `size_of` (`i96`) or that are not
// `as`-castable to zero (`i96` / `i256`), impl'd by hand.
impl IoPrimitive for i96 {
    const WIDTH: usize = 12;
    const ZERO: Self = i96::ZERO;
    const REINTERPRET_LE: bool = false; // 12-byte wire width, 16-byte storage

    fn write_le(self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.to_le_bytes());
    }

    fn from_le_slice(bytes: &[u8]) -> Self {
        i96::from_le_bytes(bytes.try_into().expect("from_le_slice expects 12 bytes"))
    }
}

impl IoPrimitive for i256 {
    const WIDTH: usize = 32;
    const ZERO: Self = i256::ZERO;
    const REINTERPRET_LE: bool = false; // Arrow's layout is not guaranteed to match

    fn write_le(self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.to_le_bytes());
    }

    fn from_le_slice(bytes: &[u8]) -> Self {
        i256::from_le_bytes(bytes.try_into().expect("from_le_slice expects 32 bytes"))
    }
}
