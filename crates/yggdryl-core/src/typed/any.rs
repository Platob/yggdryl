//! `Any` — the **erased type surface** that wraps every concrete type: [`AnySerie`] (a whole erased
//! column) and [`AnyScalar`] (one erased element).
//!
//! The typed layer is precise per element type ([`FixedSerie<Int64>`](crate::typed::FixedSerie), a
//! [`VarSerie<Utf8>`](crate::typed::VarSerie), …); `Any` is the runtime "holds any type" view over
//! all of them. It is not a new carrier — it is a pair of **aliases** onto the already-erased
//! [`Column`] / [`Value`] keystones, named so callers can read and pass the "any type" explicitly.
//! The matching [`DataTypeId::Any`](crate::datatype_id::DataTypeId::Any) tags it.
//!
//! Because `Any` **is** the erased [`Column`] / [`Value`], it is extended for free every time those
//! are: **whenever a new type is added, its [`Column`] / [`Value`] / [`ColumnField`](crate::typed::ColumnField)
//! arms MUST be added too**, and `Any` then wraps it automatically. There is deliberately no
//! separate registry to keep in sync — the erased enums are the single source of truth.

use crate::typed::nested::{Column, Value};

/// The **erased column** that wraps every concrete typed column — the runtime "holds any type"
/// carrier. An alias of [`Column`]: a heterogeneous column set (a struct's children) is a set of
/// `AnySerie`, and [`From`] erases any concrete carrier into it (`AnySerie::from(concrete_serie)` /
/// [`Serie::into_any`](crate::typed::Serie::into_any)).
pub type AnySerie = Column;

/// The **erased element** of any column — the runtime "holds any value" scalar. An alias of
/// [`Value`]: [`get_any_value_at`](crate::typed::Serie::get_any_value_at) /
/// [`get_any_scalar_at`](crate::typed::Serie::get_any_scalar_at) return one, and
/// [`set_any_scalar_at`](crate::typed::Serie::set_any_scalar_at) consumes one.
pub type AnyScalar = Value;
