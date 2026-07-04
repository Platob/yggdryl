//! Concrete floating-point data types — every Apache Arrow single- and
//! double-precision float.
//!
//! One file per type (`float16.rs`, `float32.rs`, `float64.rs`), mirroring the
//! [`integer`](crate::integer) module: a float is the same shape as an integer — a
//! fixed-width [`Primitive`](crate::Primitive) with a little-endian byte codec — so
//! each reuses the integer family's crate-internal `int_data_type!` macro
//! ([`half::f16`], `f32` and `f64` all carry the same `to_le_bytes` /
//! `from_le_bytes` surface). The matching fields and scalars live in `yggdryl-field`
//! and `yggdryl-scalar`, under the `Float64Field` / `Float64Scalar` naming convention.

mod float16;
mod float32;
mod float64;

pub use float16::Float16Type;
pub use float32::Float32Type;
pub use float64::Float64Type;
