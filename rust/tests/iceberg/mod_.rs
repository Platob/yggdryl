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
    IcebergTable, ManifestContent, ManifestEntry, ManifestFile, PartitionField, PartitionSpec,
    ScanPlan, ScanTask, Snapshot, SnapshotRef, SortField, SortOrder, TableMetadata, Transform,
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
        FormatVersion::V3,
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

/// An Arrow filesystem that refuses the next write of the version hint,
/// whichever door it takes - a create or an output stream - writing nothing.
///
/// A commit's document is the commit and its hint only where a read starts,
/// so this pins that a hint the store refuses fails no commit.
#[derive(Debug, Default)]
struct RefusedHintWrite {
    inner: yggdryl::fs::MemoryFileSystem,
    refuse_next_hint: Arc<AtomicBool>,
}

impl RefusedHintWrite {
    fn arm(&self) {
        self.refuse_next_hint.store(true, Ordering::Relaxed);
    }

    fn is_armed(&self) -> bool {
        self.refuse_next_hint.load(Ordering::Relaxed)
    }

    /// Refuse a write of `path` where it is the hint and a refusal is armed.
    fn write(&self, path: &str) -> yggdryl::Result<()> {
        if path.ends_with("/version-hint.text")
            && self.refuse_next_hint.swap(false, Ordering::Relaxed)
        {
            return Err(yggdryl::Error::Io(std::io::Error::other(
                "injected refusal of the version hint write",
            )));
        }
        Ok(())
    }
}

