//! The [`Float32Serie`] scalar: a serie of `float32` borrowing raw Arrow buffers.
//!
//! A single, possibly-null serie of `float32` (native `f32` elements) of the
//! [`TypedSerieType<Float32Type>`](yggdryl_dtype::TypedSerieType) data type, holding
//! its elements zero-copy in Arrow buffers.
//!
//! ```
//! use yggdryl_scalar::yggdryl_dtype::DataType;
//! use yggdryl_scalar::{Float32Scalar, Float32Serie, Scalar};
//!
//! let weights = Float32Serie::from(vec![1.5, 2.5, 3.5]);
//! assert_eq!(weights.len(), 3);
//! assert_eq!(weights.values(), Some(&[1.5, 2.5, 3.5][..])); // zero-copy buffer borrow
//! assert_eq!(weights.get_at::<f32>(1).unwrap(), 2.5); // converted, exact-or-error
//! assert_eq!(weights.scalar_at(1), Some(Float32Scalar::new(2.5)));
//! assert_eq!(weights.data_type().name(), "list");
//!
//! // Nulls are per element, read null-aware.
//! let sparse = Float32Serie::from(vec![Some(1.5), None]);
//! assert!(sparse.get_at::<f32>(1).is_err()); // a null element holds no value
//! assert_eq!(sparse.scalar_at(1), Some(Float32Scalar::null()));
//!
//! // The Arrow round trip shares the buffers — no element is copied.
//! let arrow = weights.to_arrow_scalar();
//! assert_eq!(arrow.len(), 1);
//! assert_eq!(Float32Serie::from_arrow(arrow.as_ref()).unwrap(), weights);
//!
//! assert!(Float32Serie::null().is_null());
//! ```

// Reuses the fixed-width little-endian primitive serie macro shared with the integer
// family (`f32`'s buffers and `to_le_bytes` / `from_le_bytes` make it identical).
crate::serie::int_serie!(
    Float32Serie,
    Float32Scalar,
    Float32Type,
    f32,
    "float32",
    Float32Array,
    4
);
