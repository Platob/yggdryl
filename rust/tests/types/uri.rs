use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use arrow_array::{Array, RecordBatch, StringArray};
use arrow_schema::DataType as ArrowDataType;

use yggdryl::DataType;
use yggdryl::FieldValue as _;
use yggdryl::arrow::{scalar_array, scalar_value};
use yggdryl::{
    ArrowCastOptions, DataTypeId, DataTypeKind, DataTypeValue as _, Field, FieldScalar, Scalar,
    StructType, Uri, UriField, UriType, Urn,
};

fn urn(text: &str) -> Scalar {
    DataType::urn().scalar(text).unwrap()
}

fn root(field: Field) -> Field {
    StructType::from_fields([field])
        .map(DataType::from)
        .unwrap()
        .required_field("row")
}

fn text_of(value: &Scalar) -> String {
    match value {
        Scalar::Urn(urn) => urn.to_string(),
        other => panic!("expected a urn scalar, got {other:?}"),
    }
}

#[test]
fn the_family_holds_two_leaves_and_names_them() {
    assert_eq!(UriType::ALL, [UriType::Url, UriType::Urn]);
    assert_eq!(DataType::url(), DataType::Uri(UriType::Url));
    assert_eq!(DataType::urn(), DataType::Uri(UriType::Urn));
    assert_eq!(DataType::url().uri_type(), Some(UriType::Url));
    assert_eq!(DataType::urn().uri_type(), Some(UriType::Urn));
    assert_eq!(DataType::utf8().uri_type(), None);
    for leaf in UriType::ALL {
        assert_eq!(leaf.family(), "uri");
        assert_eq!(leaf.kind(), DataTypeKind::Text);
        assert_eq!(UriType::from_id(leaf.id()), Some(leaf));
        assert_eq!(leaf.to_string(), leaf.id().as_str());
        assert_eq!(DataType::from(leaf), DataType::Uri(leaf));
        assert_eq!(UriType::from_dtype(&DataType::Uri(leaf)), Some(leaf));
        leaf.validate().unwrap();
    }
    assert_eq!(UriType::from_id(DataTypeId::Utf8String), None);
    // One field for the family, the leaf being what it holds.
    let typed = UriField::new("name", UriType::Urn, true);
    assert_eq!(typed.dtype(), &DataType::urn());
    assert_eq!(typed.typed_dtype(), UriType::Urn);
    let typed_field = typed.to_field();
    let scalar = FieldScalar::new(&typed_field, urn("urn:isbn:0451450523")).unwrap();
    assert_eq!(scalar.dtype(), &DataType::urn());
    assert!(FieldScalar::new(&typed.to_field(), 7_i64).is_err());
}

