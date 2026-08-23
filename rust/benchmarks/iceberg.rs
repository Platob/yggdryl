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
use criterion::{BatchSize, Criterion, Throughput, criterion_group};
use smol_str::SmolStr;
use yggdryl::iceberg::{
    CommitConflict, Compaction, DataFile, FieldSummary, FormatVersion, IcebergOptions,
    ManifestContent, ManifestEntry, ManifestFile, PartitionSpec, ScanPlan, ScanTask, Snapshot,
    SnapshotRef, SortField, SortOrder, Table, TableMetadata, Transform, assign_field_ids,
    read_manifest, read_manifest_for_plan, write_manifest,
};
use yggdryl::io::partition::partition_text;
use yggdryl::io::{Buffer, IOBase};
use yggdryl::local::Folder;
use yggdryl::{DataType, Field, MediaType, MimeType, Value};

/// Distinct venue values the planning tables partition on.
const VENUES: usize = 8;

/// The filter the pruned plan asks for: one of the eight venue values.
const PRUNED_FILTER: (&str, &str) = ("venue", "venue-2");

/// The scratch labels the benchmark tables live under, cleaned at exit.
const SCRATCH_LABELS: [&str; 6] = [
    "files-10",
    "files-200",
    "compact-200",
    "merge-50",
    "read-parallel-32",
    "commit-contended",
];

/// Spell one of the [`VENUES`] partition values.
fn venue(index: usize) -> String {
    format!("venue-{index}")
}

/// Build a scratch directory unique to this benchmark run.
fn scratch(label: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
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
        DataType::Utf8.nullable_field("venue"),
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
            .append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
            .expect("the append commits");
    }
    table
}

