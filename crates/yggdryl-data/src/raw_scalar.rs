//! The [`RawScalar`] base trait: a single, possibly-null value of a [`RawDataType`].

use super::{DataError, RawDataType};

/// A single value of a data type, possibly null — the base trait mirroring an Apache
/// Arrow `Scalar`.
///
/// It carries its [`data_type`](RawScalar::data_type) of type `D`, reports whether it
/// [`is_null`](RawScalar::is_null), and exposes the native Rust
/// [`value`](RawScalar::value) (of the associated [`Value`](RawScalar::Value) type)
/// when non-null. Arrow models a scalar as an array of exactly one value, so
/// [`to_arrow`](RawScalar::to_arrow) builds a one-element
/// [`arrow_array::ArrayRef`] (null when the scalar is null) and
/// [`from_arrow`](RawScalar::from_arrow) reads one back. Parameterising by `D` keeps
/// the concrete type available for zero-cost access; the associated `Value` names the
/// in-memory representation a concrete scalar holds. It shares [`RawDataType`]'s
/// `Debug + Send + Sync` bounds so scalar values are printable and shareable across
/// threads and FFI. The associated [`Value`](RawScalar::Value) is `?Sized`, so a
/// string scalar can expose `Value = str`.
///
/// ```
/// use yggdryl_data::{arrow_array, DataError, Int32, RawDataType, RawScalar};
/// use arrow_array::Array; // len / is_null on the arrow side
///
/// #[derive(Debug)]
/// struct Int32Scalar {
///     data_type: Int32,
///     value: Option<i32>,
/// }
///
/// impl RawScalar<Int32> for Int32Scalar {
///     type Value = i32;
///     fn data_type(&self) -> &Int32 {
///         &self.data_type
///     }
///     fn is_null(&self) -> bool {
///         self.value.is_none()
///     }
///     fn value(&self) -> Option<&i32> {
///         self.value.as_ref()
///     }
///     fn to_arrow(&self) -> arrow_array::ArrayRef {
///         std::sync::Arc::new(match self.value {
///             Some(value) => arrow_array::Int32Array::from_iter_values([value]),
///             None => arrow_array::Int32Array::new_null(1),
///         })
///     }
///     fn from_arrow(array: &dyn arrow_array::Array) -> Result<Self, DataError> {
///         if array.len() != 1 {
///             return Err(DataError::InvalidScalarLength { got: array.len() });
///         }
///         let array = array
///             .as_any()
///             .downcast_ref::<arrow_array::Int32Array>()
///             .ok_or_else(|| DataError::IncompatibleArrowType {
///                 expected: "Int32".to_string(),
///                 got: array.data_type().to_string(),
///             })?;
///         Ok(Int32Scalar {
///             data_type: Int32,
///             value: (!array.is_null(0)).then(|| array.value(0)),
///         })
///     }
/// }
///
/// let answer = Int32Scalar { data_type: Int32, value: Some(42) };
/// assert_eq!(answer.data_type().name(), "int32");
/// assert!(!answer.is_null());
/// assert_eq!(answer.value(), Some(&42));
///
/// // Arrow interop: a one-element array, round-tripped.
/// let arrow = answer.to_arrow();
/// assert_eq!(arrow.len(), 1);
/// assert_eq!(Int32Scalar::from_arrow(arrow.as_ref()).unwrap().value(), Some(&42));
///
/// let missing = Int32Scalar { data_type: Int32, value: None };
/// assert!(missing.is_null());
/// assert!(missing.to_arrow().is_null(0));
/// ```
pub trait RawScalar<D: RawDataType>: std::fmt::Debug + Send + Sync {
    /// The native Rust representation this scalar holds when non-null. May be
    /// unsized (e.g. `str`).
    type Value: ?Sized;

    /// The scalar's data type.
    fn data_type(&self) -> &D;

    /// Whether this scalar holds a null value.
    fn is_null(&self) -> bool;

    /// The scalar's value, or `None` when it [`is_null`](RawScalar::is_null).
    fn value(&self) -> Option<&Self::Value>;

    /// The Apache Arrow form of this scalar: a one-element
    /// [`arrow_array::ArrayRef`] of this scalar's data type, holding the value (or a
    /// null). This is Arrow's own scalar representation — a length-1 array — so it
    /// plugs straight into arrow-rs kernels (wrap it in `arrow_array::Scalar` for a
    /// `Datum`).
    fn to_arrow(&self) -> arrow_array::ArrayRef;

    /// Build this scalar from its one-element Apache Arrow array — the exact inverse
    /// of [`to_arrow`](RawScalar::to_arrow). An array whose length is not exactly 1
    /// errors with [`DataError::InvalidScalarLength`]; an array of a different Arrow
    /// type errors with [`DataError::IncompatibleArrowType`].
    fn from_arrow(array: &dyn arrow_array::Array) -> Result<Self, DataError>
    where
        Self: Sized;
}
