//! The schema field: the abstract `Field` base and its generic
//! implementation.

mod error;
mod fields;
// The module is named for its abstract base trait, per the
// one-file-per-type rule.
#[allow(clippy::module_inception)]
mod field;
mod typed_field;

pub use error::FieldError;
pub use field::Field;
pub use fields::{
    AnyField, BinaryField, BooleanField, Date32Field, Date64Field, Decimal128Field,
    Decimal256Field, DurationField, FixedSizeBinaryField, Float32Field, Float64Field, Int16Field,
    Int32Field, Int64Field, Int8Field, LargeBinaryField, LargeListField, LargeUtf8Field, ListField,
    MapField, StructField, Time32Field, Time64Field, TimestampField, UInt16Field, UInt32Field,
    UInt64Field, UInt8Field, Utf8Field,
};
pub use typed_field::{TypedField, TypedFieldRef};
