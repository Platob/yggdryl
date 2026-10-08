//! `rust/src/date.rs`: five temporal families: a datetime, a date, a time
//! of day, a duration and an interval, each one datatype over the leaves
//! its widths are.

mod temporal {

    use yggdryl::DateType;
    use yggdryl::{DataType, DataTypeId, DataTypeKind, TimeUnit};

    #[test]
    fn every_date_leaf_is_one_datatype_under_its_id() {
        // A date has no parameter: the unit is what the width means.
        for (leaf, id, unit, bits) in [
            (DateType::Date32, DataTypeId::Date32, TimeUnit::Day, 32),
            (
                DateType::Date64,
                DataTypeId::Date64,
                TimeUnit::Millisecond,
                64,
            ),
        ] {
            assert_eq!(leaf.id(), id);
            assert_eq!(leaf.as_str(), id.as_str());
            assert_eq!(leaf.unit(), unit);
            assert_eq!(leaf.bit_width(), bits);
            assert_eq!(leaf.id().temporal_family(), Some("date"));
            assert_eq!(DateType::from_id(id), Some(leaf));
            assert!(leaf.validate().is_ok());
            assert_eq!(leaf.to_string(), id.as_str());
            assert!(id.is_temporal(), "{id}");

            let dtype = DataType::from(leaf);
            assert_eq!(dtype, DataType::from(leaf));
            assert_eq!(dtype.id(), id);
            assert_eq!(dtype.kind(), DataTypeKind::Temporal);
            assert_eq!(dtype.date_type(), Some(leaf));
            assert_eq!(dtype.time_type(), None);
            assert_eq!(dtype.datetime_type(), None);
            assert_eq!(dtype.duration_type(), None);
            assert_eq!(dtype.interval_type(), None);
            assert_eq!(DataType::from_str(&dtype.to_string()).unwrap(), dtype);
            assert_eq!(DateType::try_from(&dtype).unwrap(), leaf);
            assert!(dtype.validate().is_ok());
        }
        assert_eq!(DateType::ALL, [DateType::Date32, DateType::Date64]);
        assert_eq!(DateType::default(), DateType::Date32);
        assert_eq!(DateType::from_id(DataTypeId::Time32), None);

        // The two constructors are the two leaves, and `date` is the narrow one.
        assert_eq!(DataType::date32(), DataType::Date32);
        assert_eq!(DataType::date64(), DataType::Date64);
        assert_eq!(DataType::from_str("date").unwrap(), DataType::date32());
        assert_eq!(DataType::from_str("date64").unwrap(), DataType::date64());

        // Another family is refused by name.
        let refused = DateType::try_from(&DataType::Int64)
            .unwrap_err()
            .to_string();
        assert!(refused.contains("expected a date datatype"), "{refused}");
        assert_eq!(DataType::Int64.date_type(), None);
    }
}

/// `Date32::from_text`, the ISO door of the date family.
mod from_text {
    use yggdryl::{DataType, Date32, Error, Scalar, TimeUnit};

    /// 2026-09-30 as days since the epoch.
    const DAY: i32 = 20_726;

    #[test]
    fn a_day_the_calendar_lacks_and_a_clock_after_the_date_are_refused() {
        for (text, position) in [
            ("2026-02-30", 8),
            ("20260230", 6),
            ("2026-13-01", 5),
            // A date states the day and stops: a clock is a datetime's.
            ("20260930T00:00", 8),
            ("2026-09-30T00:00:00", 10),
            ("2026-09-30 ", 10),
            ("", 0),
        ] {
            let error = Date32::from_text(text).unwrap_err();
            assert!(
                matches!(&error, Error::Parse { target: "date", position: held, .. } if *held == position),
                "{text:?}: {error}"
            );
        }
    }

    #[test]
    fn both_spellings_read_the_same_day_and_the_day_prints_extended() {
        for text in ["2026-09-30", "20260930"] {
            let read = Date32::from_text(text).unwrap_or_else(|error| panic!("{text:?}: {error}"));
            assert_eq!(
                (read.count(), read.unit()),
                (DAY, TimeUnit::Day),
                "{text:?}"
            );
            assert!(read.timezone().is_naive());
            assert_eq!(read.to_string(), "2026-09-30");
        }
        assert_eq!(Date32::from_text("1970-01-01").unwrap().count(), 0);
        assert_eq!(Date32::from_text("19691231").unwrap().count(), -1);
        // The value door of both widths reads through it: a `Date64` is the
        // day's midnight in milliseconds.
        assert_eq!(
            DataType::date32().scalar(Scalar::from("20260930")).unwrap(),
            Scalar::date32(DAY)
        );
        assert_eq!(
            DataType::date64()
                .scalar(Scalar::from("2026-09-30"))
                .unwrap(),
            Scalar::date64(i64::from(DAY) * 86_400_000)
        );
        assert!(
            DataType::date64()
                .scalar(Scalar::from("20260930T00:00"))
                .is_err()
        );
    }
}
