//! Iceberg schemas, metadata, manifests, partitions, and whole tables.

use std::hash::Hash;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use arrow_array::{Array, ArrayRef, Int64Array, RecordBatch, StringArray};

use crate::io::IOBase;
use crate::local::Folder;
use crate::{DataType, Field, Scalar};

use super::{
    CommitConflict, Compaction, DataFile, FieldSummary, FormatVersion, IcebergOptions,
    ManifestEntry, ManifestFile, PartitionField, PartitionSpec, ScanPlan, ScanTask, Snapshot,
    SnapshotRef, SortField, SortOrder, Table, TableMetadata, Transform, assign_field_ids,
    schema_from_json, schema_into_json,
};

#[test]
fn immutable_reports_and_metadata_have_complete_value_traits() {
    fn assert_traits<T: Clone + Eq + Ord + Hash>() {}
    assert_traits::<Compaction>();
    assert_traits::<CommitConflict>();
    assert_traits::<PartitionField>();
    assert_traits::<PartitionSpec>();
    assert_traits::<Snapshot>();
    assert_traits::<SnapshotRef>();
    assert_traits::<DataFile>();
    assert_traits::<FieldSummary>();
    assert_traits::<ManifestFile>();
    assert_traits::<SortField>();
    assert_traits::<SortOrder>();
    assert_traits::<ScanTask>();
    assert_traits::<ScanPlan>();
    assert_traits::<IcebergOptions>();
    assert_traits::<TableMetadata>();

    let partition = PartitionField::identity(1, 1_000, "day");
    let spec = PartitionSpec {
        spec_id: 3,
        fields: vec![partition.clone()],
    };
    assert_eq!(partition.stable_hash(), partition.clone().stable_hash());
    assert_eq!(spec.stable_hash(), spec.clone().stable_hash());

    let snapshot = Snapshot {
        snapshot_id: 10,
        parent_snapshot_id: Some(9),
        sequence_number: Some(2),
        timestamp_ms: 123,
        manifest_list: "metadata/snap.avro".into(),
        manifests: None,
        summary: vec![
            ("operation".into(), "append".into()),
            ("added-records".into(), "4".into()),
        ],
        schema_id: Some(1),
        encryption_key_id: None,
        first_row_id: None,
        added_rows: Some(4),
    };
    let mut equivalent_snapshot = snapshot.clone();
    equivalent_snapshot.summary.reverse();
    assert_eq!(snapshot, equivalent_snapshot);
    assert_eq!(snapshot.stable_hash(), equivalent_snapshot.stable_hash());
    assert_eq!(
        SnapshotRef::branch(10).stable_hash(),
        SnapshotRef::branch(10).stable_hash()
    );

    let data = DataFile {
        file_path: "data/part.parquet".into(),
        partition: vec![Scalar::I32(2024)],
        record_count: 4,
        file_size_in_bytes: 128,
        value_counts: vec![(2, 4), (1, 4)],
        key_metadata: Some(vec![1, 2]),
        equality_ids: Some(vec![1]),
        first_row_id: Some(8),
        referenced_data_file: Some("data/base.parquet".into()),
        content_offset: Some(32),
        content_size_in_bytes: Some(64),
        ..DataFile::default()
    };
    let summary = FieldSummary {
        lower_bound: Some(vec![1]),
        upper_bound: Some(vec![9]),
        ..FieldSummary::default()
    };
    let manifest = ManifestFile {
        manifest_path: "metadata/manifest.avro".into(),
        manifest_length: 256,
        partition_spec_id: 3,
        content: super::ManifestContent::Data,
        sequence_number: 2,
        min_sequence_number: 1,
        added_snapshot_id: 10,
        added_files_count: Some(1),
        existing_files_count: Some(0),
        deleted_files_count: Some(0),
        added_rows_count: Some(4),
        existing_rows_count: Some(0),
        deleted_rows_count: Some(0),
        partitions: vec![summary.clone()],
        key_metadata: Some(vec![3, 1, 4]),
        first_row_id: None,
    };
    let mut equivalent_data = data.clone();
    equivalent_data.value_counts.reverse();
    assert_eq!(data, equivalent_data);
    assert_eq!(data.stable_hash(), equivalent_data.stable_hash());
    assert_eq!(summary.stable_hash(), summary.clone().stable_hash());
    assert_eq!(manifest.stable_hash(), manifest.clone().stable_hash());

    let task = ScanTask {
        entry: ManifestEntry::added(10, data.clone()),
        spec: spec.clone(),
        residual: vec![0, 2],
    };
    let plan = ScanPlan {
        tasks: vec![task.clone()],
        excluded: Vec::new(),
        skipped: vec![manifest.clone()],
        manifests_read: 1,
    };
    assert_eq!(task.stable_hash(), task.clone().stable_hash());
    assert_eq!(plan.stable_hash(), plan.clone().stable_hash());

    let mut changed_entry = task.clone();
    changed_entry.entry.snapshot_id = Some(11);
    let mut changed_spec = task.clone();
    changed_spec.spec.spec_id += 1;
    let mut changed_residual = task.clone();
    changed_residual.residual.push(3);
    for changed in [changed_entry, changed_spec, changed_residual] {
        assert_ne!(task, changed);
        assert_ne!(task.stable_hash(), changed.stable_hash());
    }

    let mut changed_tasks = plan.clone();
    changed_tasks.tasks.push(task.clone());
    let mut changed_excluded = plan.clone();
    changed_excluded.excluded.push(task.clone());
    let mut changed_skipped = plan.clone();
    changed_skipped.skipped.push(manifest.clone());
    let mut changed_count = plan.clone();
    changed_count.manifests_read += 1;
    for changed in [
        changed_tasks,
        changed_excluded,
        changed_skipped,
        changed_count,
    ] {
        assert_ne!(plan, changed);
        assert_ne!(plan.stable_hash(), changed.stable_hash());
    }

    let compacted = Compaction {
        files_before: 4,
        files_after: 1,
        bytes_rewritten: 1_024,
    };
    assert_eq!(compacted.stable_hash(), compacted.stable_hash());
    let conflict = CommitConflict {
        expected_version: 1,
        beaten: 2,
        last_seen_version: 3,
    };
    assert_eq!(conflict.stable_hash(), conflict.stable_hash());

    let sort = SortField {
        source_id: 1,
        transform: Transform::Identity,
        direction: "asc".into(),
        null_order: "nulls-first".into(),
    };
    let order = SortOrder {
        order_id: 1,
        fields: vec![sort.clone()],
    };
    assert_eq!(sort.stable_hash(), sort.clone().stable_hash());
    assert_eq!(order.stable_hash(), order.clone().stable_hash());

    let mut options = IcebergOptions::new()
        .with_commit_retries(2)
        .with_commit_min_backoff_ms(3)
        .with_commit_max_backoff_ms(4)
        .with_commit_total_timeout_ms(9)
        .with_compact_after_commits(0)
        .with_read_parallel_min_files(6)
        .with_read_parallel_min_file_size_bytes(7)
        .try_with_data_mime_type(crate::MimeType::AVRO)
        .unwrap();
    options.set_target_file_size_bytes(5).unwrap();
    options.set_read_parallelism(8).unwrap();
    assert_eq!(options.stable_hash(), options.clone().stable_hash());
    assert_eq!(options.commit_retries_option(), Some(2));
    assert_eq!(options.commit_min_backoff_ms_option(), Some(3));
    assert_eq!(options.commit_max_backoff_ms_option(), Some(4));
    assert_eq!(options.commit_total_timeout_ms_option(), Some(9));
    assert_eq!(options.compact_after_commits_option(), Some(0));
    assert_eq!(options.target_file_size_bytes_option(), Some(5));
    assert_eq!(options.read_parallel_min_files_option(), Some(6));
    assert_eq!(options.read_parallel_min_file_size_bytes_option(), Some(7));
    assert_eq!(options.read_parallelism_option(), Some(8));
    assert_eq!(
        options.data_mime_type_option(),
        Some(&crate::MimeType::AVRO)
    );
    assert_eq!(IcebergOptions::new().commit_retries_option(), None);
}

#[test]
fn scan_plan_record_count_reports_invalid_manifest_totals() {
    let task = ScanTask {
        entry: ManifestEntry::added(
            1,
            DataFile {
                record_count: i64::MAX,
                ..DataFile::default()
            },
        ),
        spec: PartitionSpec::unpartitioned(),
        residual: Vec::new(),
    };
    let overflowing = ScanPlan {
        tasks: vec![task.clone(), task.clone()],
        ..ScanPlan::default()
    };
    let error = overflowing.record_count().unwrap_err();
    assert!(matches!(
        error,
        crate::Error::InvalidRecord { path, reason }
            if path == "$.tasks[1].data_file.record_count"
                && reason.contains("does not fit in i64")
    ));

    let mut negative = task;
    negative.entry.data_file.record_count = -1;
    let error = ScanPlan {
        tasks: vec![negative],
        ..ScanPlan::default()
    }
    .record_count()
    .unwrap_err();
    assert!(matches!(
        error,
        crate::Error::InvalidRecord { path, reason }
            if path == "$.tasks[0].data_file.record_count"
                && reason.contains("non-negative")
    ));
}

#[test]
fn table_metadata_state_is_read_only_through_complete_accessors() {
    let metadata = TableMetadata::new(
        FormatVersion::V2,
        "file:///table",
        trade_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();

    assert_eq!(metadata.format_version(), metadata.format_version);
    assert_eq!(metadata.table_uuid(), metadata.table_uuid.as_str());
    assert_eq!(metadata.location(), metadata.location.as_str());
    assert_eq!(
        metadata.last_sequence_number(),
        metadata.last_sequence_number
    );
    assert_eq!(metadata.last_updated_ms(), metadata.last_updated_ms);
    assert_eq!(metadata.last_column_id(), metadata.last_column_id);
    assert_eq!(metadata.schemas(), metadata.schemas.as_slice());
    assert_eq!(metadata.current_schema_id(), metadata.current_schema_id);
    assert_eq!(
        metadata.partition_specs(),
        metadata.partition_specs.as_slice()
    );
    assert_eq!(metadata.default_spec_id(), metadata.default_spec_id);
    assert_eq!(metadata.last_partition_id(), metadata.last_partition_id);
    assert_eq!(metadata.sort_orders(), metadata.sort_orders.as_slice());
    assert_eq!(
        metadata.default_sort_order_id(),
        metadata.default_sort_order_id
    );
    assert_eq!(metadata.properties(), metadata.properties.as_slice());
    assert_eq!(metadata.current_snapshot_id(), metadata.current_snapshot_id);
    assert_eq!(metadata.snapshots(), metadata.snapshots.as_slice());
    assert_eq!(metadata.snapshot_log(), metadata.snapshot_log.as_slice());
    assert_eq!(metadata.metadata_log(), metadata.metadata_log.as_slice());
    assert_eq!(metadata.refs(), metadata.refs.as_slice());
    assert_eq!(metadata.next_row_id(), metadata.next_row_id);
}

/// An Arrow filesystem that publishes one version hint and then reports that
/// same write as failed.
///
/// Object stores can acknowledge a whole-value replacement late: by the time
/// the transport error reaches the caller, a fresh handle may already see the
/// new metadata document and hint. This fixture makes that state deterministic
/// without teaching the table about a test-only storage hook.
#[derive(Debug, Default)]
struct PublishedHintFailure {
    inner: crate::arrowfs::MemoryFileSystem,
    fail_next_hint: AtomicBool,
}

impl PublishedHintFailure {
    fn arm(&self) {
        self.fail_next_hint.store(true, Ordering::Relaxed);
    }
}

impl crate::arrowfs::ArrowFileSystem for PublishedHintFailure {
    fn type_name(&self) -> &str {
        self.inner.type_name()
    }

    fn file_info(&self, path: &str) -> crate::Result<crate::arrowfs::FileInfo> {
        self.inner.file_info(path)
    }

    fn list(&self, path: &str, recursive: bool) -> crate::arrowfs::FileInfos {
        self.inner.list(path, recursive)
    }

    fn read_range(&self, path: &str, offset: u64, buffer: &mut [u8]) -> crate::Result<usize> {
        self.inner.read_range(path, offset, buffer)
    }

    fn write_full(&self, path: &str, bytes: &[u8]) -> crate::Result<()> {
        self.inner.write_full(path, bytes)?;
        if path.ends_with("/version-hint.text")
            && self.fail_next_hint.swap(false, Ordering::Relaxed)
        {
            return Err(crate::Error::Io(std::io::Error::other(
                "injected acknowledgement failure after publishing the version hint",
            )));
        }
        Ok(())
    }

    fn create_dir(&self, path: &str) -> crate::Result<()> {
        self.inner.create_dir(path)
    }

    fn delete_file(&self, path: &str) -> crate::Result<()> {
        self.inner.delete_file(path)
    }
}

/// A memory filesystem that lands one competing metadata document after a
/// writer's version preflight but before its same-version collision listing.
///
/// The injected document is the attempted document with the loser's property
/// replaced by the winner's. This isolates the race inside `commit_metadata`:
/// the writer already passed `find_metadata`, yet its publication finds a
/// valid competing document and must return through the retry gate.
#[derive(Debug, Default)]
struct SameVersionWinner {
    inner: crate::arrowfs::MemoryFileSystem,
    armed: AtomicBool,
    injections: AtomicUsize,
}

impl SameVersionWinner {
    fn arm(&self) {
        self.armed.store(true, Ordering::Relaxed);
    }

    fn injections(&self) -> usize {
        self.injections.load(Ordering::Relaxed)
    }

    fn invalid(reason: &'static str) -> crate::Error {
        crate::Error::Codec {
            format: "iceberg",
            position: 0,
            reason: reason.into(),
        }
    }
}

impl crate::arrowfs::ArrowFileSystem for SameVersionWinner {
    fn type_name(&self) -> &str {
        self.inner.type_name()
    }

    fn file_info(&self, path: &str) -> crate::Result<crate::arrowfs::FileInfo> {
        self.inner.file_info(path)
    }

    fn list(&self, path: &str, recursive: bool) -> crate::arrowfs::FileInfos {
        self.inner.list(path, recursive)
    }

    fn read_range(&self, path: &str, offset: u64, buffer: &mut [u8]) -> crate::Result<usize> {
        self.inner.read_range(path, offset, buffer)
    }

    fn write_full(&self, path: &str, bytes: &[u8]) -> crate::Result<()> {
        self.inner.write_full(path, bytes)?;
        if !path.contains("/metadata/00002-")
            || !path.ends_with(".metadata.json")
            || !self.armed.swap(false, Ordering::Relaxed)
        {
            return Ok(());
        }

        let mut document: serde_json::Value = serde_json::from_slice(bytes)?;
        let properties = document
            .as_object_mut()
            .and_then(|metadata| metadata.get_mut("properties"))
            .and_then(serde_json::Value::as_object_mut)
            .ok_or_else(|| Self::invalid("expected generated metadata properties"))?;
        if properties.remove("loser").is_none() {
            return Err(Self::invalid(
                "expected the attempted metadata to carry the loser's intent",
            ));
        }
        properties.insert(
            "winner".to_owned(),
            serde_json::Value::String("visible".to_owned()),
        );

        let (directory, candidate) = path
            .rsplit_once('/')
            .ok_or_else(|| Self::invalid("expected a metadata directory"))?;
        let first = "00002-00000000-0000-0000-0000-000000000000.metadata.json";
        let second = "00002-ffffffff-ffff-ffff-ffff-ffffffffffff.metadata.json";
        let competitor = if candidate == first { second } else { first };
        self.inner.write_full(
            &format!("{directory}/{competitor}"),
            &serde_json::to_vec(&document)?,
        )?;
        self.inner
            .write_full(&format!("{directory}/version-hint.text"), b"2")?;
        self.injections.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn create_dir(&self, path: &str) -> crate::Result<()> {
        self.inner.create_dir(path)
    }

    fn delete_file(&self, path: &str) -> crate::Result<()> {
        self.inner.delete_file(path)
    }
}

/// Build a scratch directory unique to this test and this process.
fn root(label: &str) -> std::path::PathBuf {
    let mut path = Folder::temporary().unwrap().path().unwrap();
    path.push(format!("yggdryl-iceberg-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    path
}

/// The three-column trade schema every table test writes.
fn trade_schema() -> Field {
    let mut schema = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
        DataType::Utf8.nullable_field("venue"),
    ])
    .unwrap()
    .required_field("row");
    assign_field_ids(&mut schema, 1).unwrap();
    schema.insert_metadata("iceberg:schema-id", "0").unwrap();
    schema
}

/// Build one batch of trades against [`trade_schema`].
fn trades(ids: &[i64], symbols: &[Option<&str>], venues: &[Option<&str>]) -> RecordBatch {
    let schema = trade_schema().into_arrow_schema().unwrap();
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Int64Array::from(ids.to_vec())),
            Arc::new(StringArray::from(symbols.to_vec())),
            Arc::new(StringArray::from(venues.to_vec())),
        ],
    )
    .unwrap()
}

/// Collect every row of a scan as `(id, symbol, venue)` triples.
fn collect(reader: crate::arrow::BatchReader) -> Vec<(i64, Option<String>, Option<String>)> {
    let mut rows = Vec::new();
    for batch in reader {
        let batch = batch.unwrap();
        let ids = batch
            .column_by_name("id")
            .unwrap()
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap()
            .clone();
        let symbols = batch.column_by_name("symbol").map(|column| {
            column
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap()
                .clone()
        });
        let venues = batch.column_by_name("venue").map(|column| {
            column
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap()
                .clone()
        });
        for row in 0..batch.num_rows() {
            rows.push((
                ids.value(row),
                symbols
                    .as_ref()
                    .filter(|column| !column.is_null(row))
                    .map(|column| column.value(row).to_owned()),
                venues
                    .as_ref()
                    .filter(|column| !column.is_null(row))
                    .map(|column| column.value(row).to_owned()),
            ));
        }
    }
    rows.sort();
    rows
}

mod schema_documents {
    use super::{Scalar, assign_field_ids, schema_from_json, schema_into_json};
    use crate::DataType;

    #[test]
    fn a_nested_schema_round_trips_through_json() {
        let document = crate::json::from_utf8(
            r#"{
                "type": "struct",
                "schema-id": 3,
                "fields": [
                    {"id": 1, "name": "id", "required": true, "type": "long"},
                    {"id": 2, "name": "symbol", "required": false, "type": "string",
                     "doc": "ticker"},
                    {"id": 3, "name": "legs", "required": false, "type": {
                        "type": "list", "element-id": 4, "element": {
                            "type": "struct", "fields": [
                                {"id": 5, "name": "price", "required": true,
                                 "type": "decimal(18, 4)"}
                            ]
                        }, "element-required": true
                    }},
                    {"id": 6, "name": "tags", "required": false, "type": {
                        "type": "map", "key-id": 7, "key": "string",
                        "value-id": 8, "value": "int", "value-required": false
                    }}
                ]
            }"#,
        )
        .unwrap();

        let schema = schema_from_json("row", &document).unwrap();
        assert_eq!(schema.field_len(), 4);
        assert!(!schema.is_nullable());
        assert_eq!(
            schema.fields()[1].get_metadata("iceberg:doc"),
            Some("ticker")
        );

        // Requirement inverts into nullability.
        assert!(!schema.fields()[0].is_nullable());
        assert!(schema.fields()[1].is_nullable());

        assert_eq!(schema_into_json(&schema).unwrap(), document);
    }

    #[test]
    fn identifiers_are_assigned_depth_first_and_never_reassigned() {
        let inner = DataType::from_fields([DataType::Int64.required_field("price")]).unwrap();
        let mut schema = DataType::from_fields([
            DataType::Int64.required_field("id"),
            inner.nullable_field("leg"),
        ])
        .unwrap()
        .required_field("row");

        assert_eq!(assign_field_ids(&mut schema, 1).unwrap(), 4);
        assert_eq!(schema.fields()[0].parquet_field_id().unwrap(), Some(1));
        assert_eq!(schema.fields()[1].parquet_field_id().unwrap(), Some(2));
        assert_eq!(
            schema.fields()[1].fields()[0].parquet_field_id().unwrap(),
            Some(3)
        );
        assert_eq!(super::super::last_column_id(&schema).unwrap(), 3);

        // A second pass changes nothing, because every field already has an id.
        assert_eq!(assign_field_ids(&mut schema, 100).unwrap(), 100);
        assert_eq!(schema.fields()[0].parquet_field_id().unwrap(), Some(1));
    }

    #[test]
    fn writing_a_schema_without_identifiers_says_what_to_call() {
        let schema = DataType::from_fields([DataType::Int64.required_field("id")])
            .unwrap()
            .required_field("row");

        let message = schema_into_json(&schema).unwrap_err().to_string();
        assert!(message.contains("assign_field_ids"), "{message}");
    }

    #[test]
    fn a_schema_document_that_is_not_a_struct_is_rejected() {
        let message = schema_from_json(
            "row",
            &crate::json::from_utf8(r#"{"type":"list"}"#).unwrap(),
        )
        .unwrap_err()
        .to_string();
        assert!(message.contains("\"struct\""), "{message}");

        let message = schema_from_json("row", &crate::json::from_utf8("[1, 2]").unwrap())
            .unwrap_err()
            .to_string();
        assert!(message.contains("got sequence"), "{message}");
    }

    #[test]
    fn a_field_missing_its_required_flag_is_rejected() {
        let document = crate::json::from_utf8(
            r#"{"type":"struct","fields":[{"id":1,"name":"id","type":"long"}]}"#,
        )
        .unwrap();
        let message = schema_from_json("row", &document).unwrap_err().to_string();
        assert!(message.contains("invalid schema JSON"), "{message}");
    }

    #[test]
    fn official_schema_validation_rejects_duplicate_field_ids() {
        let document = crate::json::from_utf8(
            r#"{"type":"struct","fields":[
                {"id":1,"name":"left","required":true,"type":"long"},
                {"id":1,"name":"right","required":true,"type":"string"}
            ]}"#,
        )
        .unwrap();

        let message = schema_from_json("row", &document).unwrap_err().to_string();
        assert!(message.contains("duplicate 'field.id' 1"), "{message}");
    }

    #[test]
    fn official_schema_validation_enforces_identifier_fields() {
        let document = crate::json::from_utf8(
            r#"{"type":"struct","schema-id":4,"identifier-field-ids":[1],"fields":[
                {"id":1,"name":"id","required":false,"type":"long"}
            ]}"#,
        )
        .unwrap();

        let message = schema_from_json("row", &document).unwrap_err().to_string();
        assert!(message.contains("optional field"), "{message}");
    }

    #[test]
    fn official_schema_normalization_supplies_the_default_schema_id() {
        let document = crate::json::from_utf8(
            r#"{"type":"struct","fields":[
                {"id":1,"name":"id","required":true,"type":"long"}
            ]}"#,
        )
        .unwrap();

        let schema = schema_from_json("row", &document).unwrap();
        assert_eq!(schema.get_metadata("iceberg:schema-id"), Some("0"));
        assert_eq!(
            schema_into_json(&schema)
                .unwrap()
                .get_key_str("schema-id")
                .and_then(Scalar::as_i64),
            Some(0)
        );
    }

    #[test]
    fn official_schema_normalization_sorts_identifier_field_ids() {
        let document = crate::json::from_utf8(
            r#"{"type":"struct","schema-id":3,"identifier-field-ids":[2,1],"fields":[
                {"id":1,"name":"left","required":true,"type":"long"},
                {"id":2,"name":"right","required":true,"type":"string"}
            ]}"#,
        )
        .unwrap();

        let schema = schema_from_json("row", &document).unwrap();
        assert_eq!(
            schema.get_metadata("iceberg:identifier-field-ids"),
            Some("1,2")
        );
        let emitted = schema_into_json(&schema).unwrap();
        let identifiers: Vec<i64> = emitted
            .get_key_str("identifier-field-ids")
            .into_iter()
            .flat_map(Scalar::sequence_iter)
            .filter_map(Scalar::as_i64)
            .collect();
        assert_eq!(identifiers, [1, 2]);
    }

    #[test]
    fn emitted_schema_is_checked_by_the_official_model() {
        let mut schema = DataType::from_fields([DataType::Int64.nullable_field("id")])
            .unwrap()
            .required_field("row");
        assign_field_ids(&mut schema, 1).unwrap();
        schema
            .insert_metadata("iceberg:identifier-field-ids", "1")
            .unwrap();

        let message = schema_into_json(&schema).unwrap_err().to_string();
        assert!(message.contains("optional field"), "{message}");
    }

    #[test]
    fn an_iceberg_schema_projects_into_arrow_with_its_identifiers() {
        let document = crate::json::from_utf8(
            r#"{"type":"struct","fields":[{"id":7,"name":"id","required":true,"type":"long"}]}"#,
        )
        .unwrap();
        let schema = schema_from_json("row", &document).unwrap();

        // The field id travels as Parquet metadata, which is what a data file needs.
        let arrow = schema.into_arrow_schema().unwrap();
        assert_eq!(
            arrow
                .field(0)
                .metadata()
                .get("PARQUET:field_id")
                .map(String::as_str),
            Some("7")
        );
    }

    #[test]
    fn the_v3_default_values_survive_a_round_trip() {
        let document = crate::json::from_utf8(
            r#"{"type":"struct","fields":[
                {"id":1,"name":"venue","required":false,"type":"string",
                 "initial-default":"XNAS","write-default":"XNAS"}
            ]}"#,
        )
        .unwrap();
        let schema = schema_from_json("row", &document).unwrap();
        assert_eq!(
            schema.fields()[0].get_metadata("iceberg:initial-default"),
            Some("\"XNAS\"")
        );
        let emitted = schema_into_json(&schema).unwrap();
        assert_eq!(
            emitted.get_key_str("schema-id").and_then(Scalar::as_i64),
            Some(0)
        );
        assert_eq!(
            emitted
                .get_key_str("fields")
                .and_then(Scalar::as_sequence)
                .and_then(|fields| fields.first())
                .and_then(|field| field.get_key_str("initial-default")),
            Some(&Scalar::from("XNAS"))
        );
    }

    #[test]
    fn a_schema_document_reads_through_the_core_json_parser() {
        // The point of the port: no second JSON value model reaches this module.
        let document: Scalar = crate::json::from_utf8(
            r#"{"type":"struct","fields":[{"id":1,"name":"id","required":true,"type":"long"}]}"#,
        )
        .unwrap();
        assert!(document.contains_key("fields"));
        assert!(schema_from_json("row", &document).is_ok());
    }
}

