//! The Iceberg table format: planning, metadata, manifests, partition text.
//!
//! Every case drives one question. Planning is measured against real tables on
//! local storage because a plan *is* reads - a manifest list plus one Avro
//! manifest per commit - so its cost is the number of files metadata lets it
//! skip. The metadata and manifest decoders are measured over synthesized
//! documents big enough that per-snapshot and per-entry work dominates. The
//! partition renderer is measured alone because both a table write and a
//! folder write go through it for every directory name they spell.

use std::hint::black_box;
use std::path::PathBuf;
use std::sync::Arc;

use arrow_array::{Float64Array, Int64Array, RecordBatch, StringArray};
use criterion::{BatchSize, Criterion, Throughput};
use smol_str::SmolStr;
use yggdryl::IOBase;
use yggdryl::holder::Buffer;
use yggdryl::holder::local::Folder;
use yggdryl::media::iceberg::{
    CommitConflict, Compaction, DataFile, FieldSummary, FormatVersion, IcebergOptions,
    ManifestContent, ManifestEntry, ManifestFile, PartitionSpec, ScanPlan, ScanTask, Snapshot,
    SnapshotRef, SortField, SortOrder, Table, TableMetadata, Transform, assign_field_ids,
    read_manifest, read_manifest_for_plan, read_manifest_spec, write_manifest,
};
use yggdryl::media::partition::partition_text;
use yggdryl::{DataType, Field, MediaType, MimeType, Scalar};

use crate::bench_profile;

/// The in-process S3 the object backend's own suites run on, shared with the
/// `holder` benchmark: one fixture, so the counts printed here are the counts
/// pinned in `holder::object::tests::accounting`.
#[cfg(feature = "object")]
#[path = "../../src/holder/object/tests/server.rs"]
mod server;

/// Distinct venue values the planning tables partition on.
const VENUES: usize = 8;

/// Workload dimensions; debug builds smoke-test the same paths at lower cost.
const PLAN_LARGE_FILES: usize = bench_profile::corpus(200, 20);
const COMPACT_FILES: usize = bench_profile::corpus(200, 16);
const MERGE_FILES: usize = bench_profile::corpus(50, 16);
const MANIFEST_BASE: usize = bench_profile::corpus(1_000, 100);
const MANIFEST_SCALES: [usize; 3] = [
    bench_profile::corpus(1_000, 100),
    bench_profile::corpus(10_000, 500),
    bench_profile::corpus(100_000, 1_000),
];
const READ_FILES: usize = 32;
const READ_ROWS_PER_FILE: usize = bench_profile::corpus(100_000, 512);
const READ_ROWS: usize = READ_FILES * READ_ROWS_PER_FILE;
/// Partitions of the isolated-merge table; the measured merge touches one.
const MERGE_PARTITIONS: usize = bench_profile::corpus(64, 8);
/// Partitions one parallel commit lays out, and the rows each one holds.
const COMMIT_PARTITIONS: usize = bench_profile::corpus(32, 8);
const COMMIT_ROWS_PER_PARTITION: usize = bench_profile::corpus(2_000, 50);

/// The filter the pruned plan asks for: one of the eight venue values.
const PRUNED_FILTER: (&str, &str) = ("venue", "venue-2");

/// The scratch labels the benchmark tables live under, cleaned at exit.
const SCRATCH_LABELS: [&str; 8] = [
    "files-10",
    "files-200",
    "compact-200",
    "merge-50",
    "read-parallel-32",
    "commit-contended",
    "merge-partitions",
    "commit-parallel",
];

/// Spell one of the [`VENUES`] partition values.
fn venue(index: usize) -> String {
    format!("venue-{index}")
}

/// Build a scratch directory unique to this benchmark run.
fn scratch(label: &str) -> PathBuf {
    let mut path = Folder::temporary()
        .expect("the temporary directory")
        .path()
        .expect("a platform path");
    path.push(format!(
        "yggdryl-bench-iceberg-{label}-{}",
        std::process::id()
    ));
    path
}

/// The two-column schema every planning table writes: an id and its venue.
fn plan_schema() -> Field {
    let mut schema = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("venue"),
    ])
    .expect("the static columns are unique")
    .required_field("row");
    assign_field_ids(&mut schema, 1).expect("the static schema takes identifiers");
    schema
}

/// Build a venue-partitioned table holding exactly `files` data files.
///
/// Each append is one four-row batch spanning two adjacent venues, so one
/// commit writes two data files under one manifest whose summary spans both
/// values. That is what gives the pruned plan work at every level: a filter on
/// one venue skips the manifests whose summaries exclude it outright, and in
/// every manifest it does open, the other venue's file survives to be excluded
/// by its partition tuple - so `files_skipped` cannot be zero.
fn plan_table(label: &str, files: usize) -> Table<Folder> {
    assert!(files % 2 == 0, "expected an even file count, got {files}");
    let path = scratch(label);
    let _ = std::fs::remove_dir_all(&path);
    let schema = plan_schema();
    let spec = PartitionSpec::identity(1, &schema, &["venue"]).expect("venue is a schema column");
    let mut table = Table::create(
        Folder::new(&path).expect("the scratch directory is addressable"),
        FormatVersion::V2,
        schema.clone(),
        spec,
    )
    .expect("the scratch table creates");
    let arrow = schema
        .clone()
        .into_arrow_schema()
        .expect("the schema projects to Arrow");
    for index in 0..files / 2 {
        let commit = i64::try_from(index).expect("the commit index fits an id");
        let first = venue(2 * (index % (VENUES / 2)));
        let second = venue(2 * (index % (VENUES / 2)) + 1);
        let batch = RecordBatch::try_new(
            arrow.clone(),
            vec![
                Arc::new(Int64Array::from_iter_values(
                    (0..4).map(|row| commit * 4 + row),
                )),
                Arc::new(StringArray::from(vec![
                    Some(first.clone()),
                    Some(first),
                    Some(second.clone()),
                    Some(second),
                ])),
            ],
        )
        .expect("the batch matches the schema");
        table
            .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
            .expect("the append commits");
    }
    table
}

/// Scan planning cost as the file count grows, and what pruning saves.
fn plan_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("plan");
    let small = plan_table(SCRATCH_LABELS[0], 10);
    let large = plan_table(SCRATCH_LABELS[1], PLAN_LARGE_FILES);

    // Proven once outside the timers, so no bench can silently measure an
    // empty table or a filter that prunes nothing.
    let whole = large.plan(&[]).expect("the whole-table plan reads");
    assert_eq!(
        whole.tasks.len(),
        PLAN_LARGE_FILES,
        "the large table holds every file"
    );
    assert_eq!(
        whole.manifests_read,
        PLAN_LARGE_FILES / 2,
        "one manifest per commit"
    );
    let pruned = large.plan(&[PRUNED_FILTER]).expect("the pruned plan reads");
    assert!(pruned.manifests_skipped() > 0, "summaries must prune");
    assert!(pruned.files_skipped() > 0, "partition tuples must prune");

    group.bench_function("files_10", |bencher| {
        bencher.iter(|| black_box(&small).plan(&[]).expect("the small plan reads"));
    });
    group.bench_function(format!("files_{PLAN_LARGE_FILES}"), |bencher| {
        bencher.iter(|| black_box(&large).plan(&[]).expect("the large plan reads"));
    });
    // The full side of this comparison is `files_200` above: same table, same
    // snapshot, no filter. What this one adds is the summary check per
    // manifest-list row against what it saves - three quarters of the Avro
    // manifests never opened.
    group.bench_function(format!("pruned_vs_full_{PLAN_LARGE_FILES}"), |bencher| {
        bencher.iter(|| {
            black_box(&large)
                .plan(black_box(&[PRUNED_FILTER]))
                .expect("the pruned plan reads")
        });
    });
    group.finish();
}

