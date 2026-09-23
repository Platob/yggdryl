//! `rust/src/iceberg/table.rs`: what a commit decides on the way through.
//!
//! A caller sees one call succeed or conflict, so the decisions inside it are
//! pinned here one at a time - the retry ladder's budget and jitter, the merge
//! pruning a malformed or NaN bound must never win, the partition summary a
//! NaN must not enter, the metadata names a listing accepts, and the relative
//! location two writers spell differently. All of it comes through
//! `yggdryl::internals`; what a caller observes of a committed table is pinned
//! in `rust/tests/iceberg/mod_.rs`.

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
        FormatVersion, IcebergOptions, PartitionSpec, SortField, SortOrder, Table, TableMetadata,
        Transform, assign_field_ids,
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
    fn data_files_are_sorted_by_the_default_order_the_metadata_records() {
        let path = root("sorted-files");
        let schema = schema();
        let spec = PartitionSpec::identity(1, &schema, &["venue"]).unwrap();
        let table = Table::create(
            LocalFolder::new(&path).unwrap(),
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
        let document = sorted.metadata().clone().into_json().unwrap();
        let reread = TableMetadata::from_json(&document).unwrap();
        assert_eq!(reread.default_sort_order().unwrap(), order_by_symbol());
        reread.validate().unwrap();

        // Explicitly unsorted: rows stay as they arrived, files carry no order.
        let plain = root("unsorted");
        let mut unsorted = Table::create_sorted(
            LocalFolder::new(&plain).unwrap(),
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

    #[test]
    fn batches_changing_layout_mid_commit_each_cast_to_the_table_schema() {
        let path = root("table_layouts");
        let schema = schema();
        let mut table = Table::create(
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
}
