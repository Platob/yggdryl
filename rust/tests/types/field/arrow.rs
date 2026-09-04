use std::collections::HashMap;
use std::sync::Arc;

use arrow_schema::{
    DataType as ArrowDataType, Field as ArrowField, Schema,
    ffi::{FFI_ArrowSchema, Flags},
};
use yggdryl::arrow::IPC_DICTIONARY_IDS_KEY;
use yggdryl::{DataType, Field, TimeUnit, Timezone};

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
    assert_canonical_http_metadata(&borrowed.into_arrow().unwrap());

    let shared = Arc::new(arrow.clone());
    let shared_import = Field::from_arrow_ref(Arc::clone(&shared)).unwrap();
    let shared_projection = shared_import.into_arrow_ref().unwrap();
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

    let schema = root.clone().into_arrow_ffi().unwrap();
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

    root.clone().into_arrow_ref().unwrap();
    let cached = root.into_arrow_ffi().unwrap();
    assert_flag(&cached, Flags::NULLABLE);
    assert_flag(&cached, Flags::MAP_KEYS_SORTED);
    assert_flag(cached.child(0).child(1), Flags::DICTIONARY_ORDERED);
}

#[test]
fn datatype_ffi_projection_preserves_nested_map_flags_and_rejects_invalid_state() {
    let map = DataType::map_of(DataType::Utf8, DataType::Int64, true).unwrap();
    let dtype = DataType::from_fields([Field::new("lookup", map, true)]).unwrap();

    let schema = dtype.into_arrow_ffi().unwrap();
    let map = schema.child(0);
    assert_flag(map, Flags::NULLABLE);
    assert_flag(map, Flags::MAP_KEYS_SORTED);

    assert!(DataType::FixedSizeBinary(-1).into_arrow_ffi().is_err());
    assert!(
        Field::new(
            "bad",
            DataType::DateTime64 {
                unit: TimeUnit::YearMonth,
                timezone: Timezone::NAIVE
            },
            false,
        )
        .into_arrow_ffi()
        .is_err()
    );
}

#[test]
fn geospatial_and_variant_ffi_schemas_carry_the_extension_identity() {
    let geography = Field::from_parts(
        "region",
        DataType::geography(Some("EPSG:4326"), None).unwrap(),
        true,
        [("owner", "core")],
    )
    .unwrap();

    let schema = geography.clone().into_arrow_ffi().unwrap();
    let metadata = schema.metadata().unwrap();
    assert_eq!(
        metadata.get("ARROW:extension:name"),
        Some(&"geoarrow.wkb".to_owned())
    );
    let document = metadata.get("ARROW:extension:metadata").unwrap();
    assert!(document.contains("EPSG:4326"), "{document}");
    assert!(document.contains("spherical"), "{document}");
    assert_eq!(metadata.get("owner"), Some(&"core".to_owned()));

    // A round trip through the C schema restores the exact field.
    let imported = Field::from_arrow(&ArrowField::try_from(&schema).unwrap()).unwrap();
    assert_eq!(imported, geography);

    // The cached projection path serves the same identity.
    geography.clone().into_arrow_ref().unwrap();
    let cached = geography.into_arrow_ffi().unwrap();
    assert_eq!(
        cached.metadata().unwrap().get("ARROW:extension:name"),
        Some(&"geoarrow.wkb".to_owned())
    );

    // A bare datatype carries the identity too, variant included.
    let schema = DataType::variant().into_arrow_ffi().unwrap();
    let metadata = schema.metadata().unwrap();
    assert_eq!(
        metadata.get("ARROW:extension:name"),
        Some(&"arrow.parquet.variant".to_owned())
    );
    assert_eq!(
        metadata.get("ARROW:extension:metadata"),
        Some(&String::new())
    );
    let imported = Field::from_arrow(&ArrowField::try_from(&schema).unwrap()).unwrap();
    assert_eq!(imported.dtype(), &DataType::Variant);
}

#[test]
fn ascii_ffi_schemas_carry_the_extension_identity() {
    let currency =
        Field::from_parts("ccy", DataType::FixedAscii(4), false, [("owner", "core")]).unwrap();

    let schema = currency.clone().into_arrow_ffi().unwrap();
    assert_eq!(schema.format(), "w:4");
    let metadata = schema.metadata().unwrap();
    assert_eq!(
        metadata.get("ARROW:extension:name"),
        Some(&"yggdryl.ascii".to_owned())
    );
    assert_eq!(
        metadata.get("ARROW:extension:metadata"),
        Some(&String::new())
    );
    assert_eq!(metadata.get("owner"), Some(&"core".to_owned()));

    // A round trip through the C schema restores the exact field.
    let imported = Field::from_arrow(&ArrowField::try_from(&schema).unwrap()).unwrap();
    assert_eq!(imported, currency);

    // A bare datatype carries the identity too.
    let schema = DataType::FixedAscii(16).into_arrow_ffi().unwrap();
    assert_eq!(schema.format(), "w:16");
    assert_eq!(
        schema.metadata().unwrap().get("ARROW:extension:name"),
        Some(&"yggdryl.ascii".to_owned())
    );
    let imported = Field::from_arrow(&ArrowField::try_from(&schema).unwrap()).unwrap();
    assert_eq!(imported.dtype(), &DataType::FixedAscii(16));
}

