//! `varbyte` — **variable-length, byte-granular** element types: [`Binary`] / [`Utf8`] (`i32`
//! offsets) and [`LargeBinary`] / [`LargeUtf8`] (`i64` offsets — Arrow's `Large*`).
//!
//! A variable-length column is two [`IOBase`](crate::io::memory::IOBase) buffers — an **offsets**
//! buffer and a packed **data** buffer (element `i` is `data[offsets[i]..offsets[i + 1]]`) — plus the
//! same validity bitmap the fixed families use. The [`VarSerie`] carrier implements the shared
//! [`Scalar`](crate::typed::Scalar) / [`Serie`](crate::typed::Serie) traits (its `Value` is the
//! type's owned form, `Vec<u8>` / `String`), and [`VarScalar`] is the single-value case. Every marker
//! shares the [`VarType`](crate::typed::VarType) base descriptor; the variable-length markers refine
//! it with [`VarLenType`], whose [`VarOffset`] associated type pins the offset element width — the
//! only difference between a `Binary` (`i32`) and a `LargeBinary` (`i64`) column.

mod binary;
mod large;
mod offset;
mod scalar;
mod serie;
mod utf8;

pub use binary::Binary;
pub use large::{LargeBinary, LargeUtf8};
pub use offset::{VarLenType, VarOffset};
pub use scalar::VarScalar;
pub use serie::VarSerie;
pub use utf8::Utf8;
