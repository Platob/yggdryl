//! `io::fixed` — the **fixed-width** typed layer: the numeric primitives (one per sub-module of
//! `integer`/`floating`) and the runtime-`N` fixed-size byte types (`binary`/`string`), each
//! layering the Arrow-style value types over the byte-I/O of [`Buffer`].
//!
//! The family-agnostic **root traits** live at the [`io`](crate::io) root; this module adds the
//! fixed-width `Fixed*` sub-trait (which pre-implements the mutualized logic as default methods)
//! and the concrete implementor for each root:
//!
//! | root trait ([`crate::io`]) | fixed sub-trait | concrete (fixed) |
//! | --- | --- | --- |
//! | [`DataType`](crate::io::DataType) / [`TypedDataType`](crate::io::TypedDataType) | [`FixedDataType`] | [`PrimitiveType<T>`](PrimitiveType) |
//! | [`FieldType`](crate::io::FieldType) | [`FixedField`] | [`Field`] / [`TypedField<T>`](TypedField) |
//! | [`ScalarType`](crate::io::ScalarType) | [`FixedScalar`] | [`Scalar<T>`](Scalar) |
//! | [`BufferType`](crate::io::BufferType) | [`FixedBuffer`] | [`Buffer<T>`](Buffer) (`U8Buffer` = [`Bytes`](crate::io::Bytes)) |
//! | [`SerieType`](crate::io::SerieType) | [`FixedSerie`] | [`Serie<T>`](Serie) |
//!
//! The variable-length family ([`var`](crate::io::var)) extends the same roots with its own
//! `Var*` sub-traits.
//!
//! [`NativeType`] is any `Copy` value with a fixed byte width, a [`DataTypeId`](crate::io::DataTypeId),
//! and a little-endian codec — the Rust integers `u8`…`i64`, the 128-bit `u128`/`i128`, the floats
//! `f16` (via [`half::f16`])/`f32`/`f64`, **and** the wide `u96`/`i96`/`u256`/`i256` `[u8; N]`
//! newtypes that have no Rust *or* Arrow primitive. The subset with a real zero-copy Arrow array is
//! [`ArrowNative`]; every `NativeType` still maps to *some* Arrow type via a closest-representation
//! fallback (see [`NativeType::arrow_data_type`]). A concrete primitive is a single thin
//! declarative file under `integer/`/`floating/` (`u8.rs`, `f16.rs`, …) — one [`fixed_native!`] /
//! [`native_only!`] / [`wide_int!`] plus the `fixed_dtype!` / `fixed_field!` / `fixed_scalar!` /
//! `fixed_serie!` / `fixed_buffer!` aliases — so adding a width is a handful of lines.
//!
//! The **fixed-size byte** family ([`binary`] = [`FixedBinary`], [`string`] = [`FixedUtf8`]) is a
//! different shape: values that are all exactly `N` bytes with `N` at *runtime* (a flat
//! `N`-byte-slot buffer + validity bitmap), so it implements the root traits directly over the
//! shared [`FixedSizeType`] / [`FixedSizeScalar`] / [`FixedSizeSerie`] generics rather than
//! [`NativeType`].
//!
//! Serialization for [`Scalar`] and [`Serie`] rides the [`IOCursor`](crate::io::IOCursor)
//! abstraction, so every type round-trips through any byte sink.

mod buffer;
mod dtype;
mod field;
mod fixed_size;
mod native;
mod null;
mod scalar;
mod serie;

// The concrete fixed-width primitives, grouped by category (each type is macro-backed thin
// files over the generics): `integer` (unsigned + signed) and `floating` (IEEE-754). The
// fixed-size **byte** types (runtime `N`) live in `binary` (`FixedBinary`) and `string`
// (`FixedUtf8`) over the shared `FixedSize*` generics.
pub mod binary;
pub mod decimal;
pub mod floating;
pub mod integer;
pub mod string;
pub mod temporal;

// Concrete fixed value/descriptor types.
pub use buffer::Buffer;
pub use dtype::PrimitiveType;
pub use field::{Field, TypedField};
pub use native::NativeType;
pub use null::{NullField, NullScalar, NullSerie, NullType};
pub use scalar::Scalar;
pub use serie::Serie;

/// The Arrow-native subset marker (feature `arrow`) — see [`native::ArrowNative`].
#[cfg(feature = "arrow")]
pub use native::ArrowNative;

