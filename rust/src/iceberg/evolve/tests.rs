//! Schema evolution operations and the metadata updates that carry them.

use smol_str::SmolStr;

use super::{SchemaUpdate, can_promote};
use crate::iceberg::{
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
        summary: vec![(
            SmolStr::new_static("operation"),
            SmolStr::new_static("append"),
        )],
        schema_id: Some(0),
        first_row_id: None,
        added_rows: None,
    }
}

/// The one-column ascending sort order every sort order test adds.
fn identity_order(order_id: i32) -> SortOrder {
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
    use super::{DataType, SchemaUpdate, metadata};

    #[test]
    fn an_added_top_level_column_is_numbered_above_the_last_column_id() {
        let metadata = metadata();
        let mut update = SchemaUpdate::for_metadata(&metadata).unwrap();
        update.add_column("", DataType::Int64.nullable_field("quantity"));
        let evolved = update.apply().unwrap();
        let added = evolved.get_field_by_name("quantity").unwrap();
        assert_eq!(added.parquet_field_id().unwrap(), Some(6));
        assert_eq!(evolved.field_len(), 4);
    }

    #[test]
    fn an_added_nested_struct_numbers_its_children_depth_first() {
        let metadata = metadata();
        let mut update = SchemaUpdate::for_metadata(&metadata).unwrap();
        update.add_column(
            "quote",
            DataType::from_fields([
                DataType::Int64.required_field("bid"),
                DataType::Int64.required_field("ask"),
            ])
            .unwrap()
            .nullable_field("depth"),
        );
        let evolved = update.apply().unwrap();
        let depth = evolved
            .get_field_by_name("quote")
            .unwrap()
            .get_field_by_name("depth")
            .unwrap();
        assert_eq!(depth.parquet_field_id().unwrap(), Some(6));
        assert_eq!(depth.fields()[0].parquet_field_id().unwrap(), Some(7));
        assert_eq!(depth.fields()[1].parquet_field_id().unwrap(), Some(8));
    }

    #[test]
    fn a_dropped_columns_identifier_is_never_reused_by_a_later_add() {
        let mut metadata = metadata();
        let mut update = SchemaUpdate::for_metadata(&metadata).unwrap();
        update.drop_column("id");
        // The added column even carries a stale identifier, which is discarded
        // rather than resurrected.
        update.add_column(
            "",
            DataType::Int64
                .required_field("trade_id")
                .with_parquet_field_id(1),
        );
        let evolved = update.apply().unwrap();
        assert!(evolved.get_field_by_name("id").is_none());
        assert_eq!(
            evolved
                .get_field_by_name("trade_id")
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
        let mut update = SchemaUpdate::for_metadata(&metadata).unwrap();
        update.rename_column("symbol", "ticker");
        update.rename_column("quote.price", "last_price");
        let evolved = update.apply().unwrap();
        assert_eq!(
            evolved
                .get_field_by_name("ticker")
                .unwrap()
                .parquet_field_id()
                .unwrap(),
            Some(2)
        );
        let renamed = evolved
            .get_field_by_name("quote")
            .unwrap()
            .get_field_by_name("last_price")
            .unwrap();
        assert_eq!(renamed.parquet_field_id().unwrap(), Some(4));
        assert!(evolved.get_field_by_name("symbol").is_none());
    }

    #[test]
    fn update_doc_writes_the_iceberg_doc_property() {
        let metadata = metadata();
        let mut update = SchemaUpdate::for_metadata(&metadata).unwrap();
        update.update_doc("id", "trade identifier");
        update.update_doc("quote.price", "closing price");
        let evolved = update.apply().unwrap();
        assert_eq!(
            evolved
                .get_field_by_name("id")
                .unwrap()
                .iceberg()
                .get("doc"),
            Some("trade identifier")
        );
        assert_eq!(
            evolved
                .get_field_by_name("quote")
                .unwrap()
                .get_field_by_name("price")
                .unwrap()
                .get_metadata("iceberg:doc"),
            Some("closing price")
        );
    }

    #[test]
    fn make_nullable_relaxes_a_required_column() {
        let metadata = metadata();
        let mut update = SchemaUpdate::for_metadata(&metadata).unwrap();
        update.make_nullable("id");
        update.make_nullable("quote.price");
        let evolved = update.apply().unwrap();
        assert!(evolved.get_field_by_name("id").unwrap().is_nullable());
        assert!(
            evolved
                .get_field_by_name("quote")
                .unwrap()
                .get_field_by_name("price")
                .unwrap()
                .is_nullable()
        );
    }

    #[test]
    fn update_type_applies_a_legal_promotion_at_any_depth() {
        let metadata = metadata();
        let mut update = SchemaUpdate::for_metadata(&metadata).unwrap();
        update.update_type("id", DataType::Int64);
        update.update_type("quote.price", DataType::Float64);
        let evolved = update.apply().unwrap();
        let id = evolved.get_field_by_name("id").unwrap();
        assert_eq!(id.data_type(), &DataType::Int64);
        assert_eq!(
            id.parquet_field_id().unwrap(),
            Some(1),
            "a promotion keeps the id"
        );
        assert_eq!(
            evolved
                .get_field_by_name("quote")
                .unwrap()
                .get_field_by_name("price")
                .unwrap()
                .data_type(),
            &DataType::Float64
        );
    }

    #[test]
    fn update_type_refuses_an_illegal_promotion_naming_both_sides() {
        let metadata = metadata();
        let mut update = SchemaUpdate::for_metadata(&metadata).unwrap();
        update.update_type("symbol", DataType::Int32);
        let message = update.apply().unwrap_err().to_string();
        assert!(message.contains("utf8"), "{message}");
        assert!(message.contains("int32"), "{message}");
        assert!(message.contains("symbol"), "{message}");
    }

    #[test]
    fn a_missing_path_names_the_segment_and_the_available_columns() {
        let metadata = metadata();
        let mut update = SchemaUpdate::for_metadata(&metadata).unwrap();
        update.drop_column("quote.mid");
        let message = update.apply().unwrap_err().to_string();
        assert!(message.contains("\"mid\""), "{message}");
        assert!(message.contains("price"), "{message}");
        assert!(message.contains("size"), "{message}");

        let mut update = SchemaUpdate::for_metadata(&metadata).unwrap();
        update.rename_column("book.price", "px");
        let message = update.apply().unwrap_err().to_string();
        assert!(message.contains("\"book\""), "{message}");
        assert!(message.contains("symbol"), "{message}");
    }

    #[test]
    fn descending_through_a_non_struct_column_is_refused_by_name() {
        let metadata = metadata();
        let mut update = SchemaUpdate::for_metadata(&metadata).unwrap();
        update.drop_column("symbol.inner");
        let message = update.apply().unwrap_err().to_string();
        assert!(message.contains("struct"), "{message}");
        assert!(message.contains("utf8"), "{message}");
    }

    #[test]
    fn operations_apply_in_call_order() {
        let metadata = metadata();
        let mut update = SchemaUpdate::for_metadata(&metadata).unwrap();
        update.rename_column("symbol", "ticker");
        // The second operation sees the first one's result.
        update.update_doc("ticker", "renamed first");
        let evolved = update.apply().unwrap();
        assert_eq!(
            evolved
                .get_field_by_name("ticker")
                .unwrap()
                .get_metadata("iceberg:doc"),
            Some("renamed first")
        );
    }

    #[test]
    fn evolving_twice_keeps_last_column_id_monotone_and_schema_ids_distinct() {
        let mut metadata = metadata();
        let mut update = SchemaUpdate::for_metadata(&metadata).unwrap();
        update.add_column("", DataType::Int64.nullable_field("first"));
        let first = metadata.add_schema(update.apply().unwrap()).unwrap();
        metadata.set_current_schema(first).unwrap();
        assert_eq!(metadata.last_column_id, 6);

        let mut update = SchemaUpdate::for_metadata(&metadata).unwrap();
        update.add_column("", DataType::Int64.nullable_field("second"));
        let second = metadata.add_schema(update.apply().unwrap()).unwrap();
        metadata.set_current_schema(second).unwrap();
        assert_eq!(metadata.last_column_id, 7);
        assert_ne!(first, second);
        assert_eq!(metadata.schemas.len(), 3, "every old schema is retained");
    }

    #[test]
    fn set_current_schema_of_an_unknown_id_is_refused() {
        let message = metadata().set_current_schema(9).unwrap_err().to_string();
        assert!(message.contains("9"), "{message}");
        assert!(message.contains("1 schemas"), "{message}");
    }
}

mod metadata_updates {
    use super::{
        DataType, FormatVersion, PartitionSpec, SchemaUpdate, SmolStr, SnapshotRef, TableMetadata,
        identity_order, metadata, snapshot,
    };

    #[test]
    fn properties_keep_insertion_order_and_round_trip_through_the_document() {
        let mut metadata = metadata();
        assert_eq!(metadata.set_property("owner", "kaiju").unwrap(), None);
        assert_eq!(metadata.set_property("commit.retry", "4").unwrap(), None);
        assert_eq!(
            metadata.set_property("owner", "mothra").unwrap(),
            Some(SmolStr::new_static("kaiju")),
            "a replaced value is returned"
        );
        assert_eq!(
            metadata.properties[0].0, "owner",
            "replacing keeps insertion order"
        );

        let document = metadata.to_json().unwrap();
        let mut read = TableMetadata::from_json(&document).unwrap();
        assert_eq!(read.property("owner"), Some("mothra"));
        assert_eq!(read.property("commit.retry"), Some("4"));
        assert_eq!(
            read.remove_property("owner"),
            Some(SmolStr::new_static("mothra"))
        );
        assert_eq!(read.property("owner"), None);
        assert_eq!(read.remove_property("owner"), None);
    }

    #[test]
    fn an_empty_property_key_is_refused() {
        let message = metadata().set_property("", "x").unwrap_err().to_string();
        assert!(message.contains("non-empty"), "{message}");
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

        let message = metadata
            .upgrade_format_version(FormatVersion::V2)
            .unwrap_err()
            .to_string();
        assert!(message.contains("at least 3"), "{message}");
        assert!(message.contains("got 2"), "{message}");
        assert_eq!(metadata.format_version, FormatVersion::V3);
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

        metadata.set_current_snapshot(snapshot(7, 100));
        metadata.set_current_snapshot(snapshot(9, 200));
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
        let removed = metadata.remove_snapshot_ref("main").unwrap();
        assert_eq!(removed.snapshot_id, 7);
        assert_eq!(metadata.current_snapshot_id, None);
        assert!(metadata.remove_snapshot_ref("main").is_none());
    }

    #[test]
    fn remove_snapshots_trims_the_log_and_protects_refs() {
        let mut metadata = metadata();
        metadata.set_current_snapshot(snapshot(1, 100));
        metadata.set_current_snapshot(snapshot(2, 200));

        let message = metadata.remove_snapshots(&[2]).unwrap_err().to_string();
        assert!(message.contains("current snapshot 2"), "{message}");

        metadata
            .set_snapshot_ref("audit", SnapshotRef::branch(1))
            .unwrap();
        let message = metadata.remove_snapshots(&[1]).unwrap_err().to_string();
        assert!(message.contains("audit"), "{message}");
        assert_eq!(metadata.snapshots.len(), 2, "an error removes nothing");

        assert!(metadata.remove_snapshot_ref("audit").is_some());
        metadata.remove_snapshots(&[1]).unwrap();
        assert_eq!(metadata.snapshots.len(), 1);
        assert_eq!(metadata.snapshot_log, vec![(200, 2)]);
    }

    #[test]
    fn add_spec_rejects_a_colliding_spec_id_and_keeps_last_partition_id_monotone() {
        let mut metadata = metadata();
        let spec =
            PartitionSpec::identity(1, metadata.current_schema().unwrap(), &["symbol"]).unwrap();
        assert_eq!(metadata.add_spec(spec.clone()).unwrap(), 1);
        assert_eq!(metadata.last_partition_id, 1000);

        let message = metadata.add_spec(spec).unwrap_err().to_string();
        assert!(message.contains("got 1"), "{message}");
        assert_eq!(metadata.partition_specs.len(), 2);

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
    }

    #[test]
    fn add_sort_order_rejects_a_colliding_order_id() {
        let mut metadata = metadata();
        assert_eq!(metadata.add_sort_order(identity_order(1)).unwrap(), 1);
        let message = metadata
            .add_sort_order(identity_order(1))
            .unwrap_err()
            .to_string();
        assert!(message.contains("got 1"), "{message}");

        metadata.set_default_sort_order(1).unwrap();
        assert_eq!(metadata.default_sort_order_id, 1);
        let message = metadata.set_default_sort_order(9).unwrap_err().to_string();
        assert!(message.contains("9"), "{message}");
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
            .set_data_type(DataType::from_fields(fields).unwrap())
            .unwrap();
        duplicated.schemas[0] = schema;
        let message = duplicated.validate().unwrap_err().to_string();
        assert!(message.contains("unique field ids"), "{message}");
        assert!(message.contains("got 1"), "{message}");

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
        metadata.set_location("file:///tmp/evolve-moved");
        metadata.upgrade_format_version(FormatVersion::V3).unwrap();
        metadata.set_current_snapshot(snapshot(3, 300));
        metadata
            .set_snapshot_ref("audit", SnapshotRef::branch(3))
            .unwrap();
        let spec =
            PartitionSpec::identity(1, metadata.current_schema().unwrap(), &["symbol"]).unwrap();
        metadata.add_spec(spec).unwrap();
        metadata.set_default_spec(1).unwrap();
        metadata.add_sort_order(identity_order(1)).unwrap();
        metadata.set_default_sort_order(1).unwrap();
        let mut update = SchemaUpdate::for_metadata(&metadata).unwrap();
        update.add_column("", DataType::Int64.nullable_field("quantity"));
        let schema_id = metadata.add_schema(update.apply().unwrap()).unwrap();
        metadata.set_current_schema(schema_id).unwrap();

        let document = metadata.to_json().unwrap();
        let read = TableMetadata::from_json(&document).unwrap();
        assert_eq!(read.to_json().unwrap(), document);
        assert_eq!(read.location, "file:///tmp/evolve-moved");
        assert_eq!(read.current_schema().unwrap().field_len(), 4);
        assert_eq!(read.default_spec_id, 1);
        assert_eq!(read.default_sort_order_id, 1);
        assert_eq!(read.next_row_id, Some(0));
    }
}
