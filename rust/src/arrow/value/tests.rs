//! What a value carries into an Arrow column, and what comes back out.

use crate::arrow::ArrowScalar;
use crate::{DataType, Field, TimeUnit, Value};

fn round_trip(data_type: DataType, value: Value) -> Value {
    let field = Field::new("column", data_type, true);
    ArrowScalar::from_value(field, value)
        .expect("the value materializes")
        .to_value()
        .expect("the column decodes")
}

mod widths {
    use super::{ArrowScalar, DataType, Field, Value, round_trip};

    #[test]
    fn an_unsigned_integer_survives_its_whole_range() {
        // The top of the u64 range has no int64 to fall into, so this is the
        // case a signed round trip silently corrupts.
        for value in [0, 1, i64::MAX as u64, u64::MAX] {
            assert_eq!(
                round_trip(DataType::UInt64, Value::U64(value)),
                Value::U64(value),
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
            assert_eq!(
                round_trip(column.clone(), Value::from(value)).as_i128(),
                Some(value),
                "i128 {value}"
            );
        }
    }

    #[test]
    fn an_f32_column_keeps_every_bit_of_an_f32() {
        // A value has one float width. An f32 widens into it exactly, so the
        // narrowing back is exact too - including the subnormal edges.
        for value in [0.1_f32, f32::MIN_POSITIVE, f32::MAX, -0.0, f32::EPSILON] {
            let decoded = round_trip(DataType::Float32, Value::from(value));
            #[allow(clippy::cast_possible_truncation)]
            let narrowed = decoded.as_f64().expect("a float") as f32;
            assert_eq!(narrowed.to_bits(), value.to_bits(), "f32 {value}");
        }
    }

    #[test]
    fn bytes_survive_every_binary_layout() {
        let payload = Value::from(b"\x00\xffAAPL".as_slice());
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
        let error = ArrowScalar::from_value(field, Value::from("AAPL")).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("binary"), "{message}");
        assert!(!message.is_empty());
    }
}

mod restating {
    use super::{ArrowScalar, DataType, Field, TimeUnit, Value, round_trip};

    #[test]
    fn a_decimal_is_written_at_the_scale_its_column_declares() {
        let column = DataType::decimal(9, 2).unwrap();

        // 10.50 at scale 2 is the coefficient 1050, whichever way it is spelled.
        assert_eq!(
            round_trip(column.clone(), Value::decimal(1_050, 2)).as_i128(),
            Some(1_050)
        );
        assert_eq!(
            round_trip(column.clone(), Value::decimal(105, 1)).as_i128(),
            Some(1_050)
        );

        // A coefficient that cannot be restated without losing a digit is
        // refused rather than rounded.
        let field = Field::new("price", column, true);
        assert!(ArrowScalar::from_value(field, Value::decimal(1_055, 3)).is_err());
    }

    #[test]
    fn a_temporal_is_written_at_the_unit_its_column_declares() {
        let micros = DataType::Timestamp(TimeUnit::Microsecond, None);
        let at = Value::timestamp(1_700_000_000, TimeUnit::Second, None).unwrap();

        assert_eq!(
            round_trip(micros.clone(), at).as_i64(),
            Some(1_700_000_000_000_000)
        );
        assert_eq!(
            round_trip(DataType::Date32, Value::date(19_723)).as_i64(),
            Some(19_723)
        );
        assert_eq!(
            round_trip(DataType::Date64, Value::date(2)).as_i64(),
            Some(2 * 86_400_000)
        );
        assert_eq!(
            round_trip(
                DataType::Duration(TimeUnit::Millisecond),
                Value::duration(90, TimeUnit::Second)
            )
            .as_i64(),
            Some(90_000)
        );
        assert_eq!(
            round_trip(
                DataType::time(TimeUnit::Microsecond).unwrap(),
                Value::time(45_296, TimeUnit::Second)
            )
            .as_i64(),
            Some(45_296_000_000)
        );

        // Coarsening that would drop a digit is refused, naming the kind.
        let seconds = Field::new("at", DataType::Timestamp(TimeUnit::Second, None), true);
        let error = ArrowScalar::from_value(
            seconds,
            Value::timestamp(1_500, TimeUnit::Millisecond, None).unwrap(),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("timestamp"), "{error}");
    }
}
