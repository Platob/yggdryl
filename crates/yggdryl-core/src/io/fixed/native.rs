//! [`NativeType`] — the Rust value a fixed-width column stores, plus its canonical
//! little-endian byte codec — and [`ArrowNative`], the sub-trait for the subset that has a
//! zero-copy Arrow primitive array.

use crate::io::DataTypeId;

/// A Rust value that a fixed-width [`Buffer`](super::Buffer) / [`Scalar`](super::Scalar) /
/// [`Serie`](super::Serie) can store: a `Copy` value of a known byte width with a canonical
/// **little-endian** on-the-wire form and a [`DataTypeId`] identity.
///
/// Unlike the physical layer, `NativeType` does **not** require Arrow's
/// [`ArrowNativeType`](arrow_buffer::ArrowNativeType): it is implemented for the Arrow-native
/// primitives (`u8`…`i64`, `f16`/`f32`/`f64`) *and* for the wide 96/128/256-bit types that have
/// no Arrow primitive at all. The subset with a true zero-copy Arrow array is
/// [`ArrowNative`]; every `NativeType` still maps to *some* Arrow type via
/// [`arrow_data_type`](NativeType::arrow_data_type) (a closest representation for the wide ones).
pub trait NativeType: Copy + Default + Send + Sync + core::fmt::Debug + 'static {
    /// The stable, lower-case type name, e.g. `"u8"`, `"i32"`, `"f64"`, `"i256"`.
    const NAME: &'static str;
    /// The fixed width of one value in bytes (`size_of::<Self>()`).
    const WIDTH: usize;
    /// The type's [`DataTypeId`] — drives the [`DataType`](super::DataType) drill-down predicates.
    const TYPE_ID: DataTypeId;

    /// Writes this value's little-endian bytes into the first [`WIDTH`](NativeType::WIDTH)
    /// bytes of `out` (which must be at least that long).
    fn write_le(self, out: &mut [u8]);

    /// Reads a value from the first [`WIDTH`](NativeType::WIDTH) little-endian bytes of
    /// `bytes` (which must be at least that long).
    fn read_le(bytes: &[u8]) -> Self;
}

/// The subset of [`NativeType`] backed by a **true Arrow primitive array** — the integers
/// `u8`…`i64` and the floats `f16`/`f32`/`f64`. Only these convert to/from an
/// [`arrow_array::PrimitiveArray`] zero-copy; the wide 96/128/256-bit types are `NativeType`
/// but not `ArrowNative` (Arrow has no `PrimitiveArray` for them — an integer routed through a
/// `Decimal128`/`256` array would misrepresent its semantics, so they keep only the
/// [`arrow_data_type`](NativeType::arrow_data_type) schema mapping).
///
/// It requires [`ArrowNativeType`](arrow_buffer::ArrowNativeType) (the physical zero-copy
/// contract) as a supertrait, which is exactly why the wide types — which are not
/// `ArrowNativeType` — cannot implement it.
#[cfg(feature = "arrow")]
pub trait ArrowNative: NativeType + arrow_buffer::ArrowNativeType {
    /// The matching Arrow primitive type (e.g. `Int32Type`, `Float16Type`), whose `Native` is
    /// `Self`, so a `ScalarBuffer<Self>` is exactly its `PrimitiveArray`'s values buffer.
    type Arrow: arrow_array::ArrowPrimitiveType<Native = Self>;
}
