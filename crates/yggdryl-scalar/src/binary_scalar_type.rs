//! The marker subtrait for opaque-bytes scalar types.

use yggdryl_schema::{Binary, FixedSizeBinary, LargeBinary};

use crate::ScalarType;

/// A [`ScalarType`] whose value is opaque bytes, unlocking the
/// [`from_binary`](crate::Scalar::from_binary) constructor and
/// [`as_binary`](crate::Scalar::as_binary) accessor on
/// [`Scalar`](crate::Scalar). `FixedSizeBinary` keeps validating its width
/// through the layout contract.
///
/// ```
/// use yggdryl_scalar::Scalar;
/// use yggdryl_schema::Binary;
///
/// let bytes = Scalar::from_binary(Binary, [0xDE, 0xAD]).unwrap();
/// assert_eq!(bytes.as_binary(), Some(&[0xDE, 0xAD][..]));
/// ```
pub trait BinaryScalarType: ScalarType {}

impl BinaryScalarType for Binary {}
impl BinaryScalarType for LargeBinary {}
impl BinaryScalarType for FixedSizeBinary {}
