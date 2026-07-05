//! The [`Float16Serie`] scalar: a serie of `float16` borrowing raw Arrow buffers.
//!
//! A single, possibly-null serie of `float16` (native [`half::f16`](yggdryl_dtype::half::f16)
//! elements) of the [`TypedSerieType<Float16Type>`](yggdryl_dtype::TypedSerieType) data
//! type, holding its elements zero-copy in Arrow buffers.
//!
//! ```
//! use yggdryl_scalar::yggdryl_dtype::DataType;
//! use yggdryl_scalar::half::f16;
//! use yggdryl_scalar::{Float16Scalar, Float16Serie, Scalar};
//!
//! let weights = Float16Serie::from(vec![f16::from_f32(1.5), f16::from_f32(2.5)]);
//! assert_eq!(weights.len(), 2);
//! assert_eq!(weights.value_at::<f32>(1).unwrap(), 2.5); // widens exactly
//! assert_eq!(weights.scalar_at(0), Some(Float16Scalar::new(f16::from_f32(1.5))));
//! assert_eq!(weights.data_type().name(), "list");
//!
//! // The Arrow round trip shares the buffers — no element is copied.
//! let arrow = weights.to_arrow_scalar();
//! assert_eq!(arrow.len(), 1);
//! assert_eq!(Float16Serie::from_arrow(arrow.as_ref()).unwrap(), weights);
//!
//! assert!(Float16Serie::null().is_null());
//! ```

// Reuses the fixed-width little-endian primitive serie macro shared with the integer
// family (`half::f16` is an Arrow native element with the same `to_le_bytes` /
// `from_le_bytes` buffer surface).
crate::serie::int_serie!(
    Float16Serie,
    Float16Scalar,
    Float16Type,
    half::f16,
    "float16",
    Float16Array,
    2
);
