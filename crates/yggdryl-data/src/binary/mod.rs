//! The `binary` type: [`Binary`], its field [`BinaryField`] and scalar
//! [`BinaryScalar`].
//!
//! A binary value is a variable-length sequence of bytes — Arrow's variable-size
//! binary layout, childless but with no fixed width. The scalar holds its bytes as
//! a core [`ByteBuffer`](yggdryl_core::ByteBuffer), so the value plugs straight
//! into the positioned-IO layer: borrow it with [`BinaryScalar::io`] for
//! [`RawIOBase`](yggdryl_core::RawIOBase) reads, or move it out with
//! [`BinaryScalar::into_io`] and wrap it in the core cursor / slice adapters.
//!
//! ```
//! use yggdryl_data::{Binary, BinaryScalar, DataType, RawDataType, RawScalar};
//!
//! assert_eq!((Binary.name(), Binary.arrow_format().as_str()), ("binary", "z"));
//! assert_eq!(Binary.default_value(), Vec::<u8>::new());
//!
//! let blob = BinaryScalar::new(vec![1, 2, 3]);
//! assert_eq!(blob.value(), Some(&[1, 2, 3][..]));
//! assert_eq!(
//!     BinaryScalar::from_arrow(blob.to_arrow().as_ref()).unwrap(),
//!     blob
//! );
//! ```

mod data_type;
mod field;
mod scalar;

pub use data_type::Binary;
pub use field::BinaryField;
pub use scalar::BinaryScalar;
