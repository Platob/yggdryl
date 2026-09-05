use yggdryl::types::temporal;
use yggdryl::{DataType, TimeUnit, Timezone};

use super::typed::assert_typed_marker;

#[test]
fn temporal_markers_cover_clock_calendar_and_interval_types() {
    assert_typed_marker::<temporal::DateTime64Type>(DataType::DateTime64 {
        unit: TimeUnit::Nanosecond,
        timezone: Timezone::UTC,
    });
    assert_typed_marker::<temporal::Date32Type>(DataType::Date32);
    assert_typed_marker::<temporal::Date64Type>(DataType::Date64);
    assert_typed_marker::<temporal::Time32Type>(DataType::time32(TimeUnit::Second).unwrap());
    assert_typed_marker::<temporal::Time64Type>(DataType::time64(TimeUnit::Microsecond).unwrap());
    assert_typed_marker::<temporal::Duration32Type>(DataType::Duration32(TimeUnit::Millisecond));
    assert_typed_marker::<temporal::Duration64Type>(DataType::Duration64(TimeUnit::Millisecond));
    assert_typed_marker::<temporal::IntervalType>(DataType::Interval(TimeUnit::MonthDayNano));
}
