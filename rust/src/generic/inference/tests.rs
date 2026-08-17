//! The datatype a value names, and what it refuses to name.

use crate::{DataType, Field, TimeUnit, Value};

mod scalars {
    use super::{DataType, TimeUnit, Value};

    #[test]
    fn each_integer_width_keeps_the_column_that_holds_it() {
        assert_eq!(Value::from(1_i64).data_type().unwrap(), DataType::Int64);
        assert_eq!(Value::from(1_u64).data_type().unwrap(), DataType::UInt64);

        // The whole point of keeping unsigned unsigned: the top of the range
        // has no signed column to fall into.
        assert_eq!(
            Value::U64(u64::MAX).data_type().unwrap(),
            DataType::UInt64,
            "a full-range u64 must not be described as an int64"
        );
    }

    #[test]
    fn a_wide_integer_becomes_the_exact_decimal_that_holds_it() {
        // Arrow has no 128-bit integer; an exact decimal at scale zero is one.
        assert_eq!(
            Value::I128(i128::MAX).data_type().unwrap(),
            DataType::decimal(39, 0).unwrap()
        );
        assert_eq!(
            Value::U128(u128::MAX).data_type().unwrap(),
            DataType::decimal(39, 0).unwrap()
        );
        assert_eq!(
            Value::I128(-1_000).data_type().unwrap(),
            DataType::decimal(4, 0).unwrap()
        );
    }

    #[test]
    fn a_float_is_a_double_and_an_f32_is_recorded_exactly() {
        assert_eq!(Value::from(1.5_f64).data_type().unwrap(), DataType::Float64);

        // A value has one float width. An `f32` widens into it exactly - every
        // f32 is a double - so nothing is invented and nothing is lost.
        for value in [0.1_f32, f32::MIN_POSITIVE, f32::MAX, -0.0_f32] {
            let recorded = Value::from(value);
            assert_eq!(recorded.as_f64(), Some(f64::from(value)));
            #[allow(clippy::cast_possible_truncation)]
            let narrowed = recorded.as_f64().unwrap() as f32;
            assert_eq!(narrowed.to_bits(), value.to_bits(), "{value} round-trips");
        }
    }

    #[test]
    fn a_decimal_names_the_precision_its_digits_need() {
        assert_eq!(
            Value::decimal(1_050, 2).data_type().unwrap(),
            DataType::decimal(4, 2).unwrap()
        );
        // A coefficient smaller than its scale is still `0.00…`, which needs
        // precision enough to hold the scale.
        assert_eq!(
            Value::decimal(5, 3).data_type().unwrap(),
            DataType::decimal(3, 3).unwrap()
        );
        // Thirty-nine digits are past Decimal128 and land on Decimal256.
        assert_eq!(
            Value::decimal(i128::MIN, 0).data_type().unwrap(),
            DataType::decimal(39, 0).unwrap()
        );
        assert_eq!(
            Value::decimal(0, 0).data_type().unwrap(),
            DataType::decimal(1, 0).unwrap()
        );
    }

    #[test]
    fn text_and_bytes_name_their_own_columns() {
        assert_eq!(Value::from("AAPL").data_type().unwrap(), DataType::Utf8);
        assert_eq!(
            Value::from(b"\x00\xff".as_slice()).data_type().unwrap(),
            DataType::Binary
        );
        assert_eq!(Value::Null.data_type().unwrap(), DataType::Null);
        assert_eq!(Value::from(true).data_type().unwrap(), DataType::Boolean);
    }

    #[test]
    fn a_temporal_names_its_unit_and_its_zone() {
        assert_eq!(Value::date(0).data_type().unwrap(), DataType::Date32);
        assert_eq!(
            Value::time(0, TimeUnit::Microsecond).data_type().unwrap(),
            DataType::Time64(TimeUnit::Microsecond)
        );
        assert_eq!(
            Value::time(0, TimeUnit::Second).data_type().unwrap(),
            DataType::Time32(TimeUnit::Second)
        );
        assert_eq!(
            Value::duration(0, TimeUnit::Nanosecond)
                .data_type()
                .unwrap(),
            DataType::Duration(TimeUnit::Nanosecond)
        );
        assert_eq!(
            Value::timestamp(0, TimeUnit::Microsecond, Some("Asia/Calcutta"))
                .unwrap()
                .data_type()
                .unwrap(),
            DataType::Timestamp(
                TimeUnit::Microsecond,
                Some(crate::Timezone::from_str("Asia/Kolkata").unwrap())
            )
        );
    }
}

mod containers {
    use super::{DataType, Field, Value};

    #[test]
    fn a_sequence_names_the_list_of_what_its_children_agree_on() {
        let prices = Value::from_sequence([Value::from(1_i64), Value::from(2_i64)]);
        assert_eq!(
            prices.data_type().unwrap(),
            DataType::list(Field::new("item", DataType::Int64, false))
        );

        // A null child agrees with anything and makes the item nullable.
        let sparse = Value::from_sequence([Value::from("AAPL"), Value::Null]);
        assert_eq!(
            sparse.data_type().unwrap(),
            DataType::list(Field::new("item", DataType::Utf8, true))
        );

        // Nothing but nulls names the null column, which is a real Arrow type.
        assert_eq!(
            Value::from_sequence([]).data_type().unwrap(),
            DataType::list(Field::new("item", DataType::Null, true))
        );
    }

    #[test]
    fn a_mapping_names_a_map_because_its_keys_are_values() {
        let quote = Value::from_mapping([
            (Value::from("symbol"), Value::from("AAPL")),
            (Value::from("venue"), Value::Null),
        ])
        .unwrap();

        assert_eq!(
            quote.data_type().unwrap(),
            DataType::map_of(DataType::Utf8, DataType::Utf8, false).unwrap()
        );
    }
}

mod refusals {
    use super::{TimeUnit, Value};

    #[test]
    fn children_that_disagree_are_an_error_and_not_a_guess() {
        let mixed = Value::from_sequence([Value::from(1_i64), Value::from(1.5)]);
        let message = mixed.data_type().unwrap_err().to_string();

        assert!(message.contains("one datatype"), "{message}");
        assert!(message.contains("int64"), "{message}");
        assert!(message.contains("float64"), "{message}");
    }

    #[test]
    fn an_interval_layout_is_not_a_temporal_resolution() {
        let message = Value::duration(1, TimeUnit::YearMonth)
            .data_type()
            .unwrap_err()
            .to_string();
        assert!(message.contains("temporal resolution"), "{message}");
    }

    #[test]
    fn a_value_nested_past_the_shared_limit_is_refused_and_not_a_stack_overflow() {
        let mut nested = Value::from(1_i64);
        for _ in 0..200 {
            nested = Value::from_sequence([nested]);
        }
        let message = nested.data_type().unwrap_err().to_string();
        assert!(message.contains("hard limit of 64"), "{message}");
    }
}
