//! The [`Int16Serie`] scalar: a serie of `int16` borrowing raw Arrow buffers.
//!
//! A single, possibly-null serie of `int16` (native `i16` elements) of the
//! [`TypedSerieType<Int16Type>`](yggdryl_dtype::TypedSerieType) data type, holding its
//! elements zero-copy in Arrow buffers.
//!
//! ```
//! use yggdryl_scalar::yggdryl_dtype::DataType;
//! use yggdryl_scalar::{Int16Scalar, Int16Serie, Scalar};
//!
//! let numbers = Int16Serie::from(vec![1, 2, 3]);
//! assert_eq!(numbers.len(), 3);
//! assert_eq!(numbers.values(), Some(&[1, 2, 3][..])); // zero-copy buffer borrow
//! assert_eq!(numbers.get_at::<i64>(1).unwrap(), 2); // converted, exact-or-error
//! assert_eq!(numbers.scalar_at(1), Some(Int16Scalar::new(2)));
//! assert_eq!(numbers.data_type().name(), "list");
//!
//! // Nulls are per element, read null-aware.
//! let sparse = Int16Serie::from(vec![Some(1), None]);
//! assert!(sparse.get_at::<i16>(1).is_err()); // a null element holds no value
//! assert_eq!(sparse.scalar_at(1), Some(Int16Scalar::null()));
//!
//! // The elements convert out as the Arrow primitive array, shared.
//! assert_eq!(numbers.to_arrow_array().len(), 3);
//!
//! // The Arrow round trip shares the buffers — no element is copied.
//! let arrow = numbers.to_arrow_scalar();
//! assert_eq!(arrow.len(), 1);
//! assert_eq!(Int16Serie::from_arrow(arrow.as_ref()).unwrap(), numbers);
//!
//! assert!(Int16Serie::null().is_null());
//! ```

crate::serie::int_serie!(
    Int16Serie,
    Int16Scalar,
    Int16Type,
    i16,
    "int16",
    Int16Array,
    2
);