mod types {
    use super::super::PrimitiveType;
    use crate::{DataType, TimeUnit};

    #[test]
    fn every_primitive_type_round_trips_through_its_name() {
        let names = [
            "boolean",
            "int",
            "long",
            "float",
            "double",
            "decimal(18, 4)",
            "date",
            "time",
            "timestamp",
            "timestamptz",
            "timestamp_ns",
            "timestamptz_ns",
            "unknown",
            "string",
            "uuid",
            "binary",
        ];

        for name in names {
            let parsed =
                PrimitiveType::from_str(name).unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!(parsed.to_string(), name, "{name}");
            parsed
                .into_dtype()
                .unwrap_or_else(|error| panic!("{name}: {error}"));
        }

        let fixed = PrimitiveType::from_str("fixed[16]").unwrap();
        assert_eq!(fixed.to_string(), "fixed[16]");
        assert_eq!(
            fixed.into_dtype().unwrap(),
            DataType::fixed_size_binary(16).unwrap()
        );
    }

    #[test]
    fn iceberg_temporal_types_are_microsecond_precision_unless_v3_says_otherwise() {
        assert_eq!(
            PrimitiveType::Timestamp.into_dtype().unwrap(),
            DataType::Timestamp(TimeUnit::Microsecond, None)
        );
        assert_eq!(
            PrimitiveType::TimestampNs.into_dtype().unwrap(),
            DataType::Timestamp(TimeUnit::Nanosecond, None)
        );
        assert_eq!(
            PrimitiveType::Time.into_dtype().unwrap(),
            DataType::time(TimeUnit::Microsecond).unwrap()
        );
        // A v3 unknown column has no width at all, which Arrow spells as null.
        assert_eq!(PrimitiveType::Unknown.into_dtype().unwrap(), DataType::Null);
    }

    #[test]
    fn a_datatype_iceberg_cannot_express_is_named_rather_than_widened() {
        let message = PrimitiveType::from_dtype(&DataType::Int8)
            .unwrap_err()
            .to_string();
        assert!(message.contains("int8"), "{message}");
        assert!(message.contains("expected"), "{message}");
    }

    #[test]
    fn an_ascii_width_or_a_code_is_an_iceberg_string() {
        // The padding is storage; every Iceberg reader sees the text. Iceberg
        // has nothing that carries a code's identity, so a code is a string
        // there exactly as a width is.
        for dtype in [
            DataType::FixedAscii(2),
            DataType::FixedAscii(3),
            DataType::FixedAscii(4),
            DataType::FixedAscii(8),
            DataType::FixedAscii(12),
            DataType::FixedAscii(16),
            DataType::Country,
            DataType::Currency,
            DataType::Mic,
            DataType::Cfi,
        ] {
            assert_eq!(
                PrimitiveType::from_dtype(&dtype).unwrap(),
                PrimitiveType::String
            );
        }
    }

    #[test]
    fn a_code_bound_is_portable_and_round_trips_as_a_string_datum() {
        // Bounds are what a planner prunes with, and a missed arm in the
        // value layer drops them silently rather than failing.
        for (dtype, value) in [
            (DataType::Country, "FR"),
            (DataType::Currency, "USD"),
            (DataType::Mic, "XPAR"),
            (DataType::Cfi, "ESVUFR"),
        ] {
            assert!(crate::iceberg::value::is_portable(&dtype), "{dtype}");
            let scalar = crate::Scalar::from(value);
            let bytes = crate::iceberg::value::single_value(&scalar, &dtype)
                .unwrap_or_else(|| panic!("{dtype} must encode a bound"));
            assert_eq!(bytes, value.as_bytes(), "{dtype}");
            assert_eq!(
                crate::iceberg::value::single_to_value(&bytes, &dtype),
                Some(scalar),
                "{dtype}"
            );
        }
    }

    #[test]
    fn a_malformed_type_name_reports_what_was_expected() {
        let message = PrimitiveType::from_str("decimal(18)")
            .unwrap_err()
            .to_string();
        assert!(message.contains("decimal(precision, scale)"), "{message}");

        let message = PrimitiveType::from_str("varchar").unwrap_err().to_string();
        assert!(message.contains("\"varchar\""), "{message}");
    }
}

mod partition_specs {
    use super::{PartitionSpec, Transform, trade_schema};
    use crate::iceberg::assign_field_ids;
    use crate::{DataType, Scalar};

    #[test]
    fn a_spec_round_trips_through_its_v2_document() {
        let schema = trade_schema();
        let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
        assert_eq!(spec.fields[0].field_id, 1000);
        assert_eq!(spec.fields[0].source_id, 3);

        let document = spec.clone().into_json().unwrap();
        assert_eq!(PartitionSpec::from_json(&document).unwrap(), spec);
    }

    #[test]
    fn the_bare_v1_array_reads_as_a_spec_with_numbered_fields() {
        let document =
            crate::json::from_utf8(r#"[{"name":"venue","transform":"identity","source-id":3}]"#)
                .unwrap();
        let spec = PartitionSpec::from_json(&document).unwrap();
        assert_eq!(spec.spec_id, 0);
        assert_eq!(spec.fields[0].field_id, 1000);
        assert_eq!(spec.fields[0].transform, Transform::Identity);
    }

    #[test]
    fn a_hive_directory_is_what_a_partition_tuple_names() {
        let schema = trade_schema();
        let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
        assert_eq!(
            spec.partition_path(&[Scalar::from("XNAS")]).unwrap(),
            "venue=XNAS"
        );
        // A null value is spelled `null`, which a path cannot distinguish from
        // the string; that is why the manifest is the authority.
        assert_eq!(spec.partition_path(&[Scalar::Null]).unwrap(), "venue=null");
    }

    #[test]
    fn a_spec_is_read_from_and_written_back_onto_a_field() {
        let schema = trade_schema();
        let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();

        // The tuple carries what produced it, so the spec reads back off it.
        let partition = spec.partition_field(&schema).unwrap();
        assert_eq!(partition.as_iceberg().spec_id().unwrap(), Some(1));
        let venue = partition.get_field_by_path("venue").unwrap();
        assert!(venue.is_partition());
        assert_eq!(
            venue.as_iceberg().transform().unwrap(),
            Some(Transform::Identity)
        );
        assert_eq!(venue.as_iceberg().partition_source_id().unwrap(), Some(3));
        assert_eq!(
            PartitionSpec::from_partition_field(&partition).unwrap(),
            spec
        );

        // A schema that marks its own partition columns needs no column list.
        let marked = spec.mark_partitions(&schema).unwrap();
        assert_eq!(
            marked.partition_field_names().collect::<Vec<_>>(),
            ["venue"]
        );
        assert_eq!(PartitionSpec::from_schema(1, &marked).unwrap(), spec);
        assert_eq!(marked.without_partition_fields().unwrap().field_len(), 2);

        // A schema that marks nothing partitions nothing.
        assert!(
            PartitionSpec::from_schema(0, &schema)
                .unwrap()
                .is_unpartitioned()
        );
    }

    #[test]
    fn a_partition_directory_is_spelled_the_way_every_other_lake_spells_it() {
        let schema = DataType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::Date32.nullable_field("day"),
        ])
        .unwrap()
        .required_field("row");
        let mut schema = schema;
        assign_field_ids(&mut schema, 1).unwrap();
        let spec = PartitionSpec::identity(1, &schema, &["day"]).unwrap();

        // A date is days on the wire and a calendar day in a path, which is
        // what `column=value` means everywhere else in the crate.
        assert_eq!(
            spec.partition_path(&[Scalar::date32(19_723)]).unwrap(),
            "day=2024-01-01"
        );
        assert_eq!(
            crate::io::partition::partition_text(&Scalar::date32(19_723)).unwrap(),
            "2024-01-01"
        );
    }

    #[test]
    fn every_known_transform_is_writable_and_unknown_is_refused() {
        let schema = trade_schema();
        let mut spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
        for transform in [
            Transform::Identity,
            Transform::Bucket(16),
            Transform::Truncate(4),
            Transform::Year,
            Transform::Month,
            Transform::Day,
            Transform::Hour,
            Transform::Void,
        ] {
            spec.fields[0].transform = transform;
            spec.require_writable().unwrap();
        }

        spec.fields[0].transform = Transform::Unknown;
        let message = spec.require_writable().unwrap_err().to_string();
        assert!(message.contains("unknown"), "{message}");

        for transform in [Transform::Bucket(0), Transform::Truncate(0)] {
            spec.fields[0].transform = transform;
            assert!(spec.require_writable().is_err(), "{transform}");
        }
    }

    #[test]
    fn transform_result_types_and_validation_are_owned_by_apache_iceberg() {
        let timestamp = DataType::Timestamp(crate::TimeUnit::Microsecond, None);
        assert_eq!(
            Transform::Day.result_type(&timestamp).unwrap(),
            DataType::Date32
        );
        for transform in [Transform::Year, Transform::Month, Transform::Hour] {
            assert_eq!(transform.result_type(&timestamp).unwrap(), DataType::Int32);
        }
        assert_eq!(
            Transform::Bucket(16).result_type(&DataType::Utf8).unwrap(),
            DataType::Int32
        );
        assert_eq!(
            Transform::Truncate(3)
                .result_type(&DataType::Binary)
                .unwrap(),
            DataType::Binary
        );

        assert!(Transform::Year.result_type(&DataType::Int64).is_err());
        assert!(
            Transform::Bucket(16)
                .result_type(&DataType::Boolean)
                .is_err()
        );
        assert!(
            Transform::Truncate(3)
                .result_type(&DataType::Date32)
                .is_err()
        );
        assert!(Transform::Bucket(0).result_type(&DataType::Int32).is_err());
    }

    #[test]
    fn scalar_transform_plan_is_total_at_date_extremes_and_truncates_binary() {
        let mut schema = DataType::from_fields([
            DataType::Date32.required_field("day"),
            DataType::Binary.required_field("payload"),
        ])
        .unwrap()
        .required_field("row");
        assign_field_ids(&mut schema, 1).unwrap();
        let spec = PartitionSpec {
            spec_id: 0,
            fields: vec![
                super::PartitionField {
                    source_id: 1,
                    field_id: 1000,
                    name: "year".into(),
                    transform: Transform::Year,
                },
                super::PartitionField {
                    source_id: 2,
                    field_id: 1001,
                    name: "prefix".into(),
                    transform: Transform::Truncate(2),
                },
            ],
        };
        let partition = spec.partition_field(&schema).unwrap();
        let plan = spec.write_transforms(&schema, &partition).unwrap();

        assert!(matches!(
            plan[0].partition_value(Scalar::date32(i32::MAX)).unwrap(),
            Scalar::I32(_)
        ));
        assert_eq!(
            plan[1].partition_value(Scalar::from(&b"abcd"[..])).unwrap(),
            Scalar::from(&b"ab"[..])
        );
    }

    #[test]
    fn transform_parameters_keep_the_official_unsigned_width_and_unknown_is_opaque() {
        let widest = Transform::from_str("bucket[4294967295]").unwrap();
        assert_eq!(widest, Transform::Bucket(u32::MAX));
        assert_eq!(widest.to_string(), "bucket[4294967295]");
        assert!(Transform::from_str("truncate[-1]").is_err());
        assert!(Transform::from_str("bucket[4294967296]").is_err());

        let unknown = Transform::from_str("unknown").unwrap();
        assert_eq!(unknown, Transform::Unknown);
        assert_eq!(
            unknown.result_type(&DataType::Int64).unwrap(),
            DataType::Utf8
        );
        assert_eq!(
            Transform::Void.result_type(&DataType::Int64).unwrap(),
            DataType::Int64
        );
        assert!(!unknown.is_invertible());

        let mut spec = PartitionSpec::identity(1, &trade_schema(), &["venue"]).unwrap();
        spec.fields[0].transform = unknown;
        assert!(
            spec.require_writable()
                .unwrap_err()
                .to_string()
                .contains("unknown")
        );
    }

    #[test]
    fn a_partition_column_is_nullable_even_when_its_source_is_not() {
        let schema = trade_schema();
        let spec = PartitionSpec::identity(1, &schema, &["id"]).unwrap();
        let partition = spec.partition_field(&schema).unwrap();
        assert!(!schema.fields()[0].is_nullable());
        assert!(partition.fields()[0].is_nullable());
        assert_eq!(
            partition.fields()[0].parquet_field_id().unwrap(),
            Some(1000)
        );
    }
}

mod table_metadata {
    use std::hash::{Hash, Hasher};

    use crate::Scalar;

    use super::super::{Snapshot, SnapshotRef, SortField, SortOrder, TableMetadata, Transform};
    use super::{FormatVersion, PartitionSpec, trade_schema};
    use smol_str::SmolStr;

    fn metadata(version: FormatVersion) -> TableMetadata {
        TableMetadata::new(
            version,
            "file:///tmp/table",
            trade_schema(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap()
    }

    #[test]
    fn every_format_version_round_trips_through_its_document() {
        for version in [FormatVersion::V1, FormatVersion::V2, FormatVersion::V3] {
            let original = metadata(version);
            let document = original.clone().into_json().unwrap();
            let read = TableMetadata::from_json(&document).unwrap();
            assert_eq!(read.format_version, version);
            assert_eq!(read.location, original.location);
            assert_eq!(read.last_column_id, 3);
            assert!(read.current_snapshot().is_none());
        }
    }

    #[test]
    fn sort_order_identifiers_keep_the_official_signed_64_bit_width() {
        let order_id = i64::from(i32::MAX) + 41;
        let order = SortOrder {
            order_id,
            fields: vec![SortField {
                source_id: 1,
                transform: Transform::Identity,
                direction: "asc".into(),
                null_order: "nulls-first".into(),
            }],
        };
        assert_eq!(
            SortOrder::from_json(&order.clone().into_json().unwrap()).unwrap(),
            order
        );

        let mut metadata = metadata(FormatVersion::V2);
        metadata.sort_orders.push(order);
        metadata.default_sort_order_id = order_id;
        let document = metadata.into_json().unwrap();
        let read = TableMetadata::from_json(&document).unwrap();
        assert_eq!(read.default_sort_order_id, order_id);
        assert!(
            read.sort_orders
                .iter()
                .any(|order| order.order_id == order_id)
        );

        let missing = crate::json::from_utf8(r#"{"fields":[]}"#).unwrap();
        let message = SortOrder::from_json(&missing).unwrap_err().to_string();
        assert!(
            message.contains("order-id") && message.contains("64-bit"),
            "{message}"
        );
    }

    #[test]
    fn metadata_identity_normalizes_keyed_collections_but_not_history() {
        fn hash(value: &TableMetadata) -> u64 {
            let mut state = std::collections::hash_map::DefaultHasher::new();
            value.hash(&mut state);
            state.finish()
        }

        let mut metadata = metadata(FormatVersion::V2);
        metadata.add_schema(trade_schema()).unwrap();
        metadata
            .add_spec(PartitionSpec {
                spec_id: 7,
                fields: Vec::new(),
            })
            .unwrap();
        metadata
            .add_sort_order(SortOrder {
                order_id: 7,
                fields: vec![SortField {
                    source_id: 1,
                    transform: Transform::Identity,
                    direction: "asc".into(),
                    null_order: "nulls-first".into(),
                }],
            })
            .unwrap();
        metadata.set_property("z", "last").unwrap();
        metadata.set_property("a", "first").unwrap();
        let timestamp = metadata.last_updated_ms + 60_000;
        for (snapshot_id, parent_snapshot_id) in [(1, None), (2, Some(1))] {
            metadata
                .set_current_snapshot(Snapshot {
                    snapshot_id,
                    parent_snapshot_id,
                    sequence_number: Some(snapshot_id),
                    timestamp_ms: timestamp + snapshot_id,
                    manifest_list: format!("metadata/snap-{snapshot_id}.avro").into(),
                    manifests: None,
                    summary: vec![("operation".into(), "append".into())],
                    schema_id: Some(0),
                    encryption_key_id: None,
                    first_row_id: None,
                    added_rows: None,
                })
                .unwrap();
        }
        metadata
            .set_snapshot_ref("v1", SnapshotRef::tag(1))
            .unwrap();
        metadata.metadata_log = vec![
            (10, "metadata/v1.json".into()),
            (20, "metadata/v2.json".into()),
        ];
        metadata.validate().unwrap();

        let mut equivalent = metadata.clone();
        equivalent.schemas.reverse();
        equivalent.partition_specs.reverse();
        equivalent.sort_orders.reverse();
        equivalent.properties.reverse();
        equivalent.snapshots.reverse();
        equivalent.refs.reverse();
        equivalent.validate().unwrap();

        assert_eq!(metadata, equivalent);
        assert_eq!(metadata.cmp(&equivalent), std::cmp::Ordering::Equal);
        assert_eq!(hash(&metadata), hash(&equivalent));
        assert_eq!(metadata.stable_hash(), equivalent.stable_hash());

        let mut reordered_history = metadata.clone();
        reordered_history.snapshot_log.reverse();
        reordered_history.metadata_log.reverse();
        assert_ne!(metadata, reordered_history);
        assert_eq!(
            metadata.cmp(&reordered_history),
            reordered_history.cmp(&metadata).reverse()
        );
        assert_ne!(metadata.stable_hash(), reordered_history.stable_hash());

        let earlier = metadata.clone();
        let mut middle = metadata.clone();
        middle.last_updated_ms += 1;
        let mut later = metadata;
        later.last_updated_ms += 2;
        assert!(earlier < middle);
        assert!(middle < later);
        assert!(earlier < later);
    }

    #[test]
    fn a_v1_document_carries_the_singular_schema_and_spec_keys() {
        let document = metadata(FormatVersion::V1).into_json().unwrap();
        assert!(document.contains_key("schema"), "v1 needs the singular key");
        assert!(document.contains_key("partition-spec"));
        assert!(
            !document.contains_key("last-sequence-number"),
            "v1 has no sequence numbers"
        );
    }

    #[test]
    fn a_v1_document_written_by_someone_else_reads_without_the_plural_keys() {
        let document = crate::json::from_utf8(
            r#"{"format-version":1,"table-uuid":"0b4f7721-755e-5df5-ab6b-8a23c7905a82","location":"file:///t",
                "last-updated-ms":1,"last-column-id":1,
                "schema":{"type":"struct","fields":[
                    {"id":1,"name":"id","required":true,"type":"long"}]},
                "partition-spec":[]}"#,
        )
        .unwrap();
        let metadata = TableMetadata::from_json(&document).unwrap();
        assert_eq!(metadata.format_version, FormatVersion::V1);
        assert_eq!(metadata.schemas.len(), 1);
        assert_eq!(metadata.partition_specs.len(), 1);
        assert!(metadata.default_spec().unwrap().is_unpartitioned());
    }

    #[test]
    fn v1_direct_manifest_paths_survive_official_metadata_updates() {
        let mut metadata = metadata(FormatVersion::V1);
        let snapshot = Snapshot {
            snapshot_id: 7,
            parent_snapshot_id: None,
            sequence_number: None,
            timestamp_ms: metadata.last_updated_ms + 1,
            manifest_list: SmolStr::new_static(""),
            manifests: Some(vec![
                SmolStr::new_static("file:///t/metadata/a.avro"),
                SmolStr::new_static("file:///t/metadata/b.avro"),
            ]),
            summary: vec![(
                SmolStr::new_static("operation"),
                SmolStr::new_static("append"),
            )],
            schema_id: Some(0),
            encryption_key_id: None,
            first_row_id: None,
            added_rows: None,
        };
        metadata.set_current_snapshot(snapshot.clone()).unwrap();
        metadata.set_property("owner", "v1").unwrap();

        let document = metadata.clone().into_json().unwrap();
        let encoded = document
            .get_key_str("snapshots")
            .into_iter()
            .flat_map(Scalar::sequence_iter)
            .next()
            .unwrap();
        assert!(encoded.get_key_str("manifest-list").is_none());
        assert_eq!(
            encoded
                .get_key_str("manifests")
                .map(Scalar::sequence_iter)
                .unwrap_or_default()
                .count(),
            2
        );

        let mut read = TableMetadata::from_json(&document).unwrap();
        assert_eq!(read.current_snapshot().unwrap(), &snapshot);
        read.set_location("file:///moved").unwrap();
        assert_eq!(
            read.current_snapshot().unwrap().manifests,
            snapshot.manifests
        );
        assert_eq!(
            TableMetadata::from_json(&read.into_json().unwrap())
                .unwrap()
                .current_snapshot()
                .unwrap()
                .manifests,
            snapshot.manifests
        );
    }

    #[test]
    fn a_v3_document_carries_its_row_lineage() {
        let mut metadata = metadata(FormatVersion::V3);
        let timestamp_ms = metadata.last_updated_ms + 60_000;
        metadata
            .set_current_snapshot(Snapshot {
                snapshot_id: 5,
                parent_snapshot_id: None,
                sequence_number: Some(1),
                timestamp_ms,
                manifest_list: SmolStr::new_static("file:///t/metadata/snap.avro"),
                manifests: None,
                summary: vec![(
                    SmolStr::new_static("operation"),
                    SmolStr::new_static("append"),
                )],
                schema_id: Some(0),
                encryption_key_id: None,
                first_row_id: Some(0),
                added_rows: Some(12),
            })
            .unwrap();

        let document = metadata.into_json().unwrap();
        assert_eq!(
            document
                .get_key_str("next-row-id")
                .and_then(|id| id.as_i64()),
            Some(12)
        );
        let read = TableMetadata::from_json(&document).unwrap();
        let snapshot = read.current_snapshot().unwrap();
        assert_eq!(snapshot.first_row_id, Some(0));
        assert_eq!(snapshot.added_rows, Some(12));
        assert_eq!(snapshot.operation(), "append");
    }

    #[test]
    fn official_metadata_sections_survive_an_official_builder_update() {
        let metadata = metadata(FormatVersion::V3);
        let timestamp_ms = metadata.last_updated_ms;
        let snapshot = Snapshot {
            snapshot_id: 7,
            parent_snapshot_id: None,
            sequence_number: Some(0),
            timestamp_ms,
            manifest_list: SmolStr::new_static("file:///t/metadata/snap-7.avro"),
            manifests: None,
            summary: vec![(
                SmolStr::new_static("operation"),
                SmolStr::new_static("append"),
            )],
            schema_id: Some(0),
            encryption_key_id: Some(SmolStr::new_static("key-1")),
            first_row_id: Some(0),
            added_rows: Some(0),
        }
        .into_json(FormatVersion::V3)
        .unwrap();
        let statistics = crate::json::from_utf8(
            r#"[{"snapshot-id":7,"statistics-path":"s3://bucket/stats.puffin",
                "file-size-in-bytes":413,"file-footer-size-in-bytes":42,
                "key-metadata":"c3RhdHMta2V5",
                "blob-metadata":[{"type":"apache-datasketches-theta-v1",
                    "snapshot-id":7,"sequence-number":0,"fields":[1,2],
                    "properties":{"compression":"zstd","ndv":"2"}}]}]"#,
        )
        .unwrap();
        let partition_statistics = crate::json::from_utf8(
            r#"[{"snapshot-id":7,
                "statistics-path":"s3://bucket/partition-stats.parquet",
                "file-size-in-bytes":43}]"#,
        )
        .unwrap();
        let encryption_keys = crate::json::from_utf8(
            r#"[{"key-id":"key-1","encrypted-key-metadata":"aWNlYmVyZw==",
                "encrypted-by-id":"kms-1",
                "properties":{"algorithm":"AES-256","rotation":"1"}}]"#,
        )
        .unwrap();

        let mut document = metadata.into_json().unwrap();
        document = document
            .with_key("snapshots", Scalar::from_sequence([snapshot]))
            .unwrap()
            .with_key("statistics", statistics.clone())
            .unwrap()
            .with_key("partition-statistics", partition_statistics.clone())
            .unwrap()
            .with_key("encryption-keys", encryption_keys.clone())
            .unwrap();

        let mut parsed = TableMetadata::from_json(&document).unwrap();
        assert_eq!(
            parsed
                .snapshot_by_id(7)
                .and_then(|snapshot| snapshot.encryption_key_id.as_deref()),
            Some("key-1")
        );
        parsed.set_property("owner", "official-builder").unwrap();
        assert_eq!(parsed.property("owner"), Some("official-builder"));

        let emitted = parsed.into_json().unwrap();
        assert_eq!(emitted.get_key_str("statistics"), Some(&statistics));
        assert_eq!(
            emitted.get_key_str("partition-statistics"),
            Some(&partition_statistics)
        );
        assert_eq!(
            emitted.get_key_str("encryption-keys"),
            Some(&encryption_keys)
        );
        assert_eq!(
            emitted
                .get_key_str("snapshots")
                .into_iter()
                .flat_map(Scalar::sequence_iter)
                .find(|snapshot| {
                    snapshot.get_key_str("snapshot-id").and_then(Scalar::as_i64) == Some(7)
                })
                .and_then(|snapshot| snapshot.get_key_str("key-id"))
                .and_then(Scalar::as_str),
            Some("key-1")
        );
    }

    #[test]
    fn a_current_snapshot_of_minus_one_means_there_is_none() {
        let mut document = metadata(FormatVersion::V2).into_json().unwrap();
        document = document.with_key("current-snapshot-id", -1_i64).unwrap();
        let read = TableMetadata::from_json(&document).unwrap();
        assert!(read.current_snapshot_id.is_none());
        assert!(read.current_snapshot().is_none());
    }

    #[test]
    fn an_evolved_schema_keeps_the_old_one_and_numbers_above_it() {
        let mut metadata = metadata(FormatVersion::V2);
        let mut evolved = trade_schema();
        evolved.remove_metadata("iceberg:schema-id");
        let mut fields = evolved.fields().to_vec();
        fields.push(crate::DataType::Int64.nullable_field("quantity"));
        evolved
            .set_dtype(crate::DataType::from_fields(fields).unwrap())
            .unwrap();
        super::assign_field_ids(&mut evolved, metadata.last_column_id + 1).unwrap();

        let schema_id = metadata.add_schema(evolved).unwrap();
        assert_eq!(schema_id, 1);
        assert_eq!(metadata.schemas.len(), 2, "the old schema is retained");
        assert_eq!(metadata.last_column_id, 4);
        metadata.current_schema_id = schema_id;
        assert_eq!(metadata.current_schema().unwrap().field_len(), 4);

        // And the pair survives the document.
        let read = TableMetadata::from_json(&metadata.into_json().unwrap()).unwrap();
        assert_eq!(read.schemas.len(), 2);
        assert_eq!(read.current_schema().unwrap().field_len(), 4);
        assert_eq!(read.schema_by_id(0).unwrap().field_len(), 3);
    }

    #[test]
    fn a_format_version_this_build_does_not_implement_is_named() {
        let message = FormatVersion::from_number(4).unwrap_err().to_string();
        assert!(message.contains("got 4"), "{message}");
    }
}

