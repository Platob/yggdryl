//! The marker subtrait for UTF-8 string scalar types.

use yggdryl_schema::{LargeUtf8, Utf8};

use crate::ScalarType;

/// A [`ScalarType`] whose value is a UTF-8 string, unlocking the
/// [`from_string`](crate::Scalar::from_string) constructor and
/// [`as_str`](crate::Scalar::as_str) accessor on
/// [`Scalar`](crate::Scalar).
///
/// ```
/// use yggdryl_scalar::Scalar;
/// use yggdryl_schema::LargeUtf8;
///
/// assert_eq!(Scalar::from_string(LargeUtf8, "ygg").as_str(), Some("ygg"));
/// ```
pub trait StringScalarType: ScalarType {}

impl StringScalarType for Utf8 {}
impl StringScalarType for LargeUtf8 {}
