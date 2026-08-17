//! Temporal and interval field markers.

use super::typed::define_field_types;

define_field_types!(Timestamp, "timestamp", crate::DataType::Timestamp(..));
define_field_types!(Date32, "date32", crate::DataType::Date32);
define_field_types!(Date64, "date64", crate::DataType::Date64);
define_field_types!(Time32, "time32", crate::DataType::Time32(_));
define_field_types!(Time64, "time64", crate::DataType::Time64(_));
define_field_types!(Duration, "duration", crate::DataType::Duration(_));
define_field_types!(Interval, "interval", crate::DataType::Interval(_));
