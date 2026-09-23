//! `rust/src/iceberg/mod.rs`: schemas, metadata, manifests, and whole tables.
//!
//! This is the module a caller opens a table through, so the suite is written
//! the way a caller writes one: it creates, commits, scans, compacts and reads
//! back through `yggdryl::iceberg` alone. Two things it pins are not a
//! caller's to reach - the counters a commit moves inside `TableMetadata`, and
//! the local staging a commit publishes through - and those come from
//! `yggdryl::internals`.

use std::any::Any;
use std::hash::Hash;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use arrow_array::{Array, ArrayRef, Int64Array, RecordBatch, StringArray};

use yggdryl::IOBase;
use yggdryl::fs::{
    ByteReader, ByteWriter, FileInfo, FileInfos, FileSelector, FileSystem, OutputMetadata,
    RandomAccessReader,
};
use yggdryl::iceberg::{
    CommitConflict, Compaction, DataFile, FieldSummary, FormatVersion, IcebergOptions,
    ManifestContent, ManifestEntry, ManifestFile, PartitionField, PartitionSpec, ScanPlan,
    ScanTask, Snapshot, SnapshotRef, SortField, SortOrder, Table, TableMetadata, Transform,
    assign_field_ids, schema_from_json, schema_into_json,
};
use yggdryl::local::LocalFolder;
use yggdryl::{DataType, Field, Scalar, StructType};

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
        partition: vec![Scalar::from(2024)],
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
        content: ManifestContent::Data,
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
        .try_with_data_mime_type(yggdryl::MimeType::AVRO)
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
        Some(&yggdryl::MimeType::AVRO)
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
        yggdryl::Error::InvalidRecord { path, reason }
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
        yggdryl::Error::InvalidRecord { path, reason }
            if path == "$.tasks[0].data_file.record_count"
                && reason.contains("non-negative")
    ));
}

