use std::collections::HashMap;
use std::sync::Arc;

use arrow_schema::extension::{EXTENSION_TYPE_METADATA_KEY, EXTENSION_TYPE_NAME_KEY};
use arrow_schema::{
    DataType as ArrowDataType, Field as ArrowField, Schema,
    ffi::{FFI_ArrowSchema, Flags},
};
use yggdryl::arrow::IPC_DICTIONARY_IDS_KEY;
use yggdryl::types::BytesType;
use yggdryl::{ArrowCastOptions, DataType, EdgeAlgorithm, Field, Nullability, TimeUnit, Timezone};

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

    let borrowed = Field::from_arrow_field(&arrow).unwrap();
    assert_canonical_http_metadata(&borrowed.into_arrow_field().unwrap());

    let shared = Arc::new(arrow.clone());
    let shared_import = Field::from_arrow_field_ref(Arc::clone(&shared)).unwrap();
    let shared_projection = shared_import.into_arrow_field_ref().unwrap();
    assert!(!Arc::ptr_eq(&shared, &shared_projection));
    assert_canonical_http_metadata(shared_projection.as_ref());

    let owned = Field::try_from(arrow).unwrap();
    assert_canonical_http_metadata(&owned.into_arrow_field().unwrap());
}

#[test]
fn core_ffi_projection_preserves_every_field_and_datatype_flag_recursively() {
    let mut encoded = Field::from_parts(
        "codes",
        DataType::dictionary(DataType::UInt16, DataType::utf8()).unwrap(),
        true,
        [("ARROW:extension:name", "catalog-code")],
    )
    .unwrap();
    encoded.set_dictionary_options(17, true).unwrap();

    let entries = Field::new(
        "entries",
        DataType::from_fields([Field::new("key", DataType::utf8(), false), encoded]).unwrap(),
        false,
    );
    let map = DataType::map(entries, true).unwrap();
    let root = Field::from_parts("lookup", map, true, [("owner", "core")]).unwrap();

    let schema = root.clone().into_arrow_field_ffi().unwrap();
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

    root.clone().into_arrow_field_ref().unwrap();
    let cached = root.into_arrow_field_ffi().unwrap();
    assert_flag(&cached, Flags::NULLABLE);
    assert_flag(&cached, Flags::MAP_KEYS_SORTED);
    assert_flag(cached.child(0).child(1), Flags::DICTIONARY_ORDERED);
}

