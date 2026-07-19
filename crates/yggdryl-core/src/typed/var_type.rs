//! [`VarType`] — the compile-time descriptor of a **byte-blob element type** (binary / UTF-8),
//! the variable-length counterpart of [`DataType`](super::DataType).
//!
//! Where a `DataType` element is a fixed-width `Copy` scalar, a `VarType` element is a **byte
//! sequence** whose owned form is a `Vec<u8>` (binary) or `String` (UTF-8). The same descriptor
//! backs both layouts built on it: the variable-length [`VarSerie`](super::VarSerie) (an offsets +
//! data buffer, `i32` or `i64` offsets per the marker's [`VarLenType`](super::VarLenType)) and the
//! fixed-size [`FixedSizeSerie`](super::FixedSizeSerie) (a fixed byte stride) — a marker declares only
//! its tag and the bytes↔value conversion; the carrier picks the layout.

use crate::datatype_id::DataTypeId;

/// A byte-blob element type — its owned value plus the bytes↔value conversion.
pub trait VarType: Copy + Default {
    /// The owned value of one element (`Vec<u8>` for binary, `String` for UTF-8).
    type Owned;

    /// The stable [`DataTypeId`] tag.
    const DATA_TYPE_ID: DataTypeId;

    /// The tag — the runtime form of [`DATA_TYPE_ID`](VarType::DATA_TYPE_ID).
    fn data_type_id() -> DataTypeId {
        Self::DATA_TYPE_ID
    }

    /// Reconstruct the owned value from its raw bytes, or `None` when the bytes are **invalid** for
    /// the type (a non-UTF-8 sequence for a string type).
    fn to_owned(bytes: &[u8]) -> Option<Self::Owned>;

    /// The raw bytes of an owned value — what the carrier encodes.
    fn owned_bytes(value: &Self::Owned) -> &[u8];
}
