//! The time-column predicate an integration test cannot reach.
//!
//! `accepts_time` is the crate-private rule the column reader branches on, so
//! the two agreeing is the contract. Everything a caller can observe lives in
//! `tests/hashing/txhash_arrow.rs`.

use super::{accepts_time, unix_array};
use crate::{DataType, Field, TimeUnit, Timezone};

const UNIT: TimeUnit = TimeUnit::Microsecond;

#[test]
fn accepts_time_agrees_with_the_column_reader() {
    let zoned = |unit| DataType::datetime64(unit, Timezone::UTC).unwrap();
    for dtype in [
        DataType::Int8,
        DataType::Int16,
        DataType::Int32,
        DataType::Int64,
        DataType::UInt8,
        DataType::UInt16,
        DataType::UInt32,
        DataType::UInt64,
        DataType::date32(),
        DataType::date64(),
        zoned(TimeUnit::Second),
        zoned(TimeUnit::Nanosecond),
        DataType::datetime64(TimeUnit::Millisecond, Timezone::NAIVE).unwrap(),
        DataType::utf8(),
        DataType::Float64,
        DataType::Boolean,
        DataType::time64(TimeUnit::Microsecond).unwrap(),
        DataType::duration64(TimeUnit::Second).unwrap(),
    ] {
        let arrow = Field::new("x", dtype.clone(), true)
            .into_arrow_field()
            .unwrap();
        let empty = arrow_array::new_empty_array(arrow.data_type());
        assert_eq!(
            accepts_time(&dtype),
            unix_array(empty.as_ref(), UNIT).is_ok(),
            "{dtype}"
        );
    }
}
