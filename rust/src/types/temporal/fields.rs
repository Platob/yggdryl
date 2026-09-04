//! Temporal and interval field markers.

use crate::TypedField;
use crate::types::typed::define_field_types;

define_field_types!(
    DateTime64Type,
    "datetime64",
    crate::DataType::DateTime64 { .. }
);
define_field_types!(Date32, "date32", crate::DataType::Date32);
define_field_types!(Date64, "date64", crate::DataType::Date64);
define_field_types!(Time32, "time32", crate::DataType::Time32(_));
define_field_types!(Time64, "time64", crate::DataType::Time64(_));
define_field_types!(Duration32, "duration32", crate::DataType::Duration32(_));
define_field_types!(Duration64, "duration64", crate::DataType::Duration64(_));
define_field_types!(Interval, "interval", crate::DataType::Interval(_));

/// A DateTime64-typed field.
pub type DateTime64Field = TypedField<DateTime64Type>;
/// A Date32-typed field.
pub type Date32Field = TypedField<Date32>;
/// A Date64-typed field.
pub type Date64Field = TypedField<Date64>;
/// A Time32-typed field.
pub type Time32Field = TypedField<Time32>;
/// A Time64-typed field.
pub type Time64Field = TypedField<Time64>;
/// A duration-typed field.
pub type Duration32Field = TypedField<Duration32>;
/// A Duration64-typed field.
pub type Duration64Field = TypedField<Duration64>;
/// An interval-typed field.
pub type IntervalField = TypedField<Interval>;
