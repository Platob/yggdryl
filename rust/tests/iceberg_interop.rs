//! Iceberg tables written here, and Iceberg tables written by someone else.
//!
//! Self-consistency proves nothing about a table format: a reader and a writer
//! that agree with each other can still agree on something no other
//! implementation accepts. This target therefore writes a table into
//! `target/iceberg-interop/from-rust` for an external reader to check, and
//! reads whatever an external writer left in
//! `target/iceberg-interop/from-pyiceberg`.
//!
//! `python scripts/check_iceberg_interop.py` is the driver that runs both
//! sides. Run alone, the external half is skipped and says so on stdout, which
//! is what the driver checks for.

#![cfg(feature = "iceberg")]

use std::sync::Arc;

use arrow_array::{Array, Int64Array, RecordBatch, StringArray};
use yggdryl::generic::IORecordOptions;
use yggdryl::iceberg::{EntryStatus, FormatVersion, PartitionSpec, Table, assign_field_ids};
use yggdryl::io::IOBase;
use yggdryl::local::Folder;
use yggdryl::{DataType, Field};

/// The directory both halves of the exchange live under.
fn interop_root() -> std::path::PathBuf {
    // `CARGO_MANIFEST_DIR` is `rust/`; the workspace target directory is beside it.
    let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.pop();
    path.push("target");
    path.push("iceberg-interop");
    path
}

/// The schema both sides agree on, numbered the way Iceberg numbers one.
fn schema() -> Field {
    let mut schema = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
        DataType::Utf8.nullable_field("venue"),
    ])
    .expect("a struct datatype")
    .required_field("row");
    assign_field_ids(&mut schema, 1).expect("field identifiers");
    schema
        .insert_metadata("iceberg:schema-id", "0")
        .expect("a schema id");
    schema
}

/// The rows both sides exchange, including a null partition value.
fn rows() -> RecordBatch {
    RecordBatch::try_new(
        schema().to_arrow_schema().expect("an Arrow schema"),
        vec![
            Arc::new(Int64Array::from(vec![1_i64, 2, 3, 4])),
            Arc::new(StringArray::from(vec![
                Some("AAPL"),
                Some("MSFT"),
                Some("AAPL"),
                None,
            ])),
            Arc::new(StringArray::from(vec![
                Some("XNAS"),
                Some("XNYS"),
                Some("XNAS"),
                None,
            ])),
        ],
    )
    .expect("a batch")
}

/// The rows the upsert half brings: one update and one new key.
///
/// Their identifiers are deliberately above the ones the first two partitions
/// hold, so the merge's file selection has something to exclude and PyIceberg
/// has carried-over `existing` manifest entries to read back.
fn upserted() -> RecordBatch {
    RecordBatch::try_new(
        schema().to_arrow_schema().expect("an Arrow schema"),
        vec![
            Arc::new(Int64Array::from(vec![4_i64, 5])),
            Arc::new(StringArray::from(vec![Some("GOOG"), Some("BP")])),
            Arc::new(StringArray::from(vec![None, Some("XLON")])),
        ],
    )
    .expect("a batch")
}

