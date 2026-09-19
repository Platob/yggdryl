//! Typing a regex: named captures become fields, with order, nullability and format types.

use yggdryl::types::{DateTimeType, TimeType};
use yggdryl::{DataType, Error, Field, TimeUnit, Timezone};

#[test]
fn named_captures_keep_order_nullability_and_format_types() {
    let dtype = DataType::from_regex(
        concat!(
            r"(?<enabled>true|false) ",
            r"(?<id>[-+]?\d+) ",
            r"(?<price>[-+]?\d+\.\d+) ",
            r"(?<date>\d{4}-\d{2}-\d{2}) ",
            r"(?<clock>\d{2}:\d{2}:\d{2}\.\d{6}) ",
            r"(?<stamp>\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z) ",
            r"(?<text>\S+)"
        ),
        true,
    )
    .unwrap();
    let fields = dtype.as_fields().unwrap();
    assert_eq!(
        fields.iter().map(Field::name).collect::<Vec<_>>(),
        ["enabled", "id", "price", "date", "clock", "stamp", "text"]
    );
    assert_eq!(dtype.field("enabled").unwrap().dtype(), &DataType::Boolean);
    assert_eq!(dtype.field("id").unwrap().dtype(), &DataType::Int64);
    assert_eq!(dtype.field("price").unwrap().dtype(), &DataType::Float64);
    assert_eq!(dtype.field("date").unwrap().dtype(), &DataType::date32());
    assert_eq!(
        dtype.field("clock").unwrap().dtype(),
        &DataType::Time(TimeType::Time64(TimeUnit::Microsecond))
    );
    assert_eq!(
        dtype.field("stamp").unwrap().dtype(),
        &DataType::DateTime(DateTimeType::DateTime64 {
            unit: TimeUnit::Second,
            timezone: Timezone::UTC,
        })
    );
    assert_eq!(dtype.field("text").unwrap().dtype(), &DataType::utf8());
    assert!(fields.iter().all(Field::is_nullable));
}

#[test]
fn disabling_autotype_keeps_every_capture_utf8() {
    let dtype = DataType::from_regex(r"(?<id>\d+)-(?<flag>true|false)", false).unwrap();
    assert!(
        dtype
            .as_fields()
            .unwrap()
            .iter()
            .all(|field| field.dtype() == &DataType::utf8())
    );
}

#[test]
fn malformed_and_over_nested_regexes_are_typed_errors() {
    assert!(matches!(
        DataType::from_regex("(?<id>", true),
        Err(Error::InvalidDataType { kind: "regex", .. })
    ));
    let nested = format!(
        "{}x{}",
        "(?:".repeat(DataType::PARSE_RECURSION_LIMIT + 1),
        ")".repeat(DataType::PARSE_RECURSION_LIMIT + 1)
    );
    assert!(matches!(
        DataType::from_regex(&nested, true),
        Err(Error::InvalidDataType { kind: "regex", .. })
    ));
}