#[test]
fn datatype_ffi_projection_preserves_nested_map_flags_and_rejects_invalid_state() {
    let map = DataType::map_of(DataType::utf8(), DataType::Int64, true).unwrap();
    let dtype = DataType::from_fields([Field::new("lookup", map, true)]).unwrap();

    let schema = dtype.into_arrow_datatype_ffi().unwrap();
    let map = schema.child(0);
    assert_flag(map, Flags::NULLABLE);
    assert_flag(map, Flags::MAP_KEYS_SORTED);

    // A fixed layout built by hand with no width is what `validate` catches.
    assert!(
        DataType::Bytes(BytesType::FixedBinary(0))
            .into_arrow_datatype_ffi()
            .is_err()
    );
    assert!(
        Field::new(
            "bad",
            DataType::DateTime64 {
                unit: TimeUnit::YearMonth,
                timezone: Timezone::NAIVE
            },
            false,
        )
        .into_arrow_field_ffi()
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

    let schema = geography.clone().into_arrow_field_ffi().unwrap();
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
    let imported = Field::from_arrow_field(&ArrowField::try_from(&schema).unwrap()).unwrap();
    assert_eq!(imported, geography);

    // The cached projection path serves the same identity.
    geography.clone().into_arrow_field_ref().unwrap();
    let cached = geography.into_arrow_field_ffi().unwrap();
    assert_eq!(
        cached.metadata().unwrap().get("ARROW:extension:name"),
        Some(&"geoarrow.wkb".to_owned())
    );

    // A bare datatype carries the identity too, variant included.
    let schema = DataType::variant().into_arrow_datatype_ffi().unwrap();
    let metadata = schema.metadata().unwrap();
    assert_eq!(
        metadata.get("ARROW:extension:name"),
        Some(&"arrow.parquet.variant".to_owned())
    );
    assert_eq!(
        metadata.get("ARROW:extension:metadata"),
        Some(&String::new())
    );
    let imported = Field::from_arrow_field(&ArrowField::try_from(&schema).unwrap()).unwrap();
    assert_eq!(imported.dtype(), &DataType::Variant);
}

#[test]
fn fixed_ascii_ffi_schemas_carry_the_string_document() {
    let currency = Field::from_parts(
        "ccy",
        DataType::fixed_ascii(4).unwrap(),
        false,
        [("owner", "core")],
    )
    .unwrap();

    let schema = currency.clone().into_arrow_field_ffi().unwrap();
    assert_eq!(schema.format(), "w:4");
    let metadata = schema.metadata().unwrap();
    assert_eq!(
        metadata.get("ARROW:extension:name"),
        Some(&"yggdryl.string".to_owned())
    );
    assert_eq!(
        metadata.get("ARROW:extension:metadata"),
        Some(&r#"{"layout":"fixed_string","charset":"us-ascii","fixed":4}"#.to_owned())
    );
    assert_eq!(metadata.get("owner"), Some(&"core".to_owned()));

    // A round trip through the C schema restores the exact field.
    let imported = Field::from_arrow_field(&ArrowField::try_from(&schema).unwrap()).unwrap();
    assert_eq!(imported, currency);

    // A bare datatype carries the identity too.
    let schema = DataType::fixed_ascii(16)
        .unwrap()
        .into_arrow_datatype_ffi()
        .unwrap();
    assert_eq!(schema.format(), "w:16");
    assert_eq!(
        schema.metadata().unwrap().get("ARROW:extension:name"),
        Some(&"yggdryl.string".to_owned())
    );
    let imported = Field::from_arrow_field(&ArrowField::try_from(&schema).unwrap()).unwrap();
    assert_eq!(imported.dtype(), &DataType::fixed_ascii(16).unwrap());
}

#[test]
fn arrow_exchange_sidecar_restores_nested_dictionary_ids_after_a_c_round_trip() {
    let mut region = Field::new(
        "region",
        DataType::dictionary(DataType::Int16, DataType::utf8()).unwrap(),
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
        DataType::dictionary(DataType::Int32, DataType::large_utf8()).unwrap(),
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

/// A root whose `year` derives from `event` and whose `row_digest` holds the
/// row hash, with a nested Struct declaring one of each of its own.
fn applied_root() -> Field {
    let mut inner_year = DataType::Int32.nullable_field("year");
    inner_year
        .as_partition_mut()
        .set_sources(["event"])
        .unwrap();
    inner_year
        .as_partition_mut()
        .set_transform(yggdryl::expression::Function::Year)
        .unwrap();
    let mut inner_digest = DataType::UInt64.nullable_field("trade_digest");
    inner_digest.as_digest_mut().set_holder().unwrap();
    let trade = DataType::from_fields([
        DataType::Date32.required_field("event"),
        inner_year,
        inner_digest,
    ])
    .unwrap()
    .required_field("trade");

    let mut top_year = DataType::Int32.nullable_field("top_year");
    top_year
        .as_partition_mut()
        .set_sources(["trade.year"])
        .unwrap();
    let mut row_digest = DataType::UInt64.nullable_field("row_digest");
    row_digest.as_digest_mut().set_holder().unwrap();
    DataType::from_fields([trade, top_year, row_digest])
        .unwrap()
        .required_field("row")
}

fn events() -> arrow_array::RecordBatch {
    arrow_array::RecordBatch::try_from_iter([(
        "trade",
        Arc::new(arrow_array::StructArray::from(vec![(
            Arc::new(ArrowField::new("event", ArrowDataType::Date32, false)),
            Arc::new(arrow_array::Date32Array::from(vec![19_723, 20_089])) as arrow_array::ArrayRef,
        )])) as arrow_array::ArrayRef,
    )])
    .unwrap()
}

#[test]
fn apply_arrow_batch_runs_every_protocol_and_walks_nested_declarations() {
    let root = applied_root();

    let applied = root
        .apply_arrow_batch(&events(), true, true, true, ArrowCastOptions::new())
        .unwrap();

    assert_eq!(applied.num_columns(), 3);
    let trade = applied
        .column_by_name("trade")
        .unwrap()
        .as_any()
        .downcast_ref::<arrow_array::StructArray>()
        .expect("the nested struct both protocols widened");
    assert_eq!(trade.num_columns(), 3);
    assert_eq!(
        trade
            .column_by_name("year")
            .unwrap()
            .as_any()
            .downcast_ref::<arrow_array::Int32Array>()
            .unwrap()
            .values(),
        &[2024, 2025]
    );
    // Every holder was filled, nested one included.
    assert_eq!(
        trade.column_by_name("trade_digest").unwrap().null_count(),
        0
    );
    assert_eq!(
        applied.column_by_name("row_digest").unwrap().null_count(),
        0
    );
    // The level above read what the nested partition declaration wrote.
    assert_eq!(
        applied
            .column_by_name("top_year")
            .unwrap()
            .as_any()
            .downcast_ref::<arrow_array::Int32Array>()
            .unwrap()
            .values(),
        &[2024, 2025]
    );
}

#[test]
fn apply_arrow_batch_answers_the_same_batch_the_second_time() {
    let root = applied_root();
    let once = root
        .apply_arrow_batch(&events(), true, true, true, ArrowCastOptions::new())
        .unwrap();

    assert_eq!(
        root.apply_arrow_batch(&once, true, true, true, ArrowCastOptions::new())
            .unwrap(),
        once
    );
}

#[test]
fn apply_arrow_batch_runs_only_the_protocols_it_is_asked_for() {
    let root = applied_root();

    // Cast alone materializes every declared column and writes none of them.
    let cast_only = root
        .apply_arrow_batch(&events(), false, false, true, ArrowCastOptions::new())
        .unwrap();
    assert_eq!(cast_only.num_columns(), 3);
    assert_eq!(
        cast_only.column_by_name("top_year").unwrap().null_count(),
        2
    );
    assert_eq!(
        cast_only.column_by_name("row_digest").unwrap().null_count(),
        2
    );

    // Partition alone leaves the holders untouched.
    let partitioned = root
        .apply_arrow_batch(&events(), false, true, true, ArrowCastOptions::new())
        .unwrap();
    assert_eq!(
        partitioned
            .column_by_name("top_year")
            .unwrap()
            .as_any()
            .downcast_ref::<arrow_array::Int32Array>()
            .unwrap()
            .values(),
        &[2024, 2025]
    );
    assert_eq!(
        partitioned
            .column_by_name("row_digest")
            .unwrap()
            .null_count(),
        2
    );

    // A digest over the uncast batch still reconciles for itself, because a
    // holder is addressed by position.
    let digested = root
        .apply_arrow_batch(&events(), true, false, false, ArrowCastOptions::new())
        .unwrap();
    assert_eq!(
        digested.column_by_name("row_digest").unwrap().null_count(),
        0
    );
    assert_eq!(digested.column_by_name("top_year").unwrap().null_count(), 2);
}

#[test]
fn apply_arrow_batch_refuses_a_root_that_is_not_a_struct() {
    let root = DataType::Int64.required_field("id");

    assert!(
        root.apply_arrow_batch(&events(), false, true, false, ArrowCastOptions::new())
            .is_err()
    );
}

#[test]
fn apply_arrow_schema_answers_the_shape_without_reading_a_row() {
    let root = applied_root();
    let stored = events().schema();

    let applied = root
        .apply_arrow_schema(
            Arc::clone(&stored),
            true,
            true,
            true,
            ArrowCastOptions::new(),
        )
        .unwrap();

    assert_eq!(
        applied
            .fields()
            .iter()
            .map(|field| field.name().as_str())
            .collect::<Vec<_>>(),
        ["trade", "top_year", "row_digest"]
    );
    // The nested Struct is widened in the reported schema too.
    let ArrowDataType::Struct(children) = applied.field(0).data_type() else {
        panic!("the nested declaration stays a Struct");
    };
    assert_eq!(children.len(), 3);
    // It is exactly the schema a batch comes back with.
    assert_eq!(
        applied,
        root.apply_arrow_batch(&events(), true, true, true, ArrowCastOptions::new())
            .unwrap()
            .schema()
    );
}

#[test]
fn apply_arrow_schema_refuses_a_declaration_the_reader_cannot_satisfy() {
    let root = applied_root();
    let unrelated = Arc::new(Schema::new(vec![ArrowField::new(
        "price",
        ArrowDataType::Int64,
        false,
    )]));

    // Nothing is decoded, and the missing source is named before any row is.
    let error = root
        .apply_arrow_schema(unrelated, false, true, false, ArrowCastOptions::new())
        .unwrap_err()
        .to_string();
    assert!(error.contains("trade"), "{error}");
}

#[test]
fn apply_arrow_reader_reports_the_applied_schema_before_the_first_batch() {
    let root = applied_root();
    let stored = events().schema();
    let reader = yggdryl::arrow::batch_reader(Arc::clone(&stored), [events(), events()]);

    let mut applied = root
        .apply_arrow_reader(reader, true, true, true, ArrowCastOptions::new())
        .unwrap();

    // Read before pulling: the shape is a property of the two schemas.
    let reported = arrow_array::RecordBatchReader::schema(&applied);
    assert_eq!(reported.fields().len(), 3);

    let first = applied.next().expect("one batch").unwrap();
    assert_eq!(first.schema(), reported);
    assert_eq!(first.column_by_name("row_digest").unwrap().null_count(), 0);
    assert_eq!(
        applied
            .next()
            .expect("a second batch")
            .unwrap()
            .num_columns(),
        3
    );
    assert!(applied.next().is_none());
}

#[test]
fn apply_arrow_reader_asked_for_nothing_hands_the_reader_back() {
    let root = applied_root();
    let stored = events().schema();
    let reader = yggdryl::arrow::batch_reader(Arc::clone(&stored), [events()]);

    let mut untouched = root
        .apply_arrow_reader(reader, false, false, false, ArrowCastOptions::new())
        .unwrap();

    assert_eq!(arrow_array::RecordBatchReader::schema(&untouched), stored);
    assert_eq!(
        untouched.next().expect("one batch").unwrap().num_columns(),
        1
    );
}

/// A root whose derived and held columns are declared non-null.
///
/// This is the shape strictness has to reason about: `year` and `row_digest`
/// are absent from the source and required in the schema, and the only reason
/// that is not a contradiction is that the two protocols write them.
fn required_applied_root() -> Field {
    let mut year = DataType::Int32.required_field("year");
    year.as_partition_mut().set_sources(["event"]).unwrap();
    year.as_partition_mut()
        .set_transform(yggdryl::expression::Function::Year)
        .unwrap();
    let mut row_digest = DataType::UInt64.required_field("row_digest");
    row_digest.as_digest_mut().set_holder().unwrap();
    DataType::from_fields([DataType::Date32.required_field("event"), year, row_digest])
        .unwrap()
        .required_field("row")
}

fn dates() -> arrow_array::RecordBatch {
    arrow_array::RecordBatch::try_from_iter([(
        "event",
        Arc::new(arrow_array::Date32Array::from(vec![19_723, 20_089])) as arrow_array::ArrayRef,
    )])
    .unwrap()
}

#[test]
fn a_strict_apply_lets_an_enabled_protocol_fill_its_own_required_column() {
    let root = required_applied_root();

    let applied = root
        .apply_arrow_batch(
            &dates(),
            true,
            true,
            true,
            ArrowCastOptions::new().with_nullability(Nullability::Strict),
        )
        .unwrap();

    // Both columns were absent from the source and are declared non-null; the
    // cast let them through because their protocols were about to write them,
    // and the re-check over the finished batch found them written.
    assert_eq!(applied.num_columns(), 3);
    assert_eq!(applied.column(1).null_count(), 0);
    assert_eq!(applied.column(2).null_count(), 0);
}

#[test]
fn a_strict_apply_refuses_the_column_whose_protocol_is_switched_off() {
    let root = required_applied_root();

    // The partition step is what would have written `year`, so with it off the
    // finished batch leaves a declared non-null column holding its default.
    let message = root
        .apply_arrow_batch(
            &dates(),
            true,
            false,
            true,
            ArrowCastOptions::new().with_nullability(Nullability::Strict),
        )
        .unwrap_err()
        .to_string();
    assert!(message.contains("$.year"), "{message}");

    // The default policy still fills it, unchanged.
    let filled = root
        .apply_arrow_batch(&dates(), true, false, true, ArrowCastOptions::new())
        .unwrap();
    assert_eq!(filled.num_columns(), 3);
}

#[test]
fn a_strict_apply_refuses_an_ordinary_required_column_the_source_lacks() {
    let root = Field::new(
        "row",
        DataType::from_fields([
            DataType::Date32.required_field("event"),
            DataType::utf8().required_field("venue"),
        ])
        .unwrap(),
        false,
    );

    // No protocol declares `venue`, so nothing is going to write it: the cast
    // refuses it where it stands rather than defaulting it and re-checking.
    let message = root
        .apply_arrow_batch(
            &dates(),
            true,
            true,
            true,
            ArrowCastOptions::new().with_nullability(Nullability::Strict),
        )
        .unwrap_err()
        .to_string();
    // The applied verbs answer a core error, so the runtime refusal travels
    // inside one rather than replacing it.
    assert!(
        message.contains("required Arrow field $.venue is missing from the source"),
        "{message}"
    );
}

#[test]
fn a_strict_applied_schema_is_still_derived_without_reading_a_row() {
    let root = required_applied_root();
    let stored = dates().schema();

    let applied = root
        .apply_arrow_schema(
            Arc::clone(&stored),
            true,
            true,
            true,
            ArrowCastOptions::new().with_nullability(Nullability::Strict),
        )
        .unwrap();

    assert_eq!(applied.fields().len(), 3);
    assert_eq!(applied.field(1).name(), "year");
    assert_eq!(applied.field(2).name(), "row_digest");
}

#[test]
fn a_strict_applied_reader_answers_its_schema_and_applies_every_batch() {
    let root = required_applied_root();
    let stored = dates().schema();
    let reader = yggdryl::arrow::batch_reader(Arc::clone(&stored), [dates(), dates()]);

    let mut applied = root
        .apply_arrow_reader(
            reader,
            true,
            true,
            true,
            ArrowCastOptions::new().with_nullability(Nullability::Strict),
        )
        .unwrap();

    assert_eq!(
        arrow_array::RecordBatchReader::schema(&applied)
            .fields()
            .len(),
        3
    );
    for _ in 0..2 {
        let batch = applied.next().expect("a batch").unwrap();
        assert_eq!(batch.num_columns(), 3);
        assert_eq!(batch.column(2).null_count(), 0);
    }
    assert!(applied.next().is_none());
}

fn variant_storage() -> ArrowDataType {
    ArrowDataType::Struct(arrow_schema::Fields::from(vec![
        ArrowField::new("metadata", ArrowDataType::Binary, false),
        ArrowField::new("value", ArrowDataType::Binary, false),
    ]))
}

#[test]
fn a_geometry_field_projects_the_geoarrow_extension_and_reimports_itself() {
    let field = Field::new("shape", DataType::geometry(None).unwrap(), true);
    let arrow = field.clone().into_arrow_field().unwrap();
    assert_eq!(arrow.data_type(), &ArrowDataType::Binary);
    assert_eq!(arrow.extension_type_name(), Some("geoarrow.wkb"));
    assert_eq!(
        arrow.extension_type_metadata(),
        Some(r#"{"crs":"OGC:CRS84"}"#)
    );

    let imported = Field::from_arrow_field(&arrow).unwrap();
    assert_eq!(imported, field);
    // The extension keys are transport: they never reach Field metadata.
    assert!(imported.as_metadata().is_empty());
}

#[test]
fn a_geography_projection_carries_the_edge_algorithm_and_round_trips() {
    let field = Field::new(
        "region",
        DataType::geography(Some("EPSG:4326"), Some(EdgeAlgorithm::Vincenty)).unwrap(),
        false,
    );
    let arrow = field.clone().into_arrow_field().unwrap();
    assert_eq!(
        arrow.extension_type_metadata(),
        Some(r#"{"crs":"EPSG:4326","edges":"vincenty"}"#)
    );
    assert_eq!(Field::from_arrow_field(&arrow).unwrap(), field);
}

#[test]
fn a_variant_field_projects_the_canonical_struct_and_reimports_itself() {
    let field = Field::new("payload", DataType::variant(), true);
    let arrow = field.clone().into_arrow_field().unwrap();
    assert_eq!(arrow.data_type(), &variant_storage());
    assert_eq!(arrow.extension_type_name(), Some("arrow.parquet.variant"));
    assert_eq!(arrow.extension_type_metadata(), Some(""));

    let imported = Field::from_arrow_field(&arrow).unwrap();
    assert_eq!(imported.dtype(), &DataType::Variant);
    assert!(imported.as_metadata().is_empty());
    assert_eq!(imported, field);
}

#[test]
fn a_bare_geoarrow_document_imports_as_the_default_geometry() {
    let arrow =
        ArrowField::new("shape", ArrowDataType::Binary, true).with_metadata(HashMap::from([(
            EXTENSION_TYPE_NAME_KEY.to_owned(),
            "geoarrow.wkb".to_owned(),
        )]));
    let imported = Field::from_arrow_field(&arrow).unwrap();
    assert_eq!(imported.dtype(), &DataType::geometry(None).unwrap());
    let shared = Field::from_arrow_field_ref(Arc::new(arrow.clone())).unwrap();
    assert_eq!(shared, imported);
    let owned = Field::try_from(arrow).unwrap();
    assert_eq!(owned, imported);
}

#[test]
fn an_unknown_extension_name_keeps_todays_import_exactly() {
    let arrow = ArrowField::new("raw", ArrowDataType::Binary, true).with_metadata(HashMap::from([
        (
            EXTENSION_TYPE_NAME_KEY.to_owned(),
            "someorg.blob".to_owned(),
        ),
        (EXTENSION_TYPE_METADATA_KEY.to_owned(), "{}".to_owned()),
    ]));
    let imported = Field::from_arrow_field(&arrow).unwrap();
    assert_eq!(imported.dtype(), &DataType::binary());
    assert_eq!(
        imported.get_metadata(EXTENSION_TYPE_NAME_KEY),
        Some("someorg.blob")
    );
    assert_eq!(imported.into_arrow_field().unwrap(), arrow);
}

#[test]
fn our_extension_name_over_a_foreign_storage_keeps_todays_import() {
    let arrow =
        ArrowField::new("shape", ArrowDataType::LargeBinary, true).with_metadata(HashMap::from([
            (
                EXTENSION_TYPE_NAME_KEY.to_owned(),
                "geoarrow.wkb".to_owned(),
            ),
        ]));
    let imported = Field::from_arrow_field(&arrow).unwrap();
    assert_eq!(imported.dtype(), &DataType::large_binary());
    assert_eq!(
        imported.get_metadata(EXTENSION_TYPE_NAME_KEY),
        Some("geoarrow.wkb")
    );
}

#[test]
fn a_malformed_geoarrow_document_is_refused_naming_the_key() {
    let arrow =
        ArrowField::new("shape", ArrowDataType::Binary, true).with_metadata(HashMap::from([
            (
                EXTENSION_TYPE_NAME_KEY.to_owned(),
                "geoarrow.wkb".to_owned(),
            ),
            (
                EXTENSION_TYPE_METADATA_KEY.to_owned(),
                r#"{"crs":7}"#.to_owned(),
            ),
        ]));
    let refused = Field::from_arrow_field(&arrow).unwrap_err().to_string();
    assert!(refused.contains(EXTENSION_TYPE_METADATA_KEY), "{refused}");
    assert!(refused.contains("crs"), "{refused}");
}

#[test]
fn a_caller_set_extension_key_on_an_extension_typed_field_is_refused_naming_both() {
    let field = Field::from_parts(
        "shape",
        DataType::geometry(None).unwrap(),
        false,
        [(EXTENSION_TYPE_NAME_KEY, "someorg.other")],
    )
    .unwrap();
    let refused = field.into_arrow_field().unwrap_err().to_string();
    assert!(refused.contains("someorg.other"), "{refused}");
    assert!(refused.contains("geoarrow.wkb"), "{refused}");

    let variant = Field::from_parts(
        "payload",
        DataType::variant(),
        true,
        [(EXTENSION_TYPE_METADATA_KEY, "shredded")],
    )
    .unwrap();
    let refused = variant.into_arrow_field().unwrap_err().to_string();
    assert!(refused.contains("shredded"), "{refused}");
    assert!(refused.contains("arrow.parquet.variant"), "{refused}");
}

#[test]
fn a_variant_with_a_nonempty_document_or_foreign_shape_keeps_todays_import() {
    let shredded =
        ArrowField::new("payload", variant_storage(), true).with_metadata(HashMap::from([
            (
                EXTENSION_TYPE_NAME_KEY.to_owned(),
                "arrow.parquet.variant".to_owned(),
            ),
            (
                EXTENSION_TYPE_METADATA_KEY.to_owned(),
                "shredded".to_owned(),
            ),
        ]));
    let imported = Field::from_arrow_field(&shredded).unwrap();
    assert!(matches!(imported.dtype(), DataType::Structure(_)));

    let swapped = ArrowDataType::Struct(arrow_schema::Fields::from(vec![
        ArrowField::new("value", ArrowDataType::Binary, false),
        ArrowField::new("metadata", ArrowDataType::Binary, false),
    ]));
    let swapped = ArrowField::new("payload", swapped, true).with_metadata(HashMap::from([(
        EXTENSION_TYPE_NAME_KEY.to_owned(),
        "arrow.parquet.variant".to_owned(),
    )]));
    let imported = Field::from_arrow_field(&swapped).unwrap();
    assert!(matches!(imported.dtype(), DataType::Structure(_)));
}

#[test]
fn an_ascii_field_projects_the_string_extension_and_reimports_itself() {
    // US-ASCII is UTF-8, so it rides Arrow's own text storage and the
    // charset rides the document; a width is that width's fixed binary
    // and the document names it, so no width is special.
    for (dtype, storage, document) in [
        (
            DataType::ascii(),
            ArrowDataType::Utf8,
            r#"{"layout":"string","charset":"us-ascii"}"#,
        ),
        (
            DataType::fixed_ascii(1).unwrap(),
            ArrowDataType::FixedSizeBinary(1),
            r#"{"layout":"fixed_string","charset":"us-ascii","fixed":1}"#,
        ),
        (
            DataType::fixed_ascii(3).unwrap(),
            ArrowDataType::FixedSizeBinary(3),
            r#"{"layout":"fixed_string","charset":"us-ascii","fixed":3}"#,
        ),
        (
            DataType::fixed_ascii(64).unwrap(),
            ArrowDataType::FixedSizeBinary(64),
            r#"{"layout":"fixed_string","charset":"us-ascii","fixed":64}"#,
        ),
    ] {
        let field = Field::new("ccy", dtype, false);
        let arrow = field.clone().into_arrow_field().unwrap();
        assert_eq!(arrow.data_type(), &storage);
        assert_eq!(arrow.extension_type_name(), Some("yggdryl.string"));
        assert_eq!(arrow.extension_type_metadata(), Some(document));

        let imported = Field::from_arrow_field(&arrow).unwrap();
        assert_eq!(imported, field);
        assert!(imported.as_metadata().is_empty());
    }
}

#[test]
fn a_string_extension_over_other_storage_or_a_retired_name_keeps_todays_import() {
    // The document lays out one storage. Any other storage is a foreign
    // field wearing our name, so the name stays metadata.
    let document = r#"{"layout":"string","charset":"us-ascii"}"#;
    let large =
        ArrowField::new("ccy", ArrowDataType::LargeBinary, true).with_metadata(HashMap::from([
            (
                EXTENSION_TYPE_NAME_KEY.to_owned(),
                "yggdryl.string".to_owned(),
            ),
            (EXTENSION_TYPE_METADATA_KEY.to_owned(), document.to_owned()),
        ]));
    let imported = Field::from_arrow_field(&large).unwrap();
    assert_eq!(imported.dtype(), &DataType::large_binary());
    assert_eq!(
        imported.get_metadata(EXTENSION_TYPE_NAME_KEY),
        Some("yggdryl.string")
    );
    assert_eq!(imported.into_arrow_field().unwrap(), large);

    // `yggdryl.ascii` names nothing this crate has.
    let retired = ArrowField::new("ccy", ArrowDataType::FixedSizeBinary(4), true).with_metadata(
        HashMap::from([(
            EXTENSION_TYPE_NAME_KEY.to_owned(),
            "yggdryl.ascii".to_owned(),
        )]),
    );
    let imported = Field::from_arrow_field(&retired).unwrap();
    assert_eq!(imported.dtype(), &DataType::fixed_binary(4).unwrap());
    assert_eq!(imported.into_arrow_field().unwrap(), retired);
}

#[test]
fn a_bounded_bytes_field_projects_the_bytes_extension_and_reimports_itself() {
    // The four layouts are Arrow's own and a fixed width is the storage,
    // so only a maximum rides the document.
    let bounded = BytesType::SizedBinary(16);
    let field = Field::new("key", DataType::bytes(bounded).unwrap(), true);
    let arrow = field.clone().into_arrow_field().unwrap();
    assert_eq!(arrow.data_type(), &ArrowDataType::Binary);
    assert_eq!(arrow.extension_type_name(), Some("yggdryl.bytes"));
    assert_eq!(
        arrow.extension_type_metadata(),
        Some(r#"{"layout":"sized_binary","max":16}"#)
    );
    let imported = Field::from_arrow_field(&arrow).unwrap();
    assert_eq!(imported, field);
    assert!(imported.as_metadata().is_empty());

    for dtype in [
        DataType::binary(),
        DataType::large_binary(),
        DataType::binary_view(),
        DataType::fixed_binary(16).unwrap(),
    ] {
        let arrow = Field::new("key", dtype, true).into_arrow_field().unwrap();
        assert_eq!(arrow.extension_type_name(), None, "{arrow:?}");
    }
}

#[test]
fn a_bytes_extension_over_other_storage_keeps_todays_import() {
    let document = r#"{"layout":"sized_binary","max":16}"#;
    let large =
        ArrowField::new("key", ArrowDataType::LargeBinary, true).with_metadata(HashMap::from([
            (
                EXTENSION_TYPE_NAME_KEY.to_owned(),
                "yggdryl.bytes".to_owned(),
            ),
            (EXTENSION_TYPE_METADATA_KEY.to_owned(), document.to_owned()),
        ]));
    let imported = Field::from_arrow_field(&large).unwrap();
    assert_eq!(imported.dtype(), &DataType::large_binary());
    assert_eq!(
        imported.get_metadata(EXTENSION_TYPE_NAME_KEY),
        Some("yggdryl.bytes")
    );
    assert_eq!(imported.into_arrow_field().unwrap(), large);
}

#[test]
fn a_negative_arrow_fixed_binary_width_is_refused() {
    let refused = DataType::from_arrow_datatype(&ArrowDataType::FixedSizeBinary(-1))
        .unwrap_err()
        .to_string();
    assert!(refused.contains("-1"), "{refused}");
    assert!(DataType::from_arrow_datatype(&ArrowDataType::FixedSizeBinary(0)).is_err());
}
