//! Scalar values — a single, typed cell of data.
//!
//! Every scalar carries its [`AnyType`] and may be *null*. The byte-backed
//! scalars ([`BinaryScalar`], [`StringScalar`]) hold their payload in a
//! [`Buffer`](crate::Buffer), so cloning is O(1) and the bytes are borrowed, not
//! copied, by their accessors.

mod binary;
mod string;

pub use binary::BinaryScalar;
pub use string::StringScalar;

use crate::datatype::AnyType;

/// Behaviour shared by every scalar value.
pub trait Scalar {
    /// The scalar's data type.
    fn data_type(&self) -> AnyType;

    /// Whether the scalar holds the null value.
    fn is_null(&self) -> bool;
}
