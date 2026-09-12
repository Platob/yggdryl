//! A row declared in FIX's datatype names is an ordinary row everywhere else.

use arrow_schema::DataType as ArrowDataType;
use yggdryl::{DataType, DataTypeId, Field, Scalar, StringEnum, TimeUnit, Timezone};

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
            // `Currency` and `Exchange` resolve to datatypes of their own,
            // each storing the width its standard fixes.
            ("ccy", &ArrowDataType::FixedSizeBinary(3)),
            ("venue", &ArrowDataType::FixedSizeBinary(4)),
            // The float family is FIX `float`, which states no scale.
            ("px", &ArrowDataType::Float64),
            ("qty", &ArrowDataType::Float64),
            (
                "at",
                &ArrowDataType::Timestamp(arrow_schema::TimeUnit::Nanosecond, Some("UTC".into()))
            ),
            // A FIX date is that day's midnight, and a local market date
            // states no zone - so it crosses as a zone-less timestamp rather
            // than as a day a consumer would have to cast to compare.
            (
                "day",
                &ArrowDataType::Timestamp(arrow_schema::TimeUnit::Nanosecond, None)
            ),
            ("seq", &ArrowDataType::Int64),
        ]
    );

    // The names are inert: what came back is the datatype, not a registration,
    // so the Arrow round trip is the ordinary one.
    assert_eq!(Field::from_arrow_schema("row", &schema).unwrap(), row);
    assert_eq!(
        row.dtype().get_field_by_path("at").map(Field::dtype),
        Some(&DataType::DateTime64 {
            unit: TimeUnit::Nanosecond,
            timezone: Timezone::UTC
        })
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
        "at":"2026-09-04T10:00:00.000000001Z","day":"2026-09-04T00:00:00","seq":9}"#;
    let value = yggdryl::text::json::from_utf8_with_field(message, &row).unwrap();
    let columns = value.as_sequence().expect("a canonical row");

    assert_eq!(columns[0].id(), DataTypeId::Currency);
    assert_eq!(columns[0].as_str(), Some("USD"));
    assert_eq!(columns[1].id(), DataTypeId::Mic);
    assert_eq!(columns[1].as_str(), Some("XCME"));
    // The specification declares the float family as `float` and states no
    // scale, so a price reads as a double and the exact characters stay in
    // the message the row was typed from.
    assert_eq!(columns[2], Scalar::from(101.25_f64));
    assert_eq!(columns[3], Scalar::from(7.0_f64));
    assert_eq!(columns[6], Scalar::from(9_i64));

    // The instant reads at nanoseconds, so the fraction survives.
    let at = columns[4].as_temporal().expect("a temporal reading");
    assert_eq!(at.count(), 1_788_516_000_000_000_001);
    assert_eq!(at.unit(), TimeUnit::Nanosecond);
    let day = columns[5].as_temporal().expect("a temporal reading");
    // The same day, now counted from the epoch in nanoseconds because a date
    // is an instant at midnight.
    assert_eq!(day.count(), 20_700 * 86_400 * 1_000_000_000);
    assert_eq!(day.unit(), TimeUnit::Nanosecond);

    // A value that does not fit the resolved datatype is refused by that
    // datatype, never by the name that spelled it.
    let refused = yggdryl::text::json::from_utf8_with_field(
        r#"{"ccy":"EURO!","venue":"XCME","px":"1","qty":"1","at":"2026-09-04T10:00:00Z","day":"2026-09-04T00:00:00","seq":1}"#,
        &row,
    )
    .unwrap_err()
    .to_string();
    assert!(refused.contains("at most 3 bytes"), "{refused}");
}

#[test]
fn a_prebuilt_vocabulary_declares_the_codes_a_venue_column_carries() {
    let venues = StringEnum::from_logical_name("Exchange").unwrap();
    assert_eq!(venues.len(), StringEnum::MICS.len());
    assert_eq!(venues.get("XCME"), Some("XCME"));

    // The listing is what a field declares, so a venue column crosses Arrow
    // carrying the vocabulary its values come from.
    let venue = Field::new("venue", DataType::Mic, false)
        .try_with_string_enum(&venues)
        .unwrap();
    let recovered = Field::from_arrow(&venue.clone().into_arrow().unwrap()).unwrap();
    assert_eq!(recovered, venue);
    assert_eq!(recovered.string_enum().unwrap().as_ref(), Some(&venues));

    // A member's code is the value's own bytes under the resolved width, so
    // two processes reading this schema answer the same integers.
    let members = venues.into_members(&DataType::Mic).unwrap();
    for (member, code) in &members {
        assert_eq!(
            *code,
            DataType::Mic.ascii_packed(member.as_bytes()).unwrap()
        );
    }
    assert_eq!(
        StringEnum::from_logical_name("mic")
            .unwrap()
            .into_members(&DataType::Mic)
            .unwrap(),
        members
    );
}