#[test]
fn table_metadata_state_is_read_only_through_complete_accessors() {
    // The claim is that the accessor set covers every field, so each field is
    // named here beside the accessor that answers it. A field a caller cannot
    // reach is named through `yggdryl::internals`.
    use yggdryl::internals::iceberg_metadata as fields;

    let metadata = TableMetadata::new(
        FormatVersion::V2,
        "file:///table",
        trade_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();

    assert_eq!(metadata.format_version(), fields::format_version(&metadata));
    assert_eq!(
        metadata.table_uuid(),
        fields::table_uuid(&metadata).as_str()
    );
    assert_eq!(metadata.location(), fields::location(&metadata).as_str());
    assert_eq!(
        metadata.last_sequence_number(),
        fields::last_sequence_number(&metadata)
    );
    assert_eq!(
        metadata.last_updated_ms(),
        fields::last_updated_ms(&metadata)
    );
    assert_eq!(metadata.last_column_id(), fields::last_column_id(&metadata));
    assert_eq!(metadata.schemas(), fields::schemas(&metadata));
    assert_eq!(
        metadata.current_schema_id(),
        fields::current_schema_id(&metadata)
    );
    assert_eq!(
        metadata.partition_specs(),
        fields::partition_specs(&metadata)
    );
    assert_eq!(
        metadata.default_spec_id(),
        fields::default_spec_id(&metadata)
    );
    assert_eq!(
        metadata.last_partition_id(),
        fields::last_partition_id(&metadata)
    );
    assert_eq!(metadata.sort_orders(), fields::sort_orders(&metadata));
    assert_eq!(
        metadata.default_sort_order_id(),
        fields::default_sort_order_id(&metadata)
    );
    assert_eq!(metadata.properties(), fields::properties(&metadata));
    assert_eq!(
        metadata.current_snapshot_id(),
        fields::current_snapshot_id(&metadata)
    );
    assert_eq!(metadata.snapshots(), fields::snapshots(&metadata));
    assert_eq!(metadata.snapshot_log(), fields::snapshot_log(&metadata));
    assert_eq!(metadata.metadata_log(), fields::metadata_log(&metadata));
    assert_eq!(metadata.refs(), fields::refs(&metadata));
    assert_eq!(metadata.statistics(), fields::statistics(&metadata));
    assert_eq!(
        metadata.partition_statistics(),
        fields::partition_statistics(&metadata)
    );
    assert_eq!(
        metadata.encryption_keys(),
        fields::encryption_keys(&metadata)
    );
    assert_eq!(metadata.next_row_id(), fields::next_row_id(&metadata));
}

/// An Arrow filesystem that publishes one version hint and then reports that
/// same write as failed.
///
/// An object store can report an output-stream close failure after the bytes
/// are already visible. This fixture makes that state deterministic without
/// teaching the table about a test-only storage hook.
#[derive(Debug, Default)]
struct PublishedHintFailure {
    inner: yggdryl::fs::MemoryFileSystem,
    fail_next_hint: Arc<AtomicBool>,
}

impl PublishedHintFailure {
    fn arm(&self) {
        self.fail_next_hint.store(true, Ordering::Relaxed);
    }
}

impl yggdryl::fs::FileSystem for PublishedHintFailure {
    fn type_name(&self) -> &str {
        self.inner.type_name()
    }

    fn equals(&self, other: &dyn FileSystem) -> bool {
        other
            .as_any()
            .downcast_ref::<Self>()
            .is_some_and(|other| std::ptr::eq(self, other))
    }

    fn normalize_path(&self, path: &str) -> yggdryl::Result<String> {
        self.inner.normalize_path(path)
    }

    fn file_info(&self, path: &str) -> yggdryl::Result<FileInfo> {
        self.inner.file_info(path)
    }

    fn list(&self, selector: &FileSelector) -> FileInfos {
        self.inner.list(selector)
    }

    fn create_dir(&self, path: &str, recursive: bool) -> yggdryl::Result<()> {
        self.inner.create_dir(path, recursive)
    }

    fn delete_dir(&self, path: &str) -> yggdryl::Result<()> {
        self.inner.delete_dir(path)
    }

    fn delete_dir_contents(&self, path: &str, missing_dir_ok: bool) -> yggdryl::Result<()> {
        self.inner.delete_dir_contents(path, missing_dir_ok)
    }

    fn delete_root_dir_contents(&self) -> yggdryl::Result<()> {
        self.inner.delete_root_dir_contents()
    }

    fn delete_file(&self, path: &str) -> yggdryl::Result<()> {
        self.inner.delete_file(path)
    }

    fn copy_file(&self, source: &str, target: &str) -> yggdryl::Result<()> {
        self.inner.copy_file(source, target)
    }

    fn move_file(&self, source: &str, target: &str) -> yggdryl::Result<()> {
        self.inner.move_file(source, target)
    }

    fn open_input_file(&self, path: &str) -> yggdryl::Result<Box<dyn RandomAccessReader>> {
        self.inner.open_input_file(path)
    }

    fn open_input_stream(&self, path: &str) -> yggdryl::Result<Box<dyn ByteReader>> {
        self.inner.open_input_stream(path)
    }

    fn open_output_stream(
        &self,
        path: &str,
        metadata: Option<&OutputMetadata>,
    ) -> yggdryl::Result<Box<dyn ByteWriter>> {
        let writer = self.inner.open_output_stream(path, metadata)?;
        if path.ends_with("/version-hint.text") {
            Ok(Box::new(PublishedHintWriter {
                inner: writer,
                fail_next_hint: Arc::clone(&self.fail_next_hint),
            }))
        } else {
            Ok(writer)
        }
    }

    fn open_append_stream(
        &self,
        path: &str,
        metadata: Option<&OutputMetadata>,
    ) -> yggdryl::Result<Box<dyn ByteWriter>> {
        self.inner.open_append_stream(path, metadata)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct PublishedHintWriter {
    inner: Box<dyn ByteWriter>,
    fail_next_hint: Arc<AtomicBool>,
}

impl ByteWriter for PublishedHintWriter {
    fn write(&mut self, bytes: &[u8]) -> yggdryl::Result<usize> {
        self.inner.write(bytes)
    }

    fn tell(&self) -> u64 {
        self.inner.tell()
    }

    fn flush(&mut self) -> yggdryl::Result<()> {
        self.inner.flush()
    }

    fn close(&mut self) -> yggdryl::Result<()> {
        self.inner.close()?;
        if self.fail_next_hint.swap(false, Ordering::Relaxed) {
            return Err(yggdryl::Error::Io(std::io::Error::other(
                "injected acknowledgement failure after publishing the version hint",
            )));
        }
        Ok(())
    }

    fn closed(&self) -> bool {
        self.inner.closed()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
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
    inner: yggdryl::fs::MemoryFileSystem,
    armed: Arc<AtomicBool>,
    injections: Arc<AtomicUsize>,
}

impl SameVersionWinner {
    fn arm(&self) {
        self.armed.store(true, Ordering::Relaxed);
    }

    fn injections(&self) -> usize {
        self.injections.load(Ordering::Relaxed)
    }

    fn invalid(reason: &'static str) -> yggdryl::Error {
        yggdryl::Error::Codec {
            format: "iceberg",
            position: 0,
            reason: reason.into(),
        }
    }
}

impl yggdryl::fs::FileSystem for SameVersionWinner {
    fn type_name(&self) -> &str {
        self.inner.type_name()
    }

    fn equals(&self, other: &dyn FileSystem) -> bool {
        other
            .as_any()
            .downcast_ref::<Self>()
            .is_some_and(|other| std::ptr::eq(self, other))
    }

    fn normalize_path(&self, path: &str) -> yggdryl::Result<String> {
        self.inner.normalize_path(path)
    }

    fn file_info(&self, path: &str) -> yggdryl::Result<FileInfo> {
        self.inner.file_info(path)
    }

    fn list(&self, selector: &FileSelector) -> FileInfos {
        self.inner.list(selector)
    }

    fn create_dir(&self, path: &str, recursive: bool) -> yggdryl::Result<()> {
        self.inner.create_dir(path, recursive)
    }

    fn delete_dir(&self, path: &str) -> yggdryl::Result<()> {
        self.inner.delete_dir(path)
    }

    fn delete_dir_contents(&self, path: &str, missing_dir_ok: bool) -> yggdryl::Result<()> {
        self.inner.delete_dir_contents(path, missing_dir_ok)
    }

    fn delete_root_dir_contents(&self) -> yggdryl::Result<()> {
        self.inner.delete_root_dir_contents()
    }

    fn delete_file(&self, path: &str) -> yggdryl::Result<()> {
        self.inner.delete_file(path)
    }

    fn copy_file(&self, source: &str, target: &str) -> yggdryl::Result<()> {
        self.inner.copy_file(source, target)
    }

    fn move_file(&self, source: &str, target: &str) -> yggdryl::Result<()> {
        self.inner.move_file(source, target)
    }

    fn open_input_file(&self, path: &str) -> yggdryl::Result<Box<dyn RandomAccessReader>> {
        self.inner.open_input_file(path)
    }

    fn open_input_stream(&self, path: &str) -> yggdryl::Result<Box<dyn ByteReader>> {
        self.inner.open_input_stream(path)
    }

    fn open_output_stream(
        &self,
        path: &str,
        metadata: Option<&OutputMetadata>,
    ) -> yggdryl::Result<Box<dyn ByteWriter>> {
        let writer = self.inner.open_output_stream(path, metadata)?;
        if path.contains("/metadata/00002-") && path.ends_with(".metadata.json") {
            Ok(Box::new(SameVersionWriter {
                inner: writer,
                filesystem: self.inner.clone(),
                path: path.to_owned(),
                bytes: Vec::new(),
                armed: Arc::clone(&self.armed),
                injections: Arc::clone(&self.injections),
            }))
        } else {
            Ok(writer)
        }
    }

    fn open_append_stream(
        &self,
        path: &str,
        metadata: Option<&OutputMetadata>,
    ) -> yggdryl::Result<Box<dyn ByteWriter>> {
        self.inner.open_append_stream(path, metadata)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct SameVersionWriter {
    inner: Box<dyn ByteWriter>,
    filesystem: yggdryl::fs::MemoryFileSystem,
    path: String,
    bytes: Vec<u8>,
    armed: Arc<AtomicBool>,
    injections: Arc<AtomicUsize>,
}

impl SameVersionWriter {
    fn inject_winner(&self) -> yggdryl::Result<()> {
        let mut document: serde_json::Value = serde_json::from_slice(&self.bytes)?;
        let properties = document
            .as_object_mut()
            .and_then(|metadata| metadata.get_mut("properties"))
            .and_then(serde_json::Value::as_object_mut)
            .ok_or_else(|| SameVersionWinner::invalid("expected generated metadata properties"))?;
        if properties.remove("loser").is_none() {
            return Err(SameVersionWinner::invalid(
                "expected the attempted metadata to carry the loser's intent",
            ));
        }
        properties.insert(
            "winner".to_owned(),
            serde_json::Value::String("visible".to_owned()),
        );

        let (directory, candidate) = self
            .path
            .rsplit_once('/')
            .ok_or_else(|| SameVersionWinner::invalid("expected a metadata directory"))?;
        let first = "00002-00000000-0000-0000-0000-000000000000.metadata.json";
        let second = "00002-ffffffff-ffff-ffff-ffff-ffffffffffff.metadata.json";
        let competitor = if candidate == first { second } else { first };
        write_filesystem_bytes(
            &self.filesystem,
            &format!("{directory}/{competitor}"),
            &serde_json::to_vec(&document)?,
        )?;
        write_filesystem_bytes(
            &self.filesystem,
            &format!("{directory}/version-hint.text"),
            b"2",
        )?;
        self.injections.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

impl ByteWriter for SameVersionWriter {
    fn write(&mut self, bytes: &[u8]) -> yggdryl::Result<usize> {
        let written = self.inner.write(bytes)?;
        self.bytes.extend_from_slice(&bytes[..written]);
        Ok(written)
    }

    fn tell(&self) -> u64 {
        self.inner.tell()
    }

    fn flush(&mut self) -> yggdryl::Result<()> {
        self.inner.flush()
    }

    fn close(&mut self) -> yggdryl::Result<()> {
        self.inner.close()?;
        if self.armed.swap(false, Ordering::Relaxed) {
            self.inject_winner()?;
        }
        Ok(())
    }

    fn closed(&self) -> bool {
        self.inner.closed()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

fn write_filesystem_bytes(
    filesystem: &dyn FileSystem,
    path: &str,
    bytes: &[u8],
) -> yggdryl::Result<()> {
    let mut writer = filesystem.open_output_stream(path, None)?;
    let mut position = 0;
    while position < bytes.len() {
        let written = writer.write(&bytes[position..])?;
        if written == 0 {
            return Err(yggdryl::Error::Io(std::io::Error::new(
                std::io::ErrorKind::WriteZero,
                "filesystem fixture stream stopped before the complete value was written",
            )));
        }
        position += written;
    }
    writer.close()
}

fn read_filesystem_bytes(filesystem: &dyn FileSystem, path: &str) -> yggdryl::Result<Vec<u8>> {
    let mut reader = filesystem.open_input_stream(path)?;
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..read]);
    }
    reader.close()?;
    Ok(bytes)
}

/// Every file under a filesystem folder, at any depth, by its location.
fn listed_paths(folder: &yggdryl::fs::FsFolder) -> Vec<String> {
    let mut paths: Vec<String> = yggdryl::holder::Holder::from(folder.clone())
        .ls(true, false)
        .map(|entry| entry.unwrap())
        .filter(|entry| !entry.is_container())
        .filter_map(|entry| entry.url().map(ToString::to_string))
        .collect();
    paths.sort();
    paths
}

/// An Arrow filesystem that refuses the write of one versioned metadata
/// document - `v{n}.metadata.json` - after the attempt before it landed.
///
/// A store can refuse the second of two writes as well as the first; this
/// makes it refuse exactly the one that is the commit's point of no return,
/// so what a commit leaves behind when it fails just short of it is pinned.
#[derive(Debug, Default)]
struct RefusedDocumentWrite {
    inner: yggdryl::fs::MemoryFileSystem,
    refuse_next_document: Arc<AtomicBool>,
}

impl RefusedDocumentWrite {
    fn arm(&self) {
        self.refuse_next_document.store(true, Ordering::Relaxed);
    }
}

impl yggdryl::fs::FileSystem for RefusedDocumentWrite {
    fn type_name(&self) -> &str {
        self.inner.type_name()
    }

    fn equals(&self, other: &dyn FileSystem) -> bool {
        other
            .as_any()
            .downcast_ref::<Self>()
            .is_some_and(|other| std::ptr::eq(self, other))
    }

    fn normalize_path(&self, path: &str) -> yggdryl::Result<String> {
        self.inner.normalize_path(path)
    }

    fn file_info(&self, path: &str) -> yggdryl::Result<FileInfo> {
        self.inner.file_info(path)
    }

    fn list(&self, selector: &FileSelector) -> FileInfos {
        self.inner.list(selector)
    }

    fn create_dir(&self, path: &str, recursive: bool) -> yggdryl::Result<()> {
        self.inner.create_dir(path, recursive)
    }

    fn delete_dir(&self, path: &str) -> yggdryl::Result<()> {
        self.inner.delete_dir(path)
    }

    fn delete_dir_contents(&self, path: &str, missing_dir_ok: bool) -> yggdryl::Result<()> {
        self.inner.delete_dir_contents(path, missing_dir_ok)
    }

    fn delete_root_dir_contents(&self) -> yggdryl::Result<()> {
        self.inner.delete_root_dir_contents()
    }

    fn delete_file(&self, path: &str) -> yggdryl::Result<()> {
        self.inner.delete_file(path)
    }

    fn copy_file(&self, source: &str, target: &str) -> yggdryl::Result<()> {
        self.inner.copy_file(source, target)
    }

    fn move_file(&self, source: &str, target: &str) -> yggdryl::Result<()> {
        self.inner.move_file(source, target)
    }

    fn open_input_file(&self, path: &str) -> yggdryl::Result<Box<dyn RandomAccessReader>> {
        self.inner.open_input_file(path)
    }

    fn open_input_stream(&self, path: &str) -> yggdryl::Result<Box<dyn ByteReader>> {
        self.inner.open_input_stream(path)
    }

    fn open_output_stream(
        &self,
        path: &str,
        metadata: Option<&OutputMetadata>,
    ) -> yggdryl::Result<Box<dyn ByteWriter>> {
        let name = path.rsplit('/').next().unwrap_or(path);
        if name.starts_with('v')
            && name.ends_with(".metadata.json")
            && self.refuse_next_document.swap(false, Ordering::Relaxed)
        {
            return Err(yggdryl::Error::Io(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "the versioned document write is refused",
            )));
        }
        self.inner.open_output_stream(path, metadata)
    }

    fn open_append_stream(
        &self,
        path: &str,
        metadata: Option<&OutputMetadata>,
    ) -> yggdryl::Result<Box<dyn ByteWriter>> {
        self.inner.open_append_stream(path, metadata)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A memory filesystem that lands a competing metadata-only document at
/// version 2 after a data commit's version preflight but before its
/// same-version collision listing, and records every file the loser removes.
///
/// The competitor is the version 1 document with a property added - a
/// genuine other writer's commit, naming none of the loser's files - so the
/// loser rebases onto it, publishes its manifest list again, and what it
/// does with the list of the attempt it replaces is what this pins.
#[derive(Debug, Default)]
struct MetadataOnlyWinner {
    inner: yggdryl::fs::MemoryFileSystem,
    armed: Arc<AtomicBool>,
    removed: Arc<std::sync::Mutex<Vec<String>>>,
}

impl MetadataOnlyWinner {
    fn arm(&self) {
        self.armed.store(true, Ordering::Relaxed);
    }

    fn removed(&self) -> Vec<String> {
        self.removed.lock().unwrap().clone()
    }
}

impl yggdryl::fs::FileSystem for MetadataOnlyWinner {
    fn type_name(&self) -> &str {
        self.inner.type_name()
    }

    fn equals(&self, other: &dyn FileSystem) -> bool {
        other
            .as_any()
            .downcast_ref::<Self>()
            .is_some_and(|other| std::ptr::eq(self, other))
    }

    fn normalize_path(&self, path: &str) -> yggdryl::Result<String> {
        self.inner.normalize_path(path)
    }

    fn file_info(&self, path: &str) -> yggdryl::Result<FileInfo> {
        self.inner.file_info(path)
    }

    fn list(&self, selector: &FileSelector) -> FileInfos {
        self.inner.list(selector)
    }

    fn create_dir(&self, path: &str, recursive: bool) -> yggdryl::Result<()> {
        self.inner.create_dir(path, recursive)
    }

    fn delete_dir(&self, path: &str) -> yggdryl::Result<()> {
        self.inner.delete_dir(path)
    }

    fn delete_dir_contents(&self, path: &str, missing_dir_ok: bool) -> yggdryl::Result<()> {
        self.inner.delete_dir_contents(path, missing_dir_ok)
    }

    fn delete_root_dir_contents(&self) -> yggdryl::Result<()> {
        self.inner.delete_root_dir_contents()
    }

    fn delete_file(&self, path: &str) -> yggdryl::Result<()> {
        self.removed.lock().unwrap().push(path.to_owned());
        self.inner.delete_file(path)
    }

    fn copy_file(&self, source: &str, target: &str) -> yggdryl::Result<()> {
        self.inner.copy_file(source, target)
    }

    fn move_file(&self, source: &str, target: &str) -> yggdryl::Result<()> {
        self.inner.move_file(source, target)
    }

    fn open_input_file(&self, path: &str) -> yggdryl::Result<Box<dyn RandomAccessReader>> {
        self.inner.open_input_file(path)
    }

    fn open_input_stream(&self, path: &str) -> yggdryl::Result<Box<dyn ByteReader>> {
        self.inner.open_input_stream(path)
    }

    fn open_output_stream(
        &self,
        path: &str,
        metadata: Option<&OutputMetadata>,
    ) -> yggdryl::Result<Box<dyn ByteWriter>> {
        let writer = self.inner.open_output_stream(path, metadata)?;
        if path.contains("/metadata/00002-") && path.ends_with(".metadata.json") {
            Ok(Box::new(MetadataOnlyWriter {
                inner: writer,
                filesystem: self.inner.clone(),
                path: path.to_owned(),
                armed: Arc::clone(&self.armed),
            }))
        } else {
            Ok(writer)
        }
    }

    fn open_append_stream(
        &self,
        path: &str,
        metadata: Option<&OutputMetadata>,
    ) -> yggdryl::Result<Box<dyn ByteWriter>> {
        self.inner.open_append_stream(path, metadata)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct MetadataOnlyWriter {
    inner: Box<dyn ByteWriter>,
    filesystem: yggdryl::fs::MemoryFileSystem,
    path: String,
    armed: Arc<AtomicBool>,
}

impl MetadataOnlyWriter {
    fn inject_winner(&self) -> yggdryl::Result<()> {
        let (directory, _) = self
            .path
            .rsplit_once('/')
            .ok_or_else(|| SameVersionWinner::invalid("expected a metadata directory"))?;
        let previous =
            read_filesystem_bytes(&self.filesystem, &format!("{directory}/v1.metadata.json"))?;
        let mut document: serde_json::Value = serde_json::from_slice(&previous)?;
        let metadata = document
            .as_object_mut()
            .ok_or_else(|| SameVersionWinner::invalid("expected a metadata object"))?;
        let properties = metadata
            .entry("properties")
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()))
            .as_object_mut()
            .ok_or_else(|| SameVersionWinner::invalid("expected metadata properties"))?;
        properties.insert(
            "winner".to_owned(),
            serde_json::Value::String("visible".to_owned()),
        );
        write_filesystem_bytes(
            &self.filesystem,
            &format!("{directory}/00002-ffffffff-ffff-ffff-ffff-ffffffffffff.metadata.json"),
            &serde_json::to_vec(&document)?,
        )?;
        write_filesystem_bytes(
            &self.filesystem,
            &format!("{directory}/version-hint.text"),
            b"2",
        )
    }
}

impl ByteWriter for MetadataOnlyWriter {
    fn write(&mut self, bytes: &[u8]) -> yggdryl::Result<usize> {
        self.inner.write(bytes)
    }

    fn tell(&self) -> u64 {
        self.inner.tell()
    }

    fn flush(&mut self) -> yggdryl::Result<()> {
        self.inner.flush()
    }

    fn close(&mut self) -> yggdryl::Result<()> {
        self.inner.close()?;
        if self.armed.swap(false, Ordering::Relaxed) {
            self.inject_winner()?;
        }
        Ok(())
    }

    fn closed(&self) -> bool {
        self.inner.closed()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

/// One column's Arrow array, built from the values the rows hold for it.
///
/// `yggdryl::Serie::from_scalars` types a column from the rows a caller has;
/// a row list is gathered into that column's values first, which is all this
/// adds.
fn column_of_rows(field: &Field, values: &[&Scalar]) -> yggdryl::Result<arrow_array::ArrayRef> {
    yggdryl::Serie::from_scalars(field.clone(), values.iter().copied().cloned())?
        .require_arrow_array()
}

/// Build a scratch directory unique to this test and this process.
fn root(label: &str) -> std::path::PathBuf {
    let mut path = LocalFolder::temporary().unwrap().path().unwrap();
    path.push(format!("yggdryl-iceberg-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    path
}

/// The three-column trade schema every table test writes.
fn trade_schema() -> Field {
    let mut schema = StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("symbol"),
        DataType::utf8().nullable_field("venue"),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("row");
    assign_field_ids(&mut schema, 1).unwrap();
    schema.insert_metadata("ICEBERG:schema-id", "0").unwrap();
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
fn collect(reader: yggdryl::arrow::BatchReader) -> Vec<(i64, Option<String>, Option<String>)> {
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
    use yggdryl::DataType;
    use yggdryl::StructType;

    #[test]
    fn a_nested_schema_round_trips_through_json() {
        let document = yggdryl::json::from_utf8(
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
            schema.fields()[1].get_metadata("ICEBERG:doc"),
            Some("ticker")
        );

        // Requirement inverts into nullability.
        assert!(!schema.fields()[0].is_nullable());
        assert!(schema.fields()[1].is_nullable());

        assert_eq!(schema_into_json(&schema).unwrap(), document);
    }

    #[test]
    fn identifiers_are_assigned_depth_first_and_never_reassigned() {
        let inner = DataType::from(
            StructType::from_fields([DataType::Int64.required_field("price")]).unwrap(),
        );
        let mut schema = StructType::from_fields([
            DataType::Int64.required_field("id"),
            inner.nullable_field("leg"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row");

        assert_eq!(assign_field_ids(&mut schema, 1).unwrap(), 4);
        assert_eq!(schema.fields()[0].parquet_field_id().unwrap(), Some(1));
        assert_eq!(schema.fields()[1].parquet_field_id().unwrap(), Some(2));
        assert_eq!(
            schema.fields()[1].fields()[0].parquet_field_id().unwrap(),
            Some(3)
        );
        assert_eq!(yggdryl::iceberg::last_column_id(&schema).unwrap(), 3);

        // A second pass changes nothing, because every field already has an id.
        assert_eq!(assign_field_ids(&mut schema, 100).unwrap(), 100);
        assert_eq!(schema.fields()[0].parquet_field_id().unwrap(), Some(1));
    }

    #[test]
    fn writing_a_schema_without_identifiers_says_what_to_call() {
        let schema = StructType::from_fields([DataType::Int64.required_field("id")])
            .map(DataType::from)
            .unwrap()
            .required_field("row");

        let message = schema_into_json(&schema).unwrap_err().to_string();
        assert!(message.contains("assign_field_ids"), "{message}");
    }

    #[test]
    fn a_schema_document_that_is_not_a_struct_is_rejected() {
        let message = schema_from_json(
            "row",
            &yggdryl::json::from_utf8(r#"{"type":"list"}"#).unwrap(),
        )
        .unwrap_err()
        .to_string();
        assert!(message.contains("\"struct\""), "{message}");

        let message = schema_from_json("row", &yggdryl::json::from_utf8("[1, 2]").unwrap())
            .unwrap_err()
            .to_string();
        assert!(message.contains("got list"), "{message}");
    }

    #[test]
    fn a_field_missing_its_required_flag_is_rejected() {
        let document = yggdryl::json::from_utf8(
            r#"{"type":"struct","fields":[{"id":1,"name":"id","type":"long"}]}"#,
        )
        .unwrap();
        let message = schema_from_json("row", &document).unwrap_err().to_string();
        assert!(message.contains("invalid schema JSON"), "{message}");
    }

    #[test]
    fn official_schema_validation_rejects_duplicate_field_ids() {
        let document = yggdryl::json::from_utf8(
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
        let document = yggdryl::json::from_utf8(
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
        let document = yggdryl::json::from_utf8(
            r#"{"type":"struct","fields":[
                {"id":1,"name":"id","required":true,"type":"long"}
            ]}"#,
        )
        .unwrap();

        let schema = schema_from_json("row", &document).unwrap();
        assert_eq!(schema.get_metadata("ICEBERG:schema-id"), Some("0"));
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
        let document = yggdryl::json::from_utf8(
            r#"{"type":"struct","schema-id":3,"identifier-field-ids":[2,1],"fields":[
                {"id":1,"name":"left","required":true,"type":"long"},
                {"id":2,"name":"right","required":true,"type":"string"}
            ]}"#,
        )
        .unwrap();

        let schema = schema_from_json("row", &document).unwrap();
        assert_eq!(
            schema.get_metadata("ICEBERG:identifier-field-ids"),
            Some("1,2")
        );
        let emitted = schema_into_json(&schema).unwrap();
        let identifiers: Vec<i64> = emitted
            .get_key_str("identifier-field-ids")
            .into_iter()
            .flat_map(Scalar::iter)
            .filter_map(|id| id.as_i64())
            .collect();
        assert_eq!(identifiers, [1, 2]);
    }

    #[test]
    fn emitted_schema_is_checked_by_the_official_model() {
        let mut schema = StructType::from_fields([DataType::Int64.nullable_field("id")])
            .map(DataType::from)
            .unwrap()
            .required_field("row");
        assign_field_ids(&mut schema, 1).unwrap();
        schema
            .insert_metadata("ICEBERG:identifier-field-ids", "1")
            .unwrap();

        let message = schema_into_json(&schema).unwrap_err().to_string();
        assert!(message.contains("optional field"), "{message}");
    }

    #[test]
    fn an_iceberg_schema_projects_into_arrow_with_its_identifiers() {
        let document = yggdryl::json::from_utf8(
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
        let document = yggdryl::json::from_utf8(
            r#"{"type":"struct","fields":[
                {"id":1,"name":"venue","required":false,"type":"string",
                 "initial-default":"XNAS","write-default":"XNAS"}
            ]}"#,
        )
        .unwrap();
        let schema = schema_from_json("row", &document).unwrap();
        assert_eq!(
            schema.fields()[0].get_metadata("ICEBERG:initial-default"),
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
        let document: Scalar = yggdryl::json::from_utf8(
            r#"{"type":"struct","fields":[{"id":1,"name":"id","required":true,"type":"long"}]}"#,
        )
        .unwrap();
        assert!(document.contains_key("fields"));
        assert!(schema_from_json("row", &document).is_ok());
    }
}

mod types {

    use yggdryl::iceberg::PrimitiveType;
    use yggdryl::{DataType, TimeUnit};

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
            DataType::fixed_binary(16).unwrap()
        );
    }

    #[test]
    fn iceberg_temporal_types_are_microsecond_precision_unless_v3_says_otherwise() {
        assert_eq!(
            PrimitiveType::Timestamp.into_dtype().unwrap(),
            DataType::DateTime64 {
                unit: TimeUnit::Microsecond,
                timezone: yggdryl::Timezone::NAIVE
            }
        );
        assert_eq!(
            PrimitiveType::TimestampNs.into_dtype().unwrap(),
            DataType::DateTime64 {
                unit: TimeUnit::Nanosecond,
                timezone: yggdryl::Timezone::NAIVE
            }
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
    fn a_text_storage_string_or_a_code_is_an_iceberg_string() {
        // The padding is storage; every Iceberg reader sees the text. Iceberg
        // has nothing that carries a code's identity, so a code is a string
        // there exactly as a width is.
        for dtype in [
            DataType::utf8(),
            DataType::large_utf8(),
            DataType::utf8_view(),
            DataType::ascii(),
            DataType::fixed_utf8(8).unwrap(),
            DataType::fixed_ascii(2).unwrap(),
            DataType::fixed_ascii(3).unwrap(),
            DataType::fixed_ascii(4).unwrap(),
            DataType::fixed_ascii(8).unwrap(),
            DataType::fixed_ascii(12).unwrap(),
            DataType::fixed_ascii(16).unwrap(),
        ]
        .into_iter()
        // And every registered code, read from the one listing rather than
        // named four at a time here.
        .chain(DataType::CODES.iter().map(|(_, dtype, _)| dtype.clone()))
        {
            assert_eq!(
                PrimitiveType::from_dtype(&dtype).unwrap(),
                PrimitiveType::String,
                "{dtype}"
            );
        }
    }

    #[test]
    fn a_string_in_another_charset_is_refused_by_name() {
        // Iceberg's string is UTF-8; bytes in another charset are not, and
        // writing them as a string would hand every reader mojibake.
        let latin = DataType::cp1252();
        let message = PrimitiveType::from_dtype(&latin).unwrap_err().to_string();
        assert!(
            message.contains("expected a datatype Iceberg can express"),
            "{message}"
        );
        assert!(message.contains("cp1252"), "{message}");
        assert!(!yggdryl::internals::iceberg_value::is_portable(&latin));
    }

    #[test]
    fn a_code_bound_is_portable_and_round_trips_as_a_string_datum() {
        // Bounds are what a planner prunes with, and a missed arm in the
        // value layer drops them silently rather than failing.
        for (dtype, value) in [
            (DataType::Country, "FR"),
            (DataType::Currency, "USD"),
            (DataType::MicCode, "XPAR"),
            (DataType::CfiCode, "ESVUFR"),
            (DataType::IsinCode, "US0378331005"),
            (DataType::Side, "BUY"),
            (DataType::State, "0"),
            (DataType::TimeInForce, "GTC"),
        ] {
            assert!(
                yggdryl::internals::iceberg_value::is_portable(&dtype),
                "{dtype}"
            );
            // A bound is read off a column, so the value it encodes is the
            // one the column holds: a code with a vocabulary stores its own
            // spelling, which is what the reader will compare against.
            let exact = dtype.scalar(yggdryl::Scalar::from(value)).unwrap();
            let bytes = yggdryl::internals::iceberg_value::single_value(&exact, &dtype)
                .unwrap_or_else(|| panic!("{dtype} must encode a bound"));
            assert_eq!(bytes, exact.as_str().unwrap().as_bytes(), "{dtype}");
            assert_eq!(
                yggdryl::internals::iceberg_value::single_to_value(&bytes, &dtype),
                Some(exact),
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

    use yggdryl::StructType;
    use yggdryl::iceberg::assign_field_ids;
    use yggdryl::internals::iceberg_partition::write_transforms;
    use yggdryl::{DataType, Scalar};

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
            yggdryl::json::from_utf8(r#"[{"name":"venue","transform":"identity","source-id":3}]"#)
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
    fn identity_partitions_keep_literal_and_reserved_top_level_names() {
        let mut schema = StructType::from_fields([
            DataType::utf8().required_field("a.b"),
            DataType::Int64.required_field("null"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
        assign_field_ids(&mut schema, 1).unwrap();
        let ids: Vec<i32> = schema
            .fields()
            .iter()
            .map(|field| field.parquet_field_id().unwrap().unwrap())
            .collect();

        let spec = PartitionSpec::identity(7, &schema, &["a.b", "null"]).unwrap();
        assert_eq!(
            spec.fields
                .iter()
                .map(|field| field.name.as_str())
                .collect::<Vec<_>>(),
            ["a.b", "null"]
        );
        assert_eq!(
            spec.fields
                .iter()
                .map(|field| field.source_id)
                .collect::<Vec<_>>(),
            ids
        );

        let marked = spec.mark_partitions(&schema).unwrap();
        assert_eq!(
            marked.partition_field_names().collect::<Vec<_>>(),
            ["a.b", "null"]
        );
        assert_eq!(
            marked
                .fields()
                .iter()
                .map(|field| field.parquet_field_id().unwrap())
                .collect::<Vec<_>>(),
            vec![Some(ids[0]), Some(ids[1])]
        );
        assert_eq!(PartitionSpec::from_schema(7, &marked).unwrap(), spec);
    }

    #[test]
    fn a_partition_directory_is_spelled_the_way_every_other_lake_spells_it() {
        let schema = StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::date32().nullable_field("day"),
        ])
        .map(DataType::from)
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
            yggdryl::media::partition::partition_text(&Scalar::date32(19_723)).unwrap(),
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
        let timestamp = DataType::DateTime64 {
            unit: yggdryl::TimeUnit::Microsecond,
            timezone: yggdryl::Timezone::NAIVE,
        };
        assert_eq!(
            Transform::Day.result_type(&timestamp).unwrap(),
            DataType::date32()
        );
        for transform in [Transform::Year, Transform::Month, Transform::Hour] {
            assert_eq!(transform.result_type(&timestamp).unwrap(), DataType::Int32);
        }
        assert_eq!(
            Transform::Bucket(16)
                .result_type(&DataType::utf8())
                .unwrap(),
            DataType::Int32
        );
        assert_eq!(
            Transform::Truncate(3)
                .result_type(&DataType::binary())
                .unwrap(),
            DataType::binary()
        );

        assert!(Transform::Year.result_type(&DataType::Int64).is_err());
        assert!(
            Transform::Bucket(16)
                .result_type(&DataType::Boolean)
                .is_err()
        );
        assert!(
            Transform::Truncate(3)
                .result_type(&DataType::date32())
                .is_err()
        );
        assert!(Transform::Bucket(0).result_type(&DataType::Int32).is_err());
    }

    #[test]
    fn scalar_transform_plan_is_total_at_date_extremes_and_truncates_binary() {
        let mut schema = StructType::from_fields([
            DataType::date32().required_field("day"),
            DataType::binary().required_field("payload"),
        ])
        .map(DataType::from)
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
        let plan = write_transforms(&spec, &schema, &partition).unwrap();

        assert!(
            plan[0]
                .partition_value(Scalar::date32(i32::MAX))
                .unwrap()
                .as_i64()
                .is_some()
        );
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
            DataType::utf8()
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

    use yggdryl::Scalar;

    use super::{FormatVersion, PartitionSpec, trade_schema};
    use smol_str::SmolStr;
    use yggdryl::iceberg::{Snapshot, SnapshotRef, SortField, SortOrder, TableMetadata, Transform};
    use yggdryl::internals::iceberg_metadata::{
        current_schema_id_mut, default_sort_order_id_mut, last_updated_ms_mut, metadata_log_mut,
        partition_specs_mut, properties_mut, refs_mut, schemas_mut, snapshot_log_mut,
        snapshots_mut, sort_orders_mut,
    };

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
            assert_eq!(read.format_version(), version);
            assert_eq!(read.location(), original.location());
            assert_eq!(read.last_column_id(), 3);
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
        sort_orders_mut(&mut metadata).push(order);
        *default_sort_order_id_mut(&mut metadata) = order_id;
        let document = metadata.into_json().unwrap();
        let read = TableMetadata::from_json(&document).unwrap();
        assert_eq!(read.default_sort_order_id(), order_id);
        assert!(
            read.sort_orders()
                .iter()
                .any(|order| order.order_id == order_id)
        );

        let missing = yggdryl::json::from_utf8(r#"{"fields":[]}"#).unwrap();
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
        let timestamp = metadata.last_updated_ms() + 60_000;
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
        *metadata_log_mut(&mut metadata) = vec![
            (10, "metadata/v1.json".into()),
            (20, "metadata/v2.json".into()),
        ];
        metadata.validate().unwrap();

        let mut equivalent = metadata.clone();
        schemas_mut(&mut equivalent).reverse();
        partition_specs_mut(&mut equivalent).reverse();
        sort_orders_mut(&mut equivalent).reverse();
        properties_mut(&mut equivalent).reverse();
        snapshots_mut(&mut equivalent).reverse();
        refs_mut(&mut equivalent).reverse();
        equivalent.validate().unwrap();

        assert_eq!(metadata, equivalent);
        assert_eq!(metadata.cmp(&equivalent), std::cmp::Ordering::Equal);
        assert_eq!(hash(&metadata), hash(&equivalent));
        assert_eq!(metadata.stable_hash(), equivalent.stable_hash());

        let mut reordered_history = metadata.clone();
        snapshot_log_mut(&mut reordered_history).reverse();
        metadata_log_mut(&mut reordered_history).reverse();
        assert_ne!(metadata, reordered_history);
        assert_eq!(
            metadata.cmp(&reordered_history),
            reordered_history.cmp(&metadata).reverse()
        );
        assert_ne!(metadata.stable_hash(), reordered_history.stable_hash());

        let earlier = metadata.clone();
        let mut middle = metadata.clone();
        *last_updated_ms_mut(&mut middle) += 1;
        let mut later = metadata;
        *last_updated_ms_mut(&mut later) += 2;
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
        let document = yggdryl::json::from_utf8(
            r#"{"format-version":1,"table-uuid":"0b4f7721-755e-5df5-ab6b-8a23c7905a82","location":"file:///t",
                "last-updated-ms":1,"last-column-id":1,
                "schema":{"type":"struct","fields":[
                    {"id":1,"name":"id","required":true,"type":"long"}]},
                "partition-spec":[]}"#,
        )
        .unwrap();
        let metadata = TableMetadata::from_json(&document).unwrap();
        assert_eq!(metadata.format_version(), FormatVersion::V1);
        assert_eq!(metadata.schemas().len(), 1);
        assert_eq!(metadata.partition_specs().len(), 1);
        assert!(metadata.default_spec().unwrap().is_unpartitioned());
    }

    #[test]
    fn v1_direct_manifest_paths_survive_official_metadata_updates() {
        let mut metadata = metadata(FormatVersion::V1);
        let snapshot = Snapshot {
            snapshot_id: 7,
            parent_snapshot_id: None,
            sequence_number: None,
            timestamp_ms: metadata.last_updated_ms() + 1,
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
            .flat_map(Scalar::iter)
            .next()
            .unwrap();
        assert!(encoded.get_key_str("manifest-list").is_none());
        assert_eq!(
            encoded
                .get_key_str("manifests")
                .map_or(0, |manifests| manifests.iter().count()),
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
        let timestamp_ms = metadata.last_updated_ms() + 60_000;
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
        let timestamp_ms = metadata.last_updated_ms();
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
        let statistics = yggdryl::json::from_utf8(
            r#"[{"snapshot-id":7,"statistics-path":"s3://bucket/stats.puffin",
                "file-size-in-bytes":413,"file-footer-size-in-bytes":42,
                "key-metadata":"c3RhdHMta2V5",
                "blob-metadata":[{"type":"apache-datasketches-theta-v1",
                    "snapshot-id":7,"sequence-number":0,"fields":[1,2],
                    "properties":{"compression":"zstd","ndv":"2"}}]}]"#,
        )
        .unwrap();
        let partition_statistics = yggdryl::json::from_utf8(
            r#"[{"snapshot-id":7,
                "statistics-path":"s3://bucket/partition-stats.parquet",
                "file-size-in-bytes":43}]"#,
        )
        .unwrap();
        let encryption_keys = yggdryl::json::from_utf8(
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
                .flat_map(Scalar::iter)
                .find(|snapshot| {
                    snapshot.get_key_str("snapshot-id").and_then(Scalar::as_i64) == Some(7)
                })
                .as_deref()
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
        assert!(read.current_snapshot_id().is_none());
        assert!(read.current_snapshot().is_none());
    }

    #[test]
    fn an_evolved_schema_keeps_the_old_one_and_numbers_above_it() {
        let mut metadata = metadata(FormatVersion::V2);
        let mut evolved = trade_schema();
        evolved.remove_metadata("ICEBERG:schema-id");
        let mut fields = evolved.fields().to_vec();
        fields.push(yggdryl::DataType::Int64.nullable_field("quantity"));
        evolved
            .set_dtype(yggdryl::DataType::from(
                yggdryl::StructType::from_fields(fields).unwrap(),
            ))
            .unwrap();
        super::assign_field_ids(&mut evolved, metadata.last_column_id() + 1).unwrap();

        let schema_id = metadata.add_schema(evolved).unwrap();
        assert_eq!(schema_id, 1);
        assert_eq!(metadata.schemas().len(), 2, "the old schema is retained");
        assert_eq!(metadata.last_column_id(), 4);
        *current_schema_id_mut(&mut metadata) = schema_id;
        assert_eq!(metadata.current_schema().unwrap().field_len(), 4);

        // And the pair survives the document.
        let read = TableMetadata::from_json(&metadata.into_json().unwrap()).unwrap();
        assert_eq!(read.schemas().len(), 2);
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
    use yggdryl::StructType;

    use std::sync::Arc;

    use arrow_array::{
        ArrayRef, Int32Array, Int64Array, RecordBatch, StringArray, StructArray,
        TimestampMicrosecondArray,
    };
    use arrow_schema::DataType as ArrowDataType;

    use super::{
        FormatVersion, IOBase, IcebergOptions, LocalFolder, PartitionField, PartitionSpec, Table,
        Transform, assign_field_ids, collect, root, trade_schema, trades,
    };

    use yggdryl::IOMedia;
    use yggdryl::internals::iceberg_table::child_at;
    use yggdryl::media::IORecordOptions;
    use yggdryl::{DataType, Scalar, TimeUnit};

    #[test]
    fn create_numbers_an_unnumbered_schema_itself() {
        let path = root("unnumbered-create");
        // The schema a user projects straight from Arrow: no ids anywhere.
        let schema = StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::utf8().nullable_field("venue"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row");

        let mut table = Table::create(
            LocalFolder::new(&path).unwrap(),
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
        assert_eq!(table.metadata().last_column_id(), 2);

        // The numbered table is a working table, not merely a written one.
        let batch = trades(&[7], &[None], &[Some("X")]);
        table
            .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap();
        let reopened = Table::open(LocalFolder::new(&path).unwrap()).unwrap();
        assert_eq!(collect(reopened.scan(None).unwrap()).len(), 1);
    }

    #[test]
    fn create_keeps_the_ids_a_partly_numbered_schema_carries() {
        let path = root("partly-numbered-create");
        let mut id = DataType::Int64.required_field("id");
        id.set_parquet_field_id(7);
        let schema = StructType::from_fields([id, DataType::utf8().nullable_field("venue")])
            .map(DataType::from)
            .unwrap()
            .required_field("row");

        let table = Table::create(
            LocalFolder::new(&path).unwrap(),
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
        assert_eq!(table.metadata().last_column_id(), 8);
    }

    #[test]
    fn an_empty_table_has_no_snapshot_and_reads_as_no_rows() {
        let path = root("empty");
        let schema = trade_schema();
        let table = Table::create(
            LocalFolder::new(&path).unwrap(),
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
        let reopened = Table::open(LocalFolder::new(&path).unwrap()).unwrap();
        assert_eq!(reopened.metadata_version(), 1);
        assert!(reopened.current_snapshot().is_none());
    }

    #[test]
    fn open_preserves_an_official_uuid_metadata_name_in_history() {
        let path = root("official-metadata-name");
        let table = Table::create(
            LocalFolder::new(&path).unwrap(),
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

        let mut reopened = Table::open(LocalFolder::new(&path).unwrap()).unwrap();
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
                .metadata_log()
                .iter()
                .any(|(_, location)| location.ends_with(official_name))
        );
    }

    #[test]
    fn a_commit_publishes_the_name_a_version_hint_resolves() {
        let path = root("hint-resolvable-metadata-name");
        let mut table = Table::create(
            LocalFolder::new(&path).unwrap(),
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
            LocalFolder::new(&path).unwrap(),
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
        let reopened = Table::open(LocalFolder::new(&path).unwrap()).unwrap();
        assert_eq!(reopened.metadata_file_name(), table.metadata_file_name());
        assert_eq!(reopened.metadata().property("owner"), None);
    }

    #[test]
    fn direct_create_refuses_to_replace_an_existing_table() {
        let path = root("create-conflict");
        Table::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            trade_schema(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let error = Table::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            trade_schema(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap_err();
        assert!(error.is_conflict(), "{error}");
        assert_eq!(
            Table::open(LocalFolder::new(&path).unwrap())
                .unwrap()
                .metadata_version(),
            1
        );
    }

    #[test]
    fn child_locations_require_a_table_path_boundary() {
        let path = root("location-boundary");
        let table = Table::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            trade_schema(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let sibling = format!("{}2/data/file.parquet", table.metadata().location());
        assert!(child_at(&table, &sibling).is_err());
    }

    #[test]
    fn an_unpartitioned_table_round_trips_its_rows() {
        let path = root("unpartitioned");
        let schema = trade_schema();
        let mut table = Table::create(
            LocalFolder::new(&path).unwrap(),
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
            .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
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
        let reopened = Table::open(LocalFolder::new(&path).unwrap()).unwrap();
        assert_eq!(collect(reopened.scan(None).unwrap()), rows);
    }

    #[test]
    fn a_v1_snapshot_with_direct_manifests_scans_and_time_travels() {
        let path = root("v1-direct-manifests");
        let mut table = Table::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V1,
            trade_schema(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let first = trades(&[1], &[Some("AAPL")], &[Some("XNAS")]);
        table
            .commit_append(yggdryl::arrow::batch_reader(first.schema(), [first]))
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
            .iter()
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
        std::fs::write(metadata_path, yggdryl::json::into_bytes(&document).unwrap()).unwrap();

        let mut reopened = Table::open(LocalFolder::new(&path).unwrap()).unwrap();
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
            .commit_append(yggdryl::arrow::batch_reader(second.schema(), [second]))
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
        let mut table = Table::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            schema,
            spec,
        )
        .unwrap();

        let batch = trades(
            &[1, 2, 3],
            &[Some("AAPL"), Some("MSFT"), Some("AAPL")],
            &[Some("XNAS"), Some("XNYS"), Some("XNAS")],
        );
        table
            .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap();

        let files = table.data_files().unwrap();
        assert_eq!(files.len(), 2, "one file per distinct venue");

        // The layout is the `column=value` shape the crate's own Hive reader
        // already understands.
        for (file, _) in &files {
            let url = yggdryl::Url::from_str(&file.file_path).unwrap();
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
        let folder = LocalFolder::new(&path).unwrap();
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
        let mut schema = StructType::from_fields([
            DataType::Int32.required_field("id"),
            DataType::utf8().required_field("text"),
            DataType::DateTime64 {
                unit: TimeUnit::Microsecond,
                timezone: yggdryl::Timezone::NAIVE,
            }
            .required_field("ts_year"),
            DataType::DateTime64 {
                unit: TimeUnit::Microsecond,
                timezone: yggdryl::Timezone::NAIVE,
            }
            .required_field("ts_month"),
            DataType::DateTime64 {
                unit: TimeUnit::Microsecond,
                timezone: yggdryl::Timezone::NAIVE,
            }
            .required_field("ts_day"),
            DataType::DateTime64 {
                unit: TimeUnit::Microsecond,
                timezone: yggdryl::Timezone::NAIVE,
            }
            .required_field("ts_hour"),
            DataType::Int64.required_field("retired"),
        ])
        .map(DataType::from)
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
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            schema.clone(),
            spec,
        )
        .unwrap();

        // 2017-11-16T22:31:08, the timestamp used by Apache Iceberg's own
        // transform fixtures.
        let timestamp = 1_510_871_468_000_000_i64;
        let arrow = schema.clone().into_arrow_schema().unwrap();
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
            .commit_append(yggdryl::arrow::batch_reader(arrow, [batch]))
            .unwrap();

        let files = table.data_files().unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(
            files[0].0.partition,
            vec![
                Scalar::from(3),
                Scalar::from("ice"),
                Scalar::from(47),
                Scalar::from(574),
                Scalar::date32(17_486),
                Scalar::from(419_686),
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
        let nested = StructType::from_fields([DataType::utf8().required_field("category")])
            .map(DataType::from)
            .unwrap()
            .required_field("payload");
        let mut schema = StructType::from_fields([nested])
            .map(DataType::from)
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
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            schema.clone(),
            spec,
        )
        .unwrap();

        let arrow = schema.clone().into_arrow_schema().unwrap();
        let ArrowDataType::Struct(children) = arrow.field(0).data_type() else {
            panic!("payload must be a struct")
        };
        let payload = StructArray::from(vec![(
            children[0].clone(),
            Arc::new(StringArray::from(vec!["iceberg", "icecube"])) as ArrayRef,
        )]);
        let batch = RecordBatch::try_new(arrow.clone(), vec![Arc::new(payload)]).unwrap();
        table
            .commit_append(yggdryl::arrow::batch_reader(arrow, [batch]))
            .unwrap();

        let files = table.data_files().unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0.partition, vec![Scalar::from("ic")]);
        assert!(files[0].0.file_path.contains("category_prefix=ic"));
    }

    #[test]
    fn calendar_partitions_group_distinct_instants_by_their_utc_period() {
        // Every instant is distinct, so only the calendar period can group
        // them; the ones either side of the epoch are where flooring and
        // truncating a count disagree.
        let path = root("calendar-grouping");
        let at = || DataType::DateTime64 {
            unit: TimeUnit::Microsecond,
            timezone: yggdryl::Timezone::NAIVE,
        };
        let mut schema = StructType::from_fields([
            DataType::Int64.required_field("id"),
            at().required_field("day_at"),
            at().required_field("hour_at"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
        assign_field_ids(&mut schema, 1).unwrap();
        let spec = PartitionSpec {
            spec_id: 0,
            fields: vec![
                PartitionField {
                    source_id: 2,
                    field_id: 1000,
                    name: "day".into(),
                    transform: Transform::Day,
                },
                PartitionField {
                    source_id: 3,
                    field_id: 1001,
                    name: "hour".into(),
                    transform: Transform::Hour,
                },
            ],
        };
        let mut table = Table::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            schema.clone(),
            spec,
        )
        .unwrap();

        let hour = 3_600_000_000_i64;
        let instants = [
            -hour - 1,
            -1,
            0,
            1,
            hour - 1,
            23 * hour,
            24 * hour - 1,
            24 * hour,
        ];
        let arrow = schema.clone().into_arrow_schema().unwrap();
        let batch = RecordBatch::try_new(
            arrow.clone(),
            vec![
                Arc::new(Int64Array::from_iter_values(0..8)) as ArrayRef,
                Arc::new(TimestampMicrosecondArray::from(instants.to_vec())) as ArrayRef,
                Arc::new(TimestampMicrosecondArray::from(instants.to_vec())) as ArrayRef,
            ],
        )
        .unwrap();
        table
            .commit_append(yggdryl::arrow::batch_reader(arrow, [batch]))
            .unwrap();

        let mut tuples: Vec<(Vec<Scalar>, i64)> = table
            .data_files()
            .unwrap()
            .into_iter()
            .map(|(file, _)| (file.partition, file.record_count))
            .collect();
        tuples.sort_by_key(|(_, rows)| *rows);
        let mut expected = vec![
            (vec![Scalar::date32(-1), Scalar::from(-2)], 1),
            (vec![Scalar::date32(-1), Scalar::from(-1)], 1),
            (vec![Scalar::date32(0), Scalar::from(0)], 3),
            (vec![Scalar::date32(0), Scalar::from(23)], 2),
            (vec![Scalar::date32(1), Scalar::from(24)], 1),
        ];
        expected.sort_by_key(|(_, rows)| *rows);
        assert_eq!(tuples.len(), expected.len(), "{tuples:?}");
        for tuple in &expected {
            assert!(tuples.contains(tuple), "{tuple:?} in {tuples:?}");
        }
    }

    #[test]
    fn a_day_before_the_epoch_holds_its_last_second_whatever_arrives_first() {
        // 1969-12-30T23:59:59.5 and 1969-12-30T12:00 are one UTC day, day -2.
        // Truncating the count toward zero before flooring would move the
        // first into 1969-12-31, and a write keys every instant of a day
        // together, so whichever row came first would label both.
        let last_second = -86_400_500_000_i64;
        let noon = -129_600_000_000_i64;
        for (label, order) in [
            ("day-before-epoch-a", [last_second, noon]),
            ("day-before-epoch-b", [noon, last_second]),
        ] {
            let path = root(label);
            let at = |unit| DataType::DateTime64 {
                unit,
                timezone: yggdryl::Timezone::NAIVE,
            };
            let mut schema = StructType::from_fields([
                DataType::Int64.required_field("id"),
                at(TimeUnit::Microsecond).required_field("at_us"),
                at(TimeUnit::Nanosecond).required_field("at_ns"),
            ])
            .map(DataType::from)
            .unwrap()
            .required_field("row");
            assign_field_ids(&mut schema, 1).unwrap();
            let spec = PartitionSpec {
                spec_id: 0,
                fields: vec![
                    PartitionField {
                        source_id: 2,
                        field_id: 1000,
                        name: "day_us".into(),
                        transform: Transform::Day,
                    },
                    PartitionField {
                        source_id: 3,
                        field_id: 1001,
                        name: "day_ns".into(),
                        transform: Transform::Day,
                    },
                ],
            };
            let mut table = Table::create(
                LocalFolder::new(&path).unwrap(),
                FormatVersion::V2,
                schema.clone(),
                spec,
            )
            .unwrap();
            let arrow = schema.clone().into_arrow_schema().unwrap();
            let batch = RecordBatch::try_new(
                arrow.clone(),
                vec![
                    Arc::new(Int64Array::from_iter_values(0..2)) as ArrayRef,
                    Arc::new(TimestampMicrosecondArray::from(order.to_vec())) as ArrayRef,
                    Arc::new(arrow_array::TimestampNanosecondArray::from(
                        order
                            .iter()
                            .map(|micros| micros * 1_000)
                            .collect::<Vec<_>>(),
                    )) as ArrayRef,
                ],
            )
            .unwrap();
            table
                .commit_append(yggdryl::arrow::batch_reader(arrow, [batch]))
                .unwrap();
            let tuples: Vec<(Vec<Scalar>, i64)> = table
                .data_files()
                .unwrap()
                .into_iter()
                .map(|(file, _)| (file.partition, file.record_count))
                .collect();
            assert_eq!(
                tuples,
                vec![(vec![Scalar::date32(-2), Scalar::date32(-2)], 2)],
                "{label}"
            );
        }
    }

    #[test]
    fn a_nan_row_survives_file_pruning_and_clean_files_still_prune() {
        // Iceberg bounds leave NaN out, and this crate orders NaN past every
        // number, so only a NaN count of zero lets a float bound skip a file.
        let path = root("nan-pruning");
        let mut schema = StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::Float64.required_field("price"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
        assign_field_ids(&mut schema, 1).unwrap();
        let mut table = Table::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            schema.clone(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let arrow = schema.clone().into_arrow_schema().unwrap();
        for (ids, prices) in [([1, 2], [1.0, f64::NAN]), ([3, 4], [200.0, 300.0])] {
            let batch = RecordBatch::try_new(
                arrow.clone(),
                vec![
                    Arc::new(Int64Array::from(ids.to_vec())) as ArrayRef,
                    Arc::new(arrow_array::Float64Array::from(prices.to_vec())) as ArrayRef,
                ],
            )
            .unwrap();
            table
                .commit_append(yggdryl::arrow::batch_reader(arrow.clone(), [batch]))
                .unwrap();
        }
        let mut nans: Vec<Vec<(i32, i64)>> = table
            .data_files()
            .unwrap()
            .into_iter()
            .map(|(file, _)| file.nan_value_counts)
            .collect();
        nans.sort();
        assert_eq!(nans, vec![vec![(2, 0)], vec![(2, 1)]]);

        let ids = |filter: &str| {
            let mut ids: Vec<i64> = table
                .scan_matching(filter, None)
                .unwrap()
                .flat_map(|batch| {
                    batch
                        .unwrap()
                        .column(0)
                        .as_any()
                        .downcast_ref::<Int64Array>()
                        .unwrap()
                        .values()
                        .to_vec()
                })
                .collect();
            ids.sort_unstable();
            ids
        };
        assert_eq!(ids("price > 100"), vec![2, 3, 4]);
        assert_eq!(ids("price > 500"), vec![2]);
        // The NaN-free file is still skipped from its bounds alone.
        assert_eq!(table.plan_matching("price > 500").unwrap().tasks.len(), 1);
    }

    #[test]
    fn sliced_input_is_cut_into_files_by_its_own_rows() {
        // One batch, then the same rows as zero-copy slices of it: each slice
        // counts its own extent, so both write the same number of files.
        let files = |label: &str, slices: usize| {
            let path = root(label);
            let mut schema = StructType::from_fields([DataType::Int64.required_field("id")])
                .map(DataType::from)
                .unwrap()
                .required_field("row");
            assign_field_ids(&mut schema, 1).unwrap();
            let mut table = Table::create(
                LocalFolder::new(&path).unwrap(),
                FormatVersion::V2,
                schema.clone(),
                PartitionSpec::unpartitioned(),
            )
            .unwrap();
            let arrow = schema.clone().into_arrow_schema().unwrap();
            let rows = 200_000_usize;
            let batch = RecordBatch::try_new(
                arrow.clone(),
                vec![Arc::new(Int64Array::from_iter_values(0..rows as i64)) as ArrayRef],
            )
            .unwrap();
            let step = rows / slices;
            let pieces: Vec<RecordBatch> = (0..slices)
                .map(|piece| batch.slice(piece * step, step))
                .collect();
            let mut options = yggdryl::iceberg::IcebergOptions::default();
            options.set_target_file_size_bytes(256 * 1024).unwrap();
            table.set_options(options);
            table
                .commit_append(yggdryl::arrow::batch_reader(arrow, pieces))
                .unwrap();
            table.data_files().unwrap().len()
        };
        let whole = files("sliced-whole", 1);
        assert!(whole > 1, "{whole}");
        assert_eq!(files("sliced-pieces", 100), whole);
    }

    #[test]
    fn a_source_under_a_null_struct_partitions_as_null_whatever_its_slot_holds() {
        // The child slot under a null parent still holds bytes, and here they
        // are exactly the valid row's value: one partition key must not
        // swallow the other.
        let path = root("null-parent-partition");
        let nested = StructType::from_fields([DataType::utf8().required_field("category")])
            .map(DataType::from)
            .unwrap()
            .nullable_field("payload");
        let mut schema = StructType::from_fields([DataType::Int64.required_field("id"), nested])
            .map(DataType::from)
            .unwrap()
            .required_field("row");
        assign_field_ids(&mut schema, 1).unwrap();
        let spec = PartitionSpec {
            spec_id: 0,
            fields: vec![PartitionField {
                source_id: 3,
                field_id: 1000,
                name: "category".into(),
                transform: Transform::Identity,
            }],
        };
        let mut table = Table::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            schema.clone(),
            spec,
        )
        .unwrap();

        let arrow = schema.clone().into_arrow_schema().unwrap();
        let ArrowDataType::Struct(children) = arrow.field(1).data_type() else {
            panic!("payload must be a struct")
        };
        let payload = StructArray::try_new(
            children.clone(),
            vec![Arc::new(StringArray::from(vec!["books", "books", "games"])) as ArrayRef],
            Some(vec![true, false, true].into()),
        )
        .unwrap();
        let batch = RecordBatch::try_new(
            arrow.clone(),
            vec![
                Arc::new(Int64Array::from(vec![1, 2, 3])) as ArrayRef,
                Arc::new(payload) as ArrayRef,
            ],
        )
        .unwrap();
        table
            .commit_append(yggdryl::arrow::batch_reader(arrow, [batch]))
            .unwrap();

        let mut partitions: Vec<Scalar> = table
            .data_files()
            .unwrap()
            .into_iter()
            .map(|(file, _)| file.partition[0].clone())
            .collect();
        partitions.sort_by_key(|value| format!("{value:?}"));
        let mut expected = vec![Scalar::from("books"), Scalar::Null, Scalar::from("games")];
        expected.sort_by_key(|value| format!("{value:?}"));
        assert_eq!(partitions, expected);
    }

    #[test]
    fn a_null_partition_value_writes_its_own_directory_and_reads_back_null() {
        let path = root("null-partition");
        let schema = trade_schema();
        let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
        let mut table = Table::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            schema,
            spec,
        )
        .unwrap();

        let batch = trades(
            &[1, 2],
            &[Some("AAPL"), Some("MSFT")],
            &[Some("XNAS"), None],
        );
        table
            .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
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
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            schema,
            PartitionSpec::unpartitioned(),
        )
        .unwrap();

        let first = trades(&[1], &[Some("AAPL")], &[Some("XNAS")]);
        table
            .commit_append(yggdryl::arrow::batch_reader(first.schema(), [first]))
            .unwrap();
        let second = trades(&[2], &[Some("MSFT")], &[Some("XNYS")]);
        table
            .commit_append(yggdryl::arrow::batch_reader(second.schema(), [second]))
            .unwrap();
        assert_eq!(collect(table.scan(None).unwrap()).len(), 2);
        assert_eq!(table.manifests().unwrap().len(), 2);

        let replacement = trades(&[9], &[Some("NVDA")], &[Some("XNAS")]);
        table
            .commit_overwrite(yggdryl::arrow::batch_reader(
                replacement.schema(),
                [replacement],
            ))
            .unwrap();
        let rows = collect(table.scan(None).unwrap());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, 9);

        // The previous snapshot is still recorded, which is what makes the
        // overwrite reversible.
        assert_eq!(table.metadata().snapshots().len(), 3);
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
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            schema.clone(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let batch = trades(&[1, 2], &[Some("AAPL"), Some("MSFT")], &[Some("XNAS"); 2]);
        table
            .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap();

        let mut wanted = schema.without_fields(&["symbol", "venue"]).unwrap();
        wanted.remove_metadata("ICEBERG:schema-id");
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
        let handle = LocalFolder::new(&path)
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
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            schema.clone(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let batch = trades(&[1], &[Some("AAPL")], &[Some("XNAS")]);
        table
            .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap();

        let mut evolved = schema.clone();
        evolved.remove_metadata("ICEBERG:schema-id");
        let mut fields = evolved.fields().to_vec();
        fields.push(DataType::Int64.nullable_field("quantity"));
        evolved
            .set_dtype(DataType::from(StructType::from_fields(fields).unwrap()))
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
            .commit_append(yggdryl::arrow::batch_reader(arrow, [widened]))
            .unwrap();
        assert_eq!(collect(table.scan(None).unwrap()).len(), 2);
    }

    #[test]
    fn a_manifest_naming_a_file_that_is_not_there_fails_the_scan_not_the_metadata() {
        let path = root("missing-file");
        let schema = trade_schema();
        let mut table = Table::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            schema,
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let batch = trades(&[1], &[Some("AAPL")], &[Some("XNAS")]);
        table
            .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap();

        let file = table.data_files().unwrap()[0].0.file_path.clone();
        let relative = file.rsplit("/data/").next().unwrap().to_owned();
        std::fs::remove_file(path.join("data").join(&relative)).unwrap();

        // The metadata still reads: a manifest is metadata, and it still says
        // what it always said.
        let reopened = Table::open(LocalFolder::new(&path).unwrap()).unwrap();
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
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V1,
            schema,
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let batch = trades(&[1, 2], &[Some("AAPL"), None], &[Some("XNAS"), None]);
        table
            .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap();

        assert_eq!(
            table.current_snapshot().unwrap().sequence_number,
            None,
            "v1 snapshots carry no sequence number"
        );
        let reopened = Table::open(LocalFolder::new(&path).unwrap()).unwrap();
        assert_eq!(reopened.metadata().format_version(), FormatVersion::V1);
        assert_eq!(collect(reopened.scan(None).unwrap()).len(), 2);
    }

    #[test]
    fn a_v3_table_tracks_row_lineage_across_commits() {
        let path = root("v3");
        let schema = trade_schema();
        let mut table = Table::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
            schema,
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        assert_eq!(table.metadata().next_row_id(), Some(0));

        let first = trades(&[1, 2], &[Some("AAPL"), Some("MSFT")], &[Some("XNAS"); 2]);
        table
            .commit_append(yggdryl::arrow::batch_reader(first.schema(), [first]))
            .unwrap();
        assert_eq!(table.current_snapshot().unwrap().first_row_id, Some(0));
        assert_eq!(table.current_snapshot().unwrap().added_rows, Some(2));
        assert_eq!(table.metadata().next_row_id(), Some(2));
        assert_eq!(table.manifests().unwrap()[0].first_row_id, Some(0));
        assert_eq!(table.data_files().unwrap()[0].0.first_row_id, Some(0));

        let second = trades(&[3], &[Some("NVDA")], &[Some("XNYS")]);
        table
            .commit_append(yggdryl::arrow::batch_reader(second.schema(), [second]))
            .unwrap();
        assert_eq!(table.current_snapshot().unwrap().first_row_id, Some(2));
        assert_eq!(table.current_snapshot().unwrap().added_rows, Some(1));
        assert_eq!(table.metadata().next_row_id(), Some(3));
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

        let reopened = Table::open(LocalFolder::new(&path).unwrap()).unwrap();
        assert_eq!(reopened.metadata().next_row_id(), Some(3));
        assert_eq!(collect(reopened.scan(None).unwrap()).len(), 3);
    }

    #[test]
    fn v3_rewrites_are_rejected_before_reader_or_storage_mutation() {
        let path = root("v3-rewrite-lineage");
        let mut table = Table::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
            trade_schema(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        table.set_options(IcebergOptions::default().with_compact_after_commits(2));

        for id in [1_i64, 2] {
            let batch = trades(&[id], &[Some("AAPL")], &[Some("XNAS")]);
            table
                .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
                .unwrap();
        }
        assert_eq!(table.data_files().unwrap().len(), 2);
        assert!(
            table
                .metadata()
                .snapshots()
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
        let reader = yggdryl::arrow::batch_reader(
            reader_schema,
            std::iter::from_fn(move || {
                reader_pulls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                incoming.take()
            }),
        );
        let merge_error = table
            .commit_merge_where(&[], reader, &yggdryl::Selector::from_columns(["id"]), true)
            .expect_err("v3 keyed merge must preserve existing row IDs");
        assert_eq!(pulls.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert!(
            matches!(&merge_error, yggdryl::Error::Iceberg { source: None, .. }),
            "{merge_error:?}"
        );
        assert!(merge_error.to_string().contains("merge"), "{merge_error}");
        assert!(merge_error.to_string().contains("row IDs"), "{merge_error}");

        let compact_error = table
            .compact()
            .expect_err("v3 compaction must preserve existing row IDs");
        assert!(
            matches!(&compact_error, yggdryl::Error::Iceberg { source: None, .. }),
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
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            trade_schema(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        for id in [1_i64, 2] {
            let batch = trades(&[id], &[Some("AAPL")], &[Some("XNAS")]);
            table
                .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
                .unwrap();
        }

        let incoming = trades(&[1], &[Some("AAPL.L")], &[Some("XLON")]);
        table
            .commit_merge(
                yggdryl::arrow::batch_reader(incoming.schema(), [incoming]),
                &yggdryl::Selector::from_columns(["id"]),
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
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            schema,
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let old = trades(&[1, 2], &[Some("AAPL"), Some("MSFT")], &[Some("XNAS"); 2]);
        table
            .commit_append(yggdryl::arrow::batch_reader(old.schema(), [old]))
            .unwrap();
        table
            .commit_metadata_changes(|metadata| metadata.upgrade_format_version(FormatVersion::V3))
            .unwrap();
        assert_eq!(table.metadata().next_row_id(), Some(0));

        let added = trades(&[3], &[Some("NVDA")], &[Some("XNYS")]);
        table
            .commit_append(yggdryl::arrow::batch_reader(added.schema(), [added]))
            .unwrap();
        let snapshot = table.current_snapshot().unwrap();
        assert_eq!(snapshot.first_row_id, Some(0));
        assert_eq!(snapshot.added_rows, Some(3));
        assert_eq!(table.metadata().next_row_id(), Some(3));
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
        let message = Table::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            schema,
            spec,
        )
        .unwrap_err()
        .to_string();
        assert!(message.contains("venue"), "{message}");
        assert!(message.contains("conflicts"), "{message}");
    }

    #[test]
    fn a_folder_with_no_metadata_says_so_rather_than_pretending_to_be_a_table() {
        let path = root("not-a-table");
        std::fs::create_dir_all(&path).unwrap();
        let message = Table::open(LocalFolder::new(&path).unwrap())
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
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            schema,
            spec.clone(),
        )
        .unwrap();
        let batch = trades(&[1], &[Some("AAPL")], &[Some("XNAS")]);
        table
            .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap();

        let manifest = table.manifests().unwrap().remove(0);
        let name = manifest
            .manifest_path
            .rsplit('/')
            .next()
            .unwrap()
            .to_owned();
        let handle = LocalFolder::new(&path)
            .unwrap()
            .child_by_path(&format!("metadata/{name}"))
            .unwrap();

        // The spec comes back out of the manifest's own Avro header.
        assert_eq!(yggdryl::iceberg::read_manifest_spec(&handle).unwrap(), spec);
        let entries = yggdryl::iceberg::read_manifest(&handle).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, yggdryl::iceberg::EntryStatus::Added);
        assert_eq!(entries[0].data_file.record_count, 1);
        assert_eq!(entries[0].data_file.mime_type, yggdryl::MimeType::PARQUET);
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
    use yggdryl::StructType;

    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};

    use super::{
        Field, FormatVersion, IOBase, PartitionSpec, Table, collect, root, trade_schema, trades,
    };
    use yggdryl::DataType;
    use yggdryl::iceberg::assign_field_ids;
    use yggdryl::local::LocalFolder;

    /// A table partitioned by venue, with one commit per venue.
    ///
    /// One commit is one manifest, so this is also the smallest table whose
    /// manifest list has something to prune.
    fn venues(label: &str) -> (std::path::PathBuf, Table<LocalFolder>) {
        let path = root(label);
        let schema = trade_schema();
        let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
        let mut table = Table::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            schema,
            spec,
        )
        .unwrap();
        for (id, symbol, venue) in [
            (1_i64, "AAPL", "XNAS"),
            (2, "MSFT", "XNYS"),
            (3, "VOD", "XLON"),
        ] {
            let batch = trades(&[id], &[Some(symbol)], &[Some(venue)]);
            table
                .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
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

        // A range the summaries cannot settle still reads every manifest, and
        // still answers correctly from the rows.
        let ranged = table.plan_matching("id >= 2").unwrap();
        assert_eq!(ranged.manifests_read, 3);

        let rows = collect(table.scan_matching("id >= 2", None).unwrap());
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|(id, _, _)| *id >= 2));
    }

    #[test]
    fn one_predicate_mixes_the_partition_and_the_rows() {
        let (_path, mut table) = venues("scan-mixed");
        // A second row in a partition that already exists is what makes the row
        // half of the predicate load-bearing: with one row per venue, a
        // conjunct over the rows is settled by the partition and the test would
        // pass even if the rows were never consulted.
        let batch = trades(&[4], &[Some("BP")], &[Some("XLON")]);
        table
            .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap();

        let rows = collect(
            table
                .scan_matching("id >= 4 and symbol is not null and venue = 'XLON'", None)
                .unwrap(),
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, 4);
        assert_eq!(rows[0].2.as_deref(), Some("XLON"));

        // Each half on its own keeps more, so neither was dropped above.
        let held = collect(table.scan_matching("venue = 'XLON'", None).unwrap());
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
        let mut handle = LocalFolder::new(&path)
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
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            schema,
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        // Two commits, so two files whose id bounds do not overlap.
        for ids in [[1_i64, 2], [10, 11]] {
            let batch = trades(&ids, &[Some("AAPL"), Some("MSFT")], &[None, None]);
            table
                .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
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
        let target = StructType::from_fields([DataType::Int64.required_field("id")])
            .map(DataType::from)
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
        let mut table = Table::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            schema,
            spec,
        )
        .unwrap();
        let batch = trades(
            &[1, 2],
            &[Some("AAPL"), Some("MSFT")],
            &[Some("XNAS"), None],
        );
        table
            .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
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
    fn a_code_column_bounds_a_manifest_by_the_text_iceberg_reads() {
        // Iceberg has `string` and nothing that carries a code's identity, so
        // a `mic` column's bound is the text a reader compares. That was
        // already true of a padded column - the Iceberg writer trimmed on the
        // way out - and this pins that the storage change did not move it:
        // the bound is the value, at either end of the width.
        let path = root("code-bounds");
        let mut schema = StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::MicCode.nullable_field("venue"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
        assign_field_ids(&mut schema, 1).unwrap();
        schema.insert_metadata("ICEBERG:schema-id", "0").unwrap();
        let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
        let mut table = Table::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            schema.clone(),
            spec,
        )
        .unwrap();
        let arrow = schema.clone().into_arrow_schema().unwrap();
        for (id, venue) in [(1_i64, "XNAS"), (2, "BX")] {
            let batch = RecordBatch::try_new(
                Arc::clone(&arrow),
                vec![
                    Arc::new(Int64Array::from(vec![id])),
                    Arc::new(StringArray::from(vec![Some(venue)])),
                ],
            )
            .unwrap();
            table
                .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
                .unwrap();
        }

        let mut bounds: Vec<String> = table
            .manifests()
            .unwrap()
            .iter()
            .map(|manifest| {
                String::from_utf8(manifest.partitions[0].lower_bound.clone().unwrap()).unwrap()
            })
            .collect();
        bounds.sort();
        // `BX` is shorter than the width its standard fixes, and its bound is
        // still exactly `BX`.
        assert_eq!(bounds, ["BX", "XNAS"]);

        // And the column reads back a market identifier, not anonymous text.
        let plan = table.plan(&[("venue", "BX")]).unwrap();
        assert_eq!(plan.tasks.len(), 1);
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
        let target: Field = StructType::from_fields([
            DataType::utf8().nullable_field("venue"),
            DataType::Int64.required_field("id"),
        ])
        .map(DataType::from)
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
    use yggdryl::StructType;

    use arrow_array::{
        Array, Int64Array, RecordBatch, RecordBatchIterator, RecordBatchReader, StringArray,
    };
    use arrow_schema::{ArrowError, SchemaRef};

    use super::{
        FormatVersion, IOBase, PartitionSpec, Table, assign_field_ids, collect, root, trade_schema,
        trades,
    };
    use yggdryl::IOMedia;
    use yggdryl::holder::Buffer;
    use yggdryl::local::LocalFolder;
    use yggdryl::media::{IORecordOptions, RecordOptions};
    use yggdryl::{DataType, MimeType};

    /// Create a venue-partitioned table and return the folder addressing it.
    fn table(label: &str) -> (std::path::PathBuf, LocalFolder) {
        let path = root(label);
        let schema = trade_schema();
        let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
        Table::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            schema,
            spec,
        )
        .unwrap();
        let folder = LocalFolder::new(&path).unwrap();
        (path, folder)
    }

    /// The options an Iceberg folder is written through, with a declared schema.
    fn options(folder: &LocalFolder) -> RecordOptions {
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

    fn counted(batch: RecordBatch, pulls: Arc<AtomicUsize>) -> yggdryl::arrow::BatchReader {
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
            yggdryl::MimeType::PARQUET
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
                yggdryl::arrow::batch_reader(batch.schema(), [batch]),
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
                yggdryl::arrow::batch_reader(batch.schema(), [batch]),
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
                yggdryl::arrow::batch_reader(batch.schema(), [batch]),
                &options,
            )
            .unwrap();
        assert_eq!(
            collect(folder.read_arrow_reader(&options).unwrap()),
            vec![(9, Some("BP".to_owned()), Some("XLON".to_owned()))]
        );

        // Every write was a snapshot, so the table has one commit per call.
        let table = Table::open(LocalFolder::new(&path).unwrap()).unwrap();
        assert_eq!(table.metadata().snapshots().len(), 3);
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
                yggdryl::arrow::batch_reader(batch.schema(), [batch]),
                &options,
            )
            .unwrap();

        let merge = options.clone().with_merge_by(["id"]).unwrap();
        let batch = trades(
            &[2, 4],
            &[Some("MSFT.L"), Some("BP")],
            &[Some("XNYS"), Some("XLON")],
        );
        folder
            .merge_arrow_reader(
                yggdryl::arrow::batch_reader(batch.schema(), [batch]),
                &merge,
            )
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
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            schema,
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        for ids in [[1_i64, 2], [10, 11], [20, 21]] {
            let batch = trades(&ids, &[Some("AAPL"), Some("MSFT")], &[None, None]);
            table
                .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
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
                yggdryl::arrow::batch_reader(batch.schema(), [batch]),
                &yggdryl::Selector::from_columns(["id"]),
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
                yggdryl::arrow::batch_reader(batch.schema(), [batch]),
                &options,
            )
            .unwrap();

        // The same address a Hive lake would use, resolved through the table's
        // metadata rather than by listing the directory.
        let partition = LocalFolder::new(path.join("data").join("venue=XNYS")).unwrap();
        assert_eq!(
            collect(partition.read_arrow_reader(&options).unwrap()),
            vec![(2, Some("MSFT".to_owned()), Some("XNYS".to_owned()))]
        );

        // Overwriting one partition leaves every other one where it was.
        let mut partition = partition;
        let batch = trades(&[7], &[Some("MSFT")], &[Some("XNYS")]);
        partition
            .overwrite_arrow_reader(
                yggdryl::arrow::batch_reader(batch.schema(), [batch]),
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
        let lake = LocalFolder::new(&path).unwrap();
        let mut leaf = lake.child_by_path("part-0.parquet").unwrap();
        let batch = trades(&[1], &[Some("AAPL")], &[Some("XNAS")]);
        let options = RecordOptions::for_media_type(leaf.media_type())
            .unwrap()
            .with_field(trade_schema());
        leaf.overwrite_arrow_reader(
            yggdryl::arrow::batch_reader(batch.schema(), [batch]),
            &options,
        )
        .unwrap();
        leaf.close().unwrap();
        drop(leaf);

        let lake = LocalFolder::new(&path).unwrap();
        assert_eq!(collect(lake.read_arrow_reader(&options).unwrap()).len(), 1);
    }

    #[test]
    fn the_table_value_is_itself_a_handle_answering_from_its_metadata() {
        let path = root("handle-table-value");
        let schema = trade_schema();
        let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
        let mut table = Table::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            schema,
            spec,
        )
        .unwrap();

        // The byte surface is the folder the table lives in, but the role is
        // the table's own: one tabular value, never a folder to be listed.
        assert_eq!(table.kind(), yggdryl::IOKind::Table);
        assert!(table.is_container());
        assert!(table.is_tabular());
        assert!(!table.is_atomic());
        assert_eq!(table.root().kind(), yggdryl::IOKind::Directory);
        assert_eq!(
            IOBase::url(&table).unwrap().to_string(),
            LocalFolder::new(&path).unwrap().url().to_string()
        );
        assert!(table.child_by_path("metadata").is_ok());

        // The record surface is answered before a single data file exists:
        // the encoding from what this module writes, the schema from the
        // metadata - field identifiers included - never off decoded batches.
        let options = yggdryl::IOMedia::record_options(&table).unwrap();
        assert_eq!(options.mime_type(), yggdryl::MimeType::PARQUET);
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
                yggdryl::arrow::batch_reader(batch.schema(), [batch]),
                &options,
            )
            .unwrap();
        let batch = trades(&[3], &[Some("VOD")], &[Some("XLON")]);
        table
            .append_arrow_reader(
                yggdryl::arrow::batch_reader(batch.schema(), [batch]),
                &options,
            )
            .unwrap();
        assert_eq!(table.metadata().snapshots().len(), 2);
        assert_eq!(table.current_snapshot().unwrap().operation(), "append");
        assert_eq!(collect(table.read_arrow_reader(&options).unwrap()).len(), 3);

        // A partition filter is answered by the plan - the other partitions'
        // files are never opened - and the rows match the folder route's.
        let filtered = options.clone().with_filter("venue = 'XNYS'").unwrap();
        assert_eq!(
            collect(table.read_arrow_reader(&filtered).unwrap()),
            vec![(2, Some("MSFT".to_owned()), Some("XNYS".to_owned()))]
        );
        let plan = table.plan(&[("venue", "XNYS")]).unwrap();
        assert_eq!(plan.tasks.len(), 1);
        assert!(plan.excluded.len() + plan.skipped.len() >= 1);

        // A selection narrows the read to the named columns.
        let selected = options.clone().with_select("id").unwrap();
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
        let unanswerable = options.clone().with_filter("desk = '42'").unwrap();
        assert!(table.read_arrow_reader(&unanswerable).is_err());

        // The folder route reads the same rows through the same snapshot.
        let folder = LocalFolder::new(&path).unwrap();
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
        let mut table = Table::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            schema,
            spec,
        )
        .unwrap();
        let options = yggdryl::IOMedia::record_options(&table).unwrap();

        let batch = trades(
            &[1, 2],
            &[Some("AAPL"), Some("MSFT")],
            &[Some("XNAS"), Some("XNYS")],
        );
        table
            .append_arrow_reader(
                yggdryl::arrow::batch_reader(batch.schema(), [batch]),
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
                yggdryl::arrow::batch_reader(batch.schema(), [batch]),
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
        let merging = options.clone().with_merge_by(["id"]).unwrap();
        let batch = trades(
            &[9, 10],
            &[Some("BP.L"), Some("SHEL")],
            &[Some("XLON"), Some("XLON")],
        );
        table
            .merge_arrow_reader(
                yggdryl::arrow::batch_reader(batch.schema(), [batch]),
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
        let reopened = Table::open(LocalFolder::new(&path).unwrap()).unwrap();
        assert_eq!(reopened.metadata_version(), table.metadata_version());
        assert_eq!(reopened.metadata().snapshots().len(), 3);
    }

    #[test]
    fn the_table_value_honours_the_row_limit_like_every_handle() {
        let path = root("handle-table-limit");
        let schema = trade_schema();
        let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
        let mut table = Table::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            schema,
            spec,
        )
        .unwrap();
        let options = yggdryl::IOMedia::record_options(&table).unwrap();

        // A limited write truncates data the caller offered, so only the
        // first row of the two lands in the commit.
        let batch = trades(
            &[1, 2],
            &[Some("AAPL"), Some("MSFT")],
            &[Some("XNAS"), Some("XNYS")],
        );
        table
            .overwrite_arrow_reader(
                yggdryl::arrow::batch_reader(batch.schema(), [batch]),
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
            .with_merge_by(["id"])
            .unwrap()
            .with_max_row_size(1);
        let batch = trades(&[3], &[Some("VOD")], &[Some("XLON")]);
        let Err(error) = table.merge_arrow_reader(
            yggdryl::arrow::batch_reader(batch.schema(), [batch]),
            &merging,
        ) else {
            panic!("a limited merge must be refused");
        };
        let message = error.to_string();
        assert!(message.contains("max_row_size = 1"), "{message}");
        assert!(message.contains("merge_by `id`"), "{message}");
    }

    #[test]
    fn table_write_limit_preflight_does_not_pull_the_source() {
        let path = root("handle-table-limit-preflight");
        let schema = trade_schema();
        let mut table = Table::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            schema.clone(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let options = yggdryl::IOMedia::record_options(&table)
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
                &options.with_merge_by(["id"]).unwrap().with_max_byte_size(1),
            )
            .unwrap_err()
            .to_string();
        assert!(message.contains("merge_by"), "{message}");
        assert_eq!(pulls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn table_and_leaf_complete_unconvertible_input_onto_stored_fields_the_same_way() {
        let mut stored = StructType::from_fields([DataType::Int64.nullable_field("id")])
            .map(DataType::from)
            .unwrap()
            .required_field("row");
        assign_field_ids(&mut stored, 1).unwrap();
        stored.insert_metadata("ICEBERG:schema-id", "0").unwrap();
        let valid = RecordBatch::try_new(
            stored.clone().into_arrow_schema().unwrap(),
            vec![Arc::new(Int64Array::from(vec![Some(1)]))],
        )
        .unwrap();

        let path = root("handle-table-stored-completion");
        let mut table = Table::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            stored.clone(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let declared_table = yggdryl::IOMedia::record_options(&table)
            .unwrap()
            .with_field(stored.clone());
        table
            .overwrite_arrow_reader(
                yggdryl::arrow::batch_reader(valid.schema(), [valid.clone()]),
                &declared_table,
            )
            .unwrap();

        let mut leaf = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
        let declared_leaf = leaf.record_options().unwrap().with_field(stored.clone());
        leaf.overwrite_arrow_reader(
            yggdryl::arrow::batch_reader(valid.schema(), [valid]),
            &declared_leaf,
        )
        .unwrap();

        let loose = StructType::from_fields([DataType::utf8().nullable_field("id")])
            .map(DataType::from)
            .unwrap()
            .required_field("row");
        let bad = RecordBatch::try_new(
            loose.clone().into_arrow_schema().unwrap(),
            vec![Arc::new(StringArray::from(vec![Some("not-an-integer")]))],
        )
        .unwrap();
        let untyped_table = yggdryl::IOMedia::record_options(&table)
            .unwrap()
            .with_commit_row_size(1);
        table
            .overwrite_arrow_reader(
                yggdryl::arrow::batch_reader(bad.schema(), [bad.clone()]),
                &untyped_table,
            )
            .unwrap();
        let untyped_leaf = leaf.record_options().unwrap().with_commit_row_size(1);
        leaf.overwrite_arrow_reader(
            yggdryl::arrow::batch_reader(bad.schema(), [bad]),
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
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            schema.clone(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let options = yggdryl::IOMedia::record_options(&table)
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
                yggdryl::arrow::batch_reader(batch.schema(), [batch]),
                &options,
            )
            .unwrap();
        assert_eq!(table.metadata().snapshots().len(), 3);
        assert_eq!(collect(table.read_arrow_reader(&options).unwrap()).len(), 3);

        let batch = trades(
            &[4, 5],
            &[Some("BP"), Some("SHEL")],
            &[Some("XLON"), Some("XLON")],
        );
        table
            .append_arrow_reader(
                yggdryl::arrow::batch_reader(batch.schema(), [batch]),
                &options,
            )
            .unwrap();
        assert_eq!(table.metadata().snapshots().len(), 5);

        let merging = options.clone().with_merge_by(["id"]).unwrap();
        let batch = trades(
            &[2, 6],
            &[Some("MSFT.L"), Some("ARM")],
            &[Some("XNYS"), Some("XLON")],
        );
        table
            .merge_arrow_reader(
                yggdryl::arrow::batch_reader(batch.schema(), [batch]),
                &merging,
            )
            .unwrap();
        assert_eq!(table.metadata().snapshots().len(), 7);
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
                yggdryl::arrow::batch_reader(batch.schema(), [batch]),
                &options,
            )
            .unwrap();

        let reopened = Table::open(LocalFolder::new(&path).unwrap()).unwrap();
        assert_eq!(reopened.metadata().snapshots().len(), 3);
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
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            schema.clone(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let options = yggdryl::IOMedia::record_options(&table)
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
        assert_eq!(table.metadata().snapshots().len(), 1);
        assert_eq!(
            collect(table.read_arrow_reader(&options).unwrap()),
            vec![(7, Some("NVDA".to_owned()), Some("XNAS".to_owned()))]
        );
        let reopened = Table::open(LocalFolder::new(&path).unwrap()).unwrap();
        assert_eq!(reopened.metadata().snapshots().len(), 1);
    }

    #[test]
    fn empty_append_and_merge_create_no_table_commit() {
        let path = root("handle-table-empty-write");
        let schema = trade_schema();
        let mut table = Table::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            schema.clone(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let options = yggdryl::IOMedia::record_options(&table)
            .unwrap()
            .with_field(schema.clone());
        let arrow = schema.clone().into_arrow_schema().unwrap();
        let version = table.metadata_version();
        let snapshots = table.metadata().snapshots().len();

        table
            .append_arrow_reader(
                yggdryl::arrow::batch_reader(Arc::clone(&arrow), []),
                &options,
            )
            .unwrap();
        let zero = RecordBatch::new_empty(arrow);
        table
            .merge_arrow_reader(
                yggdryl::arrow::batch_reader(zero.schema(), [zero]),
                &options.with_merge_by(["id"]).unwrap(),
            )
            .unwrap();

        assert_eq!(table.metadata_version(), version);
        assert_eq!(table.metadata().snapshots().len(), snapshots);
    }
}

#[test]
fn time_travel_reads_a_previous_snapshot_by_id_and_by_ref() {
    let path = root("time-travel");
    let mut table = Table::create(
        LocalFolder::new(&path).unwrap(),
        FormatVersion::V2,
        trade_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();

    let first = trades(&[1], &[Some("AAPL")], &[Some("XNAS")]);
    table
        .commit_append(yggdryl::arrow::batch_reader(first.schema(), [first]))
        .unwrap();
    let past = table.current_snapshot().unwrap().snapshot_id;
    let second = trades(&[9], &[Some("NVDA")], &[Some("XNAS")]);
    table
        .commit_overwrite(yggdryl::arrow::batch_reader(second.schema(), [second]))
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
            metadata.set_snapshot_ref("audit", yggdryl::iceberg::SnapshotRef::tag(past))
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
        LocalFolder::new(&path).unwrap(),
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
    let failed: yggdryl::Result<()> = table.commit_metadata_changes(|_| {
        Err(yggdryl::Error::Codec {
            format: "iceberg",
            position: 0,
            reason: smol_str::SmolStr::new_static("rejected"),
        })
    });
    assert!(failed.is_err());
    assert_eq!(table.metadata_version(), version + 1);

    // The written document reads back with the change applied.
    let reopened = Table::open(LocalFolder::new(&path).unwrap()).unwrap();
    assert_eq!(reopened.metadata().property("owner"), Some("desk"));
    assert_eq!(reopened.metadata_version(), version + 1);

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn a_reported_hint_failure_reconciles_to_the_version_fresh_handles_see() {
    let filesystem = Arc::new(PublishedHintFailure::default());
    let folder =
        yggdryl::fs::FsFolder::from_path(filesystem.clone(), "bucket/table", None).unwrap();
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
fn a_reported_hint_failure_keeps_the_data_files_the_published_document_names() {
    let filesystem = Arc::new(PublishedHintFailure::default());
    let folder =
        yggdryl::fs::FsFolder::from_path(filesystem.clone(), "bucket/table", None).unwrap();
    let schema = trade_schema();
    let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
    let mut table = Table::create(folder.clone(), FormatVersion::V2, schema, spec).unwrap();
    let version = table.metadata_version();

    filesystem.arm();
    let batch = trades(
        &[1, 2],
        &[Some("AAPL"), Some("MSFT")],
        &[Some("XNAS"), Some("XNYS")],
    );
    let error = table
        .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
        .unwrap_err();
    assert!(
        error.to_string().contains("acknowledgement failure"),
        "{error}"
    );

    // The versioned document went out before the hint did, and it names the
    // data files, the manifest and the list: from that point nothing is
    // rolled back, whatever the hint write reports, because every fresh
    // handle resolves the version to that document.
    assert_eq!(table.metadata_version(), version + 1);
    let files = table.data_files().unwrap();
    assert_eq!(files.len(), 2, "both data files stay");
    let paths = listed_paths(&folder);
    assert_eq!(
        paths.iter().filter(|path| path.contains("/data/")).count(),
        2,
        "{paths:?}"
    );
    assert_eq!(
        paths.iter().filter(|path| path.ends_with(".avro")).count(),
        2,
        "the manifest and the list stay: {paths:?}"
    );
    let reopened = Table::open(folder).unwrap();
    assert_eq!(reopened.metadata_version(), version + 1);
    assert_eq!(
        collect(reopened.scan(None).unwrap())
            .iter()
            .map(|row| row.0)
            .collect::<Vec<_>>(),
        [1, 2],
        "the rows the published document names are read"
    );
}

#[test]
fn a_refused_document_write_rolls_the_commit_back_with_its_attempt() {
    let filesystem = Arc::new(RefusedDocumentWrite::default());
    let folder =
        yggdryl::fs::FsFolder::from_path(filesystem.clone(), "bucket/table", None).unwrap();
    let schema = trade_schema();
    let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
    let mut table = Table::create(folder.clone(), FormatVersion::V2, schema, spec).unwrap();
    let version = table.metadata_version();

    filesystem.arm();
    let batch = trades(
        &[1, 2],
        &[Some("AAPL"), Some("MSFT")],
        &[Some("XNAS"), Some("XNYS")],
    );
    let error = table
        .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
        .unwrap_err();
    assert!(error.to_string().contains("refused"), "{error}");

    // Nothing durable named the commit's files, so all of them went - the
    // data files, the manifest, the list - and so did the attempt that
    // named them, which left in place would have claimed the version for
    // good.
    assert_eq!(table.metadata_version(), version);
    assert!(table.current_snapshot().is_none());
    let paths = listed_paths(&folder);
    assert!(
        paths.iter().all(|path| !path.contains("/data/")),
        "{paths:?}"
    );
    assert!(
        paths.iter().all(|path| !path.ends_with(".avro")),
        "{paths:?}"
    );
    assert!(
        paths.iter().all(|path| !path.contains("/00002-")),
        "the attempt is removed with the files it named: {paths:?}"
    );

    // The version was not claimed: the next commit takes it.
    let batch = trades(&[3], &[Some("NVDA")], &[Some("XNAS")]);
    table
        .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
        .unwrap();
    assert_eq!(table.metadata_version(), version + 1);
    let reopened = Table::open(folder).unwrap();
    assert_eq!(reopened.metadata_version(), version + 1);
    assert_eq!(
        collect(reopened.scan(None).unwrap())
            .iter()
            .map(|row| row.0)
            .collect::<Vec<_>>(),
        [3]
    );
}

#[test]
fn a_commit_beaten_on_write_withdraws_the_list_of_the_attempt_it_replaces() {
    let filesystem = Arc::new(MetadataOnlyWinner::default());
    let folder =
        yggdryl::fs::FsFolder::from_path(filesystem.clone(), "bucket/table", None).unwrap();
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
    let batch = trades(&[1], &[Some("AAPL")], &[Some("XNAS")]);
    table
        .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
        .unwrap();
    assert_eq!(
        table.metadata_version(),
        3,
        "the rebase adopted the winner's version and committed after it"
    );
    assert_eq!(table.metadata().property("winner"), Some("visible"));

    // The first attempt's list was withdrawn when the retry published its
    // own: one removal, and the metadata directory holds one list - the one
    // the current snapshot names - so a successful commit leaves nothing
    // the metadata does not name either.
    let removed: Vec<String> = filesystem
        .removed()
        .into_iter()
        .filter(|path| path.contains("/metadata/snap-"))
        .collect();
    assert_eq!(removed.len(), 1, "one list is removed: {removed:?}");
    let lists: Vec<String> = listed_paths(&folder)
        .into_iter()
        .filter(|path| path.contains("/metadata/snap-"))
        .collect();
    assert_eq!(lists.len(), 1, "one list is left: {lists:?}");
    let named = table.current_snapshot().unwrap().manifest_list.clone();
    let named = named.rsplit('/').next().unwrap();
    assert!(
        lists[0].ends_with(named),
        "the list left is the one the snapshot names: {lists:?} vs {named}"
    );
    assert!(
        !removed[0].ends_with(named),
        "the list removed is the other one: {removed:?}"
    );
    let reopened = Table::open(folder).unwrap();
    assert_eq!(
        collect(reopened.scan(None).unwrap())
            .iter()
            .map(|row| row.0)
            .collect::<Vec<_>>(),
        [1]
    );
}

#[test]
fn a_same_version_publication_conflict_rebases_through_the_retry_gate() {
    let filesystem = Arc::new(SameVersionWinner::default());
    let folder =
        yggdryl::fs::FsFolder::from_path(filesystem.clone(), "bucket/table", None).unwrap();
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
        LocalFolder::new(&path).unwrap(),
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
        .commit_append(yggdryl::arrow::batch_reader(first.schema(), [first]))
        .unwrap();
    let second = trades(&[3], &[Some("NVDA")], &[Some("XNAS")]);
    table
        .commit_append(yggdryl::arrow::batch_reader(second.schema(), [second]))
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
        LocalFolder::new(&path).unwrap(),
        FormatVersion::V2,
        trade_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    let version = table.metadata_version();

    // A change that leaves the metadata inconsistent never becomes a document.
    let failed = table.commit_metadata_changes(|metadata| {
        *yggdryl::internals::iceberg_metadata::current_schema_id_mut(metadata) = 999;
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
        LocalFolder::new(&path).unwrap(),
        FormatVersion::V2,
        trade_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();

    let empty = trades(&[], &[], &[]);
    table
        .commit_append(yggdryl::arrow::batch_reader(empty.schema(), [empty]))
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
    let mut schema = StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Float64.nullable_field("ratio"),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("row");
    assign_field_ids(&mut schema, 1).unwrap();
    let mut table = Table::create(
        LocalFolder::new(&path).unwrap(),
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
        .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
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
        LocalFolder::new(&path).unwrap(),
        FormatVersion::V2,
        trade_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    let batch = trades(&[1], &[Some("AAPL")], &[Some("XNAS")]);
    table
        .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
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

    let reopened = Table::open(LocalFolder::new(&path).unwrap()).unwrap();
    let message = reopened.plan(&[]).unwrap_err().to_string();
    assert!(!message.is_empty());

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn a_tiny_write_target_rolls_one_append_into_multiple_data_files() {
    let path = root("target-size");
    let mut table = Table::create(
        LocalFolder::new(&path).unwrap(),
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
        .commit_append(yggdryl::arrow::batch_reader(arrow.clone(), batches.clone()))
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
        .commit_append(yggdryl::arrow::batch_reader(arrow, batches))
        .unwrap();
    let files = table.data_files().unwrap();
    assert_eq!(files.len(), 4, "one whole file plus three rolled ones");

    // One running index numbers the files of a partition group - the whole
    // commit, on an unpartitioned table: the rolled commit wrote 00000,
    // 00001, and 00002.
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
        LocalFolder::new(&path).unwrap(),
        FormatVersion::V2,
        schema,
        PartitionSpec::unpartitioned(),
    )
    .unwrap();

    // No table property, so the schema root's `ICEBERG:` property decides.
    assert_eq!(table.target_file_size_bytes().unwrap(), 1);
    let batches: Vec<RecordBatch> = (0..2)
        .map(|id| trades(&[id], &[Some("AAPL")], &[Some("XNAS")]))
        .collect();
    let arrow = batches[0].schema();
    table
        .commit_append(yggdryl::arrow::batch_reader(arrow.clone(), batches.clone()))
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
        .commit_append(yggdryl::arrow::batch_reader(arrow, batches))
        .unwrap();
    assert_eq!(table.data_files().unwrap().len(), 3);

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn an_unparseable_write_target_is_a_typed_error_naming_the_key() {
    let path = root("target-unparseable");
    let mut table = Table::create(
        LocalFolder::new(&path).unwrap(),
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
            yggdryl::Error::InvalidMetadataValue { ref key, .. }
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
            .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
            .is_err()
    );

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn compaction_merges_small_files_and_the_old_snapshot_still_time_travels() {
    let path = root("compact");
    let mut table = Table::create(
        LocalFolder::new(&path).unwrap(),
        FormatVersion::V2,
        trade_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    for id in 0..5_i64 {
        let batch = trades(&[id], &[Some("AAPL")], &[Some("XNAS")]);
        table
            .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap();
    }
    let before_rows = collect(table.scan(None).unwrap());
    assert_eq!(table.data_files().unwrap().len(), 5);
    let past = table.current_snapshot().unwrap().snapshot_id;
    let snapshots_before = table.metadata().snapshots().len();

    let compaction = table.compact().unwrap();
    assert_eq!(compaction.files_before, 5);
    assert_eq!(compaction.files_after, 1);
    assert!(compaction.bytes_rewritten > 0);

    // Fewer files, identical rows, one `replace` snapshot.
    assert_eq!(table.data_files().unwrap().len(), 1);
    assert_eq!(collect(table.scan(None).unwrap()), before_rows);
    assert_eq!(table.metadata().snapshots().len(), snapshots_before + 1);
    assert_eq!(table.current_snapshot().unwrap().operation(), "replace");
    assert_eq!(
        table.metadata().snapshots().last().unwrap().operation(),
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
    assert_eq!(table.compact().unwrap(), Compaction::default());
    assert_eq!(table.metadata_version(), version);
    assert_eq!(table.metadata().snapshots().len(), snapshots_before + 1);

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn compaction_respects_partitions_and_pruning_still_prunes_after_it() {
    let path = root("compact-partitions");
    let schema = trade_schema();
    let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
    let mut table = Table::create(
        LocalFolder::new(&path).unwrap(),
        FormatVersion::V2,
        schema,
        spec,
    )
    .unwrap();

    // Two commits spanning two venues each: four small files, two per venue.
    for id in 0..2_i64 {
        let batch = trades(
            &[2 * id, 2 * id + 1],
            &[Some("AAPL"), Some("MSFT")],
            &[Some("XNAS"), Some("XNYS")],
        );
        table
            .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap();
    }
    // And one venue holding a single file, which no compaction may touch.
    let lone = trades(&[9], &[Some("VOD")], &[Some("XLON")]);
    table
        .commit_append(yggdryl::arrow::batch_reader(lone.schema(), [lone]))
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
    let mut schema = StructType::from_fields(
        (0..300).map(|index| DataType::Int64.nullable_field(format!("column_{index:03}"))),
    )
    .map(DataType::from)
    .unwrap()
    .required_field("row");
    assign_field_ids(&mut schema, 1).unwrap();
    assert_eq!(schema.max_parquet_field_id().unwrap(), Some(300));

    let mut table = Table::create(
        LocalFolder::new(&path).unwrap(),
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
        .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
        .unwrap();

    // Reopening parses the wide schema back and reads every column.
    let reopened = Table::open(LocalFolder::new(&path).unwrap()).unwrap();
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
fn ordered(reader: yggdryl::arrow::BatchReader) -> Vec<(i64, Option<String>, Option<String>)> {
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
    use yggdryl::iceberg::IcebergOptions;

    let path = root("options-layers");
    let mut table = Table::create(
        LocalFolder::new(&path).unwrap(),
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
    assert_eq!(options.read_parallel_min_files(), 2);
    assert_eq!(options.read_parallel_min_file_size_bytes(), 64 * 1024);

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
    use yggdryl::iceberg::IcebergOptions;

    let path = root("commit-total-timeout");
    let mut winner = Table::create(
        LocalFolder::new(&path).unwrap(),
        FormatVersion::V2,
        trade_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    let mut stale = Table::open(LocalFolder::new(&path).unwrap()).unwrap();
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

    let reopened = Table::open(LocalFolder::new(&path).unwrap()).unwrap();
    assert_eq!(reopened.metadata().property("winner"), Some("visible"));
    assert_eq!(reopened.metadata().property("loser"), Some("hidden"));

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn an_unparseable_option_property_is_typed_and_an_explicit_option_shadows_it() {
    use yggdryl::iceberg::IcebergOptions;

    let path = root("options-unparseable");
    let mut table = Table::create(
        LocalFolder::new(&path).unwrap(),
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
            yggdryl::Error::InvalidMetadataValue { ref key, .. } if key == "read.parallelism"
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
    use yggdryl::iceberg::IcebergOptions;

    let path = root("append-conflict");
    let mut first = Table::create(
        LocalFolder::new(&path).unwrap(),
        FormatVersion::V2,
        trade_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    let mut second = Table::open(LocalFolder::new(&path).unwrap()).unwrap();
    second.set_options(
        IcebergOptions::new()
            .with_commit_min_backoff_ms(1)
            .with_commit_max_backoff_ms(2),
    );
    assert_eq!(second.metadata_version(), 1);

    let winner = trades(&[1], &[Some("AAPL")], &[Some("XNAS")]);
    first
        .commit_append(yggdryl::arrow::batch_reader(winner.schema(), [winner]))
        .unwrap();
    // The second handle still describes version 1, so its append is beaten
    // and must rebase onto the winner's commit rather than clobber it.
    let beaten = trades(&[2], &[Some("MSFT")], &[Some("XNYS")]);
    second
        .commit_append(yggdryl::arrow::batch_reader(beaten.schema(), [beaten]))
        .unwrap();
    assert_eq!(
        second.metadata_version(),
        3,
        "the rebase adopted the winner's version"
    );

    // Both rows, two snapshots, and the loser's snapshot parents the winner's.
    let reopened = Table::open(LocalFolder::new(&path).unwrap()).unwrap();
    assert_eq!(reopened.metadata_version(), 3);
    let rows = collect(reopened.scan(None).unwrap());
    assert_eq!(rows.iter().map(|row| row.0).collect::<Vec<_>>(), [1, 2]);
    assert_eq!(reopened.metadata().snapshots().len(), 2);
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
    use yggdryl::iceberg::IcebergOptions;

    let path = root("changes-conflict");
    let mut first = Table::create(
        LocalFolder::new(&path).unwrap(),
        FormatVersion::V2,
        trade_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    let mut second = Table::open(LocalFolder::new(&path).unwrap()).unwrap();
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

    let reopened = Table::open(LocalFolder::new(&path).unwrap()).unwrap();
    assert_eq!(reopened.metadata_version(), 3);
    assert_eq!(reopened.metadata().property("owner"), Some("alpha"));
    assert_eq!(reopened.metadata().property("team"), Some("beta"));

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn a_beaten_overwrite_exhausts_its_retries_into_a_conflict_naming_versions() {
    use yggdryl::iceberg::IcebergOptions;

    let path = root("overwrite-conflict");
    let mut first = Table::create(
        LocalFolder::new(&path).unwrap(),
        FormatVersion::V2,
        trade_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    let stored = trades(&[1], &[Some("AAPL")], &[Some("XNAS")]);
    first
        .commit_append(yggdryl::arrow::batch_reader(stored.schema(), [stored]))
        .unwrap();

    // The second handle plans against version 2; the first then commits twice
    // more, so the overwrite's plan is two commits stale.
    let mut second = Table::open(LocalFolder::new(&path).unwrap()).unwrap();
    second.set_options(
        IcebergOptions::new()
            .with_commit_retries(1)
            .with_commit_min_backoff_ms(1)
            .with_commit_max_backoff_ms(1),
    );
    for id in [2_i64, 3] {
        let batch = trades(&[id], &[Some("NVDA")], &[Some("XNAS")]);
        first
            .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap();
    }

    let incoming = trades(&[9], &[Some("VOD")], &[Some("XLON")]);
    let error = second
        .commit_overwrite(yggdryl::arrow::batch_reader(incoming.schema(), [incoming]))
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
    let reopened = Table::open(LocalFolder::new(&path).unwrap()).unwrap();
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
    use yggdryl::iceberg::IcebergOptions;

    let path = root("parallel-read");
    let mut table = Table::create(
        LocalFolder::new(&path).unwrap(),
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
            .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
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
        LocalFolder::new(&path).unwrap(),
        FormatVersion::V2,
        trade_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    let first = trades(&[1], &[Some("AAPL")], &[Some("XNAS")]);
    table
        .commit_append(yggdryl::arrow::batch_reader(first.schema(), [first]))
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
        .commit_append(yggdryl::arrow::batch_reader(second.schema(), [second]))
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
    let cutoff = table.metadata().last_updated_ms() + 10_000;
    let expired = table.expire_snapshots(Some(cutoff), None, &[]).unwrap();
    assert_eq!(expired, vec![past]);
    assert_eq!(table.metadata().snapshots().len(), 1);
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

    use yggdryl::{Scalar, TimeUnit};

    /// Append `rows` under `children`, scan them back, and return the records.
    fn round_trip(label: &str, children: Vec<Field>, rows: &[Vec<Scalar>]) -> Vec<Scalar> {
        let path = root(label);
        let schema = StructType::from_fields(children.clone())
            .map(DataType::from)
            .unwrap()
            .required_field("row");
        let mut table = Table::create(
            LocalFolder::new(&path).unwrap(),
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
                column_of_rows(child, &column).unwrap()
            })
            .collect();
        let batch = arrow_array::RecordBatch::try_new(schema.into_arrow_schema().unwrap(), columns)
            .unwrap();
        table
            .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap();

        let reopened = Table::open(LocalFolder::new(&path).unwrap()).unwrap();
        let mut records = Vec::new();
        for batch in reopened.scan(None).unwrap() {
            let landed = yggdryl::Serie::from_arrow_batch(
                None,
                &batch.unwrap(),
                yggdryl::ArrowCastOptions::default(),
            )
            .unwrap();
            let value = Scalar::from(landed);
            records.extend(value.sequence_rows().unwrap().iter().cloned());
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
            DataType::date32().nullable_field("day"),
            DataType::Time64(TimeUnit::Microsecond).nullable_field("tod"),
            DataType::DateTime64 {
                unit: TimeUnit::Microsecond,
                timezone: yggdryl::Timezone::NAIVE,
            }
            .nullable_field("at"),
            DataType::utf8().nullable_field("name"),
            DataType::binary().nullable_field("raw"),
            DataType::fixed_binary(4).unwrap().nullable_field("tag"),
        ];
        let rows = vec![
            vec![
                Scalar::from(true),
                Scalar::from(41),
                Scalar::from(1),
                Scalar::from(0.5_f32),
                Scalar::from(2.25_f64),
                Scalar::d128(1_500_000, 4),
                Scalar::date32(20_000),
                Scalar::time64(
                    43_200_000_000,
                    TimeUnit::Microsecond,
                    yggdryl::Timezone::NAIVE,
                )
                .unwrap(),
                Scalar::datetime64(
                    1_700_000_000_000_000,
                    TimeUnit::Microsecond,
                    yggdryl::Timezone::NAIVE,
                )
                .unwrap(),
                Scalar::from("alpha"),
                Scalar::from(vec![1_u8, 2]),
                Scalar::from(vec![9_u8, 9, 9, 9]),
            ],
            // A row of nulls proves every column's null path through Parquet.
            vec![
                Scalar::Null,
                Scalar::Null,
                Scalar::from(2),
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
        assert_eq!(first[0], Scalar::from(true));
        assert_eq!(first[2], Scalar::from(1));
        assert_eq!(first[5], Scalar::d128(1_500_000, 4));
        assert_eq!(
            first[8],
            Scalar::datetime64(
                1_700_000_000_000_000,
                TimeUnit::Microsecond,
                yggdryl::Timezone::NAIVE,
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
        let point = StructType::from_fields([
            DataType::Int64.required_field("x"),
            DataType::utf8().nullable_field("label"),
        ])
        .map(DataType::from)
        .unwrap();
        let deep = StructType::from_fields([
            DataType::List(Arc::new(DataType::Int64.nullable_field("item"))).nullable_field("xs"),
            DataType::map_of(DataType::utf8(), point.clone(), false)
                .unwrap()
                .nullable_field("m"),
        ])
        .map(DataType::from)
        .unwrap();
        let children = vec![
            DataType::Int64.required_field("id"),
            point.clone().nullable_field("p"),
            DataType::List(Arc::new(deep.clone().nullable_field("item"))).nullable_field("rows"),
        ];

        let point_value =
            |x: i64, label: &str| Scalar::from_sequence([Scalar::from(x), Scalar::from(label)]);
        let deep_value = Scalar::from_sequence([
            Scalar::from_sequence([Scalar::from(1), Scalar::Null, Scalar::from(3)]),
            Scalar::from_mapping([(Scalar::from("origin"), point_value(0, "o"))]).unwrap(),
        ]);
        let rows = vec![vec![
            Scalar::from(1),
            point_value(7, "seven"),
            Scalar::from_sequence([deep_value]),
        ]];

        let records = round_trip("types-nested", children, &rows);
        assert_eq!(records.len(), 1);
        let values = records[0].as_sequence().unwrap();
        // The struct survives with both children.
        let fields = values[1].as_sequence().expect("a struct row");
        assert_eq!(fields[0], Scalar::from(7));
        assert_eq!(fields[1], Scalar::from("seven"));
        // The deep list<struct<list, map<utf8, struct>>> survives one level in.
        let outer = values[2].as_sequence().expect("a list of deep rows");
        assert_eq!(outer.len(), 1);
    }

    #[test]
    fn a_merge_updates_on_a_composite_key_of_mixed_types() {
        let path = root("types-merge");
        let children = vec![
            DataType::Int64.required_field("id"),
            DataType::utf8().required_field("venue"),
            DataType::Decimal128 {
                precision: 18,
                scale: 4,
            }
            .nullable_field("price"),
        ];
        let schema = StructType::from_fields(children.clone())
            .map(DataType::from)
            .unwrap()
            .required_field("row");
        let mut table = Table::create(
            LocalFolder::new(&path).unwrap(),
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
                            0 => Scalar::from(*id),
                            1 => Scalar::from(*venue),
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
                    column_of_rows(child, &refs).unwrap()
                })
                .collect();
            arrow_array::RecordBatch::try_new(schema.clone().into_arrow_schema().unwrap(), arrays)
                .unwrap()
        };

        let first = batch_of(&[(1, "XNAS", 10_000), (2, "XNYS", 20_000)]);
        table
            .commit_append(yggdryl::arrow::batch_reader(first.schema(), [first]))
            .unwrap();
        // The same (id, venue) updates; a new pair appends.
        let second = batch_of(&[(1, "XNAS", 99_000), (3, "XPAR", 30_000)]);
        table
            .commit_merge(
                yggdryl::arrow::batch_reader(second.schema(), [second]),
                &yggdryl::Selector::from_columns(["id", "venue"]),
                false,
            )
            .unwrap();

        let mut prices = std::collections::BTreeMap::new();
        for batch in table.scan(None).unwrap() {
            let landed = yggdryl::Serie::from_arrow_batch(
                None,
                &batch.unwrap(),
                yggdryl::ArrowCastOptions::default(),
            )
            .unwrap();
            let value = Scalar::from(landed);
            for row in value.sequence_rows().unwrap().iter() {
                let fields = row.as_sequence().unwrap();
                let Some(id) = fields[0].as_i64() else {
                    panic!()
                };
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
            LocalFolder::new(&path).unwrap(),
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
                    let mut table = Table::open(LocalFolder::new(&path).unwrap()).unwrap();
                    opened.wait();
                    let batch = trades(
                        &[writer * 10, writer * 10 + 1],
                        &[Some("S"), Some("S")],
                        &[Some("V"), Some("V")],
                    );
                    let _held = gate.lock().unwrap();
                    table
                        .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
                        .unwrap();
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }

        let reopened = Table::open(LocalFolder::new(&path).unwrap()).unwrap();
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
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            schema,
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let seed = trades(&[1, 2], &[Some("A"), Some("B")], &[Some("V"), Some("V")]);
        writer
            .commit_append(yggdryl::arrow::batch_reader(seed.schema(), [seed]))
            .unwrap();

        // A second handle grows stale the moment the first commits again.
        let mut stale = Table::open(LocalFolder::new(&path).unwrap()).unwrap();
        stale.set_options(yggdryl::iceberg::IcebergOptions::default().with_commit_retries(1));
        let win = trades(&[3], &[Some("C")], &[Some("V")]);
        writer
            .commit_append(yggdryl::arrow::batch_reader(win.schema(), [win]))
            .unwrap();

        let incoming = trades(&[2], &[Some("B2")], &[Some("V")]);
        let error = stale
            .commit_merge(
                yggdryl::arrow::batch_reader(incoming.schema(), [incoming]),
                &yggdryl::Selector::from_columns(["id"]),
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
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            schema,
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        table
            .set_options(yggdryl::iceberg::IcebergOptions::default().with_compact_after_commits(2));

        for id in 0..4_i64 {
            let batch = trades(&[id], &[Some("S")], &[Some("V")]);
            table
                .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
                .unwrap();
        }

        // Two cadence points passed, so replace snapshots appear on their own
        // and the live file count shrank below one-per-append.
        let operations: Vec<String> = table
            .metadata()
            .snapshots()
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
    use arrow_array::{Int64Array, StringArray};

    use super::*;
    use yggdryl::IOMedia;
    use yggdryl::Url;
    use yggdryl::holder::Buffer;
    use yggdryl::media::{IORecordOptions, RecordOptions};
    use yggdryl::text::TextOptions;

    const HEADER: &str = r"^\[(?<level>[A-Z]+)\] id=(?<id>\d+)\s*";

    fn named(bytes: &[u8]) -> Buffer {
        let mut handle = Buffer::new()
            .with_media_type(Url::from_str("file:///events.log").unwrap().media_type());
        handle.write_all_bytes(bytes).unwrap();
        handle
    }

    #[test]
    fn regex_typed_text_rows_stream_into_a_table_through_record_media() {
        let path = root("text-record-media");
        let source = named(b"[INFO] id=7 first  \n[WARN] id=42 second\n");
        let mut options: RecordOptions = TextOptions::new()
            .try_with_rowheader(HEADER)
            .unwrap()
            .try_with_rstrip([r"\s+$"])
            .unwrap()
            .into();
        options.set_batch_row_size(Some(2));

        // The row opens with the eighteen event columns, two of them the
        // `uint64` codes Iceberg has no type for: the table takes the
        // schema as the scheme widens it, `decimal(20, 0)` for those, as a
        // FIX row's table does.
        let mut schema = source
            .read_arrow_field(&options)
            .unwrap()
            .into_scheme_compat(&yggdryl::Scheme::ICEBERG)
            .unwrap();
        assert_eq!(
            schema.get_field_by_path("id").unwrap().dtype(),
            &yggdryl::DataType::Int64
        );
        assert_eq!(
            schema.get_field_by_path("currhashcode").unwrap().dtype(),
            &yggdryl::DataType::decimal128(20, 0).unwrap()
        );
        schema.assign_parquet_field_ids(1).unwrap();

        let catalog =
            yggdryl::iceberg::Catalog::new(LocalFolder::new(path.join("warehouse")).unwrap());
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
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(ids.values(), &[7, 42]);
        assert_eq!(
            levels.iter().collect::<Vec<_>>(),
            [Some("INFO"), Some("WARN")]
        );
        // The body is the retained record past its row header, the edges
        // stripped; the captures are their own columns.
        assert_eq!(
            bodies.iter().collect::<Vec<_>>(),
            [Some("first"), Some("second")]
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
    use super::trade_schema;
    use yggdryl::IOBase;
    use yggdryl::Scalar;
    use yggdryl::holder::Buffer;
    use yggdryl::iceberg::{
        DataFile, FormatVersion, ManifestEntry, PartitionSpec, read_manifest,
        read_manifest_for_plan, write_manifest,
    };

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
            // A float bound counts only beside a NaN count of zero.
            assert_eq!(
                full.data_file.nan_value_counts,
                pruned.data_file.nan_value_counts
            );
            // ...and skips what it never reads.
            assert!(pruned.data_file.column_sizes.is_empty());
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
        let schema = yggdryl::json::from_utf8(
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
        let row = yggdryl::json::from_utf8(
            r#"{"status":1,"snapshot_id":77,"data_file":{
                "file_path":"file:///t/data/part-0.parquet",
                "column_sizes":[{"key":1,"value":512}],
                "value_counts":[{"key":1,"value":100}]}}"#,
        )
        .unwrap();
        let mut handle = Buffer::new();
        yggdryl::avro::write_container(&mut handle, &schema, &[], &[row]).unwrap();

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
    use yggdryl::MimeType;
    use yggdryl::iceberg::IcebergOptions;

    /// The manifests' `(mime_type, path)` pairs of the current snapshot.
    fn formats(table: &Table<LocalFolder>) -> Vec<(MimeType, String)> {
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
            LocalFolder::new(&path).unwrap(),
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
            LocalFolder::new(&path).unwrap(),
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
            .commit_append(yggdryl::arrow::batch_reader(
                batch.schema(),
                [batch.clone()],
            ))
            .unwrap_err()
            .to_string();
        assert!(message.contains("write.format.default"), "{message}");
        table.set_options(
            IcebergOptions::new()
                .try_with_data_mime_type(MimeType::PARQUET)
                .unwrap(),
        );
        table
            .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap();
    }

    #[test]
    fn metadata_only_formats_are_refused_up_front_naming_the_format() {
        for mime_type in [MimeType::ORC, MimeType::PUFFIN] {
            let name = mime_type.extension().unwrap();
            let path = root(&format!("format-{}", name.to_ascii_lowercase()));
            let mut table = Table::create(
                LocalFolder::new(&path).unwrap(),
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
                .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
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
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            trade_schema(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();

        // One Parquet append, then one Avro append via the explicit option.
        let first = trades(&[1], &[Some("AAPL")], &[Some("XNAS")]);
        table
            .commit_append(yggdryl::arrow::batch_reader(first.schema(), [first]))
            .unwrap();
        table.set_options(
            IcebergOptions::new()
                .try_with_data_mime_type(MimeType::AVRO)
                .unwrap(),
        );
        let second = trades(&[2], &[None], &[None]);
        table
            .commit_append(yggdryl::arrow::batch_reader(second.schema(), [second]))
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
            LocalFolder::new(&path).unwrap(),
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
            .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap();

        let recorded = formats(&table);
        assert!(
            recorded
                .iter()
                .all(|(mime_type, _)| mime_type == &MimeType::AVRO),
            "{recorded:?}"
        );
        // A fresh open reads the mixed chain from storage alone.
        let reopened = Table::open(LocalFolder::new(&path).unwrap()).unwrap();
        assert_eq!(collect(reopened.scan(None).unwrap()).len(), 2);
    }
}

/// Regressions the Spark interop exchange surfaced, pinned here.
mod interop_regressions {
    use super::*;
    use yggdryl::iceberg::{PartitionField, SchemaUpdate};

    /// A file written before a rename reads under the new name: Iceberg
    /// resolves a column by field id, not by name, so the old file's
    /// `symbol` column is the schema's `ticker` column.
    #[test]
    fn a_scan_resolves_renamed_columns_by_field_id() {
        let path = root("rename-by-id");
        let mut table = Table::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            trade_schema(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let batch = trades(&[1], &[Some("AAPL")], &[Some("XNAS")]);
        table
            .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
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
                    transform: yggdryl::iceberg::Transform::Identity,
                },
                PartitionField {
                    source_id: 1,
                    field_id: 1001,
                    name: "id_bucket".into(),
                    transform: yggdryl::iceberg::Transform::Bucket(4),
                },
            ],
        };
        let file = yggdryl::iceberg::DataFile {
            partition: vec![Scalar::from("AAPL"), Scalar::from(3_i64)],
            ..Default::default()
        };

        let restored =
            yggdryl::internals::iceberg_scan::partition_columns(&spec, &schema, &file).unwrap();
        // Only the identity field restores; the bucket value stays in the
        // manifest where it belongs.
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].0.name(), "symbol");
        assert_eq!(restored[0].1, Scalar::from("AAPL"));
    }
}

#[test]
fn a_uuid_column_keeps_its_type_through_a_round_trip() {
    // `uuid` and `fixed[16]` were one physical type until `uuid` became a
    // datatype of its own; the spelling now survives in the datatype, so
    // rewriting another writer's metadata cannot demote the column. Surfaced
    // by the Spark interop exchange.
    let document = yggdryl::json::from_bytes(
        br#"{"type":"struct","schema-id":0,"fields":[
            {"id":1,"name":"id","required":true,"type":"long"},
            {"id":2,"name":"u","required":false,"type":"uuid"},
            {"id":3,"name":"f","required":false,"type":"fixed[16]"}]}"#,
    )
    .unwrap();
    let root = schema_from_json("row", &document).unwrap();
    let emitted = schema_into_json(&root).unwrap();
    let rendered = String::from_utf8(yggdryl::json::into_bytes(&emitted).unwrap()).unwrap();
    assert!(rendered.contains(r#""type":"uuid""#), "{rendered}");
    assert!(rendered.contains(r#""type":"fixed[16]""#), "{rendered}");
}

/// Partition isolation, default keys, sorted files, parallel writes, and the
/// v3 types: the private half of the contract `docs/media/index.md` (Iceberg)
/// states, pinned where the plan, the grouping, and the data-file handles
/// are visible.
mod isolation {
    use yggdryl::StructType;

    use std::sync::{Arc, Mutex};

    use arrow_array::{
        Array, ArrayRef, BinaryArray, Int64Array, NullArray, RecordBatch, TimestampMicrosecondArray,
    };

    use super::{
        FormatVersion, IcebergOptions, PartitionSpec, SortField, SortOrder, Table, Transform,
        assign_field_ids, collect, root, schema_from_json, schema_into_json, trade_schema, trades,
    };

    use yggdryl::holder::{Buffer, Holder};
    use yggdryl::internals::iceberg_table::child_at;
    use yggdryl::local::LocalFolder;
    use yggdryl::media::{IORecordOptions, RecordOptions};
    use yggdryl::{DataType, Field, IOBase, IOMedia, Scalar, TimeUnit, Timezone};

    /// A table folder that records every relative path resolved through it.
    ///
    /// The table reaches every data file, manifest, and metadata document
    /// with one `child_by_path` on its root, so the paths seen here are the
    /// files a read opened: a merge into one partition must resolve no data
    /// file of another. `fail_after` turns the Nth partition writer's own root
    /// into a handle no file can be written through, which is how a worker
    /// failure is made deterministic.
    struct Recording {
        inner: LocalFolder,
        seen: Arc<Mutex<Vec<String>>>,
        roots: Arc<Mutex<usize>>,
        fail_after: Option<usize>,
    }

    impl Recording {
        fn new(path: &std::path::Path) -> Self {
            Self {
                inner: LocalFolder::new(path).unwrap(),
                seen: Arc::new(Mutex::new(Vec::new())),
                roots: Arc::new(Mutex::new(0)),
                fail_after: None,
            }
        }

        /// The `data/` paths resolved since the last reset, sorted.
        fn data_files(&self) -> Vec<String> {
            let mut seen: Vec<String> = self
                .seen
                .lock()
                .unwrap()
                .iter()
                .filter(|path| path.starts_with("data/") && !path.ends_with('/'))
                .cloned()
                .collect();
            seen.sort();
            seen.dedup();
            seen
        }

        fn reset(&self) {
            self.seen.lock().unwrap().clear();
        }
    }

    impl std::fmt::Debug for Recording {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.debug_struct("Recording").finish_non_exhaustive()
        }
    }

    impl IOMedia for Recording {
        yggdryl::impl_default_iomedia!();
    }

    impl IOBase for Recording {
        yggdryl::delegate_iobase!(inner: pread, read_all_bytes, read_range_bytes, pstream_bytes,
            pwrite, size, capacity, reserve, truncate, uri, url, bound_location, mtime, media_type,
            set_media_type, flush, open, opened, close, parent, ls, kind, clear, remove,
            is_atomic, is_tabular, is_io);

        fn child_by_path(&self, path: &str) -> yggdryl::Result<Holder> {
            if path == "." {
                let mut roots = self.roots.lock().unwrap();
                *roots += 1;
                if self.fail_after.is_some_and(|limit| *roots > limit) {
                    // A byte buffer has no children, so the writer that gets
                    // this root fails at its first data file.
                    return Ok(Holder::Buffer(Buffer::new()));
                }
            }
            self.seen.lock().unwrap().push(path.to_owned());
            self.inner.child_by_path(path)
        }
    }

    /// One row per venue, one commit per row: three partitions, three files.
    fn venues(label: &str) -> (std::path::PathBuf, Table<LocalFolder>) {
        let path = root(label);
        let schema = trade_schema();
        let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
        let mut table = Table::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            schema,
            spec,
        )
        .unwrap();
        for (id, symbol, venue) in [
            (1_i64, "AAPL", "XNAS"),
            (2, "MSFT", "XNYS"),
            (3, "VOD", "XLON"),
        ] {
            let batch = trades(&[id], &[Some(symbol)], &[Some(venue)]);
            table
                .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
                .unwrap();
        }
        (path, table)
    }

    fn file_paths(table: &Table<impl IOBase>) -> Vec<String> {
        let mut paths: Vec<String> = table
            .data_files()
            .unwrap()
            .into_iter()
            .map(|(file, _)| file.file_path.to_string())
            .collect();
        paths.sort();
        paths
    }

    fn rows_of(reader: yggdryl::arrow::BatchReader) -> Vec<(i64, Option<String>, Option<String>)> {
        let mut rows = collect(reader);
        rows.sort();
        rows
    }

    /// The `id` column of every batch, in the order the scan yields rows.
    fn ids_in_order(reader: yggdryl::arrow::BatchReader) -> Vec<i64> {
        let mut ids = Vec::new();
        for batch in reader {
            let batch = batch.unwrap();
            let column = batch
                .column_by_name("id")
                .unwrap()
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap();
            ids.extend(column.values().iter().copied());
        }
        ids
    }

    #[test]
    fn a_range_and_a_membership_prune_exactly_as_an_equality_does() {
        let (_path, table) = venues("isolation-expressions");

        // The reference: one venue by equality skips two manifests outright.
        let equal = table.plan_matching("venue = 'XNAS'").unwrap();
        assert_eq!(equal.tasks.len(), 1);
        assert_eq!(equal.manifests_skipped(), 2);
        assert_eq!(equal.files_skipped(), 0);

        // Two venues by membership: two manifests read, one skipped.
        let members = table.plan_matching("venue in ('XNAS', 'XLON')").unwrap();
        assert_eq!(members.tasks.len(), 2);
        assert_eq!(members.manifests_skipped(), 1);
        assert_eq!(members.files_skipped(), 0);

        // A range over the text: XLON < XNAS < XNYS, so the range keeps two.
        let ranged = table
            .plan_matching("venue between 'XLON' and 'XNAS'")
            .unwrap();
        assert_eq!(ranged.tasks.len(), 2);
        assert_eq!(ranged.manifests_skipped(), 1);

        // A null test prunes too: no partition is null, so nothing is read.
        let nulls = table.plan_matching("venue is null").unwrap();
        assert_eq!(nulls.tasks.len(), 0);
        assert_eq!(nulls.manifests_skipped(), 3);
    }

    #[test]
    fn a_time_partition_range_skips_manifests_like_the_equality_form() {
        let filesystem: Arc<dyn yggdryl::fs::FileSystem> =
            Arc::new(yggdryl::fs::MemoryFileSystem::new());
        filesystem
            .create_dir("isolation-timepartition", true)
            .unwrap();
        let folder =
            yggdryl::fs::FsFolder::from_path(filesystem, "isolation-timepartition", None).unwrap();
        let mut schema = StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::DateTime64 {
                unit: TimeUnit::Microsecond,
                timezone: Timezone::UTC,
            }
            .required_field("timepartition"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
        assign_field_ids(&mut schema, 1).unwrap();
        let spec = PartitionSpec::identity(1, &schema, &["timepartition"]).unwrap();
        let mut table = Table::create(folder, FormatVersion::V2, schema.clone(), spec).unwrap();
        table.set_options(
            IcebergOptions::new()
                .try_with_write_staging(yggdryl::iceberg::WriteStaging::Off)
                .unwrap(),
        );
        let arrow = schema.into_arrow_schema().unwrap();
        let hour = 3_600_000_000_i64;
        // Three hourly partitions from 2024-01-01T00:00Z, one commit each.
        for index in 0..3_i64 {
            let batch = RecordBatch::try_new(
                Arc::clone(&arrow),
                vec![
                    Arc::new(Int64Array::from(vec![index])),
                    Arc::new(
                        TimestampMicrosecondArray::from(vec![1_704_067_200_000_000 + index * hour])
                            .with_timezone("UTC"),
                    ),
                ],
            )
            .unwrap();
            table
                .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
                .unwrap();
        }

        let equal = table
            .plan_matching("timepartition = '2024-01-01T01:00:00Z'")
            .unwrap();
        assert_eq!(equal.tasks.len(), 1);
        assert_eq!(equal.manifests_skipped(), 2);

        let ranged = table
            .plan_matching(
                "timepartition between '2024-01-01T01:00:00Z' and '2024-01-01T01:59:59Z'",
            )
            .unwrap();
        assert_eq!(ranged.tasks.len(), 1, "{ranged:?}");
        assert_eq!(ranged.manifests_skipped(), 2);

        let wide = table
            .plan_matching("timepartition >= '2024-01-01T01:00:00Z'")
            .unwrap();
        assert_eq!(wide.tasks.len(), 2);
        assert_eq!(wide.manifests_skipped(), 1);
    }

    #[test]
    fn a_record_read_pushes_the_whole_where_into_the_plan_and_opens_only_survivors() {
        let (path, _) = venues("isolation-record-read");
        let recording = Recording::new(&path);
        let table = Table::open(recording).unwrap();
        let options = IOMedia::record_options(&table)
            .unwrap()
            .with_filter("venue in ('XNAS', 'XLON') and id >= 1")
            .unwrap();

        table.root().reset();
        let rows = rows_of(IOMedia::read_arrow_reader(&table, &options).unwrap());
        assert_eq!(
            rows,
            vec![
                (1, Some("AAPL".to_owned()), Some("XNAS".to_owned())),
                (3, Some("VOD".to_owned()), Some("XLON".to_owned())),
            ]
        );
        let opened = table.root().data_files();
        assert_eq!(opened.len(), 2, "{opened:?}");
        assert!(
            opened.iter().all(|file| !file.contains("venue=XNYS")),
            "{opened:?}"
        );

        // The select narrows what each file decodes: only `id` and the
        // filter's own `venue` are read, and the reader reports `id` alone.
        let narrowed = IOMedia::record_options(&table)
            .unwrap()
            .with_filter("venue = 'XLON'")
            .unwrap()
            .with_select("id")
            .unwrap();
        let mut reader = IOMedia::read_arrow_reader(&table, &narrowed).unwrap();
        let batch = reader.next().unwrap().unwrap();
        assert_eq!(batch.schema().fields().len(), 1);
        assert_eq!(batch.schema().field(0).name(), "id");
        assert_eq!(batch.num_rows(), 1);

        // A where over a select alias runs after the projection, so the
        // scan is unfiltered and the alias still answers.
        let aliased = IOMedia::record_options(&table)
            .unwrap()
            .with_select("id as trade")
            .unwrap()
            .with_filter("trade = 2")
            .unwrap();
        let rows: usize = IOMedia::read_arrow_reader(&table, &aliased)
            .unwrap()
            .map(|batch| batch.unwrap().num_rows())
            .sum();
        assert_eq!(rows, 1);

        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn a_merge_into_one_partition_reads_no_other_partition_and_carries_their_files() {
        let (path, _) = venues("isolation-merge");
        let mut table = Table::open(Recording::new(&path)).unwrap();
        let before = file_paths(&table);
        assert_eq!(before.len(), 3);

        table.root().reset();
        let batch = trades(&[1, 4], &[Some("AAPL.O"), Some("NVDA")], &[Some("XNAS"); 2]);
        table
            .commit_merge(
                yggdryl::arrow::batch_reader(batch.schema(), [batch]),
                &yggdryl::Selector::from_columns(["id"]),
                true,
            )
            .unwrap();

        // The only data file resolved on the way was XNAS's own.
        let opened = table.root().data_files();
        assert_eq!(opened.len(), 1, "{opened:?}");
        assert!(opened[0].contains("venue=XNAS"), "{opened:?}");

        // The other partitions' files are carried under their exact paths.
        let after = file_paths(&table);
        let carried: Vec<&String> = before.iter().filter(|p| after.contains(p)).collect();
        assert_eq!(carried.len(), 2, "before {before:?} after {after:?}");
        assert!(carried.iter().all(|p| !p.contains("venue=XNAS")));
        assert_eq!(after.len(), 3);

        // 1 updated in place, 4 appended, 2 and 3 untouched.
        assert_eq!(
            rows_of(table.scan(None).unwrap()),
            vec![
                (1, Some("AAPL.O".to_owned()), Some("XNAS".to_owned())),
                (2, Some("MSFT".to_owned()), Some("XNYS".to_owned())),
                (3, Some("VOD".to_owned()), Some("XLON".to_owned())),
                (4, Some("NVDA".to_owned()), Some("XNAS".to_owned())),
            ]
        );
        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn the_partition_columns_lead_the_key_so_a_moved_row_is_a_new_row() {
        let (path, mut table) = venues("isolation-moved-row");
        // Key = (venue, id): id 2 under XLON is not id 2 under XNYS.
        let batch = trades(&[2], &[Some("MSFT.L")], &[Some("XLON")]);
        table
            .commit_merge(
                yggdryl::arrow::batch_reader(batch.schema(), [batch]),
                &yggdryl::Selector::from_columns(["id"]),
                true,
            )
            .unwrap();
        assert_eq!(
            rows_of(table.scan(None).unwrap()),
            vec![
                (1, Some("AAPL".to_owned()), Some("XNAS".to_owned())),
                (2, Some("MSFT".to_owned()), Some("XNYS".to_owned())),
                (2, Some("MSFT.L".to_owned()), Some("XLON".to_owned())),
                (3, Some("VOD".to_owned()), Some("XLON".to_owned())),
            ]
        );
        // Naming the partition column in the key changes nothing: it is
        // there already, once.
        let batch = trades(&[2], &[Some("MSFT.LN")], &[Some("XLON")]);
        table
            .commit_merge(
                yggdryl::arrow::batch_reader(batch.schema(), [batch]),
                &yggdryl::Selector::from_columns(["venue", "id"]),
                true,
            )
            .unwrap();
        let rows = rows_of(table.scan(None).unwrap());
        assert_eq!(rows.len(), 4);
        assert!(rows.contains(&(2, Some("MSFT.LN".to_owned()), Some("XLON".to_owned()))));
        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn a_keyless_merge_replaces_the_partitions_the_rows_fall_in() {
        let (path, mut table) = venues("isolation-keyless");
        let before = file_paths(&table);
        let batch = trades(&[7, 8], &[Some("BP"), Some("HSBA")], &[Some("XLON"); 2]);
        table
            .commit_merge(
                yggdryl::arrow::batch_reader(batch.schema(), [batch]),
                &yggdryl::Selector::all(),
                true,
            )
            .unwrap();
        let after = file_paths(&table);
        assert_eq!(before.iter().filter(|p| after.contains(p)).count(), 2);
        assert_eq!(
            rows_of(table.scan(None).unwrap()),
            vec![
                (1, Some("AAPL".to_owned()), Some("XNAS".to_owned())),
                (2, Some("MSFT".to_owned()), Some("XNYS".to_owned())),
                (7, Some("BP".to_owned()), Some("XLON".to_owned())),
                (8, Some("HSBA".to_owned()), Some("XLON".to_owned())),
            ]
        );

        // An unpartitioned table has no key at all without one named.
        let flat = root("isolation-keyless-flat");
        let mut flat_table = Table::create(
            LocalFolder::new(&flat).unwrap(),
            FormatVersion::V2,
            trade_schema(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let batch = trades(&[1], &[Some("AAPL")], &[Some("XNAS")]);
        let message = flat_table
            .commit_merge(
                yggdryl::arrow::batch_reader(batch.schema(), [batch]),
                &yggdryl::Selector::all(),
                true,
            )
            .unwrap_err()
            .to_string();
        assert!(message.contains("empty match key"), "{message}");
        assert!(flat_table.current_snapshot().is_none());
        let _ = std::fs::remove_dir_all(&path);
        let _ = std::fs::remove_dir_all(&flat);
    }

    #[test]
    fn duplicate_keys_in_one_write_keep_the_last_row() {
        let (path, mut table) = venues("isolation-duplicates");
        let batch = trades(
            &[9, 9, 9],
            &[Some("first"), Some("second"), Some("last")],
            &[Some("XNAS"); 3],
        );
        table
            .commit_merge(
                yggdryl::arrow::batch_reader(batch.schema(), [batch]),
                &yggdryl::Selector::from_columns(["id"]),
                true,
            )
            .unwrap();
        let rows = rows_of(table.scan_where(&[("venue", "XNAS")], None).unwrap());
        assert_eq!(
            rows,
            vec![
                (1, Some("AAPL".to_owned()), Some("XNAS".to_owned())),
                (9, Some("last".to_owned()), Some("XNAS".to_owned())),
            ]
        );
        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn files_are_sorted_by_the_default_order_and_their_bounds_are_monotone() {
        let path = root("isolation-sorted");
        let schema = trade_schema();
        let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
        let mut table = Table::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            schema.clone(),
            spec,
        )
        .unwrap();
        // The default order is the partition's source column.
        let order = table.metadata().default_sort_order().unwrap();
        assert_eq!(order.order_id, 1);
        assert_eq!(order.fields.len(), 1);
        assert_eq!(order.fields[0].source_id, 3);
        assert_eq!(order.fields[0].transform, Transform::Identity);
        assert_eq!(order.fields[0].direction, "asc");
        assert_eq!(order.fields[0].null_order, "nulls-first");
        assert_eq!(table.metadata().default_sort_order_id(), 1);

        // Written unsorted, in one commit, under a tiny target: one file per
        // row, each file's rows in venue order and the files monotone.
        table
            .commit_metadata_changes(|metadata| {
                metadata.set_property("write.target-file-size-bytes", "1")?;
                Ok(())
            })
            .unwrap();
        let batch = trades(
            &[1, 2, 3, 4],
            &[Some("d"), Some("c"), Some("b"), Some("a")],
            &[Some("XNAS"); 4],
        );
        table
            .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap();
        let files = table.data_files().unwrap();
        assert_eq!(files.len(), 4);
        for (file, _) in &files {
            assert_eq!(file.sort_order_id, Some(1));
        }
        // Explicit: a second commit sorted by symbol lays symbols out
        // ascending across the files it writes.
        let sorted = root("isolation-sorted-explicit");
        let mut by_symbol = Table::create_sorted(
            LocalFolder::new(&sorted).unwrap(),
            FormatVersion::V2,
            schema.clone(),
            PartitionSpec::identity(1, &schema, &["venue"]).unwrap(),
            SortOrder {
                order_id: 1,
                fields: vec![SortField {
                    source_id: 2,
                    transform: Transform::Identity,
                    direction: "asc".into(),
                    null_order: "nulls-last".into(),
                }],
            },
        )
        .unwrap();
        by_symbol
            .commit_metadata_changes(|metadata| {
                metadata.set_property("write.target-file-size-bytes", "1")?;
                Ok(())
            })
            .unwrap();
        let batch = trades(
            &[1, 2, 3, 4],
            &[Some("d"), Some("c"), Some("b"), Some("a")],
            &[Some("XNAS"); 4],
        );
        by_symbol
            .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap();
        let mut bounds: Vec<(String, String)> = by_symbol
            .data_files()
            .unwrap()
            .into_iter()
            .map(|(file, _)| {
                let lower = file
                    .lower_bounds
                    .iter()
                    .find_map(|(id, bytes)| {
                        (*id == 2).then(|| String::from_utf8(bytes.clone()).unwrap())
                    })
                    .unwrap();
                let upper = file
                    .upper_bounds
                    .iter()
                    .find_map(|(id, bytes)| {
                        (*id == 2).then(|| String::from_utf8(bytes.clone()).unwrap())
                    })
                    .unwrap();
                (file.file_path.to_string(), format!("{lower}{upper}"))
            })
            .map(|(name, bounds)| (name.rsplit('/').next().unwrap().to_owned(), bounds))
            .collect();
        // File 00000 holds the smallest symbol, 00003 the largest.
        bounds.sort();
        let laid_out: Vec<&str> = bounds.iter().map(|(_, bounds)| bounds.as_str()).collect();
        assert_eq!(laid_out, ["aa", "bb", "cc", "dd"]);
        // And a scan returns the rows in file order: ids 4, 3, 2, 1 carry
        // symbols a, b, c, d.
        assert_eq!(ids_in_order(by_symbol.scan(None).unwrap()), [4, 3, 2, 1]);

        // The order round-trips through the metadata document and the
        // official validation of a reopened table.
        let document = by_symbol.metadata().clone().into_json().unwrap();
        let reread = super::TableMetadata::from_json(&document).unwrap();
        assert_eq!(reread.default_sort_order_id(), 1);
        assert_eq!(reread.default_sort_order().unwrap().fields[0].source_id, 2);
        let reopened = Table::open(LocalFolder::new(&sorted).unwrap()).unwrap();
        assert_eq!(reopened.metadata().default_sort_order_id(), 1);
        reopened.metadata().validate().unwrap();

        let _ = std::fs::remove_dir_all(&path);
        let _ = std::fs::remove_dir_all(&sorted);
    }

    #[test]
    fn an_explicitly_unsorted_table_keeps_the_order_the_rows_arrived_in() {
        let path = root("isolation-unsorted");
        let schema = trade_schema();
        let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
        let mut table = Table::create_sorted(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            schema,
            spec,
            SortOrder::unsorted(),
        )
        .unwrap();
        assert_eq!(table.metadata().default_sort_order_id(), 0);
        assert_eq!(table.metadata().sort_orders().len(), 1);
        let batch = trades(
            &[3, 1, 2],
            &[Some("c"), Some("a"), Some("b")],
            &[Some("XNAS"); 3],
        );
        table
            .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap();
        assert_eq!(ids_in_order(table.scan(None).unwrap()), [3, 1, 2]);
        assert_eq!(table.data_files().unwrap()[0].0.sort_order_id, None);
        // An unpartitioned table's default is the unsorted order too.
        let flat = root("isolation-unsorted-flat");
        let flat_table = Table::create(
            LocalFolder::new(&flat).unwrap(),
            FormatVersion::V2,
            trade_schema(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        assert_eq!(flat_table.metadata().default_sort_order_id(), 0);
        let _ = std::fs::remove_dir_all(&path);
        let _ = std::fs::remove_dir_all(&flat);
    }

    #[test]
    fn write_parallelism_resolves_like_every_option_and_defaults_to_the_read_parallelism() {
        let mut options = IcebergOptions::new();
        assert_eq!(options.write_parallelism(), options.read_parallelism());
        assert_eq!(options.write_parallelism_option(), None);
        options.set_read_parallelism(3).unwrap();
        assert_eq!(options.write_parallelism(), 3);
        options.set_write_parallelism(5).unwrap();
        assert_eq!(options.write_parallelism(), 5);
        assert_eq!(options.write_parallelism_option(), Some(5));
        let refused = options.set_write_parallelism(0).unwrap_err().to_string();
        assert!(refused.contains("write.parallelism"), "{refused}");
        assert_eq!(options.write_parallelism(), 5, "a refusal changes nothing");
        assert_eq!(
            IcebergOptions::new()
                .try_with_write_parallelism(2)
                .unwrap()
                .write_parallelism(),
            2
        );

        let path = root("isolation-write-parallelism");
        let mut table = Table::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            trade_schema(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        table
            .commit_metadata_changes(|metadata| {
                metadata.set_property(IcebergOptions::READ_PARALLELISM_KEY, "2")?;
                metadata.set_property(IcebergOptions::WRITE_PARALLELISM_KEY, "6")?;
                Ok(())
            })
            .unwrap();
        assert_eq!(table.options().unwrap().write_parallelism(), 6);
        table
            .commit_metadata_changes(|metadata| {
                let _ = metadata.remove_property(IcebergOptions::WRITE_PARALLELISM_KEY);
                Ok(())
            })
            .unwrap();
        assert_eq!(
            table.options().unwrap().write_parallelism(),
            2,
            "absent: the resolved read parallelism"
        );
        table
            .commit_metadata_changes(|metadata| {
                metadata.set_property(IcebergOptions::WRITE_PARALLELISM_KEY, "zero")?;
                Ok(())
            })
            .unwrap();
        let message = table.options().unwrap_err().to_string();
        assert!(message.contains("write.parallelism"), "{message}");
        table.set_options(IcebergOptions::new().try_with_write_parallelism(4).unwrap());
        assert_eq!(table.options().unwrap().write_parallelism(), 4);
        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn a_parallel_commit_writes_the_same_table_as_a_sequential_one_in_group_order() {
        let batch = trades(
            &[1, 2, 3, 4, 5, 6, 7, 8],
            &[Some("a"); 8],
            &[
                Some("v1"),
                Some("v2"),
                Some("v3"),
                Some("v4"),
                Some("v5"),
                Some("v6"),
                Some("v7"),
                Some("v8"),
            ],
        );
        let mut layouts = Vec::new();
        for parallelism in [1, 4] {
            let path = root(&format!("isolation-parallel-{parallelism}"));
            let schema = trade_schema();
            let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
            let mut table = Table::create(
                LocalFolder::new(&path).unwrap(),
                FormatVersion::V2,
                schema,
                spec,
            )
            .unwrap();
            table.set_options(
                IcebergOptions::new()
                    .try_with_write_parallelism(parallelism)
                    .unwrap(),
            );
            table
                .commit_append(yggdryl::arrow::batch_reader(
                    batch.schema(),
                    [batch.clone()],
                ))
                .unwrap();
            // The manifest lists the files in group order: v1 first, v8 last.
            let venues: Vec<String> = table
                .data_files()
                .unwrap()
                .into_iter()
                .map(|(file, _)| file.partition[0].as_str().unwrap().to_owned())
                .collect();
            assert_eq!(venues, ["v1", "v2", "v3", "v4", "v5", "v6", "v7", "v8"]);
            layouts.push(rows_of(table.scan(None).unwrap()));
            let _ = std::fs::remove_dir_all(&path);
        }
        assert_eq!(layouts[0], layouts[1]);
        assert_eq!(layouts[0].len(), 8);
    }

    #[test]
    fn a_failing_partition_writer_fails_the_commit_before_any_metadata() {
        let path = root("isolation-worker-failure");
        let schema = trade_schema();
        let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
        Table::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            schema,
            spec,
        )
        .unwrap();
        let mut recording = Recording::new(&path);
        // The third partition group's writer gets a root it cannot write through.
        recording.fail_after = Some(2);
        let mut table = Table::open(recording).unwrap();
        table.set_options(IcebergOptions::new().try_with_write_parallelism(4).unwrap());
        let version = table.metadata_version();
        let batch = trades(
            &[1, 2, 3, 4],
            &[Some("a"); 4],
            &[Some("v1"), Some("v2"), Some("v3"), Some("v4")],
        );
        let error = table
            .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap_err()
            .to_string();
        assert!(error.contains("container"), "{error}");
        assert_eq!(table.metadata_version(), version);
        assert!(table.current_snapshot().is_none());
        let reopened = Table::open(LocalFolder::new(&path).unwrap()).unwrap();
        assert_eq!(reopened.metadata_version(), version);
        assert!(reopened.current_snapshot().is_none());
        let _ = std::fs::remove_dir_all(&path);
    }

    /// A schema with an `unknown` column beside the ordinary ones.
    fn unknown_schema() -> Field {
        let mut schema = StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::Null.nullable_field("later"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
        assign_field_ids(&mut schema, 1).unwrap();
        schema
    }

    #[test]
    fn unknown_is_null_in_both_directions_and_only_a_v3_table_carries_it() {
        // Schema document -> field: `unknown` is DataType::Null and optional.
        let document: Scalar = yggdryl::json::from_utf8(
            r#"{"type":"struct","schema-id":0,"fields":[
                {"id":1,"name":"id","required":true,"type":"long"},
                {"id":2,"name":"later","required":false,"type":"unknown"},
                {"id":3,"name":"tags","required":false,"type":{"type":"list","element-id":4,"element":"unknown","element-required":false}}
            ]}"#,
        )
        .unwrap();
        let schema = schema_from_json("row", &document).unwrap();
        assert_eq!(schema.fields()[1].dtype(), &DataType::Null);
        assert!(schema.fields()[1].is_nullable());
        // Field -> document: the same spelling, the placeholder never shows.
        let written = schema_into_json(&schema).unwrap();
        assert_eq!(written, document);
        assert!(
            !yggdryl::json::into_utf8(&written)
                .unwrap()
                .contains("binary")
        );

        // A required unknown column is refused by name.
        let mut required = StructType::from_fields([DataType::Null.required_field("never")])
            .map(DataType::from)
            .unwrap()
            .required_field("row");
        assign_field_ids(&mut required, 1).unwrap();
        let message = schema_into_json(&required).unwrap_err().to_string();
        assert!(
            message.contains("never") && message.contains("optional"),
            "{message}"
        );

        // A v2 table refuses the type by name; a v3 table takes it.
        let v2 = root("isolation-unknown-v2");
        let message = Table::create(
            LocalFolder::new(&v2).unwrap(),
            FormatVersion::V2,
            unknown_schema(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap_err()
        .to_string();
        assert!(
            message.contains("later") && message.contains("unknown") && message.contains("v2"),
            "{message}"
        );

        let v3 = root("isolation-unknown-v3");
        let schema = unknown_schema();
        let mut table = Table::create(
            LocalFolder::new(&v3).unwrap(),
            FormatVersion::V3,
            schema.clone(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let arrow = schema.into_arrow_schema().unwrap();
        let batch = RecordBatch::try_new(
            Arc::clone(&arrow),
            vec![
                Arc::new(Int64Array::from(vec![1, 2])),
                Arc::new(NullArray::new(2)),
            ],
        )
        .unwrap();
        table
            .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap();

        // The column reads back as nulls, typed as the schema says...
        let mut reader = table.scan(None).unwrap();
        assert_eq!(
            reader.schema().field(1).data_type(),
            &arrow_schema::DataType::Null
        );
        let read = reader.next().unwrap().unwrap();
        assert_eq!(read.num_rows(), 2);
        assert_eq!(read.column(1).logical_null_count(), 2);
        // ...and the data file never stored it, as the spec requires.
        let (file, _) = table.data_files().unwrap().remove(0);
        let handle = child_at(&table, &file.file_path).unwrap();
        let stored =
            yggdryl::parquet::read_field(&handle, &yggdryl::parquet::ParquetOptions::new())
                .unwrap();
        assert_eq!(stored.field_len(), 1);
        assert_eq!(stored.fields()[0].name(), "id");
        assert!(file.value_counts.iter().all(|(id, _)| *id != 2));

        // The metadata document spells it and reads back through the
        // official validation of a reopened table.
        let reopened = Table::open(LocalFolder::new(&v3).unwrap()).unwrap();
        assert_eq!(
            reopened.schema().unwrap().fields()[1].dtype(),
            &DataType::Null
        );
        reopened.metadata().validate().unwrap();
        let text =
            yggdryl::json::into_utf8(&reopened.metadata().clone().into_json().unwrap()).unwrap();
        assert!(text.contains("\"unknown\""), "{text}");

        let _ = std::fs::remove_dir_all(&v2);
        let _ = std::fs::remove_dir_all(&v3);
    }

    #[test]
    fn an_unknown_column_promotes_to_any_type_in_v3() {
        let v3 = root("isolation-unknown-promotion");
        let mut table = Table::create(
            LocalFolder::new(&v3).unwrap(),
            FormatVersion::V3,
            unknown_schema(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let mut evolved = unknown_schema();
        let mut later = DataType::Int64.nullable_field("later");
        later.set_parquet_field_id(2);
        evolved.set_field("later", later).unwrap();
        table.evolve_schema(evolved).unwrap();
        assert_eq!(
            table.schema().unwrap().fields()[1].dtype(),
            &DataType::Int64
        );
        let _ = std::fs::remove_dir_all(&v3);
    }

    /// One variant column: the two binaries the encoding states, per row.
    fn variant_column(rows: usize, field: &arrow_schema::Field) -> ArrayRef {
        let arrow_schema::DataType::Struct(children) = field.data_type() else {
            panic!(
                "a variant lays out as the struct of its two binaries, got {}",
                field.data_type()
            );
        };
        let variants: Vec<yggdryl::Variant> = (0..rows)
            .map(|row| {
                yggdryl::Variant::encode(
                    &yggdryl::Scalar::from_struct([("row", yggdryl::Scalar::from(row as i64))])
                        .unwrap(),
                )
                .unwrap()
            })
            .collect();
        Arc::new(arrow_array::StructArray::new(
            children.clone(),
            vec![
                Arc::new(BinaryArray::from_iter_values(
                    variants.iter().map(yggdryl::Variant::metadata),
                )) as ArrayRef,
                Arc::new(BinaryArray::from_iter_values(
                    variants.iter().map(yggdryl::Variant::value),
                )) as ArrayRef,
            ],
            None,
        ))
    }

    #[test]
    fn variant_is_the_semi_structured_datatype_in_both_directions_and_rides_a_data_file() {
        use yggdryl::iceberg::PrimitiveType;

        assert_eq!(
            PrimitiveType::from_str("variant").unwrap(),
            PrimitiveType::Variant
        );
        assert_eq!(PrimitiveType::Variant.to_string(), "variant");
        assert_eq!(
            PrimitiveType::Variant.into_dtype().unwrap(),
            DataType::Variant
        );
        assert_eq!(
            PrimitiveType::from_dtype(&DataType::Variant).unwrap(),
            PrimitiveType::Variant
        );
        // Not the same thing as unknown: one is a type-per-value, the other
        // the absence of a type.
        assert_ne!(
            PrimitiveType::Variant.into_dtype().unwrap(),
            PrimitiveType::Unknown.into_dtype().unwrap()
        );

        let document: Scalar = yggdryl::json::from_utf8(
            r#"{"type":"struct","schema-id":0,"fields":[
                {"id":1,"name":"id","required":true,"type":"long"},
                {"id":2,"name":"payload","required":false,"type":"variant"}
            ]}"#,
        )
        .unwrap();
        let schema = schema_from_json("row", &document).unwrap();
        assert_eq!(schema.fields()[1].dtype(), &DataType::Variant);
        assert_eq!(schema_into_json(&schema).unwrap(), document);

        let v2 = root("isolation-variant-v2");
        let message = Table::create(
            LocalFolder::new(&v2).unwrap(),
            FormatVersion::V2,
            schema.clone(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap_err()
        .to_string();
        assert!(
            message.contains("payload") && message.contains("variant") && message.contains("v2"),
            "{message}"
        );

        let v3 = root("isolation-variant-v3");
        let mut table = Table::create(
            LocalFolder::new(&v3).unwrap(),
            FormatVersion::V3,
            schema.clone(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let arrow = schema.into_arrow_schema().unwrap();
        let batch = RecordBatch::try_new(
            Arc::clone(&arrow),
            vec![
                Arc::new(Int64Array::from(vec![1, 2])),
                variant_column(2, arrow.field(1)),
            ],
        )
        .unwrap();
        table
            .commit_append(yggdryl::arrow::batch_reader(
                batch.schema(),
                [batch.clone()],
            ))
            .unwrap();
        let mut reader = table.scan(None).unwrap();
        let read = reader.next().unwrap().unwrap();
        assert_eq!(read, batch);
        assert_eq!(
            reader
                .schema()
                .field(1)
                .metadata()
                .get(arrow_schema::extension::EXTENSION_TYPE_NAME_KEY)
                .map(String::as_str),
            Some(yggdryl::VARIANT_EXTENSION_NAME)
        );
        let reopened = Table::open(LocalFolder::new(&v3).unwrap()).unwrap();
        assert_eq!(
            reopened.schema().unwrap().fields()[1].dtype(),
            &DataType::Variant
        );
        reopened.metadata().validate().unwrap();

        let _ = std::fs::remove_dir_all(&v2);
        let _ = std::fs::remove_dir_all(&v3);
    }

    #[test]
    fn the_record_surface_keys_a_merge_by_the_partition_and_the_options_key() {
        let (path, _) = venues("isolation-record-merge");
        let mut table = Table::open(LocalFolder::new(&path).unwrap()).unwrap();
        let options: RecordOptions = IOMedia::record_options(&table)
            .unwrap()
            .with_field(trade_schema());
        // No key: the XNAS partition is replaced, the others carried.
        let batch = trades(&[5], &[Some("TSLA")], &[Some("XNAS")]);
        table
            .merge_arrow_reader(
                yggdryl::arrow::batch_reader(batch.schema(), [batch]),
                &options,
            )
            .unwrap();
        assert_eq!(
            rows_of(table.scan(None).unwrap()),
            vec![
                (2, Some("MSFT".to_owned()), Some("XNYS".to_owned())),
                (3, Some("VOD".to_owned()), Some("XLON".to_owned())),
                (5, Some("TSLA".to_owned()), Some("XNAS".to_owned())),
            ]
        );
        // With a key: an upsert within the partition.
        let keyed = options.with_merge_by("id").unwrap();
        let batch = trades(&[5, 6], &[Some("TSLA.O"), Some("AMD")], &[Some("XNAS"); 2]);
        table
            .merge_arrow_reader(
                yggdryl::arrow::batch_reader(batch.schema(), [batch]),
                &keyed,
            )
            .unwrap();
        assert_eq!(
            rows_of(table.scan(None).unwrap()),
            vec![
                (2, Some("MSFT".to_owned()), Some("XNYS".to_owned())),
                (3, Some("VOD".to_owned()), Some("XLON".to_owned())),
                (5, Some("TSLA.O".to_owned()), Some("XNAS".to_owned())),
                (6, Some("AMD".to_owned()), Some("XNAS".to_owned())),
            ]
        );
        let _ = std::fs::remove_dir_all(&path);
    }
}

/// The staging of one commit: a transaction over the files it writes.
///
/// `Staging` is private to the module, so its drop, rollback and default
/// rules are pinned here; what a staged commit costs over a store is
/// pinned in `rust/tests/s3/mod_.rs`.
mod staging_transaction {
    use std::path::{Path, PathBuf};

    use super::{FormatVersion, IcebergOptions, PartitionSpec, Table, root, trade_schema, trades};
    use yggdryl::holder::Holder;
    use yggdryl::iceberg::WriteStaging;
    use yggdryl::internals::iceberg_staging::Staging;
    use yggdryl::local::LocalFolder;
    use yggdryl::{IOBase, MediaType, MimeType, Url};

    fn staging_folder(label: &str) -> (PathBuf, WriteStaging) {
        let path = root(label);
        let staging = WriteStaging::Folder(Url::from_path(&path).unwrap());
        (path, staging)
    }

    fn is_empty_dir(path: &Path) -> bool {
        !path.exists() || std::fs::read_dir(path).unwrap().next().is_none()
    }

    /// Every regular file under `path`, at any depth.
    fn files_under(path: &Path) -> Vec<PathBuf> {
        let mut files = Vec::new();
        let mut pending = vec![path.to_path_buf()];
        while let Some(directory) = pending.pop() {
            let Ok(entries) = std::fs::read_dir(&directory) else {
                continue;
            };
            for entry in entries {
                let entry = entry.unwrap().path();
                if entry.is_dir() {
                    pending.push(entry);
                } else {
                    files.push(entry);
                }
            }
        }
        files.sort();
        files
    }

    #[test]
    fn staging_is_off_for_a_local_root_and_the_temporary_folder_for_a_remote_one() {
        assert!(
            Staging::begin(Some(&WriteStaging::Off), true, 1)
                .unwrap()
                .directory()
                .is_none()
        );
        assert!(
            Staging::begin(None, false, 1)
                .unwrap()
                .directory()
                .is_none()
        );
        let remote = Staging::begin(None, true, 1).unwrap();
        let directory = remote.directory().unwrap().to_path_buf();
        assert!(directory.starts_with(LocalFolder::temporary().unwrap().path().unwrap()));
        assert!(
            directory
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("yggdryl-iceberg-1-"),
            "{}",
            directory.display()
        );
        assert!(
            !directory.exists(),
            "nothing is created until a file is staged"
        );
    }

    #[test]
    fn a_staging_directory_is_the_commits_own_and_goes_when_the_commit_ends() {
        let (base, staging) = staging_folder("staging-drop");
        let table_path = root("staging-drop-table");
        let root_handle = Holder::folder(&table_path).unwrap();
        let media = MediaType::new(MimeType::FILE);

        // Two commits of one snapshot id - a retry, a concurrent writer -
        // stage apart, and neither can see the other's files.
        let first = Staging::begin(Some(&staging), false, 7).unwrap();
        let second = Staging::begin(Some(&staging), false, 7).unwrap();
        let first_dir = first.directory().unwrap().to_path_buf();
        let second_dir = second.directory().unwrap().to_path_buf();
        assert_ne!(first_dir, second_dir);
        assert!(first_dir.starts_with(&base) && second_dir.starts_with(&base));

        let ((), size) = first
            .publish(&root_handle, "data/venue=XNAS/part.bin", &media, |handle| {
                handle.write_all_bytes(b"PAR1")
            })
            .unwrap();
        assert_eq!(size, 4);
        // The staged file went out and is already gone; the directory is
        // the commit's until the commit ends; the other commit sees nothing.
        assert!(first_dir.exists());
        assert!(files_under(&first_dir).is_empty());
        assert!(!second_dir.exists());
        let published = table_path.join("data").join("venue=XNAS").join("part.bin");
        assert_eq!(std::fs::read(&published).unwrap(), b"PAR1");

        // Dropping a staging that never finished rolls its published file
        // back and removes its directory: a failed commit leaves nothing.
        drop(first);
        assert!(!first_dir.exists());
        assert!(!published.exists(), "the published file is removed again");

        // A committed staging keeps what it published and removes only the
        // directory, when it drops.
        let third = Staging::begin(Some(&staging), false, 8).unwrap();
        let third_dir = third.directory().unwrap().to_path_buf();
        third
            .publish(&root_handle, "data/venue=XNYS/part.bin", &media, |handle| {
                handle.write_all_bytes(b"PAR1")
            })
            .unwrap();
        third.commit();
        drop(third);
        assert!(!third_dir.exists());
        assert_eq!(
            std::fs::read(table_path.join("data").join("venue=XNYS").join("part.bin")).unwrap(),
            b"PAR1"
        );
        drop(second);
        assert!(is_empty_dir(&base));
        let _ = std::fs::remove_dir_all(&base);
        let _ = std::fs::remove_dir_all(&table_path);
    }

    #[test]
    fn a_failed_publication_rolls_the_commit_back_and_leaves_no_staged_file() {
        let (base, staging) = staging_folder("staging-rollback");
        let path = root("staging-rollback-table");
        let schema = trade_schema();
        let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
        let mut table = Table::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            schema,
            spec,
        )
        .unwrap();
        // A regular file where the XNAS partition directory belongs: that
        // partition's file is staged, and its publication is what fails.
        std::fs::create_dir_all(path.join("data")).unwrap();
        let blocker = path.join("data").join("venue=XNAS");
        std::fs::write(&blocker, b"a file where a directory would be").unwrap();
        table.set_options(
            IcebergOptions::new()
                .try_with_write_staging(staging)
                .unwrap()
                .try_with_write_parallelism(1)
                .unwrap(),
        );
        let version = table.metadata_version();

        // XLON's file is staged and published first; XNAS's publication
        // fails, and XNYS's is never attempted.
        let batch = trades(
            &[1, 2, 3],
            &[Some("a"), Some("b"), Some("c")],
            &[Some("XLON"), Some("XNAS"), Some("XNYS")],
        );
        table
            .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap_err();
        assert_eq!(table.metadata_version(), version);
        assert!(table.current_snapshot().is_none());
        assert_eq!(
            files_under(&path.join("data")),
            [blocker],
            "the file the commit published is removed again"
        );
        assert!(
            files_under(&path.join("metadata"))
                .iter()
                .all(|file| !file.to_string_lossy().ends_with(".avro")),
            "no manifest and no manifest list were published"
        );
        assert!(is_empty_dir(&base), "no staged file survives");
        let reopened = Table::open(LocalFolder::new(&path).unwrap()).unwrap();
        assert_eq!(reopened.metadata_version(), version);
        assert!(reopened.current_snapshot().is_none());
        let _ = std::fs::remove_dir_all(&base);
        let _ = std::fs::remove_dir_all(&path);
    }
}
