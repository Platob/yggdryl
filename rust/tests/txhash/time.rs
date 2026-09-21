//! `rust/src/txhash/time.rs`: unix counts - one resolution restated as another,
//! an instant read out of a value, and the clock read once.

mod txhash {
    use yggdryl::txhash::{restate_unix, unix_from_scalar, unix_now};

    use yggdryl::{Error, Scalar, TimeUnit, Timezone};

    #[test]
    fn restate_unix_floors_to_coarser_and_scales_to_finer() {
        use TimeUnit::{Day, Microsecond, Millisecond, Nanosecond, Second};
        let cases: [(i64, TimeUnit, TimeUnit, i64); 10] = [
            (1_999, Nanosecond, Microsecond, 1),
            (-1, Nanosecond, Microsecond, -1),
            (-1_000, Nanosecond, Microsecond, -1),
            (-1_001, Nanosecond, Microsecond, -2),
            (1_999, Millisecond, Second, 1),
            (7, Second, Second, 7),
            (1, Second, Nanosecond, 1_000_000_000),
            (1, Day, Second, 86_400),
            (-1, Day, Millisecond, -86_400_000),
            (
                1_700_000_000_123,
                Millisecond,
                Microsecond,
                1_700_000_000_123_000,
            ),
        ];
        for (count, from, into, expected) in cases {
            assert_eq!(
                restate_unix(count, from, into).unwrap(),
                expected,
                "{count} {from} -> {into}"
            );
        }
        assert!(matches!(
            restate_unix(i64::MAX, Second, Millisecond),
            Err(Error::ArithmeticOverflow { .. })
        ));
        assert!(matches!(
            restate_unix(1, TimeUnit::YearMonth, Second),
            Err(Error::InvalidDataType { kind: "TxHash", .. })
        ));
        assert!(matches!(
            restate_unix(1, Second, Day),
            Err(Error::InvalidDataType { kind: "TxHash", .. })
        ));
    }

    #[test]
    fn unix_from_scalar_reads_every_instant_spelling() {
        let unit = TimeUnit::Microsecond;
        let read = |value: Scalar| unix_from_scalar(&value, unit).unwrap();
        assert_eq!(read(Scalar::from(7_i64)), 7);
        assert_eq!(read(Scalar::from(7_u8)), 7);
        assert_eq!(read(Scalar::from(-7_i32)), -7);
        // A zoned datetime already counts from the epoch; the zone moves nothing.
        let kolkata = Timezone::from_str("Asia/Kolkata").unwrap();
        assert_eq!(
            read(Scalar::from_datetime(1_700_000_000, TimeUnit::Second, kolkata).unwrap()),
            1_700_000_000_000_000
        );
        // A naive datetime is read as if it were UTC.
        assert_eq!(
            read(
                Scalar::from_datetime(1_700_000_000_000, TimeUnit::Millisecond, Timezone::NAIVE)
                    .unwrap()
            ),
            1_700_000_000_000_000
        );
        assert_eq!(
            read(Scalar::from_datetime(1_999, TimeUnit::Nanosecond, Timezone::UTC).unwrap()),
            1
        );
        assert_eq!(read(Scalar::date32(1)), 86_400_000_000);
        assert_eq!(read(Scalar::date64(1_500)), 1_500_000);
        assert_eq!(read(Scalar::from("1970-01-01T00:00:01Z")), 1_000_000);
        assert_eq!(
            read(Scalar::from("1970-01-01T00:00:01+01:00")),
            -3_599_000_000
        );
        assert_eq!(read(Scalar::from("1970-01-01T00:00:01")), 1_000_000);
        assert_eq!(read(Scalar::from("1970-01-01T00:00:01.5")), 1_500_000);
        assert_eq!(read(Scalar::from("1970-01-01T00:00:00.0000019")), 1);
        // Date-only text is that day's midnight, as a date scalar is - one
        // reading of a datetime rather than a third reading beside it, so the
        // compact spelling a wire writes answers the same count.
        assert_eq!(read(Scalar::from("1970-01-02")), 86_400_000_000);
        assert_eq!(read(Scalar::from("19700102")), 86_400_000_000);
        assert_eq!(read(Scalar::from("19700102Z")), 86_400_000_000);
        assert_eq!(
            unix_from_scalar(&Scalar::from(1_700_000_000_i64), TimeUnit::Second).unwrap(),
            1_700_000_000
        );
        // Text is read at the resolution its digits spell, so an instant past
        // what nanoseconds count is still a count of seconds ...
        assert_eq!(
            unix_from_scalar(&Scalar::from("2400-01-01T00:00:00Z"), TimeUnit::Second).unwrap(),
            13_569_465_600
        );
        // ... until its digits name nanoseconds.
        assert!(matches!(
            unix_from_scalar(
                &Scalar::from("2400-01-01T00:00:00.1234567Z"),
                TimeUnit::Second
            ),
            Err(Error::Parse { .. })
        ));

        for (value, why) in [
            (Scalar::Null, "a null"),
            (Scalar::from(true), "a boolean"),
            (Scalar::from(1.5_f64), "a float"),
            (
                Scalar::from_time(1, TimeUnit::Second, Timezone::NAIVE).unwrap(),
                "a time of day",
            ),
            (
                Scalar::from_duration(1, TimeUnit::Second, Timezone::NAIVE).unwrap(),
                "a duration",
            ),
            (Scalar::from_sequence([Scalar::from(1)]), "a sequence"),
        ] {
            assert!(
                matches!(
                    unix_from_scalar(&value, unit),
                    Err(Error::InvalidRecord { .. })
                ),
                "{why}"
            );
        }
        assert!(
            matches!(
                unix_from_scalar(&Scalar::from("yesterday"), unit),
                Err(Error::Parse { .. })
            ),
            "text that is no timestamp"
        );
        assert!(matches!(
            unix_from_scalar(&Scalar::from(i128::MAX), unit),
            Err(Error::ArithmeticOverflow { .. })
        ));
        assert!(matches!(
            unix_from_scalar(&Scalar::from(1), TimeUnit::Day),
            Err(Error::InvalidDataType { kind: "TxHash", .. })
        ));
    }

    #[test]
    fn unix_now_counts_forward_at_every_resolution() {
        let seconds = unix_now(TimeUnit::Second).unwrap();
        let millis = unix_now(TimeUnit::Millisecond).unwrap();
        let micros = unix_now(TimeUnit::Microsecond).unwrap();
        let nanos = unix_now(TimeUnit::Nanosecond).unwrap();
        assert!(seconds > 1_700_000_000, "the clock reads after 2023");
        assert!(millis / 1_000 >= seconds);
        assert!(micros / 1_000_000 >= seconds);
        assert!(nanos / 1_000_000_000 >= seconds);
        assert!(matches!(
            unix_now(TimeUnit::Day),
            Err(Error::InvalidDataType { kind: "TxHash", .. })
        ));
    }
}
