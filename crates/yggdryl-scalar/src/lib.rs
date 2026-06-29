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

mod binary;
mod scalar;

pub use binary::Binary;
pub use scalar::Scalar;
