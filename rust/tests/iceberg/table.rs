//! `rust/src/iceberg/table.rs`: what a commit decides on the way through.
//!
//! A caller sees one call succeed or conflict, so the decisions inside it are
//! pinned here one at a time - the retry ladder's budget and jitter, the merge
//! pruning a malformed or NaN bound must never win, the partition summary a
//! NaN must not enter, the metadata names a listing accepts, and the relative
//! location two writers spell differently. All of it comes through
//! `yggdryl::internals`; what a caller observes of a committed table is pinned
//! in `rust/tests/iceberg/mod_.rs`, and of a table reached by its location
//! alone in [`located`] below.

#[cfg(feature = "internals")]
mod internal {
    mod retry_tests {
        use yggdryl::internals::iceberg_table::{
            CommitSettings, backoff_ms, reserve_retry_backoff, retry_wait_ms,
        };

        #[test]
        fn retry_budget_includes_its_exact_boundary_and_exhaustion_is_a_conflict() {
            let mut spent = 4;
            assert!(reserve_retry_backoff(&mut spent, 1, 5));
            assert_eq!(spent, 5);
            assert!(!reserve_retry_backoff(&mut spent, 1, 5));
            assert_eq!(spent, 5, "a refused wait does not consume budget");

            let settings = CommitSettings::new(1, 0, 0, 0);
            let mut beaten = 0;
            let mut spent = 0;
            assert_eq!(
                retry_wait_ms(&settings, &mut beaten, &mut spent, 2, 2).unwrap(),
                0
            );
            let error = retry_wait_ms(&settings, &mut beaten, &mut spent, 2, 2).unwrap_err();
            assert!(error.is_conflict(), "{error}");
        }

        #[test]
        fn full_jitter_never_exceeds_its_exponential_window() {
            for attempt in 0..8 {
                let cap = 10_u64.saturating_mul(1_u64 << attempt).min(100);
                for _ in 0..32 {
                    assert!(backoff_ms(attempt, 10, 100) <= cap);
                }
            }
            assert_eq!(backoff_ms(4, 0, 100), 0);
        }
    }

    mod key_bound_tests {
        use std::sync::Arc;

        use arrow_array::{Float64Array, RecordBatch};

        use yggdryl::iceberg::{DataFile, ManifestEntry, PartitionSpec};
        use yggdryl::internals::iceberg_table::{KeyBound, KeyBounds, summaries};
        use yggdryl::internals::iceberg_value::single_value;
        use yggdryl::{DataType, Scalar, Selector, StructType};

        #[test]
        fn malformed_external_bounds_cannot_exclude_a_merge_file() {
            let incoming = 37_i64.to_le_bytes().to_vec();
            let bound = KeyBound::new(
                1,
                DataType::Int64,
                false,
                false,
                Some(incoming.clone()),
                Some(incoming),
            );
            let file = DataFile {
                record_count: 1,
                lower_bounds: vec![(1, vec![0; 3])],
                upper_bounds: vec![(1, vec![0; 9])],
                null_value_counts: vec![(1, 0)],
                ..DataFile::default()
            };
            assert!(bound.may_hold(&file));

            let incoming = 1.5_f64.to_le_bytes().to_vec();
            let float_bound = KeyBound::new(
                1,
                DataType::Float64,
                false,
                false,
                Some(incoming.clone()),
                Some(incoming),
            );
            let nan = f64::NAN.to_le_bytes().to_vec();
            let file = DataFile {
                record_count: 1,
                lower_bounds: vec![(1, nan.clone())],
                upper_bounds: vec![(1, nan)],
                null_value_counts: vec![(1, 0)],
                ..DataFile::default()
            };
            assert!(float_bound.may_hold(&file));
        }

        #[test]
        fn generated_nan_merge_bounds_are_conservatively_unbounded() {
            let mut schema = StructType::from_fields([DataType::Float64.required_field("ratio")])
                .map(DataType::from)
                .unwrap()
                .required_field("row");
            yggdryl::iceberg::assign_field_ids(&mut schema, 1).unwrap();
            let arrow_schema = schema.clone().into_arrow_schema().unwrap();
            let batch = RecordBatch::try_new(
                arrow_schema,
                vec![Arc::new(Float64Array::from(vec![1.5, f64::NAN]))],
            )
            .unwrap();

            let bounds =
                KeyBounds::of(&[batch], &schema, &Selector::from_columns(["ratio"])).unwrap();
            assert!(bounds.column_is_unbounded(0));
        }

        #[test]
        fn partition_summaries_omit_nan_bounds() {
            let mut schema = StructType::from_fields([DataType::Float64.required_field("ratio")])
                .map(DataType::from)
                .unwrap()
                .required_field("row");
            yggdryl::iceberg::assign_field_ids(&mut schema, 1).unwrap();
            let spec = PartitionSpec::identity(0, &schema, &["ratio"]).unwrap();
            let entry = |value| {
                ManifestEntry::added(
                    1,
                    DataFile {
                        record_count: 1,
                        partition: vec![value],
                        ..DataFile::default()
                    },
                )
            };

            let only_nan = summaries(&spec, &schema, &[entry(Scalar::from(f64::NAN))]).unwrap();
            assert!(only_nan[0].lower_bound.is_none());
            assert!(only_nan[0].upper_bound.is_none());

            let finite = Scalar::from(1.5_f64);
            let encoded = single_value(&finite, &DataType::Float64).unwrap();
            let mixed = summaries(
                &spec,
                &schema,
                &[entry(Scalar::from(f64::NAN)), entry(finite)],
            )
            .unwrap();
            assert_eq!(mixed[0].lower_bound.as_deref(), Some(encoded.as_slice()));
            assert_eq!(mixed[0].upper_bound.as_deref(), Some(encoded.as_slice()));
        }
    }

    mod metadata_name_tests {
        use yggdryl::internals::iceberg_table::metadata_version_from_name;

        #[test]
        fn accepts_exact_hadoop_and_official_metadata_names() {
            for (name, version) in [
                ("v3.metadata.json", 3),
                ("v00003.gz.metadata.json", 3),
                (
                    "00003-2cd22b57-5127-4198-92ba-e4e67c79821b.metadata.json",
                    3,
                ),
                ("9-2cd22b57-5127-4198-92ba-e4e67c79821b.gz.metadata.json", 9),
            ] {
                assert_eq!(metadata_version_from_name(name), Some(version), "{name}");
            }
        }

        #[test]
        fn rejects_metadata_lookalikes() {
            for name in [
                "v.metadata.json",
                "v2backup.metadata.json",
                "vv2.metadata.json",
                "00002-not-a-uuid.metadata.json",
                "00002-2cd22b57-5127-4198-92ba-e4e67c79821b.extra.metadata.json",
                "00002-2cd22b57-5127-4198-92ba-e4e67c79821b.zst.metadata.json",
                "2.metadata.json",
                "v2.json",
            ] {
                assert_eq!(metadata_version_from_name(name), None, "{name}");
            }
        }
    }

    mod location_tests {
        use yggdryl::internals::iceberg_table::relative_location;

        #[test]
        fn an_empty_uri_authority_is_the_same_place_spelled_shorter() {
            // Whichever writer spelled which: the crate writes the table location
            // with the authority, Spark commits its manifest lists without it.
            for (base, location) in [
                (
                    "file:///warehouse/db/t",
                    "file:/warehouse/db/t/metadata/snap-1.avro",
                ),
                (
                    "file:/warehouse/db/t",
                    "file:///warehouse/db/t/metadata/snap-1.avro",
                ),
                (
                    "file:/warehouse/db/t",
                    "file:/warehouse/db/t/metadata/snap-1.avro",
                ),
            ] {
                assert_eq!(
                    relative_location(base, location).unwrap(),
                    "metadata/snap-1.avro",
                    "{base} -> {location}"
                );
            }
        }

        #[test]
        fn a_real_authority_and_a_windows_drive_are_left_alone() {
            assert_eq!(
                relative_location("s3://bucket/db/t", "s3://bucket/db/t/data/0.parquet").unwrap(),
                "data/0.parquet"
            );
            assert_eq!(
                relative_location("C:\\warehouse\\t", "C:\\warehouse\\t\\data\\0.parquet").unwrap(),
                "data/0.parquet"
            );
            // A neighbour is not a child, however either side is spelled.
            relative_location("file:///warehouse/db/t", "file:/warehouse/db/other/x").unwrap_err();
            relative_location("s3://bucket/db/t", "s3://other/db/t/x").unwrap_err();
        }
    }
}

