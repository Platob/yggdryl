//! The calendar arithmetic and the native-spelling decision behind TOML's
//! date-time projection.

use crate::{TimeUnit, Timezone, Value};

use super::{
    SECONDS_PER_DAY, civil_from_days, days_from_civil, envelope_payload_depth, is_enveloped,
    native_datetime, split_count,
};

mod calendar {
    use super::{civil_from_days, days_from_civil};

    #[test]
    fn every_day_toml_can_spell_survives_the_conversion_both_ways() {
        // 0000-01-01 and 9999-12-31 are the first and last days TOML has
        // syntax for, and walking all of them proves the pair is an exact
        // inverse rather than proving it for a handful of chosen days.
        let first = days_from_civil(toml::value::Date {
            year: 0,
            month: 1,
            day: 1,
        });
        let last = days_from_civil(toml::value::Date {
            year: 9_999,
            month: 12,
            day: 31,
        });
        assert_eq!(first, -719_528);
        assert_eq!(last, 2_932_896);

        for days in first..=last {
            let date = civil_from_days(days).expect("a day inside TOML's year range");
            assert_eq!(days_from_civil(date), days, "{days}");
        }
    }

    #[test]
    fn a_day_outside_tomls_four_digit_year_has_no_civil_date() {
        assert!(civil_from_days(-719_529).is_none());
        assert!(civil_from_days(2_932_897).is_none());
        assert!(civil_from_days(i64::MAX / 86_400).is_none());
    }

    #[test]
    fn the_epoch_and_the_leap_days_around_it_land_on_the_right_dates() {
        for (days, year, month, day) in [
            (0, 1970, 1, 1),
            (-1, 1969, 12, 31),
            (19_675, 2023, 11, 14),
            (3_433, 1979, 5, 27),
            // 2000 is a leap year and 1900 is not, which is where a naive
            // four-year rule goes wrong.
            (11_016, 2000, 2, 29),
            (-25_508, 1900, 3, 1),
        ] {
            let date = civil_from_days(days).expect("a representable day");
            assert_eq!((date.year, date.month, date.day), (year, month, day));
        }
    }
}

mod splitting {
    use super::{TimeUnit, split_count};

    #[test]
    fn a_count_splits_into_whole_seconds_and_the_nanoseconds_after_them() {
        assert_eq!(split_count(90, TimeUnit::Second), Some((90, 0)));
        assert_eq!(
            split_count(1_500, TimeUnit::Millisecond),
            Some((1, 500_000_000))
        );
        assert_eq!(
            split_count(1_000_001, TimeUnit::Microsecond),
            Some((1, 1_000))
        );
        assert_eq!(split_count(123, TimeUnit::Nanosecond), Some((0, 123)));
    }

    #[test]
    fn a_count_before_the_epoch_keeps_a_positive_fraction() {
        // Half a second before the epoch is second -1 plus 500 milliseconds,
        // not second 0 minus 500, because a clock reading has no sign.
        assert_eq!(
            split_count(-500, TimeUnit::Millisecond),
            Some((-1, 500_000_000))
        );
        assert_eq!(split_count(-1, TimeUnit::Second), Some((-1, 0)));
    }

    #[test]
    fn a_calendar_interval_has_no_reading_on_a_clock() {
        for unit in [
            TimeUnit::YearMonth,
            TimeUnit::DayTime,
            TimeUnit::MonthDayNano,
        ] {
            assert_eq!(split_count(1, unit), None);
        }
    }
}

mod spelling {
    use super::{SECONDS_PER_DAY, TimeUnit, Timezone, Value, native_datetime};

    /// Return the TOML spelling of a value, or the empty string when it has none.
    fn spelled(value: &Value) -> String {
        native_datetime(value)
            .map(|datetime| datetime.to_string())
            .unwrap_or_default()
    }

    #[test]
    fn each_temporal_takes_the_toml_form_that_matches_its_shape() {
        assert_eq!(spelled(&Value::date(3_433)), "1979-05-27");
        assert_eq!(spelled(&Value::time(27_120, TimeUnit::Second)), "07:32:00");
        assert_eq!(
            spelled(&Value::timestamp_in(296_638_320, TimeUnit::Second, None)),
            "1979-05-27T07:32:00"
        );
        assert_eq!(
            spelled(&Value::timestamp_in(
                296_638_320,
                TimeUnit::Second,
                Some(Timezone::UTC)
            )),
            "1979-05-27T07:32:00Z"
        );
        assert_eq!(
            spelled(&Value::timestamp(296_638_320, TimeUnit::Second, Some("-07:00")).unwrap()),
            "1979-05-27T00:32:00-07:00"
        );
    }

