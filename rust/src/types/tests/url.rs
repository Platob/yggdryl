use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use arrow_array::{Array, RecordBatch, StringArray};
use arrow_schema::DataType as ArrowDataType;

use super::super::DataType;
use crate::arrow::{scalar_array, scalar_value};
use crate::{
    ArrowCast, ArrowCastOptions, DataTypeId, DataTypeKind, Field, FieldScalar, Scalar, Url,
    UrlField,
};

fn url(text: &str) -> Scalar {
    DataType::Url.scalar(text).unwrap()
}

fn root(field: Field) -> Field {
    DataType::from_fields([field])
        .unwrap()
        .required_field("row")
}

fn text_of(value: &Scalar) -> String {
    match value {
        Scalar::Url(url) => url.to_string(),
        other => panic!("expected a url scalar, got {other:?}"),
    }
}

#[test]
fn datatype_identity_naming_and_serde_are_total() {
    let dtype = DataType::Url;
    assert_eq!(dtype.id(), DataTypeId::Url);
    assert_eq!(dtype.kind(), DataTypeKind::Text);
    assert_eq!(dtype.name(), "url");
    assert_eq!(dtype.to_string(), "url");
    assert_eq!("URL".parse::<DataType>().unwrap(), dtype);
    assert_eq!("URL".parse::<DataTypeId>().unwrap(), DataTypeId::Url);
    assert_eq!(DataTypeId::Url.as_str(), "url");
    // Appended last, because `as_u8` is a wire contract.
    assert_eq!(DataTypeId::Url.as_u8(), 59);
    assert_eq!(DataTypeId::Url.fixed_byte_width(), None);
    assert!(!DataTypeId::Url.is_parameterized());
    assert!(DataTypeId::Url.is_string());
    assert!(!dtype.is_nested());
    dtype.validate().unwrap();

    assert_eq!(dtype.clone().into_json().unwrap(), r#"{"type":"url"}"#);
    assert_eq!(DataType::from_json(r#"{"type":"url"}"#).unwrap(), dtype);
}

#[test]
fn a_value_is_canonicalized_and_refuses_what_is_not_a_location() {
    // A scheme folds to lower case and percent-encoding to upper: one spelling
    // per location, whichever spelling was written.
    assert_eq!(
        text_of(&url("HTTPS://example.com/a%2fb?x=1#f")),
        "https://example.com/a%2Fb?x=1#f"
    );
    // A bare platform path is a `file:` URL, which is what the holders address
    // themselves by.
    assert_eq!(text_of(&url("/lake/part.txt")), "file:///lake/part.txt");
    // Re-reading the canonical text answers the same value.
    let once = url("HTTPS://example.com/a");
    assert_eq!(url(&text_of(&once)), once);

    // Relative text names no location, so it is not one.
    assert!(DataType::Url.scalar("./relative").is_err());
    assert!(DataType::Url.scalar("example.com/x").is_err());
    assert!(DataType::Url.scalar("").is_err());
    assert_eq!(DataType::Url.scalar(Scalar::Null).unwrap(), Scalar::Null);
}

#[test]
fn the_scalar_carries_the_datatype_and_orders_by_canonical_text() {
    let value = url("https://example.com/b");
    assert_eq!(value.id(), DataTypeId::Url);
    assert_eq!(value.kind(), "url");

    // Lexicographic on the canonical text, which is Arrow's own string
    // ordering over the storage - there is no numeric component to sort by.
    let mut sorted = [
        url("https://example.com/b"),
        url("file:///a"),
        url("https://example.com/a"),
    ];
    sorted.sort();
    assert_eq!(
        sorted.iter().map(text_of).collect::<Vec<_>>(),
        [
            "file:///a",
            "https://example.com/a",
            "https://example.com/b"
        ]
    );

    // Equal values hash equally, which is what a key column needs.
    let hash = |value: &Scalar| {
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    };
    assert_eq!(
        hash(&url("HTTPS://example.com/a")),
        hash(&url("https://example.com/a"))
    );
}

#[test]
fn structured_text_round_trips_the_canonical_spelling() {
    let value = url("https://example.com/a");
    // The tagged structural form is what carries the datatype back, so a
    // value written as a URL reads as one rather than as text.
    let tagged = serde_json::to_string(&value).unwrap();
    assert_eq!(tagged, r#"{"type":"url","value":"https://example.com/a"}"#);
    assert_eq!(serde_json::from_str::<Scalar>(&tagged).unwrap(), value);
}

#[test]
fn arrow_stores_canonical_utf8_under_an_extension_name_that_survives_a_round_trip() {
    let field = Field::new("location", DataType::Url, true);
    let arrow = field.clone().into_arrow().unwrap();
    assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
    assert_eq!(
        arrow
            .metadata()
            .get("ARROW:extension:name")
            .map(String::as_str),
        Some("yggdryl.url")
    );
    // The extension name is what makes a URL column come back a URL column
    // rather than prose that happens to look like one.
    assert_eq!(Field::from_arrow(&arrow).unwrap().dtype(), &DataType::Url);

    let value = url("https://example.com/a");
    let stored = scalar_array(&field, &value).unwrap();
    assert_eq!(stored.data_type(), &ArrowDataType::Utf8);
    assert_eq!(
        stored
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap()
            .value(0),
        "https://example.com/a"
    );
    assert_eq!(scalar_value(&field, stored.as_ref()).unwrap(), value);
}

#[test]
fn a_text_column_is_ingested_and_canonicalized_and_a_bad_row_names_itself() {
    let target = root(Field::new("location", DataType::Url, true));
    let source_schema = root(Field::new("location", DataType::utf8(), true))
        .into_arrow_schema()
        .unwrap();
    let batch = RecordBatch::try_new(
        Arc::clone(&source_schema),
        vec![Arc::new(StringArray::from(vec![
            Some("HTTPS://example.com/a"),
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
    // Canonicalized on the way in, so the column holds one spelling per
    // location however each row was written.
    assert_eq!(column.value(0), "https://example.com/a");
    assert!(column.is_null(1));

    let bad = RecordBatch::try_new(
        source_schema,
        vec![Arc::new(StringArray::from(vec![Some("./relative")]))],
    )
    .unwrap();
    let error = target
        .cast_arrow_batch(bad, ArrowCastOptions::new().with_safe(false))
        .unwrap_err()
        .to_string();
    assert!(error.contains("row 0"), "{error}");
    assert!(error.contains("does not read as url"), "{error}");
}

#[test]
fn defaults_merges_and_typed_fields_do_not_fall_through() {
    // A location has no zero, so the default is the shortest URL the
    // validator accepts.
    assert_eq!(text_of(&DataType::Url.default_value().unwrap()), "file:///");
    assert!(DataType::Url.is_default_value(&url("file:///")).unwrap());

    // Merging into text would drop the validation that makes it a URL, so
    // only an equal type merges.
    assert_eq!(
        DataType::Url.merge_with(&DataType::Url, true).unwrap(),
        DataType::Url
    );
    let refused = DataType::Url
        .merge_with(&DataType::utf8(), true)
        .unwrap_err()
        .to_string();
    assert!(refused.contains("url"), "{refused}");
    assert!(refused.contains("utf8"), "{refused}");

    let typed = UrlField::new("location", true);
    assert_eq!(typed.dtype(), &DataType::Url);
    let scalar = FieldScalar::new(typed.as_field(), url("https://example.com/a")).unwrap();
    assert_eq!(scalar.dtype(), &DataType::Url);
    assert!(FieldScalar::new(typed.as_field(), 7_i64).is_err());
}

#[test]
fn the_value_type_is_the_one_the_handles_address_themselves_by() {
    // Not a second URL type: the scalar carries `crate::Url`, so a column read
    // out of a table is the value a handle can be opened from.
    let value = url("file:///lake/part.txt");
    let Scalar::Url(held) = &value else {
        panic!("expected a url scalar")
    };
    let held: &Url = held;
    assert_eq!(held.file_name(), Some("part.txt"));
    assert_eq!(held.scheme().as_str(), "file");
}
