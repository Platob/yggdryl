//! `varbyte` — **variable-length, byte-granular** element types: [`Binary`] and [`Utf8`].
//!
//! A variable-length column is two [`IOBase`](crate::io::memory::IOBase) buffers — an `i32`
//! **offsets** buffer and a packed **data** buffer (element `i` is `data[offsets[i]..offsets[i + 1]]`)
//! — plus the same validity bitmap the fixed families use. The [`VarSerie`] carrier implements the
//! shared [`Scalar`](crate::typed::Scalar) / [`Serie`](crate::typed::Serie) traits (its `Value` is
//! the type's owned form, `Vec<u8>` / `String`), and [`VarScalar`] is the single-value case. Both
//! types share the [`VarType`](crate::typed::VarType) base descriptor.

mod binary;
mod scalar;
mod serie;
mod utf8;

pub use binary::Binary;
pub use scalar::VarScalar;
pub use serie::VarSerie;
pub use utf8::Utf8;