#[test]
fn arrow_exchange_sidecar_restores_nested_dictionary_ids_after_a_c_round_trip() {
    let mut region = Field::new(
        "region",
        DataType::dictionary(DataType::Int16, DataType::Utf8).unwrap(),
        true,
    );
    region.set_dictionary_options(-7, true).unwrap();

    let mut catalog = Field::new(
        "catalog",
        DataType::dictionary(DataType::UInt8, DataType::from_fields([region]).unwrap()).unwrap(),
        false,
    );
    catalog.set_dictionary_options(42, false).unwrap();

    let mut item = Field::new(
        "item",
        DataType::dictionary(DataType::Int32, DataType::LargeUtf8).unwrap(),
        true,
    );
    item.set_dictionary_options(i64::MIN, true).unwrap();

    let root = Field::from_parts(
        "row",
        DataType::from_fields([catalog, Field::new("labels", DataType::list(item), true)]).unwrap(),
        false,
        [("owner", "core")],
    )
    .unwrap();

    let projected = root.clone().into_arrow_exchange_schema().unwrap();
    assert_eq!(
        projected
            .metadata()
            .get(IPC_DICTIONARY_IDS_KEY)
            .map(String::as_str),
        Some("v1;0=42;0.0=-7;1.0=-9223372036854775808")
    );

    // This is the same interface PyArrow crosses.  Its schema has no
    // dictionary-ID slot, but schema metadata and dictionary ordering survive.
    let ffi = FFI_ArrowSchema::try_from(&projected).unwrap();
    let crossed = Schema::try_from(&ffi).unwrap();
    #[allow(deprecated)]
    {
        assert_eq!(crossed.field(0).dict_id(), Some(0));
    }
    assert_eq!(
        crossed
            .metadata()
            .get(IPC_DICTIONARY_IDS_KEY)
            .map(String::as_str),
        Some("v1;0=42;0.0=-7;1.0=-9223372036854775808")
    );

    let restored = Field::from_arrow_schema("row", &crossed).unwrap();
    assert_eq!(restored, root);
    assert_eq!(restored.get_metadata("owner"), Some("core"));
    assert!(!restored.has_metadata(IPC_DICTIONARY_IDS_KEY));
}

fn assert_dictionary_sidecar_error(schema: &Schema) {
    match Field::from_arrow_schema("row", schema).unwrap_err() {
        yggdryl::arrow::Error::Core(yggdryl::Error::InvalidMetadataValue { key, .. }) => {
            assert_eq!(key, IPC_DICTIONARY_IDS_KEY)
        }
        error => panic!("expected a typed dictionary-sidecar error, got {error}"),
    }
}

#[test]
#[allow(deprecated)]
fn arrow_exchange_sidecar_rejects_malformed_missing_and_conflicting_entries() {
    let dictionary = ArrowField::new_dict(
        "code",
        ArrowDataType::Dictionary(
            Box::new(ArrowDataType::Int16),
            Box::new(ArrowDataType::Utf8),
        ),
        false,
        0,
        false,
    );
    for encoded in [
        "",
        "v2;0=1",
        "v1;",
        "v1;0=x",
        "v1;00=1",
        "v1;0=0",
        "v1;0=1;0=2",
        "v1;1=1;0=2",
    ] {
        let schema = Schema::new_with_metadata(
            vec![dictionary.clone()],
            HashMap::from([(IPC_DICTIONARY_IDS_KEY.to_owned(), encoded.to_owned())]),
        );
        assert_dictionary_sidecar_error(&schema);
    }

    let missing = Schema::new_with_metadata(
        vec![dictionary.clone()],
        HashMap::from([(IPC_DICTIONARY_IDS_KEY.to_owned(), "v1;1=9".to_owned())]),
    );
    assert_dictionary_sidecar_error(&missing);

    let not_dictionary = Schema::new_with_metadata(
        vec![ArrowField::new("code", ArrowDataType::Int16, false)],
        HashMap::from([(IPC_DICTIONARY_IDS_KEY.to_owned(), "v1;0=9".to_owned())]),
    );
    assert_dictionary_sidecar_error(&not_dictionary);

    let conflicting = Schema::new_with_metadata(
        vec![ArrowField::new_dict(
            "code",
            ArrowDataType::Dictionary(
                Box::new(ArrowDataType::Int16),
                Box::new(ArrowDataType::Utf8),
            ),
            false,
            7,
            false,
        )],
        HashMap::from([(IPC_DICTIONARY_IDS_KEY.to_owned(), "v1;0=8".to_owned())]),
    );
    assert_dictionary_sidecar_error(&conflicting);
}

#[test]
fn arrow_exchange_projection_refuses_caller_owned_sidecar_metadata() {
    let root = Field::from_parts(
        "row",
        DataType::from_fields([DataType::Int64.required_field("id")]).unwrap(),
        false,
        [(IPC_DICTIONARY_IDS_KEY, "v1;0=7")],
    )
    .unwrap();

    match root.into_arrow_exchange_schema().unwrap_err() {
        yggdryl::arrow::Error::Core(yggdryl::Error::InvalidMetadataValue { key, .. }) => {
            assert_eq!(key, IPC_DICTIONARY_IDS_KEY)
        }
        error => panic!("expected a typed dictionary-sidecar error, got {error}"),
    }
}
