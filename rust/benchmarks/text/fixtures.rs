use std::str::FromStr;

use yggdryl::text::xml::{ATTRIBUTE_PREFIX, TEXT_KEY};
use yggdryl::{DataType, Field, I256, Scalar, TimeUnit, Timezone};

pub(crate) fn nested(depth: usize) -> Scalar {
    (0..depth).fold(Scalar::from(0), |value, _| Scalar::from_sequence([value]))
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

/// One root element around `value`, because an XML document is exactly one.
pub(crate) fn xml_document(name: &str, value: Scalar) -> Scalar {
    Scalar::from_record([(name, value)]).unwrap()
}

/// A document `depth` elements deep.
///
/// XML repeats an element to frame a sequence, so depth is records where
/// [`nested`] builds sequences.
pub(crate) fn xml_nested(depth: usize) -> Scalar {
    (0..depth).fold(Scalar::from("0"), |value, _| xml_document("level", value))
}

/// A root holding `width` sibling elements, each one leaf of character data.
pub(crate) fn xml_wide(width: i64) -> Scalar {
    xml_document(
        "row",
        Scalar::from_record((0..width).map(|index| (format!("cell_{index}"), Scalar::from(index))))
            .unwrap(),
    )
}

/// A root repeating one element `width` times, each carrying its values as
/// attributes beside its character data - the shape no other format has.
pub(crate) fn xml_attributed(width: i64) -> Scalar {
    xml_document(
        "row",
        Scalar::from_record([(
            "cell".to_owned(),
            Scalar::from_sequence((0..width).map(|index| {
                Scalar::from_record([
                    (format!("{ATTRIBUTE_PREFIX}index"), Scalar::from(index)),
                    (format!("{ATTRIBUTE_PREFIX}unit"), Scalar::from("bps")),
                    (TEXT_KEY.to_owned(), Scalar::from(index * 7)),
                ])
                .unwrap()
            })),
        )])
        .unwrap(),
    )
}
