//! The marker subtrait for 32-bit time units.

use crate::{AnyTime32Unit, Millisecond, Second, TimeUnit};

/// A [`TimeUnit`] a 32-bit time of day can hold — second or millisecond, per
/// the Arrow columnar spec — unlocking [`Time32<U>`](crate::Time32).
///
/// ```
/// use yggdryl_schema::{Second, Time, Time32};
///
/// let time = Time32::from_parts(Second);
/// assert_eq!(time.to_string(), "time32(s)");
/// ```
pub trait Time32Unit: TimeUnit {}

impl Time32Unit for Second {}
impl Time32Unit for Millisecond {}
impl Time32Unit for AnyTime32Unit {}