/// A fifty-column schema, distinct per revision the way evolution leaves them.
fn wide_schema(revision: i32) -> Field {
    let mut schema =
        DataType::from_fields((0..50).map(|column| {
            DataType::Int64.required_field(format!("column-{revision}-{column:02}"))
        }))
        .expect("the generated columns are unique")
        .required_field("row");
    assign_field_ids(&mut schema, 1).expect("the generated schema takes identifiers");
    schema
}

/// A metadata document shaped like a long-lived table: 100 snapshots, 3
/// schemas of 50 columns, and a 100-entry snapshot log.
fn synthesized_metadata() -> TableMetadata {
    let mut metadata = TableMetadata::new(
        FormatVersion::V2,
        "file:///bench/table",
        wide_schema(0),
        PartitionSpec::unpartitioned(),
    )
    .expect("the synthetic table describes");
    for revision in 1..3 {
        metadata
            .add_schema(wide_schema(revision))
            .expect("the evolved schema adds");
    }
    let base_timestamp_ms = metadata.last_updated_ms();
    for index in 1..=100_i64 {
        metadata
            .set_current_snapshot(Snapshot {
                snapshot_id: index,
                parent_snapshot_id: (index > 1).then(|| index - 1),
                sequence_number: Some(index),
                timestamp_ms: base_timestamp_ms + index,
                manifest_list: SmolStr::new(format!(
                    "file:///bench/table/metadata/snap-{index}.avro"
                )),
                manifests: None,
                summary: vec![
                    (
                        SmolStr::new_static("operation"),
                        SmolStr::new_static("append"),
                    ),
                    (
                        SmolStr::new_static("added-records"),
                        SmolStr::new_static("4"),
                    ),
                ],
                schema_id: Some(0),
                encryption_key_id: None,
                first_row_id: None,
                added_rows: None,
            })
            .expect("the snapshot adds");
    }
    metadata
}

/// `TableMetadata::from_json` throughput over a long-lived table's document.
fn metadata_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("metadata");
    let metadata = synthesized_metadata();
    let document = metadata
        .clone()
        .into_json()
        .expect("the synthetic metadata projects to JSON");
    let text = String::from_utf8(
        yggdryl::text::json::into_bytes(&document).expect("the synthetic document encodes"),
    )
    .expect("the encoded document is UTF-8");

    // Proven once outside the timer: the text really carries the shape the
    // benchmark claims to parse.
    let parsed =
        TableMetadata::from_json(&yggdryl::text::json::from_utf8(&text).expect("the text parses"))
            .expect("the document reads back");
    assert_eq!(parsed.snapshots().len(), 100);
    assert_eq!(parsed.schemas().len(), 3);
    assert_eq!(parsed.snapshot_log().len(), 100);

    group.throughput(Throughput::Bytes(text.len() as u64));
    group.bench_function("parse_json", |bencher| {
        bencher.iter(|| {
            let value = yggdryl::text::json::from_utf8(black_box(text.as_str()))
                .expect("the serialized document parses");
            TableMetadata::from_json(&value).expect("the parsed document reads")
        });
    });
    let mut expired = metadata.clone();
    assert_eq!(
        expired
            .expire_snapshots(Some(i64::MAX), Some(1), &[])
            .expect("the synthetic snapshots expire")
            .len(),
        99
    );
    group.bench_function("expire_snapshots_100", |bencher| {
        bencher.iter_batched(
            || metadata.clone(),
            |mut metadata| {
                black_box(
                    metadata
                        .expire_snapshots(Some(i64::MAX), Some(1), &[])
                        .expect("the synthetic snapshots expire"),
                )
            },
            BatchSize::SmallInput,
        );
    });
    group.finish();
}

/// Build `count` synthetic manifest entries over the venue-partitioned schema.
///
/// Each entry carries what the table's own writer records: a partition tuple,
/// per-column counts, and encoded bounds, so the decode pays the per-field
/// map work a real manifest costs.
fn manifest_entries(count: usize) -> Vec<ManifestEntry> {
    (0..count)
        .map(|index| {
            let name = venue(index % VENUES);
            let row = i64::try_from(index).expect("the entry index fits a row count");
            let base = row * 100;
            ManifestEntry::added(
                7_001,
                DataFile {
                    file_path: format!(
                        "file:///bench/table/data/venue={name}/part-{index:05}.parquet"
                    )
                    .into(),
                    partition: vec![Scalar::from(name.as_str())],
                    record_count: 100,
                    file_size_in_bytes: 4_096,
                    column_sizes: vec![(1, 800), (2, 1_600)],
                    value_counts: vec![(1, 100), (2, 100)],
                    null_value_counts: vec![(1, 0), (2, 0)],
                    lower_bounds: vec![
                        (1, base.to_le_bytes().to_vec()),
                        (2, name.clone().into_bytes()),
                    ],
                    upper_bounds: vec![
                        (1, (base + 99).to_le_bytes().to_vec()),
                        (2, name.into_bytes()),
                    ],
                    split_offsets: vec![4],
                    ..DataFile::default()
                },
            )
        })
        .collect()
}

