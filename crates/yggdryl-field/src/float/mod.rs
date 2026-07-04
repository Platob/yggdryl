//! Concrete floating-point fields — every Apache Arrow single- and double-precision
//! float.
//!
//! One file per type (`float32.rs`, `float64.rs`), mirroring the
//! [`integer`](crate::integer) module: a float field is the same shape as an
//! integer field — a name paired with a fixed-width float data type — so each reuses
//! the integer family's crate-internal `int_field!` macro, which also wires the
//! [`FieldFactory`](crate::FieldFactory) so the data type builds its field
//! (`Float64Type.field("weight", false)`).

mod float16;
mod float32;
mod float64;

pub use float16::Float16Field;
pub use float32::Float32Field;
pub use float64::Float64Field;
