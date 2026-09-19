//! The temporal scalar: exact widths, the zone every value carries, and
//! the readers that answer for each family.

use yggdryl::{DataType, DataTypeId, Scalar, TemporalFamily, TimeUnit, Timezone, Value};
use yggdryl::{Date32, DateTime64, Interval};

#[test]
fn constructors_reject_illegal_width_unit_combinations() {
    assert!(Scalar::time32(1, TimeUnit::Microsecond, Timezone::NAIVE).is_err());
    assert!(Scalar::time64(1, TimeUnit::Millisecond, Timezone::NAIVE).is_err());
    assert!(Scalar::duration32(1, TimeUnit::DayTime).is_err());
    assert!(Scalar::duration64(1, TimeUnit::DayTime).is_err());
    assert!(Scalar::duration32_in(1, TimeUnit::Second, Timezone::UTC).is_err());
    assert!(Scalar::duration64_in(1, TimeUnit::Second, Timezone::UTC).is_err());
    assert!(Scalar::date32_in(1, TimeUnit::DayTime, Timezone::NAIVE).is_err());
    assert!(Scalar::date64_in(1, TimeUnit::Second, Timezone::NAIVE).is_err());
    // An interval refuses a component its layout has nowhere to put, and a
    // unit that is no layout.
    assert!(Scalar::interval(1, 2, 0, TimeUnit::YearMonth).is_err());
    assert!(Scalar::interval(1, 0, 0, TimeUnit::DayTime).is_err());
    assert!(Scalar::interval(0, 0, 0, TimeUnit::Second).is_err());
}

#[test]
fn time_of_day_refuses_a_zone_its_datatype_would_lose() {
    for error in [
        Scalar::time32(1, TimeUnit::Second, Timezone::UTC).unwrap_err(),
        Scalar::time64(1, TimeUnit::Microsecond, Timezone::UTC).unwrap_err(),
    ] {
        let message = error.to_string();
        assert!(message.contains("timezone"), "{message}");
        assert!(message.contains("no timezone"), "{message}");
    }
}

#[test]
fn every_temporal_carries_a_zone() {
    let values = [
        Scalar::date32(1),
        Scalar::date64(86_400_000),
        Scalar::time32(1, TimeUnit::Second, Timezone::NAIVE).unwrap(),
        Scalar::time64(1, TimeUnit::Microsecond, Timezone::NAIVE).unwrap(),
        Scalar::datetime64(1, TimeUnit::Nanosecond, Timezone::UTC).unwrap(),
        Scalar::duration32(1, TimeUnit::Millisecond).unwrap(),
        Scalar::duration64(1, TimeUnit::Microsecond).unwrap(),
        Scalar::interval(1, 2, 3, TimeUnit::MonthDayNano).unwrap(),
    ];
    assert!(values.iter().all(Scalar::is_temporal));
}

#[test]
fn family_constructors_select_exact_widths() {
    let values = [
        Scalar::from_date(1, TimeUnit::Day, Timezone::NAIVE).unwrap(),
        Scalar::from_date(86_400_000, TimeUnit::Millisecond, Timezone::NAIVE).unwrap(),
        Scalar::from_time(1, TimeUnit::Second, Timezone::NAIVE).unwrap(),
        Scalar::from_time(1, TimeUnit::Nanosecond, Timezone::NAIVE).unwrap(),
        Scalar::from_datetime(1, TimeUnit::Microsecond, Timezone::UTC).unwrap(),
        Scalar::from_duration(i64::from(i32::MAX), TimeUnit::Second, Timezone::NAIVE).unwrap(),
        Scalar::from_duration(i64::from(i32::MAX) + 1, TimeUnit::Second, Timezone::NAIVE).unwrap(),
    ];
    assert!(matches!(values[0], Scalar::Date32(_)));
    assert!(matches!(values[1], Scalar::Date64(_)));
    assert!(matches!(values[2], Scalar::Time32(_)));
    assert!(matches!(values[3], Scalar::Time64(_)));
    assert!(matches!(values[4], Scalar::DateTime64(_)));
    assert!(matches!(values[5], Scalar::Duration32(_)));
    assert!(matches!(values[6], Scalar::Duration64(_)));

    for count in [i64::from(i32::MIN), i64::from(i32::MAX)] {
        assert!(matches!(
            Scalar::from_duration(count, TimeUnit::Nanosecond, Timezone::NAIVE).unwrap(),
            Scalar::Duration32(_)
        ));
    }
    for count in [i64::from(i32::MIN) - 1, i64::from(i32::MAX) + 1] {
        assert!(matches!(
            Scalar::from_duration(count, TimeUnit::Nanosecond, Timezone::NAIVE).unwrap(),
            Scalar::Duration64(_)
        ));
    }
}