/// `read_manifest` over a thousand-entry manifest held in memory.
fn manifest_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("manifest");
    let schema = plan_schema();
    let spec = PartitionSpec::identity(0, &schema, &["venue"]).expect("venue is a schema column");
    let entries = manifest_entries(MANIFEST_BASE);
    let mut buffer = Buffer::new();
    buffer.set_media_type(MediaType::new(MimeType::AVRO));
    write_manifest(&mut buffer, FormatVersion::V2, &schema, &spec, &entries)
        .expect("the synthetic manifest encodes");

    // Proven once outside the timer: the container really holds the entries.
    assert_eq!(
        read_manifest(&buffer)
            .expect("the manifest reads back")
            .len(),
        MANIFEST_BASE
    );

    group.throughput(Throughput::Elements(MANIFEST_BASE as u64));
    group.bench_function(format!("decode_{MANIFEST_BASE}"), |bencher| {
        bencher.iter(|| read_manifest(black_box(&buffer)).expect("the manifest decodes"));
    });

    // The planning fast path against the full decode, at manifest scale. The
    // filtered variant keeps counts and bounds - the lazy statistics a
    // filtered plan consults - and the unfiltered one skips even those.
    for scale in MANIFEST_SCALES {
        let entries = manifest_entries(scale);
        let mut stored = Buffer::new();
        stored.set_media_type(MediaType::new(MimeType::AVRO));
        write_manifest(&mut stored, FormatVersion::V2, &schema, &spec, &entries)
            .expect("the synthetic manifest encodes");
        // Proven once outside the timers: both paths see every entry.
        assert_eq!(
            read_manifest_for_plan(&stored, true)
                .expect("the planning path reads")
                .len(),
            scale
        );
        assert_eq!(
            read_manifest_spec(&stored).expect("the manifest header reads"),
            spec
        );
        let elements = u64::try_from(scale).expect("the scale fits");
        group.throughput(Throughput::Elements(elements));
        if scale > 1_000 {
            group.sample_size(20);
        }
        group.bench_function(format!("decode_full/{scale}"), |bencher| {
            bencher.iter(|| read_manifest(black_box(&stored)).expect("the manifest decodes"));
        });
        group.bench_function(format!("decode_plan_with_stats/{scale}"), |bencher| {
            bencher.iter(|| {
                read_manifest_for_plan(black_box(&stored), true).expect("the plan path decodes")
            });
        });
        group.bench_function(format!("decode_plan_identity_only/{scale}"), |bencher| {
            bencher.iter(|| {
                read_manifest_for_plan(black_box(&stored), false).expect("the plan path decodes")
            });
        });
        if scale == MANIFEST_SCALES[2] {
            group.bench_function(format!("decode_spec_header/{scale}"), |bencher| {
                bencher.iter(|| {
                    read_manifest_spec(black_box(&stored)).expect("the manifest header decodes")
                });
            });
        }
    }
    group.finish();
}

/// The single-value renderer every partition directory name goes through.
fn partition_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("partition");
    let date = Scalar::date32(19_723);
    let text = Scalar::from("XNAS");

    // Proven once outside the timers: both values render, and the date renders
    // as calendar text rather than its day count.
    assert_eq!(
        partition_text(&date).expect("the date renders"),
        "2024-01-01"
    );
    assert_eq!(partition_text(&text).expect("the text renders"), "XNAS");

    group.bench_function("text_render/date", |bencher| {
        bencher.iter(|| partition_text(black_box(&date)).expect("the date renders"));
    });
    group.bench_function("text_render/utf8", |bencher| {
        bencher.iter(|| partition_text(black_box(&text)).expect("the text renders"));
    });
    group.finish();
}

/// Stable structural hashes over representative immutable Iceberg values.
fn identity_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("identity");
    let schema = plan_schema();
    let spec = PartitionSpec::identity(1, &schema, &["venue"]).expect("valid partition spec");
    let partition = spec.fields[0].clone();
    let metadata = synthesized_metadata();
    let snapshot = metadata.snapshots()[0].clone();
    let entry = manifest_entries(1).into_iter().next().expect("one entry");
    let data = entry.data_file.clone();
    let summary = FieldSummary::default();
    let manifest = ManifestFile {
        manifest_path: "metadata/manifest.avro".into(),
        manifest_length: 4_096,
        partition_spec_id: spec.spec_id,
        content: ManifestContent::Data,
        sequence_number: 1,
        min_sequence_number: 1,
        added_snapshot_id: snapshot.snapshot_id,
        added_files_count: Some(1),
        existing_files_count: Some(0),
        deleted_files_count: Some(0),
        added_rows_count: Some(data.record_count),
        existing_rows_count: Some(0),
        deleted_rows_count: Some(0),
        partitions: vec![summary.clone()],
        key_metadata: None,
        first_row_id: None,
    };
    let snapshot_ref = SnapshotRef::branch(snapshot.snapshot_id);
    let compaction = Compaction {
        files_before: 8,
        files_after: 2,
        bytes_rewritten: 32_768,
    };
    let options = IcebergOptions::new()
        .with_commit_retries(2)
        .try_with_data_mime_type(MimeType::AVRO)
        .expect("Avro is an Iceberg data MIME type");
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
    let conflict = CommitConflict {
        expected_version: 1,
        beaten: 2,
        last_seen_version: 3,
    };
    let task = ScanTask {
        entry: entry.clone(),
        spec: spec.clone(),
        residual: vec![0],
    };
    let plan = ScanPlan {
        tasks: vec![task.clone()],
        excluded: Vec::new(),
        skipped: vec![manifest.clone()],
        manifests_read: 1,
    };

    group.bench_function("stable_hash_partition_field", |bencher| {
        bencher.iter(|| black_box(&partition).stable_hash());
    });
    group.bench_function("stable_hash_partition_spec", |bencher| {
        bencher.iter(|| black_box(&spec).stable_hash());
    });
    group.bench_function("stable_hash_snapshot", |bencher| {
        bencher.iter(|| black_box(&snapshot).stable_hash());
    });
    group.bench_function("stable_hash_snapshot_ref", |bencher| {
        bencher.iter(|| black_box(&snapshot_ref).stable_hash());
    });
    group.bench_function("stable_hash_data_file", |bencher| {
        bencher.iter(|| black_box(&data).stable_hash());
    });
    group.bench_function("stable_hash_manifest_file", |bencher| {
        bencher.iter(|| black_box(&manifest).stable_hash());
    });
    group.bench_function("stable_hash_manifest_entry", |bencher| {
        bencher.iter(|| black_box(&entry).stable_hash());
    });
    group.bench_function("stable_hash_field_summary", |bencher| {
        bencher.iter(|| black_box(&summary).stable_hash());
    });
    group.bench_function("stable_hash_compaction", |bencher| {
        bencher.iter(|| black_box(&compaction).stable_hash());
    });
    group.bench_function("stable_hash_options", |bencher| {
        bencher.iter(|| black_box(&options).stable_hash());
    });
    group.bench_function("stable_hash_sort_field", |bencher| {
        bencher.iter(|| black_box(&sort).stable_hash());
    });
    group.bench_function("stable_hash_sort_order", |bencher| {
        bencher.iter(|| black_box(&order).stable_hash());
    });
    group.bench_function("stable_hash_commit_conflict", |bencher| {
        bencher.iter(|| black_box(&conflict).stable_hash());
    });
    group.bench_function("stable_hash_scan_task", |bencher| {
        bencher.iter(|| black_box(&task).stable_hash());
    });
    group.bench_function("stable_hash_scan_plan", |bencher| {
        bencher.iter(|| black_box(&plan).stable_hash());
    });
    group.bench_function("stable_hash_table_metadata", |bencher| {
        bencher.iter(|| black_box(&metadata).stable_hash());
    });
    group.finish();
}

/// What compaction buys a planner: the 200-file table, folded once.
///
/// The compaction itself runs outside the timer - it is a one-off maintenance
/// write - and what is measured is the plan every later read starts with,
/// against the same snapshot shape `plan/files_200` measures before folding.
fn compact_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("compact");
    let mut table = plan_table(SCRATCH_LABELS[2], COMPACT_FILES);

    // Proven once outside the timer: the fold really happened, so the plan
    // being measured reads 8 files where the uncompacted table read 200.
    let compaction = table.compact().expect("the table compacts");
    assert_eq!(
        compaction.files_before, COMPACT_FILES,
        "every small file rewrites"
    );
    assert_eq!(compaction.files_after, VENUES, "one merged file per venue");
    let plan = table.plan(&[]).expect("the compacted plan reads");
    assert_eq!(plan.tasks.len(), VENUES);

    group.bench_function(format!("plan_after_compact_{COMPACT_FILES}"), |bencher| {
        bencher.iter(|| {
            black_box(&table)
                .plan(&[])
                .expect("the compacted plan reads")
        });
    });
    group.finish();
}

