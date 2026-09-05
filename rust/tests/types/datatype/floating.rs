use yggdryl::{DataType, Error};

#[test]
fn decimal_selects_the_smallest_arrow_representation() {
    assert_eq!(
        DataType::decimal(1, 0).unwrap(),
        DataType::Decimal32 {
            precision: 1,
            scale: 0,
        }
    );
    assert_eq!(
        DataType::decimal(10, 2).unwrap(),
        DataType::Decimal64 {
            precision: 10,
            scale: 2,
        }
    );
    assert_eq!(
        DataType::decimal(19, 2).unwrap(),
        DataType::Decimal128 {
            precision: 19,
            scale: 2,
        }
    );
    assert_eq!(
        DataType::decimal(38, 38).unwrap(),
        DataType::Decimal128 {
            precision: 38,
            scale: 38,
        }
    );
    assert_eq!(
        DataType::decimal(39, 39).unwrap(),
        DataType::Decimal256 {
            precision: 39,
            scale: 39,
        }
    );
    assert_eq!(
        DataType::decimal(76, -20).unwrap(),
        DataType::Decimal256 {
            precision: 76,
            scale: -20,
        }
    );
}

#[test]
fn decimal_uses_the_selected_constructor_validation() {
    for (precision, scale, expected_kind) in [
        (0, 0, "Decimal32"),
        (12, 13, "Decimal64"),
        (39, 40, "Decimal256"),
        (77, 0, "Decimal256"),
    ] {
        let error = DataType::decimal(precision, scale).unwrap_err();
        assert!(
            matches!(
                error,
                Error::InvalidDataType { kind, .. } if kind == expected_kind
            ),
            "unexpected error for decimal({precision},{scale}): {error}"
        );
    }
}

#[test]
fn decimal_preserves_existing_negative_scale_rules() {
    assert_eq!(
        DataType::decimal(9, i8::MIN).unwrap(),
        DataType::decimal32(9, i8::MIN).unwrap()
    );
    assert_eq!(
        DataType::decimal(18, i8::MIN).unwrap(),
        DataType::decimal64(18, i8::MIN).unwrap()
    );
    assert_eq!(
        DataType::decimal(38, i8::MIN).unwrap(),
        DataType::decimal128(38, i8::MIN).unwrap()
    );
    assert_eq!(
        DataType::decimal(39, i8::MIN).unwrap(),
        DataType::decimal256(39, i8::MIN).unwrap()
    );
}

#[test]
fn generic_decimal_parser_selects_and_round_trips_physical_storage() {
    for (expression, expected) in [
        (
            "decimal(9,2)",
            DataType::Decimal32 {
                precision: 9,
                scale: 2,
            },
        ),
        (
            "decimal(10,2)",
            DataType::Decimal64 {
                precision: 10,
                scale: 2,
            },
        ),
        (
            "decimal(38,18)",
            DataType::Decimal128 {
                precision: 38,
                scale: 18,
            },
        ),
        (
            "decimal(39,18)",
            DataType::Decimal256 {
                precision: 39,
                scale: 18,
            },
        ),
        (
            "numeric(76,20)",
            DataType::Decimal256 {
                precision: 76,
                scale: 20,
            },
        ),
    ] {
        let parsed = DataType::from_str(expression).unwrap();
        assert_eq!(parsed, expected, "{expression}");
        assert_eq!(DataType::from_str(&parsed.to_string()).unwrap(), parsed);
    }

    assert!(DataType::from_str("decimal(77,0)").is_err());
    assert!(DataType::from_str("decimal128(39,0)").is_err());
}
