use std::collections::HashMap;
use std::sync::Arc;

use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, ffi::Flags};
use yggdryl::{DataType, Field, TimeUnit};

fn assert_flag(schema: &arrow_schema::ffi::FFI_ArrowSchema, flag: Flags) {
    assert!(schema.flags().unwrap().contains(flag));
}

fn assert_canonical_http_metadata(field: &ArrowField) {
    assert_eq!(field.metadata().len(), 2);
    assert_eq!(
        field
            .metadata()
            .get("http:content-type")
            .map(String::as_str),
        Some("application/json")
    );
    assert_eq!(
        field
            .metadata()
            .get("http:content-length")
            .map(String::as_str),
        Some("42")
    );
    assert!(!field.metadata().contains_key("HTTPS:Content-Type"));
    assert!(!field.metadata().contains_key("HTTP:Content-Length"));
}

#[test]
fn arrow_import_rebuilds_noncanonical_http_metadata_for_every_ownership_path() {
    let arrow =
        ArrowField::new("payload", ArrowDataType::Binary, false).with_metadata(HashMap::from([
            (
                "HTTPS:Content-Type".to_owned(),
                "application/json".to_owned(),
            ),
            ("HTTP:Content-Length".to_owned(), "00042".to_owned()),
        ]));

    let borrowed = Field::from_arrow(&arrow).unwrap();
    assert_canonical_http_metadata(&borrowed.to_arrow().unwrap());

    let shared = Arc::new(arrow.clone());
    let shared_import = Field::from_arrow_ref(Arc::clone(&shared)).unwrap();
    let shared_projection = shared_import.to_arrow_ref().unwrap();
    assert!(!Arc::ptr_eq(&shared, &shared_projection));
    assert_canonical_http_metadata(shared_projection.as_ref());

    let owned = Field::try_from(arrow).unwrap();
    assert_canonical_http_metadata(&owned.into_arrow().unwrap());
}

#[test]
fn core_ffi_projection_preserves_every_field_and_datatype_flag_recursively() {
    let mut encoded = Field::from_parts(
        "codes",
        DataType::dictionary(DataType::UInt16, DataType::Utf8).unwrap(),
        true,
        [("ARROW:extension:name", "catalog-code")],
    )
    .unwrap();
    encoded.set_dictionary_options(17, true).unwrap();

    let entries = Field::new(
        "entries",
        DataType::from_fields([Field::new("key", DataType::Utf8, false), encoded]).unwrap(),
        false,
    );
    let map = DataType::map(entries, true).unwrap();
    let root = Field::from_parts("lookup", map, true, [("owner", "core")]).unwrap();

    let schema = root.to_arrow_ffi().unwrap();
    assert_eq!(schema.name(), Some("lookup"));
    assert_eq!(
        schema.metadata().unwrap().get("owner"),
        Some(&"core".into())
    );
    assert_flag(&schema, Flags::NULLABLE);
    assert_flag(&schema, Flags::MAP_KEYS_SORTED);

    let entries = schema.child(0);
    assert_eq!(entries.name(), Some("entries"));
    assert!(!entries.flags().unwrap().contains(Flags::NULLABLE));
    let encoded = entries.child(1);
    assert_eq!(encoded.name(), Some("codes"));
    assert_eq!(
        encoded.metadata().unwrap().get("ARROW:extension:name"),
        Some(&"catalog-code".into())
    );
    assert_flag(encoded, Flags::NULLABLE);
    assert_flag(encoded, Flags::DICTIONARY_ORDERED);
    assert!(encoded.dictionary().is_some());

    root.to_arrow_ref().unwrap();
    let cached = root.to_arrow_ffi().unwrap();
    assert_flag(&cached, Flags::NULLABLE);
    assert_flag(&cached, Flags::MAP_KEYS_SORTED);
    assert_flag(cached.child(0).child(1), Flags::DICTIONARY_ORDERED);
}

#[test]
fn datatype_ffi_projection_preserves_nested_map_flags_and_rejects_invalid_state() {
    let map = DataType::map_of(DataType::Utf8, DataType::Int64, true).unwrap();
    let data_type = DataType::from_fields([Field::new("lookup", map, true)]).unwrap();

    let schema = data_type.to_arrow_ffi().unwrap();
    let map = schema.child(0);
    assert_flag(map, Flags::NULLABLE);
    assert_flag(map, Flags::MAP_KEYS_SORTED);

    assert!(DataType::FixedSizeBinary(-1).to_arrow_ffi().is_err());
    assert!(
        Field::new("bad", DataType::Timestamp(TimeUnit::YearMonth, None), false,)
            .to_arrow_ffi()
            .is_err()
    );
}