// The fixed family's `Fixed*` sub-traits — the mutualized pre-implementations layered over the
// family-agnostic roots (`DataType`, `FieldType`, … which live in [`crate::io`]).
pub use buffer::FixedBuffer;
pub use dtype::FixedDataType;
pub use field::FixedField;
pub use scalar::FixedScalar;
pub use serie::FixedSerie;

// The runtime-`N` fixed-size byte family: the shared generics + the two concrete kinds.
pub use binary::{
    FixedBinary, FixedBinaryField, FixedBinaryScalar, FixedBinarySerie, FixedBinaryType,
};
pub use fixed_size::{
    FixedElement, FixedSizeField, FixedSizeScalar, FixedSizeSerie, FixedSizeType,
};
pub use string::{FixedUtf8, FixedUtf8Field, FixedUtf8Scalar, FixedUtf8Serie, FixedUtf8Type};

// The half-precision value type, re-exported for constructing `f16` scalars/columns
// (`F16Scalar::of(f16::from_f32(1.5))`).
pub use half::f16;

// The scaled-decimal family: the two shared traits, the generic value + columnar types, the four
// width markers, and their `D*` aliases — re-exported at the `fixed` root alongside the primitives.
pub use decimal::{
    D128Field, D128Scalar, D128Serie, D128Type, D256Field, D256Scalar, D256Serie, D256Type,
    D32Field, D32Scalar, D32Serie, D32Type, D64Field, D64Scalar, D64Serie, D64Type, Dec128, Dec256,
    Dec32, Dec64, Decimal, DecimalBacking, DecimalCoeff, DecimalError, DecimalField, DecimalScalar,
    DecimalSerie, DecimalType, D128, D256, D32, D64,
};

// The temporal columnar family: the two shared traits, the generic quartet, the nine concept+width
// markers, and their per-width aliases — re-exported at the `fixed` root alongside the value types.
pub use temporal::{
    Date32Field, Date32Kind, Date32Scalar, Date32Serie, Date32Type, Date64Field, Date64Kind,
    Date64Scalar, Date64Serie, Date64Type, Duration32Field, Duration32Kind, Duration32Scalar,
    Duration32Serie, Duration32Type, Duration64Field, Duration64Kind, Duration64Scalar,
    Duration64Serie, Duration64Type, TemporalBacking, TemporalField, TemporalNative,
    TemporalScalar, TemporalSerie, TemporalType, Time32Field, Time32Kind, Time32Scalar,
    Time32Serie, Time32Type, Time64Field, Time64Kind, Time64Scalar, Time64Serie, Time64Type,
    Ts32Field, Ts32Kind, Ts32Scalar, Ts32Serie, Ts32Type, Ts64Field, Ts64Kind, Ts64Scalar,
    Ts64Serie, Ts64Type, Ts96Field, Ts96Kind, Ts96Scalar, Ts96Serie, Ts96Type,
};

// Re-export every per-type alias at the `fixed` root, so `fixed::U8Buffer` etc. keep working
// regardless of the integer/floating grouping.
pub use floating::{
    F16Buffer, F16DataType, F16Field, F16Scalar, F16Serie, F32Buffer, F32DataType, F32Field,
    F32Scalar, F32Serie, F64Buffer, F64DataType, F64Field, F64Scalar, F64Serie,
};
pub use integer::{
    Bytes, I128Buffer, I128DataType, I128Field, I128Scalar, I128Serie, I16Buffer, I16DataType,
    I16Field, I16Scalar, I16Serie, I256Buffer, I256DataType, I256Field, I256Scalar, I256Serie,
    I32Buffer, I32DataType, I32Field, I32Scalar, I32Serie, I64Buffer, I64DataType, I64Field,
    I64Scalar, I64Serie, I8Buffer, I8DataType, I8Field, I8Scalar, I8Serie, I96Buffer, I96DataType,
    I96Field, I96Scalar, I96Serie, U128Buffer, U128DataType, U128Field, U128Scalar, U128Serie,
    U16Buffer, U16DataType, U16Field, U16Scalar, U16Serie, U256Buffer, U256DataType, U256Field,
    U256Scalar, U256Serie, U32Buffer, U32DataType, U32Field, U32Scalar, U32Serie, U64Buffer,
    U64DataType, U64Field, U64Scalar, U64Serie, U8Buffer, U8DataType, U8Field, U8Scalar, U8Serie,
    U96Buffer, U96DataType, U96Field, U96Scalar, U96Serie, I256, I96, U256, U96,
};

