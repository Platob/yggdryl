//! The marker subtrait for opaque-bytes scalar types.

use yggdryl_schema::{BinaryType, FixedSizeBinaryType, LargeBinaryType};

use crate::ScalarType;

/// A [`ScalarType`] whose value is opaque bytes, unlocking the
/// [`from_binary`](crate::Scalar::from_binary) constructor and
/// [`as_binary`](crate::Scalar::as_binary) accessor on
/// [`Scalar`](crate::Scalar). `FixedSizeBinaryType` keeps validating its width
/// through the layout contract.
///
/// ```
/// use yggdryl_scalar::Scalar;
/// use yggdryl_schema::BinaryType;
///
/// let bytes = Scalar::from_binary(BinaryType, [0xDE, 0xAD]).unwrap();
/// assert_eq!(bytes.as_binary(), Some(&[0xDE, 0xAD][..]));
/// ```
pub trait BinaryScalarType: ScalarType {}

impl BinaryScalarType for BinaryType {}
impl BinaryScalarType for LargeBinaryType {}
impl BinaryScalarType for FixedSizeBinaryType {}
