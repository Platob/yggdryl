//! The `serie` type: [`SerieType`] and its traits [`Serie`] / [`TypedSerie`].
//!
//! A serie value is a variable-length sequence of one value type (the Apache Arrow
//! `list`). [`SerieType`] is the concrete, *dynamic* data type — it carries its
//! Arrow `"item"` child losslessly, like the dynamic
//! [`StructType`](crate::StructType) / [`UnionType`](crate::UnionType) — with
//! [`Serie`] its untyped surface. [`TypedSerieType<D>`] is the statically-typed serie
//! of a value type `D` (adding [`TypedSerie`] and the byte codec), erasing back to
//! [`SerieType`] with [`erase`](TypedSerieType::erase). The matching field and scalars
//! (`SerieField`, `Serie`, `Int64Serie`) live in `yggdryl-field` and `yggdryl-scalar`.
//!
//! ```
//! use yggdryl_dtype::{DataType, Int64Type, TypedDataType, TypedSerie, TypedSerieType};
//!
//! let serie = TypedSerieType::new(Int64Type);
//! assert_eq!((serie.name(), serie.arrow_format().as_str()), ("list", "+l"));
//! assert_eq!(serie.value_type().name(), "int64");
//! assert_eq!(serie.default_value(), Vec::<i64>::new());
//! ```

mod data_type;
#[allow(clippy::module_inception)] // the base-trait module shares the family's bare name
mod serie;
mod typed_serie;
mod typed_serie_type;

pub use data_type::SerieType;
pub use serie::Serie;
pub use typed_serie::TypedSerie;
pub use typed_serie_type::TypedSerieType;
