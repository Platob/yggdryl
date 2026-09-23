//! `rust/src/media_type/datatype.rs`: the two MIME-shaped datatypes: the
//! bare type, and the type with its charset and content codings.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use arrow_array::{Array, RecordBatch, StringArray};
use arrow_schema::DataType as ArrowDataType;

use yggdryl::DataType;
use yggdryl::arrow::{scalar_array, scalar_value};
use yggdryl::{
    ArrowCastOptions, Charset, DataTypeId, DataTypeKind, Field, FieldScalar, MediaType,
    MediaTypeField, MimeType, MimeTypeField, Scalar, Serie, StructType,
};

fn mime(text: &str) -> Scalar {
    DataType::MimeType.scalar(text).unwrap()
}

fn media(text: &str) -> Scalar {
    DataType::MediaType.scalar(text).unwrap()
}

fn root(field: Field) -> Field {
    StructType::from_fields([field])
        .map(DataType::from)
        .unwrap()
        .required_field("row")
}

fn mime_text(value: &Scalar) -> String {
    match value {
        Scalar::MimeType(value) => value.as_str().to_owned(),
        other => panic!("expected a mimetype scalar, got {other:?}"),
    }
}

fn media_text(value: &Scalar) -> String {
    match value {
        Scalar::MediaType(value) => value.to_string(),
        other => panic!("expected a mediatype scalar, got {other:?}"),
    }
}