mod tables {
    use std::sync::Arc;

    use arrow_array::{
        ArrayRef, Int32Array, Int64Array, RecordBatch, StringArray, StructArray,
        TimestampMicrosecondArray,
    };
    use arrow_schema::DataType as ArrowDataType;

    use super::{
        Folder, FormatVersion, IOBase, IcebergOptions, PartitionField, PartitionSpec, Table,
        Transform, assign_field_ids, collect, root, trade_schema, trades,
    };
    use crate::generic::IORecordOptions;
    use crate::io::IOMedia;
    use crate::{DataType, Scalar, TimeUnit};

    #[test]
    fn create_numbers_an_unnumbered_schema_itself() {
        let path = root("unnumbered-create");
        // The schema a user projects straight from Arrow: no ids anywhere.
        let schema = DataType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::Utf8.nullable_field("venue"),
        ])
        .unwrap()
        .required_field("row");

        let mut table = Table::create(
            Folder::new(&path).unwrap(),
            FormatVersion::V2,
            schema,
            PartitionSpec::unpartitioned(),
        )
        .unwrap();

        // Depth-first from 1, and the document records the numbering.
        let stored = table.schema().unwrap();
        let ids: Vec<i32> = stored
            .fields()
            .iter()
            .map(|child| child.parquet_field_id().unwrap().unwrap())
            .collect();
        assert_eq!(ids, [1, 2]);
        assert_eq!(table.metadata().last_column_id, 2);

        // The numbered table is a working table, not merely a written one.
        let batch = trades(&[7], &[None], &[Some("X")]);
        table
            .commit_append(crate::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap();
        let reopened = Table::open(Folder::new(&path).unwrap()).unwrap();
        assert_eq!(collect(reopened.scan(None).unwrap()).len(), 1);
    }

    #[test]
    fn create_keeps_the_ids_a_partly_numbered_schema_carries() {
        let path = root("partly-numbered-create");
        let mut id = DataType::Int64.required_field("id");
        id.set_parquet_field_id(7);
        let schema = DataType::from_fields([id, DataType::Utf8.nullable_field("venue")])
            .unwrap()
            .required_field("row");

        let table = Table::create(
            Folder::new(&path).unwrap(),
            FormatVersion::V2,
            schema,
            PartitionSpec::unpartitioned(),
        )
        .unwrap();

        // The carried id stays, the fresh one lands above it.
        let stored = table.schema().unwrap();
        let ids: Vec<i32> = stored
            .fields()
            .iter()
            .map(|child| child.parquet_field_id().unwrap().unwrap())
            .collect();
        assert_eq!(ids, [7, 8]);
        assert_eq!(table.metadata().last_column_id, 8);
    }

    #[test]
    fn an_empty_table_has_no_snapshot_and_reads_as_no_rows() {
        let path = root("empty");
        let schema = trade_schema();
        let table = Table::create(
            Folder::new(&path).unwrap(),
            FormatVersion::V2,
            schema,
            PartitionSpec::unpartitioned(),
        )
        .unwrap();

        assert!(table.current_snapshot().is_none());
        assert!(table.manifests().unwrap().is_empty());
        assert!(table.data_files().unwrap().is_empty());
        assert_eq!(table.row_size().unwrap(), 0);
        assert_eq!(table.column_size().unwrap(), 3);
        assert_eq!(collect(table.scan(None).unwrap()).len(), 0);

        // The document is on disk and reopening finds it.
        let reopened = Table::open(Folder::new(&path).unwrap()).unwrap();
        assert_eq!(reopened.metadata_version(), 1);
        assert!(reopened.current_snapshot().is_none());
    }

    #[test]
    fn open_preserves_an_official_uuid_metadata_name_in_history() {
        let path = root("official-metadata-name");
        let table = Table::create(
            Folder::new(&path).unwrap(),
            FormatVersion::V2,
            trade_schema(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let generated_name = table.metadata_file_name();
        drop(table);

        let official_name = "00001-123e4567-e89b-12d3-a456-426614174000.metadata.json";
        std::fs::rename(
            path.join("metadata").join(generated_name),
            path.join("metadata").join(official_name),
        )
        .unwrap();

        let mut reopened = Table::open(Folder::new(&path).unwrap()).unwrap();
        assert_eq!(reopened.metadata_file_name(), official_name);
        reopened
            .commit_metadata_changes(|metadata| {
                metadata.set_property("owner", "interop")?;
                Ok(())
            })
            .unwrap();
        assert!(
            reopened
                .metadata()
                .metadata_log
                .iter()
                .any(|(_, location)| location.ends_with(official_name))
        );
    }

    #[test]
    fn a_commit_publishes_the_name_a_version_hint_resolves() {
        let path = root("hint-resolvable-metadata-name");
        let mut table = Table::create(
            Folder::new(&path).unwrap(),
            FormatVersion::V2,
            trade_schema(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        assert_eq!(table.metadata_file_name(), "v1.metadata.json");
        table
            .commit_metadata_changes(|metadata| {
                metadata.set_property("owner", "interop")?;
                Ok(())
            })
            .unwrap();
        assert_eq!(table.metadata_file_name(), "v2.metadata.json");

        // `v{version}` is the only spelling a numeric hint resolves, so it is
        // the only one a catalog-free reader - Spark's Hadoop tables among
        // them - can follow. The unique attempt each commit writes is the
        // attempt and not the table, so nothing of it survives the commit.
        let mut names = std::fs::read_dir(path.join("metadata"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        names.sort();
        assert_eq!(
            names,
            ["v1.metadata.json", "v2.metadata.json", "version-hint.text"]
        );
        assert_eq!(
            std::fs::read_to_string(path.join("metadata").join("version-hint.text")).unwrap(),
            "2"
        );
    }

    #[test]
    fn metadata_compression_uses_the_official_property_and_gzip_magic() {
        let path = root("gzip-metadata");
        let mut table = Table::create(
            Folder::new(&path).unwrap(),
            FormatVersion::V2,
            trade_schema(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        table
            .commit_metadata_changes(|metadata| {
                metadata.set_property("write.metadata.compression-codec", "GZIP")?;
                Ok(())
            })
            .unwrap();

        assert_eq!(table.metadata_file_name(), "v2.gz.metadata.json");
        assert!(
            std::fs::read(path.join("metadata").join(table.metadata_file_name()))
                .unwrap()
                .starts_with(&[0x1f, 0x8b])
        );
        let reopened = Table::open(Folder::new(&path).unwrap()).unwrap();
        assert_eq!(reopened.metadata_file_name(), table.metadata_file_name());
        assert_eq!(reopened.metadata().property("owner"), None);
    }

    #[test]
    fn direct_create_refuses_to_replace_an_existing_table() {
        let path = root("create-conflict");
        Table::create(
            Folder::new(&path).unwrap(),
            FormatVersion::V2,
            trade_schema(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let error = Table::create(
            Folder::new(&path).unwrap(),
            FormatVersion::V2,
            trade_schema(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap_err();
        assert!(error.is_conflict(), "{error}");
        assert_eq!(
            Table::open(Folder::new(&path).unwrap())
                .unwrap()
                .metadata_version(),
            1
        );
    }

    #[test]
    fn child_locations_require_a_table_path_boundary() {
        let path = root("location-boundary");
        let table = Table::create(
            Folder::new(&path).unwrap(),
            FormatVersion::V2,
            trade_schema(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let sibling = format!("{}2/data/file.parquet", table.metadata().location);
        assert!(table.child_at(&sibling).is_err());
    }

    #[test]
    fn an_unpartitioned_table_round_trips_its_rows() {
        let path = root("unpartitioned");
        let schema = trade_schema();
        let mut table = Table::create(
            Folder::new(&path).unwrap(),
            FormatVersion::V2,
            schema.clone(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();

        let batch = trades(
            &[1, 2, 3],
            &[Some("AAPL"), Some("MSFT"), Some("AAPL")],
            &[Some("XNAS"), Some("XNYS"), Some("XNAS")],
        );
        table
            .commit_append(crate::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap();

        let snapshot = table.current_snapshot().expect("a snapshot");
        assert_eq!(snapshot.operation(), "append");
        assert_eq!(snapshot.summary_value("added-records"), Some("3"));
        assert_eq!(table.row_size().unwrap(), 3);
        assert_eq!(table.column_size().unwrap(), 3);
        assert_eq!(table.data_files().unwrap().len(), 1);

        let rows = collect(table.scan(None).unwrap());
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].0, 1);
        assert_eq!(rows[0].1.as_deref(), Some("AAPL"));

        // And a reopened table sees exactly the same thing.
        let reopened = Table::open(Folder::new(&path).unwrap()).unwrap();
        assert_eq!(collect(reopened.scan(None).unwrap()), rows);
    }

    #[test]
    fn a_v1_snapshot_with_direct_manifests_scans_and_time_travels() {
        let path = root("v1-direct-manifests");
        let mut table = Table::create(
            Folder::new(&path).unwrap(),
            FormatVersion::V1,
            trade_schema(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let first = trades(&[1], &[Some("AAPL")], &[Some("XNAS")]);
        table
            .commit_append(crate::arrow::batch_reader(first.schema(), [first]))
            .unwrap();
        let snapshot_id = table.current_snapshot().unwrap().snapshot_id;
        let manifest_paths: Vec<Scalar> = table
            .manifests()
            .unwrap()
            .into_iter()
            .map(|manifest| Scalar::from(manifest.manifest_path))
            .collect();

        // This is the original v1 snapshot fixture: manifests were embedded
        // directly in metadata before manifest lists became universal.
        let mut document = table.metadata().clone().into_json().unwrap();
        let snapshots = document
            .get_key_str("snapshots")
            .unwrap()
            .sequence_iter()
            .map(|snapshot| {
                snapshot
                    .without_key("manifest-list")
                    .unwrap()
                    .with_key("manifests", Scalar::from_sequence(manifest_paths.clone()))
                    .unwrap()
            })
            .collect::<Vec<_>>();
        document = document
            .with_key("snapshots", Scalar::from_sequence(snapshots))
            .unwrap();
        let metadata_path = path.join("metadata").join(table.metadata_file_name());
        std::fs::write(metadata_path, crate::json::into_bytes(&document).unwrap()).unwrap();

        let mut reopened = Table::open(Folder::new(&path).unwrap()).unwrap();
        let v1 = reopened.current_snapshot().unwrap();
        assert_eq!(v1.snapshot_id, snapshot_id);
        assert!(v1.manifest_list.is_empty());
        assert_eq!(v1.manifests.as_ref().map(Vec::len), Some(1));
        let synthesized = reopened.manifests().unwrap();
        assert_eq!(synthesized.len(), 1);
        assert_eq!(synthesized[0].added_files_count, Some(1));
        assert_eq!(collect(reopened.scan(None).unwrap()).len(), 1);

        let second = trades(&[2], &[Some("MSFT")], &[Some("XNYS")]);
        reopened
            .commit_append(crate::arrow::batch_reader(second.schema(), [second]))
            .unwrap();
        assert_eq!(collect(reopened.scan(None).unwrap()).len(), 2);
        assert_eq!(
            collect(reopened.scan_at(snapshot_id, &[], None).unwrap())
                .into_iter()
                .map(|row| row.0)
                .collect::<Vec<_>>(),
            [1]
        );
        assert!(
            reopened
                .metadata()
                .snapshot_by_id(snapshot_id)
                .unwrap()
                .manifests
                .is_some()
        );
    }

    #[test]
    fn a_partitioned_write_lays_files_out_the_hive_way() {
        let path = root("partitioned");
        let schema = trade_schema();
        let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
        let mut table =
            Table::create(Folder::new(&path).unwrap(), FormatVersion::V2, schema, spec).unwrap();

        let batch = trades(
            &[1, 2, 3],
            &[Some("AAPL"), Some("MSFT"), Some("AAPL")],
            &[Some("XNAS"), Some("XNYS"), Some("XNAS")],
        );
        table
            .commit_append(crate::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap();

        let files = table.data_files().unwrap();
        assert_eq!(files.len(), 2, "one file per distinct venue");

        // The layout is the `column=value` shape the crate's own Hive reader
        // already understands.
        for (file, _) in &files {
            let url = crate::Url::from_str(&file.file_path).unwrap();
            let partitions = url.hive_partitions();
            assert_eq!(partitions.len(), 1, "{}", file.file_path);
            assert_eq!(partitions[0].0, "venue");
            assert_eq!(
                Scalar::from(partitions[0].1.as_str()),
                file.partition[0],
                "the path and the manifest agree"
            );
        }

        // And `children_where` selects one partition's leaves from the folder.
        let folder = Folder::new(&path).unwrap();
        let selected: Vec<_> = folder
            .children_where(&[("venue", "XNAS")], false)
            .unwrap()
            .collect();
        assert_eq!(selected.len(), 1);

        let rows = collect(table.scan(None).unwrap());
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn data_writes_compute_all_supported_partition_transforms() {
        let path = root("transformed-partitions");
        let mut schema = DataType::from_fields([
            DataType::Int32.required_field("id"),
            DataType::Utf8.required_field("text"),
            DataType::Timestamp(TimeUnit::Microsecond, None).required_field("ts_year"),
            DataType::Timestamp(TimeUnit::Microsecond, None).required_field("ts_month"),
            DataType::Timestamp(TimeUnit::Microsecond, None).required_field("ts_day"),
            DataType::Timestamp(TimeUnit::Microsecond, None).required_field("ts_hour"),
            DataType::Int64.required_field("retired"),
        ])
        .unwrap()
        .required_field("row");
        assign_field_ids(&mut schema, 1).unwrap();
        let transforms = [
            (1, "id_bucket", Transform::Bucket(16)),
            (2, "text_prefix", Transform::Truncate(3)),
            (3, "year", Transform::Year),
            (4, "month", Transform::Month),
            (5, "day", Transform::Day),
            (6, "hour", Transform::Hour),
            (7, "retired_null", Transform::Void),
        ];
        let spec = PartitionSpec {
            spec_id: 0,
            fields: transforms
                .into_iter()
                .enumerate()
                .map(|(offset, (source_id, name, transform))| PartitionField {
                    source_id,
                    field_id: 1000 + i32::try_from(offset).unwrap(),
                    name: name.into(),
                    transform,
                })
                .collect(),
        };
        let mut table = Table::create(
            Folder::new(&path).unwrap(),
            FormatVersion::V2,
            schema.clone(),
            spec,
        )
        .unwrap();

        // 2017-11-16T22:31:08, the timestamp used by Apache Iceberg's own
        // transform fixtures.
        let timestamp = 1_510_871_468_000_000_i64;
        let arrow = crate::arrow::arrow_schema_from_field(&schema).unwrap();
        let batch = RecordBatch::try_new(
            arrow.clone(),
            vec![
                Arc::new(Int32Array::from(vec![34, 34])) as ArrayRef,
                Arc::new(StringArray::from(vec!["iceberg", "icecube"])) as ArrayRef,
                Arc::new(TimestampMicrosecondArray::from(vec![
                    timestamp,
                    timestamp + 1_000_000,
                ])) as ArrayRef,
                Arc::new(TimestampMicrosecondArray::from(vec![
                    timestamp,
                    timestamp + 1_000_000,
                ])) as ArrayRef,
                Arc::new(TimestampMicrosecondArray::from(vec![
                    timestamp,
                    timestamp + 1_000_000,
                ])) as ArrayRef,
                Arc::new(TimestampMicrosecondArray::from(vec![
                    timestamp,
                    timestamp + 1_000_000,
                ])) as ArrayRef,
                Arc::new(Int64Array::from(vec![9, 10])) as ArrayRef,
            ],
        )
        .unwrap();
        table
            .commit_append(crate::arrow::batch_reader(arrow, [batch]))
            .unwrap();

        let files = table.data_files().unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(
            files[0].0.partition,
            vec![
                Scalar::I32(3),
                Scalar::from("ice"),
                Scalar::I32(47),
                Scalar::I32(574),
                Scalar::date32(17_486),
                Scalar::I32(419_686),
                Scalar::Null,
            ]
        );
        assert!(
            files[0].0.file_path.contains(
                "id_bucket=3/text_prefix=ice/year=47/month=574/day=2017-11-16/\
                 hour=419686/retired_null=null"
            ),
            "{}",
            files[0].0.file_path
        );
        assert_eq!(
            table
                .scan(None)
                .unwrap()
                .map(|batch| batch.unwrap().num_rows())
                .sum::<usize>(),
            2
        );
    }

    #[test]
    fn a_nested_struct_partition_source_is_resolved_by_field_id() {
        let path = root("nested-transformed-partition");
        let nested = DataType::from_fields([DataType::Utf8.required_field("category")])
            .unwrap()
            .required_field("payload");
        let mut schema = DataType::from_fields([nested])
            .unwrap()
            .required_field("row");
        assign_field_ids(&mut schema, 1).unwrap();
        let spec = PartitionSpec {
            spec_id: 0,
            fields: vec![PartitionField {
                source_id: 2,
                field_id: 1000,
                name: "category_prefix".into(),
                transform: Transform::Truncate(2),
            }],
        };
        let mut table = Table::create(
            Folder::new(&path).unwrap(),
            FormatVersion::V2,
            schema.clone(),
            spec,
        )
        .unwrap();

        let arrow = crate::arrow::arrow_schema_from_field(&schema).unwrap();
        let ArrowDataType::Struct(children) = arrow.field(0).data_type() else {
            panic!("payload must be a struct")
        };
        let payload = StructArray::from(vec![(
            children[0].clone(),
            Arc::new(StringArray::from(vec!["iceberg", "icecube"])) as ArrayRef,
        )]);
        let batch = RecordBatch::try_new(arrow.clone(), vec![Arc::new(payload)]).unwrap();
        table
            .commit_append(crate::arrow::batch_reader(arrow, [batch]))
            .unwrap();

        let files = table.data_files().unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0.partition, vec![Scalar::from("ic")]);
        assert!(files[0].0.file_path.contains("category_prefix=ic"));
    }

    #[test]
    fn a_null_partition_value_writes_its_own_directory_and_reads_back_null() {
        let path = root("null-partition");
        let schema = trade_schema();
        let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
        let mut table =
            Table::create(Folder::new(&path).unwrap(), FormatVersion::V2, schema, spec).unwrap();

        let batch = trades(
            &[1, 2],
            &[Some("AAPL"), Some("MSFT")],
            &[Some("XNAS"), None],
        );
        table
            .commit_append(crate::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap();

        let files = table.data_files().unwrap();
        assert_eq!(files.len(), 2);
        let null_file = files
            .iter()
            .find(|(file, _)| file.partition[0].is_null())
            .expect("a file for the null partition");
        assert!(
            null_file.0.file_path.contains("venue=null"),
            "{}",
            null_file.0.file_path
        );

        let rows = collect(table.scan(None).unwrap());
        assert_eq!(rows.len(), 2);
        let null_row = rows.iter().find(|row| row.0 == 2).unwrap();
        assert_eq!(null_row.2, None, "the null partition value stays null");
    }

    #[test]
    fn appending_keeps_what_is_stored_and_overwriting_replaces_it() {
        let path = root("append-overwrite");
        let schema = trade_schema();
        let mut table = Table::create(
            Folder::new(&path).unwrap(),
            FormatVersion::V2,
            schema,
            PartitionSpec::unpartitioned(),
        )
        .unwrap();

        let first = trades(&[1], &[Some("AAPL")], &[Some("XNAS")]);
        table
            .commit_append(crate::arrow::batch_reader(first.schema(), [first]))
            .unwrap();
        let second = trades(&[2], &[Some("MSFT")], &[Some("XNYS")]);
        table
            .commit_append(crate::arrow::batch_reader(second.schema(), [second]))
            .unwrap();
        assert_eq!(collect(table.scan(None).unwrap()).len(), 2);
        assert_eq!(table.manifests().unwrap().len(), 2);

        let replacement = trades(&[9], &[Some("NVDA")], &[Some("XNAS")]);
        table
            .commit_overwrite(crate::arrow::batch_reader(
                replacement.schema(),
                [replacement],
            ))
            .unwrap();
        let rows = collect(table.scan(None).unwrap());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, 9);

        // The previous snapshot is still recorded, which is what makes the
        // overwrite reversible.
        assert_eq!(table.metadata().snapshots.len(), 3);
        assert!(
            table
                .current_snapshot()
                .and_then(|snapshot| snapshot.parent_snapshot_id)
                .is_some()
        );
    }

    #[test]
    fn a_scan_pushes_the_requested_columns_down_to_each_file() {
        let path = root("pushdown");
        let schema = trade_schema();
        let mut table = Table::create(
            Folder::new(&path).unwrap(),
            FormatVersion::V2,
            schema.clone(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let batch = trades(&[1, 2], &[Some("AAPL"), Some("MSFT")], &[Some("XNAS"); 2]);
        table
            .commit_append(crate::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap();

        let mut wanted = schema.without_fields(&["symbol", "venue"]).unwrap();
        wanted.remove_metadata("iceberg:schema-id");
        let reader = table.scan(Some(&wanted)).unwrap();
        assert_eq!(reader.schema().fields().len(), 1);
        for batch in reader {
            let batch = batch.unwrap();
            assert_eq!(batch.num_columns(), 1);
            assert_eq!(batch.schema().field(0).name(), "id");
        }

        // The narrowing happens at the file, not after it: the data file still
        // stores three columns, and the reader the scan opens over it reports
        // one, which is what a projection mask does rather than a later drop.
        let file = table.data_files().unwrap()[0].0.file_path.clone();
        let relative = file.rsplit("/data/").next().unwrap().to_owned();
        let handle = Folder::new(&path)
            .unwrap()
            .child_by_path(&format!("data/{relative}"))
            .unwrap();
        let options = handle.record_options().unwrap();
        assert_eq!(handle.read_arrow_field(&options).unwrap().field_len(), 3);
        assert_eq!(
            handle
                .read_arrow_reader(&options.with_field(wanted))
                .unwrap()
                .schema()
                .fields()
                .len(),
            1
        );
    }

    #[test]
    fn a_table_whose_schema_evolved_reads_old_files_with_the_new_column_null() {
        let path = root("evolution");
        let schema = trade_schema();
        let mut table = Table::create(
            Folder::new(&path).unwrap(),
            FormatVersion::V2,
            schema.clone(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let batch = trades(&[1], &[Some("AAPL")], &[Some("XNAS")]);
        table
            .commit_append(crate::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap();

        let mut evolved = schema.clone();
        evolved.remove_metadata("iceberg:schema-id");
        let mut fields = evolved.fields().to_vec();
        fields.push(DataType::Int64.nullable_field("quantity"));
        evolved
            .set_dtype(DataType::from_fields(fields).unwrap())
            .unwrap();
        super::assign_field_ids(&mut evolved, 4).unwrap();
        assert_eq!(table.evolve_schema(evolved).unwrap(), 1);
        assert_eq!(table.schema().unwrap().field_len(), 4);

        // The file written before the column existed still reads.
        let mut found = 0;
        for batch in table.scan(None).unwrap() {
            let batch = batch.unwrap();
            assert_eq!(batch.num_columns(), 4);
            let quantity = batch.column_by_name("quantity").unwrap();
            assert_eq!(quantity.null_count(), batch.num_rows());
            found += batch.num_rows();
        }
        assert_eq!(found, 1);

        // And new rows carry the new column.
        let arrow = table.schema().unwrap().clone().into_arrow_schema().unwrap();
        let widened = arrow_array::RecordBatch::try_new(
            arrow.clone(),
            vec![
                std::sync::Arc::new(arrow_array::Int64Array::from(vec![2_i64])),
                std::sync::Arc::new(arrow_array::StringArray::from(vec![Some("MSFT")])),
                std::sync::Arc::new(arrow_array::StringArray::from(vec![Some("XNYS")])),
                std::sync::Arc::new(arrow_array::Int64Array::from(vec![Some(50_i64)])),
            ],
        )
        .unwrap();
        table
            .commit_append(crate::arrow::batch_reader(arrow, [widened]))
            .unwrap();
        assert_eq!(collect(table.scan(None).unwrap()).len(), 2);
    }

    #[test]
    fn a_manifest_naming_a_file_that_is_not_there_fails_the_scan_not_the_metadata() {
        let path = root("missing-file");
        let schema = trade_schema();
        let mut table = Table::create(
            Folder::new(&path).unwrap(),
            FormatVersion::V2,
            schema,
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let batch = trades(&[1], &[Some("AAPL")], &[Some("XNAS")]);
        table
            .commit_append(crate::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap();

        let file = table.data_files().unwrap()[0].0.file_path.clone();
        let relative = file.rsplit("/data/").next().unwrap().to_owned();
        std::fs::remove_file(path.join("data").join(&relative)).unwrap();

        // The metadata still reads: a manifest is metadata, and it still says
        // what it always said.
        let reopened = Table::open(Folder::new(&path).unwrap()).unwrap();
        assert_eq!(reopened.data_files().unwrap().len(), 1);

        // The read is where absence shows up, and a missing resource is empty
        // rather than an error, so the scan yields no rows.
        assert_eq!(collect(reopened.scan(None).unwrap()).len(), 0);
    }

    #[test]
    fn a_v1_table_writes_and_reads_without_sequence_numbers() {
        let path = root("v1");
        let schema = trade_schema();
        let mut table = Table::create(
            Folder::new(&path).unwrap(),
            FormatVersion::V1,
            schema,
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let batch = trades(&[1, 2], &[Some("AAPL"), None], &[Some("XNAS"), None]);
        table
            .commit_append(crate::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap();

        assert_eq!(
            table.current_snapshot().unwrap().sequence_number,
            None,
            "v1 snapshots carry no sequence number"
        );
        let reopened = Table::open(Folder::new(&path).unwrap()).unwrap();
        assert_eq!(reopened.metadata().format_version, FormatVersion::V1);
        assert_eq!(collect(reopened.scan(None).unwrap()).len(), 2);
    }

    #[test]
    fn a_v3_table_tracks_row_lineage_across_commits() {
        let path = root("v3");
        let schema = trade_schema();
        let mut table = Table::create(
            Folder::new(&path).unwrap(),
            FormatVersion::V3,
            schema,
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        assert_eq!(table.metadata().next_row_id, Some(0));

        let first = trades(&[1, 2], &[Some("AAPL"), Some("MSFT")], &[Some("XNAS"); 2]);
        table
            .commit_append(crate::arrow::batch_reader(first.schema(), [first]))
            .unwrap();
        assert_eq!(table.current_snapshot().unwrap().first_row_id, Some(0));
        assert_eq!(table.current_snapshot().unwrap().added_rows, Some(2));
        assert_eq!(table.metadata().next_row_id, Some(2));
        assert_eq!(table.manifests().unwrap()[0].first_row_id, Some(0));
        assert_eq!(table.data_files().unwrap()[0].0.first_row_id, Some(0));

        let second = trades(&[3], &[Some("NVDA")], &[Some("XNYS")]);
        table
            .commit_append(crate::arrow::batch_reader(second.schema(), [second]))
            .unwrap();
        assert_eq!(table.current_snapshot().unwrap().first_row_id, Some(2));
        assert_eq!(table.current_snapshot().unwrap().added_rows, Some(1));
        assert_eq!(table.metadata().next_row_id, Some(3));
        let manifests = table.manifests().unwrap();
        assert_eq!(manifests[0].first_row_id, Some(0));
        assert_eq!(manifests[1].first_row_id, Some(2));
        let mut row_ids: Vec<i64> = table
            .data_files()
            .unwrap()
            .into_iter()
            .filter_map(|(file, _)| file.first_row_id)
            .collect();
        row_ids.sort_unstable();
        assert_eq!(row_ids, [0, 2]);

        let reopened = Table::open(Folder::new(&path).unwrap()).unwrap();
        assert_eq!(reopened.metadata().next_row_id, Some(3));
        assert_eq!(collect(reopened.scan(None).unwrap()).len(), 3);
    }

    #[test]
    fn v3_rewrites_are_rejected_before_reader_or_storage_mutation() {
        let path = root("v3-rewrite-lineage");
        let mut table = Table::create(
            Folder::new(&path).unwrap(),
            FormatVersion::V3,
            trade_schema(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        table.set_options(IcebergOptions::default().with_compact_after_commits(2));

        for id in [1_i64, 2] {
            let batch = trades(&[id], &[Some("AAPL")], &[Some("XNAS")]);
            table
                .commit_append(crate::arrow::batch_reader(batch.schema(), [batch]))
                .unwrap();
        }
        assert_eq!(table.data_files().unwrap().len(), 2);
        assert!(
            table
                .metadata()
                .snapshots
                .iter()
                .all(|snapshot| snapshot.operation() == "append"),
            "v3 auto-compaction must not rewrite retained rows"
        );

        let children = |directory: &str| {
            let mut paths: Vec<_> = std::fs::read_dir(path.join(directory))
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect();
            paths.sort();
            paths
        };
        let version = table.metadata_version();
        let metadata_hash = table.metadata().stable_hash();
        let data_files = children("data");
        let metadata_files = children("metadata");

        let incoming = trades(&[1], &[Some("AAPL.L")], &[Some("XLON")]);
        let reader_schema = incoming.schema();
        let pulls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let reader_pulls = Arc::clone(&pulls);
        let mut incoming = Some(incoming);
        let reader = crate::arrow::batch_reader(
            reader_schema,
            std::iter::from_fn(move || {
                reader_pulls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                incoming.take()
            }),
        );
        let merge_error = table
            .commit_merge_where(&[], reader, &["id".to_owned()], true)
            .expect_err("v3 keyed merge must preserve existing row IDs");
        assert_eq!(pulls.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert!(
            matches!(&merge_error, crate::Error::Iceberg { source: None, .. }),
            "{merge_error:?}"
        );
        assert!(merge_error.to_string().contains("merge"), "{merge_error}");
        assert!(merge_error.to_string().contains("row IDs"), "{merge_error}");

        let compact_error = table
            .compact()
            .expect_err("v3 compaction must preserve existing row IDs");
        assert!(
            matches!(&compact_error, crate::Error::Iceberg { source: None, .. }),
            "{compact_error:?}"
        );
        assert!(
            compact_error.to_string().contains("compaction"),
            "{compact_error}"
        );
        assert!(
            compact_error.to_string().contains("row IDs"),
            "{compact_error}"
        );

        assert_eq!(table.metadata_version(), version);
        assert_eq!(table.metadata().stable_hash(), metadata_hash);
        assert_eq!(children("data"), data_files);
        assert_eq!(children("metadata"), metadata_files);
    }

    #[test]
    fn v2_still_permits_keyed_merge_and_compaction() {
        let path = root("v2-rewrite-lineage");
        let mut table = Table::create(
            Folder::new(&path).unwrap(),
            FormatVersion::V2,
            trade_schema(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        for id in [1_i64, 2] {
            let batch = trades(&[id], &[Some("AAPL")], &[Some("XNAS")]);
            table
                .commit_append(crate::arrow::batch_reader(batch.schema(), [batch]))
                .unwrap();
        }

        let incoming = trades(&[1], &[Some("AAPL.L")], &[Some("XLON")]);
        table
            .commit_merge(
                crate::arrow::batch_reader(incoming.schema(), [incoming]),
                &["id".to_owned()],
                true,
            )
            .unwrap();
        assert_eq!(table.data_files().unwrap().len(), 2);

        let compaction = table.compact().unwrap();
        assert_eq!(compaction.files_before, 2);
        assert_eq!(compaction.files_after, 1);
        let mut rows = collect(table.scan(None).unwrap());
        rows.sort_by_key(|(id, _, _)| *id);
        assert_eq!(
            rows,
            vec![
                (1, Some("AAPL.L".to_owned()), Some("XLON".to_owned())),
                (2, Some("AAPL".to_owned()), Some("XNAS".to_owned())),
            ]
        );
    }

    #[test]
    fn the_first_v3_commit_after_upgrade_assigns_retained_rows() {
        let path = root("v3-upgrade-lineage");
        let schema = trade_schema();
        let mut table = Table::create(
            Folder::new(&path).unwrap(),
            FormatVersion::V2,
            schema,
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let old = trades(&[1, 2], &[Some("AAPL"), Some("MSFT")], &[Some("XNAS"); 2]);
        table
            .commit_append(crate::arrow::batch_reader(old.schema(), [old]))
            .unwrap();
        table
            .commit_metadata_changes(|metadata| metadata.upgrade_format_version(FormatVersion::V3))
            .unwrap();
        assert_eq!(table.metadata().next_row_id, Some(0));

        let added = trades(&[3], &[Some("NVDA")], &[Some("XNYS")]);
        table
            .commit_append(crate::arrow::batch_reader(added.schema(), [added]))
            .unwrap();
        let snapshot = table.current_snapshot().unwrap();
        assert_eq!(snapshot.first_row_id, Some(0));
        assert_eq!(snapshot.added_rows, Some(3));
        assert_eq!(table.metadata().next_row_id, Some(3));
        let manifests = table.manifests().unwrap();
        assert_eq!(manifests[0].first_row_id, Some(0));
        assert_eq!(manifests[1].first_row_id, Some(2));
        let mut row_ids: Vec<i64> = table
            .data_files()
            .unwrap()
            .into_iter()
            .filter_map(|(file, _)| file.first_row_id)
            .collect();
        row_ids.sort_unstable();
        assert_eq!(row_ids, [0, 2]);
    }

    #[test]
    fn a_spec_conflicting_with_its_source_name_is_refused_at_creation() {
        let path = root("unwritable-spec");
        let schema = trade_schema();
        let mut spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
        spec.fields[0].transform = super::Transform::Bucket(8);
        let message = Table::create(Folder::new(&path).unwrap(), FormatVersion::V2, schema, spec)
            .unwrap_err()
            .to_string();
        assert!(message.contains("venue"), "{message}");
        assert!(message.contains("conflicts"), "{message}");
    }

    #[test]
    fn a_folder_with_no_metadata_says_so_rather_than_pretending_to_be_a_table() {
        let path = root("not-a-table");
        std::fs::create_dir_all(&path).unwrap();
        let message = Table::open(Folder::new(&path).unwrap())
            .unwrap_err()
            .to_string();
        assert!(message.contains("metadata document"), "{message}");
    }

    #[test]
    fn a_manifest_describes_itself_well_enough_to_be_read_alone() {
        let path = root("self-describing");
        let schema = trade_schema();
        let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
        let mut table = Table::create(
            Folder::new(&path).unwrap(),
            FormatVersion::V2,
            schema,
            spec.clone(),
        )
        .unwrap();
        let batch = trades(&[1], &[Some("AAPL")], &[Some("XNAS")]);
        table
            .commit_append(crate::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap();

        let manifest = table.manifests().unwrap().remove(0);
        let name = manifest
            .manifest_path
            .rsplit('/')
            .next()
            .unwrap()
            .to_owned();
        let handle = Folder::new(&path)
            .unwrap()
            .child_by_path(&format!("metadata/{name}"))
            .unwrap();

        // The spec comes back out of the manifest's own Avro header.
        assert_eq!(super::super::read_manifest_spec(&handle).unwrap(), spec);
        let entries = super::super::read_manifest(&handle).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, super::super::EntryStatus::Added);
        assert_eq!(entries[0].data_file.record_count, 1);
        assert_eq!(entries[0].data_file.mime_type, crate::MimeType::PARQUET);
        // Statistics are keyed by field id, which is what a planner needs.
        assert!(
            entries[0]
                .data_file
                .value_counts
                .iter()
                .any(|(id, count)| *id == 1 && *count == 1)
        );
    }
}

mod planning {
    use super::{
        Field, FormatVersion, IOBase, PartitionSpec, Table, collect, root, trade_schema, trades,
    };
    use crate::DataType;
    use crate::local::Folder;

    /// A table partitioned by venue, with one commit per venue.
    ///
    /// One commit is one manifest, so this is also the smallest table whose
    /// manifest list has something to prune.
    fn venues(label: &str) -> (std::path::PathBuf, Table<Folder>) {
        let path = root(label);
        let schema = trade_schema();
        let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
        let mut table =
            Table::create(Folder::new(&path).unwrap(), FormatVersion::V2, schema, spec).unwrap();
        for (id, symbol, venue) in [
            (1_i64, "AAPL", "XNAS"),
            (2, "MSFT", "XNYS"),
            (3, "VOD", "XLON"),
        ] {
            let batch = trades(&[id], &[Some(symbol)], &[Some(venue)]);
            table
                .commit_append(crate::arrow::batch_reader(batch.schema(), [batch]))
                .unwrap();
        }
        (path, table)
    }

    #[test]
    fn a_partition_filter_skips_the_manifests_its_summaries_exclude() {
        let (_path, table) = venues("plan-manifests");

        let whole = table.plan(&[]).unwrap();
        assert_eq!(whole.tasks.len(), 3);
        assert_eq!(whole.manifests_read, 3);
        assert_eq!(whole.manifests_skipped(), 0);

        let filtered = table.plan(&[("venue", "XNYS")]).unwrap();
        assert_eq!(filtered.tasks.len(), 1);
        assert_eq!(filtered.record_count().unwrap(), 1);
        // Two manifests were excluded by their summaries alone, so two Avro
        // files were never opened.
        assert_eq!(filtered.manifests_skipped(), 2);
        assert_eq!(filtered.manifests_read, 1);
        assert_eq!(filtered.files_skipped(), 0);
    }

    #[test]
    fn an_expression_prunes_manifests_before_a_byte_is_read() {
        let (_path, table) = venues("plan-expression");

        // The same three-level pruning, driven by the crate's one filter type
        // rather than by an equality pair.
        let filtered = table.plan_matching("venue = 'XNYS'").unwrap();
        assert_eq!(filtered.tasks.len(), 1);
        assert_eq!(filtered.manifests_skipped(), 2);
        assert_eq!(filtered.manifests_read, 1);

        // A question about the *file* prunes at the same level, because a Hive
        // path is a statistic and an identity partition writes one.
        let by_path = table
            .plan_matching("&holder.partition['venue'] = 'XNYS'")
            .unwrap();
        assert_eq!(by_path.tasks.len(), 1);
        assert_eq!(by_path.manifests_skipped(), 2);

        // A range the summaries cannot settle still reads every manifest, and
        // still answers correctly from the rows.
        let ranged = table.plan_matching("id >= 2").unwrap();
        assert_eq!(ranged.manifests_read, 3);

        let rows = collect(table.scan_matching("id >= 2", None).unwrap());
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|(id, _, _)| *id >= 2));
    }

    #[test]
    fn one_predicate_mixes_the_file_and_the_rows() {
        let (_path, mut table) = venues("scan-mixed");
        // A second row in a partition that already exists is what makes the row
        // half of the predicate load-bearing: with one row per venue, a
        // conjunct over the rows is settled by the partition and the test would
        // pass even if the rows were never consulted.
        let batch = trades(&[4], &[Some("BP")], &[Some("XLON")]);
        table
            .commit_append(crate::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap();

        let rows = collect(
            table
                .scan_matching(
                    "id >= 4 and symbol is not null and &holder.partition['venue'] = 'XLON'",
                    None,
                )
                .unwrap(),
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, 4);
        assert_eq!(rows[0].2.as_deref(), Some("XLON"));

        // Each half on its own keeps more, so neither was dropped above.
        let held = collect(
            table
                .scan_matching("&holder.partition['venue'] = 'XLON'", None)
                .unwrap(),
        );
        assert_eq!(held.iter().map(|row| row.0).collect::<Vec<_>>(), vec![3, 4]);
        let ranged = collect(table.scan_matching("id >= 4", None).unwrap());
        assert_eq!(ranged.iter().map(|row| row.0).collect::<Vec<_>>(), vec![4]);

        // A predicate the metadata proves empty reads nothing at all.
        let empty = table.plan_matching("id > 1000").unwrap();
        assert_eq!(empty.tasks.len(), 0);
        assert_eq!(empty.files_skipped(), 4);

        // The pair spelling and the expression spelling are one plan. Each side
        // is pinned to the measured number, so two broken sides cannot agree
        // their way past this.
        let by_pair = table.plan(&[("venue", "XLON")]).unwrap();
        let by_text = table.plan_matching("venue = 'XLON'").unwrap();
        assert_eq!(by_pair.tasks.len(), 2);
        assert_eq!(by_text.tasks.len(), 2);
        assert_eq!(by_pair.manifests_skipped(), 2);
        assert_eq!(by_text.manifests_skipped(), 2);
    }

    #[test]
    fn a_filtered_read_never_opens_the_files_the_metadata_excluded() {
        let (path, table) = venues("plan-untouched");
        let excluded = table
            .plan(&[("venue", "XLON")])
            .unwrap()
            .tasks
            .remove(0)
            .entry
            .data_file
            .file_path
            .clone();

        // Replacing the excluded file's bytes with nonsense is the one proof
        // that the read never reaches it.
        let relative = excluded.rsplit("/data/").next().unwrap().to_owned();
        let mut handle = Folder::new(&path)
            .unwrap()
            .child_by_path(&format!("data/{relative}"))
            .unwrap();
        handle.write_all_bytes(b"not a parquet file").unwrap();

        assert_eq!(
            collect(table.scan_where(&[("venue", "XNAS")], None).unwrap()),
            vec![(1, Some("AAPL".to_owned()), Some("XNAS".to_owned()))]
        );
        assert!(
            table.scan(None).unwrap().any(|batch| batch.is_err()),
            "the unfiltered scan does read the file the filtered one skipped"
        );
    }

    #[test]
    fn a_filter_on_a_stored_column_prunes_by_statistics_and_then_by_row() {
        let path = root("plan-statistics");
        let schema = trade_schema();
        let mut table = Table::create(
            Folder::new(&path).unwrap(),
            FormatVersion::V2,
            schema,
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        // Two commits, so two files whose id bounds do not overlap.
        for ids in [[1_i64, 2], [10, 11]] {
            let batch = trades(&ids, &[Some("AAPL"), Some("MSFT")], &[None, None]);
            table
                .commit_append(crate::arrow::batch_reader(batch.schema(), [batch]))
                .unwrap();
        }

        let plan = table.plan(&[("id", "10")]).unwrap();
        assert_eq!(plan.tasks.len(), 1, "the file whose id bounds exclude 10");
        assert_eq!(plan.files_skipped(), 1);
        // A statistic bounds a file rather than selecting a row, so the file
        // that survived is still filtered down to the row that matches.
        assert_eq!(
            collect(table.scan_where(&[("id", "10")], None).unwrap()),
            vec![(10, Some("AAPL".to_owned()), None)]
        );
    }

    #[test]
    fn a_filter_column_the_read_never_asked_for_is_read_and_then_dropped() {
        let (_path, table) = venues("plan-projection");
        let target = DataType::from_fields([DataType::Int64.required_field("id")])
            .unwrap()
            .required_field("row");

        let reader = table
            .scan_where(&[("symbol", "MSFT")], Some(&target))
            .unwrap();
        let batches: Vec<_> = reader.map(Result::unwrap).collect();
        let rows: i64 = batches
            .iter()
            .map(|batch| i64::try_from(batch.num_rows()).unwrap())
            .sum();
        assert_eq!(rows, 1);
        // The projection is what the caller asked for, not what the filter
        // needed in order to answer.
        assert_eq!(batches[0].schema().fields().len(), 1);
        assert_eq!(batches[0].schema().field(0).name(), "id");
    }

    #[test]
    fn a_null_partition_value_is_addressed_by_the_text_the_layout_spells() {
        let path = root("plan-null");
        let schema = trade_schema();
        let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
        let mut table =
            Table::create(Folder::new(&path).unwrap(), FormatVersion::V2, schema, spec).unwrap();
        let batch = trades(
            &[1, 2],
            &[Some("AAPL"), Some("MSFT")],
            &[Some("XNAS"), None],
        );
        table
            .commit_append(crate::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap();

        let plan = table.plan(&[("venue", "null")]).unwrap();
        assert_eq!(plan.tasks.len(), 1);
        assert_eq!(
            collect(table.scan_where(&[("venue", "null")], None).unwrap()),
            vec![(2, Some("MSFT".to_owned()), None)]
        );
    }

    #[test]
    fn a_filter_naming_a_column_the_schema_does_not_have_lists_the_ones_it_does() {
        let (_path, table) = venues("plan-unknown");
        let message = table.plan(&[("exchange", "XNAS")]).unwrap_err().to_string();
        // The whole clause, not just the two names in it: asserting only that
        // the column list appears somewhere let a stray ", got" sit between
        // "it has" and the list, which is what a reader would actually see.
        // The sentence now comes from the one binder every filter in the
        // workspace goes through, so it says "the schema" rather than "the
        // table schema" - a lake reaches the same words.
        assert!(
            message.contains(
                "expected a column the schema declares, got \"exchange\"; it has id, symbol, venue"
            ),
            "{message}"
        );
    }

    #[test]
    fn a_manifest_list_carries_the_summaries_the_next_plan_prunes_with() {
        let (_path, table) = venues("plan-summaries");
        let manifests = table.manifests().unwrap();
        assert_eq!(manifests.len(), 3);
        for manifest in &manifests {
            assert_eq!(manifest.partitions.len(), 1, "one summary per spec field");
            let summary = &manifest.partitions[0];
            assert!(!summary.contains_null);
            assert_eq!(summary.lower_bound, summary.upper_bound);
            assert!(summary.lower_bound.is_some());
        }
        // The bounds are the venue strings themselves, which is the Iceberg
        // single-value encoding for text.
        let mut venues: Vec<String> = manifests
            .iter()
            .map(|manifest| {
                String::from_utf8(manifest.partitions[0].lower_bound.clone().unwrap()).unwrap()
            })
            .collect();
        venues.sort();
        assert_eq!(venues, ["XLON", "XNAS", "XNYS"]);
    }

    #[test]
    fn an_entry_inherits_the_sequence_number_its_manifest_records() {
        let (_path, table) = venues("plan-inheritance");
        let plan = table.plan(&[]).unwrap();
        let mut numbers: Vec<i64> = plan
            .tasks
            .iter()
            .map(|task| task.entry.sequence_number.unwrap())
            .collect();
        numbers.sort_unstable();
        // Three commits, three sequence numbers, none of them written into the
        // entries themselves.
        assert_eq!(numbers, [1, 2, 3]);
    }

    #[test]
    fn a_scan_root_the_caller_declares_is_what_the_reader_reports() {
        let (_path, table) = venues("plan-schema");
        let target: Field = DataType::from_fields([
            DataType::Utf8.nullable_field("venue"),
            DataType::Int64.required_field("id"),
        ])
        .unwrap()
        .required_field("row");
        let reader = table.scan(Some(&target)).unwrap();
        let schema = reader.schema();
        assert_eq!(schema.fields().len(), 2);
        assert_eq!(schema.field(0).name(), "venue");
        assert_eq!(schema.field(1).name(), "id");
    }
}

mod handles {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use arrow_array::{
        Array, Int64Array, RecordBatch, RecordBatchIterator, RecordBatchReader, StringArray,
    };
    use arrow_schema::{ArrowError, SchemaRef};

    use super::{
        FormatVersion, IOBase, PartitionSpec, Table, assign_field_ids, collect, root, trade_schema,
        trades,
    };
    use crate::generic::{IORecordOptions, RecordOptions};
    use crate::io::{Buffer, IOMedia};
    use crate::local::Folder;
    use crate::{DataType, MimeType};

    /// Create a venue-partitioned table and return the folder addressing it.
    fn table(label: &str) -> (std::path::PathBuf, Folder) {
        let path = root(label);
        let schema = trade_schema();
        let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
        Table::create(Folder::new(&path).unwrap(), FormatVersion::V2, schema, spec).unwrap();
        let folder = Folder::new(&path).unwrap();
        (path, folder)
    }

    /// The options an Iceberg folder is written through, with a declared schema.
    fn options(folder: &Folder) -> RecordOptions {
        folder.record_options().unwrap().with_field(trade_schema())
    }

    struct CountedReader {
        schema: SchemaRef,
        batch: Option<RecordBatch>,
        pulls: Arc<AtomicUsize>,
    }

    impl Iterator for CountedReader {
        type Item = std::result::Result<RecordBatch, ArrowError>;

        fn next(&mut self) -> Option<Self::Item> {
            let batch = self.batch.take()?;
            self.pulls.fetch_add(1, Ordering::SeqCst);
            Some(Ok(batch))
        }
    }

    impl RecordBatchReader for CountedReader {
        fn schema(&self) -> SchemaRef {
            Arc::clone(&self.schema)
        }
    }

    fn counted(batch: RecordBatch, pulls: Arc<AtomicUsize>) -> crate::arrow::BatchReader {
        Box::new(CountedReader {
            schema: batch.schema(),
            batch: Some(batch),
            pulls,
        })
    }

    #[test]
    fn a_table_folder_answers_with_the_encoding_of_its_data_files() {
        let (_path, folder) = table("handle-options");
        // Nothing has been written yet, so there is no leaf to read a media
        // type off; the metadata still knows what the rows will be.
        assert_eq!(
            folder.record_options().unwrap().mime_type(),
            crate::MimeType::PARQUET
        );
    }

    #[test]
    fn the_three_record_methods_reach_a_table_through_its_snapshots() {
        let (path, mut folder) = table("handle-three");
        let options = options(&folder);

        let batch = trades(
            &[1, 2],
            &[Some("AAPL"), Some("MSFT")],
            &[Some("XNAS"), Some("XNYS")],
        );
        folder
            .overwrite_arrow_reader(
                crate::arrow::batch_reader(batch.schema(), [batch]),
                &options,
            )
            .unwrap();
        assert_eq!(
            collect(folder.read_arrow_reader(&options).unwrap()).len(),
            2
        );

        let batch = trades(&[3], &[Some("VOD")], &[Some("XLON")]);
        folder
            .append_arrow_reader(
                crate::arrow::batch_reader(batch.schema(), [batch]),
                &options,
            )
            .unwrap();
        assert_eq!(
            collect(folder.read_arrow_reader(&options).unwrap()).len(),
            3
        );

        // An overwrite replaces every row, and the table still reads as a table.
        let batch = trades(&[9], &[Some("BP")], &[Some("XLON")]);
        folder
            .overwrite_arrow_reader(
                crate::arrow::batch_reader(batch.schema(), [batch]),
                &options,
            )
            .unwrap();
        assert_eq!(
            collect(folder.read_arrow_reader(&options).unwrap()),
            vec![(9, Some("BP".to_owned()), Some("XLON".to_owned()))]
        );

        // Every write was a snapshot, so the table has one commit per call.
        let table = Table::open(Folder::new(&path).unwrap()).unwrap();
        assert_eq!(table.metadata().snapshots.len(), 3);
        assert_eq!(table.current_snapshot().unwrap().operation(), "overwrite");
    }

    #[test]
    fn a_write_with_a_match_key_upserts_the_table_through_the_same_surface() {
        let (_path, mut folder) = table("handle-merge");
        let options = options(&folder);

        let batch = trades(
            &[1, 2, 3],
            &[Some("AAPL"), Some("MSFT"), Some("VOD")],
            &[Some("XNAS"), Some("XNYS"), Some("XLON")],
        );
        folder
            .overwrite_arrow_reader(
                crate::arrow::batch_reader(batch.schema(), [batch]),
                &options,
            )
            .unwrap();

        let merge = options.clone().with_merge_by_names(["id"]);
        let batch = trades(
            &[2, 4],
            &[Some("MSFT.L"), Some("BP")],
            &[Some("XNYS"), Some("XLON")],
        );
        folder
            .merge_arrow_reader(crate::arrow::batch_reader(batch.schema(), [batch]), &merge)
            .unwrap();

        assert_eq!(
            collect(folder.read_arrow_reader(&options).unwrap()),
            vec![
                (1, Some("AAPL".to_owned()), Some("XNAS".to_owned())),
                (2, Some("MSFT.L".to_owned()), Some("XNYS".to_owned())),
                (3, Some("VOD".to_owned()), Some("XLON".to_owned())),
                (4, Some("BP".to_owned()), Some("XLON".to_owned())),
            ]
        );
    }

    #[test]
    fn a_merge_reads_only_the_files_whose_statistics_can_hold_a_key() {
        let path = root("handle-merge-plan");
        let schema = trade_schema();
        let mut table = Table::create(
            Folder::new(&path).unwrap(),
            FormatVersion::V2,
            schema,
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        for ids in [[1_i64, 2], [10, 11], [20, 21]] {
            let batch = trades(&ids, &[Some("AAPL"), Some("MSFT")], &[None, None]);
            table
                .commit_append(crate::arrow::batch_reader(batch.schema(), [batch]))
                .unwrap();
        }
        let before: Vec<String> = table
            .data_files()
            .unwrap()
            .into_iter()
            .map(|(file, _)| file.file_path.to_string())
            .collect();
        assert_eq!(before.len(), 3);

        let batch = trades(&[11], &[Some("MSFT.L")], &[None]);
        table
            .commit_merge(
                crate::arrow::batch_reader(batch.schema(), [batch]),
                &["id".to_owned()],
                true,
            )
            .unwrap();

        let after: Vec<String> = table
            .data_files()
            .unwrap()
            .into_iter()
            .map(|(file, _)| file.file_path.to_string())
            .collect();
        // Two of the three files were carried into the new snapshot untouched:
        // their id bounds cannot hold the key 11, so they were never read and
        // never rewritten.
        let carried = before.iter().filter(|path| after.contains(path)).count();
        assert_eq!(carried, 2, "before {before:?} after {after:?}");
        assert_eq!(after.len(), 3);

        let mut ids: Vec<i64> = collect(table.scan(None).unwrap())
            .into_iter()
            .map(|(id, _, _)| id)
            .collect();
        ids.sort_unstable();
        assert_eq!(ids, [1, 2, 10, 11, 20, 21]);
        assert_eq!(
            collect(table.scan_where(&[("id", "11")], None).unwrap()),
            vec![(11, Some("MSFT.L".to_owned()), None)]
        );
    }

    #[test]
    fn a_handle_addressing_one_partition_reads_and_writes_only_that_partition() {
        let (path, mut folder) = table("handle-partition");
        let options = options(&folder);
        let batch = trades(
            &[1, 2, 3],
            &[Some("AAPL"), Some("MSFT"), Some("VOD")],
            &[Some("XNAS"), Some("XNYS"), Some("XLON")],
        );
        folder
            .overwrite_arrow_reader(
                crate::arrow::batch_reader(batch.schema(), [batch]),
                &options,
            )
            .unwrap();

        // The same address a Hive lake would use, resolved through the table's
        // metadata rather than by listing the directory.
        let partition = Folder::new(path.join("data").join("venue=XNYS")).unwrap();
        assert_eq!(
            collect(partition.read_arrow_reader(&options).unwrap()),
            vec![(2, Some("MSFT".to_owned()), Some("XNYS".to_owned()))]
        );

        // Overwriting one partition leaves every other one where it was.
        let mut partition = partition;
        let batch = trades(&[7], &[Some("MSFT")], &[Some("XNYS")]);
        partition
            .overwrite_arrow_reader(
                crate::arrow::batch_reader(batch.schema(), [batch]),
                &options,
            )
            .unwrap();
        assert_eq!(
            collect(folder.read_arrow_reader(&options).unwrap()),
            vec![
                (1, Some("AAPL".to_owned()), Some("XNAS".to_owned())),
                (3, Some("VOD".to_owned()), Some("XLON".to_owned())),
                (7, Some("MSFT".to_owned()), Some("XNYS".to_owned())),
            ]
        );
    }

    #[test]
    fn a_folder_that_is_not_a_table_still_reads_as_the_leaves_beneath_it() {
        let path = root("handle-plain");
        let lake = Folder::new(&path).unwrap();
        let mut leaf = lake.child_by_path("part-0.parquet").unwrap();
        let batch = trades(&[1], &[Some("AAPL")], &[Some("XNAS")]);
        let options = RecordOptions::for_media_type(leaf.media_type())
            .unwrap()
            .with_field(trade_schema());
        leaf.overwrite_arrow_reader(
            crate::arrow::batch_reader(batch.schema(), [batch]),
            &options,
        )
        .unwrap();
        leaf.close().unwrap();
        drop(leaf);

        let lake = Folder::new(&path).unwrap();
        assert_eq!(collect(lake.read_arrow_reader(&options).unwrap()).len(), 1);
    }

    #[test]
    fn the_table_value_is_itself_a_handle_answering_from_its_metadata() {
        let path = root("handle-table-value");
        let schema = trade_schema();
        let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
        let mut table =
            Table::create(Folder::new(&path).unwrap(), FormatVersion::V2, schema, spec).unwrap();

        // The byte surface is the folder the table lives in, but the role is
        // the table's own: one tabular value, never a folder to be listed.
        assert_eq!(table.kind(), crate::IOKind::Table);
        assert!(table.is_container());
        assert!(table.is_tabular());
        assert!(!table.is_atomic());
        assert_eq!(table.root().kind(), crate::IOKind::Directory);
        assert_eq!(
            IOBase::url(&table).unwrap().to_string(),
            Folder::new(&path).unwrap().url().to_string()
        );
        assert!(table.child_by_path("metadata").is_ok());

        // The record surface is answered before a single data file exists:
        // the encoding from what this module writes, the schema from the
        // metadata - field identifiers included - never off decoded batches.
        let options = crate::io::IOMedia::record_options(&table).unwrap();
        assert_eq!(options.mime_type(), crate::MimeType::PARQUET);
        let field = table.read_arrow_field(&options).unwrap();
        assert_eq!(field.name(), options.name());
        assert_eq!(field.fields()[0].parquet_field_id().unwrap(), Some(1));
        assert_eq!(field.fields()[2].parquet_field_id().unwrap(), Some(3));
        let declared = options.clone().with_field(trade_schema());
        assert_eq!(table.read_arrow_field(&declared).unwrap(), trade_schema());

        // Writing through the generic surface is one commit each, and the
        // in-memory metadata follows without reopening anything.
        let batch = trades(
            &[1, 2],
            &[Some("AAPL"), Some("MSFT")],
            &[Some("XNAS"), Some("XNYS")],
        );
        table
            .append_arrow_reader(
                crate::arrow::batch_reader(batch.schema(), [batch]),
                &options,
            )
            .unwrap();
        let batch = trades(&[3], &[Some("VOD")], &[Some("XLON")]);
        table
            .append_arrow_reader(
                crate::arrow::batch_reader(batch.schema(), [batch]),
                &options,
            )
            .unwrap();
        assert_eq!(table.metadata().snapshots.len(), 2);
        assert_eq!(table.current_snapshot().unwrap().operation(), "append");
        assert_eq!(collect(table.read_arrow_reader(&options).unwrap()).len(), 3);

        // A partition filter is answered by the plan - the other partitions'
        // files are never opened - and the rows match the folder route's.
        let filtered = options.clone().with_filter_partitions([("venue", "XNYS")]);
        assert_eq!(
            collect(table.read_arrow_reader(&filtered).unwrap()),
            vec![(2, Some("MSFT".to_owned()), Some("XNYS".to_owned()))]
        );
        let plan = table.plan(&[("venue", "XNYS")]).unwrap();
        assert_eq!(plan.tasks.len(), 1);
        assert!(plan.excluded.len() + plan.skipped.len() >= 1);

        // A selection narrows the read to the named columns.
        let selected = options.clone().with_select_by_names(["id"]);
        let reader = table.read_arrow_reader(&selected).unwrap();
        let names: Vec<String> = reader
            .schema()
            .fields()
            .iter()
            .map(|field| field.name().clone())
            .collect();
        assert_eq!(names, ["id"]);

        // A filter naming a column the schema does not declare is an error,
        // exactly as `scan_where` reports it: the schema is authoritative.
        let unanswerable = options.clone().with_filter_partitions([("desk", "42")]);
        assert!(table.read_arrow_reader(&unanswerable).is_err());

        // The folder route reads the same rows through the same snapshot.
        let folder = Folder::new(&path).unwrap();
        assert_eq!(
            collect(table.read_arrow_reader(&options).unwrap()),
            collect(folder.read_arrow_reader(&options).unwrap())
        );
    }

    #[test]
    fn a_write_through_the_table_value_is_one_commit_and_history_survives() {
        let path = root("handle-table-write");
        let schema = trade_schema();
        let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
        let mut table =
            Table::create(Folder::new(&path).unwrap(), FormatVersion::V2, schema, spec).unwrap();
        let options = crate::io::IOMedia::record_options(&table).unwrap();

        let batch = trades(
            &[1, 2],
            &[Some("AAPL"), Some("MSFT")],
            &[Some("XNAS"), Some("XNYS")],
        );
        table
            .append_arrow_reader(
                crate::arrow::batch_reader(batch.schema(), [batch]),
                &options,
            )
            .unwrap();
        let past = table.current_snapshot().unwrap().snapshot_id;
        let version = table.metadata_version();

        // No match key replaces every row; the snapshot it replaced is
        // retained and still reads exactly as it was written.
        let batch = trades(&[9], &[Some("BP")], &[Some("XLON")]);
        table
            .overwrite_arrow_reader(
                crate::arrow::batch_reader(batch.schema(), [batch]),
                &options,
            )
            .unwrap();
        assert_eq!(table.current_snapshot().unwrap().operation(), "overwrite");
        assert_eq!(table.metadata_version(), version + 1);
        assert_eq!(
            collect(table.read_arrow_reader(&options).unwrap()),
            vec![(9, Some("BP".to_owned()), Some("XLON".to_owned()))]
        );
        assert_eq!(collect(table.scan_at(past, &[], None).unwrap()).len(), 2);

        // A match key merges: `9` is stored and updates, `10` appends.
        let merging = options.clone().with_merge_by_names(["id"]);
        let batch = trades(
            &[9, 10],
            &[Some("BP.L"), Some("SHEL")],
            &[Some("XLON"), Some("XLON")],
        );
        table
            .merge_arrow_reader(
                crate::arrow::batch_reader(batch.schema(), [batch]),
                &merging,
            )
            .unwrap();
        assert_eq!(
            collect(table.read_arrow_reader(&options).unwrap()),
            vec![
                (9, Some("BP.L".to_owned()), Some("XLON".to_owned())),
                (10, Some("SHEL".to_owned()), Some("XLON".to_owned())),
            ]
        );

        // Reopening reads the same history this value already reports.
        let reopened = Table::open(Folder::new(&path).unwrap()).unwrap();
        assert_eq!(reopened.metadata_version(), table.metadata_version());
        assert_eq!(reopened.metadata().snapshots.len(), 3);
    }

    #[test]
    fn the_table_value_honours_the_row_limit_like_every_handle() {
        let path = root("handle-table-limit");
        let schema = trade_schema();
        let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
        let mut table =
            Table::create(Folder::new(&path).unwrap(), FormatVersion::V2, schema, spec).unwrap();
        let options = crate::io::IOMedia::record_options(&table).unwrap();

        // A limited write truncates data the caller offered, so only the
        // first row of the two lands in the commit.
        let batch = trades(
            &[1, 2],
            &[Some("AAPL"), Some("MSFT")],
            &[Some("XNAS"), Some("XNYS")],
        );
        table
            .overwrite_arrow_reader(
                crate::arrow::batch_reader(batch.schema(), [batch]),
                &options.clone().with_max_row_size(1),
            )
            .unwrap();
        assert_eq!(collect(table.read_arrow_reader(&options).unwrap()).len(), 1);

        // A limited read counts result rows, and `Some(0)` is a valid ask.
        let limited = options.clone().with_max_row_size(0);
        assert_eq!(collect(table.read_arrow_reader(&limited).unwrap()).len(), 0);

        // A limit combined with a match key is refused naming both settings,
        // on a table exactly as on a leaf.
        let merging = options
            .clone()
            .with_merge_by_names(["id"])
            .with_max_row_size(1);
        let batch = trades(&[3], &[Some("VOD")], &[Some("XLON")]);
        let Err(error) = table.merge_arrow_reader(
            crate::arrow::batch_reader(batch.schema(), [batch]),
            &merging,
        ) else {
            panic!("a limited merge must be refused");
        };
        let message = error.to_string();
        assert!(message.contains("max_row_size = 1"), "{message}");
        assert!(message.contains("merge_by_names [\"id\"]"), "{message}");
    }

    #[test]
    fn table_write_limit_preflight_does_not_pull_the_source() {
        let path = root("handle-table-limit-preflight");
        let schema = trade_schema();
        let mut table = Table::create(
            Folder::new(&path).unwrap(),
            FormatVersion::V2,
            schema.clone(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let options = crate::io::IOMedia::record_options(&table)
            .unwrap()
            .with_field(schema);

        let pulls = Arc::new(AtomicUsize::new(0));
        let batch = trades(&[1], &[Some("AAPL")], &[Some("XNAS")]);
        table
            .append_arrow_reader(
                counted(batch, Arc::clone(&pulls)),
                &options.clone().with_max_row_size(0),
            )
            .unwrap();
        assert_eq!(pulls.load(Ordering::SeqCst), 0);

        let batch = trades(&[1], &[Some("AAPL")], &[Some("XNAS")]);
        let message = table
            .merge_arrow_reader(
                counted(batch, Arc::clone(&pulls)),
                &options.with_merge_by_names(["id"]).with_max_byte_size(1),
            )
            .unwrap_err()
            .to_string();
        assert!(message.contains("merge_by_names"), "{message}");
        assert_eq!(pulls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn table_and_leaf_complete_unconvertible_input_onto_stored_fields_the_same_way() {
        let mut stored = DataType::from_fields([DataType::Int64.nullable_field("id")])
            .unwrap()
            .required_field("row");
        assign_field_ids(&mut stored, 1).unwrap();
        stored.insert_metadata("iceberg:schema-id", "0").unwrap();
        let valid = RecordBatch::try_new(
            crate::arrow::arrow_schema_from_field(&stored).unwrap(),
            vec![Arc::new(Int64Array::from(vec![Some(1)]))],
        )
        .unwrap();

        let path = root("handle-table-stored-completion");
        let mut table = Table::create(
            Folder::new(&path).unwrap(),
            FormatVersion::V2,
            stored.clone(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let declared_table = crate::io::IOMedia::record_options(&table)
            .unwrap()
            .with_field(stored.clone());
        table
            .overwrite_arrow_reader(
                crate::arrow::batch_reader(valid.schema(), [valid.clone()]),
                &declared_table,
            )
            .unwrap();

        let mut leaf = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
        let declared_leaf = leaf.record_options().unwrap().with_field(stored.clone());
        leaf.overwrite_arrow_reader(
            crate::arrow::batch_reader(valid.schema(), [valid]),
            &declared_leaf,
        )
        .unwrap();

        let loose = DataType::from_fields([DataType::Utf8.nullable_field("id")])
            .unwrap()
            .required_field("row");
        let bad = RecordBatch::try_new(
            crate::arrow::arrow_schema_from_field(&loose).unwrap(),
            vec![Arc::new(StringArray::from(vec![Some("not-an-integer")]))],
        )
        .unwrap();
        let untyped_table = crate::io::IOMedia::record_options(&table)
            .unwrap()
            .with_commit_row_size(1);
        table
            .overwrite_arrow_reader(
                crate::arrow::batch_reader(bad.schema(), [bad.clone()]),
                &untyped_table,
            )
            .unwrap();
        let untyped_leaf = leaf.record_options().unwrap().with_commit_row_size(1);
        leaf.overwrite_arrow_reader(
            crate::arrow::batch_reader(bad.schema(), [bad]),
            &untyped_leaf,
        )
        .unwrap();

        for mut reader in [
            table.read_arrow_reader(&declared_table).unwrap(),
            leaf.read_arrow_reader(&declared_leaf).unwrap(),
        ] {
            let batch = reader.next().unwrap().unwrap();
            let ids = batch
                .column_by_name("id")
                .unwrap()
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap();
            assert!(ids.is_null(0));
        }
    }

    #[test]
    fn table_commit_row_size_publishes_each_intent_at_the_requested_cadence() {
        let path = root("handle-table-commit-cadence");
        let schema = trade_schema();
        let mut table = Table::create(
            Folder::new(&path).unwrap(),
            FormatVersion::V2,
            schema.clone(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let options = crate::io::IOMedia::record_options(&table)
            .unwrap()
            .with_field(schema)
            .with_commit_row_size(1);

        let batch = trades(
            &[1, 2, 3],
            &[Some("AAPL"), Some("MSFT"), Some("VOD")],
            &[Some("XNAS"), Some("XNYS"), Some("XLON")],
        );
        table
            .overwrite_arrow_reader(
                crate::arrow::batch_reader(batch.schema(), [batch]),
                &options,
            )
            .unwrap();
        assert_eq!(table.metadata().snapshots.len(), 3);
        assert_eq!(collect(table.read_arrow_reader(&options).unwrap()).len(), 3);

        let batch = trades(
            &[4, 5],
            &[Some("BP"), Some("SHEL")],
            &[Some("XLON"), Some("XLON")],
        );
        table
            .append_arrow_reader(
                crate::arrow::batch_reader(batch.schema(), [batch]),
                &options,
            )
            .unwrap();
        assert_eq!(table.metadata().snapshots.len(), 5);

        let merging = options.clone().with_merge_by_names(["id"]);
        let batch = trades(
            &[2, 6],
            &[Some("MSFT.L"), Some("ARM")],
            &[Some("XNYS"), Some("XLON")],
        );
        table
            .merge_arrow_reader(
                crate::arrow::batch_reader(batch.schema(), [batch]),
                &merging,
            )
            .unwrap();
        assert_eq!(table.metadata().snapshots.len(), 7);
        assert_eq!(collect(table.read_arrow_reader(&options).unwrap()).len(), 6);
    }

    #[test]
    fn a_table_located_through_its_folder_keeps_the_same_commit_cadence() {
        let (path, mut folder) = table("handle-located-table-cadence");
        let options = options(&folder).with_commit_row_size(1);
        let batch = trades(
            &[1, 2, 3],
            &[Some("AAPL"), Some("MSFT"), Some("VOD")],
            &[Some("XNAS"), Some("XNYS"), Some("XLON")],
        );

        folder
            .overwrite_arrow_reader(
                crate::arrow::batch_reader(batch.schema(), [batch]),
                &options,
            )
            .unwrap();

        let reopened = Table::open(Folder::new(&path).unwrap()).unwrap();
        assert_eq!(reopened.metadata().snapshots.len(), 3);
        assert_eq!(
            collect(reopened.read_arrow_reader(&options).unwrap()).len(),
            3
        );
    }

    #[test]
    fn a_table_source_failure_leaves_the_committed_prefix_visible() {
        let path = root("handle-table-partial-commit");
        let schema = trade_schema();
        let mut table = Table::create(
            Folder::new(&path).unwrap(),
            FormatVersion::V2,
            schema.clone(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let options = crate::io::IOMedia::record_options(&table)
            .unwrap()
            .with_field(schema)
            .with_commit_row_size(1);
        let first = trades(&[7], &[Some("NVDA")], &[Some("XNAS")]);
        let reader = Box::new(RecordBatchIterator::new(
            [
                Ok(first.clone()),
                Err(ArrowError::ComputeError(
                    "later Iceberg source failure".into(),
                )),
            ],
            first.schema(),
        ));

        let message = table
            .overwrite_arrow_reader(reader, &options)
            .unwrap_err()
            .to_string();

        assert!(
            message.contains("later Iceberg source failure"),
            "{message}"
        );
        assert_eq!(table.metadata().snapshots.len(), 1);
        assert_eq!(
            collect(table.read_arrow_reader(&options).unwrap()),
            vec![(7, Some("NVDA".to_owned()), Some("XNAS".to_owned()))]
        );
        let reopened = Table::open(Folder::new(&path).unwrap()).unwrap();
        assert_eq!(reopened.metadata().snapshots.len(), 1);
    }

    #[test]
    fn empty_append_and_merge_create_no_table_commit() {
        let path = root("handle-table-empty-write");
        let schema = trade_schema();
        let mut table = Table::create(
            Folder::new(&path).unwrap(),
            FormatVersion::V2,
            schema.clone(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let options = crate::io::IOMedia::record_options(&table)
            .unwrap()
            .with_field(schema.clone());
        let arrow = crate::arrow::arrow_schema_from_field(&schema).unwrap();
        let version = table.metadata_version();
        let snapshots = table.metadata().snapshots.len();

        table
            .append_arrow_reader(crate::arrow::batch_reader(Arc::clone(&arrow), []), &options)
            .unwrap();
        let zero = RecordBatch::new_empty(arrow);
        table
            .merge_arrow_reader(
                crate::arrow::batch_reader(zero.schema(), [zero]),
                &options.with_merge_by_names(["id"]),
            )
            .unwrap();

        assert_eq!(table.metadata_version(), version);
        assert_eq!(table.metadata().snapshots.len(), snapshots);
    }
}

#[test]
fn time_travel_reads_a_previous_snapshot_by_id_and_by_ref() {
    let path = root("time-travel");
    let mut table = Table::create(
        Folder::new(&path).unwrap(),
        FormatVersion::V2,
        trade_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();

    let first = trades(&[1], &[Some("AAPL")], &[Some("XNAS")]);
    table
        .commit_append(crate::arrow::batch_reader(first.schema(), [first]))
        .unwrap();
    let past = table.current_snapshot().unwrap().snapshot_id;
    let second = trades(&[9], &[Some("NVDA")], &[Some("XNAS")]);
    table
        .commit_overwrite(crate::arrow::batch_reader(second.schema(), [second]))
        .unwrap();

    // The present shows the overwrite; the retained snapshot shows history.
    assert_eq!(collect(table.scan(None).unwrap())[0].0, 9);
    let history = collect(table.scan_at(past, &[], None).unwrap());
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].0, 1);

    // Planning history prunes exactly as planning the present does.
    let plan = table.plan_at(past, &[("venue", "XLON")]).unwrap();
    assert_eq!(plan.tasks.len(), 0);
    assert_eq!(table.plan_at(past, &[]).unwrap().tasks.len(), 1);

    // A tag names the snapshot, and an unknown ref says which refs exist.
    table
        .commit_metadata_changes(|metadata| {
            metadata.set_snapshot_ref("audit", crate::iceberg::SnapshotRef::tag(past))
        })
        .unwrap();
    assert_eq!(table.snapshot_by_ref("audit").unwrap().snapshot_id, past);
    let message = table.snapshot_by_ref("missing").unwrap_err().to_string();
    assert!(message.contains("audit"), "{message}");

    // A snapshot nobody retained is refused naming the ones that are.
    let message = match table.scan_at(-1, &[], None) {
        Err(error) => error.to_string(),
        Ok(_) => unreachable!("a snapshot nobody retained must not scan"),
    };
    assert!(message.contains("retained"), "{message}");

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn a_metadata_only_commit_writes_a_version_and_a_failure_leaves_none() {
    let path = root("metadata-commit");
    let mut table = Table::create(
        Folder::new(&path).unwrap(),
        FormatVersion::V2,
        trade_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    let version = table.metadata_version();

    table
        .commit_metadata_changes(|metadata| {
            metadata.set_property("owner", "desk")?;
            Ok(())
        })
        .unwrap();
    assert_eq!(table.metadata_version(), version + 1);
    assert_eq!(table.metadata().property("owner"), Some("desk"));

    // A rejected change is a commit that never happened.
    let failed: crate::Result<()> = table.commit_metadata_changes(|_| {
        Err(crate::Error::Codec {
            format: "iceberg",
            position: 0,
            reason: smol_str::SmolStr::new_static("rejected"),
        })
    });
    assert!(failed.is_err());
    assert_eq!(table.metadata_version(), version + 1);

    // The written document reads back with the change applied.
    let reopened = Table::open(Folder::new(&path).unwrap()).unwrap();
    assert_eq!(reopened.metadata().property("owner"), Some("desk"));
    assert_eq!(reopened.metadata_version(), version + 1);

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn a_reported_hint_failure_reconciles_to_the_version_fresh_handles_see() {
    let filesystem = Arc::new(PublishedHintFailure::default());
    let folder = crate::arrowfs::Folder::from_location(filesystem.clone(), "bucket/table").unwrap();
    let mut table = Table::create(
        folder.clone(),
        FormatVersion::V2,
        trade_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    let version = table.metadata_version();

    filesystem.arm();
    let error = table
        .commit_metadata_changes(|metadata| {
            metadata.set_property("owner", "desk")?;
            Ok(())
        })
        .unwrap_err();
    assert!(
        error.to_string().contains("acknowledgement failure"),
        "{error}"
    );

    // The error remains the backend's own answer, but in-memory state follows
    // the same discovery result any fresh handle observes. Keeping the prior
    // version here would make one object contradict the published table.
    assert_eq!(table.metadata_version(), version + 1);
    assert_eq!(table.metadata().property("owner"), Some("desk"));
    let reopened = Table::open(folder).unwrap();
    assert_eq!(reopened.metadata_version(), table.metadata_version());
    assert_eq!(reopened.metadata().property("owner"), Some("desk"));
}

#[test]
fn a_same_version_publication_conflict_rebases_through_the_retry_gate() {
    let filesystem = Arc::new(SameVersionWinner::default());
    let folder = crate::arrowfs::Folder::from_location(filesystem.clone(), "bucket/table").unwrap();
    let mut table = Table::create(
        folder.clone(),
        FormatVersion::V2,
        trade_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    table.set_options(
        IcebergOptions::new()
            .with_commit_retries(1)
            .with_commit_min_backoff_ms(0)
            .with_commit_max_backoff_ms(0),
    );

    filesystem.arm();
    let applications = AtomicUsize::new(0);
    table
        .commit_metadata_changes(|metadata| {
            applications.fetch_add(1, Ordering::Relaxed);
            metadata.set_property("loser", "visible")?;
            Ok(())
        })
        .unwrap();

    assert_eq!(filesystem.injections(), 1);
    assert_eq!(
        applications.load(Ordering::Relaxed),
        2,
        "the change must be reapplied to the competing winner"
    );
    assert_eq!(table.metadata_version(), 3);
    assert_eq!(table.metadata().property("winner"), Some("visible"));
    assert_eq!(table.metadata().property("loser"), Some("visible"));

    let reopened = Table::open(folder).unwrap();
    assert_eq!(reopened.metadata_version(), 3);
    assert_eq!(reopened.metadata().property("winner"), Some("visible"));
    assert_eq!(reopened.metadata().property("loser"), Some("visible"));
}

#[test]
fn the_inspection_tables_report_history_snapshots_and_files() {
    let path = root("inspect");
    let mut table = Table::create(
        Folder::new(&path).unwrap(),
        FormatVersion::V2,
        trade_schema(),
        PartitionSpec::identity(0, &trade_schema(), &["venue"]).unwrap(),
    )
    .unwrap();
    let first = trades(
        &[1, 2],
        &[Some("AAPL"), Some("MSFT")],
        &[Some("XNAS"), Some("XNYS")],
    );
    table
        .commit_append(crate::arrow::batch_reader(first.schema(), [first]))
        .unwrap();
    let second = trades(&[3], &[Some("NVDA")], &[Some("XNAS")]);
    table
        .commit_append(crate::arrow::batch_reader(second.schema(), [second]))
        .unwrap();

    // History: two snapshots, both on the current ancestry chain.
    let history: Vec<RecordBatch> = table
        .inspect_history()
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    assert_eq!(history[0].num_rows(), 2);
    let ancestor = history[0]
        .column_by_name("is_current_ancestor")
        .unwrap()
        .as_any()
        .downcast_ref::<arrow_array::BooleanArray>()
        .unwrap()
        .clone();
    assert!(ancestor.value(0) && ancestor.value(1));

    // Snapshots: the operation column reads straight off the summary.
    let snapshots: Vec<RecordBatch> = table
        .inspect_snapshots()
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    assert_eq!(snapshots[0].num_rows(), 2);
    let operations = snapshots[0]
        .column_by_name("operation")
        .unwrap()
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap()
        .clone();
    assert_eq!(operations.value(0), "append");

    // Files: three partitioned files, each naming its column=value chain.
    let files: Vec<RecordBatch> = table
        .inspect_files()
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    assert_eq!(files[0].num_rows(), 3);
    let partitions = files[0]
        .column_by_name("partition")
        .unwrap()
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap()
        .clone();
    let rendered: Vec<&str> = (0..3).map(|row| partitions.value(row)).collect();
    assert!(rendered.contains(&"venue=XNAS"), "{rendered:?}");
    assert!(rendered.contains(&"venue=XNYS"), "{rendered:?}");

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn a_commit_refuses_metadata_that_does_not_hold_together() {
    let path = root("invalid-commit");
    let mut table = Table::create(
        Folder::new(&path).unwrap(),
        FormatVersion::V2,
        trade_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    let version = table.metadata_version();

    // A change that leaves the metadata inconsistent never becomes a document.
    let failed = table.commit_metadata_changes(|metadata| {
        metadata.current_schema_id = 999;
        Ok(())
    });
    let message = failed.unwrap_err().to_string();
    assert!(message.contains("999"), "{message}");
    assert_eq!(table.metadata_version(), version);
    assert!(table.schema().is_ok());

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn a_zero_row_append_commits_a_snapshot_that_reads_as_nothing() {
    let path = root("zero-row");
    let mut table = Table::create(
        Folder::new(&path).unwrap(),
        FormatVersion::V2,
        trade_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();

    let empty = trades(&[], &[], &[]);
    table
        .commit_append(crate::arrow::batch_reader(empty.schema(), [empty]))
        .unwrap();

    // The commit is real - it has a snapshot - and the table stays empty.
    assert!(table.current_snapshot().is_some());
    assert_eq!(collect(table.scan(None).unwrap()).len(), 0);
    assert_eq!(table.plan(&[]).unwrap().tasks.len(), 0);

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn a_nan_value_neither_poisons_a_bound_nor_hides_a_row() {
    let path = root("nan-bounds");
    let mut schema = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Float64.nullable_field("ratio"),
    ])
    .unwrap()
    .required_field("row");
    assign_field_ids(&mut schema, 1).unwrap();
    let mut table = Table::create(
        Folder::new(&path).unwrap(),
        FormatVersion::V2,
        schema.clone(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();

    let arrow_schema = schema.into_arrow_schema().unwrap();
    let batch = RecordBatch::try_new(
        arrow_schema,
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3])),
            Arc::new(arrow_array::Float64Array::from(vec![
                1.5,
                f64::NAN,
                f64::INFINITY,
            ])),
        ],
    )
    .unwrap();
    table
        .commit_append(crate::arrow::batch_reader(batch.schema(), [batch]))
        .unwrap();

    // Every row reads back, and a filter on the finite value still finds it:
    // a NaN in the column must not produce a bound that excludes the file.
    let mut ids = Vec::new();
    for batch in table.scan(None).unwrap() {
        let batch = batch.unwrap();
        let column = batch
            .column_by_name("id")
            .unwrap()
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap()
            .clone();
        for row in 0..batch.num_rows() {
            ids.push(column.value(row));
        }
    }
    assert_eq!(ids, [1, 2, 3]);
    assert_eq!(table.plan(&[("ratio", "1.5")]).unwrap().tasks.len(), 1);

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn a_truncated_manifest_is_a_typed_error_and_not_a_panic() {
    let path = root("corrupt-manifest");
    let mut table = Table::create(
        Folder::new(&path).unwrap(),
        FormatVersion::V2,
        trade_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    let batch = trades(&[1], &[Some("AAPL")], &[Some("XNAS")]);
    table
        .commit_append(crate::arrow::batch_reader(batch.schema(), [batch]))
        .unwrap();

    // Truncate every Avro manifest under metadata/ to a torn prefix.
    for entry in std::fs::read_dir(path.join("metadata")).unwrap() {
        let file = entry.unwrap().path();
        if file
            .extension()
            .is_some_and(|extension| extension == "avro")
        {
            let bytes = std::fs::read(&file).unwrap();
            std::fs::write(&file, &bytes[..bytes.len().min(16)]).unwrap();
        }
    }

    let reopened = Table::open(Folder::new(&path).unwrap()).unwrap();
    let message = reopened.plan(&[]).unwrap_err().to_string();
    assert!(!message.is_empty());

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn a_tiny_write_target_rolls_one_append_into_multiple_data_files() {
    let path = root("target-size");
    let mut table = Table::create(
        Folder::new(&path).unwrap(),
        FormatVersion::V2,
        trade_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();

    let batches: Vec<RecordBatch> = (0..3)
        .map(|id| trades(&[id], &[Some("AAPL")], &[Some("XNAS")]))
        .collect();
    let arrow = batches[0].schema();

    // Under the 512 MiB default, three tiny batches land in one file.
    assert_eq!(
        table.target_file_size_bytes().unwrap(),
        512 * 1024 * 1024,
        "the default is Iceberg's own"
    );
    table
        .commit_append(crate::arrow::batch_reader(arrow.clone(), batches.clone()))
        .unwrap();
    assert_eq!(table.data_files().unwrap().len(), 1);

    // A one-byte target reaches the limit at every batch boundary, so the
    // same three batches become three files in the one partition.
    table
        .commit_metadata_changes(|metadata| {
            metadata.set_property("write.target-file-size-bytes", "1")?;
            Ok(())
        })
        .unwrap();
    assert_eq!(table.target_file_size_bytes().unwrap(), 1);
    table
        .commit_append(crate::arrow::batch_reader(arrow, batches))
        .unwrap();
    let files = table.data_files().unwrap();
    assert_eq!(files.len(), 4, "one whole file plus three rolled ones");

    // One running index numbers the commit's files, whatever partition each
    // lands in: the rolled commit wrote 00000, 00001, and 00002.
    let snapshot = table.current_snapshot().unwrap().snapshot_id;
    let mut indices: Vec<String> = files
        .iter()
        .filter_map(|(file, _)| {
            let name = file.file_path.rsplit('/').next()?;
            name.contains(&format!("-{snapshot}-"))
                .then(|| name.split('-').next().unwrap_or_default().to_owned())
        })
        .collect();
    indices.sort();
    assert_eq!(indices, ["00000", "00001", "00002"]);

    // However the rows were laid out, they all read back.
    assert_eq!(collect(table.scan(None).unwrap()).len(), 6);

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn the_schema_root_write_target_is_honored_when_the_table_property_is_absent() {
    let path = root("target-schema-root");
    let mut schema = trade_schema();
    schema
        .as_iceberg_mut()
        .insert("write.target-file-size-bytes", "1")
        .unwrap();
    let mut table = Table::create(
        Folder::new(&path).unwrap(),
        FormatVersion::V2,
        schema,
        PartitionSpec::unpartitioned(),
    )
    .unwrap();

    // No table property, so the schema root's `iceberg:` property decides.
    assert_eq!(table.target_file_size_bytes().unwrap(), 1);
    let batches: Vec<RecordBatch> = (0..2)
        .map(|id| trades(&[id], &[Some("AAPL")], &[Some("XNAS")]))
        .collect();
    let arrow = batches[0].schema();
    table
        .commit_append(crate::arrow::batch_reader(arrow.clone(), batches.clone()))
        .unwrap();
    assert_eq!(table.data_files().unwrap().len(), 2);

    // The moment the table property exists, it wins over the schema root.
    table
        .commit_metadata_changes(|metadata| {
            metadata.set_property("write.target-file-size-bytes", "1073741824")?;
            Ok(())
        })
        .unwrap();
    assert_eq!(table.target_file_size_bytes().unwrap(), 1 << 30);
    table
        .commit_append(crate::arrow::batch_reader(arrow, batches))
        .unwrap();
    assert_eq!(table.data_files().unwrap().len(), 3);

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn an_unparseable_write_target_is_a_typed_error_naming_the_key() {
    let path = root("target-unparseable");
    let mut table = Table::create(
        Folder::new(&path).unwrap(),
        FormatVersion::V2,
        trade_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    table
        .commit_metadata_changes(|metadata| {
            metadata.set_property("write.target-file-size-bytes", "512 MB")?;
            Ok(())
        })
        .unwrap();

    let error = table.target_file_size_bytes().unwrap_err();
    assert!(
        matches!(
            error,
            crate::Error::InvalidMetadataValue { ref key, .. }
                if key == "write.target-file-size-bytes"
        ),
        "{error:?}"
    );
    let message = error.to_string();
    assert!(
        message.contains("write.target-file-size-bytes"),
        "{message}"
    );
    assert!(message.contains("512 MB"), "{message}");
    assert!(message.contains("expected"), "{message}");

    // A present but unparseable target never silently becomes the default:
    // the write refuses rather than guessing.
    let batch = trades(&[1], &[Some("AAPL")], &[Some("XNAS")]);
    assert!(
        table
            .commit_append(crate::arrow::batch_reader(batch.schema(), [batch]))
            .is_err()
    );

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn compaction_merges_small_files_and_the_old_snapshot_still_time_travels() {
    let path = root("compact");
    let mut table = Table::create(
        Folder::new(&path).unwrap(),
        FormatVersion::V2,
        trade_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    for id in 0..5_i64 {
        let batch = trades(&[id], &[Some("AAPL")], &[Some("XNAS")]);
        table
            .commit_append(crate::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap();
    }
    let before_rows = collect(table.scan(None).unwrap());
    assert_eq!(table.data_files().unwrap().len(), 5);
    let past = table.current_snapshot().unwrap().snapshot_id;
    let snapshots_before = table.metadata().snapshots.len();

    let compaction = table.compact().unwrap();
    assert_eq!(compaction.files_before, 5);
    assert_eq!(compaction.files_after, 1);
    assert!(compaction.bytes_rewritten > 0);

    // Fewer files, identical rows, one `replace` snapshot.
    assert_eq!(table.data_files().unwrap().len(), 1);
    assert_eq!(collect(table.scan(None).unwrap()), before_rows);
    assert_eq!(table.metadata().snapshots.len(), snapshots_before + 1);
    assert_eq!(table.current_snapshot().unwrap().operation(), "replace");
    assert_eq!(
        table.metadata().snapshots.last().unwrap().operation(),
        "replace",
        "the retained snapshot view stays oldest first after official map normalization"
    );
    let inspected: Vec<RecordBatch> = table
        .inspect_snapshots()
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    let operations = inspected[0]
        .column_by_name("operation")
        .unwrap()
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert_eq!(operations.value(operations.len() - 1), "replace");

    // The pre-compaction snapshot is untouched, so time travel still reads
    // the five small files it names.
    assert_eq!(
        collect(table.scan_at(past, &[], None).unwrap()),
        before_rows
    );
    assert_eq!(table.plan_at(past, &[]).unwrap().tasks.len(), 5);

    // A second compaction has nothing to do: zeros, and no new snapshot.
    let version = table.metadata_version();
    assert_eq!(
        table.compact().unwrap(),
        super::table::Compaction::default()
    );
    assert_eq!(table.metadata_version(), version);
    assert_eq!(table.metadata().snapshots.len(), snapshots_before + 1);

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn compaction_respects_partitions_and_pruning_still_prunes_after_it() {
    let path = root("compact-partitions");
    let schema = trade_schema();
    let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
    let mut table =
        Table::create(Folder::new(&path).unwrap(), FormatVersion::V2, schema, spec).unwrap();

    // Two commits spanning two venues each: four small files, two per venue.
    for id in 0..2_i64 {
        let batch = trades(
            &[2 * id, 2 * id + 1],
            &[Some("AAPL"), Some("MSFT")],
            &[Some("XNAS"), Some("XNYS")],
        );
        table
            .commit_append(crate::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap();
    }
    // And one venue holding a single file, which no compaction may touch.
    let lone = trades(&[9], &[Some("VOD")], &[Some("XLON")]);
    table
        .commit_append(crate::arrow::batch_reader(lone.schema(), [lone]))
        .unwrap();
    assert_eq!(table.data_files().unwrap().len(), 5);
    let lone_path = table
        .data_files()
        .unwrap()
        .into_iter()
        .find(|(file, _)| file.partition[0] == Scalar::from("XLON"))
        .unwrap()
        .0
        .file_path;
    let before_rows = collect(table.scan(None).unwrap());

    let compaction = table.compact().unwrap();
    assert_eq!(compaction.files_before, 4, "the single-file group is kept");
    assert_eq!(compaction.files_after, 2, "one merged file per venue");

    // Files of different partitions never merged: each venue holds exactly
    // one file, in its own directory, and the lone file kept its location.
    let files = table.data_files().unwrap();
    assert_eq!(files.len(), 3);
    for venue in ["XNAS", "XNYS", "XLON"] {
        let held: Vec<_> = files
            .iter()
            .filter(|(file, _)| file.partition[0] == Scalar::from(venue))
            .collect();
        assert_eq!(held.len(), 1, "{venue}");
        assert!(
            held[0].0.file_path.contains(&format!("venue={venue}")),
            "{}",
            held[0].0.file_path
        );
    }
    assert!(
        files.iter().any(|(file, _)| file.file_path == lone_path),
        "the uncompacted file is carried, not rewritten"
    );
    assert_eq!(collect(table.scan(None).unwrap()), before_rows);

    // Pruning still works over the compacted layout: a venue filter opens one
    // file, skips the carried manifest outright, and reads the right rows.
    let plan = table.plan(&[("venue", "XNAS")]).unwrap();
    assert_eq!(plan.tasks.len(), 1);
    assert_eq!(plan.files_skipped(), 1, "the other merged venue's file");
    assert_eq!(plan.manifests_skipped(), 1, "the carried XLON manifest");
    assert_eq!(
        collect(table.scan_where(&[("venue", "XNAS")], None).unwrap()),
        vec![
            (0, Some("AAPL".to_owned()), Some("XNAS".to_owned())),
            (2, Some("AAPL".to_owned()), Some("XNAS".to_owned())),
        ]
    );

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn a_wide_schema_round_trips_with_every_field_numbered() {
    let path = root("wide");
    let mut schema = DataType::from_fields(
        (0..300).map(|index| DataType::Int64.nullable_field(format!("column_{index:03}"))),
    )
    .unwrap()
    .required_field("row");
    assign_field_ids(&mut schema, 1).unwrap();
    assert_eq!(schema.max_parquet_field_id().unwrap(), Some(300));

    let mut table = Table::create(
        Folder::new(&path).unwrap(),
        FormatVersion::V2,
        schema.clone(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    let arrow_schema = schema.into_arrow_schema().unwrap();
    let columns: Vec<ArrayRef> = (0..300)
        .map(|index| Arc::new(Int64Array::from(vec![index])) as ArrayRef)
        .collect();
    let batch = RecordBatch::try_new(arrow_schema, columns).unwrap();
    table
        .commit_append(crate::arrow::batch_reader(batch.schema(), [batch]))
        .unwrap();

    // Reopening parses the wide schema back and reads every column.
    let reopened = Table::open(Folder::new(&path).unwrap()).unwrap();
    assert_eq!(reopened.schema().unwrap().field_len(), 300);
    let read = reopened.scan(None).unwrap().next().unwrap().unwrap();
    assert_eq!(read.num_columns(), 300);
    assert_eq!(read.num_rows(), 1);

    let _ = std::fs::remove_dir_all(&path);
}

/// Collect every row of a scan as `(id, symbol, venue)` triples, in arrival
/// order.
///
/// [`collect`] sorts, which is right for tests that only care what a table
/// holds; the parallel-read tests care that two paths yield the same rows in
/// the same order, so this one never reorders.
fn ordered(reader: crate::arrow::BatchReader) -> Vec<(i64, Option<String>, Option<String>)> {
    let mut rows = Vec::new();
    for batch in reader {
        let batch = batch.unwrap();
        let ids = batch
            .column_by_name("id")
            .unwrap()
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap()
            .clone();
        let symbols = batch
            .column_by_name("symbol")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap()
            .clone();
        let venues = batch
            .column_by_name("venue")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap()
            .clone();
        for row in 0..batch.num_rows() {
            rows.push((
                ids.value(row),
                (!symbols.is_null(row)).then(|| symbols.value(row).to_owned()),
                (!venues.is_null(row)).then(|| venues.value(row).to_owned()),
            ));
        }
    }
    rows
}

#[test]
fn options_resolve_explicitly_then_by_property_then_by_default() {
    use super::IcebergOptions;

    let path = root("options-layers");
    let mut table = Table::create(
        Folder::new(&path).unwrap(),
        FormatVersion::V2,
        trade_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();

    // Nothing configured: every field answers its documented default.
    let options = table.options().unwrap();
    assert_eq!(options.commit_retries(), 4);
    assert_eq!(options.commit_min_backoff_ms(), 100);
    assert_eq!(options.commit_max_backoff_ms(), 60_000);
    assert_eq!(options.commit_total_timeout_ms(), 1_800_000);
    assert_eq!(options.target_file_size_bytes(), 512 * 1024 * 1024);
    assert!((1..=8).contains(&options.read_parallelism()));
    assert_eq!(options.read_parallel_min_files(), 16);
    assert_eq!(options.read_parallel_min_file_size_bytes(), 4 * 1024 * 1024);

    // A table property overrides the default.
    table
        .commit_metadata_changes(|metadata| {
            metadata.set_property(IcebergOptions::COMMIT_RETRIES_KEY, "7")?;
            metadata.set_property(IcebergOptions::COMMIT_TOTAL_TIMEOUT_MS_KEY, "7000")?;
            metadata.set_property(IcebergOptions::READ_PARALLEL_MIN_FILES_KEY, "3")?;
            Ok(())
        })
        .unwrap();
    let options = table.options().unwrap();
    assert_eq!(options.commit_retries(), 7);
    assert_eq!(options.commit_total_timeout_ms(), 7000);
    assert_eq!(options.read_parallel_min_files(), 3);
    assert_eq!(options.commit_min_backoff_ms(), 100, "untouched: default");

    // An explicit option overrides the property; unset fields still read it.
    table.set_options(IcebergOptions::new().with_commit_retries(2));
    let options = table.options().unwrap();
    assert_eq!(options.commit_retries(), 2, "explicit beats property");
    assert_eq!(
        options.read_parallel_min_files(),
        3,
        "property still speaks"
    );

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn zero_total_retry_budget_allows_a_zero_wait_rebase() {
    use super::IcebergOptions;

    let path = root("commit-total-timeout");
    let mut winner = Table::create(
        Folder::new(&path).unwrap(),
        FormatVersion::V2,
        trade_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    let mut stale = Table::open(Folder::new(&path).unwrap()).unwrap();
    winner.set_options(IcebergOptions::new().with_commit_total_timeout_ms(0));
    stale.set_options(
        IcebergOptions::new()
            .with_commit_retries(10)
            .with_commit_min_backoff_ms(0)
            .with_commit_total_timeout_ms(0),
    );

    winner
        .commit_metadata_changes(|metadata| {
            metadata.set_property("winner", "visible")?;
            Ok(())
        })
        .unwrap();
    stale
        .commit_metadata_changes(|metadata| {
            metadata.set_property("loser", "hidden")?;
            Ok(())
        })
        .unwrap();
    assert_eq!(stale.metadata_version(), 3);

    let reopened = Table::open(Folder::new(&path).unwrap()).unwrap();
    assert_eq!(reopened.metadata().property("winner"), Some("visible"));
    assert_eq!(reopened.metadata().property("loser"), Some("hidden"));

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn an_unparseable_option_property_is_typed_and_an_explicit_option_shadows_it() {
    use super::IcebergOptions;

    let path = root("options-unparseable");
    let mut table = Table::create(
        Folder::new(&path).unwrap(),
        FormatVersion::V2,
        trade_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    table
        .commit_metadata_changes(|metadata| {
            metadata.set_property(IcebergOptions::READ_PARALLELISM_KEY, "many")?;
            Ok(())
        })
        .unwrap();

    // The resolution is a typed error naming the key and the value.
    let error = table.options().unwrap_err();
    assert!(
        matches!(
            error,
            crate::Error::InvalidMetadataValue { ref key, .. } if key == "read.parallelism"
        ),
        "{error:?}"
    );
    let message = error.to_string();
    assert!(message.contains("read.parallelism"), "{message}");
    assert!(message.contains("many"), "{message}");
    assert!(message.contains("expected"), "{message}");

    // A scan consults the same key, so it refuses too rather than guessing.
    assert!(table.scan(None).is_err());

    // An explicit option shadows the broken property without ever parsing
    // it, which is what lets a caller repair the table.
    table.set_options(IcebergOptions::new().try_with_read_parallelism(2).unwrap());
    assert_eq!(table.options().unwrap().read_parallelism(), 2);
    assert_eq!(collect(table.scan(None).unwrap()).len(), 0);

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn a_beaten_append_rebases_and_keeps_both_writers_rows() {
    use super::IcebergOptions;

    let path = root("append-conflict");
    let mut first = Table::create(
        Folder::new(&path).unwrap(),
        FormatVersion::V2,
        trade_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    let mut second = Table::open(Folder::new(&path).unwrap()).unwrap();
    second.set_options(
        IcebergOptions::new()
            .with_commit_min_backoff_ms(1)
            .with_commit_max_backoff_ms(2),
    );
    assert_eq!(second.metadata_version(), 1);

    let winner = trades(&[1], &[Some("AAPL")], &[Some("XNAS")]);
    first
        .commit_append(crate::arrow::batch_reader(winner.schema(), [winner]))
        .unwrap();
    // The second handle still describes version 1, so its append is beaten
    // and must rebase onto the winner's commit rather than clobber it.
    let beaten = trades(&[2], &[Some("MSFT")], &[Some("XNYS")]);
    second
        .commit_append(crate::arrow::batch_reader(beaten.schema(), [beaten]))
        .unwrap();
    assert_eq!(
        second.metadata_version(),
        3,
        "the rebase adopted the winner's version"
    );

    // Both rows, two snapshots, and the loser's snapshot parents the winner's.
    let reopened = Table::open(Folder::new(&path).unwrap()).unwrap();
    assert_eq!(reopened.metadata_version(), 3);
    let rows = collect(reopened.scan(None).unwrap());
    assert_eq!(rows.iter().map(|row| row.0).collect::<Vec<_>>(), [1, 2]);
    assert_eq!(reopened.metadata().snapshots.len(), 2);
    let current = reopened.current_snapshot().unwrap();
    let parent = current.parent_snapshot_id.unwrap();
    let winner_snapshot = reopened.metadata().snapshot_by_id(parent).unwrap();
    assert_eq!(winner_snapshot.parent_snapshot_id, None);
    assert!(
        current.sequence_number.unwrap() > winner_snapshot.sequence_number.unwrap(),
        "the rebased commit sequences after the winner"
    );

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn concurrent_metadata_commits_rebase_and_both_changes_survive() {
    use super::IcebergOptions;

    let path = root("changes-conflict");
    let mut first = Table::create(
        Folder::new(&path).unwrap(),
        FormatVersion::V2,
        trade_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    let mut second = Table::open(Folder::new(&path).unwrap()).unwrap();
    second.set_options(
        IcebergOptions::new()
            .with_commit_min_backoff_ms(1)
            .with_commit_max_backoff_ms(2),
    );

    first
        .commit_metadata_changes(|metadata| {
            metadata.set_property("owner", "alpha")?;
            Ok(())
        })
        .unwrap();
    // The second commit is beaten, so its closure re-runs on the winner's
    // document - which is why both properties survive.
    second
        .commit_metadata_changes(|metadata| {
            metadata.set_property("team", "beta")?;
            Ok(())
        })
        .unwrap();

    let reopened = Table::open(Folder::new(&path).unwrap()).unwrap();
    assert_eq!(reopened.metadata_version(), 3);
    assert_eq!(reopened.metadata().property("owner"), Some("alpha"));
    assert_eq!(reopened.metadata().property("team"), Some("beta"));

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn a_beaten_overwrite_exhausts_its_retries_into_a_conflict_naming_versions() {
    use super::IcebergOptions;

    let path = root("overwrite-conflict");
    let mut first = Table::create(
        Folder::new(&path).unwrap(),
        FormatVersion::V2,
        trade_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    let stored = trades(&[1], &[Some("AAPL")], &[Some("XNAS")]);
    first
        .commit_append(crate::arrow::batch_reader(stored.schema(), [stored]))
        .unwrap();

    // The second handle plans against version 2; the first then commits twice
    // more, so the overwrite's plan is two commits stale.
    let mut second = Table::open(Folder::new(&path).unwrap()).unwrap();
    second.set_options(
        IcebergOptions::new()
            .with_commit_retries(1)
            .with_commit_min_backoff_ms(1)
            .with_commit_max_backoff_ms(1),
    );
    for id in [2_i64, 3] {
        let batch = trades(&[id], &[Some("NVDA")], &[Some("XNAS")]);
        first
            .commit_append(crate::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap();
    }

    let incoming = trades(&[9], &[Some("VOD")], &[Some("XLON")]);
    let error = second
        .commit_overwrite(crate::arrow::batch_reader(incoming.schema(), [incoming]))
        .unwrap_err();
    assert!(error.is_conflict(), "{error}");
    let message = error.to_string();
    assert!(
        message.contains("expected to commit version 3"),
        "{message}"
    );
    assert!(message.contains("got beaten 2 times"), "{message}");
    assert!(message.contains("last saw version 4"), "{message}");

    // The failed overwrite restored its handle and left no visible change.
    assert_eq!(second.metadata_version(), 2);
    let reopened = Table::open(Folder::new(&path).unwrap()).unwrap();
    assert_eq!(reopened.metadata_version(), 4);
    assert_eq!(
        collect(reopened.scan(None).unwrap())
            .iter()
            .map(|row| row.0)
            .collect::<Vec<_>>(),
        [1, 2, 3],
        "the winner's rows survive and the loser's were never published"
    );

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn a_parallel_read_yields_the_sequential_rows_in_the_sequential_order() {
    use super::IcebergOptions;

    let path = root("parallel-read");
    let mut table = Table::create(
        Folder::new(&path).unwrap(),
        FormatVersion::V2,
        trade_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    // Twenty commits of three rows each: twenty data files, ids 0..60 laid
    // down in commit order, which is the order a sequential scan yields.
    for file in 0..20_i64 {
        let symbol = if file % 2 == 0 { "AAPL" } else { "MSFT" };
        let batch = trades(
            &[3 * file, 3 * file + 1, 3 * file + 2],
            &[Some(symbol); 3],
            &[Some("XNAS"); 3],
        );
        table
            .commit_append(crate::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap();
    }

    let sequential = IcebergOptions::new().try_with_read_parallelism(1).unwrap();
    let parallel = IcebergOptions::new()
        .try_with_read_parallelism(3)
        .unwrap()
        .with_read_parallel_min_files(1)
        .with_read_parallel_min_file_size_bytes(0);

    table.set_options(sequential);
    let baseline = ordered(table.scan(None).unwrap());
    assert_eq!(baseline.len(), 60);
    assert_eq!(
        baseline.iter().map(|row| row.0).collect::<Vec<_>>(),
        (0..60).collect::<Vec<_>>(),
        "the sequential scan reads the files in plan order"
    );
    let filtered_baseline = ordered(table.scan_where(&[("symbol", "AAPL")], None).unwrap());
    assert_eq!(filtered_baseline.len(), 30);

    // The parallel path must be indistinguishable, row for row and in order.
    table.set_options(parallel.clone());
    assert_eq!(ordered(table.scan(None).unwrap()), baseline);
    assert_eq!(
        ordered(table.scan_where(&[("symbol", "AAPL")], None).unwrap()),
        filtered_baseline
    );

    // Dropping a parallel reader mid-stream neither hangs nor poisons the
    // table: the detached workers exit at their next send.
    let mut abandoned = table.scan(None).unwrap();
    assert!(abandoned.next().is_some());
    drop(abandoned);
    assert_eq!(ordered(table.scan(None).unwrap()), baseline);

    // Below the file threshold the sequential single-open path answers,
    // behaviorally identical.
    table.set_options(
        IcebergOptions::new()
            .try_with_read_parallelism(3)
            .unwrap()
            .with_read_parallel_min_files(1_000),
    );
    assert_eq!(ordered(table.scan(None).unwrap()), baseline);

    // The default 4 MiB floor also keeps these tiny files sequential: none
    // of them counts toward justifying threads.
    table.set_options(
        IcebergOptions::new()
            .try_with_read_parallelism(3)
            .unwrap()
            .with_read_parallel_min_files(1),
    );
    assert_eq!(ordered(table.scan(None).unwrap()), baseline);

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn branches_and_tags_round_trip_through_table_level_commits() {
    let path = root("table-refs");
    let mut table = Table::create(
        Folder::new(&path).unwrap(),
        FormatVersion::V2,
        trade_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    let first = trades(&[1], &[Some("AAPL")], &[Some("XNAS")]);
    table
        .commit_append(crate::arrow::batch_reader(first.schema(), [first]))
        .unwrap();
    let past = table.current_snapshot().unwrap().snapshot_id;

    table.create_branch("audit", past).unwrap();
    table.create_tag("v1", past).unwrap();
    let version = table.metadata_version();

    // A taken name is refused and the refusal commits nothing.
    assert!(table.create_branch("audit", past).is_err());
    assert_eq!(table.metadata_version(), version);

    let second = trades(&[2], &[Some("MSFT")], &[Some("XNYS")]);
    table
        .commit_append(crate::arrow::batch_reader(second.schema(), [second]))
        .unwrap();
    let head = table.current_snapshot().unwrap().snapshot_id;

    // The branch and the tag still read the past; main reads the present.
    assert_eq!(
        collect(table.scan_ref("audit", &[], None).unwrap()).len(),
        1
    );
    assert_eq!(collect(table.scan_ref("v1", &[], None).unwrap()).len(), 1);
    assert_eq!(collect(table.scan_ref("main", &[], None).unwrap()).len(), 2);

    // Fast-forwarding moves the branch to a descendant; the tag never moves.
    table.fast_forward_branch("audit", head).unwrap();
    assert_eq!(
        collect(table.scan_ref("audit", &[], None).unwrap()).len(),
        2
    );
    assert_eq!(table.snapshot_by_ref("v1").unwrap().snapshot_id, past);

    // Removing the tag returns it; a second removal names what exists.
    let removed = table.remove_snapshot_ref("v1").unwrap();
    assert!(removed.is_tag());
    assert_eq!(removed.snapshot_id, past);
    let message = table.remove_snapshot_ref("v1").unwrap_err().to_string();
    assert!(message.contains("audit"), "{message}");

    // With nothing anchoring the first snapshot, expiring everything older
    // than the near future removes exactly it - and the table still reads.
    let cutoff = table.metadata().last_updated_ms + 10_000;
    let expired = table.expire_snapshots(Some(cutoff), None, &[]).unwrap();
    assert_eq!(expired, vec![past]);
    assert_eq!(table.metadata().snapshots.len(), 1);
    assert_eq!(collect(table.scan(None).unwrap()).len(), 2);
    assert!(table.scan_at(past, &[], None).is_err());

    // Nothing left to expire commits nothing: no new version is written.
    let version = table.metadata_version();
    assert_eq!(
        table.expire_snapshots(Some(i64::MAX), None, &[]).unwrap(),
        Vec::<i64>::new()
    );
    assert_eq!(table.metadata_version(), version);

    let _ = std::fs::remove_dir_all(&path);
}

mod datatype_coverage {
    //! Every mapped Iceberg type round-trips through append, scan, and merge.

    use std::sync::Arc;

    use super::*;
    use crate::arrow::value::array_from_values;
    use crate::{Scalar, TimeUnit};

    /// Append `rows` under `children`, scan them back, and return the records.
    fn round_trip(label: &str, children: Vec<Field>, rows: &[Vec<Scalar>]) -> Vec<Scalar> {
        let path = root(label);
        let schema = DataType::from_fields(children.clone())
            .unwrap()
            .required_field("row");
        let mut table = Table::create(
            Folder::new(&path).unwrap(),
            FormatVersion::V2,
            schema.clone(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();

        let columns: Vec<_> = children
            .iter()
            .enumerate()
            .map(|(index, child)| {
                let column: Vec<&Scalar> = rows.iter().map(|row| &row[index]).collect();
                array_from_values(child, &column).unwrap()
            })
            .collect();
        let batch = arrow_array::RecordBatch::try_new(schema.into_arrow_schema().unwrap(), columns)
            .unwrap();
        table
            .commit_append(crate::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap();

        let reopened = Table::open(Folder::new(&path).unwrap()).unwrap();
        let mut records = Vec::new();
        for batch in reopened.scan(None).unwrap() {
            let value = crate::arrow::batch_to_value(&batch.unwrap()).unwrap();
            records.extend(value.as_sequence().unwrap().iter().cloned());
        }
        records
    }

    #[test]
    fn every_v2_primitive_round_trips_through_a_data_file() {
        let children = vec![
            DataType::Boolean.nullable_field("flag"),
            DataType::Int32.nullable_field("small"),
            DataType::Int64.required_field("id"),
            DataType::Float32.nullable_field("ratio"),
            DataType::Float64.nullable_field("value"),
            DataType::Decimal128 {
                precision: 18,
                scale: 4,
            }
            .nullable_field("price"),
            DataType::Date32.nullable_field("day"),
            DataType::Time64(TimeUnit::Microsecond).nullable_field("tod"),
            DataType::Timestamp(TimeUnit::Microsecond, None).nullable_field("at"),
            DataType::Utf8.nullable_field("name"),
            DataType::Binary.nullable_field("raw"),
            DataType::FixedSizeBinary(4).nullable_field("tag"),
        ];
        let rows = vec![
            vec![
                Scalar::Bool(true),
                Scalar::I64(41),
                Scalar::I64(1),
                Scalar::from(0.5_f32),
                Scalar::from(2.25_f64),
                Scalar::d128(1_500_000, 4),
                Scalar::date32(20_000),
                Scalar::time64(
                    43_200_000_000,
                    TimeUnit::Microsecond,
                    crate::Timezone::NAIVE,
                )
                .unwrap(),
                Scalar::datetime64(
                    1_700_000_000_000_000,
                    TimeUnit::Microsecond,
                    crate::Timezone::NAIVE,
                )
                .unwrap(),
                Scalar::String("alpha".into()),
                Scalar::from(vec![1_u8, 2]),
                Scalar::from(vec![9_u8, 9, 9, 9]),
            ],
            // A row of nulls proves every column's null path through Parquet.
            vec![
                Scalar::Null,
                Scalar::Null,
                Scalar::I64(2),
                Scalar::Null,
                Scalar::Null,
                Scalar::Null,
                Scalar::Null,
                Scalar::Null,
                Scalar::Null,
                Scalar::Null,
                Scalar::Null,
                Scalar::Null,
            ],
        ];

        let records = round_trip("types-primitive", children, &rows);
        assert_eq!(records.len(), 2);
        let first = records[0].as_sequence().unwrap();
        assert_eq!(first[0], Scalar::Bool(true));
        assert_eq!(first[2], Scalar::I64(1));
        assert_eq!(first[5], Scalar::d128(1_500_000, 4));
        assert_eq!(
            first[8],
            Scalar::datetime64(
                1_700_000_000_000_000,
                TimeUnit::Microsecond,
                crate::Timezone::NAIVE,
            )
            .unwrap()
        );
        assert_eq!(first[11], Scalar::from(vec![9_u8, 9, 9, 9]));
        let second = records[1].as_sequence().unwrap();
        assert_eq!(second[0], Scalar::Null);
        assert_eq!(second[5], Scalar::Null);
    }

    #[test]
    fn nested_and_deeply_nested_shapes_round_trip_through_data_files() {
        let point = DataType::from_fields([
            DataType::Int64.required_field("x"),
            DataType::Utf8.nullable_field("label"),
        ])
        .unwrap();
        let deep = DataType::from_fields([
            DataType::List(Arc::new(DataType::Int64.nullable_field("item"))).nullable_field("xs"),
            DataType::map_of(DataType::Utf8, point.clone(), false)
                .unwrap()
                .nullable_field("m"),
        ])
        .unwrap();
        let children = vec![
            DataType::Int64.required_field("id"),
            point.clone().nullable_field("p"),
            DataType::List(Arc::new(deep.clone().nullable_field("item"))).nullable_field("rows"),
        ];

        let point_value = |x: i64, label: &str| {
            Scalar::from_sequence([Scalar::I64(x), Scalar::String(label.into())])
        };
        let deep_value = Scalar::from_sequence([
            Scalar::from_sequence([Scalar::I64(1), Scalar::Null, Scalar::I64(3)]),
            Scalar::from_mapping([(Scalar::from("origin"), point_value(0, "o"))]).unwrap(),
        ]);
        let rows = vec![vec![
            Scalar::I64(1),
            point_value(7, "seven"),
            Scalar::from_sequence([deep_value]),
        ]];

        let records = round_trip("types-nested", children, &rows);
        assert_eq!(records.len(), 1);
        let values = records[0].as_sequence().unwrap();
        // The struct survives with both children.
        match &values[1] {
            Scalar::Sequence(fields) => {
                assert_eq!(fields[0], Scalar::I64(7));
                assert_eq!(fields[1], Scalar::String("seven".into()));
            }
            other => panic!("expected a struct row back, got {}", other.kind()),
        }
        // The deep list<struct<list, map<utf8, struct>>> survives one level in.
        let outer = values[2].as_sequence().expect("a list of deep rows");
        assert_eq!(outer.len(), 1);
    }

    #[test]
    fn a_merge_updates_on_a_composite_key_of_mixed_types() {
        let path = root("types-merge");
        let children = vec![
            DataType::Int64.required_field("id"),
            DataType::Utf8.required_field("venue"),
            DataType::Decimal128 {
                precision: 18,
                scale: 4,
            }
            .nullable_field("price"),
        ];
        let schema = DataType::from_fields(children.clone())
            .unwrap()
            .required_field("row");
        let mut table = Table::create(
            Folder::new(&path).unwrap(),
            FormatVersion::V2,
            schema.clone(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();

        let batch_of = |rows: &[(i64, &str, i128)]| {
            let columns: Vec<Vec<Scalar>> = (0..3)
                .map(|index| {
                    rows.iter()
                        .map(|(id, venue, price)| match index {
                            0 => Scalar::I64(*id),
                            1 => Scalar::String((*venue).into()),
                            _ => Scalar::d128(*price, 4),
                        })
                        .collect()
                })
                .collect();
            let arrays: Vec<_> = children
                .iter()
                .zip(columns.iter())
                .map(|(child, column): (&Field, &Vec<Scalar>)| {
                    let refs: Vec<&Scalar> = column.iter().collect();
                    array_from_values(child, &refs).unwrap()
                })
                .collect();
            arrow_array::RecordBatch::try_new(schema.clone().into_arrow_schema().unwrap(), arrays)
                .unwrap()
        };

        let first = batch_of(&[(1, "XNAS", 10_000), (2, "XNYS", 20_000)]);
        table
            .commit_append(crate::arrow::batch_reader(first.schema(), [first]))
            .unwrap();
        // The same (id, venue) updates; a new pair appends.
        let second = batch_of(&[(1, "XNAS", 99_000), (3, "XPAR", 30_000)]);
        table
            .commit_merge(
                crate::arrow::batch_reader(second.schema(), [second]),
                &["id".to_owned(), "venue".to_owned()],
                false,
            )
            .unwrap();

        let mut prices = std::collections::BTreeMap::new();
        for batch in table.scan(None).unwrap() {
            let value = crate::arrow::batch_to_value(&batch.unwrap()).unwrap();
            for row in value.as_sequence().unwrap() {
                let fields = row.as_sequence().unwrap();
                let Scalar::I64(id) = fields[0] else { panic!() };
                prices.insert(id, fields[2].clone());
            }
        }
        assert_eq!(prices.len(), 3);
        assert_eq!(prices[&1], Scalar::d128(99_000, 4));
        assert_eq!(prices[&3], Scalar::d128(30_000, 4));
    }
}

mod concurrency_and_compaction {
    //! Real racing writers, a beaten merge, and the compaction cadence.

    use super::*;

    #[test]
    fn stale_threads_rebase_and_every_writer_lands() {
        let path = root("threads");
        let schema = trade_schema();
        Table::create(
            Folder::new(&path).unwrap(),
            FormatVersion::V2,
            schema,
            PartitionSpec::unpartitioned(),
        )
        .unwrap();

        // Every thread opens its handle at version 1, so every commit after
        // the first is beaten and must rebase. The lock serializes only the
        // publish - plain storage has no compare-and-swap, so unserialized
        // metadata writes can tear, exactly as the commit documentation says -
        // which leaves the part under test deterministic: stale handles,
        // real threads, and the rebase that reconciles them.
        let gate = std::sync::Arc::new(std::sync::Mutex::new(()));
        let opened = std::sync::Arc::new(std::sync::Barrier::new(4));
        let handles: Vec<_> = (0..4)
            .map(|writer: i64| {
                let path = path.clone();
                let gate = std::sync::Arc::clone(&gate);
                let opened = std::sync::Arc::clone(&opened);
                std::thread::spawn(move || {
                    let mut table = Table::open(Folder::new(&path).unwrap()).unwrap();
                    opened.wait();
                    let batch = trades(
                        &[writer * 10, writer * 10 + 1],
                        &[Some("S"), Some("S")],
                        &[Some("V"), Some("V")],
                    );
                    let _held = gate.lock().unwrap();
                    table
                        .commit_append(crate::arrow::batch_reader(batch.schema(), [batch]))
                        .unwrap();
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }

        let reopened = Table::open(Folder::new(&path).unwrap()).unwrap();
        let mut ids: Vec<i64> = collect(reopened.scan(None).unwrap())
            .into_iter()
            .map(|(id, _, _)| id)
            .collect();
        ids.sort_unstable();
        assert_eq!(ids, [0, 1, 10, 11, 20, 21, 30, 31]);
        // Four commits landed on top of the created table.
        assert_eq!(reopened.metadata_version(), 5);
    }

    #[test]
    fn a_beaten_merge_reports_the_conflict_rather_than_rebasing() {
        let path = root("beaten-merge");
        let schema = trade_schema();
        let mut writer = Table::create(
            Folder::new(&path).unwrap(),
            FormatVersion::V2,
            schema,
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let seed = trades(&[1, 2], &[Some("A"), Some("B")], &[Some("V"), Some("V")]);
        writer
            .commit_append(crate::arrow::batch_reader(seed.schema(), [seed]))
            .unwrap();

        // A second handle grows stale the moment the first commits again.
        let mut stale = Table::open(Folder::new(&path).unwrap()).unwrap();
        stale.set_options(crate::iceberg::IcebergOptions::default().with_commit_retries(1));
        let win = trades(&[3], &[Some("C")], &[Some("V")]);
        writer
            .commit_append(crate::arrow::batch_reader(win.schema(), [win]))
            .unwrap();

        let incoming = trades(&[2], &[Some("B2")], &[Some("V")]);
        let error = stale
            .commit_merge(
                crate::arrow::batch_reader(incoming.schema(), [incoming]),
                &["id".to_owned()],
                false,
            )
            .expect_err("a beaten merge cannot rebase");
        assert!(
            error.to_string().contains("got beaten"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn the_cadence_compacts_after_every_n_data_commits_by_itself() {
        let path = root("auto-compact");
        let schema = trade_schema();
        let mut table = Table::create(
            Folder::new(&path).unwrap(),
            FormatVersion::V2,
            schema,
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        table.set_options(crate::iceberg::IcebergOptions::default().with_compact_after_commits(2));

        for id in 0..4_i64 {
            let batch = trades(&[id], &[Some("S")], &[Some("V")]);
            table
                .commit_append(crate::arrow::batch_reader(batch.schema(), [batch]))
                .unwrap();
        }

        // Two cadence points passed, so replace snapshots appear on their own
        // and the live file count shrank below one-per-append.
        let operations: Vec<String> = table
            .metadata()
            .snapshots
            .iter()
            .map(|snapshot| snapshot.operation().to_owned())
            .collect();
        let replaces = operations.iter().filter(|op| *op == "replace").count();
        assert!(
            replaces >= 1,
            "no automatic compaction ran; operations: {operations:?}"
        );
        assert!(table.data_files().unwrap().len() < 4);
        // And nothing was lost along the way.
        assert_eq!(collect(table.scan(None).unwrap()).len(), 4);
    }
}

mod line_projection {
    use arrow_array::{BinaryArray, Int64Array, StringArray};

    use super::*;
    use crate::Url;
    use crate::generic::{IORecordOptions, RecordOptions};
    use crate::io::{Buffer, IOMedia};
    use crate::text::TextOptions;

    const HEADER: &str = r"^\[(?<level>[A-Z]+)\] id=(?<id>\d+)\s*";

    fn named(bytes: &[u8]) -> Buffer {
        let mut handle = Buffer::new()
            .with_media_type(Url::from_str("file:///events.log").unwrap().media_type());
        handle.write_all_bytes(bytes).unwrap();
        handle
    }

    #[test]
    fn adaptive_text_rows_stream_into_a_table_through_record_media() {
        let path = root("text-record-media");
        let source = named(b"[INFO] id=7 first  \n[WARN] id=42 second\n");
        let mut options: RecordOptions = TextOptions::new()
            .try_with_rowheader(HEADER)
            .unwrap()
            .try_with_rstrip(r"\s+$")
            .unwrap()
            .into();
        options.set_batch_row_size(Some(2));

        let mut schema = source.read_arrow_field(&options).unwrap();
        assert_eq!(
            schema.get_field_by_path("id").unwrap().dtype(),
            &crate::DataType::Int64
        );
        schema.assign_parquet_field_ids(1).unwrap();

        let catalog = super::super::Catalog::new(Folder::new(path.join("warehouse")).unwrap());
        catalog.tables().create("logs.events", schema).unwrap();
        let table = catalog
            .tables()
            .append_arrow_reader("logs.events", source.read_arrow_reader(&options).unwrap())
            .unwrap();

        let batches = table
            .scan(None)
            .unwrap()
            .collect::<std::result::Result<Vec<RecordBatch>, _>>()
            .unwrap();
        assert_eq!(batches.iter().map(RecordBatch::num_rows).sum::<usize>(), 2);
        let batch = &batches[0];
        let ids = batch
            .column_by_name("id")
            .unwrap()
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        let levels = batch
            .column_by_name("level")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let bodies = batch
            .column_by_name("body")
            .unwrap()
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap();
        assert_eq!(ids.values(), &[7, 42]);
        assert_eq!(
            levels.iter().collect::<Vec<_>>(),
            [Some("INFO"), Some("WARN")]
        );
        assert_eq!(
            bodies.iter().collect::<Vec<_>>(),
            [Some(&b"first"[..]), Some(&b"second"[..])]
        );
        assert_eq!(
            table
                .current_snapshot()
                .unwrap()
                .summary_value("added-records"),
            Some("2")
        );

        let _ = std::fs::remove_dir_all(path);
    }
}

mod manifest_planning {
    use super::super::manifest::read_manifest_for_plan;
    use super::super::{
        DataFile, FormatVersion, ManifestEntry, PartitionSpec, read_manifest, write_manifest,
    };
    use super::trade_schema;
    use crate::Scalar;
    use crate::io::{Buffer, IOBase};

    /// One entry carrying every statistic a manifest can record.
    fn full_entry(index: i64) -> ManifestEntry {
        ManifestEntry::added(
            7_001,
            DataFile {
                file_path: smol_str::format_smolstr!("file:///t/data/part-{index}.parquet"),
                partition: vec![Scalar::from("XNAS")],
                record_count: 100 + index,
                file_size_in_bytes: 4_096,
                column_sizes: vec![(1, 512), (2, 256)],
                value_counts: vec![(1, 100), (2, 90)],
                null_value_counts: vec![(1, 0), (2, 10)],
                nan_value_counts: vec![(1, 0)],
                lower_bounds: vec![(1, 1_i64.to_le_bytes().to_vec())],
                upper_bounds: vec![(1, 9_i64.to_le_bytes().to_vec())],
                split_offsets: vec![4],
                sort_order_id: Some(0),
                ..DataFile::default()
            },
        )
    }

    fn stored() -> Buffer {
        let schema = trade_schema();
        let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
        let mut handle = Buffer::new();
        write_manifest(
            &mut handle,
            FormatVersion::V2,
            &schema,
            &spec,
            &[full_entry(0), full_entry(1)],
        )
        .unwrap();
        handle
    }

    #[test]
    fn the_planning_fast_path_agrees_on_every_field_it_decodes() {
        let handle = stored();
        let full = read_manifest(&handle).unwrap();
        let pruned = read_manifest_for_plan(&handle, true).unwrap();
        assert_eq!(full.len(), pruned.len());
        for (full, pruned) in full.iter().zip(&pruned) {
            assert_eq!(full.status, pruned.status);
            assert_eq!(full.snapshot_id, pruned.snapshot_id);
            assert_eq!(full.sequence_number, pruned.sequence_number);
            assert_eq!(full.data_file.file_path, pruned.data_file.file_path);
            assert_eq!(full.data_file.partition, pruned.data_file.partition);
            assert_eq!(full.data_file.record_count, pruned.data_file.record_count);
            assert_eq!(
                full.data_file.file_size_in_bytes,
                pruned.data_file.file_size_in_bytes
            );
            // A filtered plan keeps what pruning consults...
            assert_eq!(full.data_file.value_counts, pruned.data_file.value_counts);
            assert_eq!(
                full.data_file.null_value_counts,
                pruned.data_file.null_value_counts
            );
            assert_eq!(full.data_file.lower_bounds, pruned.data_file.lower_bounds);
            assert_eq!(full.data_file.upper_bounds, pruned.data_file.upper_bounds);
            // ...and skips what it never reads.
            assert!(pruned.data_file.column_sizes.is_empty());
            assert!(pruned.data_file.nan_value_counts.is_empty());
            assert!(pruned.data_file.split_offsets.is_empty());
        }
    }

    #[test]
    fn an_unfiltered_plan_skips_the_statistics_maps_entirely() {
        let handle = stored();
        let pruned = read_manifest_for_plan(&handle, false).unwrap();
        assert_eq!(pruned.len(), 2);
        assert!(pruned[0].data_file.value_counts.is_empty());
        assert!(pruned[0].data_file.lower_bounds.is_empty());
        assert_eq!(pruned[0].data_file.record_count, 100);
        assert_eq!(pruned[0].data_file.partition, vec![Scalar::from("XNAS")]);
    }

    #[test]
    fn writing_the_same_manifest_twice_produces_the_same_bytes() {
        let first = stored().read_all_bytes().unwrap();
        let second = stored().read_all_bytes().unwrap();
        assert_eq!(
            first, second,
            "a manifest writer must be a pure function of its input"
        );
    }

    #[test]
    fn an_avro_container_without_required_iceberg_metadata_is_not_a_manifest() {
        // Avro named-type references are valid container syntax, but a real
        // Iceberg manifest must also carry its schema and partition metadata.
        // The official parser owns that distinction for both planning modes.
        let schema = crate::json::from_utf8(
            r#"{"type":"record","name":"manifest_entry","fields":[
                {"name":"status","type":"int"},
                {"name":"snapshot_id","type":["null","long"],"default":null},
                {"name":"data_file","type":{"type":"record","name":"r2","fields":[
                    {"name":"file_path","type":"string"},
                    {"name":"column_sizes","type":["null",{"type":"array","items":
                        {"type":"record","name":"kv","fields":[
                            {"name":"key","type":"int"},
                            {"name":"value","type":"long"}
                        ]}}],"default":null},
                    {"name":"value_counts","type":["null",{"type":"array","items":"kv"}],
                     "default":null}
                ]}}
            ]}"#,
        )
        .unwrap();
        let row = crate::json::from_utf8(
            r#"{"status":1,"snapshot_id":77,"data_file":{
                "file_path":"file:///t/data/part-0.parquet",
                "column_sizes":[{"key":1,"value":512}],
                "value_counts":[{"key":1,"value":100}]}}"#,
        )
        .unwrap();
        let mut handle = Buffer::new();
        crate::avro::write_container(&mut handle, &schema, &[], &[row]).unwrap();

        for statistics in [false, true] {
            let message = read_manifest_for_plan(&handle, statistics)
                .unwrap_err()
                .to_string();
            assert!(
                message.contains("schema is required in manifest metadata"),
                "{message}"
            );
        }
    }
}

/// The data-file MIME type: option, property, default, and mixing.
mod data_mime_type {
    use super::*;
    use crate::MimeType;
    use crate::iceberg::IcebergOptions;

    /// The manifests' `(mime_type, path)` pairs of the current snapshot.
    fn formats(table: &Table<Folder>) -> Vec<(MimeType, String)> {
        table
            .data_files()
            .unwrap()
            .into_iter()
            .map(|(file, _)| (file.mime_type, file.file_path.to_string()))
            .collect()
    }

    #[test]
    fn options_accept_only_iceberg_mime_types_atomically() {
        let mut options = IcebergOptions::new();
        let before = options.clone();
        assert!(options.set_data_mime_type(MimeType::JSON).is_err());
        assert_eq!(options, before);

        let options = options
            .try_with_data_mime_type(MimeType::from_str("application/avro").unwrap())
            .unwrap();
        assert_eq!(options.data_mime_type(), MimeType::AVRO);
    }

    #[test]
    fn the_default_mime_type_is_parquet_and_the_option_layers_resolve() {
        let path = root("format-layers");
        let mut table = Table::create(
            Folder::new(&path).unwrap(),
            FormatVersion::V2,
            trade_schema(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();

        // Nothing configured: the default is the spec's own, Parquet.
        assert_eq!(table.options().unwrap().data_mime_type(), MimeType::PARQUET);

        // The table property layer, under the spec's own key, in the spec's
        // own lowercase spelling.
        table
            .commit_metadata_changes(|metadata| {
                metadata
                    .set_property("write.format.default", "avro")
                    .map(|_| ())
            })
            .unwrap();
        assert_eq!(table.options().unwrap().data_mime_type(), MimeType::AVRO);

        // The explicit option shadows the property.
        table.set_options(
            IcebergOptions::new()
                .try_with_data_mime_type(MimeType::PARQUET)
                .unwrap(),
        );
        assert_eq!(table.options().unwrap().data_mime_type(), MimeType::PARQUET);
    }

    #[test]
    fn an_unparseable_format_property_is_a_typed_error_naming_the_key() {
        let path = root("format-unparseable");
        let mut table = Table::create(
            Folder::new(&path).unwrap(),
            FormatVersion::V2,
            trade_schema(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        table
            .commit_metadata_changes(|metadata| {
                metadata
                    .set_property("write.format.default", "csv")
                    .map(|_| ())
            })
            .unwrap();

        let message = table.options().unwrap_err().to_string();
        assert!(message.contains("write.format.default"), "{message}");
        assert!(message.contains("csv"), "{message}");

        // The write path resolves the same layer, so it fails the same way -
        // and an explicit option shadows the broken property, which is what
        // lets a caller repair it.
        let batch = trades(&[1], &[Some("AAPL")], &[Some("XNAS")]);
        let message = table
            .commit_append(crate::arrow::batch_reader(batch.schema(), [batch.clone()]))
            .unwrap_err()
            .to_string();
        assert!(message.contains("write.format.default"), "{message}");
        table.set_options(
            IcebergOptions::new()
                .try_with_data_mime_type(MimeType::PARQUET)
                .unwrap(),
        );
        table
            .commit_append(crate::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap();
    }

    #[test]
    fn metadata_only_formats_are_refused_up_front_naming_the_format() {
        for mime_type in [MimeType::ORC, MimeType::PUFFIN] {
            let name = mime_type.extension().unwrap();
            let path = root(&format!("format-{}", name.to_ascii_lowercase()));
            let mut table = Table::create(
                Folder::new(&path).unwrap(),
                FormatVersion::V2,
                trade_schema(),
                PartitionSpec::unpartitioned(),
            )
            .unwrap();
            table.set_options(
                IcebergOptions::new()
                    .try_with_data_mime_type(mime_type.clone())
                    .unwrap(),
            );

            let batch = trades(&[1], &[Some("AAPL")], &[Some("XNAS")]);
            let message = table
                .commit_append(crate::arrow::batch_reader(batch.schema(), [batch]))
                .unwrap_err()
                .to_string();
            assert!(message.contains(mime_type.as_str()), "{message}");
            assert!(message.contains("application/avro"), "{message}");
            assert!(table.current_snapshot().is_none());
            assert!(!path.join("data").exists());
        }
    }

    #[test]
    fn a_table_whose_files_mix_formats_writes_and_scans_as_one_shape() {
        let path = root("format-mixed");
        let mut table = Table::create(
            Folder::new(&path).unwrap(),
            FormatVersion::V2,
            trade_schema(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();

        // One Parquet append, then one Avro append via the explicit option.
        let first = trades(&[1], &[Some("AAPL")], &[Some("XNAS")]);
        table
            .commit_append(crate::arrow::batch_reader(first.schema(), [first]))
            .unwrap();
        table.set_options(
            IcebergOptions::new()
                .try_with_data_mime_type(MimeType::AVRO)
                .unwrap(),
        );
        let second = trades(&[2], &[None], &[None]);
        table
            .commit_append(crate::arrow::batch_reader(second.schema(), [second]))
            .unwrap();

        // The manifests record what was actually written, and each file's
        // name carries its own format's extension.
        let mut recorded = formats(&table);
        recorded.sort();
        assert_eq!(recorded.len(), 2);
        assert!(
            recorded
                .iter()
                .any(|(mime, path)| { mime == &MimeType::PARQUET && path.ends_with(".parquet") })
        );
        assert!(
            recorded
                .iter()
                .any(|(mime, path)| mime == &MimeType::AVRO && path.ends_with(".avro"))
        );

        // The mixed table scans as one shape.
        assert_eq!(
            collect(table.scan(None).unwrap()),
            [
                (1, Some("AAPL".to_owned()), Some("XNAS".to_owned())),
                (2, None, None),
            ]
        );

        // An Avro-written file still carries manifest statistics measured
        // from its rows, so a filtered plan can skip it.
        let avro_file = table
            .data_files()
            .unwrap()
            .into_iter()
            .map(|(file, _)| file)
            .find(|file| file.mime_type == MimeType::AVRO)
            .unwrap();
        assert_eq!(avro_file.record_count, 1);
        assert!(!avro_file.value_counts.is_empty());
        assert!(!avro_file.null_value_counts.is_empty());
        assert!(!avro_file.lower_bounds.is_empty());
    }

    #[test]
    fn an_avro_format_table_property_writes_avro_files_and_reads_back() {
        let path = root("format-avro-property");
        let mut table = Table::create(
            Folder::new(&path).unwrap(),
            FormatVersion::V2,
            trade_schema(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        table
            .commit_metadata_changes(|metadata| {
                metadata
                    .set_property("write.format.default", "avro")
                    .map(|_| ())
            })
            .unwrap();

        let batch = trades(&[1, 2], &[Some("AAPL"), None], &[Some("XNAS"), None]);
        table
            .commit_append(crate::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap();

        let recorded = formats(&table);
        assert!(
            recorded
                .iter()
                .all(|(mime_type, _)| mime_type == &MimeType::AVRO),
            "{recorded:?}"
        );
        // A fresh open reads the mixed chain from storage alone.
        let reopened = Table::open(Folder::new(&path).unwrap()).unwrap();
        assert_eq!(collect(reopened.scan(None).unwrap()).len(), 2);
    }
}

/// Regressions the Spark interop exchange surfaced, pinned here.
mod interop_regressions {
    use super::*;
    use crate::iceberg::{PartitionField, SchemaUpdate};

    /// A file written before a rename reads under the new name: Iceberg
    /// resolves a column by field id, not by name, so the old file's
    /// `symbol` column is the schema's `ticker` column.
    #[test]
    fn a_scan_resolves_renamed_columns_by_field_id() {
        let path = root("rename-by-id");
        let mut table = Table::create(
            Folder::new(&path).unwrap(),
            FormatVersion::V2,
            trade_schema(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let batch = trades(&[1], &[Some("AAPL")], &[Some("XNAS")]);
        table
            .commit_append(crate::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap();

        table
            .commit_metadata_changes(|metadata| {
                let mut update = SchemaUpdate::from_metadata(metadata)?;
                update.rename_column("symbol", "ticker");
                let evolved = update.into_field()?;
                let schema_id = metadata.add_schema(evolved)?;
                metadata.set_current_schema(schema_id)
            })
            .unwrap();

        // The pre-rename data file stores the column as `symbol`; the scan
        // must answer it under `ticker` rather than inventing a null column.
        let rows: Vec<(i64, Option<String>)> = table
            .scan(None)
            .unwrap()
            .map(|batch| {
                let batch = batch.unwrap();
                let ids = batch
                    .column_by_name("id")
                    .unwrap()
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .unwrap()
                    .clone();
                let tickers = batch
                    .column_by_name("ticker")
                    .unwrap()
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .unwrap()
                    .clone();
                (ids.value(0), Some(tickers.value(0).to_owned()))
            })
            .collect();
        assert_eq!(rows, [(1, Some("AAPL".to_owned()))]);

        // A projected scan pushes the file's own name down and still answers
        // under the schema's.
        let projection = table
            .schema()
            .unwrap()
            .clone()
            .without_fields(&["venue"])
            .unwrap();
        let projected = table.scan(Some(&projection)).unwrap();
        let mut names = Vec::new();
        let mut values = Vec::new();
        for batch in projected {
            let batch = batch.unwrap();
            names = batch
                .schema()
                .fields()
                .iter()
                .map(|field| field.name().clone())
                .collect();
            values.push(
                batch
                    .column_by_name("ticker")
                    .unwrap()
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .unwrap()
                    .value(0)
                    .to_owned(),
            );
        }
        assert_eq!(names, ["id", "ticker"]);
        assert_eq!(values, ["AAPL"]);
    }

    /// A transformed partition field restores no column: `at_day` is not a
    /// schema column, and the source column rides in the data file itself.
    #[test]
    fn transformed_partition_fields_restore_no_column() {
        let schema = trade_schema();
        let spec = PartitionSpec {
            spec_id: 0,
            fields: vec![
                PartitionField {
                    source_id: 2,
                    field_id: 1000,
                    name: "symbol".into(),
                    transform: crate::iceberg::Transform::Identity,
                },
                PartitionField {
                    source_id: 1,
                    field_id: 1001,
                    name: "id_bucket".into(),
                    transform: crate::iceberg::Transform::Bucket(4),
                },
            ],
        };
        let file = super::super::manifest::DataFile {
            partition: vec![Scalar::from("AAPL"), Scalar::from(3_i64)],
            ..Default::default()
        };

        let restored = super::super::scan::partition_columns(&spec, &schema, &file).unwrap();
        // Only the identity field restores; the bucket value stays in the
        // manifest where it belongs.
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].0.name(), "symbol");
        assert_eq!(restored[0].1, Scalar::from("AAPL"));
    }
}

#[test]
fn a_uuid_column_keeps_its_type_through_a_round_trip() {
    // `uuid` and `fixed[16]` were one physical type until `guid` became a
    // datatype of its own; the spelling now survives in the datatype, so
    // rewriting another writer's metadata cannot demote the column. Surfaced
    // by the Spark interop exchange.
    let document = crate::json::from_bytes(
        br#"{"type":"struct","schema-id":0,"fields":[
            {"id":1,"name":"id","required":true,"type":"long"},
            {"id":2,"name":"u","required":false,"type":"uuid"},
            {"id":3,"name":"f","required":false,"type":"fixed[16]"}]}"#,
    )
    .unwrap();
    let root = schema_from_json("row", &document).unwrap();
    let emitted = schema_into_json(&root).unwrap();
    let rendered = String::from_utf8(crate::json::into_bytes(&emitted).unwrap()).unwrap();
    assert!(rendered.contains(r#""type":"uuid""#), "{rendered}");
    assert!(rendered.contains(r#""type":"fixed[16]""#), "{rendered}");
}
