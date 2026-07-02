//! The `map` type: [`MapType`], its traits [`RawMap`] / [`Map`], field
//! [`MapField`] and scalar [`MapScalar`].
//!
//! A map value is a variable-length sequence of key–value entries.
//! [`MapType<K, V>`] is the concrete data type (a [`Nested`](crate::Nested) type
//! whose single child is the `"entries"` struct of `"key"` and `"value"`),
//! [`RawMap`] its untyped surface, [`Map`] the typed layer whenever both types
//! have codecs, and [`MapScalar`] a single, possibly-null sequence of key–value
//! inner scalars.
//!
//! ```
//! use yggdryl_data::{
//!     DataType, Int64, Int64Scalar, MapScalar, MapType, RawDataType, RawScalar, UInt8,
//!     UInt8Scalar,
//! };
//!
//! let map = MapType::new(UInt8, Int64);
//! assert_eq!((map.name(), map.arrow_format().as_str()), ("map", "+m"));
//! assert_eq!(map.default_value(), Vec::<(u8, i64)>::new());
//!
//! let ranks = MapScalar::new(vec![(UInt8Scalar::new(7), Int64Scalar::new(42))]).unwrap();
//! assert_eq!(ranks.value().map(<[_]>::len), Some(1));
//! assert_eq!(MapScalar::from_arrow(ranks.to_arrow().as_ref()).unwrap(), ranks);
//! ```

mod data_type;
mod field;
#[allow(clippy::module_inception)]
mod map;
mod raw_map;
mod scalar;

pub use data_type::MapType;
pub use field::MapField;
pub use map::Map;
pub use raw_map::RawMap;
pub use scalar::MapScalar;
