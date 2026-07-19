//! `map` — the **map family**: the key→value map nested carrier.
//!
//! [`MapField`] is the schema (name + nullability + metadata + the child key / value fields +
//! `keys_sorted`), [`MapScalar`] is one map element (its entries materialized as owned parallel
//! [`Value`](super::Value)s), and [`MapSerie`] is the map column itself — an `i32`-offsets buffer
//! over a two-column [`StructSerie`](crate::typed::StructSerie) of flattened key + value entries,
//! with graph discovery ([`MapSerie::keys`] / [`MapSerie::values`]) and deep, in-place mutation of an
//! entry series ([`MapSerie::keys_mut`] / [`MapSerie::values_mut`]). It implements
//! [`Scalar`](crate::typed::Scalar) / [`Serie`](crate::typed::Serie), so a map is itself a column and
//! nests inside a struct or a list. Map **keys are non-nullable** (an Arrow constraint), validated on
//! build with a guided error.

mod field;
mod scalar;
mod serie;

pub use field::MapField;
pub use scalar::MapScalar;
pub use serie::MapSerie;
