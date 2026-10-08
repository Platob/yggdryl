//! `rust/src/iceberg/staging.rs`: the Iceberg contract a caller has:
//! expression-driven scans, partition keys as the primary keys, sorted data
//! files, parallel partition writes, and the v3 `unknown` and `variant`
//! types.
//!
//! Everything here reaches the crate through `yggdryl::`; the plan counts and
//! grouping the crate alone can see are pinned in
//! `rust/src/iceberg/tests.rs`.

mod iceberg {
    use arrow_array::{Array, Int64Array, RecordBatch, StringArray};
    use std::sync::Arc;
    use yggdryl::arrow::BatchReader;

    use yggdryl::iceberg::{
        FormatVersion, IcebergOptions, IcebergTable, PartitionSpec, WriteStaging, assign_field_ids,
    };
    use yggdryl::local::LocalFolder;

    use yggdryl::{DataType, Field, IOBase, StructType};

    /// A scratch directory unique to this test and this process.
    fn root(label: &str) -> std::path::PathBuf {
        let mut path = LocalFolder::temporary().unwrap().path().unwrap();
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

    #[test]
    fn write_parallelism_round_trips_and_a_parallel_commit_lists_files_in_group_order() {
        let path = root("write-parallelism");
        let schema = schema();
        let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
            schema,
            spec,
        )
        .unwrap();

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
            IcebergOptions::from_metadata(table.metadata().unwrap())
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
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
            schema,
            spec,
        )
        .unwrap();

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
            IcebergOptions::from_metadata(table.metadata().unwrap())
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
}

/// What an unstaged commit costs an object store, by request.
#[cfg(feature = "s3")]
mod object_store {
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::arrow::BatchReader;
    use yggdryl::iceberg::{
        FormatVersion, IcebergOptions, IcebergTable, PartitionSpec, WriteStaging, assign_field_ids,
    };
    use yggdryl::s3::{self, Credentials, S3Options};
    use yggdryl::{DataType, Field, StructType};

    use crate::server::FakeS3;

    const BUCKET: &str = "trades";

    /// Options reaching `store` and consulting nothing outside the test.
    fn options(store: &FakeS3) -> S3Options {
        S3Options::default()
            .with_environment(false)
            .with_endpoint(store.endpoint())
            .with_region("us-east-1")
            .with_path_style(true)
            .with_credentials(Credentials::new("AKIAIOSFODNN7EXAMPLE", "wJalrXUtnFEMI"))
    }

    fn schema() -> Field {
        let mut schema = StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::utf8().nullable_field("venue"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
        assign_field_ids(&mut schema, 1).unwrap();
        schema
    }

    fn rows(ids: &[i64], venues: &[&str]) -> BatchReader {
        let batch = RecordBatch::try_new(
            schema().into_arrow_schema().unwrap(),
            vec![
                Arc::new(Int64Array::from(ids.to_vec())),
                Arc::new(StringArray::from(venues.to_vec())),
            ],
        )
        .unwrap();
        yggdryl::arrow::batch_reader(batch.schema(), [batch])
    }

    /// The requests the store answered since it was last cleared, as
    /// `METHOD key` - a listing as `LIST` - in the order they arrived.
    fn methods(store: &FakeS3) -> Vec<String> {
        store
            .requests()
            .iter()
            .map(|request| {
                let listing = request
                    .query
                    .iter()
                    .any(|(name, value)| name == "list-type" && value == "2");
                if listing {
                    "LIST".to_owned()
                } else {
                    request.method.clone()
                }
            })
            .collect()
    }

    /// A commit that stages nothing writes every file straight to the store,
    /// one `PUT` each, and asks nothing back: a data file's statistics and
    /// length are what its encoder closed it with, a manifest's and a
    /// manifest list's length what they were laid out as - so no `HEAD`
    /// sizes a file, no tail or footer `GET` reads one, and no probe of a
    /// fresh name asks whether it holds rows. Before, each data file cost a
    /// `HEAD` probing its fresh name, its `PUT`, a `HEAD` and a tail and a
    /// footer `GET` reading its statistics back, and a `HEAD` for its size,
    /// and the manifest and the list a `HEAD` each for theirs.
    #[test]
    fn an_unstaged_commit_costs_one_put_per_file_and_reads_nothing_back() {
        let store = FakeS3::start();
        store.create_bucket(BUCKET);
        let root =
            s3::folder_with(&format!("s3://{BUCKET}/lake/unstaged"), options(&store)).unwrap();
        let schema = schema();
        let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
        let mut table =
            IcebergTable::create(root.clone(), FormatVersion::V3, schema, spec).unwrap();
        table.set_options(
            IcebergOptions::new()
                .try_with_write_staging(WriteStaging::Off)
                .unwrap(),
        );

        store.clear_requests();
        table
            .commit_append(rows(&[1, 2, 3], &["XNAS", "XNYS", "XLON"]))
            .unwrap();
        // Three data files, the manifest and the list, then the chain: the
        // hint read that re-checks the version, the document's claim, its
        // other spelling's `GET`, and the hint.
        let mut sent = methods(&store);
        println!("an unstaged commit of three partitions: {sent:?}");
        sent.sort();
        assert_eq!(
            sent,
            [
                "GET", "GET", "PUT", "PUT", "PUT", "PUT", "PUT", "PUT", "PUT"
            ],
        );

        let opened = IcebergTable::open(root).unwrap();
        assert_eq!(
            opened
                .scan(None)
                .unwrap()
                .map(|batch| batch.unwrap().num_rows())
                .sum::<usize>(),
            3
        );
    }
}