/// Build an unpartitioned table of `files` single-row data files.
///
/// One append is one commit is one file, so the merge benchmark gets a table
/// whose per-file id bounds are as tight as bounds can be - which is exactly
/// what lets the measured upsert carry most files unread.
fn merge_table(label: &str, files: usize) -> Table<Folder> {
    let path = scratch(label);
    let _ = std::fs::remove_dir_all(&path);
    let schema = plan_schema();
    let mut table = Table::create(
        Folder::new(&path).expect("the scratch directory is addressable"),
        FormatVersion::V2,
        schema.clone(),
        PartitionSpec::unpartitioned(),
    )
    .expect("the scratch table creates");
    let arrow = schema
        .into_arrow_schema()
        .expect("the schema projects to Arrow");
    for index in 0..files {
        let id = i64::try_from(index).expect("the file index fits an id");
        let batch = RecordBatch::try_new(
            arrow.clone(),
            vec![
                Arc::new(Int64Array::from(vec![id])),
                Arc::new(StringArray::from(vec![Some(venue(index % VENUES))])),
            ],
        )
        .expect("the batch matches the schema");
        table
            .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
            .expect("the append commits");
    }
    table
}

/// Upserting ten keyed rows into a table of fifty single-row files.
fn merge_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("merge");
    let mut table = merge_table(SCRATCH_LABELS[3], MERGE_FILES);
    let arrow = plan_schema()
        .into_arrow_schema()
        .expect("the schema projects to Arrow");
    let upsert = RecordBatch::try_new(
        arrow,
        vec![
            Arc::new(Int64Array::from_iter_values(0..10)),
            Arc::new(StringArray::from(vec![Some(venue(0)); 10])),
        ],
    )
    .expect("the upsert batch matches the schema");
    let merge_by = yggdryl::Selector::from_columns(["id"]);

    // Proven once outside the timer, which also settles the table into the
    // steady state every measured merge sees: the ten matched single-row files
    // fold into one and the other forty are carried untouched, so an upsert of
    // stored keys adds no row and every later merge rewrites that one file.
    table
        .commit_merge(
            yggdryl::arrow::batch_reader(upsert.schema(), [upsert.clone()]),
            &merge_by,
            true,
        )
        .expect("the priming merge commits");
    let plan = table.plan(&[]).expect("the merged table plans");
    assert_eq!(
        plan.record_count().expect("the planned row count fits i64"),
        i64::try_from(MERGE_FILES).expect("the file count fits i64"),
        "an upsert of stored keys adds no row"
    );
    assert_eq!(
        plan.tasks.len(),
        MERGE_FILES - 10 + 1,
        "ten matched files fold into one"
    );

    group.bench_function(format!("upsert_into_{MERGE_FILES}_files"), |bencher| {
        bencher.iter(|| {
            table
                .commit_merge(
                    yggdryl::arrow::batch_reader(upsert.schema(), [upsert.clone()]),
                    black_box(&merge_by),
                    true,
                )
                .expect("the merge commits");
        });
    });
    group.finish();
}

/// Build a venue-partitioned table of `partitions` single-row data files.
///
/// One append per partition is one commit is one file, and every file holds
/// the same id, so the id bounds cannot tell the partitions apart: only the
/// partition isolation can keep the measured merge from reading them all.
fn partitioned_merge_table(label: &str, partitions: usize) -> Table<Folder> {
    let path = scratch(label);
    let _ = std::fs::remove_dir_all(&path);
    let schema = plan_schema();
    let spec = PartitionSpec::identity(1, &schema, &["venue"]).expect("venue is a schema column");
    let mut table = Table::create(
        Folder::new(&path).expect("the scratch directory is addressable"),
        FormatVersion::V2,
        schema.clone(),
        spec,
    )
    .expect("the scratch table creates");
    let arrow = schema
        .into_arrow_schema()
        .expect("the schema projects to Arrow");
    for index in 0..partitions {
        let batch = RecordBatch::try_new(
            arrow.clone(),
            vec![
                Arc::new(Int64Array::from(vec![1_i64])),
                Arc::new(StringArray::from(vec![Some(venue(index))])),
            ],
        )
        .expect("the batch matches the schema");
        table
            .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
            .expect("the append commits");
    }
    table
}

/// Upserting into one partition of many: the merge reads that partition's
/// file and carries every other one under its own path.
fn isolated_merge_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("merge");
    let mut table = partitioned_merge_table(SCRATCH_LABELS[6], MERGE_PARTITIONS);
    let arrow = plan_schema()
        .into_arrow_schema()
        .expect("the schema projects to Arrow");
    let upsert = RecordBatch::try_new(
        arrow,
        vec![
            Arc::new(Int64Array::from_iter_values(0..10)),
            Arc::new(StringArray::from(vec![Some(venue(0)); 10])),
        ],
    )
    .expect("the upsert batch matches the schema");
    let merge_by = yggdryl::Selector::from_columns(["id"]);

    // Proven once outside the timer: every file of every other partition
    // keeps its exact path, so the merge rewrote one partition of many.
    let before: std::collections::BTreeSet<SmolStr> = table
        .data_files()
        .expect("the table lists its files")
        .into_iter()
        .map(|(file, _)| file.file_path)
        .collect();
    table
        .commit_merge(
            yggdryl::arrow::batch_reader(upsert.schema(), [upsert.clone()]),
            &merge_by,
            true,
        )
        .expect("the priming merge commits");
    let after: std::collections::BTreeSet<SmolStr> = table
        .data_files()
        .expect("the table lists its files")
        .into_iter()
        .map(|(file, _)| file.file_path)
        .collect();
    assert_eq!(
        before.intersection(&after).count(),
        MERGE_PARTITIONS - 1,
        "every other partition's file is carried under its own path"
    );

    group.throughput(Throughput::Elements(10));
    group.bench_function(format!("one_partition_of_{MERGE_PARTITIONS}"), |bencher| {
        bencher.iter(|| {
            table
                .commit_merge(
                    yggdryl::arrow::batch_reader(upsert.schema(), [upsert.clone()]),
                    black_box(&merge_by),
                    true,
                )
                .expect("the merge commits");
        });
    });
    group.finish();
}

/// One batch spanning many partitions, written as one commit.
fn partitioned_commit_batch(partitions: usize, rows_per_partition: usize) -> RecordBatch {
    let arrow = plan_schema()
        .into_arrow_schema()
        .expect("the schema projects to Arrow");
    let rows = partitions * rows_per_partition;
    RecordBatch::try_new(
        arrow,
        vec![
            Arc::new(Int64Array::from_iter_values(
                (0..rows).map(|row| i64::try_from(row).expect("the row fits an id")),
            )),
            Arc::new(StringArray::from_iter_values(
                (0..rows).map(|row| venue(row % partitions)),
            )),
        ],
    )
    .expect("the batch matches the schema")
}

