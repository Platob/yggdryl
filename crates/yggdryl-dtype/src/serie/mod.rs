//! The `serie` type: [`SerieType`] and its traits [`Serie`] / [`TypedSerie`].
//!
//! A serie value is a variable-length sequence of one value type (the Apache Arrow
//! `list`). [`SerieType<D>`] is the concrete data type (a [`Nested`](crate::Nested)
//! type whose single child is the nullable `"item"` field), [`Serie`] its untyped
//! surface, and [`TypedSerie`] the typed layer whenever the value type has a codec.
//! The matching field and scalars (`SerieField`, `Serie`, `Int64Serie`) live in
//! `yggdryl-field` and `yggdryl-scalar`.
//!
//! ```
//! use yggdryl_dtype::{DataType, Int64Type, Serie, SerieType, TypedDataType};
//!
//! let serie = SerieType::new(Int64Type);
//! assert_eq!((serie.name(), serie.arrow_format().as_str()), ("list", "+l"));
//! assert_eq!(serie.value_type().name(), "int64");
//! assert_eq!(serie.default_value(), Vec::<i64>::new());
//! ```

mod data_type;
#[allow(clippy::module_inception)] // the base-trait module shares the family's bare name
mod serie;
mod typed_serie;

pub use data_type::SerieType;
pub use serie::Serie;
pub use typed_serie::TypedSerie;
