use std::str::FromStr;

use yggdryl::{DataType, Field, I256, TimeUnit, Timezone, Value};

pub(crate) fn nested(depth: usize) -> Value {
    (0..depth).fold(Value::Null, |value, _| Value::from_sequence([value]))
}

pub(crate) fn representative() -> Value {
    Value::from_record([
        ("symbol", Value::from("MSFT")),
        ("quantity", Value::from(120_i64)),
        ("price", Value::from(413.75_f64)),
        (
            "tags",
            Value::from_sequence([Value::from("closing"), Value::from("auction")]),
        ),
    ])
    .unwrap()
}

/// Exact values projected through natural scalars and restored by one field.
pub(crate) fn typed() -> (Value, Field) {
    let value = Value::from_record([
        ("amount", Value::d256(I256::from_str("1234500").unwrap(), 4)),
        (
            "at",
            Value::datetime64(0, TimeUnit::Second, Timezone::UTC).unwrap(),
        ),
        ("payload", Value::from(vec![0, 1, 255])),
    ])
    .unwrap();
    let field = Field::new(
        "row",
        DataType::from_fields([
            Field::new("amount", DataType::decimal256(76, 4).unwrap(), false),
            Field::new(
                "at",
                DataType::Timestamp(TimeUnit::Second, Some(Timezone::UTC)),
                false,
            ),
            Field::new("payload", DataType::Binary, false),
        ])
        .unwrap(),
        false,
    );
    (value, field)
}