/// Scan planning cost as the file count grows, and what pruning saves.
fn plan_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("plan");
    let small = plan_table(SCRATCH_LABELS[0], 10);
    let large = plan_table(SCRATCH_LABELS[1], 200);

    // Proven once outside the timers, so no bench can silently measure an
    // empty table or a filter that prunes nothing.
    let whole = large.plan(&[]).expect("the whole-table plan reads");
    assert_eq!(whole.tasks.len(), 200, "the large table holds 200 files");
    assert_eq!(whole.manifests_read, 100, "one manifest per commit");
    let pruned = large.plan(&[PRUNED_FILTER]).expect("the pruned plan reads");
    assert!(pruned.manifests_skipped() > 0, "summaries must prune");
    assert!(pruned.files_skipped() > 0, "partition tuples must prune");

    group.bench_function("files_10", |bencher| {
        bencher.iter(|| black_box(&small).plan(&[]).expect("the small plan reads"));
    });
    group.bench_function("files_200", |bencher| {
        bencher.iter(|| black_box(&large).plan(&[]).expect("the large plan reads"));
    });
    // The full side of this comparison is `files_200` above: same table, same
    // snapshot, no filter. What this one adds is the summary check per
    // manifest-list row against what it saves - three quarters of the Avro
    // manifests never opened.
    group.bench_function("pruned_vs_full_200", |bencher| {
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
    for index in 1..=100_i64 {
        metadata.set_current_snapshot(Snapshot {
            snapshot_id: index,
            parent_snapshot_id: (index > 1).then(|| index - 1),
            sequence_number: Some(index),
            timestamp_ms: 1_700_000_000_000 + index,
            manifest_list: SmolStr::new(format!("file:///bench/table/metadata/snap-{index}.avro")),
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
            first_row_id: None,
            added_rows: None,
        });
    }
    metadata
}

/// `TableMetadata::from_json` throughput over a long-lived table's document.
fn metadata_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("metadata");
    let document = synthesized_metadata()
        .into_json()
        .expect("the synthetic metadata projects to JSON");
    let text = String::from_utf8(
        yggdryl::json::into_bytes(&document).expect("the synthetic document encodes"),
    )
    .expect("the encoded document is UTF-8");

    // Proven once outside the timer: the text really carries the shape the
    // benchmark claims to parse.
    let parsed =
        TableMetadata::from_json(&yggdryl::json::from_utf8(&text).expect("the text parses"))
            .expect("the document reads back");
    assert_eq!(parsed.snapshots.len(), 100);
    assert_eq!(parsed.schemas.len(), 3);
    assert_eq!(parsed.snapshot_log.len(), 100);

    group.throughput(Throughput::Bytes(text.len() as u64));
    group.bench_function("parse_json", |bencher| {
        bencher.iter(|| {
            let value = yggdryl::json::from_utf8(black_box(text.as_str()))
                .expect("the serialized document parses");
            TableMetadata::from_json(&value).expect("the parsed document reads")
        });
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
                    partition: vec![Value::from(name.as_str())],
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
    let entries = manifest_entries(1_000);
    let mut buffer = Buffer::new();
    buffer.set_media_type(MediaType::new(MimeType::AVRO));
    write_manifest(&mut buffer, FormatVersion::V2, &schema, &spec, &entries)
        .expect("the synthetic manifest encodes");

    // Proven once outside the timer: the container really holds the entries.
    assert_eq!(
        read_manifest(&buffer)
            .expect("the manifest reads back")
            .len(),
        1_000
    );

    group.throughput(Throughput::Elements(1_000));
    group.bench_function("decode_1000", |bencher| {
        bencher.iter(|| read_manifest(black_box(&buffer)).expect("the manifest decodes"));
    });

    // The planning fast path against the full decode, at manifest scale. The
    // filtered variant keeps counts and bounds - the lazy statistics a
    // filtered plan consults - and the unfiltered one skips even those.
    for scale in [1_000_usize, 10_000, 100_000] {
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
    }
    group.finish();
}

/// The single-value renderer every partition directory name goes through.
fn partition_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("partition");
    let date = Value::date32(19_723);
    let text = Value::from("XNAS");

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
    let snapshot = metadata.snapshots[0].clone();
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
        added_files_count: 1,
        existing_files_count: 0,
        deleted_files_count: 0,
        added_rows_count: data.record_count,
        existing_rows_count: 0,
        deleted_rows_count: 0,
        partitions: vec![summary.clone()],
        first_row_id: None,
    };
    let snapshot_ref = SnapshotRef::branch(snapshot.snapshot_id);
    let compaction = Compaction {
        files_before: 8,
        files_after: 2,
        bytes_rewritten: 32_768,
    };
    let options = IcebergOptions::new().with_commit_retries(2);
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
    let mut table = plan_table(SCRATCH_LABELS[2], 200);

    // Proven once outside the timer: the fold really happened, so the plan
    // being measured reads 8 files where the uncompacted table read 200.
    let compaction = table.compact().expect("the table compacts");
    assert_eq!(compaction.files_before, 200, "every small file rewrites");
    assert_eq!(compaction.files_after, VENUES, "one merged file per venue");
    let plan = table.plan(&[]).expect("the compacted plan reads");
    assert_eq!(plan.tasks.len(), VENUES);

    group.bench_function("plan_after_compact_200", |bencher| {
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
            .append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
            .expect("the append commits");
    }
    table
}

/// Upserting ten keyed rows into a table of fifty single-row files.
fn merge_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("merge");
    let mut table = merge_table(SCRATCH_LABELS[3], 50);
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
    let merge_by_names = vec!["id".to_owned()];

    // Proven once outside the timer, which also settles the table into the
    // steady state every measured merge sees: the ten matched single-row files
    // fold into one and the other forty are carried untouched, so an upsert of
    // stored keys adds no row and every later merge rewrites that one file.
    table
        .merge(
            yggdryl::arrow::batch_reader(upsert.schema(), [upsert.clone()]),
            &merge_by_names,
            true,
        )
        .expect("the priming merge commits");
    let plan = table.plan(&[]).expect("the merged table plans");
    assert_eq!(
        plan.record_count(),
        50,
        "an upsert of stored keys adds no row"
    );
    assert_eq!(plan.tasks.len(), 41, "ten matched files fold into one");

    group.bench_function("upsert_into_50_files", |bencher| {
        bencher.iter(|| {
            table
                .merge(
                    yggdryl::arrow::batch_reader(upsert.schema(), [upsert.clone()]),
                    black_box(&merge_by_names),
                    true,
                )
                .expect("the merge commits");
        });
    });
    group.finish();
}

