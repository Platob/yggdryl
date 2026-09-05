//! What a value carries into an Arrow column, and what comes back out.

use crate::arrow::{scalar_array, scalar_value};
use crate::{DataType, Field, Scalar, TimeUnit};

fn round_trip(dtype: DataType, value: Scalar) -> Scalar {
    let field = Field::new("column", dtype, true);
    let array = scalar_array(&field, &value).expect("the value materializes");
    scalar_value(&field, array.as_ref()).expect("the column decodes")
}

mod widths {
    use super::{DataType, Field, Scalar, round_trip, scalar_array};
    use crate::I256;

    #[test]
    fn an_unsigned_integer_survives_its_whole_range() {
        // The top of the u64 range has no int64 to fall into, so this is the
        // case a signed round trip silently corrupts.
        for value in [0, 1, i64::MAX as u64, u64::MAX] {
            assert_eq!(
                round_trip(DataType::UInt64, Scalar::from(value)),
                Scalar::from(value),
                "u64 {value}"
            );
        }
    }

    #[test]
    fn a_128_bit_integer_survives_the_widest_exact_column() {
        // Arrow has no 128-bit integer; a decimal at scale zero is the column
        // that holds one, and every digit has to come back.
        let column = DataType::decimal(38, 0).unwrap();
        for value in [1_i128, -1, i64::MAX as i128 + 1, 10_i128.pow(37)] {
            // The column is a decimal, so the value reads back as one - at
            // scale zero, carrying every digit.
            assert_eq!(
                round_trip(column.clone(), Scalar::from(value)),
                Scalar::d128(value, 0),
                "i128 {value}"
            );
        }
    }

    #[test]
    fn a_256_bit_decimal_survives_without_narrowing() {
        let coefficient = "12345678901234567890123456789012345678901234567890"
            .parse::<I256>()
            .unwrap();
        let value = Scalar::d256(coefficient, 7);
        assert_eq!(
            round_trip(DataType::decimal256(57, 7).unwrap(), value.clone()),
            value
        );
    }

    #[test]
    fn an_f32_column_keeps_every_bit_of_an_f32() {
        // A value has one float width. An f32 widens into it exactly, so the
        // narrowing back is exact too - including the subnormal edges.
        for value in [0.1_f32, f32::MIN_POSITIVE, f32::MAX, -0.0, f32::EPSILON] {
            let decoded = round_trip(DataType::Float32, Scalar::from(value));
            #[allow(clippy::cast_possible_truncation)]
            let narrowed = decoded.as_f64().expect("a float") as f32;
            assert_eq!(narrowed.to_bits(), value.to_bits(), "f32 {value}");
        }
    }

    #[test]
    fn bytes_survive_every_binary_layout() {
        let payload = Scalar::from(b"\x00\xffAAPL".as_slice());
        for column in [
            DataType::Binary,
            DataType::LargeBinary,
            DataType::BinaryView,
            DataType::FixedSizeBinary(6),
        ] {
            assert_eq!(round_trip(column.clone(), payload.clone()), payload);
        }
    }

    #[test]
    fn a_value_that_is_not_bytes_is_reported_and_not_silently_dropped() {
        // Reading through `as_bytes` alone turned every other kind into a
        // null, so a string written into a binary column simply vanished.
        let field = Field::new("payload", DataType::Binary, true);
        let error = scalar_array(&field, &Scalar::from("AAPL")).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("binary"), "{message}");
        assert!(!message.is_empty());
    }
}

mod bulk {
    use super::{DataType, Field, Scalar};

    #[test]
    fn one_native_sequence_builds_one_arrow_array() {
        let field = DataType::Int64.nullable_field("id");
        let values = Scalar::from_sequence([Scalar::from(1), Scalar::Null, Scalar::from(3)]);
        let array = crate::arrow::array_from_value(&field, &values).unwrap();
        assert_eq!(
            crate::arrow::array_to_value(&field, array.as_ref()).unwrap(),
            Scalar::from_sequence([Scalar::from(1), Scalar::Null, Scalar::from(3)])
        );
    }

