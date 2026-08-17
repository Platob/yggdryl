use yggdryl::Value;

pub(crate) fn nested(depth: usize) -> Value {
    (0..depth).fold(Value::Null, |value, _| Value::from_sequence([value]))
}

pub(crate) fn representative() -> Value {
    Value::from_mapping([
        (Value::from("symbol"), Value::from("MSFT")),
        (Value::from("quantity"), Value::from(120_i64)),
        (Value::from("price"), Value::from(413.75_f64)),
        (
            Value::from("tags"),
            Value::from_sequence([Value::from("closing"), Value::from("auction")]),
        ),
    ])
    .unwrap()
}

/// A value no text format spells natively, so every envelope path carries it.
///
/// The non-string key forces the whole mapping through the mapping envelope,
/// and its three values are the byte, wide-integer, and non-finite float
/// envelopes, so one fixture measures the encoder's whole envelope surface.
pub(crate) fn exotic() -> Value {
    Value::from_mapping([
        (
            Value::from_sequence([Value::from("payload")]),
            Value::from(vec![0, 1, 255]),
        ),
        (Value::from("sequence"), Value::U128(u128::MAX)),
        (Value::from("price"), Value::from(f64::INFINITY)),
    ])
    .unwrap()
}