/// The four-column trade schema the read benchmark scans.
fn read_schema() -> Field {
    let mut schema = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Float64.nullable_field("price"),
        DataType::Utf8.nullable_field("venue"),
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
            .append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
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
    let mut table = read_table(SCRATCH_LABELS[4], 32, 100_000);

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
    assert_eq!(scan_rows(&table), 3_200_000);
    table.set_options(parallel.clone());
    assert_eq!(scan_rows(&table), 3_200_000);

    group.throughput(Throughput::Elements(3_200_000));
    table.set_options(sequential);
    group.bench_function("parallel_vs_sequential_32x4mb/parallelism-1", |bencher| {
        bencher.iter(|| scan_rows(black_box(&table)));
    });
    table.set_options(parallel);
    group.bench_function("parallel_vs_sequential_32x4mb/parallelism-4", |bencher| {
        bencher.iter(|| scan_rows(black_box(&table)));
    });
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
/// memory mapping, and `yggdryl::local` documents the consequence: two
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
                                .append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
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
    use std::sync::atomic::{AtomicUsize, Ordering};
    use yggdryl::arrowfs::{ArrowFileSystem, FileInfo, FileInfos, MemoryFileSystem};
    use yggdryl::iceberg::Catalog;

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

    impl ArrowFileSystem for Counting {
        fn type_name(&self) -> &str {
            self.inner.type_name()
        }

        fn file_info(&self, path: &str) -> yggdryl::Result<FileInfo> {
            self.count();
            self.inner.file_info(path)
        }

        fn list(&self, path: &str, recursive: bool) -> FileInfos {
            self.count();
            self.inner.list(path, recursive)
        }

        fn read_range(&self, path: &str, offset: u64, buffer: &mut [u8]) -> yggdryl::Result<usize> {
            self.count();
            self.inner.read_range(path, offset, buffer)
        }

        fn write_full(&self, path: &str, bytes: &[u8]) -> yggdryl::Result<()> {
            self.count();
            self.inner.write_full(path, bytes)
        }

        fn create_dir(&self, path: &str) -> yggdryl::Result<()> {
            self.count();
            self.inner.create_dir(path)
        }

        fn delete_file(&self, path: &str) -> yggdryl::Result<()> {
            self.count();
            self.inner.delete_file(path)
        }
    }

    let schema = || {
        DataType::from_fields([DataType::Int64.required_field("id")])
            .expect("a valid struct root")
            .required_field("row")
    };
    let counted = || {
        let filesystem = Arc::new(Counting::default());
        let warehouse = yggdryl::arrowfs::Folder::from_location(
            Arc::clone(&filesystem) as Arc<dyn ArrowFileSystem>,
            "warehouse",
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

criterion_group!(
    iceberg,
    plan_benchmarks,
    metadata_benchmarks,
    manifest_benchmarks,
    partition_benchmarks,
    identity_benchmarks,
    compact_benchmarks,
    merge_benchmarks,
    read_benchmarks,
    contended_commit_benchmarks,
    catalog_resolve_benchmarks
);

fn main() {
    iceberg();
    Criterion::default().configure_from_args().final_summary();
    // The planning tables are real directories, so the run removes what it
    // built rather than leaving scratch tables behind.
    for label in SCRATCH_LABELS {
        let _ = std::fs::remove_dir_all(scratch(label));
    }
}
