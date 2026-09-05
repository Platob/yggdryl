//! Temporal and interval field markers.

use crate::TypedField;
use crate::types::typed::define_field_types;

define_field_types!(
    DateTime64Type,
    "datetime64",
    crate::DataType::DateTime64 { .. }
);
define_field_types!(Date32Type, "date32", crate::DataType::Date32);
define_field_types!(Date64Type, "date64", crate::DataType::Date64);
define_field_types!(Time32Type, "time32", crate::DataType::Time32(_));
define_field_types!(Time64Type, "time64", crate::DataType::Time64(_));
define_field_types!(Duration32Type, "duration32", crate::DataType::Duration32(_));
define_field_types!(Duration64Type, "duration64", crate::DataType::Duration64(_));
define_field_types!(IntervalType, "interval", crate::DataType::Interval(_));

/// A DateTime64-typed field.
pub type DateTime64Field = TypedField<DateTime64Type>;
/// A Date32-typed field.
pub type Date32Field = TypedField<Date32Type>;
/// A Date64-typed field.
pub type Date64Field = TypedField<Date64Type>;
/// A Time32-typed field.
pub type Time32Field = TypedField<Time32Type>;
/// A Time64-typed field.
pub type Time64Field = TypedField<Time64Type>;
/// A duration-typed field.
pub type Duration32Field = TypedField<Duration32Type>;
/// A Duration64-typed field.
pub type Duration64Field = TypedField<Duration64Type>;
/// An interval-typed field.
pub type IntervalField = TypedField<IntervalType>;