impl yggdryl::fs::FileSystem for RefusedHintWrite {
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
        self.write(path)?;
        self.inner.open_output_stream(path, metadata)
    }

    fn open_append_stream(
        &self,
        path: &str,
        metadata: Option<&OutputMetadata>,
    ) -> yggdryl::Result<Box<dyn ByteWriter>> {
        self.inner.open_append_stream(path, metadata)
    }

    fn create_file(&self, path: &str, bytes: &[u8]) -> yggdryl::Result<()> {
        self.write(path)?;
        self.inner.create_file(path, bytes)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A memory filesystem that lands one competing metadata document at version
/// 2 in the instant between a writer's look at the hint and its exclusive
/// create of that version.
///
/// The injected document is either the attempted snapshot itself, or the
/// attempted metadata change with the loser's property replaced by the
/// winner's. It takes the versioned name just before the writer's create,
/// after the writer passed `find_metadata`, forcing the refused-create path.
#[derive(Debug, Default)]
struct SameVersionWinner {
    inner: yggdryl::fs::MemoryFileSystem,
    armed: Arc<AtomicBool>,
    own_snapshot: Arc<AtomicBool>,
    delay_own_snapshot: Arc<AtomicBool>,
    retry_creates: Arc<AtomicUsize>,
    injections: Arc<AtomicUsize>,
}

impl SameVersionWinner {
    fn arm(&self) {
        self.armed.store(true, Ordering::Relaxed);
    }

    fn arm_own_snapshot(&self) {
        self.own_snapshot.store(true, Ordering::Relaxed);
        self.arm();
    }

    fn arm_delayed_own_snapshot(&self) {
        self.delay_own_snapshot.store(true, Ordering::Relaxed);
        self.arm_own_snapshot();
    }

    fn reveal_own_snapshot(&self) {
        self.delay_own_snapshot.store(false, Ordering::Relaxed);
    }

    fn retry_creates(&self) -> usize {
        self.retry_creates.load(Ordering::Relaxed)
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

    /// Create the winner of `path` from the loser's attempted `bytes`, and
    /// the hint naming it.
    fn inject_winner(&self, path: &str, bytes: &[u8]) -> yggdryl::Result<()> {
        if self.own_snapshot.swap(false, Ordering::Relaxed) {
            self.inner.create_file(path, bytes)?;
            let (directory, _) = path
                .rsplit_once('/')
                .ok_or_else(|| Self::invalid("expected a metadata directory"))?;
            write_filesystem_bytes(&self.inner, &format!("{directory}/version-hint.text"), b"2")?;
            self.injections.fetch_add(1, Ordering::Relaxed);
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
        let (directory, _) = path
            .rsplit_once('/')
            .ok_or_else(|| Self::invalid("expected a metadata directory"))?;
        self.inner
            .create_file(path, &serde_json::to_vec(&document)?)?;
        write_filesystem_bytes(&self.inner, &format!("{directory}/version-hint.text"), b"2")?;
        self.injections.fetch_add(1, Ordering::Relaxed);
        Ok(())
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
        if path.ends_with("/metadata/v2.metadata.json")
            && self.delay_own_snapshot.load(Ordering::Relaxed)
        {
            return Err(yggdryl::Error::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "the newly published metadata is not visible yet",
            )));
        }
        self.inner.open_input_stream(path)
    }

    fn open_output_stream(
        &self,
        path: &str,
        metadata: Option<&OutputMetadata>,
    ) -> yggdryl::Result<Box<dyn ByteWriter>> {
        self.inner.open_output_stream(path, metadata)
    }

    fn open_append_stream(
        &self,
        path: &str,
        metadata: Option<&OutputMetadata>,
    ) -> yggdryl::Result<Box<dyn ByteWriter>> {
        self.inner.open_append_stream(path, metadata)
    }

    fn create_file(&self, path: &str, bytes: &[u8]) -> yggdryl::Result<()> {
        if path.ends_with("/metadata/v2.metadata.json") {
            if self.armed.swap(false, Ordering::Relaxed) {
                self.inject_winner(path, bytes)?;
            } else if self.delay_own_snapshot.swap(false, Ordering::Relaxed) {
                self.retry_creates.fetch_add(1, Ordering::Relaxed);
            }
        }
        self.inner.create_file(path, bytes)
    }

    fn as_any(&self) -> &dyn Any {
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

/// An Arrow filesystem that refuses the create of one versioned metadata
/// document - `v{n}.metadata.json` - after every file it names landed.
///
/// A store can refuse the last of a commit's writes as well as the first;
/// this makes it refuse exactly the one that is the commit's point of no
/// return, so what a commit leaves behind when it fails just short of it is
/// pinned.
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
        self.inner.open_output_stream(path, metadata)
    }

    fn open_append_stream(
        &self,
        path: &str,
        metadata: Option<&OutputMetadata>,
    ) -> yggdryl::Result<Box<dyn ByteWriter>> {
        self.inner.open_append_stream(path, metadata)
    }

    fn create_file(&self, path: &str, bytes: &[u8]) -> yggdryl::Result<()> {
        let name = path.rsplit('/').next().unwrap_or(path);
        if name.starts_with('v')
            && name.ends_with(".metadata.json")
            && self.refuse_next_document.swap(false, Ordering::Relaxed)
        {
            return Err(yggdryl::Error::Io(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "the versioned document create is refused",
            )));
        }
        self.inner.create_file(path, bytes)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A memory filesystem that lands a competing metadata-only document at
/// version 2 between a data commit's look at the hint and its exclusive
/// create of that version, and records every file the loser removes.
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

/// Create a metadata-only winner at `path`, version 2, from the version 1
/// document beside it - that document with the property `winner` added - and
/// the hint naming it.
fn land_metadata_only_winner(filesystem: &dyn FileSystem, path: &str) -> yggdryl::Result<()> {
    let (directory, _) = path
        .rsplit_once('/')
        .ok_or_else(|| SameVersionWinner::invalid("expected a metadata directory"))?;
    let previous = read_filesystem_bytes(filesystem, &format!("{directory}/v1.metadata.json"))?;
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
    filesystem.create_file(path, &serde_json::to_vec(&document)?)?;
    write_filesystem_bytes(filesystem, &format!("{directory}/version-hint.text"), b"2")
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
        self.inner.open_output_stream(path, metadata)
    }

    fn open_append_stream(
        &self,
        path: &str,
        metadata: Option<&OutputMetadata>,
    ) -> yggdryl::Result<Box<dyn ByteWriter>> {
        self.inner.open_append_stream(path, metadata)
    }

    fn create_file(&self, path: &str, bytes: &[u8]) -> yggdryl::Result<()> {
        if path.ends_with("/metadata/v2.metadata.json") && self.armed.swap(false, Ordering::Relaxed)
        {
            land_metadata_only_winner(&self.inner, path)?;
        }
        self.inner.create_file(path, bytes)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A memory filesystem whose version hint reads as one replaced under the
/// read: the size of the hint a commit removed, then no byte of the one it
/// created in its place - `expected 1 bytes at offset 0, got 0`, what a
/// reader on a store that sizes a file before it reads it meets between a
/// commit's removal of the hint and its re-creation. It counts the listings
/// of `metadata/`, and armed, it lands a metadata-only winner at version 2
/// just before a commit's claim of it - every read the beaten commit then
/// looks for the winner by, the hint and the listing, failing once.
#[derive(Debug, Default)]
struct ReplacedHint {
    inner: yggdryl::fs::MemoryFileSystem,
    failing_hint_reads: AtomicUsize,
    failing_listings: AtomicUsize,
    listings: AtomicUsize,
    armed: AtomicBool,
}

impl ReplacedHint {
    /// Fail the next `count` reads of the hint.
    fn fail_hint_reads(&self, count: usize) {
        self.failing_hint_reads.store(count, Ordering::Relaxed);
    }

    fn arm(&self) {
        self.armed.store(true, Ordering::Relaxed);
    }

    /// The listings taken so far.
    fn listings(&self) -> usize {
        self.listings.load(Ordering::Relaxed)
    }

    /// The failures armed and not yet met: hint reads, then listings.
    fn pending(&self) -> (usize, usize) {
        (
            self.failing_hint_reads.load(Ordering::Relaxed),
            self.failing_listings.load(Ordering::Relaxed),
        )
    }

    /// Spend one failure of `left`, answering whether there was one.
    // `fetch_update` is the spelling the declared MSRV, Rust 1.94, knows;
    // its rename `try_update` came later.
    #[allow(deprecated)]
    fn spend(left: &AtomicUsize) -> bool {
        let mut current = left.load(Ordering::Relaxed);
        while current > 0 {
            match left.compare_exchange_weak(
                current,
                current - 1,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return true,
                Err(actual) => current = actual,
            }
        }
        false
    }

    /// Fail a read of `path` where it is the hint and a failure is armed.
    fn read(&self, path: &str) -> yggdryl::Result<()> {
        if path.ends_with("/metadata/version-hint.text") && Self::spend(&self.failing_hint_reads) {
            return Err(yggdryl::Error::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "expected 1 bytes at offset 0, got 0",
            )));
        }
        Ok(())
    }
}

impl yggdryl::fs::FileSystem for ReplacedHint {
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
        self.listings.fetch_add(1, Ordering::Relaxed);
        if Self::spend(&self.failing_listings) {
            return FileInfos::failing(yggdryl::Error::Io(std::io::Error::other(
                "the listing was cut short",
            )));
        }
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
        self.read(path)?;
        self.inner.open_input_file(path)
    }

    fn open_input_stream(&self, path: &str) -> yggdryl::Result<Box<dyn ByteReader>> {
        self.read(path)?;
        self.inner.open_input_stream(path)
    }

    fn open_output_stream(
        &self,
        path: &str,
        metadata: Option<&OutputMetadata>,
    ) -> yggdryl::Result<Box<dyn ByteWriter>> {
        self.inner.open_output_stream(path, metadata)
    }

    fn open_append_stream(
        &self,
        path: &str,
        metadata: Option<&OutputMetadata>,
    ) -> yggdryl::Result<Box<dyn ByteWriter>> {
        self.inner.open_append_stream(path, metadata)
    }

    fn create_file(&self, path: &str, bytes: &[u8]) -> yggdryl::Result<()> {
        if path.ends_with("/metadata/v2.metadata.json") && self.armed.swap(false, Ordering::Relaxed)
        {
            land_metadata_only_winner(&self.inner, path)?;
            self.failing_hint_reads.store(1, Ordering::Relaxed);
            self.failing_listings.store(1, Ordering::Relaxed);
        }
        self.inner.create_file(path, bytes)
    }

    fn as_any(&self) -> &dyn Any {
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
    fn a_description_is_the_doc_a_column_states_at_every_depth() {
        let mut price = DataType::Float64.nullable_field("price");
        price.set_description("the closing price").unwrap();
        let mut quote =
            DataType::from(StructType::from_fields([price]).unwrap()).nullable_field("quote");
        quote.set_description("the last quote").unwrap();
        let mut id = DataType::Int64.required_field("id");
        id.set_description("the row identifier").unwrap();
        let silent = DataType::Int64.nullable_field("silent");
        let mut schema = DataType::from(StructType::from_fields([id, quote, silent]).unwrap())
            .required_field("row");
        assign_field_ids(&mut schema, 1).unwrap();

        let document = schema_into_json(&schema).unwrap();
        let fields = document.get_key_str("fields").unwrap();
        let doc = |entry: &Scalar| {
            entry
                .get_key_str("doc")
                .and_then(Scalar::as_str)
                .map(str::to_owned)
        };
        let entries = fields.sequence_rows().unwrap();
        assert_eq!(doc(&entries[0]).as_deref(), Some("the row identifier"));
        assert_eq!(doc(&entries[1]).as_deref(), Some("the last quote"));
        assert_eq!(doc(&entries[2]), None, "no description states no doc");
        let nested = entries[1]
            .get_key_str("type")
            .unwrap()
            .get_key_str("fields")
            .unwrap();
        let nested = nested.sequence_rows().unwrap();
        assert_eq!(doc(&nested[0]).as_deref(), Some("the closing price"));

        // Read back, every description is where it was: each column is the
        // same field, the description under its own key. The root alone
        // differs, by the schema id a document read states.
        let read = schema_from_json("row", &document).unwrap();
        assert_eq!(read.fields(), schema.fields());
        assert_eq!(read.fields()[0].description(), Some("the row identifier"));
        assert_eq!(
            read.fields()[1].fields()[0].description(),
            Some("the closing price")
        );
        assert_eq!(read.fields()[2].description(), None);
    }

    #[test]
    fn a_doc_another_writer_broke_over_lines_reads_as_one_line() {
        let document = yggdryl::json::from_utf8(
            r#"{
                "type": "struct",
                "fields": [
                    {"id": 1, "name": "a", "required": true, "type": "long",
                     "doc": "first line\nsecond\tline\r\n"},
                    {"id": 2, "name": "b", "required": true, "type": "long", "doc": ""},
                    {"id": 3, "name": "c", "required": true, "type": "long", "doc": "\n"}
                ]
            }"#,
        )
        .unwrap();
        let schema = schema_from_json("row", &document).unwrap();
        assert_eq!(
            schema.fields()[0].description(),
            Some("first line second line")
        );
        assert_eq!(schema.fields()[1].description(), None, "an empty doc");
        assert_eq!(schema.fields()[2].description(), None, "a blank doc");
    }

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
        // A column's `doc` is the field's own description, never a
        // property of its own.
        assert_eq!(schema.fields()[1].description(), Some("ticker"));
        assert_eq!(schema.fields()[1].get_metadata("ICEBERG:doc"), None);

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
        assert!(message.contains("got serie"), "{message}");
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
        // A v3 unknown column holds nothing yet and may become any type, so
        // it reads as the variant that holds whatever it becomes.
        assert_eq!(
            PrimitiveType::Unknown.into_dtype().unwrap(),
            DataType::Variant
        );
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
            (DataType::Ccy, "USD"),
            (DataType::Mic, "XPAR"),
            (DataType::Cfi, "ESVUFR"),
            (DataType::Isin, "US0378331005"),
            // A pair's bound is the canonical spelling the column holds.
            (DataType::Forex, "eurusd"),
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
    fn an_enum_bound_is_the_int_its_column_stores() {
        use yggdryl::internals::iceberg_value::{is_portable, single_to_value, single_value};
        use yggdryl::{Scalar, State};

        // An enum column stores its member's code, so its bounds are Iceberg
        // ints a planner compares in code order - a state's in lifecycle
        // order - and never the name a text codec writes.
        let members = [
            (
                DataType::State,
                Scalar::State(State::Unknown),
                State::Unknown.code(),
            ),
            (
                DataType::State,
                Scalar::State(State::New),
                State::New.code(),
            ),
            (
                DataType::State,
                Scalar::State(State::Filled),
                State::Filled.code(),
            ),
            (
                DataType::State,
                Scalar::State(State::Expired),
                State::Expired.code(),
            ),
        ];
        for (dtype, exact, code) in members {
            assert!(is_portable(&dtype), "{dtype}");
            assert_eq!(
                PrimitiveType::from_dtype(&dtype).unwrap(),
                PrimitiveType::Int
            );
            let bytes = single_value(&exact, &dtype).expect("an enum member encodes a bound");
            // Whatever width the column stores, an Iceberg int is four bytes.
            assert_eq!(bytes, i32::from(code).to_le_bytes(), "{exact:?}");
            assert_eq!(single_to_value(&bytes, &dtype), Some(exact), "{dtype}");
        }
        // The code of no member reads as no bound rather than as a member.
        assert_eq!(
            single_to_value(&7_i32.to_le_bytes(), &DataType::State),
            None
        );
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
        for version in [FormatVersion::V3, FormatVersion::V2, FormatVersion::V1] {
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

        let mut metadata = metadata(FormatVersion::V3);
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

        let mut metadata = metadata(FormatVersion::V3);
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
                    // The row lineage a v3 table asks of every snapshot.
                    first_row_id: Some(0),
                    added_rows: Some(0),
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
        // v1's singular keys: the document pinned here.
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
        // v1's own document, read back.
        assert_eq!(metadata.format_version(), FormatVersion::V1);
        assert_eq!(metadata.schemas().len(), 1);
        assert_eq!(metadata.partition_specs().len(), 1);
        assert!(metadata.default_spec().unwrap().is_unpartitioned());
    }

    #[test]
    fn v1_direct_manifest_paths_survive_official_metadata_updates() {
        // v1's direct manifest paths: the contract pinned here.
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
        let mut document = metadata(FormatVersion::V3).into_json().unwrap();
        document = document.with_key("current-snapshot-id", -1_i64).unwrap();
        let read = TableMetadata::from_json(&document).unwrap();
        assert!(read.current_snapshot_id().is_none());
        assert!(read.current_snapshot().is_none());
    }

    #[test]
    fn an_evolved_schema_keeps_the_old_one_and_numbers_above_it() {
        let mut metadata = metadata(FormatVersion::V3);
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
        FormatVersion, IOBase, IcebergTable, LocalFolder, PartitionField, PartitionSpec, Transform,
        assign_field_ids, collect, root, trade_schema, trades,
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

        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
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
        assert_eq!(table.metadata().unwrap().last_column_id(), 2);

        // The numbered table is a working table, not merely a written one.
        let batch = trades(&[7], &[None], &[Some("X")]);
        table
            .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap();
        let reopened = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
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

        let table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
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
        assert_eq!(table.metadata().unwrap().last_column_id(), 8);
    }

    #[test]
    fn an_empty_table_has_no_snapshot_and_reads_as_no_rows() {
        let path = root("empty");
        let schema = trade_schema();
        let table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
            schema,
            PartitionSpec::unpartitioned(),
        )
        .unwrap();

        assert!(table.current_snapshot().unwrap().is_none());
        assert!(table.manifests().unwrap().is_empty());
        assert!(table.data_files().unwrap().is_empty());
        assert_eq!(table.row_size().unwrap(), 0);
        assert_eq!(table.column_size().unwrap(), 3);
        assert_eq!(collect(table.scan(None).unwrap()).len(), 0);

        // The document is on disk and reopening finds it.
        let reopened = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
        assert_eq!(reopened.metadata_version().unwrap(), 1);
        assert!(reopened.current_snapshot().unwrap().is_none());
    }

    #[test]
    fn open_preserves_an_official_uuid_metadata_name_in_history() {
        let path = root("official-metadata-name");
        let table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
            trade_schema(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let generated_name = table.metadata_file_name().unwrap();
        drop(table);

        let official_name = "00001-123e4567-e89b-12d3-a456-426614174000.metadata.json";
        std::fs::rename(
            path.join("metadata").join(generated_name),
            path.join("metadata").join(official_name),
        )
        .unwrap();

        let mut reopened = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
        assert_eq!(reopened.metadata_file_name().unwrap(), official_name);
        reopened
            .commit_metadata_changes(|metadata| {
                metadata.set_property("owner", "interop")?;
                Ok(())
            })
            .unwrap();
        assert!(
            reopened
                .metadata()
                .unwrap()
                .metadata_log()
                .iter()
                .any(|(_, location)| location.ends_with(official_name))
        );
    }

    #[test]
    fn a_commit_publishes_the_name_a_version_hint_resolves() {
        let path = root("hint-resolvable-metadata-name");
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
            trade_schema(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        assert_eq!(table.metadata_file_name().unwrap(), "v1.metadata.json");
        table
            .commit_metadata_changes(|metadata| {
                metadata.set_property("owner", "interop")?;
                Ok(())
            })
            .unwrap();
        assert_eq!(table.metadata_file_name().unwrap(), "v2.metadata.json");

        // `v{version}` is the only spelling a numeric hint resolves, so it is
        // the only one a catalog-free reader - Spark's Hadoop tables among
        // them - can follow, and the commit creates exactly it: the create is
        // the claim, so no other file is written beside it.
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
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
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

        assert_eq!(table.metadata_file_name().unwrap(), "v2.gz.metadata.json");
        assert!(
            std::fs::read(
                path.join("metadata")
                    .join(table.metadata_file_name().unwrap())
            )
            .unwrap()
            .starts_with(&[0x1f, 0x8b])
        );
        let reopened = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
        assert_eq!(
            reopened.metadata_file_name().unwrap(),
            table.metadata_file_name().unwrap()
        );
        assert_eq!(reopened.metadata().unwrap().property("owner"), None);
    }

    #[test]
    fn direct_create_refuses_to_replace_an_existing_table() {
        let path = root("create-conflict");
        IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
            trade_schema(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let error = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
            trade_schema(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap_err();
        assert!(error.is_conflict(), "{error}");
        assert_eq!(
            IcebergTable::open(LocalFolder::new(&path).unwrap())
                .unwrap()
                .metadata_version()
                .unwrap(),
            1
        );
    }

    #[test]
    fn child_locations_require_a_table_path_boundary() {
        let path = root("location-boundary");
        let table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
            trade_schema(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let sibling = format!(
            "{}2/data/file.parquet",
            table.metadata().unwrap().location()
        );
        assert!(child_at(&table, &sibling).is_err());
    }

    #[test]
    fn an_unpartitioned_table_round_trips_its_rows() {
        let path = root("unpartitioned");
        let schema = trade_schema();
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
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

        let snapshot = table.current_snapshot().unwrap().expect("a snapshot");
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
        let reopened = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
        assert_eq!(collect(reopened.scan(None).unwrap()), rows);
    }

    #[test]
    fn a_v1_snapshot_with_direct_manifests_scans_and_time_travels() {
        let path = root("v1-direct-manifests");
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            // v1's direct manifests: the contract pinned here.
            FormatVersion::V1,
            trade_schema(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let first = trades(&[1], &[Some("AAPL")], &[Some("XNAS")]);
        table
            .commit_append(yggdryl::arrow::batch_reader(first.schema(), [first]))
            .unwrap();
        let snapshot_id = table.current_snapshot().unwrap().unwrap().snapshot_id;
        let manifest_paths: Vec<Scalar> = table
            .manifests()
            .unwrap()
            .into_iter()
            .map(|manifest| Scalar::from(manifest.manifest_path))
            .collect();

        // This is the original v1 snapshot fixture: manifests were embedded
        // directly in metadata before manifest lists became universal.
        let mut document = table.metadata().unwrap().clone().into_json().unwrap();
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
        let metadata_path = path
            .join("metadata")
            .join(table.metadata_file_name().unwrap());
        std::fs::write(metadata_path, yggdryl::json::into_bytes(&document).unwrap()).unwrap();

        let mut reopened = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
        let v1 = reopened.current_snapshot().unwrap().unwrap();
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
                .unwrap()
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
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
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
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
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
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
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
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
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
            let mut table = IcebergTable::create(
                LocalFolder::new(&path).unwrap(),
                FormatVersion::V3,
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
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
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
            let mut table = IcebergTable::create(
                LocalFolder::new(&path).unwrap(),
                FormatVersion::V3,
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
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
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
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
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
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
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
        assert_eq!(table.metadata().unwrap().snapshots().len(), 3);
        assert!(
            table
                .current_snapshot()
                .unwrap()
                .and_then(|snapshot| snapshot.parent_snapshot_id)
                .is_some()
        );
    }

    #[test]
    fn a_scan_pushes_the_requested_columns_down_to_each_file() {
        let path = root("pushdown");
        let schema = trade_schema();
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
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
    fn a_required_column_with_an_initial_default_reads_it_in_files_that_predate_it() {
        let path = root("evolution-initial-default");
        let schema = trade_schema();
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
            schema.clone(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let batch = trades(&[1], &[Some("AAPL")], &[Some("XNAS")]);
        table
            .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap();

        // A required column is added with the value older files read it as.
        let mut evolved = schema.clone();
        evolved.remove_metadata("ICEBERG:schema-id");
        let mut quantity = DataType::Int64.required_field("quantity");
        quantity
            .insert_metadata("ICEBERG:initial-default", "7")
            .unwrap();
        let mut fields = evolved.fields().to_vec();
        fields.push(quantity);
        evolved
            .set_dtype(DataType::from(StructType::from_fields(fields).unwrap()))
            .unwrap();
        super::assign_field_ids(&mut evolved, 4).unwrap();
        table.evolve_schema(evolved).unwrap();

        let arrow = table.schema().unwrap().clone().into_arrow_schema().unwrap();
        let widened = arrow_array::RecordBatch::try_new(
            arrow.clone(),
            vec![
                std::sync::Arc::new(arrow_array::Int64Array::from(vec![2_i64])),
                std::sync::Arc::new(arrow_array::StringArray::from(vec![Some("MSFT")])),
                std::sync::Arc::new(arrow_array::StringArray::from(vec![Some("XNYS")])),
                std::sync::Arc::new(arrow_array::Int64Array::from(vec![50_i64])),
            ],
        )
        .unwrap();
        table
            .commit_append(yggdryl::arrow::batch_reader(arrow, [widened]))
            .unwrap();

        // The file written before the column existed reads its initial
        // default; the file written after keeps its own value.
        let mut quantities = Vec::new();
        for batch in table.scan(None).unwrap() {
            let batch = batch.unwrap();
            let column = batch.column_by_name("quantity").unwrap();
            assert_eq!(column.null_count(), 0);
            quantities.extend(
                column
                    .as_any()
                    .downcast_ref::<arrow_array::Int64Array>()
                    .unwrap()
                    .values()
                    .iter()
                    .copied(),
            );
        }
        quantities.sort_unstable();
        assert_eq!(quantities, [7, 50]);
    }

    #[test]
    fn a_table_whose_schema_evolved_reads_old_files_with_the_new_column_null() {
        let path = root("evolution");
        let schema = trade_schema();
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
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
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
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
        let reopened = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
        assert_eq!(reopened.data_files().unwrap().len(), 1);

        // The read is where absence shows up, and a missing resource is empty
        // rather than an error, so the scan yields no rows.
        assert_eq!(collect(reopened.scan(None).unwrap()).len(), 0);
    }

    #[test]
    fn a_v1_table_writes_and_reads_without_sequence_numbers() {
        let path = root("v1");
        let schema = trade_schema();
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            // v1 writes no sequence number: the contract pinned here.
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
            table.current_snapshot().unwrap().unwrap().sequence_number,
            None,
            "v1 snapshots carry no sequence number"
        );
        let reopened = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
        assert_eq!(
            reopened.metadata().unwrap().format_version(),
            // v1 writes no sequence number: the contract pinned here.
            FormatVersion::V1
        );
        assert_eq!(collect(reopened.scan(None).unwrap()).len(), 2);
    }

    #[test]
    fn a_v3_table_tracks_row_lineage_across_commits() {
        let path = root("v3");
        let schema = trade_schema();
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
            schema,
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        assert_eq!(table.metadata().unwrap().next_row_id(), Some(0));

        let first = trades(&[1, 2], &[Some("AAPL"), Some("MSFT")], &[Some("XNAS"); 2]);
        table
            .commit_append(yggdryl::arrow::batch_reader(first.schema(), [first]))
            .unwrap();
        assert_eq!(
            table.current_snapshot().unwrap().unwrap().first_row_id,
            Some(0)
        );
        assert_eq!(
            table.current_snapshot().unwrap().unwrap().added_rows,
            Some(2)
        );
        assert_eq!(table.metadata().unwrap().next_row_id(), Some(2));
        assert_eq!(table.manifests().unwrap()[0].first_row_id, Some(0));
        assert_eq!(table.data_files().unwrap()[0].0.first_row_id, Some(0));

        let second = trades(&[3], &[Some("NVDA")], &[Some("XNYS")]);
        table
            .commit_append(yggdryl::arrow::batch_reader(second.schema(), [second]))
            .unwrap();
        assert_eq!(
            table.current_snapshot().unwrap().unwrap().first_row_id,
            Some(2)
        );
        assert_eq!(
            table.current_snapshot().unwrap().unwrap().added_rows,
            Some(1)
        );
        assert_eq!(table.metadata().unwrap().next_row_id(), Some(3));
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

        let reopened = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
        assert_eq!(reopened.metadata().unwrap().next_row_id(), Some(3));
        assert_eq!(collect(reopened.scan(None).unwrap()).len(), 3);
    }

    #[test]
    fn v3_rewrites_are_rejected_before_reader_or_storage_mutation() {
        let path = root("v3-rewrite-lineage");
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
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
        assert_eq!(table.data_files().unwrap().len(), 2);

        let children = |directory: &str| {
            let mut paths: Vec<_> = std::fs::read_dir(path.join(directory))
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect();
            paths.sort();
            paths
        };
        let version = table.metadata_version().unwrap();
        let metadata_hash = table.metadata().unwrap().stable_hash();
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
            matches!(
                &merge_error,
                yggdryl::Error::External {
                    origin: "Iceberg",
                    source: None,
                    ..
                }
            ),
            "{merge_error:?}"
        );
        assert!(merge_error.to_string().contains("merge"), "{merge_error}");
        assert!(merge_error.to_string().contains("row IDs"), "{merge_error}");

        let compact_error = table
            .compact()
            .expect_err("v3 compaction must preserve existing row IDs");
        assert!(
            matches!(
                &compact_error,
                yggdryl::Error::External {
                    origin: "Iceberg",
                    source: None,
                    ..
                }
            ),
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

        assert_eq!(table.metadata_version().unwrap(), version);
        assert_eq!(table.metadata().unwrap().stable_hash(), metadata_hash);
        assert_eq!(children("data"), data_files);
        assert_eq!(children("metadata"), metadata_files);
    }

    #[test]
    fn v2_still_permits_keyed_merge_and_compaction() {
        let path = root("v2-rewrite-lineage");
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            // v2 permits the rewrites v3 refuses: the contract pinned here.
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
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            // The v2 -> v3 upgrade path.
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
        assert_eq!(table.metadata().unwrap().next_row_id(), Some(0));

        let added = trades(&[3], &[Some("NVDA")], &[Some("XNYS")]);
        table
            .commit_append(yggdryl::arrow::batch_reader(added.schema(), [added]))
            .unwrap();
        let snapshot = table.current_snapshot().unwrap().unwrap();
        assert_eq!(snapshot.first_row_id, Some(0));
        assert_eq!(snapshot.added_rows, Some(3));
        assert_eq!(table.metadata().unwrap().next_row_id(), Some(3));
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
        let message = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
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
        let message = IcebergTable::open(LocalFolder::new(&path).unwrap())
            .unwrap_err()
            .to_string();
        assert!(message.contains("metadata document"), "{message}");
    }

    #[test]
    fn a_manifest_describes_itself_well_enough_to_be_read_alone() {
        let path = root("self-describing");
        let schema = trade_schema();
        let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
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
        Field, FormatVersion, IOBase, IcebergTable, PartitionSpec, collect, root, trade_schema,
        trades,
    };
    use yggdryl::DataType;
    use yggdryl::iceberg::assign_field_ids;
    use yggdryl::local::LocalFolder;

    /// A table partitioned by venue, with one commit per venue.
    ///
    /// One commit is one manifest, so this is also the smallest table whose
    /// manifest list has something to prune.
    fn venues(label: &str) -> (std::path::PathBuf, IcebergTable<LocalFolder>) {
        let path = root(label);
        let schema = trade_schema();
        let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
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
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
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
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
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
            DataType::Mic.nullable_field("venue"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
        assign_field_ids(&mut schema, 1).unwrap();
        schema.insert_metadata("ICEBERG:schema-id", "0").unwrap();
        let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
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
    use yggdryl::iceberg::{SortField, SortOrder, Transform};

    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use yggdryl::StructType;

    use arrow_array::{
        Array, Int64Array, RecordBatch, RecordBatchIterator, RecordBatchReader, StringArray,
    };
    use arrow_schema::{ArrowError, SchemaRef};

    use super::{
        FormatVersion, IOBase, IcebergTable, PartitionSpec, assign_field_ids, collect, root,
        trade_schema, trades,
    };
    use yggdryl::IOMedia;
    use yggdryl::holder::Buffer;
    use yggdryl::local::LocalFolder;
    use yggdryl::media::{IORecordOptions, RecordOptions};
    use yggdryl::{DataType, MimeType};

    /// Create a v3 venue-partitioned table and return the folder addressing it.
    fn table(label: &str) -> (std::path::PathBuf, LocalFolder) {
        table_at(label, FormatVersion::V3)
    }

    /// [`table`] at `version`, for a test whose writes only that version takes.
    fn table_at(label: &str, version: FormatVersion) -> (std::path::PathBuf, LocalFolder) {
        let path = root(label);
        let schema = trade_schema();
        let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
        IcebergTable::create(LocalFolder::new(&path).unwrap(), version, schema, spec).unwrap();
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

        // An overwrite replaces the partition its rows fall in - `XLON`,
        // which held `VOD` - and no other, and the table still reads as a
        // table.
        let batch = trades(&[9], &[Some("BP")], &[Some("XLON")]);
        folder
            .overwrite_arrow_reader(
                yggdryl::arrow::batch_reader(batch.schema(), [batch]),
                &options,
            )
            .unwrap();
        let mut rows = collect(folder.read_arrow_reader(&options).unwrap());
        rows.sort();
        assert_eq!(
            rows,
            vec![
                (1, Some("AAPL".to_owned()), Some("XNAS".to_owned())),
                (2, Some("MSFT".to_owned()), Some("XNYS".to_owned())),
                (9, Some("BP".to_owned()), Some("XLON".to_owned())),
            ]
        );

        // Every write was a snapshot, so the table has one commit per call.
        let table = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
        assert_eq!(table.metadata().unwrap().snapshots().len(), 3);
        assert_eq!(
            table.current_snapshot().unwrap().unwrap().operation(),
            "overwrite"
        );
    }

    #[test]
    fn a_write_with_a_match_key_upserts_the_table_through_the_same_surface() {
        // A keyed merge: v2 only, refused on v3.
        let (_path, mut folder) = table_at("handle-merge", FormatVersion::V2);
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
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            // A keyed merge: v2 only, refused on v3.
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
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
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
        // A declared schema comes back as it stands, declaring the order a
        // record read proves: the venue partitions arrive in tuple order.
        let declared = options.clone().with_field(trade_schema());
        let mut ordered = trade_schema();
        ordered
            .as_sort_mut()
            .set_by_texts(["venue nulls first"])
            .unwrap();
        assert_eq!(table.read_arrow_field(&declared).unwrap(), ordered);

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
        assert_eq!(table.metadata().unwrap().snapshots().len(), 2);
        assert_eq!(
            table.current_snapshot().unwrap().unwrap().operation(),
            "append"
        );
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
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            // Its writes include a keyed merge: v2 only, refused on v3.
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
        let past = table.current_snapshot().unwrap().unwrap().snapshot_id;
        let version = table.metadata_version().unwrap();

        // An overwrite replaces the partitions its rows fall in - `XLON`
        // held nothing - and keeps every other; the snapshot before it is
        // retained and still reads exactly as it was written.
        let batch = trades(&[9], &[Some("BP")], &[Some("XLON")]);
        table
            .overwrite_arrow_reader(
                yggdryl::arrow::batch_reader(batch.schema(), [batch]),
                &options,
            )
            .unwrap();
        assert_eq!(
            table.current_snapshot().unwrap().unwrap().operation(),
            "overwrite"
        );
        assert_eq!(table.metadata_version().unwrap(), version + 1);
        let mut rows = collect(table.read_arrow_reader(&options).unwrap());
        rows.sort();
        assert_eq!(
            rows,
            vec![
                (1, Some("AAPL".to_owned()), Some("XNAS".to_owned())),
                (2, Some("MSFT".to_owned()), Some("XNYS".to_owned())),
                (9, Some("BP".to_owned()), Some("XLON".to_owned())),
            ]
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
        let mut rows = collect(table.read_arrow_reader(&options).unwrap());
        rows.sort();
        assert_eq!(
            rows,
            vec![
                (1, Some("AAPL".to_owned()), Some("XNAS".to_owned())),
                (2, Some("MSFT".to_owned()), Some("XNYS".to_owned())),
                (9, Some("BP.L".to_owned()), Some("XLON".to_owned())),
                (10, Some("SHEL".to_owned()), Some("XLON".to_owned())),
            ]
        );

        // Reopening reads the same history this value already reports.
        let reopened = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
        assert_eq!(
            reopened.metadata_version().unwrap(),
            table.metadata_version().unwrap()
        );
        assert_eq!(reopened.metadata().unwrap().snapshots().len(), 3);
    }

    #[test]
    fn the_table_value_honours_the_row_limit_like_every_handle() {
        let path = root("handle-table-limit");
        let schema = trade_schema();
        let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
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
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
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
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
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
            .with_commit_batch_num(1);
        table
            .overwrite_arrow_reader(
                yggdryl::arrow::batch_reader(bad.schema(), [bad.clone()]),
                &untyped_table,
            )
            .unwrap();
        let untyped_leaf = leaf.record_options().unwrap().with_commit_batch_num(1);
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

    /// One batch per row, so a cadence of one batch is a commit per row.
    fn one_row_batches(batch: &RecordBatch) -> yggdryl::arrow::BatchReader {
        let rows: Vec<RecordBatch> = (0..batch.num_rows())
            .map(|row| batch.slice(row, 1))
            .collect();
        yggdryl::arrow::batch_reader(batch.schema(), rows)
    }

    #[test]
    fn table_commit_batch_num_publishes_each_intent_at_the_requested_cadence() {
        let path = root("handle-table-commit-cadence");
        let schema = trade_schema();
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            // Its writes include a keyed merge: v2 only, refused on v3.
            FormatVersion::V2,
            schema.clone(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let options = yggdryl::IOMedia::record_options(&table)
            .unwrap()
            .with_field(schema)
            .with_commit_batch_num(1);

        let batch = trades(
            &[1, 2, 3],
            &[Some("AAPL"), Some("MSFT"), Some("VOD")],
            &[Some("XNAS"), Some("XNYS"), Some("XLON")],
        );
        table
            .overwrite_arrow_reader(one_row_batches(&batch), &options)
            .unwrap();
        assert_eq!(table.metadata().unwrap().snapshots().len(), 3);
        assert_eq!(collect(table.read_arrow_reader(&options).unwrap()).len(), 3);

        let batch = trades(
            &[4, 5],
            &[Some("BP"), Some("SHEL")],
            &[Some("XLON"), Some("XLON")],
        );
        table
            .append_arrow_reader(one_row_batches(&batch), &options)
            .unwrap();
        assert_eq!(table.metadata().unwrap().snapshots().len(), 5);

        let merging = options.clone().with_merge_by(["id"]).unwrap();
        let batch = trades(
            &[2, 6],
            &[Some("MSFT.L"), Some("ARM")],
            &[Some("XNYS"), Some("XLON")],
        );
        table
            .merge_arrow_reader(one_row_batches(&batch), &merging)
            .unwrap();
        assert_eq!(table.metadata().unwrap().snapshots().len(), 7);
        assert_eq!(collect(table.read_arrow_reader(&options).unwrap()).len(), 6);
    }

    #[test]
    fn a_table_located_through_its_folder_keeps_the_same_commit_cadence() {
        let (path, mut folder) = table("handle-located-table-cadence");
        let options = options(&folder).with_commit_batch_num(1);
        let batch = trades(
            &[1, 2, 3],
            &[Some("AAPL"), Some("MSFT"), Some("VOD")],
            &[Some("XNAS"), Some("XNYS"), Some("XLON")],
        );

        folder
            .overwrite_arrow_reader(one_row_batches(&batch), &options)
            .unwrap();

        let reopened = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
        assert_eq!(reopened.metadata().unwrap().snapshots().len(), 3);
        assert_eq!(
            collect(reopened.read_arrow_reader(&options).unwrap()).len(),
            3
        );
    }

    #[test]
    fn an_unset_cadence_commits_a_table_once_and_cuts_its_files_at_the_target() {
        // Three one-row batches are one commit whatever the target file
        // size: the rows of a write are held under the spill bound until the
        // source ends, so a stream of any length is one snapshot. The target
        // cuts the files, never the commits: a one-byte target lays every
        // row in a file of its own, and every row still reads back once.
        let rows = || {
            one_row_batches(&trades(
                &[1, 2, 3],
                &[Some("AAPL"), Some("MSFT"), Some("VOD")],
                &[Some("XNAS"), Some("XNYS"), Some("XLON")],
            ))
        };
        for (label, target, files) in [
            ("default", None, 1),
            ("tiny", Some(1), 3),
            ("larger-than-the-stream", Some(1 << 20), 1),
        ] {
            let path = root(&format!("handle-table-byte-cadence-{label}"));
            let schema = trade_schema();
            let mut table = IcebergTable::create(
                LocalFolder::new(&path).unwrap(),
                FormatVersion::V3,
                schema.clone(),
                PartitionSpec::unpartitioned(),
            )
            .unwrap();
            if let Some(target) = target {
                let mut explicit = yggdryl::iceberg::IcebergOptions::default();
                explicit.set_target_file_size_bytes(target).unwrap();
                table.set_options(explicit);
            }
            let options = yggdryl::IOMedia::record_options(&table)
                .unwrap()
                .with_field(schema);
            assert_eq!(options.commit_batch_num(), None, "{label}");

            table.append_arrow_reader(rows(), &options).unwrap();
            assert_eq!(table.metadata().unwrap().snapshots().len(), 1, "{label}");
            assert_eq!(table.data_files().unwrap().len(), files, "{label}");
            assert_eq!(
                collect(table.read_arrow_reader(&options).unwrap()).len(),
                3,
                "{label}"
            );

            // An overwrite is one commit too, so it is one atomic replacement.
            table.overwrite_arrow_reader(rows(), &options).unwrap();
            assert_eq!(table.metadata().unwrap().snapshots().len(), 2, "{label}");
            assert_eq!(table.data_files().unwrap().len(), files, "{label}");
            assert_eq!(
                collect(table.read_arrow_reader(&options).unwrap()).len(),
                3,
                "{label}"
            );
            let _ = std::fs::remove_dir_all(&path);
        }
    }

    /// Store a target file size on the table at `path`, so every handle
    /// that opens it - a held [`Table`] or a folder addressing it - reads
    /// the same cadence.
    fn set_target_file_size(path: &std::path::Path, target: u64) {
        IcebergTable::open(LocalFolder::new(path).unwrap())
            .unwrap()
            .commit_metadata_changes(|metadata| {
                metadata.set_property("write.target-file-size-bytes", target.to_string())?;
                Ok(())
            })
            .unwrap();
    }

    /// The snapshots the table at `path` holds, read afresh.
    fn snapshots(path: &std::path::Path) -> usize {
        IcebergTable::open(LocalFolder::new(path).unwrap())
            .unwrap()
            .metadata()
            .unwrap()
            .snapshots()
            .len()
    }

    /// `(id, symbol, venue)` triples as [`collect`] answers them.
    fn triples(rows: &[(i64, &str, &str)]) -> Vec<(i64, Option<String>, Option<String>)> {
        rows.iter()
            .map(|(id, symbol, venue)| (*id, Some((*symbol).to_owned()), Some((*venue).to_owned())))
            .collect()
    }

    #[test]
    fn an_unset_cadence_commits_a_located_table_once_whatever_its_target() {
        // The folder addressing a table writes as the table does: every
        // write is one commit, under the 512 MiB default target and under a
        // one-byte target alike - the target cuts files, never commits - and
        // every row still reads back exactly once.
        for (label, target) in [("default", None), ("tiny", Some(1))] {
            // Its writes include a keyed merge: v2 only, refused on v3.
            let (path, mut folder) = table_at(
                &format!("handle-located-byte-cadence-{label}"),
                FormatVersion::V2,
            );
            if let Some(target) = target {
                set_target_file_size(&path, target);
            }
            let options = options(&folder);
            assert_eq!(options.commit_batch_num(), None, "{label}");
            let commits = |_batches: usize| 1;

            // The first commit overwrites, every later one appends.
            let batch = trades(
                &[1, 2, 3],
                &[Some("AAPL"), Some("MSFT"), Some("VOD")],
                &[Some("XNAS"), Some("XNYS"), Some("XLON")],
            );
            folder
                .overwrite_arrow_reader(one_row_batches(&batch), &options)
                .unwrap();
            let mut expected = commits(3);
            assert_eq!(snapshots(&path), expected, "{label}");

            let batch = trades(&[4, 5], &[Some("BP"), Some("SHEL")], &[Some("XLON"); 2]);
            folder
                .append_arrow_reader(one_row_batches(&batch), &options)
                .unwrap();
            expected += commits(2);
            assert_eq!(snapshots(&path), expected, "{label}");

            // A keyed merge upserts on every commit: the update of one
            // commit and the insert of the next both land, once.
            let merging = options.clone().with_merge_by(["id"]).unwrap();
            let batch = trades(
                &[2, 6],
                &[Some("MSFT.L"), Some("ARM")],
                &[Some("XNYS"), Some("XLON")],
            );
            folder
                .merge_arrow_reader(one_row_batches(&batch), &merging)
                .unwrap();
            expected += commits(2);
            assert_eq!(snapshots(&path), expected, "{label}");
            assert_eq!(
                collect(folder.read_arrow_reader(&options).unwrap()),
                triples(&[
                    (1, "AAPL", "XNAS"),
                    (2, "MSFT.L", "XNYS"),
                    (3, "VOD", "XLON"),
                    (4, "BP", "XLON"),
                    (5, "SHEL", "XLON"),
                    (6, "ARM", "XLON"),
                ]),
                "{label}"
            );
            let _ = std::fs::remove_dir_all(&path);
        }
    }

    #[test]
    fn an_unset_cadence_merges_a_held_table_once_keyed_or_not() {
        let path = root("handle-table-merge-byte-cadence");
        let schema = trade_schema();
        let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            // A keyed merge: v2 only, refused on v3.
            FormatVersion::V2,
            schema.clone(),
            spec,
        )
        .unwrap();
        let mut explicit = yggdryl::iceberg::IcebergOptions::default();
        explicit.set_target_file_size_bytes(1).unwrap();
        table.set_options(explicit);
        let options = yggdryl::IOMedia::record_options(&table)
            .unwrap()
            .with_field(schema);
        assert_eq!(options.commit_batch_num(), None);
        // One batch is one commit whatever the target.
        let seed = trades(
            &[10, 11, 12],
            &[Some("OLD"); 3],
            &[Some("XNAS"), Some("XLON"), Some("XNYS")],
        );
        table
            .append_arrow_reader(
                yggdryl::arrow::batch_reader(seed.schema(), [seed]),
                &options,
            )
            .unwrap();
        assert_eq!(table.metadata().unwrap().snapshots().len(), 1);

        // Keyed: the one-row batches are one commit, upserting by the key.
        let keyed = options.clone().with_merge_by(["id"]).unwrap();
        let batch = trades(
            &[10, 20],
            &[Some("NEW"), Some("ADD")],
            &[Some("XNAS"), Some("XLON")],
        );
        table
            .merge_arrow_reader(one_row_batches(&batch), &keyed)
            .unwrap();
        assert_eq!(table.metadata().unwrap().snapshots().len(), 2);
        assert_eq!(
            collect(table.read_arrow_reader(&options).unwrap()),
            triples(&[
                (10, "NEW", "XNAS"),
                (11, "OLD", "XLON"),
                (12, "OLD", "XNYS"),
                (20, "ADD", "XLON"),
            ])
        );

        // Keyed by the partition alone: the one commit replaces the two
        // partitions the rows fall in with all four rows, and the partition
        // no row names keeps its own.
        let batch = trades(
            &[1, 2, 3, 4],
            &[Some("A"), Some("B"), Some("C"), Some("D")],
            &[Some("XNAS"), Some("XLON"), Some("XNAS"), Some("XLON")],
        );
        assert!(options.merge_by().is_empty());
        table
            .merge_arrow_reader(one_row_batches(&batch), &options)
            .unwrap();
        assert_eq!(table.metadata().unwrap().snapshots().len(), 3);
        assert_eq!(
            collect(table.read_arrow_reader(&options).unwrap()),
            triples(&[
                (1, "A", "XNAS"),
                (2, "B", "XLON"),
                (3, "C", "XNAS"),
                (4, "D", "XLON"),
                (12, "OLD", "XNYS"),
            ])
        );
        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn a_merge_keyed_by_the_partition_alone_keeps_every_row_across_its_commits() {
        // Each door replaces a partition on the first commit of the write
        // that reaches it and appends on the later ones: the held table with
        // no key and the folder with the partition column as the key commit
        // once, and a write session asked for one batch a commit replaces on
        // the first and appends on the three after - the merge their own
        // refusal of an empty key leaves.
        let seed = trades(
            &[10, 11, 12],
            &[Some("OLD"); 3],
            &[Some("XNAS"), Some("XLON"), Some("XNYS")],
        );
        let incoming = trades(
            &[1, 2, 3, 4],
            &[Some("A"), Some("B"), Some("C"), Some("D")],
            &[Some("XNAS"), Some("XLON"), Some("XNAS"), Some("XLON")],
        );
        let expected = triples(&[
            (1, "A", "XNAS"),
            (2, "B", "XLON"),
            (3, "C", "XNAS"),
            (4, "D", "XLON"),
            (12, "OLD", "XNYS"),
        ]);
        for door in ["table", "folder", "session"] {
            let (path, mut folder) = table(&format!("handle-partition-merge-{door}"));
            set_target_file_size(&path, 1);
            let options = options(&folder);
            folder
                .append_arrow_reader(
                    yggdryl::arrow::batch_reader(seed.schema(), [seed.clone()]),
                    &options,
                )
                .unwrap();
            let by_partition = options.clone().with_merge_by(["venue"]).unwrap();
            match door {
                "table" => {
                    let mut table = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
                    table
                        .merge_arrow_reader(one_row_batches(&incoming), &options)
                        .unwrap();
                }
                "folder" => {
                    folder
                        .merge_arrow_reader(one_row_batches(&incoming), &by_partition)
                        .unwrap();
                }
                _ => {
                    // A session publishes by its own byte default, so one
                    // batch a commit is asked for.
                    let cadence = by_partition.clone().with_commit_batch_num(1);
                    let mut session = yggdryl::ArrowWriteSession::merge(&cadence).unwrap();
                    assert!(
                        session
                            .push(&mut folder, one_row_batches(&incoming))
                            .unwrap()
                    );
                    session.finish(&mut folder).unwrap();
                }
            }
            let commits = if door == "session" { 4 } else { 1 };
            assert_eq!(snapshots(&path), 1 + commits, "{door}");
            assert_eq!(
                collect(folder.read_arrow_reader(&options).unwrap()),
                expected,
                "{door}"
            );
            let _ = std::fs::remove_dir_all(&path);
        }
    }

    /// A session commits through the table it locates off the handle, so
    /// the handle's own table is closed after each cadence and reads the
    /// commits on its next verb, and a commit through it after the session
    /// is no conflict.
    #[test]
    fn a_write_session_leaves_the_handle_it_was_given_current() {
        let seed = trades(
            &[1, 2],
            &[Some("A"), Some("B")],
            &[Some("XNAS"), Some("XLON")],
        );
        let (path, folder) = table("session-current");
        let mut held = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
        let options = options(&folder);
        held.append_arrow_reader(
            yggdryl::arrow::batch_reader(seed.schema(), [seed.clone()]),
            &options,
        )
        .unwrap();
        let cadence = options
            .clone()
            .with_merge_by(["venue"])
            .unwrap()
            .with_commit_batch_num(1);
        let mut session = yggdryl::ArrowWriteSession::merge(&cadence).unwrap();
        let incoming = trades(&[3], &[Some("C")], &[Some("XNAS")]);
        assert!(session.push(&mut held, one_row_batches(&incoming)).unwrap());
        session.finish(&mut held).unwrap();
        assert_eq!(snapshots(&path), 2);
        assert_eq!(
            collect(held.read_arrow_reader(&options).unwrap()),
            triples(&[(2, "B", "XLON"), (3, "C", "XNAS")]),
            "the handle reads what the session committed"
        );
        let later = trades(&[4], &[Some("D")], &[Some("XLON")]);
        held.merge_arrow_reader(one_row_batches(&later), &cadence)
            .unwrap();
        assert_eq!(
            snapshots(&path),
            3,
            "a merge after the session rebases on it"
        );
        let mut rows = collect(held.read_arrow_reader(&options).unwrap());
        rows.sort();
        assert_eq!(
            rows,
            triples(&[(3, "C", "XNAS"), (4, "D", "XLON")]),
            "the merge replaced the XLON row through the handle"
        );
    }

    #[test]
    fn an_overwrite_replaces_the_partitions_its_rows_fall_in_through_every_door() {
        // `XNAS` and `XLON` are reached and replaced, `XNYS` is not and
        // keeps its row. The held table and the folder commit once; a write
        // session asked for one batch a commit replaces each partition on
        // the first commit that reaches it and appends on the later ones, so
        // every incoming row of a partition survives its four commits.
        let seed = trades(
            &[10, 11, 12],
            &[Some("OLD"); 3],
            &[Some("XNAS"), Some("XLON"), Some("XNYS")],
        );
        let incoming = trades(
            &[1, 2, 3, 4],
            &[Some("A"), Some("B"), Some("C"), Some("D")],
            &[Some("XNAS"), Some("XLON"), Some("XNAS"), Some("XLON")],
        );
        let expected = triples(&[
            (1, "A", "XNAS"),
            (2, "B", "XLON"),
            (3, "C", "XNAS"),
            (4, "D", "XLON"),
            (12, "OLD", "XNYS"),
        ]);
        for door in ["table", "folder", "session"] {
            let (path, mut folder) = table(&format!("handle-partition-overwrite-{door}"));
            let options = options(&folder);
            folder
                .append_arrow_reader(
                    yggdryl::arrow::batch_reader(seed.schema(), [seed.clone()]),
                    &options,
                )
                .unwrap();
            let kept = venue_files(&path, "XNYS");
            let result = match door {
                "table" => {
                    let mut table = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
                    table
                        .overwrite_arrow_reader(one_row_batches(&incoming), &options)
                        .unwrap()
                }
                "folder" => folder
                    .overwrite_arrow_reader(one_row_batches(&incoming), &options)
                    .unwrap(),
                _ => {
                    let cadence = options.clone().with_commit_batch_num(1);
                    let mut session = yggdryl::ArrowWriteSession::overwrite(&cadence).unwrap();
                    assert!(
                        session
                            .push(&mut folder, one_row_batches(&incoming))
                            .unwrap()
                    );
                    session.finish(&mut folder).unwrap()
                }
            };
            // Every door answers the four rows it read and wrote, whatever
            // it replaced and however many commits it took.
            assert_eq!(result, yggdryl::IOResult::new(4, 4), "{door}");
            let commits = if door == "session" { 4 } else { 1 };
            assert_eq!(snapshots(&path), 1 + commits, "{door}");
            let mut rows = collect(folder.read_arrow_reader(&options).unwrap());
            rows.sort();
            assert_eq!(rows, expected, "{door}");
            assert_eq!(
                venue_files(&path, "XNYS"),
                kept,
                "{door}: a partition no row reaches keeps its files"
            );
            let _ = std::fs::remove_dir_all(&path);
        }
    }

    /// The data files of one venue with the row lineage each carries, sorted.
    fn venue_files(path: &std::path::Path, venue: &str) -> Vec<(String, Option<i64>)> {
        let table = IcebergTable::open(LocalFolder::new(path).unwrap()).unwrap();
        let mut files: Vec<(String, Option<i64>)> = table
            .data_files()
            .unwrap()
            .into_iter()
            .filter(|(file, _)| {
                file.partition.first().and_then(yggdryl::Scalar::as_str) == Some(venue)
            })
            .map(|(file, _)| (file.file_path.to_string(), file.first_row_id))
            .collect();
        files.sort();
        files
    }

    #[test]
    fn an_overwrite_on_format_v3_carries_untouched_files_with_their_row_lineage() {
        // No stored row is read or rewritten, so the replacement holds on
        // v3: the files of the partitions the rows do not reach stay the
        // same files under the same row identifiers.
        let path = root("handle-partition-overwrite-v3");
        let schema = trade_schema();
        let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
            schema,
            spec,
        )
        .unwrap();
        let seed = trades(
            &[10, 11, 12],
            &[Some("OLD"); 3],
            &[Some("XNAS"), Some("XLON"), Some("XNYS")],
        );
        table
            .commit_append(yggdryl::arrow::batch_reader(seed.schema(), [seed]))
            .unwrap();
        let (xnas, xnys) = (venue_files(&path, "XNAS"), venue_files(&path, "XNYS"));
        assert!(xnas[0].1.is_some(), "a v3 file states its first row id");

        let incoming = trades(
            &[1, 2],
            &[Some("B"), Some("P")],
            &[Some("XLON"), Some("XPAR")],
        );
        table
            .commit_overwrite(yggdryl::arrow::batch_reader(incoming.schema(), [incoming]))
            .unwrap();
        assert_eq!(
            table.current_snapshot().unwrap().unwrap().operation(),
            "overwrite"
        );
        assert_eq!(venue_files(&path, "XNAS"), xnas);
        assert_eq!(venue_files(&path, "XNYS"), xnys);
        let mut rows = collect(table.scan(None).unwrap());
        rows.sort();
        assert_eq!(
            rows,
            triples(&[
                (1, "B", "XLON"),
                (2, "P", "XPAR"),
                (10, "OLD", "XNAS"),
                (12, "OLD", "XNYS"),
            ])
        );

        // A merge keyed by the partition alone is the same replacement, so
        // it holds on v3 too; a keyed one still rewrites stored rows.
        let options = yggdryl::IOMedia::record_options(&table).unwrap();
        let again = trades(&[3], &[Some("Q")], &[Some("XPAR")]);
        table
            .merge_arrow_reader(
                yggdryl::arrow::batch_reader(again.schema(), [again]),
                &options,
            )
            .unwrap();
        let mut rows = collect(table.scan(None).unwrap());
        rows.sort();
        assert_eq!(rows.len(), 4);
        assert!(rows.contains(&(3, Some("Q".to_owned()), Some("XPAR".to_owned()))));
        assert_eq!(venue_files(&path, "XNAS"), xnas);
        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn an_overwrite_with_no_row_replaces_nothing_and_a_scope_replaces_itself() {
        let (path, mut folder) = table("handle-partition-overwrite-empty");
        let options = options(&folder);
        let seed = trades(&[10, 11], &[Some("OLD"); 2], &[Some("XNAS"), Some("XLON")]);
        folder
            .append_arrow_reader(
                yggdryl::arrow::batch_reader(seed.schema(), [seed.clone()]),
                &options,
            )
            .unwrap();

        // No row reaches a partition: nothing is replaced, nothing committed.
        folder
            .overwrite_arrow_reader(yggdryl::arrow::batch_reader(seed.schema(), []), &options)
            .unwrap();
        assert_eq!(snapshots(&path), 1);
        assert_eq!(
            collect(folder.read_arrow_reader(&options).unwrap()).len(),
            2
        );

        // A `where` naming a partition replaces it whatever the rows reach:
        // with no row it is emptied, and the other partition is kept.
        let scoped = options.clone().with_filter("venue = 'XLON'").unwrap();
        folder
            .overwrite_arrow_reader(yggdryl::arrow::batch_reader(seed.schema(), []), &scoped)
            .unwrap();
        assert_eq!(snapshots(&path), 2);
        assert_eq!(
            collect(folder.read_arrow_reader(&options).unwrap()),
            triples(&[(10, "OLD", "XNAS")])
        );

        // A filterless scope replaces the whole table, and `clear` empties it.
        let mut table = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
        let whole = trades(&[7], &[Some("VOD")], &[Some("XLON")]);
        table
            .commit_overwrite_where(&[], yggdryl::arrow::batch_reader(whole.schema(), [whole]))
            .unwrap();
        assert_eq!(
            collect(table.scan(None).unwrap()),
            triples(&[(7, "VOD", "XLON")])
        );
        table.clear().unwrap();
        assert!(collect(table.scan(None).unwrap()).is_empty());
        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn a_sorted_table_orders_each_partition_across_the_batches_of_one_commit() {
        // Rows arriving out of the table's order, one batch each, are one
        // commit whose partition is sorted as a whole: the one file the
        // partition lands in reads back in symbol order, not arrival order.
        let path = root("handle-table-sorted-across-batches");
        let schema = trade_schema();
        let mut table = IcebergTable::create_sorted(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
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
        let options = yggdryl::IOMedia::record_options(&table)
            .unwrap()
            .with_field(schema);
        let batch = trades(
            &[1, 2, 3, 4],
            &[Some("d"), Some("c"), Some("b"), Some("a")],
            &[Some("XNAS"); 4],
        );
        table
            .append_arrow_reader(one_row_batches(&batch), &options)
            .unwrap();
        assert_eq!(table.metadata().unwrap().snapshots().len(), 1);
        let files = table.data_files().unwrap();
        assert_eq!(
            files.len(),
            1,
            "one partition, one file under the default target"
        );
        assert_eq!(files[0].0.sort_order_id, Some(1));
        assert_eq!(files[0].0.record_count, 4);
        // `collect` sorts, so the file's own row order is read off the ids.
        let ids: Vec<i64> = table
            .read_arrow_reader(&options)
            .unwrap()
            .flat_map(|batch| {
                batch
                    .unwrap()
                    .column_by_name("id")
                    .unwrap()
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .unwrap()
                    .values()
                    .to_vec()
            })
            .collect();
        assert_eq!(ids, [4, 3, 2, 1], "symbols a, b, c, d");
        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn num_threads_writes_the_same_files_in_the_same_order_and_refuses_zero() {
        let batch = trades(
            &[1, 2, 3, 4, 5, 6],
            &[
                Some("A"),
                Some("B"),
                Some("C"),
                Some("D"),
                Some("E"),
                Some("F"),
            ],
            &[
                Some("XNAS"),
                Some("XLON"),
                Some("XNYS"),
                Some("XNAS"),
                Some("XLON"),
                Some("XNYS"),
            ],
        );
        let mut layouts = Vec::new();
        for threads in [1_usize, 3] {
            let path = root(&format!("handle-table-num-threads-{threads}"));
            let schema = trade_schema();
            let mut table = IcebergTable::create(
                LocalFolder::new(&path).unwrap(),
                FormatVersion::V3,
                schema.clone(),
                PartitionSpec::identity(1, &schema, &["venue"]).unwrap(),
            )
            .unwrap();
            let options = yggdryl::IOMedia::record_options(&table)
                .unwrap()
                .with_field(schema)
                .with_num_threads(threads);
            table
                .append_arrow_reader(one_row_batches(&batch), &options)
                .unwrap();
            assert_eq!(table.metadata().unwrap().snapshots().len(), 1);
            // One file per partition holding both of its rows, whatever
            // thread wrote each, so the layout is the same on one thread as
            // on three; a scan's file order is the plan's, so the layouts
            // compare by partition.
            let mut layout: Vec<(Vec<yggdryl::Scalar>, i64)> = table
                .data_files()
                .unwrap()
                .iter()
                .map(|(file, _)| (file.partition.clone(), file.record_count))
                .collect();
            layout.sort();
            layouts.push(layout);
            let mut rows = collect(table.read_arrow_reader(&options).unwrap());
            rows.sort();
            assert_eq!(
                rows,
                triples(&[
                    (1, "A", "XNAS"),
                    (2, "B", "XLON"),
                    (3, "C", "XNYS"),
                    (4, "D", "XNAS"),
                    (5, "E", "XLON"),
                    (6, "F", "XNYS"),
                ])
            );

            // Zero is refused before the source is pulled: no snapshot is added.
            let error = table
                .append_arrow_reader(
                    one_row_batches(&batch),
                    &options.clone().with_num_threads(0),
                )
                .unwrap_err()
                .to_string();
            assert!(error.contains("$.num_threads"), "{error}");
            assert_eq!(table.metadata().unwrap().snapshots().len(), 1);
            let _ = std::fs::remove_dir_all(&path);
        }
        assert_eq!(layouts[0], layouts[1]);
        assert_eq!(
            layouts[0].len(),
            3,
            "one file per partition: {:?}",
            layouts[0]
        );
        assert!(
            layouts[0].iter().all(|(_, rows)| *rows == 2),
            "{:?}",
            layouts[0]
        );
    }

    #[test]
    fn a_table_source_failure_leaves_the_committed_prefix_visible() {
        let path = root("handle-table-partial-commit");
        let schema = trade_schema();
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
            schema.clone(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let options = yggdryl::IOMedia::record_options(&table)
            .unwrap()
            .with_field(schema)
            .with_commit_batch_num(1);
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
        assert_eq!(table.metadata().unwrap().snapshots().len(), 1);
        assert_eq!(
            collect(table.read_arrow_reader(&options).unwrap()),
            vec![(7, Some("NVDA".to_owned()), Some("XNAS".to_owned()))]
        );
        let reopened = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
        assert_eq!(reopened.metadata().unwrap().snapshots().len(), 1);
    }

    #[test]
    fn empty_append_and_merge_create_no_table_commit() {
        let path = root("handle-table-empty-write");
        let schema = trade_schema();
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
            schema.clone(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let options = yggdryl::IOMedia::record_options(&table)
            .unwrap()
            .with_field(schema.clone());
        let arrow = schema.clone().into_arrow_schema().unwrap();
        let version = table.metadata_version().unwrap();
        let snapshots = table.metadata().unwrap().snapshots().len();

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

        assert_eq!(table.metadata_version().unwrap(), version);
        assert_eq!(table.metadata().unwrap().snapshots().len(), snapshots);
    }
}

#[test]
fn time_travel_reads_a_previous_snapshot_by_id_and_by_ref() {
    let path = root("time-travel");
    let mut table = IcebergTable::create(
        LocalFolder::new(&path).unwrap(),
        FormatVersion::V3,
        trade_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();

    let first = trades(&[1], &[Some("AAPL")], &[Some("XNAS")]);
    table
        .commit_append(yggdryl::arrow::batch_reader(first.schema(), [first]))
        .unwrap();
    let past = table.current_snapshot().unwrap().unwrap().snapshot_id;
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
    let mut table = IcebergTable::create(
        LocalFolder::new(&path).unwrap(),
        FormatVersion::V3,
        trade_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    let version = table.metadata_version().unwrap();

    table
        .commit_metadata_changes(|metadata| {
            metadata.set_property("owner", "desk")?;
            Ok(())
        })
        .unwrap();
    assert_eq!(table.metadata_version().unwrap(), version + 1);
    assert_eq!(table.metadata().unwrap().property("owner"), Some("desk"));

    // A rejected change is a commit that never happened.
    let failed: yggdryl::Result<()> = table.commit_metadata_changes(|_| {
        Err(yggdryl::Error::Codec {
            format: "iceberg",
            position: 0,
            reason: smol_str::SmolStr::new_static("rejected"),
        })
    });
    assert!(failed.is_err());
    assert_eq!(table.metadata_version().unwrap(), version + 1);

    // The written document reads back with the change applied.
    let reopened = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
    assert_eq!(reopened.metadata().unwrap().property("owner"), Some("desk"));
    assert_eq!(reopened.metadata_version().unwrap(), version + 1);

    let _ = std::fs::remove_dir_all(&path);
}

/// The document is the commit and the hint only where a read starts, so a
/// hint the store refuses leaves the commit made: it answers `Ok`, the hint
/// keeps naming the version before, and a fresh handle walks past it to the
/// document the commit wrote.
#[test]
fn a_refused_hint_write_leaves_the_commit_made() {
    let filesystem = Arc::new(RefusedHintWrite::default());
    let folder =
        yggdryl::fs::FsFolder::from_path(filesystem.clone(), "bucket/table", None).unwrap();
    let mut table = IcebergTable::create(
        folder.clone(),
        FormatVersion::V3,
        trade_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    let version = table.metadata_version().unwrap();

    filesystem.arm();
    table
        .commit_metadata_changes(|metadata| {
            metadata.set_property("owner", "desk")?;
            Ok(())
        })
        .unwrap();
    assert!(!filesystem.is_armed(), "the hint write was refused");
    assert_eq!(table.metadata_version().unwrap(), version + 1);
    assert_eq!(table.metadata().unwrap().property("owner"), Some("desk"));
    assert_eq!(
        read_filesystem_bytes(
            filesystem.as_ref(),
            "bucket/table/metadata/version-hint.text"
        )
        .unwrap(),
        version.to_string().as_bytes(),
        "the hint names the version before"
    );
    let reopened = IcebergTable::open(folder).unwrap();
    assert_eq!(reopened.metadata_version().unwrap(), version + 1);
    assert_eq!(reopened.metadata().unwrap().property("owner"), Some("desk"));
}

/// A data commit whose hint the store refuses keeps every file its document
/// names - the data files, the manifest and the list - and a fresh handle
/// reads its rows.
#[test]
fn a_refused_hint_write_keeps_the_data_files_the_document_names() {
    let filesystem = Arc::new(RefusedHintWrite::default());
    let folder =
        yggdryl::fs::FsFolder::from_path(filesystem.clone(), "bucket/table", None).unwrap();
    let schema = trade_schema();
    let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
    let mut table = IcebergTable::create(folder.clone(), FormatVersion::V3, schema, spec).unwrap();
    let version = table.metadata_version().unwrap();

    filesystem.arm();
    let batch = trades(
        &[1, 2],
        &[Some("AAPL"), Some("MSFT")],
        &[Some("XNAS"), Some("XNYS")],
    );
    table
        .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
        .unwrap();
    assert!(!filesystem.is_armed(), "the hint write was refused");
    assert_eq!(table.metadata_version().unwrap(), version + 1);
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
    let reopened = IcebergTable::open(folder).unwrap();
    assert_eq!(reopened.metadata_version().unwrap(), version + 1);
    assert_eq!(
        collect(reopened.scan(None).unwrap())
            .iter()
            .map(|row| row.0)
            .collect::<Vec<_>>(),
        [1, 2],
        "the rows the document names are read"
    );
}

/// The document `metadata/v{version}.metadata.json` under `path` as JSON,
/// with the property `name` set to `value`, gzipped: what a commit under
/// the gzip codec would have written at that version.
fn gzipped_variant(path: &std::path::Path, version: u32, name: &str, value: &str) -> Vec<u8> {
    let plain = std::fs::read(path.join(format!("metadata/v{version}.metadata.json"))).unwrap();
    let mut document: serde_json::Value = serde_json::from_slice(&plain).unwrap();
    document["properties"][name] = serde_json::Value::String(value.to_owned());
    yggdryl::gzip::dump(&serde_json::to_vec(&document).unwrap()).unwrap()
}

/// The names `metadata/` holds under `path`, sorted.
fn metadata_names(path: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(path.join("metadata"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// A version has two names, and the create that claims one excludes only
/// that one: a commit that lands `v2.metadata.json` beside a
/// `v2.gz.metadata.json` another writer claimed under the gzip codec reads
/// that spelling once, withdraws its own document and is beaten - so it
/// rebases onto the gzip document and lands at version 3, that document its
/// parent, and the version holds one document.
#[test]
fn a_claim_beside_the_other_spelling_withdraws_and_rebases_onto_it() {
    let path = root("other-spelling-claim");
    let mut table = IcebergTable::create(
        LocalFolder::new(&path).unwrap(),
        FormatVersion::V3,
        trade_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    table.set_options(
        IcebergOptions::new()
            .with_commit_retries(2)
            .with_commit_min_backoff_ms(0)
            .with_commit_max_backoff_ms(0),
    );
    // The other writer's claim of version 2, under the gzip codec; it has
    // not written the hint yet, so the hint still names version 1 and the
    // commit below claims version 2 without a look past it.
    std::fs::write(
        path.join("metadata/v2.gz.metadata.json"),
        gzipped_variant(&path, 1, "write.metadata.compression-codec", "gzip"),
    )
    .unwrap();

    table
        .commit_metadata_changes(|metadata| {
            metadata.set_property("owner", "desk")?;
            Ok(())
        })
        .unwrap();
    for opened in [
        &table,
        &IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap(),
    ] {
        assert_eq!(opened.metadata_version().unwrap(), 3);
        assert_eq!(opened.metadata_file_name().unwrap(), "v3.gz.metadata.json");
        let metadata = opened.metadata().unwrap();
        assert_eq!(metadata.property("owner"), Some("desk"));
        let parent = &metadata.metadata_log().last().unwrap().1;
        assert!(
            parent.ends_with("/metadata/v2.gz.metadata.json"),
            "the other writer's document is the parent: {parent}"
        );
    }
    assert_eq!(
        metadata_names(&path),
        [
            "v1.metadata.json",
            "v2.gz.metadata.json",
            "v3.gz.metadata.json",
            "version-hint.text"
        ],
        "the withdrawn claim is gone"
    );
    let _ = std::fs::remove_dir_all(&path);
}

/// A version both of whose spellings hold a whole document forked - two
/// commits claimed it under two codecs - and a reader refuses it naming
/// both, through the hint, past an older hint and through a listing alike,
/// rather than choosing one by its own codec and dropping the other's
/// commit. A commit that meets the fork waits for one claim to withdraw,
/// and reports the fork once its budget is spent. A spelling still being
/// written is no document yet, and forks nothing.
#[test]
fn a_version_held_under_both_spellings_is_refused_as_a_fork() {
    let path = root("forked-version");
    let mut table = IcebergTable::create(
        LocalFolder::new(&path).unwrap(),
        FormatVersion::V3,
        trade_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    for owner in ["desk", "risk"] {
        table
            .commit_metadata_changes(|metadata| {
                metadata.set_property("owner", owner)?;
                Ok(())
            })
            .unwrap();
    }
    let mut held = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
    assert_eq!(held.metadata_version().unwrap(), 3);
    let hint = path.join("metadata/version-hint.text");
    let gzipped = gzipped_variant(&path, 3, "owner", "ops");

    // Half a gzip document is one still being written: no fork.
    std::fs::write(
        path.join("metadata/v3.gz.metadata.json"),
        &gzipped[..gzipped.len() / 2],
    )
    .unwrap();
    let opened = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
    assert_eq!(opened.metadata_file_name().unwrap(), "v3.metadata.json");

    std::fs::write(path.join("metadata/v3.gz.metadata.json"), &gzipped).unwrap();
    for (case, text) in [
        ("the hint", Some("3")),
        ("an older hint", Some("2")),
        ("a listing", None),
    ] {
        match text {
            Some(text) => std::fs::write(&hint, text).unwrap(),
            None => std::fs::remove_file(&hint).unwrap(),
        }
        let error = IcebergTable::open(LocalFolder::new(&path).unwrap())
            .and_then(|table| table.metadata_version())
            .unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains("v3.metadata.json")
                && message.contains("v3.gz.metadata.json")
                && message.contains("forked"),
            "{case}: {message}"
        );
    }

    // A commit holding version 3, its hint naming 2, walks into the fork,
    // waits as a beaten attempt, and reports the fork itself once the
    // budget is spent, holding what it held.
    std::fs::write(&hint, "2").unwrap();
    held.set_options(
        IcebergOptions::new()
            .with_commit_retries(2)
            .with_commit_min_backoff_ms(0)
            .with_commit_max_backoff_ms(0),
    );
    let error = held
        .commit_metadata_changes(|metadata| {
            metadata.set_property("owner", "late")?;
            Ok(())
        })
        .unwrap_err();
    assert!(error.to_string().contains("forked"), "{error}");
    assert_eq!(held.metadata_version().unwrap(), 3);
    let _ = std::fs::remove_dir_all(&path);
}

/// A create claims the one name `v1.metadata.json`, which cannot see a table
/// laid out under another - another catalog's
/// `00001-{uuid}.metadata.json`, a first document spelled
/// `v1.gz.metadata.json`, a later version - so it lists `metadata/` once
/// and refuses over any metadata document there, naming it and writing
/// nothing, rather than hiding that table behind a hint of its own. A
/// folder whose `metadata/` holds anything else is created in.
#[test]
fn a_create_over_any_metadata_document_is_refused() {
    for planted in [
        "00001-1b0f7a2e-6c8d-4a39-9f2e-2f0d6a5b8c41.metadata.json",
        "v1.gz.metadata.json",
        "v3.metadata.json",
    ] {
        let path = root("create-over-metadata");
        let metadata = path.join("metadata");
        std::fs::create_dir_all(&metadata).unwrap();
        std::fs::write(metadata.join(planted), b"{}").unwrap();
        let error = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
            trade_schema(),
            PartitionSpec::unpartitioned(),
        )
        .map(|_| ())
        .unwrap_err();
        assert!(error.is_conflict(), "{planted}: {error}");
        assert!(error.to_string().contains(planted), "{planted}: {error}");
        let names: Vec<String> = std::fs::read_dir(&metadata)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, [planted], "nothing is written beside it");
        let _ = std::fs::remove_dir_all(&path);
    }

    let path = root("create-over-other-files");
    std::fs::create_dir_all(path.join("metadata")).unwrap();
    std::fs::write(path.join("metadata").join("notes.txt"), b"desk").unwrap();
    let table = IcebergTable::create(
        LocalFolder::new(&path).unwrap(),
        FormatVersion::V3,
        trade_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    assert_eq!(table.metadata_version().unwrap(), 1);
    let _ = std::fs::remove_dir_all(&path);
}

/// A hint is where an open starts, and one that does not read as a version
/// is no hint: gone between a commit's removal and its re-creation, created
/// and still empty, holding what no version spells, failing as a store that
/// sizes a file before it reads it fails on one replaced in between, or
/// naming a document still being written. Each open lists `metadata/` and
/// answers the newest whole document; a hint naming one takes no listing.
#[test]
fn a_hint_that_reads_as_no_version_leaves_the_listing_to_answer() {
    let filesystem = Arc::new(ReplacedHint::default());
    let folder =
        yggdryl::fs::FsFolder::from_path(filesystem.clone(), "bucket/table", None).unwrap();
    let mut table = IcebergTable::create(
        folder.clone(),
        FormatVersion::V3,
        trade_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    for owner in ["desk", "risk"] {
        table
            .commit_metadata_changes(|metadata| {
                metadata.set_property("owner", owner)?;
                Ok(())
            })
            .unwrap();
    }
    assert_eq!(table.metadata_version().unwrap(), 3);
    let hint = "bucket/table/metadata/version-hint.text";
    let opens_at_the_newest = |listed: bool, case: &str| {
        let before = filesystem.listings();
        let opened =
            IcebergTable::open(folder.clone()).unwrap_or_else(|error| panic!("{case}: {error}"));
        assert_eq!(opened.metadata_version().unwrap(), 3, "{case}");
        assert_eq!(
            opened.metadata_file_name().unwrap(),
            "v3.metadata.json",
            "{case}"
        );
        assert_eq!(
            opened.metadata().unwrap().property("owner"),
            Some("risk"),
            "{case}"
        );
        assert_eq!(
            filesystem.listings() - before,
            usize::from(listed),
            "{case}"
        );
    };

    opens_at_the_newest(false, "the hint the last commit wrote");
    filesystem.delete_file(hint).unwrap();
    opens_at_the_newest(true, "no hint");
    for (text, case) in [
        (&b""[..], "an empty hint"),
        (b"three", "a hint of no number"),
        (b"v3\n", "a hint spelling a name"),
        (b"-1", "a hint of no version"),
    ] {
        write_filesystem_bytes(filesystem.as_ref(), hint, text).unwrap();
        opens_at_the_newest(true, case);
    }
    write_filesystem_bytes(filesystem.as_ref(), hint, b"3").unwrap();
    filesystem.fail_hint_reads(1);
    opens_at_the_newest(true, "a hint replaced under the read");
    assert_eq!(filesystem.pending(), (0, 0));

    // A hint naming a document still being written - empty, or not yet
    // whole - leaves the listing to answer, which steps below that name.
    let whole = read_filesystem_bytes(
        filesystem.as_ref(),
        "bucket/table/metadata/v3.metadata.json",
    )
    .unwrap();
    write_filesystem_bytes(filesystem.as_ref(), hint, b"4").unwrap();
    for (written, case) in [
        (&whole[..0], "a hinted document still empty"),
        (&whole[..whole.len() / 2], "a hinted document not yet whole"),
    ] {
        write_filesystem_bytes(
            filesystem.as_ref(),
            "bucket/table/metadata/v4.metadata.json",
            written,
        )
        .unwrap();
        opens_at_the_newest(true, case);
    }
}

/// A commit whose claim is lost to a winner it cannot read yet - the hint
/// replaced under its read and the listing cut short - is beaten all the
/// same: it waits, looks again, finds the winner and rebases onto it, rather
/// than reporting the version it lost.
#[test]
fn a_lost_claim_whose_winner_cannot_be_read_yet_waits_and_rebases() {
    let filesystem = Arc::new(ReplacedHint::default());
    let folder =
        yggdryl::fs::FsFolder::from_path(filesystem.clone(), "bucket/table", None).unwrap();
    let mut table = IcebergTable::create(
        folder.clone(),
        FormatVersion::V3,
        trade_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    table.set_options(
        IcebergOptions::new()
            .with_commit_retries(2)
            .with_commit_min_backoff_ms(0)
            .with_commit_max_backoff_ms(0),
    );

    filesystem.arm();
    table
        .commit_metadata_changes(|metadata| {
            metadata.set_property("loser", "rebased")?;
            Ok(())
        })
        .unwrap();
    assert_eq!(
        filesystem.pending(),
        (0, 0),
        "the beaten commit's reading of the winner failed"
    );
    for opened in [&table, &IcebergTable::open(folder).unwrap()] {
        assert_eq!(opened.metadata_version().unwrap(), 3);
        let metadata = opened.metadata().unwrap();
        assert_eq!(metadata.property("winner"), Some("visible"));
        assert_eq!(metadata.property("loser"), Some("rebased"));
    }
}

#[test]
fn a_refused_document_create_rolls_the_commit_back_and_claims_nothing() {
    let filesystem = Arc::new(RefusedDocumentWrite::default());
    let folder =
        yggdryl::fs::FsFolder::from_path(filesystem.clone(), "bucket/table", None).unwrap();
    let schema = trade_schema();
    let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
    let mut table = IcebergTable::create(folder.clone(), FormatVersion::V3, schema, spec).unwrap();
    let version = table.metadata_version().unwrap();

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
    // data files, the manifest, the list - and the version's document was
    // never created, so nothing claims the version.
    assert_eq!(table.metadata_version().unwrap(), version);
    assert!(table.current_snapshot().unwrap().is_none());
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
        paths
            .iter()
            .all(|path| !path.ends_with("/metadata/v2.metadata.json")),
        "no document claims the version: {paths:?}"
    );

    // The version was not claimed: the next commit takes it.
    let batch = trades(&[3], &[Some("NVDA")], &[Some("XNAS")]);
    table
        .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
        .unwrap();
    assert_eq!(table.metadata_version().unwrap(), version + 1);
    let reopened = IcebergTable::open(folder).unwrap();
    assert_eq!(reopened.metadata_version().unwrap(), version + 1);
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
    let mut table = IcebergTable::create(
        folder.clone(),
        FormatVersion::V3,
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
        table.metadata_version().unwrap(),
        3,
        "the rebase adopted the winner's version and committed after it"
    );
    assert_eq!(
        table.metadata().unwrap().property("winner"),
        Some("visible")
    );

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
    let named = table
        .current_snapshot()
        .unwrap()
        .unwrap()
        .manifest_list
        .clone();
    let named = named.rsplit('/').next().unwrap();
    assert!(
        lists[0].ends_with(named),
        "the list left is the one the snapshot names: {lists:?} vs {named}"
    );
    assert!(
        !removed[0].ends_with(named),
        "the list removed is the other one: {removed:?}"
    );
    let reopened = IcebergTable::open(folder).unwrap();
    assert_eq!(
        collect(reopened.scan(None).unwrap())
            .iter()
            .map(|row| row.0)
            .collect::<Vec<_>>(),
        [1]
    );
}

/// A store can durably create our metadata document, then lose the create
/// answer. The retry sees our exact snapshot already in the winning document;
/// publishing it again would duplicate its id, and rolling back its staging
/// would delete files the winner names.
#[test]
fn a_refused_create_of_our_own_snapshot_finishes_one_append() {
    let filesystem = Arc::new(SameVersionWinner::default());
    let folder =
        yggdryl::fs::FsFolder::from_path(filesystem.clone(), "bucket/own-snapshot", None).unwrap();
    let mut table = IcebergTable::create(
        folder.clone(),
        FormatVersion::V3,
        trade_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    table.set_options(
        IcebergOptions::new()
            .with_commit_retries(0)
            .with_commit_min_backoff_ms(0)
            .with_commit_max_backoff_ms(0),
    );

    filesystem.arm_own_snapshot();
    let rows = trades(&[70, 71], &[Some("A"), Some("B")], &[Some("X"), Some("Y")]);
    table
        .commit_append(yggdryl::arrow::batch_reader(rows.schema(), [rows]))
        .unwrap();

    assert_eq!(filesystem.injections(), 1);
    assert_eq!(table.metadata_version().unwrap(), 2);
    assert_eq!(table.metadata().unwrap().snapshots().len(), 1);
    let reopened = IcebergTable::open(folder.clone()).unwrap();
    assert_eq!(reopened.metadata_version().unwrap(), 2);
    assert_eq!(reopened.metadata().unwrap().snapshots().len(), 1);
    let mut ids: Vec<i64> = collect(reopened.scan(None).unwrap())
        .into_iter()
        .map(|(id, _, _)| id)
        .collect();
    ids.sort_unstable();
    assert_eq!(ids, [70, 71]);
    let paths = listed_paths(&folder);
    assert_eq!(
        paths
            .iter()
            .filter(|path| path.ends_with(".parquet"))
            .count(),
        1
    );
    assert_eq!(
        paths.iter().filter(|path| path.ends_with(".avro")).count(),
        2
    );
    assert_eq!(
        paths
            .iter()
            .filter(|path| path.ends_with(".metadata.json"))
            .count(),
        2
    );
}

/// A claim can be durable while its document is temporarily unreadable. The
/// retry of that same version must keep the attempt's snapshot and manifest
/// list until it can recognize the published document.
#[test]
fn a_delayed_own_snapshot_reuses_the_same_append_attempt() {
    let filesystem = Arc::new(SameVersionWinner::default());
    let folder =
        yggdryl::fs::FsFolder::from_path(filesystem.clone(), "bucket/delayed-snapshot", None)
            .unwrap();
    let mut table = IcebergTable::create(
        folder.clone(),
        FormatVersion::V3,
        trade_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    table.set_options(
        IcebergOptions::new()
            .with_commit_retries(2)
            .with_commit_min_backoff_ms(0)
            .with_commit_max_backoff_ms(0),
    );

    filesystem.arm_delayed_own_snapshot();
    let rows = trades(&[70, 71], &[Some("A"), Some("B")], &[Some("X"), Some("Y")]);
    table
        .commit_append(yggdryl::arrow::batch_reader(rows.schema(), [rows]))
        .unwrap();

    assert_eq!(filesystem.injections(), 1);
    assert_eq!(filesystem.retry_creates(), 1);
    assert_eq!(table.metadata_version().unwrap(), 2);
    assert_eq!(table.metadata().unwrap().snapshots().len(), 1);
    let reopened = IcebergTable::open(folder.clone()).unwrap();
    let mut ids: Vec<i64> = collect(reopened.scan(None).unwrap())
        .into_iter()
        .map(|(id, _, _)| id)
        .collect();
    ids.sort_unstable();
    assert_eq!(ids, [70, 71]);
    let paths = listed_paths(&folder);
    assert_eq!(
        paths
            .iter()
            .filter(|path| path.ends_with(".parquet"))
            .count(),
        1
    );
    assert_eq!(
        paths.iter().filter(|path| path.ends_with(".avro")).count(),
        2
    );
    assert_eq!(
        paths
            .iter()
            .filter(|path| path.ends_with(".metadata.json"))
            .count(),
        2
    );
}

/// A durable document may stay unreadable for the entire retry budget. Both
/// blind and keyed appends report the bounded conflict, but neither may roll
/// back files that document already names.
#[test]
fn an_unreadable_own_snapshot_keeps_its_files_after_retry_exhaustion() {
    for keyed in [false, true] {
        let filesystem = Arc::new(SameVersionWinner::default());
        let name = if keyed {
            "bucket/unreadable-keyed-snapshot"
        } else {
            "bucket/unreadable-snapshot"
        };
        let folder = yggdryl::fs::FsFolder::from_path(filesystem.clone(), name, None).unwrap();
        let mut schema = trade_schema();
        if keyed {
            schema
                .as_iceberg_mut()
                .set_identifier_field_ids(&[1])
                .unwrap();
        }
        let mut table = IcebergTable::create(
            folder.clone(),
            FormatVersion::V3,
            schema,
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        table.set_options(
            IcebergOptions::new()
                .with_commit_retries(0)
                .with_commit_min_backoff_ms(0)
                .with_commit_max_backoff_ms(0),
        );

        filesystem.arm_delayed_own_snapshot();
        let rows = trades(&[70, 71], &[Some("A"), Some("B")], &[Some("X"), Some("Y")]);
        let error = table
            .commit_append(yggdryl::arrow::batch_reader(rows.schema(), [rows]))
            .unwrap_err();
        assert!(error.is_conflict(), "{keyed}: {error}");
        assert!(
            error
                .to_string()
                .contains("concurrent Iceberg metadata commit"),
            "{keyed}: {error}"
        );
        assert_eq!(table.metadata_version().unwrap(), 1, "{keyed}");
        assert!(table.current_snapshot().unwrap().is_none(), "{keyed}");
        assert_eq!(filesystem.injections(), 1, "{keyed}");
        assert_eq!(filesystem.retry_creates(), 0, "{keyed}");

        filesystem.reveal_own_snapshot();
        let reopened = IcebergTable::open(folder.clone()).unwrap();
        assert_eq!(reopened.metadata_version().unwrap(), 2, "{keyed}");
        assert_eq!(reopened.metadata().unwrap().snapshots().len(), 1, "{keyed}");
        let mut ids: Vec<i64> = collect(reopened.scan(None).unwrap())
            .into_iter()
            .map(|(id, _, _)| id)
            .collect();
        ids.sort_unstable();
        assert_eq!(ids, [70, 71], "{keyed}");
        let paths = listed_paths(&folder);
        assert_eq!(
            paths
                .iter()
                .filter(|path| path.ends_with(".parquet"))
                .count(),
            1,
            "{keyed}"
        );
        assert_eq!(
            paths.iter().filter(|path| path.ends_with(".avro")).count(),
            2,
            "{keyed}"
        );
        assert_eq!(
            paths
                .iter()
                .filter(|path| path.ends_with(".metadata.json"))
                .count(),
            2,
            "{keyed}"
        );
    }
}

#[test]
fn a_same_version_publication_conflict_rebases_through_the_retry_gate() {
    let filesystem = Arc::new(SameVersionWinner::default());
    let folder =
        yggdryl::fs::FsFolder::from_path(filesystem.clone(), "bucket/table", None).unwrap();
    let mut table = IcebergTable::create(
        folder.clone(),
        FormatVersion::V3,
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
    assert_eq!(table.metadata_version().unwrap(), 3);
    assert_eq!(
        table.metadata().unwrap().property("winner"),
        Some("visible")
    );
    assert_eq!(table.metadata().unwrap().property("loser"), Some("visible"));

    let reopened = IcebergTable::open(folder).unwrap();
    assert_eq!(reopened.metadata_version().unwrap(), 3);
    assert_eq!(
        reopened.metadata().unwrap().property("winner"),
        Some("visible")
    );
    assert_eq!(
        reopened.metadata().unwrap().property("loser"),
        Some("visible")
    );
}

#[test]
fn the_inspection_tables_report_history_snapshots_and_files() {
    let path = root("inspect");
    let mut table = IcebergTable::create(
        LocalFolder::new(&path).unwrap(),
        FormatVersion::V3,
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
    let mut table = IcebergTable::create(
        LocalFolder::new(&path).unwrap(),
        FormatVersion::V3,
        trade_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    let version = table.metadata_version().unwrap();

    // A change that leaves the metadata inconsistent never becomes a document.
    let failed = table.commit_metadata_changes(|metadata| {
        *yggdryl::internals::iceberg_metadata::current_schema_id_mut(metadata) = 999;
        Ok(())
    });
    let message = failed.unwrap_err().to_string();
    assert!(message.contains("999"), "{message}");
    assert_eq!(table.metadata_version().unwrap(), version);
    assert!(table.schema().is_ok());

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn a_zero_row_append_commits_a_snapshot_that_reads_as_nothing() {
    let path = root("zero-row");
    let mut table = IcebergTable::create(
        LocalFolder::new(&path).unwrap(),
        FormatVersion::V3,
        trade_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();

    let empty = trades(&[], &[], &[]);
    table
        .commit_append(yggdryl::arrow::batch_reader(empty.schema(), [empty]))
        .unwrap();

    // The commit is real - it has a snapshot - and the table stays empty.
    assert!(table.current_snapshot().unwrap().is_some());
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
    let mut table = IcebergTable::create(
        LocalFolder::new(&path).unwrap(),
        FormatVersion::V3,
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
    let mut table = IcebergTable::create(
        LocalFolder::new(&path).unwrap(),
        FormatVersion::V3,
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

    let reopened = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
    let message = reopened.plan(&[]).unwrap_err().to_string();
    assert!(!message.is_empty());

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn a_tiny_write_target_rolls_one_append_into_multiple_data_files() {
    let path = root("target-size");
    let mut table = IcebergTable::create(
        LocalFolder::new(&path).unwrap(),
        FormatVersion::V3,
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
    let snapshot = table.current_snapshot().unwrap().unwrap().snapshot_id;
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
    let mut table = IcebergTable::create(
        LocalFolder::new(&path).unwrap(),
        FormatVersion::V3,
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
    let mut table = IcebergTable::create(
        LocalFolder::new(&path).unwrap(),
        FormatVersion::V3,
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
    let mut table = IcebergTable::create(
        LocalFolder::new(&path).unwrap(),
        // Compaction: v2 only, refused on v3.
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
    let past = table.current_snapshot().unwrap().unwrap().snapshot_id;
    let snapshots_before = table.metadata().unwrap().snapshots().len();

    let compaction = table.compact().unwrap();
    assert_eq!(compaction.files_before, 5);
    assert_eq!(compaction.files_after, 1);
    assert!(compaction.bytes_rewritten > 0);

    // Fewer files, identical rows, one `replace` snapshot.
    assert_eq!(table.data_files().unwrap().len(), 1);
    assert_eq!(collect(table.scan(None).unwrap()), before_rows);
    assert_eq!(
        table.metadata().unwrap().snapshots().len(),
        snapshots_before + 1
    );
    assert_eq!(
        table.current_snapshot().unwrap().unwrap().operation(),
        "replace"
    );
    assert_eq!(
        table
            .metadata()
            .unwrap()
            .snapshots()
            .last()
            .unwrap()
            .operation(),
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
    let version = table.metadata_version().unwrap();
    assert_eq!(table.compact().unwrap(), Compaction::default());
    assert_eq!(table.metadata_version().unwrap(), version);
    assert_eq!(
        table.metadata().unwrap().snapshots().len(),
        snapshots_before + 1
    );

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn compaction_respects_partitions_and_pruning_still_prunes_after_it() {
    let path = root("compact-partitions");
    let schema = trade_schema();
    let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
    let mut table = IcebergTable::create(
        LocalFolder::new(&path).unwrap(),
        // Compaction: v2 only, refused on v3.
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

    let mut table = IcebergTable::create(
        LocalFolder::new(&path).unwrap(),
        FormatVersion::V3,
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
    let reopened = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
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
    let mut table = IcebergTable::create(
        LocalFolder::new(&path).unwrap(),
        FormatVersion::V3,
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
    // The whole host, with no bound of the crate's own.
    assert_eq!(
        options.read_parallelism(),
        std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get)
    );
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
fn a_count_property_reads_as_every_count_reads_and_refuses_what_none_does() {
    use yggdryl::iceberg::IcebergOptions;

    let path = root("option-integer-spelling");
    let mut table = IcebergTable::create(
        LocalFolder::new(&path).unwrap(),
        FormatVersion::V3,
        trade_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();

    // Surrounding blanks and an explicit plus are no part of a number. The
    // two keys are the crate's own, which the official table-property reader
    // does not also hold to its untrimmed spelling.
    table
        .commit_metadata_changes(|metadata| {
            metadata.set_property(IcebergOptions::READ_PARALLEL_MIN_FILES_KEY, " 4 ")?;
            metadata.set_property(IcebergOptions::READ_PARALLEL_MIN_FILE_SIZE_KEY, "+9")?;
            Ok(())
        })
        .unwrap();
    let options = table.options().unwrap();
    assert_eq!(options.read_parallel_min_files(), 4);
    assert_eq!(options.read_parallel_min_file_size_bytes(), 9);

    // A fraction, an exponent, a unit, a word and a sign an unsigned count
    // cannot hold are refused naming the key and the text.
    for text in ["5.0", "1e3", "1_000", "512 MB", "many", "-1"] {
        table
            .commit_metadata_changes(|metadata| {
                metadata.set_property(IcebergOptions::READ_PARALLEL_MIN_FILES_KEY, text)?;
                Ok(())
            })
            .unwrap();
        let error = table.options().unwrap_err();
        assert!(
            matches!(
                error,
                yggdryl::Error::InvalidMetadataValue { ref key, .. }
                    if key == IcebergOptions::READ_PARALLEL_MIN_FILES_KEY
            ),
            "{text}: {error:?}"
        );
        assert!(error.to_string().contains(text), "{text}: {error}");
    }

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn zero_total_retry_budget_allows_a_zero_wait_rebase() {
    use yggdryl::iceberg::IcebergOptions;

    let path = root("commit-total-timeout");
    let mut winner = IcebergTable::create(
        LocalFolder::new(&path).unwrap(),
        FormatVersion::V3,
        trade_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    let mut stale = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
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
    assert_eq!(stale.metadata_version().unwrap(), 3);

    let reopened = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
    assert_eq!(
        reopened.metadata().unwrap().property("winner"),
        Some("visible")
    );
    assert_eq!(
        reopened.metadata().unwrap().property("loser"),
        Some("hidden")
    );

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn an_unparseable_option_property_is_typed_and_an_explicit_option_shadows_it() {
    use yggdryl::iceberg::IcebergOptions;

    let path = root("options-unparseable");
    let mut table = IcebergTable::create(
        LocalFolder::new(&path).unwrap(),
        FormatVersion::V3,
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
    let mut first = IcebergTable::create(
        LocalFolder::new(&path).unwrap(),
        FormatVersion::V3,
        trade_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    let mut second = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
    second.set_options(
        IcebergOptions::new()
            .with_commit_min_backoff_ms(1)
            .with_commit_max_backoff_ms(2),
    );
    assert_eq!(second.metadata_version().unwrap(), 1);

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
        second.metadata_version().unwrap(),
        3,
        "the rebase adopted the winner's version"
    );

    // Both rows, two snapshots, and the loser's snapshot parents the winner's.
    let reopened = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
    assert_eq!(reopened.metadata_version().unwrap(), 3);
    let rows = collect(reopened.scan(None).unwrap());
    assert_eq!(rows.iter().map(|row| row.0).collect::<Vec<_>>(), [1, 2]);
    assert_eq!(reopened.metadata().unwrap().snapshots().len(), 2);
    let current = reopened.current_snapshot().unwrap().unwrap();
    let parent = current.parent_snapshot_id.unwrap();
    let winner_snapshot = reopened.metadata().unwrap().snapshot_by_id(parent).unwrap();
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
    let mut first = IcebergTable::create(
        LocalFolder::new(&path).unwrap(),
        FormatVersion::V3,
        trade_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    let mut second = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
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

    let reopened = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
    assert_eq!(reopened.metadata_version().unwrap(), 3);
    assert_eq!(
        reopened.metadata().unwrap().property("owner"),
        Some("alpha")
    );
    assert_eq!(reopened.metadata().unwrap().property("team"), Some("beta"));

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn a_beaten_overwrite_exhausts_its_retries_into_a_conflict_naming_versions() {
    use yggdryl::iceberg::IcebergOptions;

    let path = root("overwrite-conflict");
    let mut first = IcebergTable::create(
        LocalFolder::new(&path).unwrap(),
        FormatVersion::V3,
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
    let mut second = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
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
    assert_eq!(second.metadata_version().unwrap(), 2);
    let reopened = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
    assert_eq!(reopened.metadata_version().unwrap(), 4);
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
    let mut table = IcebergTable::create(
        LocalFolder::new(&path).unwrap(),
        FormatVersion::V3,
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
    let mut table = IcebergTable::create(
        LocalFolder::new(&path).unwrap(),
        FormatVersion::V3,
        trade_schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    let first = trades(&[1], &[Some("AAPL")], &[Some("XNAS")]);
    table
        .commit_append(yggdryl::arrow::batch_reader(first.schema(), [first]))
        .unwrap();
    let past = table.current_snapshot().unwrap().unwrap().snapshot_id;

    table.create_branch("audit", past).unwrap();
    table.create_tag("v1", past).unwrap();
    let version = table.metadata_version().unwrap();

    // A taken name is refused and the refusal commits nothing.
    assert!(table.create_branch("audit", past).is_err());
    assert_eq!(table.metadata_version().unwrap(), version);

    let second = trades(&[2], &[Some("MSFT")], &[Some("XNYS")]);
    table
        .commit_append(yggdryl::arrow::batch_reader(second.schema(), [second]))
        .unwrap();
    let head = table.current_snapshot().unwrap().unwrap().snapshot_id;

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
    let cutoff = table.metadata().unwrap().last_updated_ms() + 10_000;
    let expired = table.expire_snapshots(Some(cutoff), None, &[]).unwrap();
    assert_eq!(expired, vec![past]);
    assert_eq!(table.metadata().unwrap().snapshots().len(), 1);
    assert_eq!(collect(table.scan(None).unwrap()).len(), 2);
    assert!(table.scan_at(past, &[], None).is_err());

    // Nothing left to expire commits nothing: no new version is written.
    let version = table.metadata_version().unwrap();
    assert_eq!(
        table.expire_snapshots(Some(i64::MAX), None, &[]).unwrap(),
        Vec::<i64>::new()
    );
    assert_eq!(table.metadata_version().unwrap(), version);

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
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
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

        let reopened = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
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
                Scalar::decimal128(1_500_000, 4),
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
        assert_eq!(first[5], Scalar::decimal128(1_500_000, 4));
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
            DataType::Serie(Arc::new(DataType::Int64.nullable_field("item"))).nullable_field("xs"),
            DataType::map_of(DataType::utf8(), point.clone(), false)
                .unwrap()
                .nullable_field("m"),
        ])
        .map(DataType::from)
        .unwrap();
        let children = vec![
            DataType::Int64.required_field("id"),
            point.clone().nullable_field("p"),
            DataType::Serie(Arc::new(deep.clone().nullable_field("item"))).nullable_field("rows"),
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
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            // A keyed merge: v2 only, refused on v3.
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
                            _ => Scalar::decimal128(*price, 4),
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
        assert_eq!(prices[&1], Scalar::decimal128(99_000, 4));
        assert_eq!(prices[&3], Scalar::decimal128(30_000, 4));
    }
}

mod concurrency {
    //! Real racing writers and a beaten merge.
    //!
    //! Nothing here serializes a writer: every version is claimed by an
    //! exclusive create of its document, so of the threads racing for one
    //! version exactly one lands it and the others are told, and what the
    //! table holds afterwards is exactly what the commits that returned
    //! wrote, in the order the snapshot log states.

    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::{Arc, Barrier};

    use super::*;

    /// The retry budget a racing appender commits under: a thousand
    /// attempts, 1 to 16 ms apart, inside ten minutes, so the budget is never
    /// what ends a commit. Of `n` commits one writer is beaten at most once
    /// per commit another writer lands, plus once per winner whose document
    /// it met before that document was whole; the jittered waits keep two
    /// beaten writers from meeting again in step.
    fn racing() -> IcebergOptions {
        IcebergOptions::new()
            .with_commit_retries(1_000)
            .with_commit_min_backoff_ms(1)
            .with_commit_max_backoff_ms(16)
            .with_commit_total_timeout_ms(600_000)
    }

    /// The table's snapshots from the first to the current one, walked from
    /// the current snapshot back through each parent: the log a reader
    /// trusts, read from the metadata alone.
    fn chain(table: &IcebergTable<LocalFolder>) -> Vec<yggdryl::iceberg::Snapshot> {
        let metadata = table.metadata().unwrap();
        let by_id: BTreeMap<i64, &yggdryl::iceberg::Snapshot> = metadata
            .snapshots()
            .iter()
            .map(|snapshot| (snapshot.snapshot_id, snapshot))
            .collect();
        let mut chain = Vec::new();
        let mut next = table
            .current_snapshot()
            .unwrap()
            .map(|snapshot| snapshot.snapshot_id);
        while let Some(id) = next {
            let snapshot = by_id[&id];
            chain.push(snapshot.clone());
            next = snapshot.parent_snapshot_id;
        }
        chain.reverse();
        chain
    }

    /// Every id one snapshot of the table reads.
    fn ids_at(table: &IcebergTable<LocalFolder>, snapshot_id: i64) -> BTreeSet<i64> {
        collect(table.scan_at(snapshot_id, &[], None).unwrap())
            .into_iter()
            .map(|(id, _, _)| id)
            .collect()
    }

    /// The `rows` rows of commit `commit`: ids `commit * 10 + row`, so a
    /// row's id names the commit that wrote it.
    fn batch(commit: i64, rows: i64) -> RecordBatch {
        let ids: Vec<i64> = (0..rows).map(|row| commit * 10 + row).collect();
        let symbols = vec![Some("S"); ids.len()];
        let venues = vec![Some("V"); ids.len()];
        trades(&ids, &symbols, &venues)
    }

    /// No lock serializes these writers, and none needs to: the old
    /// protocol's unserialized document writes could land one over another,
    /// which is why the test once held a lock around each commit; each
    /// version is now claimed by an exclusive create, so a stale writer that
    /// loses the claim is told and rebases.
    #[test]
    fn stale_threads_rebase_and_every_writer_lands() {
        let path = root("threads");
        let schema = trade_schema();
        IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
            schema,
            PartitionSpec::unpartitioned(),
        )
        .unwrap();

        // Every thread opens its handle at version 1, so every commit after
        // the first is beaten and must rebase.
        let opened = Arc::new(Barrier::new(4));
        let handles: Vec<_> = (0..4)
            .map(|writer: i64| {
                let path = path.clone();
                let opened = Arc::clone(&opened);
                std::thread::spawn(move || {
                    let mut table = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
                    table.set_options(racing());
                    opened.wait();
                    let batch = trades(
                        &[writer * 10, writer * 10 + 1],
                        &[Some("S"), Some("S")],
                        &[Some("V"), Some("V")],
                    );
                    table
                        .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
                        .unwrap();
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }

        let reopened = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
        let mut ids: Vec<i64> = collect(reopened.scan(None).unwrap())
            .into_iter()
            .map(|(id, _, _)| id)
            .collect();
        ids.sort_unstable();
        assert_eq!(ids, [0, 1, 10, 11, 20, 21, 30, 31]);
        // Four commits landed on top of the created table.
        assert_eq!(reopened.metadata_version().unwrap(), 5);
        let _ = std::fs::remove_dir_all(&path);
    }

    /// Eight writers, each holding version 1, append four times each with
    /// nothing between them while two readers open the table over and over:
    /// every row lands exactly once, every commit is one version, and the
    /// snapshots chain one onto the next. No open fails - a reader meets the
    /// hint between its removal and its re-creation, and documents still
    /// being written, and reads past both - and each answers a whole
    /// version: its snapshot holds the two rows of every commit up to it, it
    /// is never older than the reader's open before it, and an open begun
    /// after the last commit returned answers that commit.
    #[test]
    fn racing_appenders_each_land_every_commit_once_on_one_chain() {
        const WRITERS: i64 = 8;
        const COMMITS: i64 = 4;
        const READERS: usize = 2;
        let path = root("racing-appenders");
        IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
            trade_schema(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();

        let done = Arc::new(AtomicBool::new(false));
        let readers: Vec<_> = (0..READERS)
            .map(|reader| {
                let path = path.clone();
                let done = Arc::clone(&done);
                std::thread::spawn(move || {
                    let mut seen: Vec<u32> = Vec::new();
                    loop {
                        // Read before the open, so the last open begins
                        // after every writer returned.
                        let last = done.load(Ordering::Acquire);
                        let open = seen.len();
                        let table = IcebergTable::open(LocalFolder::new(&path).unwrap())
                            .unwrap_or_else(|error| panic!("reader {reader}, open {open}: {error}"));
                        let version = table.metadata_version().unwrap();
                        let rows = collect(table.scan(None).unwrap_or_else(|error| {
                            panic!("reader {reader}, open {open} at version {version}: {error}")
                        }))
                        .len();
                        assert_eq!(
                            rows,
                            2 * (version as usize - 1),
                            "reader {reader}, open {open}: version {version} holds every commit up to it"
                        );
                        if let Some(previous) = seen.last() {
                            assert!(
                                version >= *previous,
                                "reader {reader}, open {open}: version {version} after {previous}"
                            );
                        }
                        seen.push(version);
                        if last {
                            return seen;
                        }
                    }
                })
            })
            .collect();

        let start = Arc::new(Barrier::new(WRITERS as usize));
        let handles: Vec<_> = (0..WRITERS)
            .map(|writer| {
                let path = path.clone();
                let start = Arc::clone(&start);
                std::thread::spawn(move || {
                    let mut table = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
                    table.set_options(racing());
                    start.wait();
                    for commit in 0..COMMITS {
                        let rows = batch(writer * 10 + commit, 2);
                        table
                            .commit_append(yggdryl::arrow::batch_reader(rows.schema(), [rows]))
                            .unwrap_or_else(|error| {
                                panic!("writer {writer}, commit {commit}: {error}")
                            });
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
        done.store(true, Ordering::Release);
        let latest = 1 + (WRITERS * COMMITS) as u32;
        for reader in readers {
            let seen = reader.join().unwrap();
            assert!(seen.iter().all(|version| (1..=latest).contains(version)));
            assert_eq!(seen.last(), Some(&latest), "an open after the last commit");
        }

        let table = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
        let mut ids: Vec<i64> = collect(table.scan(None).unwrap())
            .into_iter()
            .map(|(id, _, _)| id)
            .collect();
        ids.sort_unstable();
        let expected: Vec<i64> = (0..WRITERS)
            .flat_map(|writer| (0..COMMITS).map(move |commit| writer * 10 + commit))
            .flat_map(|commit| [commit * 10, commit * 10 + 1])
            .collect();
        assert_eq!(ids, expected, "every row lands exactly once");
        assert_eq!(table.metadata_version().unwrap(), latest);

        // The chain from the current snapshot back holds every snapshot the
        // table has, each the child of the one before it, each one commit's
        // two rows, in the order the snapshot log states.
        let chain = chain(&table);
        assert_eq!(chain.len(), (WRITERS * COMMITS) as usize);
        assert_eq!(chain.len(), table.metadata().unwrap().snapshots().len());
        assert_eq!(chain[0].parent_snapshot_id, None);
        for (index, pair) in chain.windows(2).enumerate() {
            assert_eq!(
                pair[1].parent_snapshot_id,
                Some(pair[0].snapshot_id),
                "snapshot {} chains onto the one before it",
                index + 1
            );
        }
        for snapshot in &chain {
            assert_eq!(snapshot.summary_value("operation"), Some("append"));
            assert_eq!(snapshot.summary_value("added-records"), Some("2"));
        }
        let logged: Vec<i64> = table
            .metadata()
            .unwrap()
            .snapshot_log()
            .iter()
            .map(|entry| entry.1)
            .collect();
        let chained: Vec<i64> = chain.iter().map(|snapshot| snapshot.snapshot_id).collect();
        assert_eq!(logged, chained, "the snapshot log is the chain");
        let _ = std::fs::remove_dir_all(&path);
    }

    /// Appenders racing a writer that flips `write.metadata.compression-codec`
    /// back and forth: the version after each flip is claimed under two
    /// spellings at once, and each claim reads the other spelling before it
    /// commits, so every commit lands exactly once, every version holds one
    /// document, and the snapshots chain one onto the next.
    #[test]
    fn codec_flips_racing_appenders_leave_one_document_per_version() {
        const APPENDERS: i64 = 4;
        const APPENDS: i64 = 3;
        const FLIPS: usize = 4;
        let path = root("codec-flips");
        IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
            trade_schema(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();

        let start = Arc::new(Barrier::new(APPENDERS as usize + 1));
        let mut handles: Vec<_> = (0..APPENDERS)
            .map(|appender| {
                let path = path.clone();
                let start = Arc::clone(&start);
                std::thread::spawn(move || {
                    let mut table = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
                    table.set_options(racing());
                    start.wait();
                    for append in 0..APPENDS {
                        let rows = batch(appender * 10 + append, 2);
                        table
                            .commit_append(yggdryl::arrow::batch_reader(rows.schema(), [rows]))
                            .unwrap_or_else(|error| {
                                panic!("appender {appender}, append {append}: {error}")
                            });
                    }
                })
            })
            .collect();
        handles.push({
            let path = path.clone();
            let start = Arc::clone(&start);
            std::thread::spawn(move || {
                let mut table = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
                table.set_options(racing());
                start.wait();
                for flip in 0..FLIPS {
                    let codec = if flip % 2 == 0 { "gzip" } else { "none" };
                    table
                        .commit_metadata_changes(|metadata| {
                            metadata.set_property("write.metadata.compression-codec", codec)?;
                            Ok(())
                        })
                        .unwrap_or_else(|error| panic!("flip {flip}: {error}"));
                }
            })
        });
        for handle in handles {
            handle.join().unwrap();
        }

        let table = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
        let latest = 1 + (APPENDERS * APPENDS) as u32 + FLIPS as u32;
        assert_eq!(table.metadata_version().unwrap(), latest);
        let names: BTreeSet<String> = std::fs::read_dir(path.join("metadata"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".metadata.json"))
            .collect();
        for version in 1..=latest {
            let spelled = [
                format!("v{version}.metadata.json"),
                format!("v{version}.gz.metadata.json"),
            ]
            .into_iter()
            .filter(|name| names.contains(name))
            .count();
            assert_eq!(
                spelled, 1,
                "version {version} holds one document: {names:?}"
            );
        }
        assert_eq!(names.len(), latest as usize, "{names:?}");
        let mut ids: Vec<i64> = collect(table.scan(None).unwrap())
            .into_iter()
            .map(|(id, _, _)| id)
            .collect();
        ids.sort_unstable();
        let expected: Vec<i64> = (0..APPENDERS)
            .flat_map(|appender| (0..APPENDS).map(move |append| appender * 10 + append))
            .flat_map(|commit| [commit * 10, commit * 10 + 1])
            .collect();
        assert_eq!(ids, expected, "every row lands exactly once");
        let chain = chain(&table);
        assert_eq!(chain.len(), (APPENDERS * APPENDS) as usize);
        for pair in chain.windows(2) {
            assert_eq!(pair[1].parent_snapshot_id, Some(pair[0].snapshot_id));
        }
        let _ = std::fs::remove_dir_all(&path);
    }

    /// The interleaving the race above can lose a commit in, laid out by
    /// hand: a writer reads a gzip claim of version 2, that claim is then
    /// withdrawn - as its own check withdraws it once a plain claim of the
    /// version landed before that check ran - and an appender's plain
    /// version 2 commits in its place. The writer's commit claims version 3
    /// on the document it read, finds that base no longer the version's one
    /// document, withdraws its claim and rebases onto the version that
    /// stands: the appender's rows, which it was told landed, are kept.
    #[test]
    fn a_commit_built_on_a_withdrawn_claim_rebases_onto_the_version_that_stands() {
        let path = root("withdrawn-base");
        let metadata = path.join("metadata");
        IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
            trade_schema(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        // The appender holds version 1 before anything else lands.
        let mut appender = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();

        // The flipper claims version 2 under gzip, and the writer reads it.
        let mut flipper = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
        flipper
            .commit_metadata_changes(|metadata| {
                metadata.set_property("write.metadata.compression-codec", "gzip")?;
                Ok(())
            })
            .unwrap();
        let mut writer = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
        assert_eq!(writer.metadata_version().unwrap(), 2);

        // The claim is withdrawn, and the appender's plain version 2 lands
        // alone and commits.
        std::fs::remove_file(metadata.join("v2.gz.metadata.json")).unwrap();
        let rows = batch(1, 2);
        appender
            .commit_append(yggdryl::arrow::batch_reader(rows.schema(), [rows]))
            .unwrap();
        assert!(metadata.join("v2.metadata.json").is_file());

        // The writer commits on the withdrawn document it read.
        let rows = batch(2, 2);
        writer
            .commit_append(yggdryl::arrow::batch_reader(rows.schema(), [rows]))
            .unwrap();

        let table = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
        assert_eq!(table.metadata_version().unwrap(), 3);
        let mut ids: Vec<i64> = collect(table.scan(None).unwrap())
            .into_iter()
            .map(|(id, _, _)| id)
            .collect();
        ids.sort_unstable();
        assert_eq!(ids, [10, 11, 20, 21], "the appender's commit is kept");
        let chain = chain(&table);
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[1].parent_snapshot_id, Some(chain[0].snapshot_id));
        assert_eq!(
            table
                .metadata()
                .unwrap()
                .property("write.metadata.compression-codec"),
            None,
            "the withdrawn claim's change is in no document"
        );
        let mut names: Vec<String> = std::fs::read_dir(&metadata)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".metadata.json"))
            .collect();
        names.sort();
        assert_eq!(
            names,
            ["v1.metadata.json", "v2.metadata.json", "v3.metadata.json"]
        );
        let _ = std::fs::remove_dir_all(&path);
    }

    /// A claim withdrawn after a writer read it takes back the files it
    /// named - its rebase withdraws the manifest list it wrote - so the
    /// writer's next commit finds the list of the snapshot it holds gone.
    /// Laid out by hand: the claim of version 2 and its list removed after
    /// the writer read them. The writer's commit finds the document it holds
    /// no longer stands, and rebases onto the version that does - version 1
    /// - rather than failing on the missing list.
    #[test]
    fn a_commit_on_a_claim_whose_files_were_withdrawn_rebases_onto_what_stands() {
        let path = root("withdrawn-files");
        let metadata = path.join("metadata");
        IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
            trade_schema(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let mut claimer = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
        let rows = batch(1, 2);
        claimer
            .commit_append(yggdryl::arrow::batch_reader(rows.schema(), [rows]))
            .unwrap();
        let list = claimer
            .current_snapshot()
            .unwrap()
            .unwrap()
            .manifest_list
            .clone();
        let mut writer = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
        assert_eq!(writer.metadata_version().unwrap(), 2);

        // The claim of version 2 is withdrawn, and its list with it.
        std::fs::remove_file(metadata.join("v2.metadata.json")).unwrap();
        std::fs::remove_file(yggdryl::Url::from_str(&list).unwrap().into_path().unwrap()).unwrap();

        let rows = batch(2, 2);
        writer
            .commit_append(yggdryl::arrow::batch_reader(rows.schema(), [rows]))
            .unwrap();
        let table = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
        assert_eq!(table.metadata_version().unwrap(), 2);
        let mut ids: Vec<i64> = collect(table.scan(None).unwrap())
            .into_iter()
            .map(|(id, _, _)| id)
            .collect();
        ids.sort_unstable();
        assert_eq!(ids, [20, 21], "the commit built on the version that stands");
        assert_eq!(chain(&table).len(), 1);
        let _ = std::fs::remove_dir_all(&path);
    }

    /// Appends racing whole-table overwrites: every attempt either lands or
    /// is a typed `CommitConflict`, and the rows the table holds are exactly
    /// the replay of its snapshot log - an overwrite the one row set it
    /// wrote, an append what was there and its own rows.
    #[test]
    fn appends_racing_overwrites_hold_what_the_snapshot_log_replays() {
        const APPENDERS: i64 = 4;
        const APPENDS: i64 = 3;
        const OVERWRITERS: i64 = 2;
        const OVERWRITES: i64 = 2;
        let path = root("appends-racing-overwrites");
        IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
            trade_schema(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();

        // An appender's commit is `100 + appender * 10 + append` and writes
        // two rows; an overwrite's is `500 + overwriter * 10 + overwrite`
        // and writes three, the same three on every attempt.
        let start = Arc::new(Barrier::new((APPENDERS + OVERWRITERS) as usize));
        let appenders: Vec<_> = (0..APPENDERS)
            .map(|appender| {
                let path = path.clone();
                let start = Arc::clone(&start);
                std::thread::spawn(move || {
                    let mut table = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
                    table.set_options(racing());
                    start.wait();
                    for append in 0..APPENDS {
                        let rows = batch(100 + appender * 10 + append, 2);
                        table
                            .commit_append(yggdryl::arrow::batch_reader(rows.schema(), [rows]))
                            .unwrap_or_else(|error| {
                                panic!("appender {appender}, append {append}: {error}")
                            });
                    }
                })
            })
            .collect();

        // An overwrite cannot rebase, so each attempt plans on a fresh
        // handle and a beaten one reports its conflict after two short
        // waits; the overwriter tries again until it lands, counting.
        let overwriters: Vec<_> = (0..OVERWRITERS)
            .map(|overwriter| {
                let path = path.clone();
                let start = Arc::clone(&start);
                std::thread::spawn(move || {
                    start.wait();
                    let (mut attempts, mut landed, mut conflicts) = (0_u32, 0_u32, 0_u32);
                    for overwrite in 0..OVERWRITES {
                        loop {
                            assert!(attempts < 10_000, "overwriter {overwriter} never lands");
                            attempts += 1;
                            let mut table =
                                IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
                            table.set_options(racing().with_commit_retries(2));
                            let rows = batch(500 + overwriter * 10 + overwrite, 3);
                            match table.commit_overwrite(yggdryl::arrow::batch_reader(
                                rows.schema(),
                                [rows],
                            )) {
                                Ok(()) => {
                                    landed += 1;
                                    break;
                                }
                                Err(error)
                                    if error.is_conflict()
                                        && error.to_string().contains("got beaten") =>
                                {
                                    conflicts += 1;
                                }
                                Err(error) => {
                                    panic!(
                                        "overwriter {overwriter}, overwrite {overwrite}: {error}"
                                    )
                                }
                            }
                        }
                    }
                    (attempts, landed, conflicts)
                })
            })
            .collect();
        for handle in appenders {
            handle.join().unwrap();
        }
        let mut landed_overwrites = 0;
        for handle in overwriters {
            let (attempts, landed, conflicts) = handle.join().unwrap();
            println!("overwriter: {attempts} attempts, {landed} landed, {conflicts} conflicts");
            assert_eq!(
                landed + conflicts,
                attempts,
                "every attempt lands or conflicts"
            );
            landed_overwrites += landed;
        }
        assert!(landed_overwrites >= 1, "an overwrite landed");
        assert_eq!(landed_overwrites, (OVERWRITERS * OVERWRITES) as u32);

        // Replay the log the metadata states, snapshot by snapshot: an
        // overwrite reads exactly the one commit's rows it wrote, an append
        // reads everything before it and exactly one commit's rows more.
        let table = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
        let chain = chain(&table);
        assert_eq!(
            chain.len(),
            (APPENDERS * APPENDS + OVERWRITERS * OVERWRITES) as usize,
            "one snapshot per commit that landed"
        );
        let mut replayed: BTreeSet<i64> = BTreeSet::new();
        let mut commits: Vec<i64> = Vec::new();
        for snapshot in &chain {
            let read = ids_at(&table, snapshot.snapshot_id);
            let operation = snapshot.summary_value("operation");
            let added: BTreeSet<i64> = match operation {
                Some("append") => {
                    assert!(replayed.is_subset(&read), "an append removes nothing");
                    read.difference(&replayed).copied().collect()
                }
                Some("overwrite") => read.clone(),
                other => panic!("unexpected operation {other:?}"),
            };
            let commit = *added.first().unwrap() / 10;
            let expected_rows = if commit >= 500 { 3 } else { 2 };
            assert_eq!(
                added,
                (0..expected_rows).map(|row| commit * 10 + row).collect(),
                "snapshot {} adds one whole commit",
                snapshot.snapshot_id
            );
            assert_eq!(
                operation == Some("overwrite"),
                commit >= 500,
                "the operation the log states is the commit's"
            );
            let count = expected_rows.to_string();
            assert_eq!(
                snapshot.summary_value("added-records"),
                Some(count.as_str())
            );
            replayed = match operation {
                Some("overwrite") => added,
                _ => replayed.union(&added).copied().collect(),
            };
            assert_eq!(read, replayed);
            commits.push(commit);
        }
        commits.sort_unstable();
        let mut expected: Vec<i64> = (0..APPENDERS)
            .flat_map(|appender| (0..APPENDS).map(move |append| 100 + appender * 10 + append))
            .chain((0..OVERWRITERS).flat_map(|overwriter| {
                (0..OVERWRITES).map(move |overwrite| 500 + overwriter * 10 + overwrite)
            }))
            .collect();
        expected.sort_unstable();
        assert_eq!(commits, expected, "every commit that returned, once");
        let current: BTreeSet<i64> = collect(table.scan(None).unwrap())
            .into_iter()
            .map(|(id, _, _)| id)
            .collect();
        assert_eq!(current, replayed, "the table holds the replay");
        let _ = std::fs::remove_dir_all(&path);
    }

    /// Eight creators of one table in one empty folder: one creates it, the
    /// other seven are told the version is taken, and the folder holds the
    /// one document.
    #[test]
    fn racing_creators_make_one_table() {
        const CREATORS: usize = 8;
        let path = root("racing-creators");
        let start = Arc::new(Barrier::new(CREATORS));
        let handles: Vec<_> = (0..CREATORS)
            .map(|_| {
                let path = path.clone();
                let start = Arc::clone(&start);
                std::thread::spawn(move || {
                    let folder = LocalFolder::new(&path).unwrap();
                    start.wait();
                    IcebergTable::create(
                        folder,
                        FormatVersion::V3,
                        trade_schema(),
                        PartitionSpec::unpartitioned(),
                    )
                    .map(|table| table.metadata().unwrap().table_uuid().to_owned())
                })
            })
            .collect();
        let outcomes: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        let created: Vec<_> = outcomes
            .iter()
            .filter_map(|outcome| outcome.as_ref().ok())
            .collect();
        assert_eq!(created.len(), 1, "one creator wins");
        for outcome in &outcomes {
            if let Err(error) = outcome {
                assert!(error.is_conflict(), "a loser is told: {error}");
            }
        }

        let table = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
        assert_eq!(table.metadata_version().unwrap(), 1);
        assert_eq!(table.metadata().unwrap().table_uuid(), created[0].as_str());
        let mut names = std::fs::read_dir(path.join("metadata"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        names.sort();
        assert_eq!(names, ["v1.metadata.json", "version-hint.text"]);
        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn a_beaten_merge_reports_the_conflict_rather_than_rebasing() {
        let path = root("beaten-merge");
        let schema = trade_schema();
        let mut writer = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            // A keyed merge: v2 only, refused on v3.
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
        let mut stale = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
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

        // The row opens with the seventeen event columns, two of them the
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
            schema.get_field_by_path("hashcode").unwrap().dtype(),
            &yggdryl::DataType::decimal128(20, 0).unwrap()
        );
        schema.assign_parquet_field_ids(1).unwrap();

        let catalog = yggdryl::Catalog::from(yggdryl::iceberg::IcebergCatalog::bound(
            "warehouse",
            yggdryl::holder::Holder::LocalFolder(LocalFolder::new(path.join("warehouse")).unwrap()),
        ));
        // A create descends through existing namespaces only.
        catalog
            .namespaces()
            .create("logs", &yggdryl::Properties::new())
            .unwrap();
        catalog
            .tables()
            .create("logs.events", &schema, &yggdryl::Properties::new())
            .unwrap();
        let table = catalog
            .tables()
            .append_arrow_reader("logs.events", source.read_arrow_reader(&options).unwrap())
            .unwrap();

        let batches = yggdryl::IOMedia::read_arrow_reader(
            &table,
            &yggdryl::IOMedia::record_options(&table).unwrap(),
        )
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
        let Some(table) = table.downcast_ref::<yggdryl::iceberg::IcebergTable<yggdryl::Handle>>()
        else {
            panic!("expected an Iceberg table, got {table:?}");
        };
        assert_eq!(
            table
                .current_snapshot()
                .unwrap()
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
            FormatVersion::V3,
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
    fn formats(table: &IcebergTable<LocalFolder>) -> Vec<(MimeType, String)> {
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
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
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
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
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
            let mut table = IcebergTable::create(
                LocalFolder::new(&path).unwrap(),
                FormatVersion::V3,
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
            assert!(table.current_snapshot().unwrap().is_none());
            assert!(!path.join("data").exists());
        }
    }

    #[test]
    fn a_table_whose_files_mix_formats_writes_and_scans_as_one_shape() {
        let path = root("format-mixed");
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
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
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
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
        let reopened = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
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
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
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
/// v3 types: the private half of the contract `docs/media/iceberg.md`
/// states, pinned where the plan, the grouping, and the data-file handles
/// are visible.
mod isolation {
    use yggdryl::StructType;

    use std::sync::{Arc, Mutex};

    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    use arrow_array::{
        Array, ArrayRef, BinaryArray, Int64Array, NullArray, RecordBatch, RecordBatchIterator,
        RecordBatchReader, StringArray, TimestampMicrosecondArray,
    };
    use arrow_schema::{ArrowError, SchemaRef};

    use super::{
        FormatVersion, IcebergOptions, IcebergTable, PartitionSpec, SortField, SortOrder,
        Transform, assign_field_ids, collect, root, schema_from_json, schema_into_json,
        trade_schema, trades,
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
            pwrite, create_bytes, size, capacity, reserve, truncate, uri, url, bound_location, mtime, media_type,
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

    /// One row per venue, one commit per row: three partitions, three files,
    /// in a v3 table.
    fn venues(label: &str) -> (std::path::PathBuf, IcebergTable<LocalFolder>) {
        venues_at(label, FormatVersion::V3)
    }

    /// [`venues`] at `version`, for a test whose writes only that version takes.
    fn venues_at(
        label: &str,
        version: FormatVersion,
    ) -> (std::path::PathBuf, IcebergTable<LocalFolder>) {
        let path = root(label);
        let schema = trade_schema();
        let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
        let mut table =
            IcebergTable::create(LocalFolder::new(&path).unwrap(), version, schema, spec).unwrap();
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

    fn file_paths(table: &IcebergTable<impl IOBase>) -> Vec<String> {
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
        let mut table =
            IcebergTable::create(folder, FormatVersion::V3, schema.clone(), spec).unwrap();
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
        let table = IcebergTable::open(recording).unwrap();
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
        // A keyed merge: v2 only, refused on v3.
        let (path, _) = venues_at("isolation-merge", FormatVersion::V2);
        let mut table = IcebergTable::open(Recording::new(&path)).unwrap();
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
        // A keyed merge: v2 only, refused on v3.
        let (path, mut table) = venues_at("isolation-moved-row", FormatVersion::V2);
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
        let mut flat_table = IcebergTable::create(
            LocalFolder::new(&flat).unwrap(),
            FormatVersion::V3,
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
        assert!(flat_table.current_snapshot().unwrap().is_none());
        let _ = std::fs::remove_dir_all(&path);
        let _ = std::fs::remove_dir_all(&flat);
    }

    #[test]
    fn duplicate_keys_in_one_write_keep_the_last_row() {
        // A keyed merge: v2 only, refused on v3.
        let (path, mut table) = venues_at("isolation-duplicates", FormatVersion::V2);
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
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
            schema.clone(),
            spec,
        )
        .unwrap();
        // The default order is the partition's source column.
        let order = table.metadata().unwrap().default_sort_order().unwrap();
        assert_eq!(order.order_id, 1);
        assert_eq!(order.fields.len(), 1);
        assert_eq!(order.fields[0].source_id, 3);
        assert_eq!(order.fields[0].transform, Transform::Identity);
        assert_eq!(order.fields[0].direction, "asc");
        assert_eq!(order.fields[0].null_order, "nulls-first");
        assert_eq!(table.metadata().unwrap().default_sort_order_id(), 1);

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
        let mut by_symbol = IcebergTable::create_sorted(
            LocalFolder::new(&sorted).unwrap(),
            FormatVersion::V3,
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
        let document = by_symbol.metadata().unwrap().clone().into_json().unwrap();
        let reread = super::TableMetadata::from_json(&document).unwrap();
        assert_eq!(reread.default_sort_order_id(), 1);
        assert_eq!(reread.default_sort_order().unwrap().fields[0].source_id, 2);
        let reopened = IcebergTable::open(LocalFolder::new(&sorted).unwrap()).unwrap();
        assert_eq!(reopened.metadata().unwrap().default_sort_order_id(), 1);
        reopened.metadata().unwrap().validate().unwrap();

        let _ = std::fs::remove_dir_all(&path);
        let _ = std::fs::remove_dir_all(&sorted);
    }

    #[test]
    fn an_explicitly_unsorted_table_keeps_the_order_the_rows_arrived_in() {
        let path = root("isolation-unsorted");
        let schema = trade_schema();
        let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
        let mut table = IcebergTable::create_sorted(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
            schema,
            spec,
            SortOrder::unsorted(),
        )
        .unwrap();
        assert_eq!(table.metadata().unwrap().default_sort_order_id(), 0);
        assert_eq!(table.metadata().unwrap().sort_orders().len(), 1);
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
        let flat_table = IcebergTable::create(
            LocalFolder::new(&flat).unwrap(),
            FormatVersion::V3,
            trade_schema(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        assert_eq!(flat_table.metadata().unwrap().default_sort_order_id(), 0);
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
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
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
            let mut table = IcebergTable::create(
                LocalFolder::new(&path).unwrap(),
                FormatVersion::V3,
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
        IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
            schema,
            spec,
        )
        .unwrap();
        let mut recording = Recording::new(&path);
        // The third partition group's writer gets a root it cannot write through.
        recording.fail_after = Some(2);
        let mut table = IcebergTable::open(recording).unwrap();
        table.set_options(IcebergOptions::new().try_with_write_parallelism(4).unwrap());
        let version = table.metadata_version().unwrap();
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
        assert_eq!(table.metadata_version().unwrap(), version);
        assert!(table.current_snapshot().unwrap().is_none());
        let reopened = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
        assert_eq!(reopened.metadata_version().unwrap(), version);
        assert!(reopened.current_snapshot().unwrap().is_none());
        let _ = std::fs::remove_dir_all(&path);
    }

    /// The write parallelism a host leg runs at: the host's, and never one,
    /// so the threaded path runs on a one-core runner too.
    fn host_parallelism() -> usize {
        IcebergOptions::default_read_parallelism().max(2)
    }

    /// A venue-partitioned table, sorted where `order` says, writing on
    /// `parallelism` threads and cutting files at `target` where one is given.
    fn venue_table(
        label: &str,
        parallelism: usize,
        order: Option<SortOrder>,
        target: Option<u64>,
    ) -> (std::path::PathBuf, IcebergTable<LocalFolder>) {
        let path = root(label);
        let schema = trade_schema();
        let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
        let folder = LocalFolder::new(&path).unwrap();
        let mut table = match order {
            Some(order) => {
                IcebergTable::create_sorted(folder, FormatVersion::V3, schema, spec, order)
            }
            None => IcebergTable::create(folder, FormatVersion::V3, schema, spec),
        }
        .unwrap();
        let mut options = IcebergOptions::new()
            .try_with_write_parallelism(parallelism)
            .unwrap();
        if let Some(target) = target {
            options = options.try_with_target_file_size_bytes(target).unwrap();
        }
        table.set_options(options);
        (path, table)
    }

    /// `batches` batches of `rows` rows whose venue cycles through `venues`
    /// row by row, so every batch spans every venue. A row's id is its
    /// arrival position in the stream, and its symbol cycles through `symbols`
    /// two rows at a time, so sort keys repeat within and across batches.
    fn interleaved(
        venues: &[&str],
        symbols: &[&str],
        batches: usize,
        rows: usize,
    ) -> Vec<RecordBatch> {
        (0..batches)
            .map(|batch| {
                let positions = batch * rows..(batch + 1) * rows;
                let ids: Vec<i64> = positions
                    .clone()
                    .map(|position| i64::try_from(position).unwrap())
                    .collect();
                let symbols: Vec<Option<&str>> = positions
                    .clone()
                    .map(|position| Some(symbols[(position / 2) % symbols.len()]))
                    .collect();
                let venues: Vec<Option<&str>> = positions
                    .map(|position| Some(venues[position % venues.len()]))
                    .collect();
                trades(&ids, &symbols, &venues)
            })
            .collect()
    }

    /// Every row of a reader as `(id, symbol, venue)`, in the order it yields them.
    fn rows_in_order(
        reader: yggdryl::arrow::BatchReader,
    ) -> Vec<(i64, Option<String>, Option<String>)> {
        let mut rows = Vec::new();
        for batch in reader {
            let batch = batch.unwrap();
            let text = |name: &str| {
                batch
                    .column_by_name(name)
                    .unwrap()
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .unwrap()
                    .clone()
            };
            let ids = batch
                .column_by_name("id")
                .unwrap()
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap()
                .clone();
            let (symbols, venues) = (text("symbol"), text("venue"));
            for row in 0..batch.num_rows() {
                let cell = |column: &StringArray| {
                    (!column.is_null(row)).then(|| column.value(row).to_owned())
                };
                rows.push((ids.value(row), cell(&symbols), cell(&venues)));
            }
        }
        rows
    }

    /// The ids of one venue's rows, in the order `rows` holds them.
    fn venue_ids(rows: &[(i64, Option<String>, Option<String>)], venue: &str) -> Vec<i64> {
        rows.iter()
            .filter(|(_, _, value)| value.as_deref() == Some(venue))
            .map(|(id, _, _)| *id)
            .collect()
    }

    /// The manifest's data files in plan order, each as its path below
    /// `data/` with the snapshot id and the uuid masked off the name - so the
    /// partition directory and the file's index are what remains - beside
    /// its record count.
    fn file_layout(table: &IcebergTable<LocalFolder>) -> Vec<(String, i64)> {
        table
            .data_files()
            .unwrap()
            .into_iter()
            .map(|(file, _)| {
                let path = file.file_path.to_string().replace('\\', "/");
                let start = path.rfind("/data/").map_or(0, |slash| slash + 1);
                let below = path[start..].strip_prefix("data/").expect("a data path");
                let (directory, name) = below.rsplit_once('/').unwrap();
                let index = name.split_once('-').expect("an indexed name").0;
                (format!("{directory}/{index}"), file.record_count)
            })
            .collect()
    }

    /// The table's metadata documents, by name.
    fn metadata_documents(path: &std::path::Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(path.join("metadata"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_group_cut_into_files_lists_them_in_one_order_and_numbering_on_any_parallelism() {
        // Three venues interleaved over six batches, sixty rows each, under a
        // target small enough to cut each venue into several files. The
        // cuts are computed from the whole group, so the manifest lists the
        // groups in tuple order - every one open when the source ends - and
        // each group's files by their index from 00000 up, whatever thread
        // wrote which file.
        let venues = ["v3", "v1", "v2"];
        let batches = interleaved(&venues, &["a", "b", "c", "d", "e", "f", "g"], 6, 30);
        let mut legs = Vec::new();
        for parallelism in [1, host_parallelism()] {
            let (path, mut table) = venue_table(
                &format!("isolation-cut-files-{parallelism}"),
                parallelism,
                None,
                Some(128),
            );
            table
                .commit_append(yggdryl::arrow::batch_reader(
                    batches[0].schema(),
                    batches.clone(),
                ))
                .unwrap();
            assert_eq!(table.metadata().unwrap().snapshots().len(), 1);
            let layout = file_layout(&table);
            let mut seen = Vec::new();
            for venue in ["v1", "v2", "v3"] {
                let files: Vec<&(String, i64)> = layout
                    .iter()
                    .filter(|(name, _)| name.starts_with(&format!("venue={venue}/")))
                    .collect();
                assert!(files.len() >= 3, "{venue}: {layout:?}");
                for (index, (name, _)) in files.iter().enumerate() {
                    assert_eq!(*name, format!("venue={venue}/{index:05}"), "{layout:?}");
                }
                assert_eq!(files.iter().map(|(_, rows)| rows).sum::<i64>(), 60);
                seen.extend(files.into_iter().cloned());
            }
            assert_eq!(seen, layout, "group order is the tuples': v1, v2, v3");
            let rows = rows_in_order(table.scan(None).unwrap());
            for (position, venue) in venues.iter().enumerate() {
                let expected: Vec<i64> = (0..180).skip(position).step_by(3).collect();
                assert_eq!(
                    venue_ids(&rows, venue),
                    expected,
                    "{venue} in arrival order"
                );
            }
            legs.push((layout, rows));
            let _ = std::fs::remove_dir_all(&path);
        }
        assert_eq!(legs[0], legs[1]);
    }

    #[test]
    fn a_skewed_commit_lists_its_partitions_in_tuple_order_never_by_size() {
        // Four light venues arrive first, out of their sorted order; the
        // heaviest venue arrives only in the last batch, and sorts first.
        // Every partition is open when the source ends, so the manifest
        // lists them in tuple order: neither a group's size, nor when it
        // arrived, nor which group finishes last moves it.
        let light = trades(
            &[1, 2, 3, 4, 5, 6, 7, 8],
            &[Some("a"); 8],
            &[
                Some("v3"),
                Some("v1"),
                Some("v4"),
                Some("v2"),
                Some("v1"),
                Some("v3"),
                Some("v2"),
                Some("v4"),
            ],
        );
        let heavy_ids: Vec<i64> = (100..4_100).collect();
        let mut heavy_venues = vec![Some("v0"); heavy_ids.len()];
        heavy_venues[0] = Some("v2");
        let heavy = trades(&heavy_ids, &vec![Some("b"); heavy_ids.len()], &heavy_venues);
        for parallelism in [1, host_parallelism()] {
            let (path, mut table) = venue_table(
                &format!("isolation-skewed-{parallelism}"),
                parallelism,
                None,
                None,
            );
            table
                .commit_append(yggdryl::arrow::batch_reader(
                    light.schema(),
                    [light.clone(), light.slice(0, 0), heavy.clone()],
                ))
                .unwrap();
            let layout: Vec<(String, i64)> = file_layout(&table);
            assert_eq!(
                layout,
                [
                    ("venue=v0/00000".to_owned(), 3_999),
                    ("venue=v1/00000".to_owned(), 2),
                    ("venue=v2/00000".to_owned(), 3),
                    ("venue=v3/00000".to_owned(), 2),
                    ("venue=v4/00000".to_owned(), 2),
                ],
                "parallelism {parallelism}"
            );
            let _ = std::fs::remove_dir_all(&path);
        }
    }

    #[test]
    fn a_sorted_table_sorts_each_group_stably_across_the_batches_of_one_commit() {
        // Sort keys repeat within and across eight batches, every batch
        // spanning both venues; ids are arrival positions. Each partition
        // reads back ordered by symbol, and rows of one symbol keep the
        // order they arrived in.
        let order = SortOrder {
            order_id: 1,
            fields: vec![SortField {
                source_id: 2,
                transform: Transform::Identity,
                direction: "asc".into(),
                null_order: "nulls-last".into(),
            }],
        };
        let venues = ["v2", "v1"];
        let batches = interleaved(&venues, &["d", "b", "c", "a"], 8, 12);
        let arrived = rows_in_order(yggdryl::arrow::batch_reader(
            batches[0].schema(),
            batches.clone(),
        ));
        let mut legs = Vec::new();
        for parallelism in [1, host_parallelism()] {
            let (path, mut table) = venue_table(
                &format!("isolation-stable-sort-{parallelism}"),
                parallelism,
                Some(order.clone()),
                None,
            );
            table
                .commit_append(yggdryl::arrow::batch_reader(
                    batches[0].schema(),
                    batches.clone(),
                ))
                .unwrap();
            let scanned = rows_in_order(table.scan(None).unwrap());
            let options = table.record_options().unwrap();
            let read = rows_in_order(table.read_arrow_reader(&options).unwrap());
            for venue in venues {
                let mut expected: Vec<(i64, Option<String>, Option<String>)> = arrived
                    .iter()
                    .filter(|(_, _, value)| value.as_deref() == Some(venue))
                    .cloned()
                    .collect();
                expected.sort_by(|left, right| left.1.cmp(&right.1));
                let ids: Vec<i64> = expected.iter().map(|(id, _, _)| *id).collect();
                assert_eq!(venue_ids(&scanned, venue), ids, "{venue}, scanned");
                assert_eq!(venue_ids(&read, venue), ids, "{venue}, read");
            }
            assert_eq!(scanned.len(), 96);
            legs.push(scanned);
            let _ = std::fs::remove_dir_all(&path);
        }
        assert_eq!(legs[0], legs[1]);
    }

    #[test]
    fn an_unsorted_table_keeps_each_partitions_rows_in_arrival_order_across_many_batches() {
        // Sixteen batches of nine rows, each spanning three venues; with no
        // sort order a partition's file holds its rows as they arrived.
        let venues = ["v2", "v3", "v1"];
        let batches = interleaved(&venues, &["z", "y"], 16, 9);
        let mut legs = Vec::new();
        for parallelism in [1, host_parallelism()] {
            let (path, mut table) = venue_table(
                &format!("isolation-arrival-order-{parallelism}"),
                parallelism,
                None,
                None,
            );
            table
                .commit_append(yggdryl::arrow::batch_reader(
                    batches[0].schema(),
                    batches.clone(),
                ))
                .unwrap();
            let scanned = rows_in_order(table.scan(None).unwrap());
            let options = table.record_options().unwrap();
            let read = rows_in_order(table.read_arrow_reader(&options).unwrap());
            for (position, venue) in venues.iter().enumerate() {
                let expected: Vec<i64> = (0..144).skip(position).step_by(3).collect();
                assert_eq!(venue_ids(&scanned, venue), expected, "{venue}, scanned");
                assert_eq!(venue_ids(&read, venue), expected, "{venue}, read");
            }
            legs.push(scanned);
            let _ = std::fs::remove_dir_all(&path);
        }
        assert_eq!(legs[0], legs[1]);
    }

    #[test]
    fn a_source_failing_mid_stream_fails_the_commit_with_that_batchs_error_and_writes_no_metadata()
    {
        // The source fails at its third batch and again at its fourth; the
        // commit answers the first failure in stream order, whatever was
        // pulled ahead of it, and leaves the table as it was.
        let batches = interleaved(&["v1", "v2", "v3"], &["a"], 5, 6);
        let failing = || -> yggdryl::arrow::BatchReader {
            Box::new(RecordBatchIterator::new(
                [
                    Ok(batches[0].clone()),
                    Ok(batches[1].clone()),
                    Err(ArrowError::ComputeError("source failure at batch 2".into())),
                    Err(ArrowError::ComputeError("source failure at batch 3".into())),
                    Ok(batches[4].clone()),
                ],
                batches[0].schema(),
            ))
        };
        for parallelism in [1, host_parallelism()] {
            let (path, mut table) = venue_table(
                &format!("isolation-source-failure-{parallelism}"),
                parallelism,
                None,
                None,
            );
            let seed = trades(&[900], &[Some("SEED")], &[Some("v1")]);
            table
                .commit_append(yggdryl::arrow::batch_reader(seed.schema(), [seed]))
                .unwrap();
            let version = table.metadata_version().unwrap();
            let files = table.data_files().unwrap();
            let documents = metadata_documents(&path);

            let options = table
                .record_options()
                .unwrap()
                .with_field(trade_schema())
                .with_num_threads(parallelism);
            let errors = [
                table.commit_append(failing()).unwrap_err().to_string(),
                table
                    .append_arrow_reader(failing(), &options)
                    .unwrap_err()
                    .to_string(),
            ];
            for error in errors {
                assert!(error.contains("source failure at batch 2"), "{error}");
                assert!(!error.contains("batch 3"), "{error}");
            }
            for table in [
                table,
                IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap(),
            ] {
                assert_eq!(table.metadata_version().unwrap(), version);
                assert_eq!(table.metadata().unwrap().snapshots().len(), 1);
                assert_eq!(table.data_files().unwrap(), files);
                assert_eq!(
                    rows_in_order(table.scan(None).unwrap()),
                    [(900, Some("SEED".to_owned()), Some("v1".to_owned()))]
                );
            }
            assert_eq!(metadata_documents(&path), documents);
            let _ = std::fs::remove_dir_all(&path);
        }
    }

    /// A source of many batches that counts each one pulled from it.
    struct Pulled {
        schema: SchemaRef,
        batches: std::vec::IntoIter<RecordBatch>,
        pulls: Arc<AtomicUsize>,
    }

    impl Iterator for Pulled {
        type Item = std::result::Result<RecordBatch, ArrowError>;

        fn next(&mut self) -> Option<Self::Item> {
            let batch = self.batches.next()?;
            self.pulls.fetch_add(1, AtomicOrdering::SeqCst);
            Some(Ok(batch))
        }
    }

    impl RecordBatchReader for Pulled {
        fn schema(&self) -> SchemaRef {
            Arc::clone(&self.schema)
        }
    }

    #[test]
    fn zero_threads_is_refused_at_every_table_write_door_before_a_batch_is_pulled() {
        let (path, mut table) = venue_table("isolation-zero-threads-pulls", 1, None, None);
        let batches = interleaved(&["v1", "v2", "v3"], &["a", "b"], 64, 256);
        let options = table
            .record_options()
            .unwrap()
            .with_field(trade_schema())
            .with_num_threads(0);
        let pulls = Arc::new(AtomicUsize::new(0));
        let source = || -> yggdryl::arrow::BatchReader {
            Box::new(Pulled {
                schema: batches[0].schema(),
                batches: batches.clone().into_iter(),
                pulls: Arc::clone(&pulls),
            })
        };
        let keyed = options.clone().with_merge_by(["id"]).unwrap();
        let errors = [
            table.append_arrow_reader(source(), &options).unwrap_err(),
            table
                .overwrite_arrow_reader(source(), &options)
                .unwrap_err(),
            table.merge_arrow_reader(source(), &keyed).unwrap_err(),
        ];
        for error in errors {
            let error = error.to_string();
            assert!(error.contains("$.num_threads"), "{error}");
        }
        assert_eq!(pulls.load(AtomicOrdering::SeqCst), 0);
        assert!(table.current_snapshot().unwrap().is_none());
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
    fn unknown_is_a_declared_variant_in_both_directions_and_only_a_v3_table_carries_it() {
        // Schema document -> field: `unknown` is a variant its field declares
        // `unknown`, optional, at every depth.
        let document: Scalar = yggdryl::json::from_utf8(
            r#"{"type":"struct","schema-id":0,"fields":[
                {"id":1,"name":"id","required":true,"type":"long"},
                {"id":2,"name":"later","required":false,"type":"unknown"},
                {"id":3,"name":"tags","required":false,"type":{"type":"list","element-id":4,"element":"unknown","element-required":false}}
            ]}"#,
        )
        .unwrap();
        let schema = schema_from_json("row", &document).unwrap();
        assert_eq!(schema.fields()[1].dtype(), &DataType::Variant);
        assert!(schema.fields()[1].as_iceberg().is_unknown());
        assert!(schema.fields()[1].is_nullable());
        let element = schema.fields()[2].get_field_at(0).unwrap();
        assert_eq!(element.dtype(), &DataType::Variant);
        assert!(element.as_iceberg().is_unknown());
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
        let message = IcebergTable::create(
            LocalFolder::new(&v2).unwrap(),
            // v2 refuses the v3 type: the contract pinned here.
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
        let mut table = IcebergTable::create(
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

        // The null column it was written as reads back as nulls, typed as
        // the schema says - the variant...
        let mut reader = table.scan(None).unwrap();
        assert_eq!(
            reader.schema().field(1).data_type(),
            table
                .schema()
                .unwrap()
                .clone()
                .into_arrow_schema()
                .unwrap()
                .field(1)
                .data_type()
        );
        assert!(matches!(
            reader.schema().field(1).data_type(),
            arrow_schema::DataType::Struct(_)
        ));
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
        let reopened = IcebergTable::open(LocalFolder::new(&v3).unwrap()).unwrap();
        assert_eq!(
            reopened.schema().unwrap().fields()[1].dtype(),
            &DataType::Variant
        );
        assert!(
            reopened.schema().unwrap().fields()[1]
                .as_iceberg()
                .is_unknown()
        );
        reopened.metadata().unwrap().validate().unwrap();
        let text =
            yggdryl::json::into_utf8(&reopened.metadata().unwrap().clone().into_json().unwrap())
                .unwrap();
        assert!(text.contains("\"unknown\""), "{text}");

        let _ = std::fs::remove_dir_all(&v2);
        let _ = std::fs::remove_dir_all(&v3);
    }

    #[test]
    fn an_unknown_column_promotes_to_any_type_in_v3() {
        let v3 = root("isolation-unknown-promotion");
        let mut table = IcebergTable::create(
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

    #[test]
    fn a_nested_unknown_slot_stores_the_null_column_and_promotes_over_old_files() {
        let document: Scalar = yggdryl::json::from_utf8(
            r#"{"type":"struct","schema-id":0,"fields":[
                {"id":1,"name":"id","required":true,"type":"long"},
                {"id":2,"name":"s","required":false,"type":{"type":"struct","fields":[
                    {"id":3,"name":"a","required":false,"type":"long"},
                    {"id":4,"name":"later","required":false,"type":"unknown"}
                ]}},
                {"id":5,"name":"tags","required":false,"type":{"type":"list","element-id":6,"element":"unknown","element-required":false}}
            ]}"#,
        )
        .unwrap();
        let schema = schema_from_json("row", &document).unwrap();
        let v3 = root("isolation-nested-unknown");
        let mut table = IcebergTable::create(
            LocalFolder::new(&v3).unwrap(),
            FormatVersion::V3,
            schema.clone(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let rows = |text: &str| {
            let rows: Scalar = yggdryl::json::from_utf8(text).unwrap();
            yggdryl::Serie::from_scalars(
                schema.clone(),
                rows.iter().map(std::borrow::Cow::into_owned),
            )
            .unwrap()
            .into_arrow_batch()
            .unwrap()
        };
        let nulls = rows(
            r#"[{"id":1,"s":{"a":5,"later":null},"tags":[null,null]},
                {"id":2,"s":null,"tags":null}]"#,
        );
        table
            .commit_append(yggdryl::arrow::batch_reader(nulls.schema(), [nulls]))
            .unwrap();

        // The data file keeps each slot its parent's layout has, as Arrow's
        // null column - never a variant group.
        let (file, _) = table.data_files().unwrap().remove(0);
        let handle = child_at(&table, &file.file_path).unwrap();
        let stored =
            yggdryl::parquet::read_field(&handle, &yggdryl::parquet::ParquetOptions::new())
                .unwrap();
        assert_eq!(
            stored
                .get_field("s")
                .unwrap()
                .get_field("later")
                .unwrap()
                .dtype(),
            &DataType::Null
        );
        assert_eq!(
            stored
                .get_field("tags")
                .unwrap()
                .get_field_at(0)
                .unwrap()
                .dtype(),
            &DataType::Null
        );
        let read: usize = table
            .scan(None)
            .unwrap()
            .map(|batch| batch.unwrap().num_rows())
            .sum();
        assert_eq!(read, 2);

        // A value in a nested slot is refused by its path.
        for (text, path) in [
            (r#"[{"id":3,"s":{"a":1,"later":7},"tags":null}]"#, "s.later"),
            (
                r#"[{"id":3,"s":null,"tags":[null,"seven"]}]"#,
                "tags.element",
            ),
        ] {
            let batch = rows(text);
            let message = table
                .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
                .unwrap_err()
                .to_string();
            assert!(
                message.contains(path) && message.contains("unknown"),
                "{message}"
            );
        }

        // Promoted, the slot reads the old file's nulls as its new type.
        let mut update =
            yggdryl::iceberg::SchemaUpdate::from_metadata(table.metadata().unwrap()).unwrap();
        update.update_type("s.later", DataType::Int64);
        table.update_schema(&update).unwrap();
        let mut promoted = 0;
        for batch in table.scan(None).unwrap() {
            let batch = batch.unwrap();
            let s = batch
                .column_by_name("s")
                .unwrap()
                .as_any()
                .downcast_ref::<arrow_array::StructArray>()
                .unwrap();
            let later = s.column_by_name("later").unwrap();
            assert_eq!(later.data_type(), &arrow_schema::DataType::Int64);
            assert_eq!(later.logical_null_count(), batch.num_rows());
            promoted += batch.num_rows();
        }
        assert_eq!(promoted, 2);
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
        // `unknown` reads as the same datatype: the absence of a type is told
        // apart by its field's declaration, not by a datatype of its own.
        assert_eq!(
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
        let message = IcebergTable::create(
            LocalFolder::new(&v2).unwrap(),
            // v2 refuses the v3 type: the contract pinned here.
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
        let mut table = IcebergTable::create(
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
        let reopened = IcebergTable::open(LocalFolder::new(&v3).unwrap()).unwrap();
        assert_eq!(
            reopened.schema().unwrap().fields()[1].dtype(),
            &DataType::Variant
        );
        reopened.metadata().unwrap().validate().unwrap();

        let _ = std::fs::remove_dir_all(&v2);
        let _ = std::fs::remove_dir_all(&v3);
    }

    #[test]
    fn the_record_surface_keys_a_merge_by_the_partition_and_the_options_key() {
        // A keyed merge: v2 only, refused on v3.
        let (path, _) = venues_at("isolation-record-merge", FormatVersion::V2);
        let mut table = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
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

    use super::{
        FormatVersion, IcebergOptions, IcebergTable, PartitionSpec, root, trade_schema, trades,
    };
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
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
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
        let version = table.metadata_version().unwrap();

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
        assert_eq!(table.metadata_version().unwrap(), version);
        assert!(table.current_snapshot().unwrap().is_none());
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
        let reopened = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
        assert_eq!(reopened.metadata_version().unwrap(), version);
        assert!(reopened.current_snapshot().unwrap().is_none());
        let _ = std::fs::remove_dir_all(&base);
        let _ = std::fs::remove_dir_all(&path);
    }
}

/// A table partitioned by one of this crate's own transforms: written by
/// `minutes[15]` and `minutes[30]`, read back through the metadata and the
/// manifests a reopened table reads, and pruned by the periods its files and
/// manifests name.
mod time_partitions {
    use std::sync::Arc;

    use arrow_array::{ArrayRef, Int64Array, RecordBatch, TimestampMicrosecondArray};

    use super::{
        FormatVersion, IcebergTable, LocalFolder, PartitionField, PartitionSpec, Transform,
        assign_field_ids, root,
    };
    use yggdryl::{DataType, Field, Scalar, StructType, TimeUnit, Timezone};

    fn schema() -> Field {
        let mut schema = StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::DateTime64 {
                unit: TimeUnit::Microsecond,
                timezone: Timezone::NAIVE,
            }
            .required_field("ts"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
        assign_field_ids(&mut schema, 1).unwrap();
        schema
    }

    fn batch(schema: &Field, ids: &[i64], seconds: &[i64]) -> RecordBatch {
        let arrow = schema.clone().into_arrow_schema().unwrap();
        RecordBatch::try_new(
            arrow,
            vec![
                Arc::new(Int64Array::from(ids.to_vec())) as ArrayRef,
                Arc::new(TimestampMicrosecondArray::from(
                    seconds
                        .iter()
                        .map(|second| second * 1_000_000)
                        .collect::<Vec<_>>(),
                )) as ArrayRef,
            ],
        )
        .unwrap()
    }

    fn rows(reader: yggdryl::arrow::BatchReader) -> usize {
        reader.map(|batch| batch.unwrap().num_rows()).sum()
    }

    /// A table at `path` partitioned by `minutes[step]` of `ts`, in a field
    /// named `ts_minutes`.
    fn minutes_table(
        path: &std::path::Path,
        schema: &Field,
        step: u32,
    ) -> IcebergTable<LocalFolder> {
        let spec = PartitionSpec {
            spec_id: 0,
            fields: vec![PartitionField {
                source_id: 2,
                field_id: 1000,
                name: "ts_minutes".into(),
                transform: Transform::Minutes(step),
            }],
        };
        IcebergTable::create(
            LocalFolder::new(path).unwrap(),
            FormatVersion::V3,
            schema.clone(),
            spec,
        )
        .unwrap()
    }

    /// Every metadata document the table wrote, as the text on disk.
    fn metadata_documents(path: &std::path::Path) -> Vec<String> {
        let documents: Vec<String> = std::fs::read_dir(path.join("metadata"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|file| file.to_string_lossy().ends_with(".metadata.json"))
            .map(|file| std::fs::read_to_string(file).unwrap())
            .collect();
        assert!(!documents.is_empty(), "the table wrote its metadata");
        documents
    }

    /// The on-disk files of a reopened table: partition tuple, row count,
    /// path, sorted.
    fn data_files(
        table: &IcebergTable<LocalFolder>,
        transform: Transform,
    ) -> Vec<(Vec<Scalar>, i64, String)> {
        let mut files: Vec<(Vec<Scalar>, i64, String)> = table
            .data_files()
            .unwrap()
            .into_iter()
            .map(|(file, spec)| {
                assert_eq!(spec.fields[0].transform, transform);
                (
                    file.partition,
                    file.record_count,
                    file.file_path.to_string(),
                )
            })
            .collect();
        files.sort();
        files
    }

    #[test]
    fn a_table_partitioned_by_minutes_15_writes_one_file_per_period_and_reads_back() {
        let path = root("minutes-15-partitions");
        let schema = schema();
        let mut table = minutes_table(&path, &schema, 15);

        // Five instants over three quarter hours of 1970-01-01: 00:00,
        // 00:14:59, 00:15, 00:30 and 00:44:59.
        let first = batch(&schema, &[1, 2, 3, 4, 5], &[0, 899, 900, 1800, 2699]);
        table
            .commit_append(yggdryl::arrow::batch_reader(first.schema(), [first]))
            .unwrap();
        // One more commit an hour in, so the manifest list has a second
        // summary to prune.
        let second = batch(&schema, &[6], &[3600]);
        table
            .commit_append(yggdryl::arrow::batch_reader(second.schema(), [second]))
            .unwrap();

        // The document on disk spells the transform as the crate does; the
        // reserved bucket the official model reads it as never lands there.
        for text in metadata_documents(&path) {
            assert!(text.contains("\"minutes[15]\""), "{text}");
            assert!(!text.contains("\"bucket[21474"), "{text}");
        }

        let reopened = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
        let files = data_files(&reopened, Transform::Minutes(15));
        assert_eq!(files.len(), 4, "one data file per quarter hour: {files:?}");
        for ((partition, count, file_path), (expected, expected_count)) in
            files.iter().zip([(0, 2), (1, 1), (2, 2), (4, 1)])
        {
            assert_eq!(partition, &vec![Scalar::from(expected)]);
            assert_eq!(*count, expected_count, "{file_path}");
            assert!(
                file_path.contains(&format!("ts_minutes={expected}/")),
                "{file_path}"
            );
        }
        assert_eq!(rows(reopened.scan(None).unwrap()), 6);

        // The second manifest holds only quarter hour 4, so its
        // manifest-list summary keeps a predicate before 00:15 from opening
        // it; of the first manifest's three files, two are skipped by the
        // `ts` bounds this writer records on every file - the period alone
        // pruning a file without column statistics is pinned in
        // `rust/tests/iceberg/scan.rs`.
        let early = reopened
            .plan_matching("ts < '1970-01-01T00:15:00'")
            .unwrap();
        assert_eq!(early.tasks.len(), 1);
        assert_eq!(early.manifests_skipped(), 1);
        assert_eq!(early.manifests_read, 1);
        assert_eq!(early.files_skipped(), 2);
        assert_eq!(early.record_count().unwrap(), 2);
        // The file's own `ts` bounds put every row before 00:15, so no
        // residual is left for its rows.
        assert!(early.tasks[0].residual.is_empty());

        // A range reaching into the third quarter hour keeps two files of
        // the first manifest and the whole second one.
        let late = reopened
            .plan_matching("ts >= '1970-01-01T00:30:00'")
            .unwrap();
        assert_eq!(late.tasks.len(), 2);
        assert_eq!(late.manifests_skipped(), 0);
        assert_eq!(late.files_skipped(), 2);
        assert_eq!(
            rows(
                reopened
                    .scan_matching("ts >= '1970-01-01T00:30:00'", None)
                    .unwrap()
            ),
            3
        );
        // An instant between two periods matches nothing, and says so
        // before a data file is opened.
        let none = reopened
            .plan_matching("ts >= '1970-01-01T00:45:00' and ts < '1970-01-01T01:00:00'")
            .unwrap();
        assert_eq!(none.tasks.len(), 0);
        assert_eq!(none.manifests_skipped(), 2);
        assert_eq!(none.files_skipped(), 0);
        assert_eq!(
            rows(
                reopened
                    .scan_matching("ts = '1970-01-01T00:14:59'", None)
                    .unwrap()
            ),
            1
        );
        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn a_table_partitioned_by_minutes_30_writes_one_file_per_half_hour() {
        let path = root("minutes-30-partitions");
        let schema = schema();
        let mut table = minutes_table(&path, &schema, 30);
        // 00:00, 00:14:59 and 00:15 are the first half hour, 00:30 and
        // 00:44:59 the second, 01:00 the third.
        let rows_in = batch(
            &schema,
            &[1, 2, 3, 4, 5, 6],
            &[0, 899, 900, 1800, 2699, 3600],
        );
        table
            .commit_append(yggdryl::arrow::batch_reader(rows_in.schema(), [rows_in]))
            .unwrap();
        for text in metadata_documents(&path) {
            assert!(text.contains("\"minutes[30]\""), "{text}");
            assert!(!text.contains("\"bucket[21474"), "{text}");
        }

        let reopened = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
        let files = data_files(&reopened, Transform::Minutes(30));
        assert_eq!(files.len(), 3, "one data file per half hour: {files:?}");
        for ((partition, count, file_path), (expected, expected_count)) in
            files.iter().zip([(0, 3), (1, 2), (2, 1)])
        {
            assert_eq!(partition, &vec![Scalar::from(expected)]);
            assert_eq!(*count, expected_count, "{file_path}");
            assert!(
                file_path.contains(&format!("ts_minutes={expected}/")),
                "{file_path}"
            );
        }
        let early = reopened
            .plan_matching("ts < '1970-01-01T00:30:00'")
            .unwrap();
        assert_eq!(early.tasks.len(), 1);
        assert_eq!(early.files_skipped(), 2);
        assert_eq!(early.record_count().unwrap(), 3);
        assert_eq!(rows(reopened.scan(None).unwrap()), 6);
        let _ = std::fs::remove_dir_all(&path);
    }
}

mod declared_schema {
    //! A table created from what its schema declares: `PARTITION:by` as
    //! its spec, `SORT:by` as its default order, both reported back on
    //! `IcebergTable::schema()` and after a reopen.

    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray, TimestampMicrosecondArray};

    use super::{FormatVersion, IcebergTable, LocalFolder, PartitionSpec, Transform, root};
    use yggdryl::iceberg::IcebergOptions;
    use yggdryl::{DataType, Field, StructType, TimeUnit, Timezone};

    /// The rows with both declarations on the root and nothing materialized:
    /// an Iceberg table keeps a derived partition's value in its manifest,
    /// so the declaration is written on the root rather than through
    /// `with_partition_by`, which would add the column to the rows.
    fn declared() -> Field {
        let mut rows = DataType::from(
            StructType::from_fields([
                DataType::Int64.required_field("id"),
                DataType::utf8().required_field("venue"),
                DataType::DateTime64 {
                    unit: TimeUnit::Microsecond,
                    timezone: Timezone::NAIVE,
                }
                .required_field("ts"),
            ])
            .unwrap(),
        )
        .required_field("row");
        rows.as_partition_mut()
            .set_by_texts(["venue", "minutes(ts, 15)"])
            .unwrap();
        rows.as_sort_mut()
            .set_by_texts(["ts desc nulls first", "id"])
            .unwrap();
        rows
    }

    fn batch(ids: &[i64], venues: &[&str], seconds: &[i64]) -> RecordBatch {
        RecordBatch::try_from_iter([
            (
                "id",
                Arc::new(Int64Array::from(ids.to_vec())) as arrow_array::ArrayRef,
            ),
            ("venue", Arc::new(StringArray::from(venues.to_vec()))),
            (
                "ts",
                Arc::new(TimestampMicrosecondArray::from(
                    seconds.iter().map(|s| s * 1_000_000).collect::<Vec<_>>(),
                )),
            ),
        ])
        .unwrap()
    }

    #[test]
    fn a_table_is_created_from_the_declarations_and_reports_them_back() {
        let path = root("declared-schema");
        let schema = declared();
        // The schema is numbered by the create, so the spec is read off it
        // there too: nothing is declared twice.
        let mut numbered = schema.clone();
        yggdryl::iceberg::assign_field_ids(&mut numbered, 1).unwrap();
        let spec = PartitionSpec::from_schema(1, &numbered).unwrap();
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
            schema.clone(),
            spec.clone(),
        )
        .unwrap();
        assert_eq!(spec.fields[1].transform, Transform::Minutes(15));
        assert_eq!(spec.fields[1].name, "ts_minutes");
        // A derived partition column is the manifest's, not the schema's.
        assert_eq!(table.schema().unwrap().field_len(), 3);

        let order = table.metadata().unwrap().default_sort_order().unwrap();
        assert_eq!(order.fields.len(), 2);
        assert_eq!(order.fields[0].source_id, 3);
        assert_eq!(order.fields[0].direction, "desc");
        assert_eq!(order.fields[0].null_order, "nulls-first");
        assert_eq!(order.fields[1].source_id, 1);
        assert_eq!(order.fields[1].direction, "asc");

        let reported = table.schema().unwrap();
        assert_eq!(
            reported.get_metadata("PARTITION:by"),
            Some(r#"["venue","minutes(ts, 15)"]"#)
        );
        assert_eq!(
            reported.get_metadata("SORT:by"),
            Some(r#"["ts desc nulls first","id"]"#)
        );
        assert_eq!(
            reported.partition_field_names().collect::<Vec<_>>(),
            ["venue"]
        );

        // Rows land in one file per venue and quarter hour, sorted by the
        // declared order.
        table.set_options(
            IcebergOptions::new()
                .try_with_target_file_size_bytes(1)
                .unwrap(),
        );
        let rows = batch(
            &[1, 2, 3, 4],
            &["X", "X", "X", "Y"],
            &[100, 1_000, 950, 100],
        );
        table
            .commit_append(yggdryl::arrow::batch_reader(rows.schema(), [rows]))
            .unwrap();
        let mut directories: Vec<String> = table
            .data_files()
            .unwrap()
            .iter()
            .map(|(file, _)| {
                let path = file.file_path.as_str();
                let start = path.find("data/").unwrap() + 5;
                path[start..].rsplit_once('/').unwrap().0.to_owned()
            })
            .collect();
        directories.sort();
        directories.dedup();
        assert_eq!(
            directories,
            [
                "venue=X/ts_minutes=0",
                "venue=X/ts_minutes=1",
                "venue=Y/ts_minutes=0"
            ]
        );
        let ids: Vec<i64> = table
            .scan(None)
            .unwrap()
            .flat_map(|batch| {
                batch
                    .unwrap()
                    .column_by_name("id")
                    .unwrap()
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .unwrap()
                    .values()
                    .to_vec()
            })
            .collect();
        assert_eq!(ids.len(), 4);

        // Reopening reads both declarations off the document.
        let reopened = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
        let reported = reopened.schema().unwrap();
        assert_eq!(
            reported.get_metadata("PARTITION:by"),
            Some(r#"["venue","minutes(ts, 15)"]"#)
        );
        assert_eq!(
            reported.get_metadata("SORT:by"),
            Some(r#"["ts desc nulls first","id"]"#)
        );
        assert_eq!(
            PartitionSpec::from_schema(1, reported).unwrap(),
            *reopened.metadata().unwrap().default_spec().unwrap()
        );
        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn a_group_already_in_the_declared_order_is_written_as_it_arrived() {
        let path = root("declared-sorted");
        let mut schema = declared();
        schema.as_sort_mut().set_by_texts(["id"]).unwrap();
        let mut numbered = schema.clone();
        yggdryl::iceberg::assign_field_ids(&mut numbered, 1).unwrap();
        let spec = PartitionSpec::from_schema(1, &numbered).unwrap();
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
            schema,
            spec,
        )
        .unwrap();
        for ids in [[1_i64, 2, 3], [3, 1, 2]] {
            let rows = batch(&ids, &["X"; 3], &[100; 3]);
            table
                .commit_append(yggdryl::arrow::batch_reader(rows.schema(), [rows]))
                .unwrap();
        }
        // Both commits read back in the declared order, whether the rows
        // arrived sorted and were kept, or arrived unsorted and were sorted.
        let ids: Vec<i64> = table
            .scan(None)
            .unwrap()
            .flat_map(|batch| {
                batch
                    .unwrap()
                    .column_by_name("id")
                    .unwrap()
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .unwrap()
                    .values()
                    .to_vec()
            })
            .collect();
        assert_eq!(ids, [1, 2, 3, 1, 2, 3]);
        for (file, _) in table.data_files().unwrap() {
            assert_eq!(file.sort_order_id, Some(1));
        }
        let _ = std::fs::remove_dir_all(&path);
    }
}