    #[test]
    fn a_fraction_is_written_to_the_shortest_exact_precision() {
        assert_eq!(
            spelled(&Value::time(27_120_100, TimeUnit::Millisecond)),
            "07:32:00.1"
        );
        assert_eq!(
            spelled(&Value::time(27_120_123_456, TimeUnit::Microsecond)),
            "07:32:00.123456"
        );
        assert_eq!(
            spelled(&Value::time(27_120_123_456_789, TimeUnit::Nanosecond)),
            "07:32:00.123456789"
        );
    }

    #[test]
    fn a_temporal_toml_cannot_hold_has_no_spelling_at_all() {
        // A zone that names a place cannot be spelled as an offset without
        // throwing the name away.
        assert!(
            native_datetime(&Value::timestamp(0, TimeUnit::Second, Some("Europe/Paris")).unwrap())
                .is_none()
        );
        // A year outside four digits has no date syntax.
        assert!(native_datetime(&Value::date(2_932_897)).is_none());
        assert!(native_datetime(&Value::timestamp_in(i64::MAX, TimeUnit::Second, None)).is_none());
        // A time of day is a reading inside one day.
        assert!(native_datetime(&Value::time(SECONDS_PER_DAY, TimeUnit::Second)).is_none());
        assert!(native_datetime(&Value::time(-1, TimeUnit::Second)).is_none());
        // Elapsed time is not a reading on a calendar at all.
        assert!(native_datetime(&Value::duration(90, TimeUnit::Second)).is_none());
    }
}

mod envelopes {
    use super::{TimeUnit, Timezone, Value, envelope_payload_depth, is_enveloped};

    #[test]
    fn a_value_toml_spells_natively_never_takes_an_envelope() {
        for value in [
            Value::Bool(true),
            Value::I64(1),
            Value::from(7_i8),
            Value::from(7_u32),
            Value::from(1.5),
            Value::from(1.5_f32),
            Value::from("text"),
            Value::from_sequence([Value::I64(1)]),
            Value::from_mapping([(Value::from("key"), Value::I64(1))]).unwrap(),
            Value::date(3_433),
            Value::time(27_120, TimeUnit::Second),
            Value::timestamp_in(0, TimeUnit::Second, Some(Timezone::UTC)),
            Value::datetime(0, TimeUnit::Second),
            // A named zone has no native TOML offset, and a duration has no
            // TOML syntax at all, but both have their classic ISO strings.
            Value::timestamp(0, TimeUnit::Second, Some("Europe/Paris")).unwrap(),
            Value::duration(90, TimeUnit::Second),
        ] {
            assert!(!is_enveloped(&value), "{value:?}");
        }
    }

    #[test]
    fn a_value_toml_has_no_syntax_for_always_takes_one() {
        for value in [
            Value::Null,
            Value::U64(1),
            Value::I128(1),
            Value::U128(1),
            Value::from(vec![0_u8]),
            Value::decimal(1_050, 2),
            // An interval-layout duration has no classic ISO spelling either.
            Value::duration(1, TimeUnit::YearMonth),
            Value::from_mapping([(Value::I64(1), Value::I64(2))]).unwrap(),
            Value::date(2_932_897),
        ] {
            assert!(is_enveloped(&value), "{value:?}");
        }
    }

    #[test]
    fn only_an_array_payload_costs_a_container_of_its_own() {
        for value in [
            Value::decimal(1_050, 2),
            // A time past its day and an interval-layout duration are the
            // readings that still envelope, and their payloads are arrays.
            Value::time(90_000, TimeUnit::Second),
            Value::duration(1, TimeUnit::YearMonth),
        ] {
            assert_eq!(envelope_payload_depth(&value), 1, "{value:?}");
        }
        for value in [Value::Null, Value::from(vec![0_u8]), Value::date(1)] {
            assert_eq!(envelope_payload_depth(&value), 0, "{value:?}");
        }
    }
}
