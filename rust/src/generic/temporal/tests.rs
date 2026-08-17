//! The four temporals, the datatype each one names, and how they compare.

use crate::{DataType, TimeUnit, Timezone, Value};

mod construction {
    use super::{TimeUnit, Timezone, Value};

    #[test]
    fn every_temporal_round_trips_through_its_accessor() {
        let at = Value::timestamp(1_700_000_000_000, TimeUnit::Millisecond, None).unwrap();
        assert_eq!(
            at.as_timestamp(),
            Some((1_700_000_000_000, TimeUnit::Millisecond, None))
        );

        let zoned =
            Value::timestamp(1_700_000_000, TimeUnit::Second, Some("Europe/Paris")).unwrap();
        assert_eq!(
            zoned.as_timestamp(),
            Some((1_700_000_000, TimeUnit::Second, Some("Europe/Paris")))
        );

        assert_eq!(Value::date(19_723).as_date(), Some(19_723));
        assert_eq!(
            Value::time(45_296_000_000, TimeUnit::Microsecond).as_time(),
            Some((45_296_000_000, TimeUnit::Microsecond))
        );
        assert_eq!(
            Value::duration(90, TimeUnit::Second).as_duration(),
            Some((90, TimeUnit::Second))
        );
    }

    #[test]
    fn only_a_zone_can_fail_to_be_built() {
        // Nothing is parsed any more except the zone, so nothing else can fail.
        assert!(Value::timestamp(0, TimeUnit::Second, Some("Europe/Paris")).is_ok());
        assert!(Value::timestamp(0, TimeUnit::Second, Some("+05:30")).is_ok());
        assert!(Value::timestamp(0, TimeUnit::Second, Some("+99:00")).is_err());

        // A caller that already holds a validated zone skips parsing entirely.
        assert_eq!(
            Value::timestamp_in(0, TimeUnit::Second, Some(Timezone::UTC)),
            Value::timestamp(0, TimeUnit::Second, Some("UTC")).unwrap()
        );
        assert_eq!(
            Value::timestamp_in(0, TimeUnit::Second, Some(Timezone::UTC))
                .as_timestamp_in()
                .and_then(|(_, _, zone)| zone),
            Some(&Timezone::UTC)
        );
    }

    #[test]
    fn an_accessor_refuses_a_value_that_is_not_its_temporal() {
        let date = Value::date(1);

        assert!(date.as_timestamp().is_none());
        assert!(date.as_time().is_none());
        assert!(date.as_duration().is_none());
        assert!(Value::from(1_i64).as_date().is_none());
        assert!(!Value::from("2024-01-02").is_temporal());
        assert!(date.is_temporal());
    }
}

mod comparison {
    use super::{TimeUnit, Timezone, Value};
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    fn hash(value: &Value) -> u64 {
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn one_instant_written_in_two_units_is_one_value() {
        let seconds = Value::duration(1, TimeUnit::Second);
        let milliseconds = Value::duration(1_000, TimeUnit::Millisecond);

        assert_eq!(seconds, milliseconds);
        assert_eq!(hash(&seconds), hash(&milliseconds));
        assert!(Value::duration(999, TimeUnit::Millisecond) < seconds);

        // The same rule holds for the other two counted temporals.
        assert_eq!(
            Value::time(1, TimeUnit::Second),
            Value::time(1_000_000, TimeUnit::Microsecond)
        );
        assert_eq!(
            Value::timestamp(1, TimeUnit::Second, None).unwrap(),
            Value::timestamp(1_000, TimeUnit::Millisecond, None).unwrap()
        );
    }

    #[test]
    fn a_zone_distinguishes_two_readings_of_one_instant() {
        let naive = Value::timestamp(0, TimeUnit::Second, None).unwrap();
        let zoned = Value::timestamp_in(0, TimeUnit::Second, Some(Timezone::UTC));

        assert_ne!(naive, zoned);
        assert_ne!(hash(&naive), hash(&zoned));
    }

    #[test]
    fn an_interval_layout_keeps_its_own_bucket() {
        // A month is not a number of nanoseconds, so it is never mistaken for
        // one: it compares apart from every resolution-unit count.
        assert_ne!(
            Value::duration(1, TimeUnit::YearMonth),
            Value::duration(1, TimeUnit::Nanosecond)
        );
        assert!(
            Value::duration(i64::MAX, TimeUnit::Nanosecond)
                < Value::duration(0, TimeUnit::YearMonth)
        );
    }

    #[test]
    fn kinds_sort_in_the_documented_sweep() {
        let ordered = [
            Value::Null,
            Value::from(true),
            Value::from(1_i64),
            Value::from(1.0),
            Value::decimal(1, 0),
            Value::from("a"),
            Value::from(b"a".as_slice()),
            Value::date(0),
            Value::time(0, TimeUnit::Second),
            Value::timestamp(0, TimeUnit::Second, None).unwrap(),
            Value::duration(0, TimeUnit::Second),
            Value::from_sequence([]),
            Value::from_mapping([]).unwrap(),
        ];
        for window in ordered.windows(2) {
            assert!(window[0] < window[1], "{:?} < {:?}", window[0], window[1]);
        }
    }
}

mod projection {
    use super::{DataType, TimeUnit, Value};

