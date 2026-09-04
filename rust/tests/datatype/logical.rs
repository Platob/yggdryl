//! A row declared in FIX's datatype names is an ordinary row everywhere else.

use arrow_array::Array;
use arrow_array::DictionaryArray;
use arrow_array::types::Int32Type;
use arrow_schema::DataType as ArrowDataType;
use yggdryl::{AsciiDictionary, DataType, Field, Scalar, TimeUnit, Timezone};

/// The declaration a FIX-fed writer would hand the schema, in FIX spellings.
const FIX_ROW: &str = "struct<ccy: Currency, venue: Exchange, px: Price, qty: Qty, \
                       at: UTCTimestamp, day: LocalMktDate, seq: SeqNum>";

#[test]
fn a_fix_declared_row_projects_to_the_arrow_types_the_names_resolved() {
    let row = Field::new("row", DataType::from_str(FIX_ROW).unwrap(), false);
    let schema = row.clone().into_arrow_schema().unwrap();
    let arrow: Vec<(&str, &ArrowDataType)> = schema
        .fields()
        .iter()
        .map(|field| (field.name().as_str(), field.data_type()))
        .collect();
    assert_eq!(
        arrow,
        [
            ("ccy", &ArrowDataType::FixedSizeBinary(4)),
            ("venue", &ArrowDataType::FixedSizeBinary(4)),
            ("px", &ArrowDataType::Decimal64(18, 8)),
            ("qty", &ArrowDataType::Decimal64(18, 8)),
            (
                "at",
                &ArrowDataType::Timestamp(arrow_schema::TimeUnit::Nanosecond, Some("UTC".into()))
            ),
            ("day", &ArrowDataType::Date32),
            ("seq", &ArrowDataType::Int64),
        ]
    );

    // The names are inert: what came back is the datatype, not a registration,
    // so the Arrow round trip is the ordinary one.
    assert_eq!(Field::from_arrow_schema("row", &schema).unwrap(), row);
    assert_eq!(
        row.dtype().get_field_by_path("at").map(Field::dtype),
        Some(&DataType::Timestamp(
            TimeUnit::Nanosecond,
            Some(Timezone::UTC)
        ))
    );
    // A row declared in the resolved spellings is the same row.
    let resolved = Field::new(
        "row",
        DataType::from_str(&row.dtype().to_string()).unwrap(),
        false,
    );
    assert_eq!(resolved, row);
}

#[test]
fn a_fix_declared_row_types_the_text_a_message_carried() {
    let row = Field::new("row", DataType::from_str(FIX_ROW).unwrap(), false);
    // A FIX value is text on the wire, so the field-directed parser is where
    // the registration earns its keep: the name typed the column, and the
    // column typed the string.
    let message = r#"{"ccy":"USD","venue":"XCME","px":"101.25","qty":"7",
        "at":"2026-09-04T10:00:00.000000001Z","day":"2026-09-04","seq":9}"#;
    let value = yggdryl::json::from_utf8_with_field(message, &row).unwrap();
    let Scalar::Sequence(columns) = &value else {
        panic!("a canonical row, got {value:?}");
    };

    assert_eq!(columns[0], Scalar::from("USD"));
    assert_eq!(columns[1], Scalar::from("XCME"));
    // The price keeps eight fractional digits exactly, which is the whole
    // reason the float family resolves to a decimal.
    assert_eq!(columns[2], Scalar::d128(10_125_000_000, 8));
    assert_eq!(columns[3], Scalar::d128(700_000_000, 8));
    assert_eq!(columns[6], Scalar::from(9_i64));

    // The instant reads at nanoseconds, so the fraction survives.
    let at = columns[4].as_temporal().expect("a temporal reading");
    assert_eq!(at.count(), 1_788_516_000_000_000_001);
    assert_eq!(at.unit(), TimeUnit::Nanosecond);
    let day = columns[5].as_temporal().expect("a temporal reading");
    assert_eq!(day.count(), 20_700);

    // A value that does not fit the resolved datatype is refused by that
    // datatype, never by the name that spelled it.
    let refused = yggdryl::json::from_utf8_with_field(
        r#"{"ccy":"EURO!","venue":"XCME","px":"1","qty":"1","at":"2026-09-04T10:00:00Z","day":"2026-09-04","seq":1}"#,
        &row,
    )
    .unwrap_err()
    .to_string();
    assert!(refused.contains("at most 4 bytes"), "{refused}");
}

#[test]
fn a_prebuilt_vocabulary_encodes_a_venue_column_against_constant_codes() {
    let mut venues = AsciiDictionary::from_logical_name("Exchange").unwrap();
    let seeded = venues.len();
    let column = venues
        .into_arrow_array([Some("XCME"), Some("XLON"), None, Some("XCME")])
        .unwrap();
    // Every value was already in the constant, so the column registered none.
    assert_eq!(venues.len(), seeded);

    let encoded = column
        .as_any()
        .downcast_ref::<DictionaryArray<Int32Type>>()
        .unwrap();
    let xcme = i32::try_from(venues.get_code("XCME").unwrap()).unwrap();
    let xlon = i32::try_from(venues.get_code("XLON").unwrap()).unwrap();
    assert_eq!(
        encoded.keys().iter().collect::<Vec<_>>(),
        [Some(xcme), Some(xlon), None, Some(xcme)]
    );
    assert_eq!(encoded.values().len(), seeded);

    // The vocabulary recovered from the array is the one that wrote it, so a
    // reader that only has the column still reads the same codes.
    assert_eq!(AsciiDictionary::from_arrow_array(encoded).unwrap(), venues);

    // Another process building the same prebuilt vocabulary agrees on every
    // code, which is what a constant seed buys over auto-registration alone.
    assert_eq!(
        AsciiDictionary::from_logical_name("mic")
            .unwrap()
            .get_code("XCME"),
        Some(i64::from(xcme))
    );
}
