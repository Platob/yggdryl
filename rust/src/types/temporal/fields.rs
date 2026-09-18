//! Temporal and interval field markers.

use crate::TypedField;
use crate::types::typed::define_field_types;

define_field_types!(
    DateTime64Type,
    DateTime64,
    crate::DataType::DateTime64 { .. }
);
define_field_types!(Date32Type, Date32, crate::DataType::Date32);
define_field_types!(Date64Type, Date64, crate::DataType::Date64);
define_field_types!(Time32Type, Time32, crate::DataType::Time32(_));
define_field_types!(Time64Type, Time64, crate::DataType::Time64(_));
define_field_types!(Duration32Type, Duration32, crate::DataType::Duration32(_));
define_field_types!(Duration64Type, Duration64, crate::DataType::Duration64(_));
define_field_types!(IntervalType, Interval, crate::DataType::Interval(_));

/// A DateTime64-typed field.
pub type DateTime64Field = TypedField<DateTime64Type>;
