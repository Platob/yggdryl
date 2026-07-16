//! `io` — the Apache Arrow-backed physical I/O layer of the core.
//!
//! This module owns yggdryl's *addressing and physical* representation. Its first concern
//! is the URI family: [`Uri`] (a generic RFC 3986 URI that doubles as a POSIX-normalized
//! filesystem path), [`Url`] (a URI guaranteed to carry a scheme), and [`Authority`] (the
//! `[user[:password]@]host[:port]` component), with [`UriError`] carrying the guided parse
//! failures and [`default_port`] mapping a well-known scheme to its port.
//!
//! Its second concern is the **byte-I/O** family: the [`IOBase`] / [`IOCursor`] / [`IOSlice`]
//! traits define positioned (`pread`/`pwrite`) and cursor (`read`/`write` + [`Whence`] seek)
//! byte access, with [`IoError`] carrying their guided failures.
//!
//! Its third concern is the **typed** serialization layer. Its family-agnostic contracts live
//! here at the root — [`DataType`] / [`TypedDataType`], [`FieldType`], [`ScalarType`],
//! [`SerieType`], [`BufferType`], and the [`DataTypeCategory`] every `is_*` predicate drills
//! on — so both concrete families extend the *same* roots. [`fixed`] holds the fixed-width
//! primitives (each a `DataType` / `Field` / `Scalar` / `Serie` / `Buffer` over a native `T`)
//! and their `Fixed*` sub-traits; [`var`] holds the variable-length ones (UTF-8, binary) and
//! their `Var*` sub-traits. The project's byte buffer [`Bytes`] is `fixed::U8Buffer`
//! (`Buffer<u8>`), the implementor the bindings hold — Arrow-backed, with zero-copy
//! reads/slices and copy-on-write writes. [`Headers`] is a byte-string key/value map
//! (case-insensitive, multi-value, HTTP-flavored) built on the same byte codec.
//!
//! Per the crate rules, Arrow (`arrow-buffer`) is the physical layer here — its types stay
//! an implementation detail and never appear in a public signature; each public type lives
//! in its own file and is mirrored, thinly, in the Python and Node extensions.

mod any_field;
mod any_scalar;
mod any_serie;
mod authority;
mod bitmap;
mod buffer_type;
mod category;
mod converter;
mod data_type;
mod data_type_id;
mod field_carrier;
mod field_type;
pub mod fixed;
mod headers;
mod io_base;
mod io_cursor;
mod io_error;
mod io_slice;
pub mod nested;
mod node_path;
mod node_ref;
mod numeric_serie;
mod percent;
mod scalar_type;
mod scheme;
mod serie_type;
mod uri;
mod uri_error;
mod url;
pub mod var;
mod whence;

// The family-agnostic, recursive **erased** primitives — an erased column (`AnySerie`, held as a
// `Box<dyn AnySerie>`), its recursive erased field (`AnyField`), and its erased cell (`AnyScalar`) —
// used by the nested types (and the bindings) to carry heterogeneous children.
pub use any_field::AnyField;
pub use any_scalar::AnyScalar;
#[cfg(feature = "arrow")]
pub use any_serie::from_arrow_any_leaf;
pub use any_serie::{boxed, read_any_leaf, AnySerie};
// The central recursive child dispatch spanning leaf + nested columns (a list/struct child can be a
// leaf, a struct, or another list) — mirrors the leaf-only `read_any_leaf` / `from_arrow_any_leaf`.
#[cfg(feature = "arrow")]
pub use nested::from_arrow_any_column;
pub use nested::read_any_column;

pub use authority::Authority;
pub use fixed::Bytes;
pub use headers::Headers;
// The parsed path value type and its guided errors — the addressing form the `get_by_path` resolvers
// on `AnySerie` / `AnyField` / `AnyScalar` walk.
pub use io_base::IOBase;
pub use io_cursor::IOCursor;
pub use io_error::IoError;
pub use io_slice::IOSlice;
pub use node_path::{NodePath, PathError, PathSegment};
pub use scheme::default_port;
pub use uri::Uri;
pub use uri_error::UriError;
pub use url::Url;
pub use whence::Whence;

// The family-agnostic typed contracts, shared by `fixed` and `var`. Each family adds its own
// `Fixed*` / `Var*` sub-traits and concrete implementors; these roots are the single place the
// abstract shape (and the [`DataTypeCategory`] drill-down axis) is defined.
pub use buffer_type::BufferType;
pub use category::DataTypeCategory;
pub use converter::{CastError, Converter, NumericCast};
pub use data_type::{DataType, TypedDataType};
pub use data_type_id::DataTypeId;
pub use field_type::FieldType;
// The numeric-analytics capability (count / sum / mean / min / max) — the stats / time-series seam,
// available exactly on the numeric columns (a `Serie<T>` with `T: NumericCast`).
pub use numeric_serie::NumericSerie;
pub use scalar_type::ScalarType;
pub use serie_type::SerieType;

/// A zero-allocation [`core::fmt::Write`] that streams formatted output straight into a
/// [`Hasher`](core::hash::Hasher).
///
/// The URI value types hash by their canonical string; this lets them feed that string to
/// the hasher a fragment at a time instead of building a `String` first — the same bytes,
/// no allocation. Paired with a `0xff` terminator it reproduces `str`'s own hash, so equal
/// canonical strings still hash equal.
pub(crate) struct HashWrite<'a, H: core::hash::Hasher>(pub(crate) &'a mut H);

impl<H: core::hash::Hasher> core::fmt::Write for HashWrite<'_, H> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.0.write(s.as_bytes());
        Ok(())
    }
}