// -------------------------------------------------------------------------------------
// Per-type declaration macros — the whole per-type surface is a few of these.
// -------------------------------------------------------------------------------------

/// Implements [`NativeType`] for a fixed-width primitive via its inherent little-endian codec.
/// The arguments are the type, its name, its matching Arrow primitive type (e.g. `Int32Type`)
/// for the `arrow`-feature interop, and its [`DataTypeId`](crate::io::DataTypeId) variant
/// (`U8` / `I32` / `F16` / …).
macro_rules! fixed_native {
    ($t:ty, $name:literal, $arrow:ident, $id:ident) => {
        // Every primitive must fit the shared 32-byte stack scratch (`MAX_WIDTH`); a wider type
        // is a compile error here, not a runtime panic.
        const _: () = assert!(
            ::core::mem::size_of::<$t>() <= 32,
            "NativeType is wider than the 32-byte MAX_WIDTH scratch"
        );

        impl $crate::io::fixed::NativeType for $t {
            const NAME: &'static str = $name;
            const WIDTH: usize = ::core::mem::size_of::<$t>();
            const TYPE_ID: $crate::io::DataTypeId = $crate::io::DataTypeId::$id;

            fn write_le(self, out: &mut [u8]) {
                out[..::core::mem::size_of::<$t>()].copy_from_slice(&self.to_le_bytes());
            }

            fn read_le(bytes: &[u8]) -> Self {
                let mut array = [0u8; ::core::mem::size_of::<$t>()];
                array.copy_from_slice(&bytes[..::core::mem::size_of::<$t>()]);
                <$t>::from_le_bytes(array)
            }
        }

        #[cfg(feature = "arrow")]
        impl $crate::io::fixed::ArrowNative for $t {
            type Arrow = ::arrow_array::types::$arrow;
        }
    };
}
pub(crate) use fixed_native;

/// Implements [`NativeType`] for a Rust-native fixed-width integer that has **no** Arrow
/// primitive array (`u128` / `i128`): the LE codec is the inherent `to_le_bytes` /
/// `from_le_bytes`. It is deliberately **not** [`ArrowNative`], so a `Buffer` / `Serie` of it
/// has the full LE codec + serialization but no zero-copy `PrimitiveArray` interop; its Arrow
/// schema mapping is the closest representation from [`DataTypeId::to_arrow`](crate::io::DataTypeId::to_arrow).
macro_rules! native_only {
    ($t:ty, $name:literal, $id:ident) => {
        const _: () = assert!(
            ::core::mem::size_of::<$t>() <= 32,
            "NativeType is wider than the 32-byte MAX_WIDTH scratch"
        );

        impl $crate::io::fixed::NativeType for $t {
            const NAME: &'static str = $name;
            const WIDTH: usize = ::core::mem::size_of::<$t>();
            const TYPE_ID: $crate::io::DataTypeId = $crate::io::DataTypeId::$id;

            fn write_le(self, out: &mut [u8]) {
                out[..::core::mem::size_of::<$t>()].copy_from_slice(&self.to_le_bytes());
            }

            fn read_le(bytes: &[u8]) -> Self {
                let mut array = [0u8; ::core::mem::size_of::<$t>()];
                array.copy_from_slice(&bytes[..::core::mem::size_of::<$t>()]);
                <$t>::from_le_bytes(array)
            }
        }
    };
}
pub(crate) use native_only;

