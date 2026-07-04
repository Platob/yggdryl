//! The [`UInt64Serie`] scalar: a serie of `uint64` borrowing raw Arrow buffers.
//!
//! A single, possibly-null serie of `uint64` (native `u64` elements) of the
//! [`SerieType<UInt64Type>`](yggdryl_dtype::SerieType) data type, holding its
//! elements zero-copy in Arrow buffers.
//!
//! ```
//! use yggdryl_scalar::yggdryl_dtype::DataType;
//! use yggdryl_scalar::{UInt64Scalar, UInt64Serie, Scalar};
//!
//! let numbers = UInt64Serie::from(vec![1, 2, 3]);
//! assert_eq!(numbers.len(), 3);
//! assert_eq!(numbers.values(), Some(&[1, 2, 3][..])); // zero-copy buffer borrow
//! assert_eq!(numbers.get_at::<i64>(1).unwrap(), 2); // converted, exact-or-error
//! assert_eq!(numbers.get_scalar_at(1), Some(UInt64Scalar::new(2)));
//! assert_eq!(numbers.data_type().name(), "list");
//!
//! // Nulls are per element, read null-aware.
//! let sparse = UInt64Serie::from(vec![Some(1), None]);
//! assert!(sparse.get_at::<u64>(1).is_err()); // a null element holds no value
//! assert_eq!(sparse.get_scalar_at(1), Some(UInt64Scalar::null()));
//!
//! // The Arrow round trip shares the buffers — no element is copied.
//! let arrow = numbers.to_arrow();
//! assert_eq!(arrow.len(), 1);
//! assert_eq!(UInt64Serie::from_arrow(arrow.as_ref()).unwrap(), numbers);
//!
//! assert!(UInt64Serie::null().is_null());
//! ```

crate::serie::int_serie!(
    UInt64Serie,
    UInt64Scalar,
    UInt64Type,
    u64,
    "uint64",
    UInt64Array,
    8
);
