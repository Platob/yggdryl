use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use arrow_array::{Array, RecordBatch, StringArray};
use arrow_schema::DataType as ArrowDataType;

use super::super::DataType;
use crate::arrow::{scalar_array, scalar_value};
use crate::{
    ArrowCast, ArrowCastOptions, DataTypeId, DataTypeKind, Field, FieldScalar, Scalar, Timezone,
    TimezoneField,
};

fn zone(text: &str) -> Scalar {
    DataType::Timezone.scalar(text).unwrap()
}

fn root(field: Field) -> Field {
    DataType::from_fields([field])
        .unwrap()
        .required_field("row")
}

fn text_of(value: &Scalar) -> String {
    match value {
        Scalar::Timezone(zone) => zone.as_str().to_owned(),
        other => panic!("expected a timezone scalar, got {other:?}"),
    }
}

#[test]
fn datatype_identity_naming_and_serde_are_total() {
    let dtype = DataType::Timezone;
    assert_eq!(dtype.id(), DataTypeId::Timezone);
    assert_eq!(dtype.kind(), DataTypeKind::Text);
    assert_eq!(dtype.name(), "timezone");
    assert_eq!(dtype.to_string(), "timezone");
    assert_eq!("TIMEZONE".parse::<DataType>().unwrap(), dtype);
    // Both other spellings a schema is written in reach the one datatype.
    assert_eq!("tz".parse::<DataType>().unwrap(), dtype);
    assert_eq!("timezone_name".parse::<DataType>().unwrap(), dtype);
    assert_eq!(
        "TIMEZONE".parse::<DataTypeId>().unwrap(),
        DataTypeId::Timezone
    );
    assert_eq!(DataTypeId::Timezone.as_str(), "timezone");
    // Appended, because `as_u8` is a wire contract.
    assert_eq!(DataTypeId::Timezone.as_u8(), 61);
    assert_eq!(DataTypeId::Timezone.fixed_byte_width(), None);
    assert!(!DataTypeId::Timezone.is_parameterized());
    assert!(!dtype.is_nested());
    dtype.validate().unwrap();

    assert_eq!(dtype.clone().into_json().unwrap(), r#"{"type":"timezone"}"#);
    assert_eq!(
        DataType::from_json(r#"{"type":"timezone"}"#).unwrap(),
        dtype
    );
}

#[test]
fn a_value_is_canonicalized_and_refuses_what_names_no_zone() {
    // An alias resolves to what it stands for and case folds, so two
    // spellings of one zone are one value.
    assert_eq!(text_of(&zone("Asia/Calcutta")), "Asia/Kolkata");
    assert_eq!(text_of(&zone("utc")), "UTC");
    assert_eq!(zone("Z"), zone("UTC"));
    // A fixed offset normalizes to `+HH:MM`.
    assert_eq!(text_of(&zone("-0800")), "-08:00");
    // The zone-free marker is a value like any other.
    assert_eq!(zone("NAIVE"), Scalar::Timezone(Timezone::NAIVE));
    // Re-reading the canonical text answers the same value.
    let once = zone("Asia/Calcutta");
    assert_eq!(zone(&text_of(&once)), once);

    assert!(DataType::Timezone.scalar("").is_err());
    assert!(DataType::Timezone.scalar("+99:00").is_err());
    assert_eq!(
        DataType::Timezone.scalar(Scalar::Null).unwrap(),
        Scalar::Null
    );
}

#[test]
fn the_scalar_carries_the_datatype_and_orders_by_canonical_name() {
    let value = zone("America/New_York");
    assert_eq!(value.id(), DataTypeId::Timezone);
    assert_eq!(value.kind(), "timezone");

    // Equal values hash equally, which is what a key column needs.
    let hash = |value: &Scalar| {
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    };
    assert_eq!(hash(&zone("Asia/Calcutta")), hash(&zone("Asia/Kolkata")));
    assert_ne!(hash(&zone("UTC")), hash(&zone("America/New_York")));
}

#[test]
fn structured_text_round_trips_the_canonical_spelling() {
    let value = zone("Asia/Calcutta");
    let tagged = serde_json::to_string(&value).unwrap();
    assert_eq!(tagged, r#"{"type":"timezone","value":"Asia/Kolkata"}"#);
    assert_eq!(serde_json::from_str::<Scalar>(&tagged).unwrap(), value);
}

#[test]
fn arrow_stores_canonical_utf8_under_an_extension_name_that_survives_a_round_trip() {
    let field = Field::new("zone", DataType::Timezone, true);
    let arrow = field.clone().into_arrow().unwrap();
    assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
    assert_eq!(
        arrow
            .metadata()
            .get("ARROW:extension:name")
            .map(String::as_str),
        Some("yggdryl.timezone")
    );
    assert_eq!(
        Field::from_arrow(&arrow).unwrap().dtype(),
        &DataType::Timezone
    );

    let value = zone("America/New_York");
    let stored = scalar_array(&field, &value).unwrap();
    assert_eq!(stored.data_type(), &ArrowDataType::Utf8);
    assert_eq!(
        stored
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap()
            .value(0),
        "America/New_York"
    );
    assert_eq!(scalar_value(&field, stored.as_ref()).unwrap(), value);
}

#[test]
fn a_text_column_is_ingested_and_canonicalized_and_a_bad_row_names_itself() {
    let target = root(Field::new("zone", DataType::Timezone, true));
    let source_schema = root(Field::new("zone", DataType::utf8(), true))
        .into_arrow_schema()
        .unwrap();
    let batch = RecordBatch::try_new(
        Arc::clone(&source_schema),
        vec![Arc::new(StringArray::from(vec![
            Some("Asia/Calcutta"),
            None,
        ]))],
    )
    .unwrap();
    let cast = target
        .cast_arrow_batch(batch, ArrowCastOptions::new().with_safe(false))
        .unwrap();
    let column = cast
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert_eq!(column.value(0), "Asia/Kolkata");
    assert!(column.is_null(1));

    let bad = RecordBatch::try_new(
        source_schema,
        vec![Arc::new(StringArray::from(vec![Some("+99:00")]))],
    )
    .unwrap();
    let error = target
        .cast_arrow_batch(bad, ArrowCastOptions::new().with_safe(false))
        .unwrap_err()
        .to_string();
    assert!(error.contains("row 0"), "{error}");
    assert!(error.contains("does not read as time zone"), "{error}");
}

#[test]
fn defaults_merges_and_typed_fields_do_not_fall_through() {
    // The zone-free marker is the zero every temporal already defaults to.
    assert_eq!(
        text_of(&DataType::Timezone.default_value().unwrap()),
        "NAIVE"
    );
    assert!(DataType::Timezone.is_default_value(&zone("NAIVE")).unwrap());

    assert_eq!(
        DataType::Timezone
            .merge_with(&DataType::Timezone, true)
            .unwrap(),
        DataType::Timezone
    );
    let refused = DataType::Timezone
        .merge_with(&DataType::utf8(), true)
        .unwrap_err()
        .to_string();
    assert!(refused.contains("timezone"), "{refused}");

    let typed = TimezoneField::new("zone", true);
    assert_eq!(typed.dtype(), &DataType::Timezone);
    let scalar = FieldScalar::new(typed.as_field(), zone("UTC")).unwrap();
    assert_eq!(scalar.dtype(), &DataType::Timezone);
    assert!(FieldScalar::new(typed.as_field(), 7_i64).is_err());
}

#[test]
fn the_value_type_is_the_one_every_temporal_already_carries() {
    // Not a second zone type: the scalar carries `crate::Timezone`, so a zone
    // read out of a column is the zone a datetime column declares.
    let value = zone("America/New_York");
    let Scalar::Timezone(held) = &value else {
        panic!("expected a timezone scalar")
    };
    let held: Timezone = *held;
    assert_eq!(held.offset_at(1_700_000_000), Some(-5 * 3600));
    assert_eq!(held.abbreviation_at(1_688_000_000), Some("EDT"));
    assert_eq!(
        Scalar::datetime64(0, crate::TimeUnit::Second, held)
            .unwrap()
            .temporal_timezone(),
        Some(held)
    );
}
