use crate::{TimeUnit, Timezone, Value};

use super::{civil_from_days, days_from_civil, native_datetime, split_count};

#[test]
fn calendar_conversion_is_exact_across_tomls_range() {
    let first = days_from_civil(0, 1, 1);
    let last = days_from_civil(9_999, 12, 31);
    assert_eq!(first, -719_528);
    assert_eq!(last, 2_932_896);
    for days in first..=last {
        let (year, month, day) = civil_from_days(days);
        assert_eq!(days_from_civil(year, month, day), days);
    }
}

#[test]
fn negative_counts_keep_a_positive_subsecond_fraction() {
    assert_eq!(
        split_count(-500, TimeUnit::Millisecond),
        Some((-1, 500_000_000))
    );
}

#[test]
fn temporal_values_use_native_toml_when_the_shape_fits() {
    let date = Value::date32(3_433);
    let time = Value::time64(27_120_123_456, TimeUnit::Microsecond, Timezone::NAIVE).unwrap();
    let datetime = Value::datetime64(296_638_320, TimeUnit::Second, Timezone::UTC).unwrap();

    assert_eq!(native_datetime(&date).unwrap().to_string(), "1979-05-27");
    assert_eq!(
        native_datetime(&time).unwrap().to_string(),
        "07:32:00.123456"
    );
    assert_eq!(
        native_datetime(&datetime).unwrap().to_string(),
        "1979-05-27T07:32:00Z"
    );
}

#[test]
fn named_zones_and_durations_have_no_native_toml_scalar() {
    let datetime = Value::datetime64(0, TimeUnit::Second, "Europe/Paris".parse().unwrap()).unwrap();
    let duration = Value::duration64(90, TimeUnit::Second).unwrap();
    assert!(native_datetime(&datetime).is_none());
    assert!(native_datetime(&duration).is_none());
}