#[test]
fn temporal_readers_answer_every_temporal_variant() {
    let interval = Interval::new(1, 2, 3, TimeUnit::MonthDayNano).unwrap();
    let cases = [
        (
            Scalar::date32(1),
            TemporalFamily::Date,
            TimeUnit::Day,
            Timezone::NAIVE,
            1_i64,
        ),
        (
            Scalar::date64(86_400_000),
            TemporalFamily::Date,
            TimeUnit::Millisecond,
            Timezone::NAIVE,
            86_400_000,
        ),
        (
            Scalar::time32(1, TimeUnit::Second, Timezone::NAIVE).unwrap(),
            TemporalFamily::Time,
            TimeUnit::Second,
            Timezone::NAIVE,
            1,
        ),
        (
            Scalar::time64(1, TimeUnit::Microsecond, Timezone::NAIVE).unwrap(),
            TemporalFamily::Time,
            TimeUnit::Microsecond,
            Timezone::NAIVE,
            1,
        ),
        (
            Scalar::datetime64(1, TimeUnit::Nanosecond, Timezone::UTC).unwrap(),
            TemporalFamily::DateTime,
            TimeUnit::Nanosecond,
            Timezone::UTC,
            1,
        ),
        (
            Scalar::duration32(-1, TimeUnit::Millisecond).unwrap(),
            TemporalFamily::Duration,
            TimeUnit::Millisecond,
            Timezone::NAIVE,
            -1,
        ),
        (
            Scalar::duration64(1, TimeUnit::Microsecond).unwrap(),
            TemporalFamily::Duration,
            TimeUnit::Microsecond,
            Timezone::NAIVE,
            1,
        ),
        (
            Scalar::interval(1, 2, 3, TimeUnit::MonthDayNano).unwrap(),
            TemporalFamily::Interval,
            TimeUnit::MonthDayNano,
            Timezone::NAIVE,
            3,
        ),
    ];
    for (value, family, unit, zone, count) in &cases {
        assert!(value.is_temporal(), "{value:?}");
        assert_eq!(value.temporal_family(), Some(*family), "{value:?}");
        assert_eq!(value.temporal_unit(), Some(*unit), "{value:?}");
        assert_eq!(value.temporal_timezone(), Some(*zone), "{value:?}");
        assert_eq!(value.temporal_count(), Some(*count), "{value:?}");
    }
    // An interval restates no count across units; all three of its
    // components are read by matching `Scalar::Interval` directly, and the
    // constructor built exactly the value the struct does.
    assert_eq!(cases[7].0.temporal_count_at(TimeUnit::Nanosecond), None);
    assert!(matches!(cases[7].0, Scalar::Interval(held) if held == interval));
    assert_eq!(cases[0].0.as_date32().map(|(count, ..)| count), Some(1));
    assert_eq!(
        cases[1].0.as_date64().map(|(count, ..)| count),
        Some(86_400_000)
    );

    let number = Scalar::from(1);
    assert!(!number.is_temporal());
    assert_eq!(number.temporal_family(), None);
    assert_eq!(number.temporal_unit(), None);
    assert_eq!(number.temporal_timezone(), None);
    assert_eq!(number.temporal_count(), None);
    assert_eq!(number.temporal_count_at(TimeUnit::Second), None);
}

#[test]
fn a_temporal_leaf_is_its_own_family() {
    // A value's datatype is its family's, with the parameters the value
    // carries.
    let value = Scalar::datetime64(1, TimeUnit::Nanosecond, Timezone::UTC).unwrap();
    let leaf = <DateTime64 as Value>::from_scalar(&value).copied().unwrap();
    assert_eq!(leaf.dtype().unwrap().id(), DataTypeId::DateTime64);
    assert_eq!(
        leaf.dtype().unwrap(),
        DataType::datetime64(TimeUnit::Nanosecond, Timezone::UTC).unwrap()
    );
    assert_eq!(<DateTime64 as Value>::into_scalar(leaf), value);
    assert!(<Date32 as Value>::from_scalar(&value).is_none());

    let day = <Date32 as Value>::from_scalar(&Scalar::date32(1))
        .copied()
        .unwrap();
    assert_eq!(day.dtype().unwrap(), DataType::date32());

    let interval = Interval::new(1, 0, 0, TimeUnit::YearMonth).unwrap();
    let held = <Interval as Value>::into_scalar(interval);
    assert_eq!(<Interval as Value>::from_scalar(&held), Some(&interval));
    assert_eq!(interval.dtype().unwrap().id(), DataTypeId::Interval);
    assert_eq!(
        interval.dtype().unwrap(),
        DataType::interval(TimeUnit::YearMonth).unwrap()
    );
}

#[test]
fn family_constructors_reject_invalid_parts() {
    assert!(Scalar::from_date(1, TimeUnit::Second, Timezone::NAIVE).is_err());
    assert!(Scalar::from_date(i64::MAX, TimeUnit::Day, Timezone::NAIVE).is_err());
    assert!(Scalar::from_time(1, TimeUnit::Day, Timezone::NAIVE).is_err());
    assert!(Scalar::from_time(1, TimeUnit::Second, Timezone::UTC).is_err());
    assert!(Scalar::from_datetime(1, TimeUnit::Day, Timezone::NAIVE).is_err());
    assert!(Scalar::from_duration(1, TimeUnit::DayTime, Timezone::NAIVE).is_err());
    assert!(Scalar::from_duration(1, TimeUnit::Second, Timezone::UTC).is_err());
}
