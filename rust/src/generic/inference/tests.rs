//! The datatype a value names, and what it refuses to name.

use crate::{DataType, Field, I256, Scalar, TimeUnit, Timezone};

mod scalars {
    use super::{DataType, I256, Scalar, TimeUnit, Timezone};

    #[test]
    fn each_integer_width_keeps_the_column_that_holds_it() {
        assert_eq!(Scalar::from(1_i64).data_type().unwrap(), DataType::Int64);
        assert_eq!(Scalar::from(1_u64).data_type().unwrap(), DataType::UInt64);

        // The whole point of keeping unsigned unsigned: the top of the range
        // has no signed column to fall into.
        assert_eq!(
            Scalar::U64(u64::MAX).data_type().unwrap(),
            DataType::UInt64,
            "a full-range u64 must not be described as an int64"
        );
    }

    #[test]
    fn a_wide_integer_becomes_the_exact_decimal_that_holds_it() {
        // Arrow has no 128-bit integer; an exact decimal at scale zero is one.
        assert_eq!(
            Scalar::I128(i128::MAX).data_type().unwrap(),
            DataType::decimal(39, 0).unwrap()
        );
        assert_eq!(
            Scalar::U128(u128::MAX).data_type().unwrap(),
            DataType::decimal(39, 0).unwrap()
        );
        assert_eq!(
            Scalar::I128(-1_000).data_type().unwrap(),
            DataType::decimal(4, 0).unwrap()
        );
    }

    #[test]
    fn each_float_keeps_its_real_width() {
        assert_eq!(
            Scalar::from(1.5_f64).data_type().unwrap(),
            DataType::Float64
        );

        for value in [0.1_f32, f32::MIN_POSITIVE, f32::MAX, -0.0_f32] {
            let recorded = Scalar::from(value);
            assert_eq!(recorded.data_type().unwrap(), DataType::Float32);
            assert_eq!(recorded.as_f32().map(f32::to_bits), Some(value.to_bits()));
        }

        let half = half::f16::from_f32(1.5);
        let recorded = Scalar::from(half);
        assert_eq!(recorded.data_type().unwrap(), DataType::Float16);
        assert_eq!(recorded.as_f16(), Some(half));
    }

    #[test]
    fn a_decimal_names_the_precision_its_digits_need() {
        assert_eq!(
            Scalar::d128(1_050, 2).data_type().unwrap(),
            DataType::decimal128(4, 2).unwrap()
        );
        // A coefficient smaller than its scale is still `0.00…`, which needs
        // precision enough to hold the scale.
        assert_eq!(
            Scalar::d128(5, 3).data_type().unwrap(),
            DataType::decimal128(3, 3).unwrap()
        );
        // Thirty-nine digits are past Decimal128 and land on Decimal256.
        assert_eq!(
            Scalar::d256(I256::from_i128(i128::MIN), 0)
                .data_type()
                .unwrap(),
            DataType::decimal256(39, 0).unwrap()
        );
        assert_eq!(
            Scalar::d128(0, 0).data_type().unwrap(),
            DataType::decimal128(1, 0).unwrap()
        );
    }

    #[test]
    fn text_and_bytes_name_their_own_columns() {
        assert_eq!(Scalar::from("AAPL").data_type().unwrap(), DataType::Utf8);
        assert_eq!(
            Scalar::from(b"\x00\xff".as_slice()).data_type().unwrap(),
            DataType::Binary
        );
        assert_eq!(Scalar::Null.data_type().unwrap(), DataType::Null);
        assert_eq!(Scalar::from(true).data_type().unwrap(), DataType::Boolean);
    }