/// Defines a **wide non-Arrow-native integer** value type as a `#[repr(transparent)]`
/// little-endian `[u8; N]` newtype (for `u96` / `i96` / `u256` / `i256`, which have no Rust
/// primitive *and* no Arrow primitive) and implements [`NativeType`] for it. The byte array is
/// the canonical storage, so:
///
/// - value identity ([`PartialEq`]/[`Eq`]/[`Hash`]) is **byte-wise** — for a fixed-width
///   two's-complement LE encoding, equal value ⇔ equal bytes, so this is exact (negatives
///   included);
/// - there is deliberately **no `Ord`/`PartialOrd`** — little-endian byte order is *not* numeric
///   order, so a derived comparison would be a silent bug;
/// - it is **not** [`ArrowNative`] (Arrow has no matching primitive array); its Arrow schema
///   mapping is the closest representation from [`DataTypeId::to_arrow`](crate::io::DataTypeId::to_arrow).
///
/// Its align-1 layout makes [`Buffer::as_slice`](Buffer::as_slice) a *total* function (the
/// element-alignment assert can never fire).
macro_rules! wide_int {
    ($ty:ident, $width:literal, $name:literal, $id:ident) => {
        #[doc = concat!("A `", $name, "` value — ", stringify!($width), " little-endian bytes, \
            with byte-canonical equality/hashing and no ordering.")]
        #[repr(transparent)]
        #[derive(Clone, Copy)]
        pub struct $ty([u8; $width]);

        impl $ty {
            #[doc = concat!("The `", $name, "` whose little-endian bytes are `bytes`.")]
            pub const fn from_le_bytes(bytes: [u8; $width]) -> Self {
                Self(bytes)
            }

            /// This value's little-endian bytes.
            pub const fn to_le_bytes(self) -> [u8; $width] {
                self.0
            }
        }

        impl ::core::default::Default for $ty {
            fn default() -> Self {
                Self([0u8; $width])
            }
        }

        // Byte-wise value identity: equal value <=> equal little-endian bytes.
        impl ::core::cmp::PartialEq for $ty {
            fn eq(&self, other: &Self) -> bool {
                self.0 == other.0
            }
        }

        impl ::core::cmp::Eq for $ty {}

        impl ::core::hash::Hash for $ty {
            fn hash<H: ::core::hash::Hasher>(&self, state: &mut H) {
                self.0.hash(state);
            }
        }

        // DESIGN: no `Ord`/`PartialOrd` — LE byte order != numeric order (for both unsigned and
        // two's-complement signed), so a derive would be a silent correctness bug.

        impl ::core::fmt::Debug for $ty {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                write!(f, concat!($name, "({:02x?})"), self.0)
            }
        }

        const _: () = assert!(
            $width <= 32,
            "NativeType is wider than the 32-byte MAX_WIDTH scratch"
        );
        const _: () = assert!(
            $width == ::core::mem::size_of::<$ty>(),
            "declared WIDTH must equal the newtype's size"
        );

        impl $crate::io::fixed::NativeType for $ty {
            const NAME: &'static str = $name;
            const WIDTH: usize = $width;
            const TYPE_ID: $crate::io::DataTypeId = $crate::io::DataTypeId::$id;

            fn write_le(self, out: &mut [u8]) {
                out[..$width].copy_from_slice(&self.0);
            }

            fn read_le(bytes: &[u8]) -> Self {
                let mut array = [0u8; $width];
                array.copy_from_slice(&bytes[..$width]);
                Self(array)
            }
        }
    };
}
pub(crate) use wide_int;

/// Declares the typed data type alias for a fixed-width type.
macro_rules! fixed_dtype {
    ($name:ident, $t:ty) => {
        /// The typed data-type descriptor for this element type.
        pub type $name = $crate::io::fixed::PrimitiveType<$t>;
    };
}
pub(crate) use fixed_dtype;

/// Declares the typed field alias for a fixed-width type.
macro_rules! fixed_field {
    ($name:ident, $t:ty) => {
        /// The typed, named-column descriptor for this element type.
        pub type $name = $crate::io::fixed::TypedField<$t>;
    };
}
pub(crate) use fixed_field;

/// Declares the scalar alias for a fixed-width type.
macro_rules! fixed_scalar {
    ($name:ident, $t:ty) => {
        /// One nullable value of this element type.
        pub type $name = $crate::io::fixed::Scalar<$t>;
    };
}
pub(crate) use fixed_scalar;

/// Declares the serie (column) alias for a fixed-width type.
macro_rules! fixed_serie {
    ($name:ident, $t:ty) => {
        /// A nullable column of this element type.
        pub type $name = $crate::io::fixed::Serie<$t>;
    };
}
pub(crate) use fixed_serie;

/// Declares the buffer alias for a fixed-width type.
macro_rules! fixed_buffer {
    ($name:ident, $t:ty) => {
        /// A contiguous buffer of this element type (with byte I/O).
        pub type $name = $crate::io::fixed::Buffer<$t>;
    };
}
pub(crate) use fixed_buffer;
