//! Schema evolution operations and the metadata updates that carry them.

use smol_str::SmolStr;

use super::{SchemaUpdate, can_promote};
use crate::media::iceberg::{
    FormatVersion, PartitionSpec, Snapshot, SnapshotRef, SortField, SortOrder, TableMetadata,
    Transform, assign_field_ids,
};
use crate::{DataType, Field};

/// The nested quote schema every evolution test starts from.
///
/// Ids run 1..=5 depth first: `id`, `symbol`, `quote`, `quote.price`,
/// `quote.size`.
fn quote_schema() -> Field {
    let mut schema = DataType::from_fields([
        DataType::Int32.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
        DataType::from_fields([
            DataType::Float32.required_field("price"),
            DataType::Int32.nullable_field("size"),
        ])
        .unwrap()
        .nullable_field("quote"),
    ])
    .unwrap()
    .required_field("row");
    assign_field_ids(&mut schema, 1).unwrap();
    schema.insert_metadata("iceberg:schema-id", "0").unwrap();
    schema
}

/// A v2 table over [`quote_schema`], unpartitioned and never written to.
fn metadata() -> TableMetadata {
    TableMetadata::new(
        FormatVersion::V2,
        "file:///tmp/evolve",
        quote_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap()
}

/// One appended snapshot with the given identifier and commit time.
fn snapshot(snapshot_id: i64, timestamp_ms: i64) -> Snapshot {
    Snapshot {
        snapshot_id,
        parent_snapshot_id: None,
        sequence_number: Some(1),
        timestamp_ms,
        manifest_list: SmolStr::new_static("file:///tmp/evolve/metadata/snap.avro"),
        manifests: None,
        summary: vec![(
            SmolStr::new_static("operation"),
            SmolStr::new_static("append"),
        )],
        schema_id: Some(0),
        encryption_key_id: None,
        first_row_id: None,
        added_rows: None,
    }
}

/// The one-column ascending sort order every sort order test adds.
fn identity_order(order_id: i64) -> SortOrder {
    SortOrder {
        order_id,
        fields: vec![SortField {
            source_id: 1,
            transform: Transform::Identity,
            direction: SmolStr::new_static("asc"),
            null_order: SmolStr::new_static("nulls-first"),
        }],
    }
}

mod promotions {
    use super::{DataType, can_promote};

    #[test]
    fn every_iceberg_legal_promotion_passes() {
        can_promote(&DataType::Int32, &DataType::Int64).unwrap();
        can_promote(&DataType::Float32, &DataType::Float64).unwrap();
        // Precision widens at the same scale, across the physical widths.
        can_promote(
            &DataType::decimal64(10, 2).unwrap(),
            &DataType::decimal64(12, 2).unwrap(),
        )
        .unwrap();
        can_promote(
            &DataType::decimal32(5, 2).unwrap(),
            &DataType::decimal64(15, 2).unwrap(),
        )
        .unwrap();
        can_promote(
            &DataType::decimal64(10, 2).unwrap(),
            &DataType::decimal128(38, 2).unwrap(),
        )
        .unwrap();
        // Identical types always pass, whatever they are.
        can_promote(&DataType::Utf8, &DataType::Utf8).unwrap();
        can_promote(&DataType::Int64, &DataType::Int64).unwrap();
        can_promote(
            &DataType::decimal128(20, 4).unwrap(),
            &DataType::decimal128(20, 4).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn an_illegal_promotion_names_both_types() {
        for (from, to) in [
            (DataType::Int64, DataType::Int32),
            (DataType::Float64, DataType::Float32),
            (
                DataType::decimal64(10, 2).unwrap(),
                DataType::decimal64(10, 3).unwrap(),
            ),
            (
                DataType::decimal64(10, 2).unwrap(),
                DataType::decimal64(9, 2).unwrap(),
            ),
            (DataType::Utf8, DataType::Int32),
            (
                DataType::decimal128(38, 2).unwrap(),
                DataType::decimal256(40, 2).unwrap(),
            ),
        ] {
            let message = can_promote(&from, &to).unwrap_err().to_string();
            assert!(message.contains(&from.to_string()), "{message}");
            assert!(message.contains(&to.to_string()), "{message}");
        }
    }
}

mod schema_updates {
    use super::{DataType, FormatVersion, SchemaUpdate, metadata};

    #[test]
    fn an_added_top_level_column_is_numbered_above_the_last_column_id() {
        let metadata = metadata();
        let mut update = SchemaUpdate::from_metadata(&metadata).unwrap();
        update.add_column("", DataType::Int64.nullable_field("quantity"));
        let evolved = update.into_field().unwrap();
        let added = evolved.get_field_by_path("quantity").unwrap();
        assert_eq!(added.parquet_field_id().unwrap(), Some(6));
        assert_eq!(evolved.field_len(), 4);
    }

    #[test]
    fn an_added_nested_struct_numbers_its_children_depth_first() {
        let metadata = metadata();
        let mut update = SchemaUpdate::from_metadata(&metadata).unwrap();
        update.add_column(
            "quote",
            DataType::from_fields([
                DataType::Int64.required_field("bid"),
                DataType::Int64.required_field("ask"),
            ])
            .unwrap()
            .nullable_field("depth"),
        );
        let evolved = update.into_field().unwrap();
        let depth = evolved
            .get_field_by_path("quote")
            .unwrap()
            .get_field_by_path("depth")
            .unwrap();
        assert_eq!(depth.parquet_field_id().unwrap(), Some(6));
        assert_eq!(depth.fields()[0].parquet_field_id().unwrap(), Some(7));
        assert_eq!(depth.fields()[1].parquet_field_id().unwrap(), Some(8));
    }

    #[test]
    fn a_dropped_columns_identifier_is_never_reused_by_a_later_add() {
        let mut metadata = metadata();
        let mut update = SchemaUpdate::from_metadata(&metadata).unwrap();
        update.drop_column("id");
        // The added column even carries a stale identifier, which is discarded
        // rather than resurrected.
        update.add_column(
            "",
            DataType::Int64
                .nullable_field("trade_id")
                .with_parquet_field_id(1),
        );
        let evolved = update.into_field().unwrap();
        assert!(evolved.get_field_by_path("id").is_none());
        assert_eq!(
            evolved
                .get_field_by_path("trade_id")
                .unwrap()
                .parquet_field_id()
                .unwrap(),
            Some(6),
            "a retired id must never come back"
        );
        metadata.add_schema(evolved).unwrap();
        assert_eq!(metadata.last_column_id, 6);
    }

    #[test]
    fn renaming_a_column_keeps_its_identifier_at_any_depth() {
        let metadata = metadata();
        let mut update = SchemaUpdate::from_metadata(&metadata).unwrap();
        update.rename_column("symbol", "ticker");
        update.rename_column("quote.price", "last_price");
        let evolved = update.into_field().unwrap();
        assert_eq!(
            evolved
                .get_field_by_path("ticker")
                .unwrap()
                .parquet_field_id()
                .unwrap(),
            Some(2)
        );
        let renamed = evolved
            .get_field_by_path("quote")
            .unwrap()
            .get_field_by_path("last_price")
            .unwrap();
        assert_eq!(renamed.parquet_field_id().unwrap(), Some(4));
        assert!(evolved.get_field_by_path("symbol").is_none());
    }

    #[test]
    fn update_doc_writes_the_iceberg_doc_property() {
        let metadata = metadata();
        let mut update = SchemaUpdate::from_metadata(&metadata).unwrap();
        update.update_doc("id", "trade identifier");
        update.update_doc("quote.price", "closing price");
        let evolved = update.into_field().unwrap();
        assert_eq!(
            evolved.get_field_by_path("id").unwrap().as_iceberg().doc(),
            Some("trade identifier")
        );
        assert_eq!(
            evolved
                .get_field_by_path("quote")
                .unwrap()
                .get_field_by_path("price")
                .unwrap()
                .get_metadata("iceberg:doc"),
            Some("closing price")
        );
    }

    #[test]
    fn make_nullable_relaxes_a_required_column() {
        let metadata = metadata();
        let mut update = SchemaUpdate::from_metadata(&metadata).unwrap();
        update.make_nullable("id");
        update.make_nullable("quote.price");
        let evolved = update.into_field().unwrap();
        assert!(evolved.get_field_by_path("id").unwrap().is_nullable());
        assert!(
            evolved
                .get_field_by_path("quote")
                .unwrap()
                .get_field_by_path("price")
                .unwrap()
                .is_nullable()
        );
    }

    #[test]
    fn update_type_applies_a_legal_promotion_at_any_depth() {
        let metadata = metadata();
        let mut update = SchemaUpdate::from_metadata(&metadata).unwrap();
        update.update_type("id", DataType::Int64);
        update.update_type("quote.price", DataType::Float64);
        let evolved = update.into_field().unwrap();
        let id = evolved.get_field_by_path("id").unwrap();
        assert_eq!(id.dtype(), &DataType::Int64);
        assert_eq!(
            id.parquet_field_id().unwrap(),
            Some(1),
            "a promotion keeps the id"
        );
        assert_eq!(
            evolved
                .get_field_by_path("quote")
                .unwrap()
                .get_field_by_path("price")
                .unwrap()
                .dtype(),
            &DataType::Float64
        );
    }

    #[test]
    fn update_type_refuses_an_illegal_promotion_naming_both_sides() {
        let metadata = metadata();
        let mut update = SchemaUpdate::from_metadata(&metadata).unwrap();
        update.update_type("symbol", DataType::Int32);
        let message = update.into_field().unwrap_err().to_string();
        assert!(message.contains("utf8"), "{message}");
        assert!(message.contains("int32"), "{message}");
        assert!(message.contains("symbol"), "{message}");
    }

    #[test]
    fn a_missing_path_names_the_segment_and_the_available_columns() {
        let metadata = metadata();
        let mut update = SchemaUpdate::from_metadata(&metadata).unwrap();
        update.drop_column("quote.mid");
        let message = update.into_field().unwrap_err().to_string();
        assert!(message.contains("\"mid\""), "{message}");
        assert!(message.contains("price"), "{message}");
        assert!(message.contains("size"), "{message}");

        let mut update = SchemaUpdate::from_metadata(&metadata).unwrap();
        update.rename_column("book.price", "px");
        let message = update.into_field().unwrap_err().to_string();
        assert!(message.contains("\"book\""), "{message}");
        assert!(message.contains("symbol"), "{message}");
    }

    #[test]
    fn descending_through_a_non_struct_column_is_refused_by_name() {
        let metadata = metadata();
        let mut update = SchemaUpdate::from_metadata(&metadata).unwrap();
        update.drop_column("symbol.inner");
        let message = update.into_field().unwrap_err().to_string();
        assert!(message.contains("struct"), "{message}");
        assert!(message.contains("utf8"), "{message}");
    }

    #[test]
    fn operations_apply_in_call_order() {
        let metadata = metadata();
        let mut update = SchemaUpdate::from_metadata(&metadata).unwrap();
        update.rename_column("symbol", "ticker");
        // The second operation sees the first one's result.
        update.update_doc("ticker", "renamed first");
        let evolved = update.into_field().unwrap();
        assert_eq!(
            evolved
                .get_field_by_path("ticker")
                .unwrap()
                .get_metadata("iceberg:doc"),
            Some("renamed first")
        );
    }

    #[test]
    fn evolving_twice_keeps_last_column_id_monotone_and_schema_ids_distinct() {
        let mut metadata = metadata();
        let mut update = SchemaUpdate::from_metadata(&metadata).unwrap();
        update.add_column("", DataType::Int64.nullable_field("first"));
        let first = metadata.add_schema(update.into_field().unwrap()).unwrap();
        metadata.set_current_schema(first).unwrap();
        assert_eq!(metadata.last_column_id, 6);

        let mut update = SchemaUpdate::from_metadata(&metadata).unwrap();
        update.add_column("", DataType::Int64.nullable_field("second"));
        let second = metadata.add_schema(update.into_field().unwrap()).unwrap();
        metadata.set_current_schema(second).unwrap();
        assert_eq!(metadata.last_column_id, 7);
        assert_ne!(first, second);
        assert_eq!(metadata.schemas.len(), 3, "every old schema is retained");
    }

    #[test]
    fn schema_ids_come_from_the_official_result_and_overflow_is_atomic() {
        let mut metadata = metadata();
        metadata.schemas[0]
            .as_iceberg_mut()
            .set_schema_id(i32::MAX)
            .unwrap();
        metadata.current_schema_id = i32::MAX;

        let mut reusable = metadata.current_schema().unwrap().clone();
        reusable.as_iceberg_mut().set_schema_id(7).unwrap();
        assert_eq!(metadata.add_schema(reusable).unwrap(), i32::MAX);
        assert_eq!(metadata.schemas.len(), 1);

        let mut update = SchemaUpdate::from_metadata(&metadata).unwrap();
        update.add_column("", DataType::Int64.nullable_field("overflow"));
        let added = update.into_field().unwrap();
        let before = metadata.clone();
        let message = metadata.add_schema(added).unwrap_err().to_string();
        assert!(message.contains("schema id") && message.contains("i32::MAX"));
        assert_eq!(metadata, before, "schema-id overflow changes nothing");
    }

    #[test]
    fn direct_schema_add_refuses_retired_ids_and_illegal_type_changes() {
        let mut metadata = metadata();
        let mut incompatible = metadata.current_schema().unwrap().clone();
        let mut replacement = DataType::Int32.nullable_field("symbol");
        replacement.set_parquet_field_id(2);
        incompatible
            .set_field_by_path("symbol", replacement)
            .unwrap();
        let message = metadata.add_schema(incompatible).unwrap_err().to_string();
        assert!(message.contains("field id 2"), "{message}");
        assert!(
            message.contains("string") && message.contains("int"),
            "{message}"
        );

        let mut update = SchemaUpdate::from_metadata(&metadata).unwrap();
        update.drop_column("symbol");
        let schema_id = metadata.add_schema(update.into_field().unwrap()).unwrap();
        metadata.set_current_schema(schema_id).unwrap();

        let mut reused = metadata.current_schema().unwrap().clone();
        let mut replacement = DataType::Utf8.nullable_field("replacement");
        replacement.set_parquet_field_id(2);
        reused
            .set_field_by_path("replacement", replacement)
            .unwrap();
        let message = metadata.add_schema(reused).unwrap_err().to_string();
        assert!(message.contains("retired field id 2"), "{message}");
    }

    #[test]
    fn direct_schema_add_requires_and_preserves_initial_defaults() {
        let mut metadata = metadata();
        let mut required = metadata.current_schema().unwrap().clone();
        required
            .set_field_by_path("quantity", DataType::Int64.required_field("quantity"))
            .unwrap();
        let message = metadata.add_schema(required).unwrap_err().to_string();
        assert!(
            message.contains("initial-default") && message.contains("required"),
            "{message}"
        );

        metadata.upgrade_format_version(FormatVersion::V3).unwrap();
        let mut with_default = metadata.current_schema().unwrap().clone();
        let mut quantity = DataType::Int64.required_field("quantity");
        quantity
            .insert_metadata("iceberg:initial-default", "0")
            .unwrap();
        with_default
            .set_field_by_path("quantity", quantity)
            .unwrap();
        let schema_id = metadata.add_schema(with_default).unwrap();
        metadata.set_current_schema(schema_id).unwrap();

        let mut changed = metadata.current_schema().unwrap().clone();
        let mut quantity = changed.get_field_by_path("quantity").unwrap().clone();
        quantity
            .insert_metadata("iceberg:initial-default", "1")
            .unwrap();
        changed.set_field_by_path("quantity", quantity).unwrap();
        let message = metadata.add_schema(changed).unwrap_err().to_string();
        assert!(message.contains("immutable initial-default"), "{message}");
    }

    #[test]
    fn set_current_schema_of_an_unknown_id_is_refused() {
        let message = metadata().set_current_schema(9).unwrap_err().to_string();
        assert!(message.contains("9"), "{message}");
        assert!(message.contains("unknown schema"), "{message}");
    }
}

mod metadata_updates {
    use super::{
        DataType, FormatVersion, PartitionSpec, SchemaUpdate, SmolStr, SnapshotRef, TableMetadata,
        identity_order, metadata, quote_schema, snapshot,
    };
    use crate::Scalar;

    #[test]
    fn properties_are_canonical_and_round_trip_through_the_document() {
        let mut metadata = metadata();
        assert_eq!(metadata.set_property("owner", "kaiju").unwrap(), None);
        assert_eq!(metadata.set_property("commit.retry", "4").unwrap(), None);
        assert_eq!(
            metadata.set_property("owner", "mothra").unwrap(),
            Some(SmolStr::new_static("kaiju")),
            "a replaced value is returned"
        );
        assert_eq!(
            metadata
                .properties
                .iter()
                .map(|(key, _)| key.as_str())
                .collect::<Vec<_>>(),
            ["commit.retry", "owner"]
        );

        let document = metadata.into_json().unwrap();
        let mut read = TableMetadata::from_json(&document).unwrap();
        assert_eq!(read.property("owner"), Some("mothra"));
        assert_eq!(read.property("commit.retry"), Some("4"));
        assert_eq!(
            read.remove_property("owner").unwrap(),
            Some(SmolStr::new_static("mothra"))
        );
        assert_eq!(read.property("owner"), None);
        assert_eq!(read.remove_property("owner").unwrap(), None);
    }

    #[test]
    fn metadata_compression_consults_only_its_official_property() {
        use iceberg_official::compression::CompressionCodec;
        use iceberg_official::spec::TableProperties;

        let mut metadata = metadata();
        metadata
            .set_property("write.target-file-size-bytes", "not-an-integer")
            .unwrap();
        assert_eq!(
            metadata.metadata_compression_codec().unwrap(),
            CompressionCodec::None
        );

        metadata
            .set_property(TableProperties::PROPERTY_METADATA_COMPRESSION_CODEC, "GzIp")
            .unwrap();
        assert!(matches!(
            metadata.metadata_compression_codec().unwrap(),
            CompressionCodec::Gzip(_)
        ));

        metadata
            .set_property(TableProperties::PROPERTY_METADATA_COMPRESSION_CODEC, "zstd")
            .unwrap();
        let message = metadata
            .metadata_compression_codec()
            .unwrap_err()
            .to_string();
        assert!(
            message.contains("zstd") && message.contains("gzip"),
            "{message}"
        );
    }

    #[test]
    fn official_statistics_and_encryption_mutations_round_trip() {
        let mut metadata = metadata();
        metadata
            .set_current_snapshot(snapshot(3, metadata.last_updated_ms + 1))
            .unwrap();
        let statistics = crate::text::json::from_utf8(
            r#"{"snapshot-id":3,"statistics-path":"s3://a/stats.puffin",
                "file-size-in-bytes":413,"file-footer-size-in-bytes":42,
                "blob-metadata":[]}"#,
        )
        .unwrap();
        assert_eq!(metadata.set_statistics(statistics.clone()).unwrap(), None);
        assert_eq!(metadata.statistics(), std::slice::from_ref(&statistics));
        assert_eq!(metadata.remove_statistics(3).unwrap(), Some(statistics));
        assert!(metadata.statistics().is_empty());

        let partition_statistics = crate::text::json::from_utf8(
            r#"{"snapshot-id":3,"statistics-path":"s3://a/partition.parquet",
                "file-size-in-bytes":43}"#,
        )
        .unwrap();
        assert_eq!(
            metadata
                .set_partition_statistics(partition_statistics.clone())
                .unwrap(),
            None
        );
        assert_eq!(
            metadata.remove_partition_statistics(3).unwrap(),
            Some(partition_statistics)
        );

        metadata.upgrade_format_version(FormatVersion::V3).unwrap();
        let key = crate::text::json::from_utf8(
            r#"{"key-id":"key-1","encrypted-key-metadata":"aWNlYmVyZw==",
                "encrypted-by-id":"kms"}"#,
        )
        .unwrap();
        assert!(metadata.add_encryption_key(key.clone()).unwrap());
        assert!(!metadata.add_encryption_key(key.clone()).unwrap());
        assert_eq!(metadata.encryption_keys(), std::slice::from_ref(&key));
        assert_eq!(metadata.remove_encryption_key("key-1").unwrap(), Some(key));
        assert!(metadata.encryption_keys().is_empty());

        let mut v2 = super::metadata();
        let message = v2.add_encryption_key(Scalar::Null).unwrap_err().to_string();
        assert!(
            message.contains("v3") && message.contains("encryption"),
            "{message}"
        );
    }

    #[test]
    fn an_empty_property_key_is_refused() {
        let message = metadata().set_property("", "x").unwrap_err().to_string();
        assert!(message.contains("non-empty"), "{message}");
    }

    #[test]
    fn official_property_and_location_rules_are_atomic() {
        let mut metadata = metadata();
        let before = metadata.clone();
        let message = metadata
            .remove_property("format-version")
            .unwrap_err()
            .to_string();
        assert!(message.contains("reserved"), "{message}");
        assert_eq!(metadata, before);

        metadata.set_location("file:///tmp/evolve/").unwrap();
        assert_eq!(metadata.location, "file:///tmp/evolve");
    }

    #[test]
    fn assign_uuid_validates_the_hyphenated_hex_shape() {
        let mut metadata = metadata();
        metadata
            .assign_uuid("0b4f7721-755e-5df5-ab6b-8a23c7905a82")
            .unwrap();
        assert_eq!(metadata.table_uuid, "0b4f7721-755e-5df5-ab6b-8a23c7905a82");

        let message = metadata.assign_uuid("not-a-uuid").unwrap_err().to_string();
        assert!(message.contains("8-4-4-4-12"), "{message}");
        assert!(message.contains("not-a-uuid"), "{message}");
        assert_eq!(
            metadata.table_uuid, "0b4f7721-755e-5df5-ab6b-8a23c7905a82",
            "an error leaves the UUID unchanged"
        );
    }

    #[test]
    fn upgrading_to_v3_initializes_next_row_id_and_a_downgrade_is_refused() {
        let mut metadata = metadata();
        assert_eq!(metadata.next_row_id, None);
        metadata.upgrade_format_version(FormatVersion::V3).unwrap();
        assert_eq!(metadata.format_version, FormatVersion::V3);
        assert_eq!(metadata.next_row_id, Some(0));

        let mut missing = metadata.clone();
        missing.next_row_id = None;
        let message = missing.validate().unwrap_err().to_string();
        assert!(
            message.contains("next-row-id") && message.contains("none"),
            "{message}"
        );
        let mut negative = metadata.clone();
        negative.next_row_id = Some(-1);
        let message = negative.validate().unwrap_err().to_string();
        assert!(
            message.contains("next-row-id") && message.contains("-1"),
            "{message}"
        );

        let message = metadata
            .upgrade_format_version(FormatVersion::V2)
            .unwrap_err()
            .to_string();
        assert!(message.contains("Cannot downgrade"), "{message}");
        assert!(message.contains("v3"), "{message}");
        assert!(message.contains("v2"), "{message}");
        assert_eq!(metadata.format_version, FormatVersion::V3);
    }

    #[test]
    fn versioned_snapshot_fields_and_refs_never_disappear_silently() {
        let mut v1 = TableMetadata::new(
            FormatVersion::V1,
            "file:///tmp/evolve-v1",
            quote_schema(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let mut first = snapshot(7, v1.last_updated_ms + 1);
        first.sequence_number = None;
        v1.set_current_snapshot(first).unwrap();
        let message = v1
            .set_snapshot_ref("audit", SnapshotRef::tag(7))
            .unwrap_err()
            .to_string();
        assert!(
            message.contains("v2 or v3") && message.contains("refs"),
            "{message}"
        );
        assert!(v1.into_json().unwrap().get_key_str("refs").is_none());

        let mut v2 = metadata();
        let mut missing_sequence = snapshot(8, v2.last_updated_ms + 1);
        missing_sequence.sequence_number = None;
        let message = v2
            .set_current_snapshot(missing_sequence)
            .unwrap_err()
            .to_string();
        assert!(
            message.contains("sequence-number") && message.contains("v2"),
            "{message}"
        );

        let mut encrypted = snapshot(9, v2.last_updated_ms + 1);
        encrypted.encryption_key_id = Some(SmolStr::new_static("key-1"));
        let message = v2.set_current_snapshot(encrypted).unwrap_err().to_string();
        assert!(
            message.contains("key-id") && message.contains("v2"),
            "{message}"
        );

        let mut dangling = snapshot(10, v2.last_updated_ms + 1);
        dangling.schema_id = Some(999);
        let message = v2.set_current_snapshot(dangling).unwrap_err().to_string();
        assert!(
            message.contains("schema id 999") && message.contains("snapshot 10"),
            "{message}"
        );
    }

    #[test]
    fn a_snapshot_ref_needs_a_retained_snapshot_and_main_follows_the_current_one() {
        let mut metadata = metadata();
        let message = metadata
            .set_snapshot_ref("audit", SnapshotRef::branch(7))
            .unwrap_err()
            .to_string();
        assert!(message.contains("audit"), "{message}");
        assert!(message.contains("7"), "{message}");
        assert!(metadata.refs.is_empty(), "an error records nothing");

        let timestamp = metadata.last_updated_ms + 60_000;
        metadata
            .set_current_snapshot(snapshot(7, timestamp))
            .unwrap();
        metadata
            .set_current_snapshot(snapshot(9, timestamp + 1))
            .unwrap();
        metadata
            .set_snapshot_ref("audit", SnapshotRef::branch(7))
            .unwrap();
        assert_eq!(metadata.refs.len(), 2, "main plus the new branch");

        // Pointing main elsewhere moves the current snapshot with it.
        metadata
            .set_snapshot_ref("main", SnapshotRef::branch(7))
            .unwrap();
        assert_eq!(metadata.current_snapshot_id, Some(7));
        assert_eq!(metadata.snapshot_log.last().map(|(_, id)| *id), Some(7));

        // And removing main clears it.
        let removed = metadata.remove_snapshot_ref("main").unwrap().unwrap();
        assert_eq!(removed.snapshot_id, 7);
        assert_eq!(metadata.current_snapshot_id, None);
        assert!(metadata.remove_snapshot_ref("main").unwrap().is_none());
    }

    #[test]
    fn explicit_expiration_protects_heads_and_trims_metadata() {
        let mut metadata = metadata();
        let timestamp = metadata.last_updated_ms + 60_000;
        metadata
            .set_current_snapshot(snapshot(1, timestamp))
            .unwrap();
        metadata
            .set_current_snapshot(snapshot(2, timestamp + 1))
            .unwrap();

        let message = metadata
            .expire_snapshots(Some(0), None, &[2])
            .unwrap_err()
            .to_string();
        assert!(message.contains("current snapshot"), "{message}");

        metadata
            .set_snapshot_ref("audit", SnapshotRef::branch(1))
            .unwrap();
        let message = metadata
            .expire_snapshots(Some(0), None, &[1])
            .unwrap_err()
            .to_string();
        assert!(
            message.contains("audit") && message.contains('1'),
            "{message}"
        );
        metadata.remove_snapshot_ref("audit").unwrap();
        metadata
            .set_statistics(
                crate::text::json::from_utf8(
                    r#"{"snapshot-id":1,"statistics-path":"s3://a/1.puffin",
                        "file-size-in-bytes":10,"file-footer-size-in-bytes":1,
                        "blob-metadata":[]}"#,
                )
                .unwrap(),
            )
            .unwrap();
        metadata
            .set_partition_statistics(
                crate::text::json::from_utf8(
                    r#"{"snapshot-id":1,"statistics-path":"s3://a/1.parquet",
                        "file-size-in-bytes":10}"#,
                )
                .unwrap(),
            )
            .unwrap();
        assert_eq!(
            metadata.expire_snapshots(Some(0), None, &[1]).unwrap(),
            vec![1]
        );
        assert!(metadata.ref_by_name("audit").is_none());
        assert_eq!(metadata.snapshots.len(), 1);
        assert_eq!(metadata.snapshot_log, vec![(timestamp + 1, 2)]);
        assert!(metadata.statistics().is_empty());
        assert!(metadata.partition_statistics().is_empty());
    }

    #[test]
    fn add_spec_reuses_equivalent_specs_and_renumbers_new_ones() {
        let mut metadata = metadata();
        let spec =
            PartitionSpec::identity(1, metadata.current_schema().unwrap(), &["symbol"]).unwrap();
        assert_eq!(metadata.add_spec(spec.clone()).unwrap(), 1);
        assert_eq!(metadata.last_partition_id, 1000);

        let mut different_ids = spec.clone();
        different_ids.spec_id = 99;
        different_ids.fields[0].field_id = 77_777;
        assert_eq!(metadata.add_spec(different_ids).unwrap(), 1);
        assert_eq!(metadata.partition_specs.len(), 2);

        let mut different = spec;
        different.fields[0].name = "symbol_bucket".into();
        different.fields[0].transform = super::Transform::Bucket(8);
        assert_eq!(metadata.add_spec(different).unwrap(), 2);
        assert_eq!(metadata.last_partition_id, 1001);

        metadata.set_default_spec(1).unwrap();
        assert_eq!(metadata.default_spec_id, 1);
        assert_eq!(
            metadata
                .current_schema()
                .unwrap()
                .partition_field_names()
                .collect::<Vec<_>>(),
            ["symbol"],
            "the schema is re-marked with the new layout"
        );
        let message = metadata.set_default_spec(9).unwrap_err().to_string();
        assert!(message.contains("9"), "{message}");

        let mut exhausted = super::metadata();
        exhausted.partition_specs[0].spec_id = i32::MAX;
        exhausted.default_spec_id = i32::MAX;
        assert_eq!(
            exhausted.add_spec(PartitionSpec::unpartitioned()).unwrap(),
            i32::MAX,
            "a compatible spec reuses the official maximum id"
        );
        let before = exhausted.clone();
        let spec =
            PartitionSpec::identity(0, exhausted.current_schema().unwrap(), &["symbol"]).unwrap();
        let message = exhausted.add_spec(spec).unwrap_err().to_string();
        assert!(
            message.contains("partition spec id") && message.contains(&i32::MAX.to_string()),
            "{message}"
        );
        assert_eq!(exhausted, before, "spec-id overflow changes nothing");
    }

    #[test]
    fn add_sort_order_reuses_equivalent_orders_and_renumbers_new_ones() {
        let mut metadata = metadata();
        assert_eq!(metadata.add_sort_order(identity_order(1)).unwrap(), 1);
        assert_eq!(metadata.add_sort_order(identity_order(1)).unwrap(), 1);

        let mut different = identity_order(1);
        different.fields[0].direction = "desc".into();
        assert_eq!(metadata.add_sort_order(different).unwrap(), 2);

        metadata.set_default_sort_order(1).unwrap();
        assert_eq!(metadata.default_sort_order_id, 1);
        let message = metadata.set_default_sort_order(9).unwrap_err().to_string();
        assert!(message.contains("9"), "{message}");
    }

    #[test]
    fn add_sort_order_preflights_the_official_identifier_increment() {
        let mut metadata = metadata();
        metadata.sort_orders.push(identity_order(i64::MAX));

        assert_eq!(
            metadata.add_sort_order(identity_order(1)).unwrap(),
            i64::MAX,
            "an equivalent order reuses the assigned maximum"
        );
        let before = metadata.clone();
        let mut different = identity_order(1);
        different.fields[0].direction = "desc".into();
        let message = metadata.add_sort_order(different).unwrap_err().to_string();
        assert!(
            message.contains("64-bit") && message.contains("i64::MAX"),
            "{message}"
        );
        assert_eq!(metadata, before, "overflow changes nothing");
    }

    #[test]
    fn validate_catches_a_duplicate_field_id_and_a_stale_last_column_id() {
        // The bad states are built by mutating the public fields directly,
        // because every method refuses to produce them.
        let mut duplicated = metadata();
        let mut schema = duplicated.schemas[0].clone();
        let mut fields = schema.fields().to_vec();
        fields[1].set_parquet_field_id(1);
        schema
            .set_dtype(DataType::from_fields(fields).unwrap())
            .unwrap();
        duplicated.schemas[0] = schema;
        let message = duplicated.validate().unwrap_err().to_string();
        assert!(message.to_lowercase().contains("duplicate"), "{message}");
        assert!(
            message.contains("field") && message.contains('1'),
            "{message}"
        );

        let mut stale = metadata();
        stale.last_column_id = 2;
        let message = stale.validate().unwrap_err().to_string();
        assert!(message.contains("at least 5"), "{message}");
        assert!(message.contains("got 2"), "{message}");
    }

    #[test]
    fn a_batch_of_updates_round_trips_through_its_document() {
        let mut metadata = metadata();
        metadata.set_property("owner", "kaiju").unwrap();
        metadata
            .assign_uuid("0b4f7721-755e-5df5-ab6b-8a23c7905a82")
            .unwrap();
        metadata.set_location("file:///tmp/evolve-moved").unwrap();
        metadata.upgrade_format_version(FormatVersion::V3).unwrap();
        let timestamp = metadata.last_updated_ms + 60_000;
        let mut snapshot = snapshot(3, timestamp);
        snapshot.first_row_id = metadata.next_row_id;
        snapshot.added_rows = Some(0);
        metadata.set_current_snapshot(snapshot).unwrap();
        metadata
            .set_snapshot_ref("audit", SnapshotRef::branch(3))
            .unwrap();
        let spec =
            PartitionSpec::identity(1, metadata.current_schema().unwrap(), &["symbol"]).unwrap();
        metadata.add_spec(spec).unwrap();
        metadata.set_default_spec(1).unwrap();
        metadata.add_sort_order(identity_order(1)).unwrap();
        metadata.set_default_sort_order(1).unwrap();
        let mut update = SchemaUpdate::from_metadata(&metadata).unwrap();
        update.add_column("", DataType::Int64.nullable_field("quantity"));
        let schema_id = metadata.add_schema(update.into_field().unwrap()).unwrap();
        metadata.set_current_schema(schema_id).unwrap();

        let document = metadata.into_json().unwrap();
        let read = TableMetadata::from_json(&document).unwrap();
        assert_eq!(read.clone().into_json().unwrap(), document);
        assert_eq!(read.location, "file:///tmp/evolve-moved");
        assert_eq!(read.current_schema().unwrap().field_len(), 4);
        assert_eq!(read.default_spec_id, 1);
        assert_eq!(read.default_sort_order_id, 1);
        assert_eq!(read.next_row_id, Some(0));
    }
}