/// One partitioned commit on one thread against four: the partition groups
/// are independent, so their files are written concurrently and the
/// manifest still lists them in group order.
fn parallel_commit_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("commit");
    group.sample_size(10);
    let path = scratch(SCRATCH_LABELS[7]);
    let schema = plan_schema();
    let batch = partitioned_commit_batch(COMMIT_PARTITIONS, COMMIT_ROWS_PER_PARTITION);
    group.throughput(Throughput::Elements(batch.num_rows() as u64));
    for parallelism in [1_usize, 4] {
        group.bench_function(
            format!("parallel_partitions_{COMMIT_PARTITIONS}/parallelism-{parallelism}"),
            |bencher| {
                bencher.iter_batched(
                    || {
                        let _ = std::fs::remove_dir_all(&path);
                        let mut table = Table::create(
                            Folder::new(&path).expect("the scratch directory is addressable"),
                            FormatVersion::V2,
                            schema.clone(),
                            PartitionSpec::identity(1, &schema, &["venue"])
                                .expect("venue is a schema column"),
                        )
                        .expect("the scratch table creates");
                        table.set_options(
                            IcebergOptions::new()
                                .try_with_write_parallelism(parallelism)
                                .expect("a positive parallelism is valid"),
                        );
                        table
                    },
                    |mut table| {
                        table
                            .commit_append(yggdryl::arrow::batch_reader(
                                batch.schema(),
                                [batch.clone()],
                            ))
                            .expect("the partitioned append commits");
                        assert_eq!(
                            table.data_files().expect("the table lists its files").len(),
                            COMMIT_PARTITIONS
                        );
                    },
                    BatchSize::PerIteration,
                );
            },
        );
    }
    group.finish();
}

/// The four-column trade schema the read benchmark scans.
fn read_schema() -> Field {
    let mut schema = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Float64.nullable_field("price"),
        DataType::utf8().nullable_field("venue"),
        DataType::Int64.required_field("ts"),
    ])
    .expect("the static columns are unique")
    .required_field("row");
    assign_field_ids(&mut schema, 1).expect("the static schema takes identifiers");
    schema
}

/// Build an unpartitioned table of `files` data files, `rows` rows each.
///
/// Each append is one commit is one file of (int64 id, float64 price, utf8
/// venue from the eight-value pool, timestamp-like int64), so the parallel
/// read gets files large enough that decode dominates the open.
fn read_table(label: &str, files: usize, rows: usize) -> Table<Folder> {
    let path = scratch(label);
    let _ = std::fs::remove_dir_all(&path);
    let schema = read_schema();
    let mut table = Table::create(
        Folder::new(&path).expect("the scratch directory is addressable"),
        FormatVersion::V2,
        schema.clone(),
        PartitionSpec::unpartitioned(),
    )
    .expect("the scratch table creates");
    let arrow = schema
        .into_arrow_schema()
        .expect("the schema projects to Arrow");
    for file in 0..files {
        let base = i64::try_from(file * rows).expect("the row index fits an id");
        let ids: Vec<i64> = (0..rows)
            .map(|row| base + i64::try_from(row).expect("the row fits an id"))
            .collect();
        let batch = RecordBatch::try_new(
            arrow.clone(),
            vec![
                Arc::new(Int64Array::from(ids.clone())),
                #[allow(clippy::cast_precision_loss)]
                Arc::new(Float64Array::from_iter_values(
                    ids.iter().map(|id| *id as f64 * 0.01),
                )),
                Arc::new(StringArray::from_iter_values(ids.iter().map(|id| {
                    venue(usize::try_from(*id).unwrap_or_default() % VENUES)
                }))),
                Arc::new(Int64Array::from_iter_values(
                    ids.iter().map(|id| 1_700_000_000_000 + *id),
                )),
            ],
        )
        .expect("the batch matches the schema");
        table
            .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
            .expect("the append commits");
    }
    table
}

/// Drain one full scan, counting the rows it yields.
fn scan_rows(table: &Table<Folder>) -> usize {
    table
        .scan(None)
        .expect("the scan plans")
        .map(|batch| batch.expect("the batch decodes").num_rows())
        .sum()
}

/// A full-table collect, decoded one file at a time versus four at a time.
///
/// The table is built once - 32 files of 100k rows - and only the options
/// change between the two measurements, so the comparison is exactly the
/// read path. The parallel side forces the thresholds low because what it
/// measures is the decode fan-out, not the decision.
fn read_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("read");
    group.sample_size(10);
    let mut table = read_table(SCRATCH_LABELS[4], READ_FILES, READ_ROWS_PER_FILE);

    let sequential = IcebergOptions::new()
        .try_with_read_parallelism(1)
        .expect("one reader thread is valid");
    let parallel = IcebergOptions::new()
        .try_with_read_parallelism(4)
        .expect("four reader threads are valid")
        .with_read_parallel_min_files(1)
        .with_read_parallel_min_file_size_bytes(0);

    // Proven once outside the timers: both paths read every row.
    table.set_options(sequential.clone());
    assert_eq!(scan_rows(&table), READ_ROWS);
    table.set_options(parallel.clone());
    assert_eq!(scan_rows(&table), READ_ROWS);

    group.throughput(Throughput::Elements(READ_ROWS as u64));
    let shape = if cfg!(debug_assertions) {
        format!("{READ_FILES}x{READ_ROWS_PER_FILE}rows_smoke")
    } else {
        "32x4mb".to_owned()
    };
    table.set_options(sequential);
    group.bench_function(
        format!("parallel_vs_sequential_{shape}/parallelism-1"),
        |bencher| {
            bencher.iter(|| scan_rows(black_box(&table)));
        },
    );
    table.set_options(parallel);
    group.bench_function(
        format!("parallel_vs_sequential_{shape}/parallelism-4"),
        |bencher| {
            bencher.iter(|| scan_rows(black_box(&table)));
        },
    );
    group.finish();
}

