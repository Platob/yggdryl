//! The [`UInt32Serie`] scalar: a serie of `uint32` borrowing raw Arrow buffers.
//!
//! A single, possibly-null serie of `uint32` (native `u32` elements) of the
//! [`TypedSerieType<UInt32Type>`](yggdryl_dtype::TypedSerieType) data type, holding its
//! elements zero-copy in Arrow buffers.
//!
//! ```
//! use yggdryl_scalar::yggdryl_dtype::DataType;
//! use yggdryl_scalar::{UInt32Scalar, UInt32Serie, Scalar};
//!
//! let numbers = UInt32Serie::from(vec![1, 2, 3]);
//! assert_eq!(numbers.len(), 3);
//! assert_eq!(numbers.values(), Some(&[1, 2, 3][..])); // zero-copy buffer borrow
//! assert_eq!(numbers.get_at::<i64>(1).unwrap(), 2); // converted, exact-or-error
//! assert_eq!(numbers.get_scalar_at(1), Some(UInt32Scalar::new(2)));
//! assert_eq!(numbers.data_type().name(), "list");
//!
//! // Nulls are per element, read null-aware.
//! let sparse = UInt32Serie::from(vec![Some(1), None]);
//! assert!(sparse.get_at::<u32>(1).is_err()); // a null element holds no value
//! assert_eq!(sparse.get_scalar_at(1), Some(UInt32Scalar::null()));
//!
//! // The elements convert out as the Arrow primitive array, shared.
//! assert_eq!(numbers.to_arrow_array().len(), 3);
//!
//! // The Arrow round trip shares the buffers — no element is copied.
//! let arrow = numbers.to_arrow_scalar();
//! assert_eq!(arrow.len(), 1);
//! assert_eq!(UInt32Serie::from_arrow(arrow.as_ref()).unwrap(), numbers);
//!
//! assert!(UInt32Serie::null().is_null());
//! ```

crate::serie::int_serie!(
    UInt32Serie,
    UInt32Scalar,
    UInt32Type,
    u32,
    "uint32",
    UInt32Array,
    4
);
