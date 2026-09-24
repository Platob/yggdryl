//! `rust/src/field.rs`: focused regression tests for the datatype
//! implementation modules.

use super::typed;

mod families {

    use arrow_schema::{DataType as ArrowDataType, Field as ArrowField};
    use std::sync::Arc;
    use yggdryl::Field;
    use yggdryl::{DataType, StructType, TimeUnit, UnionFields};

    #[test]
    fn arrow_import_preserves_nested_field_projection_arcs() {
        let inner = Arc::new(ArrowField::new("item", ArrowDataType::Utf8, true));
        let outer = Arc::new(ArrowField::new(
            "item",
            ArrowDataType::List(Arc::clone(&inner)),
            true,
        ));
        let arrow = ArrowDataType::List(Arc::clone(&outer));

        let borrowed = DataType::from_arrow_datatype(&arrow).unwrap();
        let borrowed_outer = borrowed.get_field(0).unwrap();
        assert!(Arc::ptr_eq(
            &borrowed_outer.clone().into_arrow_field_ref().unwrap(),
            &outer
        ));
        let borrowed_inner = borrowed_outer.dtype().get_field(0).unwrap();
        assert!(Arc::ptr_eq(
            &borrowed_inner.clone().into_arrow_field_ref().unwrap(),
            &inner
        ));

        let owned = DataType::try_from(arrow).unwrap();
        let owned_outer = owned.get_field(0).unwrap();
        assert!(Arc::ptr_eq(
            &owned_outer.clone().into_arrow_field_ref().unwrap(),
            &outer
        ));
        let owned_inner = owned_outer.dtype().get_field(0).unwrap();
        assert!(Arc::ptr_eq(
            &owned_inner.clone().into_arrow_field_ref().unwrap(),
            &inner
        ));
    }

    #[test]
    fn public_field_collections_validate_children_without_clone_helpers() {
        let invalid = Field::new("invalid", DataType::Time32(TimeUnit::Nanosecond), false);
        assert!(StructType::from_fields([invalid.clone()]).is_err());
        assert!(UnionFields::from_fields([(0, invalid)]).is_err());
    }
}

mod arrow {
    use std::collections::HashMap;
    use std::sync::Arc;

    use arrow_schema::extension::{EXTENSION_TYPE_METADATA_KEY, EXTENSION_TYPE_NAME_KEY};
    use arrow_schema::{
        DataType as ArrowDataType, Field as ArrowField, Schema,
        ffi::{FFI_ArrowSchema, Flags},
    };
    use yggdryl::BytesType;
    use yggdryl::arrow::IPC_DICTIONARY_IDS_KEY;
    use yggdryl::{
        ArrowCastOptions, DataType, EdgeAlgorithm, Field, Nullability, StructType, TimeUnit,
        Timezone,
    };

    fn assert_flag(schema: &arrow_schema::ffi::FFI_ArrowSchema, flag: Flags) {
        assert!(schema.flags().unwrap().contains(flag));
    }

    fn assert_canonical_http_metadata(field: &ArrowField) {
        assert_eq!(field.metadata().len(), 2);
        assert_eq!(
            field
                .metadata()
                .get("HTTP:content-type")
                .map(String::as_str),
            Some("application/json")
        );
        assert_eq!(
            field
                .metadata()
                .get("HTTP:content-length")
                .map(String::as_str),
            Some("42")
        );
        assert!(!field.metadata().contains_key("HTTPS:Content-Type"));
        assert!(!field.metadata().contains_key("HTTP:Content-Length"));
    }

