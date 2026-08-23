use yggdryl::field::temporal;
use yggdryl::{DataType, TimeUnit, Timezone};

use crate::typed::assert_typed_marker;

#[test]
fn temporal_markers_cover_clock_calendar_and_interval_types() {
    assert_typed_marker::<temporal::Timestamp>(DataType::Timestamp(
        TimeUnit::Nanosecond,
        Some(Timezone::UTC),
    ));
    assert_typed_marker::<temporal::Date32>(DataType::Date32);
    assert_typed_marker::<temporal::Date64>(DataType::Date64);
    assert_typed_marker::<temporal::Time32>(DataType::time32(TimeUnit::Second).unwrap());
    assert_typed_marker::<temporal::Time64>(DataType::time64(TimeUnit::Microsecond).unwrap());
    assert_typed_marker::<temporal::Duration32>(DataType::Duration32(TimeUnit::Millisecond));
    assert_typed_marker::<temporal::Duration64>(DataType::Duration64(TimeUnit::Millisecond));
    assert_typed_marker::<temporal::Interval>(DataType::Interval(TimeUnit::MonthDayNano));
}
