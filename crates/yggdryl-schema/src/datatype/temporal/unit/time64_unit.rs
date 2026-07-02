//! The marker subtrait for 64-bit time units.

use crate::{AnyTime64Unit, Microsecond, Nanosecond, TimeUnit};

/// A [`TimeUnit`] a 64-bit time of day can hold — microsecond or nanosecond,
/// per the Arrow columnar spec — unlocking [`Time64Type<U>`](crate::Time64Type).
///
/// ```
/// use yggdryl_schema::{Nanosecond, Time, Time64Type};
///
/// let time = Time64Type::from_parts(Nanosecond);
/// assert_eq!(time.to_string(), "time64(ns)");
/// ```
pub trait Time64Unit: TimeUnit {}

impl Time64Unit for Microsecond {}
impl Time64Unit for Nanosecond {}
impl Time64Unit for AnyTime64Unit {}
