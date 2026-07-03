//! The `map` type: [`MapType`], its traits [`RawMap`] / [`TypedMap`], field
//! [`MapField`] and scalar [`Map`].
//!
//! A map value is a variable-length sequence of key–value entries.
//! [`MapType<K, V>`] is the concrete data type (a [`Nested`](crate::Nested) type
//! whose single child is the `"entries"` struct of `"key"` and `"value"`),
//! [`RawMap`] its untyped surface, [`TypedMap`] the typed layer whenever both types
//! have codecs, and [`Map`] a single, possibly-null sequence of key–value
//! inner scalars.
//!
//! ```
//! use yggdryl_data::{
//!     DataType, Int64Type, Int64, Map, MapType, RawDataType, RawScalar, UInt8Type,
//!     UInt8,
//! };
//!
//! let map = MapType::new(UInt8Type, Int64Type);
//! assert_eq!((map.name(), map.arrow_format().as_str()), ("map", "+m"));
//! assert_eq!(map.default_value(), Vec::<(u8, i64)>::new());
//!
//! let ranks = Map::new(vec![(UInt8::new(7), Int64::new(42))]).unwrap();
//! assert_eq!(ranks.value().map(<[_]>::len), Some(1));
//! assert_eq!(Map::from_arrow(ranks.to_arrow().as_ref()).unwrap(), ranks);
//! ```

mod data_type;
mod field;
mod raw_map;
mod scalar;
mod typed_map;

pub use data_type::MapType;
pub use field::MapField;
pub use raw_map::RawMap;
pub use scalar::Map;
pub use typed_map::TypedMap;
