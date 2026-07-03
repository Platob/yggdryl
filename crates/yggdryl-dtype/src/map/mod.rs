//! The `map` type: [`MapType`] and its traits [`Map`] / [`TypedMap`].
//!
//! A map value is a variable-length sequence of key–value entries. [`MapType<K, V>`]
//! is the concrete data type (a [`Nested`](crate::Nested) type whose single child
//! is the `"entries"` struct of `"key"` and `"value"`), [`Map`] its untyped surface,
//! and [`TypedMap`] the typed layer whenever both types have codecs. The matching
//! field and scalar live in `yggdryl-field` and `yggdryl-scalar`.
//!
//! ```
//! use yggdryl_dtype::{DataType, Int64Type, Map, MapType, TypedDataType, UInt8Type};
//!
//! let map = MapType::new(UInt8Type, Int64Type);
//! assert_eq!((map.name(), map.arrow_format().as_str()), ("map", "+m"));
//! assert_eq!((map.key_type().name(), map.value_type().name()), ("uint8", "int64"));
//! assert_eq!(map.default_value(), Vec::<(u8, i64)>::new());
//! ```

mod data_type;
#[allow(clippy::module_inception)] // the base-trait module shares the family's bare name
mod map;
mod typed_map;

pub use data_type::MapType;
pub use map::Map;
pub use typed_map::TypedMap;
