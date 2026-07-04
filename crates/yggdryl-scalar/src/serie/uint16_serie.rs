//! The [`UInt16Serie`] scalar: a serie of `uint16` borrowing raw Arrow buffers.
//!
//! A single, possibly-null serie of `uint16` (native `u16` elements) of the
//! [`SerieType<UInt16Type>`](yggdryl_dtype::SerieType) data type, holding its
//! elements zero-copy in Arrow buffers.
//!
//! ```
//! use yggdryl_scalar::yggdryl_dtype::DataType;
//! use yggdryl_scalar::{UInt16Scalar, UInt16Serie, Scalar};
//!
//! let numbers = UInt16Serie::from(vec![1, 2, 3]);
//! assert_eq!(numbers.len(), 3);
//! assert_eq!(numbers.values(), Some(&[1, 2, 3][..])); // zero-copy buffer borrow
//! assert_eq!(numbers.get_at::<i64>(1).unwrap(), 2); // converted, exact-or-error
//! assert_eq!(numbers.get_scalar_at(1), Some(UInt16Scalar::new(2)));
//! assert_eq!(numbers.data_type().name(), "list");
//!
//! // Nulls are per element, read null-aware.
//! let sparse = UInt16Serie::from(vec![Some(1), None]);
//! assert!(sparse.get_at::<u16>(1).is_err()); // a null element holds no value
//! assert_eq!(sparse.get_scalar_at(1), Some(UInt16Scalar::null()));
//!
//! // The elements convert out as the Arrow primitive array, shared.
//! assert_eq!(numbers.to_arrow_array().len(), 3);
//!
//! // The Arrow round trip shares the buffers — no element is copied.
//! let arrow = numbers.to_arrow_scalar();
//! assert_eq!(arrow.len(), 1);
//! assert_eq!(UInt16Serie::from_arrow(arrow.as_ref()).unwrap(), numbers);
//!
//! assert!(UInt16Serie::null().is_null());
//! ```

crate::serie::int_serie!(
    UInt16Serie,
    UInt16Scalar,
    UInt16Type,
    u16,
    "uint16",
    UInt16Array,
    2
);