    #[test]
    fn a_temporal_names_its_unit_and_its_zone() {
        assert_eq!(Scalar::date32(0).data_type().unwrap(), DataType::Date32);
        assert_eq!(
            Scalar::time64(0, TimeUnit::Microsecond, Timezone::NAIVE)
                .unwrap()
                .data_type()
                .unwrap(),
            DataType::Time64(TimeUnit::Microsecond)
        );
        assert_eq!(
            Scalar::time32(0, TimeUnit::Second, Timezone::NAIVE)
                .unwrap()
                .data_type()
                .unwrap(),
            DataType::Time32(TimeUnit::Second)
        );
        assert_eq!(
            Scalar::duration32(0, TimeUnit::Nanosecond)
                .unwrap()
                .data_type()
                .unwrap(),
            DataType::Duration32(TimeUnit::Nanosecond)
        );
        assert_eq!(
            Scalar::duration64(0, TimeUnit::Nanosecond)
                .unwrap()
                .data_type()
                .unwrap(),
            DataType::Duration64(TimeUnit::Nanosecond)
        );
        assert_eq!(
            Scalar::datetime64_in(0, TimeUnit::Microsecond, "Asia/Calcutta")
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
    use super::{DataType, Field, Scalar};

    #[test]
    fn a_sequence_names_the_list_of_what_its_children_agree_on() {
        let prices = Scalar::from_sequence([Scalar::from(1_i64), Scalar::from(2_i64)]);
        assert_eq!(
            prices.data_type().unwrap(),
            DataType::list(Field::new("item", DataType::Int64, false))
        );

        // A null child agrees with anything and makes the item nullable.
        let sparse = Scalar::from_sequence([Scalar::from("AAPL"), Scalar::Null]);
        assert_eq!(
            sparse.data_type().unwrap(),
            DataType::list(Field::new("item", DataType::Utf8, true))
        );

        // Nothing but nulls names the null column, which is a real Arrow type.
        assert_eq!(
            Scalar::from_sequence([]).data_type().unwrap(),
            DataType::list(Field::new("item", DataType::Null, true))
        );
    }

    #[test]
    fn a_mapping_names_a_map_because_its_keys_are_values() {
        let quote = Scalar::from_mapping([
            (Scalar::from("symbol"), Scalar::from("AAPL")),
            (Scalar::from("venue"), Scalar::Null),
        ])
        .unwrap();

        assert_eq!(
            quote.data_type().unwrap(),
            DataType::map_of(DataType::Utf8, DataType::Utf8, false).unwrap()
        );
    }

    #[test]
    fn record_fields_merge_nullability_by_name() {
        let rows = Scalar::from_sequence([
            Scalar::from_record([("id", Scalar::from(1_i64)), ("venue", Scalar::Null)]).unwrap(),
            Scalar::from_record([("id", Scalar::from(2_i64)), ("venue", Scalar::from("XNAS"))])
                .unwrap(),
        ]);
        let data_type = rows.data_type().unwrap();
        let DataType::List(item) = data_type else {
            panic!("expected a list")
        };
        let fields = item.data_type().as_fields().expect("record fields");
        assert_eq!(fields[0].name(), "id");
        assert!(!fields[0].is_nullable());
        assert_eq!(fields[1].name(), "venue");
        assert_eq!(fields[1].data_type(), &DataType::Utf8);
        assert!(fields[1].is_nullable());
    }
}

mod fields {
    use super::{DataType, Scalar};

    #[test]
    fn shape_specific_fields_have_one_cross_language_name() {
        let scalar = Scalar::from(7_i64).inferred_scalar_field().unwrap();
        assert_eq!(scalar.name(), "value");
        assert_eq!(scalar.data_type(), &DataType::Int64);

        let array = Scalar::from_sequence([Scalar::from(1_i64), Scalar::Null])
            .inferred_array_field()
            .unwrap();
        assert_eq!(array.name(), "item");
        assert!(array.is_nullable());

        let rows = Scalar::from_sequence([
            Scalar::from_record([("id", Scalar::from(1_i64)), ("venue", Scalar::Null)]).unwrap(),
            Scalar::from_record([("id", Scalar::from(2_i64)), ("venue", Scalar::from("XNAS"))])
                .unwrap(),
        ]);
        let root = rows.inferred_struct_field().unwrap();
        assert_eq!(root.name(), "row");
        assert!(!root.is_nullable());
        assert!(root.get_field_by_name("venue").unwrap().is_nullable());
    }

    #[test]
    fn ambiguous_shapes_require_a_declared_field() {
        for message in [
            Scalar::from(1_i64)
                .inferred_array_field()
                .unwrap_err()
                .to_string(),
            Scalar::from_sequence([])
                .inferred_array_field()
                .unwrap_err()
                .to_string(),
            Scalar::from_sequence([])
                .inferred_struct_field()
                .unwrap_err()
                .to_string(),
            Scalar::from_sequence([Scalar::from_sequence([Scalar::from(1_i64)])])
                .inferred_struct_field()
                .unwrap_err()
                .to_string(),
        ] {
            assert!(message.contains("Field"), "{message}");
        }
    }
}

mod refusals {
    use super::{Scalar, TimeUnit, Timezone};

    #[test]
    fn children_that_disagree_are_an_error_and_not_a_guess() {
        let mixed = Scalar::from_sequence([Scalar::from(1_i64), Scalar::from(1.5)]);
        let message = mixed.data_type().unwrap_err().to_string();

        assert!(message.contains("one datatype"), "{message}");
        assert!(message.contains("int64"), "{message}");
        assert!(message.contains("float64"), "{message}");
    }

    #[test]
    fn an_interval_layout_is_not_a_temporal_resolution() {
        for message in [
            Scalar::duration32(1, TimeUnit::YearMonth)
                .unwrap_err()
                .to_string(),
            Scalar::duration64(1, TimeUnit::YearMonth)
                .unwrap_err()
                .to_string(),
        ] {
            assert!(message.contains("fixed temporal unit"), "{message}");
        }
    }

    #[test]
    fn a_time_of_day_timezone_is_never_silently_discarded() {
        for value in [
            Scalar::Time32(1, TimeUnit::Second, Timezone::UTC),
            Scalar::Time64(1, TimeUnit::Microsecond, Timezone::UTC),
        ] {
            let message = value.data_type().unwrap_err().to_string();
            assert!(message.contains("timezone"), "{message}");
        }
    }

    #[test]
    fn a_value_nested_past_the_shared_limit_is_refused_and_not_a_stack_overflow() {
        let mut nested = Scalar::from(1_i64);
        for _ in 0..200 {
            nested = Scalar::from_sequence([nested]);
        }
        let message = nested.data_type().unwrap_err().to_string();
        assert!(message.contains("hard limit of 64"), "{message}");
    }
}
