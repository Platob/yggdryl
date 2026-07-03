//! # yggdryl-field
//!
//! The Apache Arrow-centralized field layer for yggdryl, built on top of
//! `yggdryl-dtype`. It defines the **fields** of the model — named, nullable
//! columns of a data type — the second of the three data layers (`yggdryl-dtype`,
//! `yggdryl-field`, `yggdryl-scalar`), each concern its own crate, so the concrete
//! types share one naming convention across the layers (a `yggdryl_field::Int64Field`
//! names a column of the `yggdryl_dtype::Int64Type` type).
//!
//! The layer is two traits plus a factory, each re-exported at the crate root:
//!
//! - The **untyped base** [`Field`] — the FFI-facing descriptor pairing a name, a
//!   data type and a nullability flag.
//! - The **typed** [`TypedField`], generic over the data type `DT` and its native
//!   Rust type `T` — the field's values have native representation `T`.
//! - The **factory** [`FieldFactory`] — a typed data type builds its field
//!   ([`Int64Type.field("id", false)`](FieldFactory::field) → [`Int64Field`]).
//!
//! Concrete fields live in per-family modules mirroring `yggdryl-dtype` — the
//! [`integer`] module holds every signed and unsigned integer field, and the
//! [`binary`], [`null`], [`union`], [`optional`], [`serie`], [`map`] and
//! [`struct`](r#struct) modules the rest. Add more following the rules in
//! `CLAUDE.md`.
//!
//! Every field converts to and from the [`arrow_schema::Field`] it mirrors
//! (`to_arrow` / `from_arrow`, the Arrow factory). The `arrow-schema` subset crate
//! and the `yggdryl-dtype` layer are re-exported so downstream code uses the exact
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
mod field_factory;
mod typed_field;

pub use field::Field;
pub use field_factory::FieldFactory;
pub use typed_field::TypedField;

pub mod binary;
pub mod integer;
pub mod map;
pub mod null;
pub mod optional;
pub mod serie;
pub mod r#struct;
pub mod union;

pub use binary::BinaryField;
pub use map::MapField;
pub use null::NullField;
pub use optional::OptionalField;
pub use r#struct::StructField;
pub use serie::SerieField;
pub use union::UnionField;

pub use integer::{
    Int16Field, Int32Field, Int64Field, Int8Field, UInt16Field, UInt32Field, UInt64Field,
    UInt8Field,
};
