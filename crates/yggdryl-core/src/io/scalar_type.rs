//! [`ScalarType`] — the root contract for a single, nullable typed value, shared by every family.

use super::DataType;

/// The **generic scalar** root trait — one value of some type, possibly null. The abstraction
/// the fixed [`Scalar`](crate::io::fixed::Scalar) and the variable
/// [`ByteScalar`](crate::io::var::ByteScalar) both implement, so code can be generic over "a
/// scalar of type `Data`".
pub trait ScalarType {
    /// The scalar's data type descriptor.
    type Data: DataType;

    /// The scalar's data type.
    fn data_type(&self) -> Self::Data;

    /// Whether the scalar is null.
    fn is_null(&self) -> bool;

    /// Whether the scalar holds a value.
    fn is_valid(&self) -> bool {
        !self.is_null()
    }
}