#[test]
fn datatype_identity_naming_and_serde_are_total() {
    for (dtype, id, name, byte, alias) in [
        (
            DataType::MimeType,
            DataTypeId::MimeType,
            "mimetype",
            0x67_u8,
            "mime",
        ),
        (
            DataType::MediaType,
            DataTypeId::MediaType,
            "mediatype",
            0x68,
            "content_type",
        ),
    ] {
        assert_eq!(dtype.id(), id);
        assert_eq!(dtype.kind(), DataTypeKind::Text);
        assert_eq!(dtype.name(), name);
        assert_eq!(dtype.to_string(), name);
        assert_eq!(name.to_uppercase().parse::<DataType>().unwrap(), dtype);
        assert_eq!(alias.parse::<DataType>().unwrap(), dtype);
        assert_eq!(id.as_str(), name);
        // In the text family's range, because `as_u8` is a wire contract
        // laid out by family.
        assert_eq!(id.as_u8(), byte);
        assert!(!id.is_parameterized());
        assert!(!dtype.is_nested());
        dtype.validate().unwrap();

        let document = format!(r#"{{"type":"{name}"}}"#);
        assert_eq!(dtype.clone().into_json().unwrap(), document);
        assert_eq!(DataType::from_json(&document).unwrap(), dtype);
    }
}

#[test]
fn a_value_is_canonicalized_and_refuses_what_is_not_a_type() {
    // Case folds to the one canonical spelling, so two writings are one value.
    assert_eq!(mime_text(&mime("APPLICATION/JSON")), "application/json");
    assert_eq!(mime("application/json"), Scalar::from(MimeType::JSON));
    // An unregistered but well-formed name is kept as written, lower-cased.
    assert_eq!(
        mime_text(&mime("Application/Vnd.Acme+JSON")),
        "application/vnd.acme+json"
    );

    // A media type carries its charset and codings in one canonical rendering.
    assert_eq!(
        media_text(&media("APPLICATION/JSON; CHARSET=UTF-8")),
        "application/json;charset=utf-8"
    );
    // Re-reading the canonical text answers the same value.
    let once = media("application/json;charset=utf-8");
    assert_eq!(media(&media_text(&once)), once);

    // An empty text cell entering a non-text column is no value.
    assert_eq!(DataType::MimeType.scalar("").unwrap(), Scalar::Null);
    assert!(DataType::MimeType.scalar("not a type").is_err());
    // A media type's intake is total by construction - it is also the
    // filename and content-negotiation reader - so text naming no base is the
    // default base rather than a refusal. The column holds what that answers;
    // only the empty text is no value, as it is for every non-text column.
    assert_eq!(media_text(&media("README")), "application/octet-stream",);
    assert_eq!(DataType::MediaType.scalar("").unwrap(), Scalar::Null);
    assert_eq!(
        media_text(&media("part.tgz")),
        "application/x-tar;encodings=application/gzip"
    );
    assert_eq!(
        DataType::MimeType.scalar(Scalar::Null).unwrap(),
        Scalar::Null
    );
    assert_eq!(
        DataType::MediaType.scalar(Scalar::Null).unwrap(),
        Scalar::Null
    );
}

#[test]
fn the_scalars_carry_their_datatypes_and_hash_by_canonical_text() {
    assert_eq!(mime("text/csv").id(), DataTypeId::MimeType);
    assert_eq!(mime("text/csv").kind(), "mimetype");
    assert_eq!(media("text/csv").id(), DataTypeId::MediaType);
    assert_eq!(media("text/csv").kind(), "mediatype");

    let hash = |value: &Scalar| {
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    };
    assert_eq!(hash(&mime("TEXT/CSV")), hash(&mime("text/csv")));
    assert_eq!(
        hash(&media("APPLICATION/JSON; CHARSET=UTF-8")),
        hash(&media("application/json;charset=utf-8"))
    );
    // A bare type and the same type under a charset are two values.
    assert_ne!(media("text/csv"), media("text/csv; charset=utf-8"));
    // And the two datatypes never collapse into one another.
    assert_ne!(mime("text/csv"), media("text/csv"));
}

#[test]
fn structured_text_round_trips_the_canonical_spelling() {
    let value = mime("APPLICATION/JSON");
    let tagged = serde_json::to_string(&value).unwrap();
    assert_eq!(tagged, r#"{"type":"mimetype","value":"application/json"}"#);
    assert_eq!(serde_json::from_str::<Scalar>(&tagged).unwrap(), value);

    let value = media("APPLICATION/JSON; CHARSET=UTF-8");
    let tagged = serde_json::to_string(&value).unwrap();
    assert_eq!(
        tagged,
        r#"{"type":"mediatype","value":"application/json;charset=utf-8"}"#
    );
    assert_eq!(serde_json::from_str::<Scalar>(&tagged).unwrap(), value);
}

#[test]
fn arrow_stores_canonical_utf8_under_extension_names_that_survive_a_round_trip() {
    for (dtype, extension, value, stored_text) in [
        (
            DataType::MimeType,
            "yggdryl.mimetype",
            mime("application/json"),
            "application/json",
        ),
        (
            DataType::MediaType,
            "yggdryl.mediatype",
            media("application/json;charset=utf-8"),
            "application/json;charset=utf-8",
        ),
    ] {
        let field = Field::new("held", dtype.clone(), true);
        let arrow = field.clone().into_arrow_field().unwrap();
        assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
        assert_eq!(
            arrow
                .metadata()
                .get("ARROW:extension:name")
                .map(String::as_str),
            Some(extension)
        );
        assert_eq!(Field::from_arrow_field(&arrow).unwrap().dtype(), &dtype);

        let array = scalar_array(&field, &value).unwrap();
        assert_eq!(array.data_type(), &ArrowDataType::Utf8);
        assert_eq!(
            array
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap()
                .value(0),
            stored_text
        );
        assert_eq!(scalar_value(&field, array.as_ref()).unwrap(), value);
    }
}

#[test]
fn a_text_column_is_ingested_and_canonicalized_and_a_bad_row_names_itself() {
    let target = root(Field::new("held", DataType::MimeType, true));
    let source_schema = root(Field::new("held", DataType::utf8(), true))
        .into_arrow_schema()
        .unwrap();
    let batch = RecordBatch::try_new(
        Arc::clone(&source_schema),
        vec![Arc::new(StringArray::from(vec![
            Some("APPLICATION/JSON"),
            None,
        ]))],
    )
    .unwrap();
    let cast = Serie::from_arrow_batch(
        Some(&target),
        &batch,
        ArrowCastOptions::new().with_safe(false),
    )
    .unwrap()
    .into_arrow_batch()
    .unwrap();
    let column = cast
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert_eq!(column.value(0), "application/json");
    assert!(column.is_null(1));

    let bad = RecordBatch::try_new(
        source_schema,
        vec![Arc::new(StringArray::from(vec![Some("not a type")]))],
    )
    .unwrap();
    let error = Serie::from_arrow_batch(
        Some(&target),
        &bad,
        ArrowCastOptions::new().with_safe(false),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("row 0"), "{error}");
    assert!(error.contains("does not read as MIME type"), "{error}");
}

#[test]
fn defaults_merges_and_typed_fields_do_not_fall_through() {
    // Arbitrary bytes under no charset and no coding, which is what both
    // values already answer `Default` with.
    assert_eq!(
        mime_text(&DataType::MimeType.default_value().unwrap()),
        "application/octet-stream"
    );
    assert!(
        DataType::MimeType
            .is_default_value(&mime("application/octet-stream"))
            .unwrap()
    );
    assert_eq!(
        media_text(&DataType::MediaType.default_value().unwrap()),
        "application/octet-stream"
    );

    // The two never merge into each other or into text: a media type carries
    // a charset and codings a MIME type does not model.
    for (left, right) in [
        (DataType::MimeType, DataType::MediaType),
        (DataType::MediaType, DataType::utf8()),
    ] {
        assert!(left.merge_with(&right, true).is_err());
    }
    assert_eq!(
        DataType::MimeType
            .merge_with(&DataType::MimeType, true)
            .unwrap(),
        DataType::MimeType
    );

    let typed = MimeTypeField::unit("held", true);
    assert_eq!(typed.dtype(), &DataType::MimeType);
    assert!(FieldScalar::new(&typed.to_field(), 7_i64).is_err());
    let typed = MediaTypeField::unit("held", true);
    assert_eq!(typed.dtype(), &DataType::MediaType);
    let typed_field = typed.to_field();
    let scalar = FieldScalar::new(&typed_field, media("text/csv")).unwrap();
    assert_eq!(scalar.dtype(), &DataType::MediaType);
}

#[test]
fn the_value_types_are_the_ones_the_media_layer_routes_on() {
    // Not a second MIME type: the scalar carries `yggdryl::MimeType`, so a value
    // read out of a column is what `RecordOptions` routes a record read on.
    let Scalar::MimeType(held) = &mime("application/json") else {
        panic!("expected a mimetype scalar")
    };
    assert_eq!(held, &MimeType::JSON);
    assert_eq!(held.extension(), Some("json"));

    let Scalar::MediaType(held) = &media("text/csv; charset=utf-8") else {
        panic!("expected a mediatype scalar")
    };
    let held: &MediaType = held;
    assert_eq!(held.base(), &MimeType::CSV);
    assert_eq!(held.charset(), Some(Charset::Utf8));
}
