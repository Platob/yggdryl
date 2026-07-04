//! The `map` type: [`MapType`] and its traits [`Map`] / [`TypedMap`].
//!
//! A map value is a variable-length sequence of key–value entries. [`MapType`] is
//! the concrete, *dynamic* data type (a [`Nested`](crate::Nested) type whose single
//! child is the `"entries"` struct of `"key"` and `"value"`), with [`Map`] its
//! untyped surface. [`TypedMapType<K, V>`] is the statically-typed map from a key
//! type `K` to a value type `V` (adding [`TypedMap`] and the byte codec), erasing
//! back to [`MapType`] with [`erase`](TypedMapType::erase). The matching field and
//! scalar live in `yggdryl-field` and `yggdryl-scalar`.
//!
//! ```
//! use yggdryl_dtype::{DataType, Int64Type, TypedDataType, TypedMap, TypedMapType, UInt8Type};
//!
//! let map = TypedMapType::new(UInt8Type, Int64Type);
//! assert_eq!((map.name(), map.arrow_format().as_str()), ("map", "+m"));
//! assert_eq!((map.key_type().name(), map.value_type().name()), ("uint8", "int64"));
//! assert_eq!(map.default_value(), Vec::<(u8, i64)>::new());
//! ```

mod data_type;
#[allow(clippy::module_inception)] // the base-trait module shares the family's bare name
mod map;
mod typed_map;
mod typed_map_type;

pub use data_type::MapType;
pub use map::Map;
pub use typed_map::TypedMap;
pub use typed_map_type::TypedMapType;
