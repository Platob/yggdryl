//! The Iceberg contract a caller has: expression-driven scans, partition
//! keys as the primary keys, sorted data files, parallel partition writes,
//! and the v3 `unknown` and `variant` types.
//!
//! Everything here reaches the crate through `yggdryl::`; the plan counts and
//! grouping the crate alone can see are pinned in
//! `rust/src/iceberg/tests.rs`.

use std::sync::{Arc, Mutex};

use arrow_array::{
    Array, BinaryArray, Int64Array, NullArray, RecordBatch, StringArray, StructArray,
};
use yggdryl::arrow::BatchReader;
use yggdryl::holder::Holder;
use yggdryl::iceberg::{
    FormatVersion, IcebergOptions, PartitionSpec, PrimitiveType, SortField, SortOrder, Table,
    TableMetadata, Transform, WriteStaging, assign_field_ids, schema_from_json, schema_into_json,
};
use yggdryl::local::Folder;
use yggdryl::media::{IORecordOptions, RecordOptions};
use yggdryl::{DataType, Field, IOBase, IOMedia, Scalar, Selector, StructureType};

/// A table folder that records every relative path resolved through it.
///
/// Every data file a scan opens is one `child_by_path` on the table's root,
/// so the paths recorded here are the files a read or a merge touched.
#[derive(Debug)]
struct Recording {
    inner: Folder,
    seen: Arc<Mutex<Vec<String>>>,
}

