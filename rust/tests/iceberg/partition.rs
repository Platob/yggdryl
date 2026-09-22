//! `rust/src/iceberg/partition.rs`: what a partition document must spell.
//!
//! A spec read from someone else's catalog is only as safe as the reading is
//! strict, so these pin the identifiers, the one synthesizing rule v1 allows,
//! and the duplicates a spec refuses. All of it is `yggdryl::iceberg` API.

use yggdryl::iceberg::{PartitionField, PartitionSpec};

#[test]
fn modern_partition_json_requires_exact_identifiers() {
    for text in [
        r#"{"name":"day","source-id":1,"transform":"identity"}"#,
        r#"{"name":"day","source-id":1,"field-id":2147483648,"transform":"identity"}"#,
    ] {
        let document = yggdryl::json::from_utf8(text).unwrap();
        assert!(PartitionField::from_json(&document).is_err(), "{text}");
    }
    for text in [
        r#"{"fields":[]}"#,
        r#"{"spec-id":2147483648,"fields":[]}"#,
        r#"{"spec-id":1,"fields":[{"name":"day","source-id":1,"transform":"identity"}]}"#,
    ] {
        let document = yggdryl::json::from_utf8(text).unwrap();
        assert!(PartitionSpec::from_json(&document).is_err(), "{text}");
    }
}

#[test]
fn only_the_v1_array_synthesizes_field_ids() {
    let document =
        yggdryl::json::from_utf8(r#"[{"name":"day","source-id":1,"transform":"identity"}]"#)
            .unwrap();
    let spec = PartitionSpec::from_json(&document).unwrap();
    assert_eq!(spec.spec_id, 0);
    assert_eq!(spec.fields[0].field_id, 1000);
}

#[test]
fn a_spec_rejects_duplicate_field_ids_and_names() {
    for fields in [
        r#"[{"name":"a","source-id":1,"field-id":1000,"transform":"identity"},{"name":"b","source-id":2,"field-id":1000,"transform":"identity"}]"#,
        r#"[{"name":"a","source-id":1,"field-id":1000,"transform":"identity"},{"name":"a","source-id":2,"field-id":1001,"transform":"identity"}]"#,
    ] {
        let document =
            yggdryl::json::from_utf8(&format!(r#"{{"spec-id":1,"fields":{fields}}}"#)).unwrap();
        assert!(PartitionSpec::from_json(&document).is_err(), "{fields}");
    }
}

mod iceberg {
    use arrow_array::{Array, Int64Array, RecordBatch, StringArray};
    use std::sync::{Arc, Mutex};
    use yggdryl::arrow::BatchReader;
    use yggdryl::holder::Holder;
    use yggdryl::iceberg::{FormatVersion, PartitionSpec, Table, assign_field_ids};
    use yggdryl::local::Folder;
    use yggdryl::media::IORecordOptions;
    use yggdryl::{DataType, Field, IOBase, IOMedia, Selector, StructType};

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
            pwrite, size, capacity, reserve, truncate, uri, url, bound_location, mtime, media_type,
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
        let mut schema = StructType::from_fields([
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
}
