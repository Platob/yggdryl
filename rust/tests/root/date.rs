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