impl Recording {
    fn new(path: &std::path::Path) -> Self {
        Self {
            inner: Folder::new(path).unwrap(),
            seen: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn data_files(&self) -> Vec<String> {
        let mut seen: Vec<String> = self
            .seen
            .lock()
            .unwrap()
            .iter()
            .filter(|path| path.starts_with("data/"))
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

impl IOMedia for Recording {
    yggdryl::impl_default_iomedia!();
}

impl IOBase for Recording {
    yggdryl::delegate_iobase!(inner: pread, read_all_bytes, read_range_bytes, pstream_bytes,
        pwrite, size, capacity, reserve, truncate, url, bound_location, mtime, media_type,
        set_media_type, flush, open, opened, close, parent, ls, kind, clear, remove,
        is_atomic, is_tabular, is_io);

    fn child_by_path(&self, path: &str) -> yggdryl::Result<Holder> {
        self.seen.lock().unwrap().push(path.to_owned());
        self.inner.child_by_path(path)
    }
}

/// A scratch directory unique to this test and this process.
fn root(label: &str) -> std::path::PathBuf {
    let mut path = Folder::temporary().unwrap().path().unwrap();
    path.push(format!(
        "yggdryl-iceberg-contract-{label}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    path
}

fn schema() -> Field {
    let mut schema = StructureType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("symbol"),
        DataType::utf8().nullable_field("venue"),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("row");
    assign_field_ids(&mut schema, 1).unwrap();
    schema
}

fn rows(ids: &[i64], symbols: &[&str], venues: &[&str]) -> BatchReader {
    let batch = RecordBatch::try_new(
        schema().into_arrow_schema().unwrap(),
        vec![
            Arc::new(Int64Array::from(ids.to_vec())),
            Arc::new(StringArray::from(symbols.to_vec())),
            Arc::new(StringArray::from(venues.to_vec())),
        ],
    )
    .unwrap();
    yggdryl::arrow::batch_reader(batch.schema(), [batch])
}

/// Every `(id, symbol, venue)` a reader yields, sorted.
fn triples(reader: BatchReader) -> Vec<(i64, String, String)> {
    let mut out = Vec::new();
    for batch in reader {
        let batch = batch.unwrap();
        let ids = batch
            .column_by_name("id")
            .unwrap()
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        let symbols = batch
            .column_by_name("symbol")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let venues = batch
            .column_by_name("venue")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        for row in 0..batch.num_rows() {
            out.push((
                ids.value(row),
                symbols.value(row).to_owned(),
                venues.value(row).to_owned(),
            ));
        }
    }
    out.sort();
    out
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

/// Three venues, three commits, three files.
fn venues(label: &str) -> std::path::PathBuf {
    let path = root(label);
    let schema = schema();
    let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
    let mut table =
        Table::create(Folder::new(&path).unwrap(), FormatVersion::V2, schema, spec).unwrap();
    for (id, symbol, venue) in [
        (1_i64, "AAPL", "XNAS"),
        (2, "MSFT", "XNYS"),
        (3, "VOD", "XLON"),
    ] {
        table
            .commit_append(rows(&[id], &[symbol], &[venue]))
            .unwrap();
    }
    path
}

#[test]
fn a_where_on_a_record_read_prunes_with_the_whole_expression_language() {
    let path = venues("where-pushdown");
    let table = Table::open(Recording::new(&path)).unwrap();

    // The plan is what a read runs: a membership and a range skip manifests
    // exactly as an equality does.
    assert_eq!(
        table
            .plan_matching("venue = 'XNAS'")
            .unwrap()
            .manifests_skipped(),
        2
    );
    assert_eq!(
        table
            .plan_matching("venue in ('XNAS', 'XLON')")
            .unwrap()
            .manifests_skipped(),
        1
    );
    assert_eq!(
        table
            .plan_matching("venue between 'XLON' and 'XNAS'")
            .unwrap()
            .manifests_skipped(),
        1
    );

    // A read through the record options opens only the surviving files.
    let options: RecordOptions = table
        .record_options()
        .unwrap()
        .with_filter("venue in ('XNAS', 'XLON') and id > 0")
        .unwrap();
    table.root().reset();
    let read = triples(table.read_arrow_reader(&options).unwrap());
    assert_eq!(
        read,
        vec![
            (1, "AAPL".to_owned(), "XNAS".to_owned()),
            (3, "VOD".to_owned(), "XLON".to_owned()),
        ]
    );
    let opened = table.root().data_files();
    assert_eq!(opened.len(), 2, "{opened:?}");
    assert!(opened.iter().all(|file| !file.contains("venue=XNYS")));

    // The folder route pushes the same clause down.
    let folder = Folder::new(&path).unwrap();
    let read = triples(folder.read_arrow_reader(&options).unwrap());
    assert_eq!(read.len(), 2);
    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn partition_keys_are_the_primary_keys_of_a_merge() {
    let path = venues("primary-keys");
    let mut table = Table::open(Recording::new(&path)).unwrap();
    let before = file_paths(&table);

    // Keyed: (venue, id). Id 1 updates in XNAS, id 4 appends there, and no
    // other partition's file is opened or rewritten.
    table.root().reset();
    table
        .commit_merge(
            rows(&[1, 4], &["AAPL.O", "NVDA"], &["XNAS", "XNAS"]),
            &Selector::from_columns(["id"]),
            true,
        )
        .unwrap();
    let opened = table.root().data_files();
    assert_eq!(opened.len(), 1, "{opened:?}");
    assert!(opened[0].contains("venue=XNAS"));
    let after = file_paths(&table);
    assert_eq!(before.iter().filter(|p| after.contains(p)).count(), 2);

    // The same id in another partition is another row.
    table
        .commit_merge(
            rows(&[1], &["AAPL.L"], &["XLON"]),
            &Selector::from_columns(["id"]),
            true,
        )
        .unwrap();
    assert_eq!(
        triples(table.scan(None).unwrap()),
        vec![
            (1, "AAPL.L".to_owned(), "XLON".to_owned()),
            (1, "AAPL.O".to_owned(), "XNAS".to_owned()),
            (2, "MSFT".to_owned(), "XNYS".to_owned()),
            (3, "VOD".to_owned(), "XLON".to_owned()),
            (4, "NVDA".to_owned(), "XNAS".to_owned()),
        ]
    );

    // No key: the partition is the key, so XNYS is replaced whole, through
    // the record surface as much as through the commit method.
    let options = table.record_options().unwrap();
    assert!(options.merge_by().is_empty());
    table
        .merge_arrow_reader(rows(&[9], &["MSFT.N"], &["XNYS"]), &options)
        .unwrap();
    assert_eq!(
        triples(table.scan_where(&[("venue", "XNYS")], None).unwrap()),
        vec![(9, "MSFT.N".to_owned(), "XNYS".to_owned())]
    );
    assert_eq!(triples(table.scan(None).unwrap()).len(), 5);

    // Duplicate keys in one write keep the last row.
    table
        .commit_merge(
            rows(&[4, 4], &["first", "last"], &["XNAS", "XNAS"]),
            &Selector::from_columns(["id"]),
            true,
        )
        .unwrap();
    assert!(
        triples(table.scan_where(&[("venue", "XNAS")], None).unwrap()).contains(&(
            4,
            "last".to_owned(),
            "XNAS".to_owned()
        ))
    );
    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn an_unpartitioned_table_still_needs_a_key_and_merges_as_before() {
    let path = root("flat-merge");
    let mut table = Table::create(
        Folder::new(&path).unwrap(),
        FormatVersion::V2,
        schema(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    table
        .commit_append(rows(&[1, 2], &["a", "b"], &["X", "X"]))
        .unwrap();
    let refused = table
        .commit_merge(rows(&[3], &["c"], &["X"]), &Selector::all(), true)
        .unwrap_err()
        .to_string();
    assert!(refused.contains("match key"), "{refused}");
    table
        .commit_merge(
            rows(&[2, 3], &["b2", "c"], &["X", "X"]),
            &Selector::from_columns(["id"]),
            true,
        )
        .unwrap();
    assert_eq!(
        triples(table.scan(None).unwrap()),
        vec![
            (1, "a".to_owned(), "X".to_owned()),
            (2, "b2".to_owned(), "X".to_owned()),
            (3, "c".to_owned(), "X".to_owned()),
        ]
    );
    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn data_files_are_sorted_by_the_default_order_the_metadata_records() {
    let path = root("sorted-files");
    let schema = schema();
    let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
    let table = Table::create(
        Folder::new(&path).unwrap(),
        FormatVersion::V2,
        schema.clone(),
        spec,
    )
    .unwrap();
    // The default: the partition's source column, ascending, nulls first.
    let order = table.metadata().default_sort_order().unwrap();
    assert_eq!(table.metadata().default_sort_order_id(), 1);
    assert_eq!(order.fields.len(), 1);
    assert_eq!(order.fields[0].source_id, 3);
    assert_eq!(order.fields[0].transform, Transform::Identity);
    assert_eq!(order.fields[0].direction, "asc");
    assert_eq!(order.fields[0].null_order, "nulls-first");
    assert_eq!(
        SortOrder::for_spec(&PartitionSpec::unpartitioned()),
        SortOrder::unsorted()
    );

    // An explicit order sorts every file it writes; a one-byte target cuts
    // one row per file, so the bounds march with the sort.
    let by_symbol = root("sorted-by-symbol");
    let mut sorted = Table::create_sorted(
        Folder::new(&by_symbol).unwrap(),
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
    sorted.set_options(
        IcebergOptions::new()
            .try_with_target_file_size_bytes(1)
            .unwrap(),
    );
    sorted
        .commit_append(rows(&[1, 2, 3], &["c", "a", "b"], &["X", "X", "X"]))
        .unwrap();
    let files = sorted.data_files().unwrap();
    assert_eq!(files.len(), 3);
    let lower_bounds: Vec<String> = files
        .iter()
        .map(|(file, _)| {
            assert_eq!(file.sort_order_id, Some(1));
            let (_, bytes) = file.lower_bounds.iter().find(|(id, _)| *id == 2).unwrap();
            String::from_utf8(bytes.clone()).unwrap()
        })
        .collect();
    assert_eq!(lower_bounds, ["a", "b", "c"], "monotone across the files");

    // The order round-trips through the document and the official model.
    let document = sorted.metadata().clone().into_json().unwrap();
    let reread = TableMetadata::from_json(&document).unwrap();
    assert_eq!(reread.default_sort_order().unwrap(), order_by_symbol());
    reread.validate().unwrap();

    // Explicitly unsorted: rows stay as they arrived, files carry no order.
    let plain = root("unsorted");
    let mut unsorted = Table::create_sorted(
        Folder::new(&plain).unwrap(),
        FormatVersion::V2,
        schema.clone(),
        PartitionSpec::identity(1, &schema, &["venue"]).unwrap(),
        SortOrder::unsorted(),
    )
    .unwrap();
    assert_eq!(unsorted.metadata().default_sort_order_id(), 0);
    unsorted
        .commit_append(rows(&[3, 1, 2], &["c", "a", "b"], &["X", "X", "X"]))
        .unwrap();
    let ids: Vec<i64> = unsorted
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
    assert_eq!(ids, [3, 1, 2]);
    assert_eq!(unsorted.data_files().unwrap()[0].0.sort_order_id, None);

    let _ = std::fs::remove_dir_all(&path);
    let _ = std::fs::remove_dir_all(&by_symbol);
    let _ = std::fs::remove_dir_all(&plain);
}

fn order_by_symbol() -> &'static SortOrder {
    use std::sync::OnceLock;
    static ORDER: OnceLock<SortOrder> = OnceLock::new();
    ORDER.get_or_init(|| SortOrder {
        order_id: 1,
        fields: vec![SortField {
            source_id: 2,
            transform: Transform::Identity,
            direction: "asc".into(),
            null_order: "nulls-last".into(),
        }],
    })
}

#[test]
fn write_parallelism_round_trips_and_a_parallel_commit_lists_files_in_group_order() {
    let path = root("write-parallelism");
    let schema = schema();
    let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
    let mut table =
        Table::create(Folder::new(&path).unwrap(), FormatVersion::V2, schema, spec).unwrap();

    // The property layer, the explicit layer, and the default.
    assert_eq!(
        table.options().unwrap().write_parallelism(),
        table.options().unwrap().read_parallelism()
    );
    table
        .commit_metadata_changes(|metadata| {
            metadata.set_property(IcebergOptions::WRITE_PARALLELISM_KEY, "3")?;
            Ok(())
        })
        .unwrap();
    assert_eq!(table.options().unwrap().write_parallelism(), 3);
    assert_eq!(
        IcebergOptions::from_metadata(table.metadata())
            .unwrap()
            .write_parallelism_option(),
        Some(3)
    );
    table.set_options(IcebergOptions::new().try_with_write_parallelism(4).unwrap());
    assert_eq!(table.options().unwrap().write_parallelism(), 4);
    assert!(IcebergOptions::new().set_write_parallelism(0).is_err());

    // Eight partitions on four threads: the manifest lists v1 through v8.
    table
        .commit_append(rows(
            &[1, 2, 3, 4, 5, 6, 7, 8],
            &["a"; 8],
            &["v1", "v2", "v3", "v4", "v5", "v6", "v7", "v8"],
        ))
        .unwrap();
    let venues: Vec<String> = table
        .data_files()
        .unwrap()
        .into_iter()
        .map(|(file, _)| file.partition[0].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(venues, ["v1", "v2", "v3", "v4", "v5", "v6", "v7", "v8"]);
    assert_eq!(triples(table.scan(None).unwrap()).len(), 8);
    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn write_staging_round_trips_and_a_staged_commit_leaves_no_local_file() {
    let path = root("write-staging");
    let stage = root("write-staging-folder");
    let schema = schema();
    let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
    let mut table =
        Table::create(Folder::new(&path).unwrap(), FormatVersion::V2, schema, spec).unwrap();

    // The spellings: `off`, a local folder URL, a local path; never a
    // remote folder, because a staging file is a local file.
    assert_eq!(WriteStaging::from_str("off").unwrap(), WriteStaging::Off);
    let folder = WriteStaging::from_str(&stage.to_string_lossy()).unwrap();
    assert!(folder.folder().unwrap().is_local());
    assert_eq!(
        folder.to_string().parse::<WriteStaging>().unwrap(),
        folder,
        "the text round-trips"
    );
    let refused = WriteStaging::from_str("s3://trades/stage")
        .unwrap_err()
        .to_string();
    assert!(refused.contains("write.staging"), "{refused}");

    // The default is the root's own: a local table stages nothing, and the
    // options value alone says nothing until a layer speaks.
    assert_eq!(IcebergOptions::new().write_staging(), None);
    assert_eq!(table.write_staging().unwrap(), WriteStaging::Off);

    // The property layer, then the explicit layer, each read back exactly.
    table
        .commit_metadata_changes(|metadata| {
            metadata.set_property(IcebergOptions::WRITE_STAGING_KEY, folder.to_string())?;
            Ok(())
        })
        .unwrap();
    assert_eq!(table.write_staging().unwrap(), folder);
    assert_eq!(
        IcebergOptions::from_metadata(table.metadata())
            .unwrap()
            .write_staging(),
        Some(&folder)
    );
    table
        .commit_metadata_changes(|metadata| {
            metadata.set_property(IcebergOptions::WRITE_STAGING_KEY, "s3://trades/stage")?;
            Ok(())
        })
        .unwrap();
    let message = table.write_staging().unwrap_err().to_string();
    assert!(message.contains("write.staging"), "{message}");
    table.set_options(
        IcebergOptions::new()
            .try_with_write_staging(folder.clone())
            .unwrap(),
    );
    assert_eq!(table.write_staging().unwrap(), folder);

    // A staged commit reads back as any other, and the staging folder holds
    // nothing once it is done: every staged file went out and was removed.
    table
        .commit_append(rows(&[1, 2, 3], &["a", "b", "c"], &["v1", "v2", "v3"]))
        .unwrap();
    assert_eq!(triples(table.scan(None).unwrap()).len(), 3);
    assert_eq!(file_paths(&table).len(), 3);
    assert!(
        !stage.exists() || std::fs::read_dir(&stage).unwrap().next().is_none(),
        "the staging folder is empty after the commit"
    );
    table.set_options(
        IcebergOptions::new()
            .try_with_write_staging(WriteStaging::Off)
            .unwrap(),
    );
    assert_eq!(table.write_staging().unwrap(), WriteStaging::Off);
    let _ = std::fs::remove_dir_all(&path);
    let _ = std::fs::remove_dir_all(&stage);
}

#[test]
fn unknown_is_the_null_column_and_variant_is_the_semi_structured_one() {
    // The type mapping, both directions, and why the two are not one thing:
    // `unknown` has no values, `variant` has values that carry their type.
    assert_eq!(
        PrimitiveType::from_str("unknown")
            .unwrap()
            .into_dtype()
            .unwrap(),
        DataType::Null
    );
    assert_eq!(
        PrimitiveType::from_str("variant")
            .unwrap()
            .into_dtype()
            .unwrap(),
        DataType::Variant
    );
    assert_eq!(
        PrimitiveType::from_dtype(&DataType::Null)
            .unwrap()
            .to_string(),
        "unknown"
    );
    assert_eq!(
        PrimitiveType::from_dtype(&DataType::Variant)
            .unwrap()
            .to_string(),
        "variant"
    );

    let document: Scalar = yggdryl::json::from_utf8(
        r#"{"type":"struct","schema-id":0,"fields":[
            {"id":1,"name":"id","required":true,"type":"long"},
            {"id":2,"name":"later","required":false,"type":"unknown"},
            {"id":3,"name":"payload","required":false,"type":"variant"}
        ]}"#,
    )
    .unwrap();
    let schema = schema_from_json("row", &document).unwrap();
    assert_eq!(schema.fields()[1].dtype(), &DataType::Null);
    assert_eq!(schema.fields()[2].dtype(), &DataType::Variant);
    assert_eq!(schema_into_json(&schema).unwrap(), document);

    // A v2 table refuses either by name.
    let v2 = root("v3-types-v2");
    let message = Table::create(
        Folder::new(&v2).unwrap(),
        FormatVersion::V2,
        schema.clone(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap_err()
    .to_string();
    assert!(
        message.contains("later") && message.contains("unknown"),
        "{message}"
    );

    // A v3 table stores the variant, omits the unknown, and reads both back.
    let v3 = root("v3-types-v3");
    let mut table = Table::create(
        Folder::new(&v3).unwrap(),
        FormatVersion::V3,
        schema.clone(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    let arrow = schema.into_arrow_schema().unwrap();
    let arrow_schema::DataType::Struct(children) = arrow.field(2).data_type() else {
        panic!("a variant lays out as a struct");
    };
    let payload = StructArray::try_new(
        children.clone(),
        vec![
            Arc::new(BinaryArray::from_iter_values([[0x01_u8, 0x00, 0x00]; 2])),
            Arc::new(BinaryArray::from_iter_values([[0x00_u8], [0x0c]])),
        ],
        None,
    )
    .unwrap();
    let batch = RecordBatch::try_new(
        Arc::clone(&arrow),
        vec![
            Arc::new(Int64Array::from(vec![1, 2])),
            Arc::new(NullArray::new(2)),
            Arc::new(payload),
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
    assert_eq!(read.column(1).logical_null_count(), 2);

    let reopened = Table::open(Folder::new(&v3).unwrap()).unwrap();
    let stored = reopened.schema().unwrap();
    assert_eq!(stored.fields()[1].dtype(), &DataType::Null);
    assert_eq!(stored.fields()[2].dtype(), &DataType::Variant);
    reopened.metadata().validate().unwrap();

    let _ = std::fs::remove_dir_all(&v2);
    let _ = std::fs::remove_dir_all(&v3);
}
