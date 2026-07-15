//! [`VarElement`] — the marker that distinguishes the variable-length kinds ([`Utf8`] strings
//! vs [`Binary`] byte strings), the way [`NativeType`](crate::io::fixed::NativeType)
//! distinguishes the fixed primitives. The concrete markers live in the
//! [`string`](crate::io::var::string) / [`binary`](crate::io::var::binary) sub-modules.

use crate::io::{DataTypeId, IoError};

/// The kind of a variable-length value — a byte string that is either **UTF-8 text**
/// ([`Utf8`](crate::io::var::Utf8)) or **opaque binary** ([`Binary`](crate::io::var::Binary)).
/// Every var value / column / descriptor is generic over one of these, so `ByteScalar<Utf8>` and
/// `ByteScalar<Binary>` share one implementation (mirroring how `Buffer<u8>` and `Buffer<i32>`
/// share one).
pub trait VarElement: Send + Sync + 'static {
    /// The stable, lower-case type name (`"utf8"` / `"binary"`).
    const NAME: &'static str;
    /// The [`DataTypeId`] — [`Utf8`](DataTypeId::Utf8) or [`Binary`](DataTypeId::Binary).
    const TYPE_ID: DataTypeId;

    /// Validates raw bytes for this kind — UTF-8 must decode; binary accepts anything. Returns
    /// [`IoError::InvalidUtf8`] naming the failing byte for a bad UTF-8 value.
    fn validate(bytes: &[u8]) -> Result<(), IoError>;
    // The Arrow *schema* mapping is centralized in `DataTypeId::to_arrow` (keyed on `TYPE_ID`).

    /// The matching Arrow **byte-array** element type (feature `arrow`) — `Utf8Type` for strings,
    /// `BinaryType` for binary — the type parameter of the [`GenericByteArray`](arrow_array::GenericByteArray)
    /// a [`ByteSerie`](crate::io::var::ByteSerie) of this kind converts to/from. Constrained to the
    /// `i32` offset width, matching this crate's single (non-`Large`) var offset axis.
    #[cfg(feature = "arrow")]
    type Arrow: arrow_array::types::ByteArrayType<Offset = i32>;
}
