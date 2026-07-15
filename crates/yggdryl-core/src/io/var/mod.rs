//! `io::var` — the **variable-length** typed layer: the sibling of [`fixed`](super::fixed) for
//! types whose values are not a fixed byte width. Its two kinds live in sub-modules that mirror
//! `fixed`'s `integer`/`floating` grouping — [`string`] ([`Utf8`]) and [`binary`] ([`Binary`]) —
//! each stored Arrow-style as an `i32` **offsets** buffer over a contiguous **data** buffer with
//! an optional validity bitmap.
//!
//! Both kinds share **one** generic implementation, parameterized by a [`VarElement`] marker
//! (the way the fixed primitives share one, parameterized by
//! [`NativeType`](super::fixed::NativeType)):
//!
//! | root trait ([`crate::io`]) | var sub-trait | concrete | `Utf8` alias | `Binary` alias |
//! | --- | --- | --- | --- | --- |
//! | [`DataType`](crate::io::DataType) | [`VarDataType`] | [`ByteType<E>`](ByteType) | [`Utf8DataType`] | [`BinaryDataType`] |
//! | [`FieldType`](crate::io::FieldType) | [`VarField`] | [`ByteField<E>`](ByteField) | [`Utf8Field`] | [`BinaryField`] |
//! | [`ScalarType`](crate::io::ScalarType) | [`VarScalar`] | [`ByteScalar<E>`](ByteScalar) | [`Utf8Scalar`] | [`BinaryScalar`] |
//! | [`SerieType`](crate::io::SerieType) | [`VarSerie`] | [`ByteSerie<E>`](ByteSerie) | [`Utf8Serie`] | [`BinarySerie`] |
//!
//! Every value / column / descriptor validates its bytes for the kind (a [`Utf8`] value is
//! always valid UTF-8) and round-trips through the [`IOCursor`](crate::io::IOCursor)
//! abstraction, so it serializes to any byte sink.
//!
//! DESIGN: there is deliberately **no `VarBuffer`** peer of [`FixedBuffer`](super::fixed::FixedBuffer).
//! A fixed column's physical storage is one flat typed [`Buffer`](super::fixed::Buffer); a
//! variable column's is *two* buffers (offsets + data) held inside [`ByteSerie`], so a
//! standalone "var buffer" would model nothing the serie doesn't already own. The raw data
//! buffer, when needed on its own, is just [`Bytes`](crate::io::Bytes) (`Buffer<u8>`). The
//! `Large` (`i64`-offset) kinds are reserved at
//! [`DataTypeId`](crate::io::DataTypeId::LargeUtf8) for a future offset-width axis.

pub mod binary;
pub mod string;

mod dtype;
mod element;
mod field;
mod scalar;
mod serie;

// The variable-length family's `Var*` sub-traits — the siblings of the fixed family's `Fixed*`
// traits, both layered over the family-agnostic roots in [`crate::io`].
pub use dtype::VarDataType;
pub use field::VarField;
pub use scalar::VarScalar;
pub use serie::VarSerie;

// The generic concrete types + the element-marker contract.
pub use dtype::ByteType;
pub use element::VarElement;
pub use field::ByteField;
pub use scalar::ByteScalar;
pub use serie::ByteSerie;

// The concrete kinds + their aliases, re-exported at the `var` root so `var::Utf8Serie` etc.
// keep working regardless of the string/binary grouping.
pub use binary::{Binary, BinaryDataType, BinaryField, BinaryScalar, BinarySerie};
pub use string::{Utf8, Utf8DataType, Utf8Field, Utf8Scalar, Utf8Serie};
