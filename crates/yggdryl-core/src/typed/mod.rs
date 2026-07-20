//! `typed` — the **typed data serialization layer** grown on top of the [`io`](crate::io) byte
//! contract.
//!
//! Where `io` moves *bytes*, `typed` gives them a **precise element type**. Six small abstractions
//! compose it:
//!
//! - [`DataType`] — the compile-time descriptor of a fixed-width type (its [`DataTypeId`] tag + the
//!   native Rust scalar it maps to).
//! - [`Encoder`] / [`Decoder`] — write / read a native value as an element of that type into / from
//!   **any** [`IOBase`](crate::io::memory::IOBase), forwarding to the byte layer's vectorized typed
//!   array kernels.
//! - [`Reduce`] — the numeric aggregations (`sum`/`min`/`max`/`mean`/`std`/`var`/`median`/`first`/
//!   `last`/`count_ge`) a type routes to the source's [`Aggregate`](crate::io::memory::Aggregate)
//!   kernels; every [`Serie`] also inherits the type-agnostic universal aggregations
//!   (`count`/`valid_count`/`first_value`/`last_value`/`n_unique`/`min_value`/`max_value`).
//! - [`Scalar`] — an indexed, possibly-null typed value; [`Serie`] (`: Scalar`) refines it into a
//!   **column** — many elements over a data buffer plus an optional validity bit buffer.
//! - [`Field`] — a column's metadata (name, type, nullable), carried in a [`Headers`](crate::headers::Headers).
//!
//! The concrete implementations are split by **length × granularity**: [`fixedbyte`] (integers,
//! floats, decimals, and the fixed-size `FixedBinary` / `FixedUtf8`), [`fixedbit`] (booleans),
//! [`varbyte`] (the variable-length `Binary` / `Utf8`), and the reserved [`varbit`] (bit-lists). A
//! fixed-width type is one macro line; the variable-length types share the [`VarType`] base
//! descriptor over an offsets+data ([`VarSerie`]) or fixed-stride ([`FixedSizeSerie`]) carrier. All
//! carriers are generic over the type **and** the backing [`IOBase`], so a column is in-heap,
//! memory-mapped, or on device memory with no change to its surface.
//!
//! ```
//! use yggdryl_core::typed::{FixedSerie, Scalar};
//! use yggdryl_core::typed::fixedbyte::Int64;
//!
//! let col = FixedSerie::<Int64>::from_options(&[Some(4), None, Some(16), Some(42)]);
//! assert_eq!(col.len(), 4);
//! assert_eq!(col.null_count(), 1);
//! assert_eq!(col.get(0), Some(4));
//! assert_eq!(col.get(1), None);          // the null
//! assert_eq!(col.max().unwrap(), Some(42)); // vectorized reduction over the data buffer
//! ```

mod any;
mod convert;
mod data_type;
mod decimal;
mod decoder;
mod encoder;
mod field;
mod logical;
mod null;
mod parse;
mod reduce;
mod scalar;
mod serie;
mod var_type;

pub mod fixedbit;
pub mod fixedbyte;
pub mod nested;
pub mod varbit;
pub mod varbyte;

pub use any::{AnyScalar, AnySerie};
pub use convert::{convert_column, convert_column_in_place};
pub use data_type::DataType;
pub use decimal::{apply_scale, Decimal};
pub use decoder::Decoder;
pub use encoder::Encoder;
pub use field::{Field, HeaderField};
pub use fixedbyte::{Decimal16, Decimal8, FixedBinary, FixedSizeSerie, FixedUtf8, Float16, F16};
pub use logical::LogicalType;
pub use nested::{
    Column, ColumnField, FromValue, ListField, ListScalar, ListSerie, MapField, MapScalar,
    MapSerie, StructField, StructScalar, StructSerie, ToValue, Value,
};
pub use null::NullSerie;
pub use parse::{FlexibleFromStr, FlexibleToStr};
pub use reduce::Reduce;
pub use scalar::{FixedScalar, Scalar};
pub use serie::{FixedSerie, Serie};
pub use var_type::VarType;
pub use varbyte::{
    Binary, LargeBinary, LargeUtf8, Utf8, VarLenType, VarOffset, VarScalar, VarSerie,
};