/// Collect a scan as sorted `(id, symbol, venue)` triples.
fn collect(reader: yggdryl::arrow::BatchReader) -> Vec<(i64, Option<String>, Option<String>)> {
    let mut collected = Vec::new();
    for batch in reader {
        let batch = batch.expect("a batch");
        let ids = batch
            .column_by_name("id")
            .expect("an id column")
            .as_any()
            .downcast_ref::<Int64Array>()
            .expect("an int64 id column")
            .clone();
        let text = |name: &str| {
            batch.column_by_name(name).map(|column| {
                column
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .expect("a utf8 column")
                    .clone()
            })
        };
        let symbols = text("symbol");
        let venues = text("venue");
        for row in 0..batch.num_rows() {
            collected.push((
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
    collected.sort();
    collected
}

/// The rows an external reader must find, after the append and the upsert.
fn expected() -> Vec<(i64, Option<String>, Option<String>)> {
    vec![
        (1, Some("AAPL".to_owned()), Some("XNAS".to_owned())),
        (2, Some("MSFT".to_owned()), Some("XNYS".to_owned())),
        (3, Some("AAPL".to_owned()), Some("XNAS".to_owned())),
        (4, Some("GOOG".to_owned()), None),
        (5, Some("BP".to_owned()), Some("XLON".to_owned())),
    ]
}

/// The rows the first append leaves, before the upsert changes one of them.
fn appended() -> Vec<(i64, Option<String>, Option<String>)> {
    vec![
        (1, Some("AAPL".to_owned()), Some("XNAS".to_owned())),
        (2, Some("MSFT".to_owned()), Some("XNYS".to_owned())),
        (3, Some("AAPL".to_owned()), Some("XNAS".to_owned())),
        (4, None, None),
    ]
}

#[test]
fn a_table_written_here_is_left_for_an_external_reader() {
    let path = interop_root().join("from-rust");
    let _ = std::fs::remove_dir_all(&path);

    let schema = schema();
    let spec = PartitionSpec::identity(1, &schema, &["venue"]).expect("a partition spec");
    let mut table = Table::create(
        Folder::new(&path).expect("a folder"),
        FormatVersion::V2,
        schema,
        spec,
    )
    .expect("a created table");

    let batch = rows();
    table
        .append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
        .expect("an appended snapshot");
    assert_eq!(collect(table.scan(None).expect("a scan")), appended());

    // The upsert goes through the plain record surface, because that is what an
    // external reader is being asked to validate: a folder handle, a match key,
    // and one snapshot whose manifests carry both the files it rewrote and the
    // files its statistics said it could leave alone.
    let mut folder = Folder::new(&path).expect("a folder");
    let options = folder
        .record_options()
        .expect("the table's own encoding")
        .with_merge_by_names(["id"]);
    let batch = upserted();
    folder
        .write_arrow_batch_reader(
            yggdryl::arrow::batch_reader(batch.schema(), [batch]),
            &options,
        )
        .expect("a merged snapshot");

    // Reading it back here is the floor, not the proof.
    let table = Table::open(Folder::new(&path).expect("a folder")).expect("the merged table");
    assert_eq!(collect(table.scan(None).expect("a scan")), expected());
    let plan = table.plan(&[]).expect("a plan");
    assert_eq!(
        plan.tasks
            .iter()
            .filter(|task| task.entry.status == EntryStatus::Existing)
            .count(),
        2,
        "the files the merge did not have to read were carried over"
    );
    println!(
        "iceberg-interop: wrote {}",
        table.metadata_location().expect("a metadata location")
    );
}

#[test]
fn a_table_written_by_pyiceberg_reads_here() {
    let path = interop_root().join("from-pyiceberg");
    if !path.join("metadata").is_dir() {
        // Say it loudly rather than passing on an empty directory.
        println!(
            "iceberg-interop: SKIPPED the external table; {} is not there. Run `python \
             scripts/check_iceberg_interop.py` to produce it.",
            path.display()
        );
        return;
    }

    let table = Table::open(Folder::new(&path).expect("a folder")).expect("an external table");
    let metadata = table.metadata();
    assert!(
        metadata.format_version >= FormatVersion::V1,
        "an external table declares a format version"
    );
    let snapshot = table
        .current_snapshot()
        .expect("an external table with a snapshot");
    assert!(!snapshot.manifest_list.is_empty());

    let schema = table.schema().expect("an external schema");
    assert_eq!(schema.field_len(), 3);
    assert_eq!(
        schema.fields()[0].parquet_field_id().expect("an id"),
        Some(1)
    );

    let files = table.data_files().expect("external data files");
    assert!(!files.is_empty(), "an external table names its data files");

    // PyIceberg writes the rows the *first* half of the exchange appended; the
    // upsert is this crate's own and is checked on the table it wrote.
    assert_eq!(collect(table.scan(None).expect("a scan")), appended());

    // Planning against someone else's metadata is the real test of the pruning:
    // the partition tuples and the column bounds below were written by
    // PyIceberg, so a file skipped here is a file *their* statistics excluded.
    let partitioned = table
        .plan(&[("venue", "XNAS")])
        .expect("a partition-filtered plan");
    assert_eq!(partitioned.tasks.len(), 1, "one venue, one file");
    assert_eq!(
        collect(
            table
                .scan_where(&[("venue", "XNAS")], None)
                .expect("a filtered scan")
        ),
        vec![
            (1, Some("AAPL".to_owned()), Some("XNAS".to_owned())),
            (3, Some("AAPL".to_owned()), Some("XNAS".to_owned())),
        ]
    );

    let bounded = table
        .plan(&[("symbol", "MSFT")])
        .expect("a statistics-filtered plan");
    assert_eq!(
        bounded.tasks.len(),
        1,
        "the other files' symbol bounds cannot hold MSFT: {} of {} were skipped",
        bounded.files_skipped(),
        files.len()
    );
    assert_eq!(
        collect(
            table
                .scan_where(&[("symbol", "MSFT")], None)
                .expect("a filtered scan")
        ),
        vec![(2, Some("MSFT".to_owned()), Some("XNYS".to_owned()))]
    );

    println!(
        "iceberg-interop: read {} external data files, and its own statistics skipped {} of them",
        files.len(),
        bounded.files_skipped()
    );
}

#[test]
fn tables_of_the_other_format_versions_are_left_for_an_external_reader() {
    for (version, name) in [
        (FormatVersion::V1, "from-rust-v1"),
        (FormatVersion::V3, "from-rust-v3"),
    ] {
        let path = interop_root().join(name);
        let _ = std::fs::remove_dir_all(&path);
        let schema = schema();
        let spec = PartitionSpec::identity(1, &schema, &["venue"]).expect("a partition spec");
        let mut table = Table::create(Folder::new(&path).expect("a folder"), version, schema, spec)
            .expect("a created table");
        let batch = rows();
        table
            .append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
            .expect("an appended snapshot");
        assert_eq!(collect(table.scan(None).expect("a scan")), appended());
        println!("iceberg-interop: wrote {name}");
    }
}

#[test]
fn tables_written_by_pyiceberg_at_other_versions_read_here() {
    // The driver knows which versions PyIceberg managed to write, so absence
    // is reported per version rather than failing the standalone run.
    for name in ["from-pyiceberg-v1", "from-pyiceberg-v3"] {
        let path = interop_root().join(name);
        if !path.join("metadata").is_dir() {
            println!("iceberg-interop: absent {name}");
            continue;
        }
        let table = Table::open(Folder::new(&path).expect("a folder")).expect("an external table");
        // The rows come back through this crate's manifest reader, so the
        // exchange covers the per-version manifest schemas both ways.
        assert_eq!(collect(table.scan(None).expect("a scan")), appended());
        let plan = table.plan(&[("venue", "XNAS")]).expect("a filtered plan");
        assert_eq!(plan.tasks.len(), 1, "one venue, one file in {name}");
        println!("iceberg-interop: read {name}");
    }
}

/// Leave a wide deterministic manifest for the baseline readers.
///
/// `scripts/bench_avro_baseline.py` times fastavro and PyIceberg's manifest
/// reader over this exact file, so the numbers in `docs/avro.md`
/// compare implementations on identical bytes.
#[test]
fn a_large_manifest_is_left_for_baseline_readers() {
    use yggdryl::iceberg::{DataFile, ManifestEntry, write_manifest};

    let dir = interop_root();
    std::fs::create_dir_all(&dir).expect("the exchange directory");
    let schema = schema();
    let spec = PartitionSpec::identity(1, &schema, &["venue"]).expect("a partition spec");
    let entries: Vec<ManifestEntry> = (0..10_000)
        .map(|index: i64| {
            ManifestEntry::added(
                7_001,
                DataFile {
                    file_path: format!("file:///bench/data/part-{index:05}.parquet").into(),
                    partition: vec![yggdryl::Value::from(["XNAS", "XNYS"][index as usize % 2])],
                    record_count: 100 + index,
                    file_size_in_bytes: 4_096,
                    column_sizes: vec![(1, 512), (2, 256), (3, 128)],
                    value_counts: vec![(1, 100), (2, 90), (3, 80)],
                    null_value_counts: vec![(1, 0), (2, 10), (3, 20)],
                    nan_value_counts: vec![(1, 0)],
                    lower_bounds: vec![(1, index.to_le_bytes().to_vec())],
                    upper_bounds: vec![(1, (index + 100).to_le_bytes().to_vec())],
                    split_offsets: vec![4],
                    sort_order_id: Some(0),
                    ..DataFile::default()
                },
            )
        })
        .collect();
    let mut handle =
        yggdryl::local::File::new(dir.join("manifest-10k.avro")).expect("a file handle");
    write_manifest(&mut handle, FormatVersion::V2, &schema, &spec, &entries)
        .expect("the baseline manifest writes");
    println!("iceberg-interop: wrote manifest-10k.avro");
}

/// Time this crate's readers over the baseline manifest, for the comparison
/// table in `docs/avro.md`.
///
/// Gated behind `YGGDRYL_BASELINE_TIMING` because a timing only means
/// something in a release build on a quiet machine; the baseline script sets
/// the variable and runs this with `--release`.
#[test]
fn times_the_baseline_manifest_for_the_comparison_table() {
    use yggdryl::iceberg::{read_manifest, read_manifest_for_plan};

    if std::env::var_os("YGGDRYL_BASELINE_TIMING").is_none() {
        return;
    }
    let path = interop_root().join("manifest-10k.avro");
    assert!(path.exists(), "run a_large_manifest first");
    let handle = yggdryl::local::File::new(&path).expect("a file handle");

    let best = |action: &dyn Fn() -> usize| -> f64 {
        let mut fastest = f64::INFINITY;
        for _ in 0..7 {
            let started = std::time::Instant::now();
            assert_eq!(action(), 10_000);
            fastest = fastest.min(started.elapsed().as_secs_f64());
        }
        fastest * 1e3
    };
    let full = best(&|| read_manifest(&handle).expect("the manifest reads").len());
    let stats = best(&|| {
        read_manifest_for_plan(&handle, true)
            .expect("the plan path reads")
            .len()
    });
    let identity = best(&|| {
        read_manifest_for_plan(&handle, false)
            .expect("the plan path reads")
            .len()
    });
    println!(
        "iceberg-interop: timed full={full:.1}ms plan_stats={stats:.1}ms \
         plan_identity={identity:.1}ms"
    );
}