    #[test]
    fn a_temporal_names_exactly_one_arrow_datatype() {
        assert_eq!(
            Value::timestamp(0, TimeUnit::Microsecond, None)
                .unwrap()
                .temporal_data_type(),
            Some(DataType::Timestamp(TimeUnit::Microsecond, None))
        );
        assert!(matches!(
            Value::timestamp(0, TimeUnit::Microsecond, Some("UTC"))
                .unwrap()
                .temporal_data_type(),
            Some(DataType::Timestamp(TimeUnit::Microsecond, Some(_)))
        ));
        assert_eq!(Value::date(0).temporal_data_type(), Some(DataType::Date32));
        assert_eq!(
            Value::time(0, TimeUnit::Microsecond).temporal_data_type(),
            Some(DataType::time(TimeUnit::Microsecond).unwrap())
        );
        assert_eq!(
            Value::duration(0, TimeUnit::Second).temporal_data_type(),
            Some(DataType::Duration(TimeUnit::Second))
        );

        // A value that is not temporal names nothing.
        assert!(Value::from(1_i64).temporal_data_type().is_none());
    }

    #[test]
    fn restating_a_count_keeps_every_digit_or_refuses() {
        let at = Value::timestamp(1_700_000_000, TimeUnit::Second, None).unwrap();

        assert_eq!(at.temporal_count_at(TimeUnit::Second), Some(1_700_000_000));
        assert_eq!(
            at.temporal_count_at(TimeUnit::Millisecond),
            Some(1_700_000_000_000)
        );

        // Coarsening is only exact when nothing is being thrown away.
        let precise = Value::time(1_500, TimeUnit::Millisecond);
        assert_eq!(precise.temporal_count_at(TimeUnit::Second), None);
        assert_eq!(
            Value::time(2_000, TimeUnit::Millisecond).temporal_count_at(TimeUnit::Second),
            Some(2)
        );

        // A date has no unit, and an interval layout has no fixed width.
        assert_eq!(Value::date(0).temporal_count_at(TimeUnit::Second), None);
        assert_eq!(
            Value::duration(1, TimeUnit::YearMonth).temporal_count_at(TimeUnit::Second),
            None
        );
    }
}

mod codecs {
    use super::{TimeUnit, Value};
    use crate::text::{Json, TextCodec, Yaml};

    #[test]
    fn a_temporal_survives_json_and_yaml() {
        // TOML spells a date-time natively rather than through an envelope, so
        // its projection of these variants lands with the TOML change itself.
        let row = Value::from_mapping([
            (
                Value::from("at"),
                Value::timestamp(1_700_000_000_000_000, TimeUnit::Microsecond, Some("UTC"))
                    .unwrap(),
            ),
            (
                Value::from("naive"),
                Value::timestamp(1_700_000_000, TimeUnit::Second, None).unwrap(),
            ),
            (Value::from("on"), Value::date(19_723)),
            (
                Value::from("since_midnight"),
                Value::time(45_296_000_000, TimeUnit::Microsecond),
            ),
            (Value::from("took"), Value::duration(90, TimeUnit::Second)),
            (Value::from("price"), Value::decimal(-1_050, 2)),
        ])
        .unwrap();

        assert_eq!(Json.loads(&Json.dumps(&row).unwrap()).unwrap(), row);
        assert_eq!(Yaml.loads(&Yaml.dumps(&row).unwrap()).unwrap(), row);
    }

    #[test]
    fn a_temporal_survives_as_a_mapping_key_too() {
        // A key is a value here, so the envelope has to survive the explicit
        // key form both codecs fall back to for a non-plain key.
        let by_day = Value::from_mapping([
            (Value::date(19_723), Value::from(1_i64)),
            (Value::decimal(1_050, 2), Value::from(2_i64)),
        ])
        .unwrap();

        assert_eq!(Json.loads(&Json.dumps(&by_day).unwrap()).unwrap(), by_day);
        assert_eq!(Yaml.loads(&Yaml.dumps(&by_day).unwrap()).unwrap(), by_day);
    }
}