/// Four writers hammering one table: wall time per successful commit.
///
/// Each iteration starts from a fresh one-version table, then four threads
/// open their own stale handles and append one small batch each, so every
/// commit after the first observes a newer version, rebases, and pays a
/// jittered backoff - which is exactly what is being measured.
///
/// One mutex serializes the append calls themselves. The local backend is a
/// memory mapping, and `yggdryl::holder::local` documents the consequence: two
/// writers truncating one mapped file at the same instant can raise SIGBUS,
/// which no retry can catch. The gate stands in for the atomic PUT an object
/// store gives every writer, while the *handles* still race optimistically -
/// each one commits against a version another writer already advanced.
fn contended_commit_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("commit");
    group.sample_size(10);
    let path = scratch(SCRATCH_LABELS[5]);
    let schema = plan_schema();
    let arrow = schema
        .clone()
        .into_arrow_schema()
        .expect("the schema projects to Arrow");

    group.throughput(Throughput::Elements(4));
    group.bench_function("contended_append_x4", |bencher| {
        bencher.iter_batched(
            || {
                let _ = std::fs::remove_dir_all(&path);
                Table::create(
                    Folder::new(&path).expect("the scratch directory is addressable"),
                    FormatVersion::V2,
                    schema.clone(),
                    PartitionSpec::unpartitioned(),
                )
                .expect("the scratch table creates")
            },
            |_created| {
                let gate = std::sync::Mutex::new(());
                std::thread::scope(|scope| {
                    for worker in 0..4_i64 {
                        let path = &path;
                        let gate = &gate;
                        let arrow = arrow.clone();
                        scope.spawn(move || {
                            // Opened before the gate, so every handle is
                            // equally stale and every commit but the first
                            // has to rebase.
                            let mut table = Table::open(
                                Folder::new(path).expect("the table folder is addressable"),
                            )
                            .expect("the contended table opens");
                            table.set_options(
                                IcebergOptions::new()
                                    .with_commit_retries(16)
                                    .with_commit_min_backoff_ms(1)
                                    .with_commit_max_backoff_ms(20),
                            );
                            let batch = RecordBatch::try_new(
                                arrow.clone(),
                                vec![
                                    Arc::new(Int64Array::from(vec![worker])),
                                    Arc::new(StringArray::from(vec![Some(venue(
                                        usize::try_from(worker).unwrap_or_default(),
                                    ))])),
                                ],
                            )
                            .expect("the batch matches the schema");
                            let _serialized = gate.lock().expect("the gate is not poisoned");
                            table
                                .commit_append(yggdryl::arrow::batch_reader(
                                    batch.schema(),
                                    [batch],
                                ))
                                .expect("the contended append commits");
                        });
                    }
                });
            },
            BatchSize::PerIteration,
        );
    });
    group.finish();
}

