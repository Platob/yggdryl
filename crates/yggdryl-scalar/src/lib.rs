//! # yggdryl-scalar
//!
//! Arrow-centric scalar **values**. [`Scalar`] is the trait every value implements
//! — it knows its [`dtype`](Scalar::dtype) and round-trips through its raw byte
//! form ([`to_bytes`](Scalar::to_bytes) / [`from_bytes`](Scalar::from_bytes)).
//! [`Binary`] is the byte-backed value carrying any binary
//! [`DataType`](yggdryl_schema::DataType).
//!
//! New value types land here one module per concern, following the rules in
//! `CLAUDE.md`.

/// Emits a `log` event when the `log` feature is enabled, and expands to nothing
/// otherwise. Shared by every submodule via `crate::log_event!`.
macro_rules! log_event {
    ($level:ident, $($arg:tt)+) => {{
        #[cfg(feature = "log")]
        log::$level!($($arg)+);
    }};
}
pub(crate) use log_event;

mod binary;
mod scalar;

pub use binary::Binary;
pub use scalar::Scalar;
