use std::str::FromStr;

use yggdryl::{DataType, Field, I256, Scalar, TimeUnit, Timezone};

pub(crate) fn nested(depth: usize) -> Scalar {
    (0..depth).fold(Scalar::I64(0), |value, _| Scalar::from_sequence([value]))
}

pub(crate) fn representative() -> Scalar {
    Scalar::from_record([
        ("symbol", Scalar::from("MSFT")),
        ("quantity", Scalar::from(120_i64)),
        ("price", Scalar::from(413.75_f64)),
        (
            "tags",
            Scalar::from_sequence([Scalar::from("closing"), Scalar::from("auction")]),
        ),
    ])
    .unwrap()
}

/// Exact values projected through natural scalars and restored by one field.
pub(crate) fn typed() -> (Scalar, Field) {
    let value = Scalar::from_record([
        (
            "amount",
            Scalar::d256(I256::from_str("1234500").unwrap(), 4),
        ),
        (
            "at",
            Scalar::datetime64(0, TimeUnit::Second, Timezone::UTC).unwrap(),
        ),
        ("payload", Scalar::from(vec![0, 1, 255])),
    ])
    .unwrap();
    let field = Field::new(
        "row",
        DataType::from_fields([
            Field::new("amount", DataType::decimal256(76, 4).unwrap(), false),
            Field::new(
                "at",
                DataType::DateTime64 {
                    unit: TimeUnit::Second,
                    timezone: Timezone::UTC,
                },
                false,
            ),
            Field::new("payload", DataType::Binary, false),
        ])
        .unwrap(),
        false,
    );
    (value, field)
}