#[test]
fn datatype_identity_naming_and_serde_are_total() {
    let dtype = DataType::urn();
    assert_eq!(dtype.id(), DataTypeId::Urn);
    assert_eq!(dtype.kind(), DataTypeKind::Text);
    assert_eq!(dtype.name(), "urn");
    assert_eq!(dtype.to_string(), "urn");
    assert_eq!("URN".parse::<DataType>().unwrap(), dtype);
    assert_eq!("URN".parse::<DataTypeId>().unwrap(), DataTypeId::Urn);
    assert_eq!(DataTypeId::Urn.as_str(), "urn");
    // In the text family's range, because `as_u8` is a wire contract laid
    // out by family.
    assert_eq!(DataTypeId::Urn.as_u8(), 0x65);
    assert_eq!(DataTypeId::Urn.kind(), yggdryl::DataTypeKind::Text);
    assert_eq!(DataTypeId::Urn.fixed_byte_width(), None);
    assert!(!DataTypeId::Urn.is_parameterized());
    assert!(DataTypeId::Urn.is_string());
    assert!(!dtype.is_nested());
    dtype.validate().unwrap();

    assert_eq!(dtype.clone().into_json().unwrap(), r#"{"type":"urn"}"#);
    assert_eq!(DataType::from_json(r#"{"type":"urn"}"#).unwrap(), dtype);
    // Two leaves: a name is not a location, whatever text they share.
    assert_ne!(dtype, DataType::url());
}

#[test]
fn a_value_is_canonicalized_and_refuses_what_is_not_a_name() {
    // The scheme and the namespace fold to lower case: one spelling per
    // name, whichever spelling was written.
    assert_eq!(text_of(&urn("URN:ISBN:0451450523")), "urn:isbn:0451450523");
    assert_eq!(text_of(&urn("urn:example:a%20b")), "urn:example:a%20b");
    // Re-reading the canonical text answers the same value.
    let once = urn("URN:ISBN:0451450523");
    assert_eq!(urn(&text_of(&once)), once);

    // A location is not a name, and a name is not a location: each leaf
    // refuses what the other holds.
    assert!(DataType::urn().scalar("https://example.com/a").is_err());
    assert!(DataType::urn().scalar("/lake/part.txt").is_err());
    assert!(DataType::url().scalar("urn:isbn:0451450523").is_err());
    // Nothing without a namespace and a specific string is a name.
    assert!(DataType::urn().scalar("urn:isbn").is_err());
    // An empty text cell entering a non-text column is no value.
    assert_eq!(DataType::urn().scalar("").unwrap(), Scalar::Null);
    assert_eq!(DataType::urn().scalar(Scalar::Null).unwrap(), Scalar::Null);
    // A url scalar is not a urn scalar: the column decides the value.
    assert!(
        DataType::urn()
            .scalar(DataType::url().scalar("file:///a").unwrap())
            .is_err()
    );
}

#[test]
fn the_scalar_carries_the_datatype_and_orders_by_canonical_text() {
    let value = urn("urn:isbn:0451450523");
    assert_eq!(value.id(), DataTypeId::Urn);
    assert_eq!(value.kind(), "urn");
    assert_eq!(value.dtype().unwrap(), DataType::urn());

    // Lexicographic on the canonical text, which is Arrow's own string
    // ordering over the storage - there is no numeric component to sort by.
    let mut sorted = [
        urn("urn:isbn:0451450523"),
        urn("urn:example:b"),
        urn("urn:example:a"),
    ];
    sorted.sort();
    assert_eq!(
        sorted.iter().map(text_of).collect::<Vec<_>>(),
        ["urn:example:a", "urn:example:b", "urn:isbn:0451450523"]
    );

    // Equal values hash equally, which is what a key column needs.
    let hash = |value: &Scalar| {
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    };
    assert_eq!(
        hash(&urn("URN:ISBN:0451450523")),
        hash(&urn("urn:isbn:0451450523"))
    );
    // Both leaves narrow one identifier, and the scalar lends it out.
    let location = DataType::url().scalar("file:///a").unwrap();
    assert_eq!(value.as_uri().map(|uri| uri.scheme().as_str()), Some("urn"));
    assert_eq!(
        location.as_uri().map(|uri| uri.scheme().as_str()),
        Some("file")
    );
    assert_eq!(Scalar::from(7_i64).as_uri(), None);
    assert_ne!(value, location);
}

#[test]
fn structured_text_round_trips_the_canonical_spelling() {
    let value = urn("urn:isbn:0451450523");
    // The tagged structural form is what carries the datatype back, so a
    // value written as a URN reads as one rather than as text.
    let tagged = serde_json::to_string(&value).unwrap();
    assert_eq!(tagged, r#"{"type":"urn","value":"urn:isbn:0451450523"}"#);
    assert_eq!(serde_json::from_str::<Scalar>(&tagged).unwrap(), value);
}

#[test]
fn arrow_stores_canonical_utf8_under_an_extension_name_that_survives_a_round_trip() {
    let field = Field::new("name", DataType::urn(), true);
    let arrow = field.clone().into_arrow_field().unwrap();
    assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
    assert_eq!(
        arrow
            .metadata()
            .get("ARROW:extension:name")
            .map(String::as_str),
        Some("yggdryl.urn")
    );
    // The extension name is what makes a URN column come back a URN column
    // rather than a URL column or prose.
    assert_eq!(
        Field::from_arrow_field(&arrow).unwrap().dtype(),
        &DataType::urn()
    );

    let value = urn("urn:isbn:0451450523");
    let stored = scalar_array(&field, &value).unwrap();
    assert_eq!(stored.data_type(), &ArrowDataType::Utf8);
    assert_eq!(
        stored
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap()
            .value(0),
        "urn:isbn:0451450523"
    );
    assert_eq!(scalar_value(&field, stored.as_ref()).unwrap(), value);
}

#[test]
fn a_text_column_is_ingested_and_canonicalized_and_a_bad_row_names_itself() {
    let target = root(Field::new("name", DataType::urn(), true));
    let source_schema = root(Field::new("name", DataType::utf8(), true))
        .into_arrow_schema()
        .unwrap();
    let batch = RecordBatch::try_new(
        Arc::clone(&source_schema),
        vec![Arc::new(StringArray::from(vec![
            Some("URN:ISBN:0451450523"),
            Some("urn:example:a%20b"),
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
    // Canonicalized on the way in, so the column holds one spelling per name
    // however each row was written.
    assert_eq!(column.value(0), "urn:isbn:0451450523");
    assert_eq!(column.value(1), "urn:example:a%20b");
    assert!(column.is_null(2));

    let bad = RecordBatch::try_new(
        source_schema,
        vec![Arc::new(StringArray::from(vec![Some(
            "https://example.com/a",
        )]))],
    )
    .unwrap();
    let error = target
        .cast_arrow_batch(bad, ArrowCastOptions::new().with_safe(false))
        .unwrap_err()
        .to_string();
    assert!(error.contains("row 0"), "{error}");
    assert!(error.contains("does not read as urn"), "{error}");
}

#[test]
fn defaults_and_merges_do_not_fall_through() {
    // A name has no zero, so the default is the nil name: the shortest URN
    // the validator accepts, spelled as the nothing it names.
    assert_eq!(
        text_of(&DataType::urn().default_value().unwrap()),
        "urn:nil:nil"
    );
    assert!(
        DataType::urn()
            .is_default_value(&urn("urn:nil:nil"))
            .unwrap()
    );

    // Merging into text, or into the other leaf, would drop the rule that
    // makes it a name, so only an equal type merges.
    assert_eq!(
        DataType::urn().merge_with(&DataType::urn(), true).unwrap(),
        DataType::urn()
    );
    for other in [DataType::utf8(), DataType::url()] {
        let refused = DataType::urn()
            .merge_with(&other, true)
            .unwrap_err()
            .to_string();
        assert!(refused.contains("urn"), "{refused}");
        assert!(refused.contains(other.name()), "{refused}");
    }
}

#[test]
fn the_value_type_is_the_one_the_handles_address_themselves_by() {
    // Not a second URN type: the scalar carries `yggdryl::Urn`, the narrowing
    // of the one `Uri` every handle and every URL is built on.
    let value = urn("urn:isbn:0451450523");
    let Scalar::Urn(held) = &value else {
        panic!("expected a urn scalar")
    };
    let held: &Urn = held;
    assert_eq!(held.scheme().as_str(), "urn");
    assert_eq!(held.path().as_str(), "isbn:0451450523");
    let uri: Uri = held.clone().into_uri();
    assert!(uri.clone().into_url().is_err());
    assert_eq!(uri.into_urn().unwrap(), held.clone());
}