/// Name resolution through the catalog hierarchy, with backend calls counted.
///
/// Removing a probe is a round-trip saving before it is a CPU saving, so the
/// group prints the exact number of Arrow-filesystem calls each operation
/// makes once per run beside Criterion's wall time - a regression in either
/// is a regression.
fn catalog_resolve_benchmarks(criterion: &mut Criterion) {
    use std::any::Any;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use yggdryl::holder::fs::{
        ByteReader, ByteWriter, FileInfo, FileInfos, FileSelector, FileSystem, MemoryFileSystem,
        OutputMetadata, RandomAccessReader,
    };
    use yggdryl::media::iceberg::Catalog;

    /// A memory filesystem that counts every vtable call reaching it.
    #[derive(Debug, Default)]
    struct Counting {
        inner: MemoryFileSystem,
        calls: AtomicUsize,
    }

    impl Counting {
        fn count(&self) {
            self.calls.fetch_add(1, Ordering::Relaxed);
        }
    }

    impl FileSystem for Counting {
        fn type_name(&self) -> &str {
            self.inner.type_name()
        }

        fn equals(&self, other: &dyn FileSystem) -> bool {
            self.count();
            other
                .as_any()
                .downcast_ref::<Self>()
                .is_some_and(|other| std::ptr::eq(self, other))
        }

        fn normalize_path(&self, path: &str) -> yggdryl::Result<String> {
            self.count();
            self.inner.normalize_path(path)
        }

        fn file_info(&self, path: &str) -> yggdryl::Result<FileInfo> {
            self.count();
            self.inner.file_info(path)
        }

        fn list(&self, selector: &FileSelector) -> FileInfos {
            self.count();
            self.inner.list(selector)
        }

        fn create_dir(&self, path: &str, recursive: bool) -> yggdryl::Result<()> {
            self.count();
            self.inner.create_dir(path, recursive)
        }

        fn delete_dir(&self, path: &str) -> yggdryl::Result<()> {
            self.count();
            self.inner.delete_dir(path)
        }

        fn delete_dir_contents(&self, path: &str, missing_dir_ok: bool) -> yggdryl::Result<()> {
            self.count();
            self.inner.delete_dir_contents(path, missing_dir_ok)
        }

        fn delete_root_dir_contents(&self) -> yggdryl::Result<()> {
            self.count();
            self.inner.delete_root_dir_contents()
        }

        fn delete_file(&self, path: &str) -> yggdryl::Result<()> {
            self.count();
            self.inner.delete_file(path)
        }

        fn copy_file(&self, source: &str, target: &str) -> yggdryl::Result<()> {
            self.count();
            self.inner.copy_file(source, target)
        }

        fn move_file(&self, source: &str, target: &str) -> yggdryl::Result<()> {
            self.count();
            self.inner.move_file(source, target)
        }

        fn open_input_file(&self, path: &str) -> yggdryl::Result<Box<dyn RandomAccessReader>> {
            self.count();
            self.inner.open_input_file(path)
        }

        fn open_input_stream(&self, path: &str) -> yggdryl::Result<Box<dyn ByteReader>> {
            self.count();
            self.inner.open_input_stream(path)
        }

        fn open_output_stream(
            &self,
            path: &str,
            metadata: Option<&OutputMetadata>,
        ) -> yggdryl::Result<Box<dyn ByteWriter>> {
            self.count();
            self.inner.open_output_stream(path, metadata)
        }

        fn open_append_stream(
            &self,
            path: &str,
            metadata: Option<&OutputMetadata>,
        ) -> yggdryl::Result<Box<dyn ByteWriter>> {
            self.count();
            self.inner.open_append_stream(path, metadata)
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    let schema = || {
        DataType::from_fields([DataType::Int64.required_field("id")])
            .expect("a valid struct root")
            .required_field("row")
    };
    let counted = || {
        let filesystem = Arc::new(Counting::default());
        let warehouse = yggdryl::holder::fs::Folder::from_path(
            Arc::clone(&filesystem) as Arc<dyn FileSystem>,
            "warehouse",
            None,
        )
        .expect("a valid location");
        (filesystem, Catalog::new(warehouse))
    };

    let mut group = criterion.benchmark_group("catalog_resolve");

    // One populated catalog for the two read legs.
    let (filesystem, catalog) = counted();
    catalog
        .tables()
        .create("sales.eu.orders", schema())
        .expect("a creatable table");

    // The backend-call counts, printed once per leg so the round trips are a
    // number in the report rather than a claim in a comment.
    let cost = |operation: &dyn Fn()| {
        let before = filesystem.calls.load(Ordering::Relaxed);
        operation();
        filesystem.calls.load(Ordering::Relaxed) - before
    };
    println!(
        "catalog_resolve backend calls: dotted get = {}, cascaded get = {}",
        cost(&|| {
            catalog.table("sales.eu.orders").expect("an openable table");
        }),
        cost(&|| {
            catalog
                .namespaces()
                .get("sales")
                .expect("a namespace")
                .namespaces()
                .get("eu")
                .expect("a namespace")
                .tables()
                .get("orders")
                .expect("an openable table");
        }),
    );

    group.bench_function("get/dotted", |bencher| {
        bencher.iter(|| {
            black_box(&catalog)
                .table("sales.eu.orders")
                .expect("an openable table")
        });
    });

    group.bench_function("get/cascade", |bencher| {
        bencher.iter(|| {
            black_box(&catalog)
                .namespaces()
                .get("sales")
                .expect("a namespace")
                .namespaces()
                .get("eu")
                .expect("a namespace")
                .tables()
                .get("orders")
                .expect("an openable table")
        });
    });

    // The create leg builds a fresh warehouse per iteration: the point is the
    // missing three-level ancestry coming into being from the metadata write.
    let mut ancestry_calls = None;
    group.bench_function("create/missing-ancestry", |bencher| {
        bencher.iter_batched(
            counted,
            |(filesystem, catalog)| {
                let before = filesystem.calls.load(Ordering::Relaxed);
                catalog
                    .tables()
                    .create("a.b.c.orders", schema())
                    .expect("a creatable table");
                ancestry_calls.get_or_insert(filesystem.calls.load(Ordering::Relaxed) - before);
            },
            BatchSize::SmallInput,
        );
    });
    if let Some(calls) = ancestry_calls {
        println!("catalog_resolve backend calls: create into missing ancestry = {calls}");
    }

    group.finish();
}

/// The same table over an in-process S3, with every request counted.
///
/// One store, one bucket, a fresh table per measured commit. Each leg's
/// request count is printed once, by shape, beside Criterion's wall time:
/// on a real store the round trips *are* the cost, and the loopback timing
/// only shows that nothing else is hiding in them. The counts are the ones
/// `holder::object::tests::accounting` pins.
#[cfg(feature = "object")]
mod s3 {
    use std::cell::Cell;
    use std::hint::black_box;
    use std::path::PathBuf;
    use std::sync::Arc;

    use arrow_array::RecordBatch;
    use criterion::{BatchSize, Criterion, Throughput};
    use yggdryl::arrow::BatchReader;
    use yggdryl::holder::local::Folder as LocalFolder;
    use yggdryl::holder::object::{
        Credentials, File, Folder, ObjectOptions, file_with, folder_with,
    };
    use yggdryl::media::RecordOptions;
    use yggdryl::media::iceberg::{
        FormatVersion, PartitionSpec, Table, Transform, assign_field_ids,
    };
    use yggdryl::media::text::TextOptions;
    use yggdryl::{
        DataType, Field, FixCodec, FixRegistry, IOBase, IOMedia, Selector, TimeUnit, Timezone,
        fix_schema, fix_schema_carrying,
    };

    use super::server::FakeS3;
    use super::{partitioned_commit_batch, plan_schema, venue};
    use crate::bench_profile;

    const BUCKET: &str = "bench";
    const ACCESS_KEY: &str = "AKIAIOSFODNN7EXAMPLE";
    const SECRET_KEY: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";
    /// Partitions of the eight-way commits and of the scanned table.
    const PARTITIONS: usize = 8;
    /// Rows in each partition group of one measured commit.
    const ROWS_PER_PARTITION: usize = bench_profile::corpus(5_000, 50);
    /// The bridge's own log, the capture every FIX suite reads.
    const LOG: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fix/ulbridge.log"
    ));
    /// How many times the capture is repeated in the uploaded `.log` object.
    const LOG_REPEATS: usize = bench_profile::corpus(16, 1);

    fn store() -> FakeS3 {
        let store = FakeS3::start();
        store.create_bucket(BUCKET);
        store.set_recording(false);
        store
    }

    fn options(store: &FakeS3) -> ObjectOptions {
        ObjectOptions::default()
            .with_environment(false)
            .with_endpoint(store.endpoint())
            .with_region("us-east-1")
            .with_path_style(true)
            .with_credentials(Credentials::new(ACCESS_KEY, SECRET_KEY))
    }

    fn folder(store: &FakeS3, key: &str) -> Folder {
        folder_with(&format!("s3://{BUCKET}/{key}/"), options(store)).expect("a prefix handle")
    }

    fn log_object(store: &FakeS3) -> File {
        file_with(&format!("s3://{BUCKET}/logs/ulbridge.log"), options(store))
            .expect("an object handle")
    }

    /// The requests the store handled since the last clear, by shape.
    fn shape(store: &FakeS3) -> String {
        let (mut put, mut get, mut head, mut list, mut delete, mut post) = (0, 0, 0, 0, 0, 0);
        let requests = store.requests();
        for request in &requests {
            let listing = request
                .query
                .iter()
                .any(|(name, value)| name == "list-type" && value == "2");
            match request.method.as_str() {
                "PUT" => put += 1,
                "GET" if listing => list += 1,
                "GET" => get += 1,
                "HEAD" => head += 1,
                "DELETE" => delete += 1,
                "POST" => post += 1,
                _ => {}
            }
        }
        format!(
            "{} requests (PUT {put}, GET {get}, HEAD {head}, LIST {list}, DELETE {delete}, POST {post})",
            requests.len()
        )
    }

    /// Run `prepare`, then `operation` with the request log on, and print
    /// what the operation alone cost.
    fn probe<T>(
        store: &FakeS3,
        label: &str,
        prepare: impl FnOnce() -> T,
        operation: impl FnOnce(T),
    ) {
        let prepared = prepare();
        store.set_recording(true);
        store.clear_requests();
        operation(prepared);
        println!("s3 requests: {label} = {}", shape(store));
        store.set_recording(false);
    }

    /// A fresh venue-partitioned table under `key`.
    fn table(store: &FakeS3, key: &str) -> Table<Folder> {
        let schema = plan_schema();
        let spec =
            PartitionSpec::identity(1, &schema, &["venue"]).expect("venue is a schema column");
        Table::create(folder(store, key), FormatVersion::V2, schema, spec)
            .expect("the table creates")
    }

    fn reader(batch: &RecordBatch) -> BatchReader {
        yggdryl::arrow::batch_reader(batch.schema(), [batch.clone()])
    }

    fn scan_rows(table: &Table<Folder>, filters: &[(&str, &str)]) -> usize {
        table
            .scan_where(filters, None)
            .expect("the scan plans")
            .map(|batch| batch.expect("a batch").num_rows())
            .sum()
    }

    /// The text options a bridge log is read under, as the FIX pipeline
    /// benchmark reads it.
    fn text() -> RecordOptions {
        let mut options = TextOptions::new()
            .try_with_rowheader(yggdryl::ULBRIDGE_ROWHEADER)
            .expect("the row header compiles")
            .with_timezone(Timezone::UTC);
        options.start_rownum = Some(1);
        options.parse_mimetype = true;
        options.into()
    }

    fn text_rows(log: &File) -> usize {
        log.read_arrow_reader(&text())
            .expect("a reader")
            .map(|batch| batch.expect("a batch").num_rows())
            .sum()
    }

    /// The tracked seed dictionary, relative to the crate manifest.
    fn seed_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("config")
            .join("fix")
    }

    pub(super) fn benchmarks(criterion: &mut Criterion) {
        let store = store();
        let sequence = Cell::new(0_usize);
        let next = |label: &str| {
            sequence.set(sequence.get() + 1);
            format!("lake/{label}-{}", sequence.get())
        };
        let mut group = criterion.benchmark_group("s3");
        group.sample_size(10);

        // Commits: one partition group, then eight, each into a fresh table.
        for partitions in [1, PARTITIONS] {
            let batch = partitioned_commit_batch(partitions, ROWS_PER_PARTITION);
            let label = format!("commit_append/{partitions}_partitions");
            probe(
                &store,
                &label,
                || table(&store, &next("append")),
                |mut table| {
                    table
                        .commit_append(reader(&batch))
                        .expect("the append commits");
                },
            );
            group.throughput(Throughput::Elements(batch.num_rows() as u64));
            group.bench_function(label.as_str(), |bencher| {
                bencher.iter_batched(
                    || table(&store, &next("append")),
                    |mut table| {
                        table
                            .commit_append(reader(&batch))
                            .expect("the append commits");
                    },
                    BatchSize::PerIteration,
                );
            });
        }

        // An upsert into one partition of eight: the plan reads that
        // partition's manifest and file and carries the other seven.
        let seeded = partitioned_commit_batch(PARTITIONS, ROWS_PER_PARTITION);
        let upsert = partitioned_commit_batch(1, 10);
        let merge_by = Selector::from_columns(["id"]);
        let primed = |label: &str| {
            let mut table = table(&store, &next(label));
            table
                .commit_append(reader(&seeded))
                .expect("the seed commits");
            table
        };
        probe(
            &store,
            "upsert/one_partition_of_8",
            || primed("upsert"),
            |mut table| {
                table
                    .commit_merge(reader(&upsert), &merge_by, true)
                    .expect("the merge commits");
            },
        );
        group.throughput(Throughput::Elements(upsert.num_rows() as u64));
        group.bench_function("upsert/one_partition_of_8", |bencher| {
            bencher.iter_batched(
                || primed("upsert"),
                |mut table| {
                    table
                        .commit_merge(reader(&upsert), black_box(&merge_by), true)
                        .expect("the merge commits");
                },
                BatchSize::PerIteration,
            );
        });

        // Scans of one eight-partition table: whole, and one partition.
        let scanned = primed("scan");
        let two = venue(2);
        let pruned = [("venue", two.as_str())];
        assert_eq!(scan_rows(&scanned, &[]), seeded.num_rows());
        assert_eq!(scan_rows(&scanned, &pruned), ROWS_PER_PARTITION);
        probe(
            &store,
            "scan/full_8_partitions",
            || (),
            |()| {
                scan_rows(&scanned, &[]);
            },
        );
        group.throughput(Throughput::Elements(seeded.num_rows() as u64));
        group.bench_function("scan/full_8_partitions", |bencher| {
            bencher.iter(|| scan_rows(black_box(&scanned), &[]));
        });
        probe(
            &store,
            "scan/pruned_1_of_8",
            || (),
            |()| {
                scan_rows(&scanned, &pruned);
            },
        );
        group.throughput(Throughput::Elements(ROWS_PER_PARTITION as u64));
        group.bench_function("scan/pruned_1_of_8", |bencher| {
            bencher.iter(|| scan_rows(black_box(&scanned), black_box(&pruned)));
        });

        // The bridge's log as one object: read as text records, then parsed
        // and written back as FIX rows into a table on the store. There is no
        // pass between the parse and the commit: a parse settles everything a
        // message derives about itself, so the reader it answers is the one
        // the table takes.
        let mut log = log_object(&store);
        let corpus = LOG.repeat(LOG_REPEATS);
        log.write_all_bytes(&corpus).expect("the log uploads");
        let lines = text_rows(&log);
        assert!(lines > 0, "the log reads as lines");
        probe(
            &store,
            "log/text_read",
            || (),
            |()| {
                text_rows(&log);
            },
        );
        group.throughput(Throughput::Bytes(corpus.len() as u64));
        group.bench_function("log/text_read", |bencher| {
            bencher.iter(|| text_rows(black_box(&log)));
        });

        let registry = Arc::new(
            FixRegistry::from_handle(&LocalFolder::new(seed_root()).expect("a local path"))
                .expect("the tracked seed loads"),
        );
        let codec = FixCodec::new(Arc::clone(&registry));
        let carrier = log.read_arrow_field(&text()).expect("the text field");
        let fixed = fix_schema(&registry, "row").expect("the fixed schema");
        let carried = fix_schema_carrying(&carrier, &fixed).expect("the carried schema");
        // The capture's millisecond stamp is declared at the microsecond
        // resolution Iceberg spells - the write casts it - and the FIX
        // clocks are nanosecond instants, which only a v3 table stores.
        let columns = carried.fields().iter().cloned().map(|mut column| {
            if column.name() == "timestamp" {
                column
                    .set_dtype(
                        DataType::datetime64(TimeUnit::Microsecond, Timezone::UTC)
                            .expect("a microsecond clock"),
                    )
                    .expect("the stamp takes the resolution");
            }
            column
        });
        let mut schema = Field::from_parts(
            carried.name(),
            DataType::from_fields(columns).expect("the columns are distinct"),
            carried.is_nullable(),
            carried.metadata_iter(),
        )
        .expect("the schema rebuilds");
        assign_field_ids(&mut schema, 1).expect("the schema numbers");
        // There is no `timepartition` column: how a layout is cut is the
        // target's, so the table takes an `hour` transform over the
        // `updatedat` the row already carries rather than a materialized copy
        // of that instant.
        let mut spec =
            PartitionSpec::identity(1, &schema, &["updatedat"]).expect("updatedat is a column");
        spec.fields[0].transform = Transform::Hour;
        spec.fields[0].name = "updatedat_hour".into();
        let fix_table = |label: &str| {
            Table::create(
                folder(&store, &next(label)),
                FormatVersion::V3,
                schema.clone(),
                spec.clone(),
            )
            .expect("the FIX table creates")
        };
        let fix_rows = |table: &mut Table<Folder>| {
            let read = log.read_arrow_reader(&text()).expect("a reader");
            let parsed = codec
                .parse_text_arrow_reader(read)
                .expect("the lines parse");
            table.commit_append(parsed).expect("the FIX rows commit");
        };
        probe(
            &store,
            "log/fix_rows_append",
            || fix_table("fix"),
            |mut table| {
                fix_rows(&mut table);
                println!(
                    "s3 log: {lines} lines read, {} FIX rows written back",
                    table.row_size().expect("the row count")
                );
            },
        );
        group.bench_function("log/fix_rows_append", |bencher| {
            bencher.iter_batched(
                || fix_table("fix"),
                |mut table| fix_rows(&mut table),
                BatchSize::PerIteration,
            );
        });
        group.finish();
    }
}

pub(crate) fn benchmarks(criterion: &mut Criterion) {
    #[cfg(feature = "object")]
    s3::benchmarks(criterion);
    plan_benchmarks(criterion);
    metadata_benchmarks(criterion);
    manifest_benchmarks(criterion);
    partition_benchmarks(criterion);
    identity_benchmarks(criterion);
    compact_benchmarks(criterion);
    merge_benchmarks(criterion);
    isolated_merge_benchmarks(criterion);
    read_benchmarks(criterion);
    contended_commit_benchmarks(criterion);
    parallel_commit_benchmarks(criterion);
    catalog_resolve_benchmarks(criterion);
}

pub(crate) fn cleanup() {
    // The planning tables are real directories, so the run removes what it
    // built rather than leaving scratch tables behind.
    for label in SCRATCH_LABELS {
        let _ = std::fs::remove_dir_all(scratch(label));
    }
}
