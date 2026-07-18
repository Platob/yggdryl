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
//! - [`Reduce`] — the numeric aggregations (`sum`/`min`/`max`/`mean`) a type routes to the source's
//!   [`Aggregate`](crate::io::memory::Aggregate) kernels.
//! - [`Scalar`] — an indexed, possibly-null typed value; [`Serie`] (`: Scalar`) refines it into a
//!   **column** — many elements over a data buffer plus an optional validity bit buffer.
//! - [`Field`] — a column's metadata (name, type, nullable), carried in a [`Headers`](crate::headers::Headers).
//!
//! The concrete implementations are split by **length × granularity**: [`fixedbyte`] (integers,
//! floats), [`fixedbit`] (booleans), and the reserved [`varbyte`] / [`varbit`] (strings, binary,
//! bit-lists). Every fixed type is one [`fixed_numeric!`](fixedbyte::fixed_numeric)-style line, so a
//! new type is added in a single rule. The concrete value carriers [`FixedScalar`] / [`FixedSerie`]
//! are generic over the type **and** the backing [`IOBase`], so a column is in-heap, memory-mapped,
//! or on device memory with no change to its surface.
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

mod data_type;
mod decoder;
mod encoder;
mod field;
mod reduce;
mod scalar;
mod serie;

pub mod fixedbit;
pub mod fixedbyte;
pub mod varbit;
pub mod varbyte;

pub use data_type::DataType;
pub use decoder::Decoder;
pub use encoder::Encoder;
pub use field::{Field, HeaderField};
pub use reduce::Reduce;
pub use scalar::{FixedScalar, Scalar};
pub use serie::{FixedSerie, Serie};