    #[test]
    fn arrow_import_rebuilds_noncanonical_http_metadata_for_every_ownership_path() {
        let arrow = ArrowField::new("payload", ArrowDataType::Binary, false).with_metadata(
            HashMap::from([
                (
                    "HTTPS:Content-Type".to_owned(),
                    "application/json".to_owned(),
                ),
                ("HTTP:Content-Length".to_owned(), "00042".to_owned()),
            ]),
        );

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
            DataType::from(
                StructType::from_fields([Field::new("key", DataType::utf8(), false), encoded])
                    .unwrap(),
            ),
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
        let dtype =
            DataType::from(StructType::from_fields([Field::new("lookup", map, true)]).unwrap());

        let schema = dtype.into_arrow_datatype_ffi().unwrap();
        let map = schema.child(0);
        assert_flag(map, Flags::NULLABLE);
        assert_flag(map, Flags::MAP_KEYS_SORTED);

        // A fixed layout built by hand with no width is what `validate` catches.
        assert!(DataType::FixedBinary(0).into_arrow_datatype_ffi().is_err());
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
            Some(&r#"{"layout":"fixed_ascii","charset":"us-ascii","fixed":4}"#.to_owned())
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
            DataType::dictionary(
                DataType::UInt8,
                DataType::from(StructType::from_fields([region]).unwrap()),
            )
            .unwrap(),
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
            DataType::from(
                StructType::from_fields([
                    catalog,
                    Field::new("labels", DataType::serie(item), true),
                ])
                .unwrap(),
            ),
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
            DataType::from(
                StructType::from_fields([DataType::Int64.required_field("id")]).unwrap(),
            ),
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
        let trade = StructType::from_fields([
            DataType::date32().required_field("event"),
            inner_year,
            inner_digest,
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("trade");

        let mut top_year = DataType::Int32.nullable_field("top_year");
        top_year
            .as_partition_mut()
            .set_sources(["trade.year"])
            .unwrap();
        let mut row_digest = DataType::UInt64.nullable_field("row_digest");
        row_digest.as_digest_mut().set_holder().unwrap();
        StructType::from_fields([trade, top_year, row_digest])
            .map(DataType::from)
            .unwrap()
            .required_field("row")
    }

    fn events() -> arrow_array::RecordBatch {
        arrow_array::RecordBatch::try_from_iter([(
            "trade",
            Arc::new(arrow_array::StructArray::from(vec![(
                Arc::new(ArrowField::new("event", ArrowDataType::Date32, false)),
                Arc::new(arrow_array::Date32Array::from(vec![19_723, 20_089]))
                    as arrow_array::ArrayRef,
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

    /// [`applied_root`] with a signed holder beside the unsigned one: both read
    /// the same sources, and the signed one stores the digest's bits.
    fn signed_applied_root() -> Field {
        let mut signed = DataType::Int64.nullable_field("signed_digest");
        signed.as_digest_mut().set_holder().unwrap();
        let unsigned = applied_root();
        StructType::from_fields(unsigned.fields().iter().cloned().chain([signed]))
            .map(DataType::from)
            .unwrap()
            .required_field("row")
    }

    fn events_on(days: &[i32]) -> arrow_array::RecordBatch {
        arrow_array::RecordBatch::try_from_iter([(
            "trade",
            Arc::new(arrow_array::StructArray::from(vec![(
                Arc::new(ArrowField::new("event", ArrowDataType::Date32, false)),
                Arc::new(arrow_array::Date32Array::from(days.to_vec())) as arrow_array::ArrayRef,
            )])) as arrow_array::ArrayRef,
        )])
        .unwrap()
    }

    #[test]
    fn apply_arrow_reader_digests_every_batch_as_the_batch_path_does() {
        let root = signed_applied_root();
        let batches = vec![
            events_on(&[19_723, 20_089]),
            events_on(&[20_454]),
            events_on(&[]),
        ];
        let stored = batches[0].schema();
        let streamed = |digest, transform, cast| {
            root.apply_arrow_reader(
                yggdryl::arrow::batch_reader(Arc::clone(&stored), batches.clone()),
                digest,
                transform,
                cast,
                ArrowCastOptions::new(),
            )
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
        };

        // The stream plans its digest fill once, over the batches its cast
        // step already landed; each answers what the digest verb answers over
        // that landed batch, which casts and plans for itself.
        let applied = streamed(true, true, true);
        assert_eq!(applied.len(), batches.len());
        for (batch, applied) in batches.iter().zip(&applied) {
            let landed = root
                .apply_arrow_batch(batch, false, true, true, ArrowCastOptions::new())
                .unwrap();
            assert_eq!(
                applied,
                &root.as_digest().apply_arrow_batch(&landed).unwrap()
            );
            assert_eq!(
                applied,
                &root
                    .apply_arrow_batch(batch, true, true, true, ArrowCastOptions::new())
                    .unwrap()
            );
            let unsigned = applied
                .column_by_name("row_digest")
                .unwrap()
                .as_any()
                .downcast_ref::<arrow_array::UInt64Array>()
                .unwrap();
            let signed = applied
                .column_by_name("signed_digest")
                .unwrap()
                .as_any()
                .downcast_ref::<arrow_array::Int64Array>()
                .unwrap();
            assert_eq!(arrow_array::Array::null_count(signed), 0);
            assert_eq!(
                signed
                    .values()
                    .iter()
                    .map(|value| u64::from_ne_bytes(value.to_ne_bytes()))
                    .collect::<Vec<_>>(),
                unsigned.values().to_vec()
            );
        }

        // With no cast step the fill lands every batch on the root itself.
        let digested = streamed(true, false, false);
        assert_eq!(digested.len(), batches.len());
        for (batch, digested) in batches.iter().zip(&digested) {
            assert_eq!(
                digested,
                &root.as_digest().apply_arrow_batch(batch).unwrap()
            );
        }
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
        StructType::from_fields([DataType::date32().required_field("event"), year, row_digest])
            .map(DataType::from)
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
            StructType::from_fields([
                DataType::date32().required_field("event"),
                DataType::utf8().required_field("venue"),
            ])
            .map(DataType::from)
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

    /// The storage a variant column lays out: the two binaries the Parquet
    /// Variant encoding is, named and in specification order.
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
    fn a_variant_field_projects_the_two_binaries_and_reimports_itself() {
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
        let arrow =
            ArrowField::new("raw", ArrowDataType::Binary, true).with_metadata(HashMap::from([
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
        let arrow = ArrowField::new("shape", ArrowDataType::LargeBinary, true).with_metadata(
            HashMap::from([(
                EXTENSION_TYPE_NAME_KEY.to_owned(),
                "geoarrow.wkb".to_owned(),
            )]),
        );
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
        assert!(
            matches!(imported.dtype(), DataType::Struct(_)),
            "{imported}"
        );

        // The name over a storage it does not spell is a foreign field wearing
        // it: the column imports as the binary it is.
        let foreign =
            ArrowField::new("payload", ArrowDataType::Binary, true).with_metadata(HashMap::from([
                (
                    EXTENSION_TYPE_NAME_KEY.to_owned(),
                    "arrow.parquet.variant".to_owned(),
                ),
            ]));
        let imported = Field::from_arrow_field(&foreign).unwrap();
        assert!(imported.dtype().bytes_parameters().is_some(), "{imported}");

        // The two binaries a foreign writer laid out as views are the storage
        // too: the extension names the struct, never the layout inside it.
        let views = ArrowDataType::Struct(arrow_schema::Fields::from(vec![
            ArrowField::new("metadata", ArrowDataType::BinaryView, false),
            ArrowField::new("value", ArrowDataType::LargeBinary, true),
        ]));
        let views = ArrowField::new("payload", views, true).with_metadata(HashMap::from([(
            EXTENSION_TYPE_NAME_KEY.to_owned(),
            "arrow.parquet.variant".to_owned(),
        )]));
        let imported = Field::from_arrow_field(&views).unwrap();
        assert_eq!(imported.dtype(), &DataType::Variant);
    }

    #[test]
    fn variant_metadata_is_required_while_value_may_be_nullable() {
        for (metadata_nullable, is_variant) in [(false, true), (true, false)] {
            let storage = ArrowDataType::Struct(arrow_schema::Fields::from(vec![
                ArrowField::new("value", ArrowDataType::Binary, true),
                ArrowField::new("metadata", ArrowDataType::Binary, metadata_nullable),
            ]));
            let field = ArrowField::new("payload", storage, true).with_metadata(HashMap::from([(
                EXTENSION_TYPE_NAME_KEY.to_owned(),
                "arrow.parquet.variant".to_owned(),
            )]));
            let imported = Field::from_arrow_field(&field).unwrap();
            assert_eq!(matches!(imported.dtype(), DataType::Variant), is_variant);
        }
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
                r#"{"layout":"ascii","charset":"us-ascii"}"#,
            ),
            (
                DataType::fixed_ascii(1).unwrap(),
                ArrowDataType::FixedSizeBinary(1),
                r#"{"layout":"fixed_ascii","charset":"us-ascii","fixed":1}"#,
            ),
            (
                DataType::fixed_ascii(3).unwrap(),
                ArrowDataType::FixedSizeBinary(3),
                r#"{"layout":"fixed_ascii","charset":"us-ascii","fixed":3}"#,
            ),
            (
                DataType::fixed_ascii(64).unwrap(),
                ArrowDataType::FixedSizeBinary(64),
                r#"{"layout":"fixed_ascii","charset":"us-ascii","fixed":64}"#,
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
        let large = ArrowField::new("ccy", ArrowDataType::LargeBinary, true).with_metadata(
            HashMap::from([
                (
                    EXTENSION_TYPE_NAME_KEY.to_owned(),
                    "yggdryl.string".to_owned(),
                ),
                (EXTENSION_TYPE_METADATA_KEY.to_owned(), document.to_owned()),
            ]),
        );
        let imported = Field::from_arrow_field(&large).unwrap();
        assert_eq!(imported.dtype(), &DataType::large_binary());
        assert_eq!(
            imported.get_metadata(EXTENSION_TYPE_NAME_KEY),
            Some("yggdryl.string")
        );
        assert_eq!(imported.into_arrow_field().unwrap(), large);

        // `yggdryl.ascii` names nothing this crate has.
        let retired = ArrowField::new("ccy", ArrowDataType::FixedSizeBinary(4), true)
            .with_metadata(HashMap::from([(
                EXTENSION_TYPE_NAME_KEY.to_owned(),
                "yggdryl.ascii".to_owned(),
            )]));
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
        let large = ArrowField::new("key", ArrowDataType::LargeBinary, true).with_metadata(
            HashMap::from([
                (
                    EXTENSION_TYPE_NAME_KEY.to_owned(),
                    "yggdryl.bytes".to_owned(),
                ),
                (EXTENSION_TYPE_METADATA_KEY.to_owned(), document.to_owned()),
            ]),
        );
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
}

mod generic {
    use arrow_schema::{DataType as ArrowDataType, Field as ArrowField};
    use std::collections::{BTreeSet, HashSet};
    use std::sync::Arc;

    use yggdryl::{DataType, Field, StructType, TimeUnit};

    fn arrow_field_with_nested_noncanonical_location() -> ArrowField {
        let leaf = Arc::new(
            ArrowField::new("item", ArrowDataType::Int64, false).with_metadata(
                std::collections::HashMap::from([(
                    "location".to_owned(),
                    "HTTPS://example.com/table".to_owned(),
                )]),
            ),
        );
        let items = Arc::new(ArrowField::new("items", ArrowDataType::List(leaf), false));
        ArrowField::new("root", ArrowDataType::Struct(vec![items].into()), false)
    }

    fn nested_location(field: &ArrowField) -> Option<&str> {
        let ArrowDataType::Struct(fields) = field.data_type() else {
            return None;
        };
        let ArrowDataType::List(item) = fields.first()?.data_type() else {
            return None;
        };
        item.metadata().get("location").map(String::as_str)
    }

    #[test]
    #[allow(clippy::mutable_key_type)]
    fn native_order_hash_json_and_stable_hash_are_value_based() {
        let left =
            Field::from_str(r#"field("a",struct<x:int64>,nullable=false,metadata={"z":"1"})"#)
                .unwrap();
        let right =
            Field::from_str(r#"field("b",struct<x:int64>,nullable=false,metadata={"z":"1"})"#)
                .unwrap();

        let mut ordered = BTreeSet::from([right.clone(), left.clone()]);
        assert_eq!(ordered.pop_first(), Some(left.clone()));
        let hashed = HashSet::from([right.clone(), left.clone()]);
        assert!(hashed.contains(&left));
        assert_ne!(left.stable_hash(), right.stable_hash());

        let json = right.clone().into_json().unwrap();
        assert_eq!(Field::from_json(&json).unwrap(), right);
    }

    #[test]
    fn borrowed_and_consuming_arrow_paths_are_lossless() {
        let field = Field::from_str(
            r#"field("items",array<struct<id:bigint,name:string>>,nullable=false,metadata={"source":"test"})"#,
        )
        .unwrap();
        let first = Arc::new(field.clone().into_arrow_field().unwrap());
        let field = Field::from_arrow_field_ref(Arc::clone(&first)).unwrap();
        let second = field.clone().into_arrow_field_ref().unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(Field::from_arrow_field(first.as_ref()).unwrap(), field);

        let owned = field.clone().into_arrow_field().unwrap();
        assert_eq!(Field::try_from(owned).unwrap(), field);
        let shared = field.clone().into_arrow_field_ref().unwrap();
        let imported = Field::from_arrow_field_ref(Arc::clone(&shared)).unwrap();
        assert_eq!(imported, field);
        assert!(Arc::ptr_eq(
            &shared,
            &imported.into_arrow_field_ref().unwrap()
        ));
    }

    #[test]
    #[allow(deprecated)]
    fn arrow_dictionary_options_survive_parsing_and_cache_invalidation() {
        let arrow = ArrowField::new_dict(
            "codes",
            ArrowDataType::Dictionary(
                Box::new(ArrowDataType::Int16),
                Box::new(ArrowDataType::Utf8),
            ),
            true,
            41,
            true,
        );
        let mut field = Field::from_arrow_field(&arrow).unwrap();

        assert_eq!(field.dictionary_id(), Some(41));
        assert_eq!(field.dictionary_is_ordered(), Some(true));
        assert_eq!(Field::from_str(&field.to_string()).unwrap(), field);
        assert_eq!(Field::from_str(&arrow.to_string()).unwrap(), field);

        let nested = DataType::from(StructType::from_fields([field.clone()]).unwrap());
        assert_eq!(DataType::from_str(&nested.to_string()).unwrap(), nested);
        let mut different_field = field.clone();
        different_field.set_dictionary_options(42, false).unwrap();
        let different = DataType::from(StructType::from_fields([different_field]).unwrap());
        assert_ne!(nested, different);
        assert_ne!(nested.cmp(&different), std::cmp::Ordering::Equal);

        let json: serde_json::Value =
            serde_json::from_str(&field.clone().into_json().unwrap()).unwrap();
        assert_eq!(json["dictionary_id"], "41");
        assert_eq!(Field::from_json(&json.to_string()).unwrap(), field);

        let cached = field.clone().into_arrow_field_ref().unwrap();
        field.set_name("renamed_codes");
        let rebuilt = field.clone().into_arrow_field_ref().unwrap();
        assert!(!Arc::ptr_eq(&cached, &rebuilt));
        assert_eq!(rebuilt.dict_id(), Some(41));
        assert_eq!(rebuilt.dict_is_ordered(), Some(true));
    }

    #[test]
    fn no_op_metadata_update_retains_arrow_cache_effective_update_invalidates_it() {
        let field = Field::new("id", DataType::Int64, false)
            .try_with_metadata("source", "one")
            .unwrap();
        let original = Arc::new(field.clone().into_arrow_field().unwrap());
        let mut field = Field::from_arrow_field_ref(Arc::clone(&original)).unwrap();
        field.insert_metadata("source", "one").unwrap();
        assert!(Arc::ptr_eq(
            &original,
            &field.clone().into_arrow_field_ref().unwrap()
        ));

        field.insert_metadata("source", "two").unwrap();
        let changed = field.clone().into_arrow_field_ref().unwrap();
        assert!(!Arc::ptr_eq(&original, &changed));
    }

    #[test]
    fn invalid_dtype_replacement_is_transactional() {
        let mut field = Field::new("value", DataType::utf8(), true);
        assert!(
            field
                .set_dtype(DataType::Time32(TimeUnit::Nanosecond))
                .is_err()
        );
        assert_eq!(field.dtype(), &DataType::utf8());
    }

    #[test]
    fn arrow_cache_is_rebuilt_when_typed_metadata_is_canonicalized() {
        let arrow = Arc::new(
            ArrowField::new("id", ArrowDataType::Int64, false).with_metadata(
                std::collections::HashMap::from([(
                    "location".to_owned(),
                    "HTTPS://example.com/table".to_owned(),
                )]),
            ),
        );
        let field = Field::from_arrow_field_ref(Arc::clone(&arrow)).unwrap();
        assert_eq!(
            field.location().unwrap().map(|url| url.to_string()),
            Some("https://example.com/table".to_owned())
        );

        let projected = field.into_arrow_field_ref().unwrap();
        assert!(!Arc::ptr_eq(&arrow, &projected));
        assert_eq!(
            projected.metadata().get("location").map(String::as_str),
            Some("https://example.com/table")
        );
    }

    #[test]
    fn borrowed_arrow_import_rebuilds_parent_for_nested_canonicalization() {
        let arrow = arrow_field_with_nested_noncanonical_location();
        let field = Field::from_arrow_field(&arrow).unwrap();
        let projected = field.into_arrow_field().unwrap();

        assert_eq!(nested_location(&arrow), Some("HTTPS://example.com/table"));
        assert_eq!(
            nested_location(&projected),
            Some("https://example.com/table")
        );
    }

    #[test]
    fn shared_arrow_import_rebuilds_parent_for_nested_canonicalization() {
        let arrow = Arc::new(arrow_field_with_nested_noncanonical_location());
        let field = Field::from_arrow_field_ref(Arc::clone(&arrow)).unwrap();
        let projected = field.into_arrow_field_ref().unwrap();

        assert!(!Arc::ptr_eq(&arrow, &projected));
        assert_eq!(
            nested_location(&projected),
            Some("https://example.com/table")
        );

        let canonical = Field::from_arrow_field_ref(Arc::clone(&projected)).unwrap();
        assert!(Arc::ptr_eq(
            &projected,
            &canonical.into_arrow_field_ref().unwrap()
        ));
    }

    #[test]
    fn owned_arrow_import_rebuilds_parent_for_nested_canonicalization() {
        let field = Field::try_from(arrow_field_with_nested_noncanonical_location()).unwrap();
        let projected = field.into_arrow_field().unwrap();

        assert_eq!(
            nested_location(&projected),
            Some("https://example.com/table")
        );
    }

    #[test]
    fn the_init_flag_defaults_to_true_and_stores_only_when_false() {
        let mut field = Field::new("total", DataType::Int64, true);

        // An ordinary field participates in initialization and carries no key.
        assert!(field.is_init().unwrap());
        assert!(!field.has_metadata("FIELD:init"));

        // Marking it derived stores exactly one canonical value.
        field.set_init(false);
        assert!(!field.is_init().unwrap());
        assert_eq!(field.get_metadata("FIELD:init"), Some("false"));

        // Restoring the default removes the key rather than storing `true`.
        field.set_init(true);
        assert!(field.is_init().unwrap());
        assert!(!field.has_metadata("FIELD:init"));

        // The consuming form mirrors the setter.
        let derived = Field::new("total", DataType::Int64, true).with_init(false);
        assert!(!derived.is_init().unwrap());
    }

    #[test]
    fn the_init_flag_rejects_a_non_boolean_spelling() {
        let error = Field::from_parts("total", DataType::Int64, true, [("FIELD:init", "yes")])
            .unwrap_err()
            .to_string();
        assert!(error.contains("expected true or false"), "{error}");
        assert!(error.contains("\"yes\""), "{error}");

        // The canonical spellings are accepted and round-trip.
        for (text, expected) in [("true", true), ("false", false)] {
            let field =
                Field::from_parts("total", DataType::Int64, true, [("FIELD:init", text)]).unwrap();
            assert_eq!(field.is_init().unwrap(), expected, "{text}");
        }
    }

    #[test]
    fn datatype_builds_fields_in_schema_reading_order() {
        let id = DataType::Int64.named_field("id", false);
        assert_eq!(id, Field::new("id", DataType::Int64, false));

        assert_eq!(
            DataType::utf8().nullable_field("note"),
            Field::new("note", DataType::utf8(), true)
        );
        assert_eq!(
            DataType::utf8().required_field("symbol"),
            Field::new("symbol", DataType::utf8(), false)
        );

        // A nested type composes without naming the inner type twice.
        let tags = DataType::serie(DataType::utf8().nullable_field("item")).nullable_field("tags");
        assert!(tags.dtype().is_nested());
        assert_eq!(tags.name(), "tags");
    }

    #[test]
    fn a_struct_field_is_usable_as_a_schema_root() {
        let root = StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::utf8().nullable_field("symbol"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row");

        // A struct field carries everything a schema needs: the column list, the
        // per-column lookup, and validation of its own root-ness.
        assert!(root.is_struct());
        root.validate_struct_root().unwrap();
        assert_eq!(root.field_len(), 2);
        assert_eq!(root.fields().len(), 2);
        assert_eq!(root.index_of("symbol"), Some(1));
        assert_eq!(root.index_of("absent"), None);
        assert_eq!(root.get_field(0).unwrap().name(), "id");
        assert_eq!(root.get_field_by_path("symbol").unwrap().name(), "symbol");
    }

    #[test]
    fn a_root_must_be_a_non_null_struct_and_says_why() {
        let scalar = DataType::Int64.required_field("value");
        assert!(!scalar.is_struct());
        assert!(scalar.fields().is_empty());
        let message = scalar.validate_struct_root().unwrap_err().to_string();
        assert!(message.contains("expected a struct root"), "{message}");
        assert!(message.contains("int64"), "{message}");

        let nullable = StructType::from_fields([DataType::Int64.required_field("id")])
            .map(DataType::from)
            .unwrap()
            .nullable_field("row");
        let message = nullable.validate_struct_root().unwrap_err().to_string();
        assert!(message.contains("non-null struct root"), "{message}");
        assert!(message.contains("\"row\""), "{message}");
    }

    #[test]
    fn one_walk_numbers_finds_and_bounds_every_identifier_in_a_tree() {
        let mut schema = StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::serie(DataType::utf8().nullable_field("item")).nullable_field("tags"),
            StructType::from_fields([DataType::Int32.required_field("depth")])
                .map(DataType::from)
                .unwrap()
                .nullable_field("book"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row");

        // Numbering is depth first in declaration order, over every child a
        // layout has - a serie's item and a map's entries included.
        assert_eq!(schema.assign_parquet_field_ids(1).unwrap(), 6);
        assert_eq!(schema.max_parquet_field_id().unwrap(), Some(5));
        assert_eq!(
            schema.field_by_parquet_field_id(1).map(Field::name),
            Some("id")
        );
        assert_eq!(
            schema.field_by_parquet_field_id(3).map(Field::name),
            Some("item")
        );
        assert_eq!(
            schema.field_by_parquet_field_id(5).map(Field::name),
            Some("depth")
        );
        assert_eq!(schema.field_by_parquet_field_id(9), None);

        // A field that already carries an identifier keeps it, so evolving a
        // schema never renumbers the columns that were already there.
        let mut evolved = schema
            .clone()
            .try_with_dtype(
                StructType::from_fields(
                    schema
                        .fields()
                        .iter()
                        .cloned()
                        .chain([DataType::utf8().nullable_field("venue")]),
                )
                .map(DataType::from)
                .unwrap(),
            )
            .unwrap();
        assert_eq!(
            evolved
                .assign_parquet_field_ids(schema.max_parquet_field_id().unwrap().unwrap() + 1)
                .unwrap(),
            7
        );
        assert_eq!(
            evolved.field_by_parquet_field_id(1).map(Field::name),
            Some("id")
        );
        assert_eq!(
            evolved.field_by_parquet_field_id(6).map(Field::name),
            Some("venue")
        );
    }

    #[test]
    fn a_datatype_rebuilds_any_layout_from_replacement_children() {
        let list = DataType::serie(DataType::Int32.nullable_field("item"));
        assert_eq!(
            list.with_fields([DataType::Int64.nullable_field("item")])
                .unwrap(),
            DataType::serie(DataType::Int64.nullable_field("item"))
        );

        // A union keeps its type ids and its mode; only the members change.
        let union = DataType::union(
            [
                (7_i8, DataType::Int64.nullable_field("number")),
                (9_i8, DataType::utf8().nullable_field("text")),
            ],
            yggdryl::UnionMode::Dense,
        )
        .unwrap();
        let rebuilt = union
            .with_fields([
                DataType::Int32.nullable_field("number"),
                DataType::utf8().nullable_field("text"),
            ])
            .unwrap();
        assert_eq!(
            rebuilt.to_string(),
            union.to_string().replace("int64", "int32")
        );

        // The arity is the layout's, and a mismatch says which was expected.
        let message = list
            .with_fields([
                DataType::Int64.nullable_field("item"),
                DataType::Int64.nullable_field("extra"),
            ])
            .unwrap_err()
            .to_string();
        assert!(message.contains("1 children"), "{message}");
        assert!(message.contains("got 2"), "{message}");

        // A scalar has no children, so replacing none of them is itself.
        assert_eq!(DataType::Int64.with_fields([]).unwrap(), DataType::Int64);
    }

    #[test]
    fn subscripting_a_schema_node_reaches_a_nested_child() {
        let line = StructType::from_fields([
            DataType::Float64.required_field("price"),
            DataType::Int64.required_field("qty"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("line");
        let mut order = StructType::from_fields([
            DataType::Int64.required_field("id"),
            line.clone(),
            DataType::serie(DataType::utf8().nullable_field("tag")).nullable_field("tags"),
            DataType::map_of(DataType::utf8(), DataType::Int64, false)
                .unwrap()
                .nullable_field("counts"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("order");
        order.insert_metadata("owner", "trading").unwrap();

        // By name and by position, on the Field and on the DataType alike.
        assert_eq!(order["id"].dtype(), &DataType::Int64);
        assert_eq!(order[0].name(), "id");
        assert_eq!(order.dtype()["id"].dtype(), &DataType::Int64);
        assert_eq!(order.dtype()[1].name(), "line");

        // Chained descent, two levels and through a Serie and a Map.
        assert_eq!(order["line"]["price"].dtype(), &DataType::Float64);
        assert_eq!(order["tags"][0].name(), "tag");
        assert_eq!(order["counts"]["entries"]["key"].dtype(), &DataType::utf8());
        assert_eq!(order["counts"][0]["value"].dtype(), &DataType::Int64);

        // Metadata is not reachable by subscript any more, and is still reachable
        // through its own view and the named accessor.
        assert_eq!(order.get_metadata("owner"), Some("trading"));
        assert_eq!(order.as_metadata().get("owner"), Some("trading"));
        assert!(order.get_field_by_path("owner").is_none());
    }

    #[test]
    #[should_panic(expected = "is not a child of the field")]
    fn subscripting_an_absent_child_panics_by_name() {
        let row = StructType::from_fields([DataType::Int64.required_field("id")])
            .map(DataType::from)
            .unwrap()
            .required_field("row");
        let _ = &row["absent"];
    }

    #[test]
    #[should_panic(expected = "so position 3 is out of range")]
    fn subscripting_an_absent_child_panics_by_position() {
        let row = StructType::from_fields([DataType::Int64.required_field("id")])
            .map(DataType::from)
            .unwrap()
            .required_field("row");
        let _ = &row[3];
    }

    #[test]
    #[should_panic(expected = "is not a child of the datatype")]
    fn subscripting_a_scalar_datatype_panics() {
        let _ = &DataType::Int64["anything"];
    }

    #[test]
    fn child_mutation_replaces_by_position_and_appends_by_unknown_name() {
        let mut row = StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::utf8().required_field("venue"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row");

        // A known name replaces in place, keeping its position.
        row.set_field_by_path("id", DataType::utf8().required_field("id"))
            .unwrap();
        assert_eq!(row.field_len(), 2);
        assert_eq!(row[0].name(), "id");
        assert_eq!(row["id"].dtype(), &DataType::utf8());

        // An unknown name appends.
        row.set_field_by_path("price", DataType::Float64.nullable_field("price"))
            .unwrap();
        assert_eq!(row.field_len(), 3);
        assert_eq!(row[2].name(), "price");

        // A position replaces only, and never grows the node.
        row.set_field(1, DataType::Int32.required_field("venue"))
            .unwrap();
        assert_eq!(row.field_len(), 3);
        assert_eq!(row["venue"].dtype(), &DataType::Int32);
        let refused = row
            .set_field(9, DataType::Int64.required_field("late"))
            .unwrap_err();
        assert!(refused.to_string().contains("below 3"), "{refused}");
        assert_eq!(row.field_len(), 3, "a refusal leaves the field unchanged");

        // Removal closes the gap, by either key form.
        let dropped = row.remove_field_by_path("id").unwrap();
        assert_eq!(dropped.name(), "id");
        assert_eq!(row[0].name(), "venue");
        row.remove_field(0).unwrap();
        assert_eq!(row.field_len(), 1);
        assert_eq!(row[0].name(), "price");

        // A non-struct has no children to replace, and says so.
        let mut scalar = DataType::Int64.required_field("id");
        let refused = scalar
            .set_field_by_path("child", DataType::Int64.required_field("child"))
            .unwrap_err();
        assert!(refused.to_string().contains("struct field"), "{refused}");
    }

    #[test]
    fn child_mutation_invalidates_the_arrow_cache_exactly_once() {
        let mut row = StructType::from_fields([DataType::Int64.required_field("id")])
            .map(DataType::from)
            .unwrap()
            .required_field("row");
        let before = row.clone().into_arrow_field().unwrap();
        assert!(before.data_type().to_string().contains("id"));
        assert!(!before.data_type().to_string().contains("venue"));

        row.set_field_by_path("venue", DataType::utf8().nullable_field("venue"))
            .unwrap();

        // The projection is rebuilt from the mutated field, never served stale.
        let after = row.into_arrow_field().unwrap();
        assert!(
            after.data_type().to_string().contains("venue"),
            "{}",
            after.data_type()
        );
    }
}

mod nested {
    use super::typed::assert_typed_marker;
    use yggdryl::{DataType, Field, StructType, UnionMode};

    #[test]
    fn nested_markers_cover_every_child_layout() {
        let item = || Field::new("item", DataType::utf8(), true);
        assert_typed_marker::<yggdryl::SerieType>(DataType::serie(item()));
        assert_typed_marker::<yggdryl::SerieType>(DataType::serie_view(item()));
        assert_typed_marker::<yggdryl::SerieType>(DataType::fixed_size_serie(item(), 3).unwrap());
        assert_typed_marker::<yggdryl::SerieType>(DataType::large_serie(item()));
        assert_typed_marker::<yggdryl::SerieType>(DataType::large_serie_view(item()));
        assert_typed_marker::<yggdryl::StructType>(DataType::from(
            StructType::from_fields([item()]).unwrap(),
        ));
        assert_typed_marker::<yggdryl::UnionType>(
            DataType::union([(4, item())], UnionMode::Dense).unwrap(),
        );
        assert_typed_marker::<yggdryl::EnumType>(
            DataType::dictionary(DataType::Int16, DataType::utf8()).unwrap(),
        );
        assert_typed_marker::<yggdryl::MappingType>(
            DataType::map_of(DataType::utf8(), DataType::Int64, false).unwrap(),
        );
        assert_typed_marker::<yggdryl::RunEndType>(
            DataType::run_end_encoded(
                Field::new("run_ends", DataType::Int32, false),
                Field::new("values", DataType::utf8(), true),
            )
            .unwrap(),
        );
    }

    /// Item access on a schema node reaches a nested child, never metadata.
    ///
    /// Before this, `field["level"]` was a metadata lookup while
    /// `dtype["level"]` was a child, so a caller walking one object graph got
    /// two unrelated things from identical syntax. Children win: subscripting a
    /// schema node descends the schema.
    #[test]
    fn subscripting_a_schema_node_reaches_a_nested_child() {
        let line = StructType::from_fields([
            DataType::Float64.required_field("price"),
            DataType::Int64.required_field("qty"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("line");
        let order = StructType::from_fields([DataType::Int64.required_field("id"), line])
            .map(DataType::from)
            .unwrap()
            .required_field("order");

        // By name and by position, on both `Field` and `DataType`, one answer.
        assert_eq!(order["id"].dtype(), &DataType::Int64);
        assert_eq!(order.dtype()["id"].dtype(), &DataType::Int64);
        assert_eq!(order[0].name(), "id");
        assert_eq!(order.dtype()[1].name(), "line");

        // Chained subscripts are the nesting story - no dotted path form.
        assert_eq!(order["line"]["price"].dtype(), &DataType::Float64);
        assert_eq!(order["line"]["qty"].dtype(), &DataType::Int64);

        // Through a Serie item and a Map entry, the same way.
        let items = DataType::serie(order.clone().with_name("item"));
        assert_eq!(items[0]["id"].dtype(), &DataType::Int64);
        assert_eq!(items["item"]["line"]["price"].dtype(), &DataType::Float64);

        // The non-panicking form stays available and is what the docs point at.
        assert!(order.get_field_by_path("absent").is_none());
        assert!(order.get_field(9).is_none());
    }

    /// Metadata is not reachable by subscript any more, but is through its view.
    #[test]
    fn metadata_is_reached_through_its_own_surface_not_a_subscript() {
        let mut field = StructType::from_fields([DataType::Int64.required_field("id")])
            .map(DataType::from)
            .unwrap()
            .required_field("row");
        field.insert_metadata("owner", "tests").unwrap();

        // The subscript descends the schema; the metadata key is not a child.
        assert_eq!(field["id"].dtype(), &DataType::Int64);
        assert!(field.get_field_by_path("owner").is_none());

        // The named accessors and the view still answer it.
        assert_eq!(field.get_metadata("owner"), Some("tests"));
        assert_eq!(field.as_metadata().get("owner"), Some("tests"));
    }

    #[test]
    #[should_panic(expected = "is not a child of the field")]
    fn subscripting_an_absent_child_panics_with_a_useful_message() {
        let row = StructType::from_fields([DataType::Int64.required_field("id")])
            .map(DataType::from)
            .unwrap()
            .required_field("row");
        let _ = &row["absent"];
    }

    #[test]
    #[should_panic(expected = "is not a child of the datatype")]
    fn subscripting_a_non_nested_datatype_panics_naming_it() {
        let _ = &DataType::Int64["anything"];
    }

    #[test]
    #[should_panic(expected = "so position 5 is out of range")]
    fn subscripting_past_the_end_panics_naming_the_arity() {
        let row = StructType::from_fields([DataType::Int64.required_field("id")])
            .map(DataType::from)
            .unwrap()
            .required_field("row");
        let _ = &row[5];
    }

    /// Child mutation is named and cache-aware; no `&mut` child escapes it.
    #[test]
    fn child_mutation_replaces_by_position_and_appends_by_unknown_name() {
        let mut row = StructType::from_fields([DataType::Int64.required_field("id")])
            .map(DataType::from)
            .unwrap()
            .required_field("row");

        // An unknown name appends - dict-like, and how a schema is built up.
        row.set_field_by_path("venue", DataType::utf8().nullable_field("venue"))
            .unwrap();
        assert_eq!(row.field_len(), 2);
        assert_eq!(row[1].name(), "venue");

        // A known name replaces in place, keeping its position.
        row.set_field_by_path("id", DataType::utf8().required_field("id"))
            .unwrap();
        assert_eq!(row.field_len(), 2);
        assert_eq!(row[0].name(), "id");
        assert_eq!(row["id"].dtype(), &DataType::utf8());

        // A position replaces only, and never grows the node silently.
        row.set_field(1, DataType::large_utf8().nullable_field("venue"))
            .unwrap();
        assert_eq!(row["venue"].dtype(), &DataType::large_utf8());
        let message = row
            .set_field(7, DataType::Int64.nullable_field("late"))
            .unwrap_err()
            .to_string();
        assert!(message.contains("a child position below 2"), "{message}");
        assert_eq!(row.field_len(), 2, "a refusal leaves the field unchanged");

        // Removal returns the prior child and closes the gap.
        let dropped = row.remove_field_by_path("id").unwrap();
        assert_eq!(dropped.name(), "id");
        assert_eq!(row.field_len(), 1);
        assert_eq!(row[0].name(), "venue");

        // A node with no children to replace says so rather than panicking.
        let mut scalar = DataType::Int64.required_field("price");
        let message = scalar
            .set_field_by_path("child", DataType::Int64.nullable_field("child"))
            .unwrap_err()
            .to_string();
        assert!(message.contains("a struct field"), "{message}");
    }

    #[test]
    fn a_datatype_replaces_removes_and_keeps_its_layout() {
        // A position replaces through every layout, keeping it.
        let mut list = DataType::serie(DataType::Int32.nullable_field("item"));
        list.set_field_at(0, DataType::Int64.nullable_field("item"))
            .unwrap();
        assert_eq!(
            list,
            DataType::serie(DataType::Int64.nullable_field("item"))
        );

        // Growing or shrinking is a struct's business: a serie holds exactly one
        // child, so it refuses rather than quietly becoming a struct.
        let message = list
            .set_field_by_path("extra", DataType::utf8().nullable_field("extra"))
            .unwrap_err()
            .to_string();
        assert!(message.contains("a struct field"), "{message}");
        assert!(list.remove_field_at(0).is_err());
        assert_eq!(
            list,
            DataType::serie(DataType::Int64.nullable_field("item"))
        );

        // A struct grows by an unresolved name and shrinks by either key.
        let mut row = DataType::from(
            StructType::from_fields([DataType::Int64.required_field("id")]).unwrap(),
        );
        row.set_field("venue", DataType::utf8().nullable_field("venue"))
            .unwrap();
        assert_eq!(row.field_len(), 2);
        assert_eq!(row.remove_field("venue").unwrap().name(), "venue");
        assert_eq!(row.remove_field(0).unwrap().name(), "id");
        assert_eq!(row.field_len(), 0);
    }

    #[test]
    fn unnesting_flattens_structs_to_leaves_named_by_their_path() {
        let row = StructType::from_fields([
            DataType::Int64.required_field("id"),
            StructType::from_fields([
                DataType::Float64.required_field("px"),
                StructType::from_fields([DataType::utf8().required_field("ccy")])
                    .map(DataType::from)
                    .unwrap()
                    .required_field("meta"),
            ])
            .map(DataType::from)
            .unwrap()
            .nullable_field("line"),
            DataType::serie(DataType::Float64.nullable_field("item")).nullable_field("levels"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row");

        let leaves = row.unnest_fields();
        let names: Vec<&str> = leaves.iter().map(Field::name).collect();

        // Structs flatten all the way down; a serie is a leaf, not its item.
        assert_eq!(names, ["id", "line.px", "line.meta.ccy", "levels"]);

        // A leaf under a nullable ancestor is nullable, because a null parent
        // leaves it with no value to carry.
        assert!(!leaves[0].is_nullable());
        assert!(leaves[1].is_nullable(), "px is required, but line is not");
        assert!(leaves[2].is_nullable());

        // Every name it answers is one the path accessor resolves, so a flattened
        // column list and the tree it came from address children the same way.
        for leaf in &leaves {
            assert!(
                row.get_field_by_path(leaf.name()).is_some(),
                "{:?} must resolve",
                leaf.name()
            );
        }

        // A node with no children answers nothing rather than failing.
        assert!(DataType::Int64.unnest_fields().is_empty());
    }

    #[test]
    fn exploding_replaces_each_collection_with_what_it_holds() {
        let row = StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::serie(DataType::Float64.nullable_field("item")).nullable_field("levels"),
            DataType::map_of(DataType::utf8(), DataType::Int64, true)
                .unwrap()
                .nullable_field("tags"),
            DataType::dictionary(DataType::Int16, DataType::utf8())
                .unwrap()
                .required_field("codes"),
        ])
        .map(DataType::from)
        .unwrap();

        let exploded = row.explode_fields();

        // Same columns, same order: one row's worth of an expanded table.
        assert_eq!(exploded.len(), row.field_len());
        assert_eq!(
            exploded.iter().map(Field::name).collect::<Vec<_>>(),
            ["id", "levels", "tags", "codes"],
        );

        assert_eq!(exploded[0].dtype(), &DataType::Int64, "not a collection");
        assert_eq!(
            exploded[1].dtype(),
            &DataType::Float64,
            "a serie answers its item"
        );
        assert!(
            exploded[2].dtype().as_fields().is_some(),
            "a map answers its entries"
        );
        assert_eq!(
            exploded[3].dtype(),
            &DataType::utf8(),
            "a dictionary answers its value"
        );

        // The column keeps its name and is nullable when the collection or its
        // element is: an absent serie yields no element.
        assert!(exploded[1].is_nullable());

        // One level only, so the depth is the caller's decision.
        let deep = StructType::from_fields([DataType::serie(
            DataType::serie(DataType::Int64.nullable_field("item")).nullable_field("item"),
        )
        .nullable_field("deep")])
        .map(DataType::from)
        .unwrap();
        let once = DataType::from(StructType::from_fields(deep.explode_fields()).unwrap());
        assert!(matches!(once.explode_fields()[0].dtype(), DataType::Int64));
    }
}

mod declared {
    use arrow_schema::{DataType as ArrowDataType, Field as ArrowField};
    use std::collections::{BTreeSet, HashSet};
    use std::sync::Arc;
    use yggdryl::Field;
    use yggdryl::{DataType, Error};

    #[test]
    fn canonical_display_json_and_arrow_round_trip() {
        let field = Field::new(
            "items",
            DataType::serie(Field::new("item", DataType::utf8(), true)),
            false,
        )
        .try_with_metadata("source", "a, b")
        .unwrap();

        assert_eq!(Field::from_str(&field.to_string()).unwrap(), field);
        assert_eq!(
            Field::from_json(&field.clone().into_json().unwrap()).unwrap(),
            field
        );
        let arrow = field.clone().into_arrow_field_ref().unwrap();
        assert_eq!(arrow, field.clone().into_arrow_field_ref().unwrap());
        assert_eq!(Field::from_arrow_field(arrow.as_ref()).unwrap(), field);
    }

    #[test]
    fn borrowing_the_arrow_projection_builds_it_once() {
        let field = Field::from_str("price decimal(18,4) not null").unwrap();

        let first = field.as_arrow_field_ref().unwrap();
        let second = field.as_arrow_field_ref().unwrap();
        // The second call answers the cached projection, not a rebuilt one.
        assert!(Arc::ptr_eq(first, second));

        // Borrowing and consuming agree, and the field survives the borrow.
        assert_eq!(
            field.as_arrow_field_ref().unwrap(),
            &field.clone().into_arrow_field_ref().unwrap()
        );
        assert_eq!(
            Field::from_arrow_field(field.as_arrow_field_ref().unwrap().as_ref()).unwrap(),
            field
        );
    }

    #[test]
    fn sql_hive_and_wrapped_forms_parse() {
        assert_eq!(
            Field::from_str("id bigint not null").unwrap().dtype(),
            &DataType::Int64
        );
        assert!(!Field::from_str("id bigint not null").unwrap().is_nullable());
        assert_eq!(
            Field::from_str("['events': array<struct<id:bigint,name:string>>]")
                .unwrap()
                .name(),
            "events"
        );
        assert!(
            !Field::from_str("id bigint  NOT \t NULL")
                .unwrap()
                .is_nullable()
        );
        assert_eq!(Field::from_str("'it''s': string").unwrap().name(), "it's");
        assert_eq!(Field::from_str(r#""a""b": string"#).unwrap().name(), "a\"b");
        assert_eq!(Field::from_str("[a]]b] string").unwrap().name(), "a]b");
    }

    #[test]
    #[allow(deprecated)]
    fn arrow_display_and_dictionary_state_round_trip_after_cache_invalidation() {
        let arrow = ArrowField::new_dict(
            "codes",
            ArrowDataType::Dictionary(
                Box::new(ArrowDataType::Int16),
                Box::new(ArrowDataType::Utf8),
            ),
            true,
            42,
            true,
        )
        .with_metadata(std::collections::HashMap::from([(
            "source".to_owned(),
            "ipc".to_owned(),
        )]));
        let field = Field::from_arrow_field(&arrow).unwrap();
        assert_eq!(Field::from_str(&arrow.to_string()).unwrap(), field);
        assert_eq!(Field::from_str(&field.to_string()).unwrap(), field);
        assert_eq!(field.dictionary_id(), Some(42));
        assert_eq!(field.dictionary_is_ordered(), Some(true));

        let cached = Arc::new(field.clone().into_arrow_field().unwrap());
        let mut field = Field::from_arrow_field_ref(Arc::clone(&cached)).unwrap();
        assert!(Arc::ptr_eq(
            &cached,
            &field.clone().into_arrow_field_ref().unwrap()
        ));
        field.set_dictionary_options(42, true).unwrap();
        assert!(Arc::ptr_eq(
            &cached,
            &field.clone().into_arrow_field_ref().unwrap()
        ));
        field.set_dictionary_options(7, false).unwrap();
        assert!(!Arc::ptr_eq(
            &cached,
            &field.clone().into_arrow_field_ref().unwrap()
        ));
        field.set_dictionary_options(42, true).unwrap();

        field.set_name("renamed");
        let rebuilt = field.into_arrow_field().unwrap();
        assert_eq!(rebuilt.dict_id(), Some(42));
        assert_eq!(rebuilt.dict_is_ordered(), Some(true));

        let shared = Arc::new(arrow);
        let imported = Field::from_arrow_field_ref(Arc::clone(&shared)).unwrap();
        assert!(Arc::ptr_eq(
            &shared,
            &imported.into_arrow_field_ref().unwrap()
        ));
    }

    #[test]
    fn wrappers_are_bounded_and_nested_errors_use_field_offsets() {
        let accepted = format!(
            "{}id:int64{}",
            "(".repeat(DataType::PARSE_RECURSION_LIMIT),
            ")".repeat(DataType::PARSE_RECURSION_LIMIT)
        );
        assert_eq!(Field::from_str(&accepted).unwrap().name(), "id");
        let rejected_depth = DataType::PARSE_RECURSION_LIMIT + 1;
        let rejected = format!(
            "{}id:int64{}",
            "(".repeat(rejected_depth),
            ")".repeat(rejected_depth)
        );
        assert!(Field::from_str(&rejected).is_err());

        let error = Field::from_str("id: struct<x: definitely_bad>").unwrap_err();
        assert!(matches!(
            error,
            Error::Parse {
                target: "field",
                position: 3..,
                ..
            }
        ));
    }

    #[test]
    fn metadata_updates_are_sorted_atomic_and_cache_aware() {
        let mut field = Field::new("id", DataType::Int64, false);
        field
            .update_metadata([("z", "last"), ("a", "first")])
            .unwrap();
        assert_eq!(
            field.metadata_iter().collect::<Vec<_>>(),
            vec![("a", "first"), ("z", "last")]
        );
        let cached = Arc::new(field.clone().into_arrow_field().unwrap());
        let mut field = Field::from_arrow_field_ref(Arc::clone(&cached)).unwrap();
        field.insert_metadata("a", "first").unwrap();
        assert!(Arc::ptr_eq(
            &cached,
            &field.clone().into_arrow_field_ref().unwrap()
        ));
        assert!(field.update_metadata([("", "bad")]).is_err());
        assert_eq!(field.metadata_len(), 2);
    }

    #[test]
    #[allow(clippy::mutable_key_type)]
    fn native_order_hash_and_stable_hash_ignore_cache() {
        let first = Field::new("a", DataType::Int64, false);
        let second = Field::new("b", DataType::Int64, false);
        let mut ordered = BTreeSet::new();
        ordered.insert(second.clone());
        ordered.insert(first.clone());
        assert_eq!(ordered.into_iter().next().unwrap(), first);
        let mut hashed = HashSet::new();
        hashed.insert(second.clone());
        assert!(hashed.contains(&second));
        let before = second.stable_hash();
        second.clone().into_arrow_field_ref().unwrap();
        assert_eq!(before, second.stable_hash());
    }
}