mod iceberg {
    use arrow_array::{Array, Int64Array, RecordBatch, StringArray};
    use std::sync::Arc;
    use yggdryl::arrow::BatchReader;

    use yggdryl::iceberg::{
        FormatVersion, IcebergOptions, IcebergTable, PartitionSpec, SortField, SortOrder,
        TableMetadata, Transform, assign_field_ids,
    };
    use yggdryl::local::LocalFolder;

    use yggdryl::{DataType, Field, StructType};

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
    fn a_schema_update_replays_onto_the_schema_a_rival_committed() {
        use yggdryl::iceberg::SchemaUpdate;

        let path = root("update-schema");
        let mut first = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            schema(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let mut second = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
        let current = first.metadata().unwrap().current_schema_id();

        // Nothing recorded commits nothing and answers the current schema.
        let empty = SchemaUpdate::from_metadata(first.metadata().unwrap()).unwrap();
        assert_eq!(first.update_schema(&empty).unwrap(), current);

        // Both handles record against the same schema; the second commits
        // first, and the first replays onto what the second made current.
        let mut late = SchemaUpdate::from_metadata(first.metadata().unwrap()).unwrap();
        late.add_column("", DataType::Int64.nullable_field("late"));
        let mut early = SchemaUpdate::from_metadata(second.metadata().unwrap()).unwrap();
        early.add_column("", DataType::Int64.nullable_field("early"));
        let early_id = second.update_schema(&early).unwrap();
        let late_id = first.update_schema(&late).unwrap();

        assert!(late_id > early_id && early_id > current);
        let names: Vec<String> = first
            .metadata()
            .unwrap()
            .current_schema()
            .unwrap()
            .fields()
            .iter()
            .map(|field| field.name().to_owned())
            .collect();
        assert_eq!(names, ["id", "symbol", "venue", "early", "late"]);
    }

    #[test]
    fn data_files_are_sorted_by_the_default_order_the_metadata_records() {
        let path = root("sorted-files");
        let schema = schema();
        let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
        let table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            schema.clone(),
            spec,
        )
        .unwrap();
        // The default: the partition's source column, ascending, nulls first.
        let order = table.metadata().unwrap().default_sort_order().unwrap();
        assert_eq!(table.metadata().unwrap().default_sort_order_id(), 1);
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
        let mut sorted = IcebergTable::create_sorted(
            LocalFolder::new(&by_symbol).unwrap(),
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
        let document = sorted.metadata().unwrap().clone().into_json().unwrap();
        let reread = TableMetadata::from_json(&document).unwrap();
        assert_eq!(reread.default_sort_order().unwrap(), order_by_symbol());
        reread.validate().unwrap();

        // Explicitly unsorted: rows stay as they arrived, files carry no order.
        let plain = root("unsorted");
        let mut unsorted = IcebergTable::create_sorted(
            LocalFolder::new(&plain).unwrap(),
            FormatVersion::V2,
            schema.clone(),
            PartitionSpec::identity(1, &schema, &["venue"]).unwrap(),
            SortOrder::unsorted(),
        )
        .unwrap();
        assert_eq!(unsorted.metadata().unwrap().default_sort_order_id(), 0);
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

    /// A partition group arriving in several batches is ordered across them:
    /// each batch in order is not enough, so the edge between two is read
    /// too; and a group's rows land as they stood, a run of a batch - at its
    /// start, its middle or its end - as surely as rows interleaved with
    /// another partition's.
    #[test]
    fn a_group_is_ordered_across_its_batches_and_every_run_lands_intact() {
        let schema = schema();
        let batch = |ids: &[i64], symbols: &[&str], venues: &[&str]| {
            RecordBatch::try_new(
                schema.clone().into_arrow_schema().unwrap(),
                vec![
                    Arc::new(Int64Array::from(ids.to_vec())),
                    Arc::new(StringArray::from(symbols.to_vec())),
                    Arc::new(StringArray::from(venues.to_vec())),
                ],
            )
            .unwrap()
        };
        let appended = |label: &str, batches: Vec<RecordBatch>| -> Vec<i64> {
            let mut table = IcebergTable::create_sorted(
                LocalFolder::new(root(label)).unwrap(),
                FormatVersion::V2,
                schema.clone(),
                PartitionSpec::identity(1, &schema, &["venue"]).unwrap(),
                order_by_symbol().clone(),
            )
            .unwrap();
            let arrow = batches[0].schema();
            table
                .commit_append(yggdryl::arrow::batch_reader(arrow, batches))
                .unwrap();
            table
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
                .collect()
        };

        // Each batch in symbol order, the second opening below where the
        // first closed: the group is ordered as one.
        let edge = appended(
            "edge-descends",
            vec![
                batch(&[1, 2], &["b", "c"], &["X", "X"]),
                batch(&[3, 4], &["a", "d"], &["X", "X"]),
            ],
        );
        assert_eq!(edge, [3, 1, 2, 4]);
        // In order across the edge too, ties included: rows land as they came.
        let ordered = appended(
            "edge-holds",
            vec![
                batch(&[1, 2], &["a", "b"], &["X", "X"]),
                batch(&[3, 4], &["b", "c"], &["X", "X"]),
            ],
        );
        assert_eq!(ordered, [1, 2, 3, 4]);

        // Runs at a batch's start, middle and end, then two partitions
        // interleaved: files follow the groups' first rows, each group's
        // rows in symbol order.
        let runs = appended(
            "runs",
            vec![
                batch(
                    &[1, 2, 3, 4, 5],
                    &["a", "b", "a", "b", "c"],
                    &["X", "X", "Y", "Y", "Z"],
                ),
                batch(&[6, 7, 8], &["b", "a", "a"], &["W", "V", "W"]),
            ],
        );
        assert_eq!(runs, [1, 2, 3, 4, 5, 8, 6, 7]);
    }

    #[test]
    fn a_table_streams_none_of_the_files_that_store_it() {
        use yggdryl::{Error, IOBase, IOKind};

        let path = root("no-bytes");
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            schema(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        table
            .commit_append(rows(&[1, 2], &["AAPL", "MSFT"], &["XNAS", "XNAS"]))
            .unwrap();

        // The folder beneath streams its metadata, manifests and data files
        // end to end, as every folder does; the table is read as rows, and
        // those files are its storage rather than its bytes.
        assert!(
            !LocalFolder::new(&path)
                .unwrap()
                .read_all_bytes()
                .unwrap()
                .is_empty()
        );
        assert_eq!(table.kind(), IOKind::Table);
        assert!(table.pstream_bytes(0, 4096).unwrap().next().is_none());
        assert!(table.pstream_bytes(16, 4096).unwrap().next().is_none());
        assert!(table.read_all_bytes().unwrap().is_empty());
        assert_eq!(table.size(), 0);
        let refused = table.pstream_bytes(0, 0).unwrap_err();
        assert!(
            matches!(&refused, Error::Io(error) if error.kind() == std::io::ErrorKind::InvalidInput),
            "{refused:?}"
        );
        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn batches_changing_layout_mid_commit_each_cast_to_the_table_schema() {
        let path = root("table_layouts");
        let schema = schema();
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V2,
            schema.clone(),
            PartitionSpec::identity(1, &schema, &["venue"]).unwrap(),
        )
        .unwrap();
        let exact = schema.clone().into_arrow_schema().unwrap();
        // The same columns, reordered, nullable, and carrying no field ids.
        let loose = Arc::new(arrow_schema::Schema::new(vec![
            arrow_schema::Field::new("venue", arrow_schema::DataType::Utf8, true),
            arrow_schema::Field::new("id", arrow_schema::DataType::Int64, true),
            arrow_schema::Field::new("symbol", arrow_schema::DataType::Utf8, true),
        ]));
        let exact_rows = |id: i64, symbol: &str, venue: &str| {
            RecordBatch::try_new(
                Arc::clone(&exact),
                vec![
                    Arc::new(Int64Array::from(vec![id])),
                    Arc::new(StringArray::from(vec![symbol])),
                    Arc::new(StringArray::from(vec![venue])),
                ],
            )
            .unwrap()
        };
        let loose_rows = |id: i64, symbol: &str, venue: &str| {
            RecordBatch::try_new(
                Arc::clone(&loose),
                vec![
                    Arc::new(StringArray::from(vec![venue])),
                    Arc::new(Int64Array::from(vec![id])),
                    Arc::new(StringArray::from(vec![symbol])),
                ],
            )
            .unwrap()
        };
        table
            .commit_append(yggdryl::arrow::batch_reader(
                Arc::clone(&exact),
                [
                    exact_rows(1, "AAPL", "XNAS"),
                    loose_rows(2, "VOD", "XLON"),
                    exact_rows(3, "MSFT", "XNAS"),
                    loose_rows(4, "BP", "XLON"),
                ],
            ))
            .unwrap();

        let mut read: Vec<(i64, String, String)> = Vec::new();
        for batch in table.scan(None).unwrap() {
            let batch = batch.unwrap();
            let column = |name: &str| Arc::clone(batch.column_by_name(name).unwrap());
            let (ids, symbols, venues) = (column("id"), column("symbol"), column("venue"));
            let ids = ids.as_any().downcast_ref::<Int64Array>().unwrap();
            let symbols = symbols.as_any().downcast_ref::<StringArray>().unwrap();
            let venues = venues.as_any().downcast_ref::<StringArray>().unwrap();
            for row in 0..batch.num_rows() {
                read.push((
                    ids.value(row),
                    symbols.value(row).to_owned(),
                    venues.value(row).to_owned(),
                ));
            }
        }
        read.sort();
        let expected: Vec<(i64, String, String)> = [
            (1, "AAPL", "XNAS"),
            (2, "VOD", "XLON"),
            (3, "MSFT", "XNAS"),
            (4, "BP", "XLON"),
        ]
        .into_iter()
        .map(|(id, symbol, venue)| (id, symbol.to_owned(), venue.to_owned()))
        .collect();
        assert_eq!(read, expected);
        assert_eq!(table.data_files().unwrap().len(), 2, "one file per venue");
        let _ = std::fs::remove_dir_all(&path);
    }

    /// One quote: its venue, its instant and its id.
    type Quote = (&'static str, i64, i64);

    /// The quotes schema - `venue`, `ts`, `id` - numbered, declaring `sort`
    /// as the order a table created from it keeps.
    fn quotes_schema(sort: &[&str]) -> Field {
        let mut schema = StructType::from_fields([
            DataType::utf8().required_field("venue"),
            DataType::Int64.required_field("ts"),
            DataType::Int64.required_field("id"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
        if !sort.is_empty() {
            schema.as_sort_mut().set_by_texts(sort).unwrap();
        }
        assign_field_ids(&mut schema, 1).unwrap();
        schema
    }

    /// One commit's quotes, under a schema declaring nothing.
    fn quotes(rows: &[Quote]) -> BatchReader {
        let schema = Arc::new(arrow_schema::Schema::new(vec![
            arrow_schema::Field::new("venue", arrow_schema::DataType::Utf8, false),
            arrow_schema::Field::new("ts", arrow_schema::DataType::Int64, false),
            arrow_schema::Field::new("id", arrow_schema::DataType::Int64, false),
        ]));
        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![
                Arc::new(StringArray::from(
                    rows.iter().map(|row| row.0).collect::<Vec<_>>(),
                )),
                Arc::new(Int64Array::from(
                    rows.iter().map(|row| row.1).collect::<Vec<_>>(),
                )),
                Arc::new(Int64Array::from(
                    rows.iter().map(|row| row.2).collect::<Vec<_>>(),
                )),
            ],
        )
        .unwrap();
        yggdryl::arrow::batch_reader(schema, [batch])
    }

    /// A venue-partitioned quotes table under `order` - the schema's own
    /// where `None` - with one commit per entry of `commits`.
    fn quotes_table(
        label: &str,
        schema: &Field,
        order: Option<SortOrder>,
        commits: &[&[Quote]],
    ) -> IcebergTable<LocalFolder> {
        let folder = LocalFolder::new(root(label)).unwrap();
        let spec = PartitionSpec::identity(1, schema, &["venue"]).unwrap();
        let mut table = match order {
            Some(order) => {
                IcebergTable::create_sorted(folder, FormatVersion::V2, schema.clone(), spec, order)
            }
            None => IcebergTable::create(folder, FormatVersion::V2, schema.clone(), spec),
        }
        .unwrap();
        for commit in commits {
            table.commit_append(quotes(commit)).unwrap();
        }
        table
    }

    /// Every row of `batches` as `(venue, ts, id)`, in the order they came.
    fn quote_rows<'a>(
        batches: impl IntoIterator<Item = &'a RecordBatch>,
    ) -> Vec<(String, i64, i64)> {
        let mut rows = Vec::new();
        for batch in batches {
            let column = |name: &str| Arc::clone(batch.column_by_name(name).unwrap());
            let (venues, instants, ids) = (column("venue"), column("ts"), column("id"));
            let venues = venues.as_any().downcast_ref::<StringArray>().unwrap();
            let instants = instants.as_any().downcast_ref::<Int64Array>().unwrap();
            let ids = ids.as_any().downcast_ref::<Int64Array>().unwrap();
            for row in 0..batch.num_rows() {
                rows.push((
                    venues.value(row).to_owned(),
                    instants.value(row),
                    ids.value(row),
                ));
            }
        }
        rows
    }

    /// The ids of `rows`, in order.
    fn ids_of(rows: &[(String, i64, i64)]) -> Vec<i64> {
        rows.iter().map(|row| row.2).collect()
    }

    /// The records a whole-table record read yields, each as its batch.
    fn records_of(reader: yggdryl::SerieReader) -> Vec<RecordBatch> {
        reader
            .map(|record| record.unwrap().into_arrow_batch().unwrap())
            .collect()
    }

    /// The `order by` keys a record declares, as text.
    fn declared_keys(record: &yggdryl::Serie) -> Vec<String> {
        record
            .declared_order()
            .unwrap()
            .unwrap_or_default()
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    /// A record read yields partition after partition in ascending tuple
    /// order, each partition's rows in the table's order across every file
    /// that stores it - the files of one partition that overlap merged, the
    /// files that follow one another read as they are - and the root, every
    /// record, the transport schema and a schema read all declare it.
    #[test]
    fn a_record_read_yields_partitions_in_order_each_in_the_table_order() {
        use arrow_array::RecordBatchReader as _;
        use yggdryl::IOMedia;

        let order = Some(r#"["venue","ts","id"]"#);
        let table = quotes_table(
            "ordered-read",
            &quotes_schema(&["venue", "ts", "id"]),
            None,
            &[
                &[("XNYS", 30, 1), ("XNAS", 20, 2), ("XNAS", 10, 3)],
                &[("XLON", 5, 4), ("XNAS", 15, 5), ("XNYS", 10, 6)],
                &[("XNAS", 12, 7), ("XNYS", 20, 8), ("XLON", 1, 9)],
            ],
        );
        let expected = [9, 4, 3, 7, 5, 2, 6, 8, 1];

        let reader = table.read_serie(None).unwrap();
        assert_eq!(reader.field().get_metadata("SORT:by"), order);
        let records: Vec<yggdryl::Serie> = reader.map(Result::unwrap).collect();
        for record in &records {
            assert_eq!(declared_keys(record), ["venue", "ts", "id"]);
        }
        let batches: Vec<RecordBatch> = records
            .iter()
            .map(|record| record.into_arrow_batch().unwrap())
            .collect();
        assert_eq!(ids_of(&quote_rows(&batches)), expected);
        // XLON's two files and XNYS's three each cover a later stretch than
        // the one opened before them, so each is a record of its own; two of
        // XNAS's overlap, so its four rows are merged into one.
        assert_eq!(
            records.iter().map(yggdryl::Serie::len).collect::<Vec<_>>(),
            [1, 1, 4, 1, 1, 1]
        );

        // The transport face is the same stream under the same declaration,
        // and a schema read declares what the stream does.
        let options = table.record_options().unwrap();
        let transport = table.read_arrow_reader(&options).unwrap();
        assert_eq!(
            transport
                .schema()
                .metadata()
                .get("SORT:by")
                .map(String::as_str),
            order
        );
        let batches: Vec<RecordBatch> = transport.map(Result::unwrap).collect();
        assert_eq!(ids_of(&quote_rows(&batches)), expected);
        assert_eq!(
            table
                .read_arrow_field(&options)
                .unwrap()
                .get_metadata("SORT:by"),
            order
        );
        // The table still reports the order its writers keep, and a scan
        // door still reads the files as the manifests list them, commit
        // after commit.
        assert_eq!(table.schema().unwrap().get_metadata("SORT:by"), order);
        let scan = table.scan(None).unwrap();
        assert_eq!(scan.schema().metadata().get("SORT:by"), None);
        let scanned: Vec<RecordBatch> = scan.map(Result::unwrap).collect();
        let mut scanned = ids_of(&quote_rows(&scanned));
        assert_ne!(scanned, expected);
        assert_eq!(scanned[..3].iter().max(), Some(&3));
        scanned.sort_unstable();
        assert_eq!(scanned, [1, 2, 3, 4, 5, 6, 7, 8, 9]);
    }

    /// A partition whose commits each cover an earlier stretch than the one
    /// before is opened by its files' bounds, not their commit order, and so
    /// arrives in order with nothing merged: one record per data file, each
    /// the file's own rows.
    #[test]
    fn a_partition_whose_files_follow_one_another_is_read_without_a_merge() {
        use yggdryl::IOMedia;

        let table = quotes_table(
            "ordered-no-merge",
            &quotes_schema(&["venue", "ts", "id"]),
            None,
            &[
                &[("XNAS", 51, 1), ("XLON", 50, 2), ("XNAS", 50, 3)],
                &[("XNAS", 31, 4), ("XLON", 30, 5), ("XNAS", 30, 6)],
                &[("XNAS", 11, 7), ("XLON", 10, 8), ("XNAS", 10, 9)],
            ],
        );
        let records: Vec<yggdryl::Serie> = table
            .read_serie(None)
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(records.len(), table.data_files().unwrap().len());
        assert_eq!(
            records.iter().map(yggdryl::Serie::len).collect::<Vec<_>>(),
            [1, 1, 1, 2, 2, 2]
        );
        let batches: Vec<RecordBatch> = records
            .iter()
            .map(|record| record.into_arrow_batch().unwrap())
            .collect();
        assert_eq!(ids_of(&quote_rows(&batches)), [8, 5, 2, 9, 7, 6, 4, 3, 1]);
    }

    /// A `where` window prunes the files its bounds rule out and the rest
    /// still arrive in order; a row limit decodes the first partition and
    /// opens no file of the next.
    #[test]
    fn a_window_prunes_files_and_a_limit_reads_only_the_first_partition() {
        use crate::counting_filesystem::counted_folder;
        use yggdryl::IOMedia;
        use yggdryl::media::IORecordOptions;

        let schema = quotes_schema(&["venue", "ts", "id"]);
        let (filesystem, folder) = counted_folder("ordered-window");
        let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
        let mut table =
            IcebergTable::create(folder, FormatVersion::V2, schema.clone(), spec).unwrap();
        let commits: [&[Quote]; 3] = [
            &[("XNAS", 1, 1), ("XLON", 2, 2), ("XNYS", 3, 3)],
            &[("XNAS", 11, 4), ("XLON", 12, 5), ("XNYS", 13, 6)],
            &[("XNAS", 21, 7), ("XLON", 22, 8), ("XNYS", 23, 9)],
        ];
        for commit in commits {
            table.commit_append(quotes(commit)).unwrap();
        }

        // The window holds the middle commit's files alone: the other six
        // are excluded on their bounds, never opened.
        let window = "ts >= 10 and ts < 20";
        let plan = table.plan_matching(window).unwrap();
        assert_eq!((plan.tasks.len(), plan.files_skipped()), (3, 6));
        let windowed = table.record_options().unwrap().with_filter(window).unwrap();
        let reader = table.read_serie(Some(&windowed)).unwrap();
        assert_eq!(
            reader.field().get_metadata("SORT:by"),
            Some(r#"["venue","ts","id"]"#)
        );
        assert_eq!(ids_of(&quote_rows(&records_of(reader))), [5, 4, 6]);

        // A whole read opens the manifest list, the three manifests and all
        // nine data files, each of those sized five times on its way to a
        // reader; one row is the first partition's first, and only that
        // partition's three files are opened to answer it.
        let options = table.record_options().unwrap();
        let whole = filesystem.costs(|| {
            let rows = quote_rows(&records_of(table.read_serie(Some(&options)).unwrap()));
            assert_eq!(rows.len(), 9);
        });
        // Ordering costs no call: the bounds it opens files by are the
        // manifests' own, so a record read costs what a scan does.
        let scanned = filesystem.costs(|| {
            assert_eq!(table.scan(None).unwrap().map(Result::unwrap).count(), 9);
        });
        assert_eq!(scanned, whole);
        let limited = options.clone().with_max_row_size(1);
        let first = filesystem.costs(|| {
            let rows = quote_rows(&records_of(table.read_serie(Some(&limited)).unwrap()));
            assert_eq!(ids_of(&rows), [2]);
        });
        assert_eq!(
            (whole.as_str(), first.as_str()),
            (
                "file_info=45 open_input_stream=13",
                "file_info=15 open_input_stream=7"
            )
        );
    }

    /// An unsorted table and one sorted by its partition column alone hold
    /// nothing: their files stream in tuple order, each partition's in the
    /// order its commits wrote them. The first declares no order; the
    /// second its partition column, which the tuple order proves.
    #[test]
    fn a_table_sorted_by_no_more_than_its_partitions_streams_them_in_tuple_order() {
        use yggdryl::IOMedia;

        let commits: &[&[Quote]] = &[
            &[("XNYS", 3, 1), ("XNAS", 1, 2)],
            &[("XLON", 2, 3), ("XNAS", 0, 4)],
        ];
        let unsorted = quotes_table(
            "ordered-unsorted",
            &quotes_schema(&[]),
            Some(SortOrder::unsorted()),
            commits,
        );
        let reader = unsorted.read_serie(None).unwrap();
        assert_eq!(reader.field().get_metadata("SORT:by"), None);
        assert_eq!(ids_of(&quote_rows(&records_of(reader))), [3, 2, 4, 1]);
        let options = unsorted.record_options().unwrap();
        assert_eq!(
            unsorted
                .read_arrow_field(&options)
                .unwrap()
                .get_metadata("SORT:by"),
            None
        );

        // A partitioned table declaring no order keeps its partition
        // column's, ascending with nulls first.
        let by_venue = quotes_table("ordered-by-venue", &quotes_schema(&[]), None, commits);
        let reader = by_venue.read_serie(None).unwrap();
        assert_eq!(
            reader.field().get_metadata("SORT:by"),
            Some(r#"["venue nulls first"]"#)
        );
        let records: Vec<yggdryl::Serie> = reader.map(Result::unwrap).collect();
        for record in &records {
            assert_eq!(declared_keys(record), ["venue nulls first"]);
        }
        let batches: Vec<RecordBatch> = records
            .iter()
            .map(|record| record.into_arrow_batch().unwrap())
            .collect();
        assert_eq!(ids_of(&quote_rows(&batches)), [3, 2, 4, 1]);
    }

    /// A `select` keeps the part of the order whose columns it publishes
    /// unchanged: dropping `ts` leaves the partition column declared, each
    /// partition's rows in its files' order, and the stream landed behind
    /// the selector carries that declaration.
    #[test]
    fn a_select_keeps_the_part_of_the_order_it_publishes() {
        use yggdryl::IOMedia;
        use yggdryl::media::IORecordOptions;

        let table = quotes_table(
            "ordered-select",
            &quotes_schema(&["venue", "ts", "id"]),
            None,
            &[
                &[("XNAS", 20, 1), ("XLON", 5, 2)],
                &[("XNAS", 10, 3), ("XLON", 1, 4)],
            ],
        );
        let options = table
            .record_options()
            .unwrap()
            .with_select("venue, id")
            .unwrap();
        let reader = table.read_serie(Some(&options)).unwrap();
        assert_eq!(reader.field().get_metadata("SORT:by"), Some(r#"["venue"]"#));
        let mut rows: Vec<(String, i64)> = Vec::new();
        for batch in records_of(reader) {
            assert_eq!(batch.num_columns(), 2);
            let venues = Arc::clone(batch.column_by_name("venue").unwrap());
            let venues = venues.as_any().downcast_ref::<StringArray>().unwrap();
            let ids = Arc::clone(batch.column_by_name("id").unwrap());
            let ids = ids.as_any().downcast_ref::<Int64Array>().unwrap();
            for row in 0..batch.num_rows() {
                rows.push((venues.value(row).to_owned(), ids.value(row)));
            }
        }
        // Each partition's files open by where their `ts` starts.
        let expected: Vec<(String, i64)> = [("XLON", 4), ("XLON", 2), ("XNAS", 3), ("XNAS", 1)]
            .into_iter()
            .map(|(venue, id)| (venue.to_owned(), id))
            .collect();
        assert_eq!(rows, expected);
        assert_eq!(
            table
                .read_arrow_field(&options)
                .unwrap()
                .get_metadata("SORT:by"),
            Some(r#"["venue"]"#)
        );
    }

    /// The same read under one decode thread and under four yields the
    /// same records, row for row and record for record.
    #[test]
    fn a_parallel_record_read_yields_what_a_sequential_one_does() {
        use yggdryl::IOMedia;

        let venues = ["XLON", "XNAS", "XNYS"];
        let commits: Vec<Vec<Quote>> = (0..4_i64)
            .map(|commit| {
                (0..9_i64)
                    .map(|row| {
                        let venue = venues[usize::try_from(row % 3).unwrap()];
                        (venue, (7 * commit + 5 * row) % 13, commit * 9 + row)
                    })
                    .collect()
            })
            .collect();
        let commits: Vec<&[Quote]> = commits.iter().map(Vec::as_slice).collect();
        let mut table = quotes_table(
            "ordered-parallel",
            &quotes_schema(&["venue", "ts", "id"]),
            None,
            &commits,
        );
        let mut read = |threads: usize| {
            table.set_options(
                IcebergOptions::new()
                    .try_with_read_parallelism(threads)
                    .unwrap()
                    .with_read_parallel_min_files(2)
                    .with_read_parallel_min_file_size_bytes(0),
            );
            let records = records_of(table.read_serie(None).unwrap());
            (
                records
                    .iter()
                    .map(RecordBatch::num_rows)
                    .collect::<Vec<_>>(),
                quote_rows(&records),
            )
        };
        let sequential = read(1);
        let parallel = read(4);
        assert_eq!(sequential, parallel);
        let rows = sequential.1;
        assert_eq!(rows.len(), 36);
        assert!(rows.windows(2).all(|pair| pair[0] <= pair[1]), "{rows:?}");
    }

    /// Drain `reader` landed again under the root its schema declares - the
    /// door that reads a declared order before it believes it - and answer
    /// its rows.
    fn relanded(reader: BatchReader) -> Vec<(String, i64, i64)> {
        let landed = yggdryl::SerieReader::from_arrow_reader(
            None,
            reader,
            yggdryl::ArrowCastOptions::default(),
        )
        .unwrap();
        quote_rows(&records_of(landed))
    }

    /// A partition is sorted on the column a transform key reads, so the
    /// key holds across its rows and no key after it does: two rows of one
    /// `truncate(ts, 10)` bucket keep their `ts` order whatever their ids.
    /// The declaration ends at the transform - partitioned or not - and the
    /// transport, landed again under it, proves it.
    #[test]
    fn a_transform_key_is_the_last_key_a_record_read_declares() {
        use yggdryl::IOMedia;

        let schema = quotes_schema(&["venue", "truncate(ts, 10)", "id"]);
        let table = quotes_table(
            "ordered-transform",
            &schema,
            None,
            &[&[("XNAS", 11, 1), ("XLON", 3, 2), ("XNAS", 10, 5)]],
        );
        let declared = Some(r#"["venue","truncate(ts, 10)"]"#);
        let reader = table.read_serie(None).unwrap();
        assert_eq!(reader.field().get_metadata("SORT:by"), declared);
        assert_eq!(ids_of(&quote_rows(&records_of(reader))), [2, 5, 1]);
        let options = table.record_options().unwrap();
        assert_eq!(
            table
                .read_arrow_field(&options)
                .unwrap()
                .get_metadata("SORT:by"),
            declared
        );
        let rows = relanded(table.read_arrow_reader(&options).unwrap());
        assert_eq!(ids_of(&rows), [2, 5, 1]);

        let mut unpartitioned = IcebergTable::create(
            LocalFolder::new(root("ordered-transform-whole")).unwrap(),
            FormatVersion::V2,
            quotes_schema(&["truncate(ts, 10)", "id"]),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        unpartitioned
            .commit_append(quotes(&[("XNAS", 11, 1), ("XNAS", 10, 5)]))
            .unwrap();
        let reader = unpartitioned.read_serie(None).unwrap();
        assert_eq!(
            reader.field().get_metadata("SORT:by"),
            Some(r#"["truncate(ts, 10)"]"#)
        );
        assert_eq!(ids_of(&quote_rows(&records_of(reader))), [5, 1]);
        let options = unpartitioned.record_options().unwrap();
        let rows = relanded(unpartitioned.read_arrow_reader(&options).unwrap());
        assert_eq!(ids_of(&rows), [5, 1]);
    }

    /// Partitions arrive in the order of their stored tuples, which a
    /// declared field reading the partition column as another datatype
    /// does not keep - `9` before `10` is `"10"` before `"9"` as text - so
    /// such a read declares no order from that key on.
    #[test]
    fn a_partition_key_read_as_another_datatype_is_not_declared() {
        use yggdryl::IOMedia;
        use yggdryl::media::IORecordOptions;

        let schema = quotes_schema(&["ts", "id"]);
        let spec = PartitionSpec::identity(1, &schema, &["ts"]).unwrap();
        let mut table = IcebergTable::create(
            LocalFolder::new(root("ordered-retyped")).unwrap(),
            FormatVersion::V2,
            schema.clone(),
            spec,
        )
        .unwrap();
        table
            .commit_append(quotes(&[("XNAS", 10, 1), ("XNAS", 9, 2)]))
            .unwrap();
        // Stored as it is, the partition column orders the read.
        let reader = table.read_serie(None).unwrap();
        assert_eq!(
            reader.field().get_metadata("SORT:by"),
            Some(r#"["ts","id"]"#)
        );
        assert_eq!(ids_of(&quote_rows(&records_of(reader))), [2, 1]);

        let mut text = StructType::from_fields([
            DataType::utf8().required_field("venue"),
            DataType::utf8().required_field("ts"),
            DataType::Int64.required_field("id"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
        assign_field_ids(&mut text, 1).unwrap();
        let options = table.record_options().unwrap().with_field(text);
        let reader = table.read_serie(Some(&options)).unwrap();
        assert_eq!(reader.field().get_metadata("SORT:by"), None);
        let mut instants = Vec::new();
        for record in reader {
            let batch = record.unwrap().into_arrow_batch().unwrap();
            let column = Arc::clone(batch.column_by_name("ts").unwrap());
            let column = column.as_any().downcast_ref::<StringArray>().unwrap();
            instants.extend((0..column.len()).map(|row| column.value(row).to_owned()));
        }
        assert_eq!(instants, ["9", "10"]);
        assert_eq!(
            table
                .read_arrow_field(&options)
                .unwrap()
                .get_metadata("SORT:by"),
            None
        );
        let landed = yggdryl::SerieReader::from_arrow_reader(
            None,
            table.read_arrow_reader(&options).unwrap(),
            yggdryl::ArrowCastOptions::default(),
        )
        .unwrap();
        assert_eq!(landed.map(|record| record.unwrap().len()).sum::<usize>(), 2);
    }
}

mod derived_columns {
    //! A table computes the columns its schema derives, for every row
    //! written to it, and says so again when it is reopened.

    use std::sync::Arc;

    use arrow_array::{Array, Int64Array, RecordBatch, TimestampNanosecondArray};
    use yggdryl::arrow::BatchReader;
    use yggdryl::iceberg::{
        FormatVersion, IcebergTable, PartitionSpec, TableMetadata, Transform, assign_field_ids,
    };
    use yggdryl::local::LocalFolder;
    use yggdryl::media::IORecordOptions;
    use yggdryl::{DataType, Field, IOMedia, StructType, TimeUnit, Timezone};

    const QUARTER: i64 = 900_000_000_000;

    fn root(label: &str) -> std::path::PathBuf {
        let mut path = LocalFolder::temporary().unwrap().path().unwrap();
        path.push(format!(
            "yggdryl-iceberg-derived-{label}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        path
    }

    /// The rows as a caller holds them: an instant and an id, no partition.
    fn row() -> Field {
        StructType::from_fields([
            DataType::DateTime64 {
                unit: TimeUnit::Nanosecond,
                timezone: Timezone::UTC,
            }
            .required_field("ts"),
            DataType::Int64.required_field("id"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row")
    }

    /// The table's schema: the rows, partitioned by the quarter-hour each
    /// instant falls in and sorted by it, the instant and the id.
    fn declared() -> Field {
        let mut schema = row()
            .with_partition_by(["time_bucket('15 minutes', ts) as part".parse().unwrap()])
            .unwrap();
        schema
            .as_sort_mut()
            .set_by_texts(["part", "ts", "id"])
            .unwrap();
        assign_field_ids(&mut schema, 1).unwrap();
        schema
    }

    /// Instants laid out as `field` types them, its zone included.
    fn instants(values: &[i64], field: &arrow_schema::Field) -> Arc<dyn Array> {
        Arc::new(
            TimestampNanosecondArray::from(values.to_vec())
                .with_data_type(field.data_type().clone()),
        )
    }

    fn rows(stamps: &[i64], ids: &[i64]) -> BatchReader {
        let schema = row().into_arrow_schema().unwrap();
        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![
                instants(stamps, schema.field(0)),
                Arc::new(Int64Array::from(ids.to_vec())),
            ],
        )
        .unwrap();
        yggdryl::arrow::batch_reader(batch.schema(), [batch])
    }

    /// `(part, ts, id)` of every stored row, sorted.
    fn read(table: &IcebergTable<LocalFolder>) -> Vec<(i64, i64, i64)> {
        let mut read = Vec::new();
        for batch in table.scan(None).unwrap() {
            let batch = batch.unwrap();
            let stamped = |name: &str| {
                batch
                    .column_by_name(name)
                    .unwrap()
                    .as_any()
                    .downcast_ref::<TimestampNanosecondArray>()
                    .unwrap()
                    .clone()
            };
            let (parts, stamps) = (stamped("part"), stamped("ts"));
            let ids = batch
                .column_by_name("id")
                .unwrap()
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap()
                .clone();
            for index in 0..batch.num_rows() {
                read.push((parts.value(index), stamps.value(index), ids.value(index)));
            }
        }
        read.sort_unstable();
        read
    }

    #[test]
    fn a_derived_partition_column_is_computed_by_every_write_door() {
        let path = root("doors");
        let schema = declared();
        let spec = PartitionSpec::from_schema(1, &schema).unwrap();
        assert_eq!(spec.fields.len(), 1);
        assert_eq!(spec.fields[0].transform, Transform::Identity);
        assert_eq!(spec.fields[0].name, "part");
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
            schema,
            spec,
        )
        .unwrap();

        // The rows carry no `part`: the commit door computes it, and the
        // table lays its files out by it.
        table
            .commit_append(rows(
                &[1, QUARTER - 1, QUARTER, 2 * QUARTER + 7],
                &[1, 2, 3, 4],
            ))
            .unwrap();
        assert_eq!(
            read(&table),
            [
                (0, 1, 1),
                (0, QUARTER - 1, 2),
                (QUARTER, QUARTER, 3),
                (2 * QUARTER, 2 * QUARTER + 7, 4),
            ]
        );
        assert_eq!(table.data_files().unwrap().len(), 3, "one file a quarter");

        // The record door computes it too, before its `where` reads the rows
        // - which may therefore name the derived column - and an overwrite
        // replaces the quarter the rows fall in and no other.
        let options = table.record_options().unwrap();
        table
            .overwrite_arrow_reader(rows(&[QUARTER + 5], &[30]), &options)
            .unwrap();
        assert_eq!(
            read(&table),
            [
                (0, 1, 1),
                (0, QUARTER - 1, 2),
                (QUARTER, QUARTER + 5, 30),
                (2 * QUARTER, 2 * QUARTER + 7, 4),
            ]
        );
        let late = options
            .clone()
            .with_filter("part >= '1970-01-01T00:45:00Z'")
            .unwrap();
        table
            .append_arrow_reader(rows(&[3, 3 * QUARTER + 1], &[50, 51]), &late)
            .unwrap();
        assert_eq!(
            read(&table).len(),
            5,
            "the `where` kept one of the two rows"
        );
        assert!(read(&table).contains(&(3 * QUARTER, 3 * QUARTER + 1, 51)));

        // A push session shapes each batch the same way.
        let mut folder = LocalFolder::new(&path).unwrap();
        let mut session = yggdryl::ArrowWriteSession::append(&options).unwrap();
        session
            .push(&mut folder, rows(&[4 * QUARTER + 2], &[60]))
            .unwrap();
        session.finish(&mut folder).unwrap();
        let table = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
        assert!(read(&table).contains(&(4 * QUARTER, 4 * QUARTER + 2, 60)));
        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn a_reopened_table_declares_the_term_its_properties_keep() {
        let path = root("reopened");
        let schema = declared();
        let spec = PartitionSpec::from_schema(1, &schema).unwrap();
        IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
            schema,
            spec,
        )
        .unwrap();

        let table = IcebergTable::open(LocalFolder::new(&path).unwrap()).unwrap();
        let key = format!("{}part", TableMetadata::TRANSFORM_PROPERTY_PREFIX);
        assert_eq!(
            table.metadata().unwrap().property(&key),
            Some("time_bucket('15 minutes', ts)")
        );
        let part = table
            .schema()
            .unwrap()
            .fields()
            .iter()
            .find(|child| child.name() == "part")
            .unwrap()
            .clone();
        assert_eq!(
            part.as_transform().term().unwrap().unwrap().to_string(),
            "time_bucket('15 minutes', ts)"
        );
        assert_eq!(
            table.schema().unwrap().get_metadata("PARTITION:by"),
            Some(r#"["part"]"#)
        );

        // And a write through the reopened table still computes it.
        let mut table = table;
        table.commit_append(rows(&[QUARTER + 9], &[1])).unwrap();
        assert_eq!(read(&table), [(QUARTER, QUARTER + 9, 1)]);
        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn a_value_the_rows_carry_under_a_derived_name_is_computed_again() {
        let path = root("written");
        let schema = declared();
        let spec = PartitionSpec::from_schema(1, &schema).unwrap();
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
            schema,
            spec,
        )
        .unwrap();
        // The stored layout, `part` included and stating another quarter
        // than the term answers: the table owns the derivation, so what it
        // stores is what its schema says, whatever the rows carried.
        let stored = table.schema().unwrap().clone().into_arrow_schema().unwrap();
        let columns: Vec<Arc<dyn Array>> = stored
            .fields()
            .iter()
            .map(|field| match field.name().as_str() {
                "ts" => instants(&[QUARTER + 1], field),
                "part" => instants(&[5 * QUARTER], field),
                _ => Arc::new(Int64Array::from(vec![1_i64])),
            })
            .collect();
        let batch = RecordBatch::try_new(stored, columns).unwrap();
        table
            .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap();
        assert_eq!(read(&table), [(QUARTER, QUARTER + 1, 1)]);
        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn a_window_on_the_source_prunes_by_the_bucket_it_falls_in() {
        let path = root("pruned");
        let schema = declared();
        let spec = PartitionSpec::from_schema(1, &schema).unwrap();
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
            schema,
            spec,
        )
        .unwrap();
        // One commit a quarter: one manifest each, summarizing one bucket.
        for quarter in 0..4_i64 {
            table
                .commit_append(rows(
                    &[quarter * QUARTER + 5, quarter * QUARTER + 9],
                    &[quarter * 10, quarter * 10 + 1],
                ))
                .unwrap();
        }
        assert_eq!(table.manifests().unwrap().len(), 4);

        // The window names `ts` alone. Every row's `part` is the bucket its
        // `ts` falls in, so a manifest's summary of `part` bounds `ts`: the
        // three other quarters are ruled out unopened.
        let window = "ts >= '1970-01-01T00:15:00Z' and ts < '1970-01-01T00:30:00Z'";
        let plan = table.plan_matching(window).unwrap();
        assert_eq!(plan.manifests_read, 1);
        assert_eq!(plan.skipped.len(), 3);
        assert_eq!(plan.tasks.len(), 1);
        let options = table.record_options().unwrap().with_filter(window).unwrap();
        let read: usize = table
            .read_serie(Some(&options))
            .unwrap()
            .map(|record| record.unwrap().len())
            .sum();
        assert_eq!(read, 2);

        // A bucket bounds its source by its whole range, not its start: a
        // window closing the first quarter and opening the second keeps both
        // manifests, and the first quarter's file is then ruled out by its
        // own statistics of `ts`.
        let plan = table
            .plan_matching("ts >= '1970-01-01T00:14:59Z' and ts < '1970-01-01T00:15:06Z'")
            .unwrap();
        assert_eq!(plan.manifests_read, 2);
        assert_eq!(plan.skipped.len(), 2);
        assert_eq!(plan.tasks.len(), 1);
        let _ = std::fs::remove_dir_all(&path);
    }
}

mod located {
    //! A table reached by its location: opened, created, and opened or
    //! created by the URL of its folder, and dropped as the folder it is.

    use std::sync::Arc;

    use arrow_array::{Array, Int64Array, RecordBatch, StringArray};
    use yggdryl::arrow::BatchReader;
    use yggdryl::iceberg::{FormatVersion, IcebergTable, PartitionSpec, assign_field_ids};
    use yggdryl::local::LocalFolder;
    use yggdryl::{
        DataType, Field, IOBase, ObjectValue, Properties, StructType, TimeUnit, Timezone, Uri, Url,
    };

    /// A location nothing occupies, unique to this test and this process.
    fn root(label: &str) -> std::path::PathBuf {
        let mut path = LocalFolder::temporary().unwrap().path().unwrap();
        path.push(format!(
            "yggdryl-iceberg-located-{label}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        path
    }

    /// `(id, venue)` rows, partitioned by the venue the schema marks.
    fn schema() -> Field {
        StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::utf8()
                .nullable_field("venue")
                .with_partition(true),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row")
    }

    fn rows(ids: &[i64], venues: &[&str]) -> BatchReader {
        let schema = Arc::new(arrow_schema::Schema::new(vec![
            arrow_schema::Field::new("id", arrow_schema::DataType::Int64, false),
            arrow_schema::Field::new("venue", arrow_schema::DataType::Utf8, true),
        ]));
        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![
                Arc::new(Int64Array::from(ids.to_vec())),
                Arc::new(StringArray::from(venues.to_vec())),
            ],
        )
        .unwrap();
        yggdryl::arrow::batch_reader(schema, [batch])
    }

    /// Every id the current snapshot holds, ascending.
    fn ids<H: IOBase>(table: &IcebergTable<H>) -> Vec<i64> {
        let mut ids: Vec<i64> = table
            .scan(None)
            .unwrap()
            .flat_map(|batch| {
                let batch = batch.unwrap();
                let column = batch.column_by_name("id").unwrap();
                let column = column.as_any().downcast_ref::<Int64Array>().unwrap();
                (0..column.len())
                    .map(|row| column.value(row))
                    .collect::<Vec<_>>()
            })
            .collect();
        ids.sort_unstable();
        ids
    }

    #[test]
    fn a_location_creates_reopens_and_drops_the_table_its_folder_is() {
        let path = root("life");
        let location = Url::from_path(&path).unwrap();
        let properties = Properties::new();

        // Nothing is there: opening says so naming the folder, and makes
        // nothing on the way.
        let error = IcebergTable::from_url(&location, &properties).unwrap_err();
        assert!(error.to_string().contains("metadata"), "{error}");
        assert!(!path.exists());

        // Created with neither a version nor a spec stated: the lowest
        // version that states the schema, partitioned as it declares, the
        // columns numbered, and named after its folder.
        let mut table =
            IcebergTable::create_from_url(&location, &properties, None, schema(), None).unwrap();
        let metadata = table.metadata().unwrap();
        assert_eq!(metadata.format_version(), FormatVersion::V2);
        assert_eq!(metadata.default_spec().unwrap().fields.len(), 1);
        assert_eq!(
            table.schema().unwrap().fields()[0]
                .parquet_field_id()
                .unwrap(),
            Some(1)
        );
        assert_eq!(
            ObjectValue::name(&table),
            path.file_name().unwrap().to_str().unwrap()
        );
        assert!(path.join("metadata").is_dir());

        // A second create at the location is the typed conflict.
        let error = IcebergTable::create_from_url(&location, &properties, None, schema(), None)
            .unwrap_err();
        assert!(error.is_conflict(), "{error}");

        // Written through one handle, read through another opened by the
        // same location - as a `Url`, or as any identifier that locates it.
        table
            .commit_append(rows(&[2, 1], &["XNAS", "XLON"]))
            .unwrap();
        let opened = IcebergTable::from_url(&location, &properties).unwrap();
        assert_eq!(ids(&opened), [1, 2]);
        let by_path = IcebergTable::from_url(Uri::from_path(&path).unwrap(), &properties).unwrap();
        assert_eq!(
            by_path.metadata_file_name().unwrap(),
            opened.metadata_file_name().unwrap()
        );

        // Opening or creating opens what is there, as it is.
        let again = IcebergTable::open_or_create_from_url(
            &location,
            &properties,
            Some(FormatVersion::V3),
            schema(),
            Some(PartitionSpec::unpartitioned()),
        )
        .unwrap();
        assert_eq!(
            again.metadata().unwrap().format_version(),
            FormatVersion::V2
        );
        assert_eq!(ids(&again), [1, 2]);

        // A clone opens the location afresh, and reads the same table.
        assert_eq!(ids(&opened.clone()), [1, 2]);

        // Dropped as the folder it is: a populated one is refused without
        // `recursive`, and gone with it.
        let mut dropped = opened;
        dropped.remove(false).unwrap_err();
        assert!(path.join("metadata").is_dir());
        dropped.remove(true).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn a_create_states_its_version_and_its_spec_or_takes_the_schemas_own() {
        let path = root("layout");

        // Stated, both are taken as stated - a spec names its columns by
        // identifier, so its schema is numbered first.
        let mut numbered = schema();
        assign_field_ids(&mut numbered, 1).unwrap();
        let spec = PartitionSpec::identity(0, &numbered, &["venue"]).unwrap();
        let stated = IcebergTable::open_or_create_from_url(
            Url::from_path(path.join("stated")).unwrap(),
            &Properties::new(),
            Some(FormatVersion::V1),
            numbered,
            Some(spec.clone()),
        )
        .unwrap();
        let metadata = stated.metadata().unwrap();
        assert_eq!(metadata.format_version(), FormatVersion::V1);
        assert_eq!(metadata.default_spec().unwrap().fields, spec.fields);

        // The `format-version` property is the version of a create that
        // states none, and the table states the properties it was given.
        let properties = Properties::new().with_property("format-version", "3");
        let by_property = IcebergTable::create_from_url(
            Url::from_path(path.join("property")).unwrap(),
            &properties,
            None,
            schema(),
            Some(PartitionSpec::unpartitioned()),
        )
        .unwrap();
        let metadata = by_property.metadata().unwrap();
        assert_eq!(metadata.format_version(), FormatVersion::V3);
        assert!(metadata.default_spec().unwrap().is_unpartitioned());
        assert_eq!(
            ObjectValue::properties(&by_property)
                .unwrap()
                .get("format-version"),
            Some("3")
        );

        // A nanosecond instant is one no version before the third states.
        let nanos = StructType::from_fields([DataType::DateTime64 {
            unit: TimeUnit::Nanosecond,
            timezone: Timezone::UTC,
        }
        .required_field("ts")])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
        let third = IcebergTable::create_from_url(
            Url::from_path(path.join("nanos")).unwrap(),
            &Properties::new(),
            None,
            nanos,
            None,
        )
        .unwrap();
        assert_eq!(
            third.metadata().unwrap().format_version(),
            FormatVersion::V3
        );

        // A version no format has is refused before anything is written.
        let refused = path.join("refused");
        let error = IcebergTable::create_from_url(
            Url::from_path(&refused).unwrap(),
            &Properties::new().with_property("format-version", "7"),
            None,
            schema(),
            None,
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("$.with.format-version"),
            "{error}"
        );
        assert!(!refused.exists());
        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn a_table_rooted_at_its_location_takes_every_commit() {
        let path = root("commits");
        let location = Url::from_path(&path).unwrap();
        let mut table =
            IcebergTable::create_from_url(&location, &Properties::new(), None, schema(), None)
                .unwrap();

        // Two appends, a merge on the id within each venue, and an overwrite
        // of the one venue its rows fall in.
        table
            .commit_append(rows(&[1, 2], &["XNAS", "XLON"]))
            .unwrap();
        table.commit_append(rows(&[3], &["XNAS"])).unwrap();
        table
            .commit_merge(
                rows(&[2, 4], &["XLON", "XLON"]),
                &yggdryl::Selector::from_columns(["id"]),
                true,
            )
            .unwrap();
        assert_eq!(ids(&table), [1, 2, 3, 4]);
        table.commit_overwrite(rows(&[9], &["XNAS"])).unwrap();
        assert_eq!(ids(&table), [2, 4, 9]);

        // Maintenance rewrites through the same root.
        table.commit_append(rows(&[10], &["XNAS"])).unwrap();
        let compaction = table.compact().unwrap();
        assert!(compaction.files_after <= compaction.files_before);
        assert_eq!(ids(&table), [2, 4, 9, 10]);

        // Reopened by the location, the table is where the commits left it.
        let reopened = IcebergTable::from_url(&location, &Properties::new()).unwrap();
        assert_eq!(
            reopened.metadata_version().unwrap(),
            table.metadata_version().unwrap()
        );
        assert_eq!(ids(&reopened), [2, 4, 9, 10]);
        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn a_schema_keeps_the_partition_and_the_order_it_declares() {
        let path = root("declared");
        let mut declared = StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::utf8().required_field("venue"),
            "timestamp(us)"
                .parse::<DataType>()
                .unwrap()
                .required_field("ts"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
        declared
            .as_partition_mut()
            .set_by_texts(["venue", "minutes(ts, 15)"])
            .unwrap();
        declared
            .as_sort_mut()
            .set_by_texts(["ts desc", "id"])
            .unwrap();

        // Neither stated: the table partitions and sorts as its schema
        // declares, and says so again when its schema is read back.
        let table = IcebergTable::create_from_url(
            Url::from_path(path.join("declared")).unwrap(),
            &Properties::new(),
            None,
            declared.clone(),
            None,
        )
        .unwrap();
        let spec = table.metadata().unwrap().default_spec().unwrap();
        assert_eq!(
            spec.fields
                .iter()
                .map(|field| field.name.as_str())
                .collect::<Vec<_>>(),
            ["venue", "ts_minutes"]
        );
        let stored = table.schema().unwrap();
        assert_eq!(
            stored.get_metadata("PARTITION:by"),
            declared.get_metadata("PARTITION:by")
        );
        assert_eq!(
            stored.get_metadata("SORT:by"),
            declared.get_metadata("SORT:by")
        );

        // A spec stated replaces the declaration, the order staying the
        // schema's own.
        let mut numbered = declared.clone();
        assign_field_ids(&mut numbered, 1).unwrap();
        let stated = IcebergTable::create_from_url(
            Url::from_path(path.join("stated")).unwrap(),
            &Properties::new(),
            None,
            numbered,
            Some(PartitionSpec::unpartitioned()),
        )
        .unwrap();
        assert!(
            stated
                .metadata()
                .unwrap()
                .default_spec()
                .unwrap()
                .is_unpartitioned()
        );
        assert_eq!(
            stated.schema().unwrap().get_metadata("SORT:by"),
            declared.get_metadata("SORT:by")
        );
        let _ = std::fs::remove_dir_all(&path);
    }

    /// A located table states what it was given less what its store read.
    /// A local path reads nothing, so a table on one states every property,
    /// and one opened under nothing states none: stated on the table, never
    /// stored in it.
    #[test]
    fn a_table_on_a_local_path_states_every_property_it_was_given() {
        let path = root("stated");
        let location = Url::from_path(&path).unwrap();
        let properties = Properties::new()
            .with_property("owner", "ops")
            .with_property("write.parallelism", "2");

        let created =
            IcebergTable::create_from_url(&location, &properties, None, schema(), None).unwrap();
        let opened = IcebergTable::from_url(&location, &properties).unwrap();
        let again =
            IcebergTable::open_or_create_from_url(&location, &properties, None, schema(), None)
                .unwrap();
        for table in [&created, &opened, &again] {
            let stated = ObjectValue::properties(table).unwrap();
            assert_eq!(stated.get("owner"), Some("ops"));
            assert_eq!(stated.get("write.parallelism"), Some("2"));
        }

        let unstated = IcebergTable::from_url(&location, &Properties::new()).unwrap();
        let listed = ObjectValue::properties(&unstated).unwrap();
        assert_eq!(listed.get("owner"), None);
        assert_eq!(listed.get("write.parallelism"), None);
        let _ = std::fs::remove_dir_all(&path);
    }

    /// A table in an object-store folder is rooted on a handle that opens
    /// under everything it was given - the store takes only the key stated,
    /// and a clone, which opens again, writes through it - while the table
    /// states the properties less the ones the store read: who signs, where
    /// it is and how it is addressed are listed by nothing and printed by
    /// nothing, before the handle resolves and after. A table bucket's table
    /// states none, which `rust/tests/s3tables/catalog.rs` pins. The shared
    /// files are this test's own, so nothing of the operator's is read, and
    /// opening by location costs the store what an open costs: the version
    /// hint and the current document, one `GetObject` each.
    #[cfg(feature = "s3")]
    #[test]
    fn a_table_on_an_object_store_states_what_the_store_did_not_read() {
        let store = crate::server::FakeS3::start();
        store.create_buckets_on_write(true);
        store.require_access_key(Some("AKIAIOSFODNN7EXAMPLE"));
        let aws = root("object-store-identity");
        let properties = Properties::new()
            .with_property("owner", "ops")
            .with_property("endpoint", store.endpoint())
            .with_property("path_style", "true")
            .with_property("region", "us-east-1")
            .with_property("access_key_id", "AKIAIOSFODNN7EXAMPLE")
            .with_property("secret_access_key", "a-secret")
            .with_property("config_file", aws.join("config").display().to_string())
            .with_property(
                "shared_credentials_file",
                aws.join("credentials").display().to_string(),
            );
        let location = Url::from_str("s3://located-lake/quotes").unwrap();

        let created =
            IcebergTable::create_from_url(&location, &properties, None, schema(), None).unwrap();
        store.clear_requests();
        let opened = IcebergTable::from_url(&location, &properties).unwrap();
        let asked: Vec<(String, Option<String>)> = store
            .requests()
            .into_iter()
            .map(|request| (request.method, request.key))
            .collect();
        assert_eq!(
            asked,
            [
                (
                    "GET".to_owned(),
                    Some("quotes/metadata/version-hint.text".to_owned())
                ),
                (
                    "GET".to_owned(),
                    Some("quotes/metadata/v1.metadata.json".to_owned())
                ),
            ]
        );
        let again =
            IcebergTable::open_or_create_from_url(&location, &properties, None, schema(), None)
                .unwrap();
        let mut cloned = opened.clone();
        cloned
            .commit_append(rows(&[2, 1], &["XNAS", "XLON"]))
            .unwrap();
        assert_eq!(
            ids(&IcebergTable::from_url(&location, &properties).unwrap()),
            [1, 2]
        );
        for table in [&created, &opened, &again, &cloned] {
            let stated = ObjectValue::properties(table).unwrap();
            assert_eq!(stated.get("owner"), Some("ops"));
            for name in [
                "endpoint",
                "path_style",
                "region",
                "access_key_id",
                "secret_access_key",
                "config_file",
                "shared_credentials_file",
            ] {
                assert_eq!(stated.get(name), None, "{name}");
            }
            let printed = format!("{table:?}");
            assert!(!printed.contains("a-secret"), "{printed}");
            assert!(!printed.contains("AKIAIOSFODNN7EXAMPLE"), "{printed}");
        }
        let _ = std::fs::remove_dir_all(&aws);
    }

    /// A table bucket's location names a table only by a namespace and a
    /// name, and at most that: refused at `$.url` where it is read - or, in
    /// a build without the `s3tables` feature, by the scheme no backend of
    /// it holds.
    ///
    /// Nothing counts requests here - the fake control plane's suite does,
    /// in `rust/tests/s3tables/catalog.rs` - so the identity is stated in
    /// full and the endpoint is a closed loopback port: a request this door
    /// should not send would fail on this machine rather than leave it, and
    /// nothing of the operator's - a variable's key, a shared file - is read
    /// to sign one.
    #[test]
    fn a_table_bucket_location_that_names_no_table_is_refused_where_it_is_read() {
        let aws = root("no-identity");
        let properties = Properties::new()
            .with_property("access_key_id", "AKIAIOSFODNN7EXAMPLE")
            .with_property("secret_access_key", "a-secret")
            .with_property("config_file", aws.join("config").display().to_string())
            .with_property(
                "shared_credentials_file",
                aws.join("credentials").display().to_string(),
            )
            .with_property("s3tables.region", "us-east-1")
            .with_property("s3tables.endpoint", "http://127.0.0.1:1");
        for location in [
            "s3tables://lake/a/b/c",
            "s3tables://lake",
            "s3tables://lake/desk",
        ] {
            let location = Url::from_str(location).unwrap();
            let error = IcebergTable::from_url(&location, &properties).unwrap_err();
            #[cfg(feature = "s3tables")]
            assert!(error.to_string().contains("$.url"), "{location}: {error}");
            #[cfg(not(feature = "s3tables"))]
            assert!(
                matches!(error, yggdryl::Error::Unsupported { .. }),
                "{location}: {error}"
            );
        }
    }
}
