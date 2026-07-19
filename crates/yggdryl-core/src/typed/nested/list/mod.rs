//! `list` — the **list family**: the variable-length list nested carrier.
//!
//! [`ListField`] is the schema (name + nullability + metadata + the child **item** field),
//! [`ListScalar`] is one list element (its children materialized as owned [`Value`](super::Value)s),
//! and [`ListSerie`] is the list column itself — an `i32`-offsets buffer over a flattened child
//! [`Column`](super::Column), with graph discovery ([`ListSerie::values`]) and deep, in-place
//! mutation of the child series ([`ListSerie::values_mut`]). It implements
//! [`Scalar`](crate::typed::Scalar) / [`Serie`](crate::typed::Serie), so a list is itself a column
//! and nests inside a struct (or another list).

mod field;
mod scalar;
mod serie;

pub use field::ListField;
pub use scalar::ListScalar;
pub use serie::ListSerie;
