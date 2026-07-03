//! The `map` type: [`Map`] and its traits [`RawMap`] / [`TypedMap`].
//!
//! A map value is a variable-length sequence of key–value entries. [`Map<K, V>`]
//! is the concrete data type (a [`Nested`](crate::Nested) type whose single child
//! is the `"entries"` struct of `"key"` and `"value"`), [`RawMap`] its untyped
//! surface, and [`TypedMap`] the typed layer whenever both types have codecs. The
//! matching field and scalar live in `yggdryl-field` and `yggdryl-scalar`.
//!
//! ```
//! use yggdryl_dtype::{DataType, Int64, Map, RawDataType, RawMap, UInt8};
//!
//! let map = Map::new(UInt8, Int64);
//! assert_eq!((map.name(), map.arrow_format().as_str()), ("map", "+m"));
//! assert_eq!((map.key_type().name(), map.value_type().name()), ("uint8", "int64"));
//! assert_eq!(map.default_value(), Vec::<(u8, i64)>::new());
//! ```

mod data_type;
mod raw_map;
mod typed_map;

pub use data_type::Map;
pub use raw_map::RawMap;
pub use typed_map::TypedMap;
