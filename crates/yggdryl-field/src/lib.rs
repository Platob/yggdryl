//! # yggdryl-field
//!
//! The Apache Arrow-centralized field layer for yggdryl, built on top of
//! `yggdryl-dtype`. It defines the **fields** of the model — named, nullable
//! columns of a data type — the second of the three data layers (`yggdryl-dtype`,
//! `yggdryl-field`, `yggdryl-scalar`), each concern its own crate, so the concrete
//! types share one bare name across the layers (a `yggdryl_field::Int64` names a
//! column of the `yggdryl_dtype::Int64` type).
//!
//! The layer is two traits, each re-exported at the crate root:
//!
//! - The **untyped base** [`RawField`] — the FFI-facing descriptor pairing a name,
//!   a data type and a nullability flag.
//! - The **typed** [`Field`], whose data type is a `DataType<T>` — the field's
//!   values have native Rust representation `T`.
//!
//! Concrete fields live in per-family modules mirroring `yggdryl-dtype` — the
//! [`integer`] module holds every signed and unsigned integer field, and the
//! [`binary`], [`null`], [`union`], [`optional`], [`list`], [`map`] and
//! [`struct`](r#struct) modules the rest. Add more following the rules in
//! `CLAUDE.md`.
//!
//! Every field converts to and from the [`arrow_schema::Field`] it mirrors
//! (`to_arrow` / `from_arrow`). The `arrow-schema` subset crate and the
//! `yggdryl-dtype` layer are re-exported so downstream code uses the exact
//! versions this crate was built against. Skipped inputs (dropped Arrow field
//! metadata) are logged behind the off-by-default `log` cargo feature, mirroring
//! `yggdryl-core`.

/// Emits a `log` event when the `log` feature is enabled, and expands to nothing
/// otherwise (so the crate stays logging-free by default and pays no runtime
/// cost). Submodules reach it via `crate::log_event!` thanks to the re-export
/// below.
macro_rules! log_event {
    ($level:ident, $($arg:tt)+) => {{
        #[cfg(feature = "log")]
        ::log::$level!($($arg)+);
    }};
}
pub(crate) use log_event;

/// The Apache Arrow schema layer (`arrow-schema`), re-exported so downstream code
/// and the `to_arrow` / `from_arrow` surface share one version.
pub use arrow_schema;
/// The yggdryl data-type layer (`yggdryl-dtype`), re-exported so downstream code
/// reaches the data types (and [`DataError`](yggdryl_dtype::DataError)) at the
/// exact version this crate was built against.
pub use yggdryl_dtype;

mod field;
mod raw_field;

pub use field::Field;
pub use raw_field::RawField;

pub mod binary;
pub mod integer;
pub mod list;
pub mod map;
pub mod null;
pub mod optional;
pub mod r#struct;
pub mod union;

pub use binary::Binary;
pub use list::List;
pub use map::Map;
pub use null::Null;
pub use optional::Optional;
pub use r#struct::Struct;
pub use union::Union;

pub use integer::{Int16, Int32, Int64, Int8, UInt16, UInt32, UInt64, UInt8};