    #[test]
    fn named_records_build_one_schema_ordered_record_batch() {
        let root = DataType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::Utf8.nullable_field("venue"),
        ])
        .unwrap()
        .required_field("row");
        let rows = Scalar::from_sequence([
            Scalar::from_record([("venue", Scalar::from("XNAS")), ("id", Scalar::from(1))])
                .unwrap(),
            Scalar::from_record([("id", Scalar::from(2))]).unwrap(),
        ]);

        let batch = crate::arrow::batch_from_value(&root, &rows).unwrap();
        assert_eq!(
            crate::arrow::batch_to_value(&batch).unwrap(),
            Scalar::from_sequence([
                Scalar::from_sequence([Scalar::from(1), Scalar::from("XNAS")]),
                Scalar::from_sequence([Scalar::from(2), Scalar::Null]),
            ])
        );
    }

    #[test]
    fn bulk_builders_refuse_non_sequence_inputs() {
        let field = Field::new("id", DataType::Int64, false);
        assert!(crate::arrow::array_from_value(&field, &Scalar::from(1)).is_err());

        let root = DataType::from_fields([field])
            .unwrap()
            .required_field("row");
        assert!(crate::arrow::batch_from_value(&root, &Scalar::from(1)).is_err());
    }
}

mod restating {
    use super::{DataType, Field, Scalar, TimeUnit, round_trip, scalar_array};
    use crate::Timezone;

    #[test]
    fn a_decimal_is_written_at_the_scale_its_column_declares() {
        let column = DataType::decimal(9, 2).unwrap();

        // 10.50 at scale 2 is the coefficient 1050, whichever way it is spelled,
        // and it reads back as the decimal it is.
        assert_eq!(
            round_trip(column.clone(), Scalar::d128(1_050, 2)),
            Scalar::d128(1_050, 2)
        );
        assert_eq!(
            round_trip(column.clone(), Scalar::d128(105, 1)),
            Scalar::d128(1_050, 2)
        );

        // A coefficient that cannot be restated without losing a digit is
        // refused rather than rounded.
        let field = Field::new("price", column, true);
        assert!(scalar_array(&field, &Scalar::d128(1_055, 3)).is_err());
    }

    #[test]
    fn a_temporal_is_written_at_the_unit_its_column_declares() {
        let micros = DataType::DateTime64 {
            unit: TimeUnit::Microsecond,
            timezone: Timezone::NAIVE,
        };
        let at =
            Scalar::datetime64(1_700_000_000, TimeUnit::Second, crate::Timezone::NAIVE).unwrap();

        assert_eq!(
            round_trip(micros.clone(), at),
            Scalar::datetime64(
                1_700_000_000_000_000,
                TimeUnit::Microsecond,
                crate::Timezone::NAIVE,
            )
            .unwrap()
        );
        assert_eq!(
            round_trip(DataType::Date32, Scalar::date32(19_723)),
            Scalar::date32(19_723)
        );
        // A Date64 spells its day in milliseconds; the day is what reads back.
        assert_eq!(
            round_trip(DataType::Date64, Scalar::date32(2)),
            Scalar::date64(172_800_000)
        );
        assert_eq!(
            round_trip(
                DataType::Duration64(TimeUnit::Millisecond),
                Scalar::duration64(90, TimeUnit::Second).unwrap()
            ),
            Scalar::duration64(90_000, TimeUnit::Millisecond).unwrap()
        );
        assert_eq!(
            round_trip(
                DataType::time(TimeUnit::Microsecond).unwrap(),
                Scalar::time32(45_296, TimeUnit::Second, crate::Timezone::NAIVE).unwrap()
            ),
            Scalar::time64(
                45_296_000_000,
                TimeUnit::Microsecond,
                crate::Timezone::NAIVE,
            )
            .unwrap()
        );

        // Coarsening that would drop a digit is refused, naming the kind.
        let seconds = Field::new(
            "at",
            DataType::DateTime64 {
                unit: TimeUnit::Second,
                timezone: Timezone::NAIVE,
            },
            true,
        );
        let error = scalar_array(
            &seconds,
            &Scalar::datetime64(1_500, TimeUnit::Millisecond, crate::Timezone::NAIVE).unwrap(),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("datetime"), "{error}");
    }

    #[test]
    fn duration32_checks_its_logical_width_on_both_arrow_directions() {
        let field = Field::new("elapsed", DataType::Duration32(TimeUnit::Second), false);
        let maximum = Scalar::duration32(i32::MAX, TimeUnit::Second).unwrap();
        assert_eq!(round_trip(field.dtype().clone(), maximum.clone()), maximum);

        let too_wide = i64::from(i32::MAX) + 1;
        assert!(scalar_array(&field, &Scalar::from(too_wide)).is_err());
        let foreign = arrow_array::DurationSecondArray::from(vec![too_wide]);
        assert!(crate::arrow::scalar_value(&field, &foreign).is_err());
    }
}
