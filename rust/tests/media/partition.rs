//! `rust/src/media/partition.rs`: hive partition columns, projected in and out
//! of Arrow data, and the lake layout they write.

mod lake {
    use super::{Field, IOBase, RecordOptions, partitions, prices, schema, with_partitions};

    use std::path::{Path, PathBuf};

    use arrow_array::{Array, Int32Array, RecordBatch, RecordBatchIterator, StringArray};
    use arrow_schema::ArrowError;

    use yggdryl::DataType;
    use yggdryl::IOMedia;
    use yggdryl::StructType;
    use yggdryl::holder::Holder;
    use yggdryl::media::IORecordOptions;

    /// Build an empty `lake/` under the temp directory and hold it as a folder.
    fn lake(label: &str) -> (PathBuf, Holder) {
        let mut root = yggdryl::local::LocalFolder::temporary()
            .unwrap()
            .path()
            .unwrap();
        root.push(format!("yggdryl-lake-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let holder = Holder::folder(&root).unwrap();
        (root, holder)
    }

    /// The options a partitioned Arrow IPC lake is read and written under.
    fn options(field: Option<Field>) -> RecordOptions {
        let options = RecordOptions::for_mime_type(&yggdryl::MimeType::ARROW_STREAM).unwrap();
        match field {
            Some(field) => options.with_field(field),
            None => options,
        }
    }

    /// Seed one partition directly, which is how a layout comes into being.
    fn seed(root: &Path, directory: &str, batch: &RecordBatch) {
        let mut leaf = Holder::folder(root.join(directory))
            .unwrap()
            .child_by_path("part-0.arrows")
            .unwrap();
        leaf.overwrite_arrow_reader(
            yggdryl::arrow::batch_reader(batch.schema(), [batch.clone()]),
            &options(None),
        )
        .unwrap();
        leaf.flush().unwrap();
    }

    /// Every row a lake holds, as `(year, month, price)`.
    fn rows(handle: &Holder, field: &Field) -> Vec<(i32, String, i64)> {
        let mut found = Vec::new();
        for batch in handle
            .read_arrow_reader(&options(Some(field.clone())))
            .unwrap()
        {
            let batch = batch.unwrap();
            let price = batch
                .column_by_name("price")
                .unwrap()
                .as_any()
                .downcast_ref::<arrow_array::Int64Array>()
                .unwrap()
                .clone();
            let year = batch
                .column_by_name("year")
                .unwrap()
                .as_any()
                .downcast_ref::<Int32Array>()
                .unwrap()
                .clone();
            let month = batch
                .column_by_name("month")
                .unwrap()
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap()
                .clone();
            for row in 0..batch.num_rows() {
                found.push((
                    year.value(row),
                    month.value(row).to_owned(),
                    price.value(row),
                ));
            }
        }
        found.sort_unstable();
        found
    }

    #[test]
    fn a_folder_reads_as_the_table_beneath_it() {
        let (root, handle) = lake("read");
        seed(&root, "year=2024/month=01", &prices());

        // The folder holds no bytes of its own, so the encoding comes from what
        // is under it rather than from a media type it does not have.
        assert!(matches!(
            handle.record_options().unwrap(),
            RecordOptions::Ipc(_)
        ));

        let field = schema();
        // The leaf stores one column; the read restores the other two from the
        // directory names and types them as the schema declares.
        assert_eq!(
            rows(&handle, &field),
            vec![
                (2024, "01".to_owned(), 10),
                (2024, "01".to_owned(), 20),
                (2024, "01".to_owned(), 30),
            ]
        );
        let read = handle.read_arrow_reader(&options(Some(field))).unwrap();
        assert_eq!(read.schema().fields().len(), 3);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_folder_write_routes_each_row_to_the_partition_it_belongs_to() {
        let (root, mut handle) = lake("route");
        // One partition exists, which is what tells the folder that `year` and
        // `month` are its partition columns.
        seed(&root, "year=2024/month=01", &prices());

        let field = schema();
        let mut incoming = with_partitions(&prices(), &partitions(), Some(&field)).unwrap();
        // Move the rows into a partition that does not exist yet.
        incoming = RecordBatch::try_new(
            incoming.schema(),
            vec![
                incoming.column(0).clone(),
                std::sync::Arc::new(Int32Array::from(vec![2025, 2025, 2024])),
                std::sync::Arc::new(StringArray::from(vec!["02", "02", "01"])),
            ],
        )
        .unwrap();

        handle
            .overwrite_arrow_reader(
                yggdryl::arrow::batch_reader(incoming.schema(), [incoming]),
                &options(Some(field.clone())),
            )
            .unwrap();

        // The new partition was created and the old one was replaced because
        // the caller selected overwrite explicitly.
        assert!(root.join("year=2025").join("month=02").is_dir());
        assert_eq!(
            rows(&handle, &field),
            vec![
                (2024, "01".to_owned(), 30),
                (2025, "02".to_owned(), 10),
                (2025, "02".to_owned(), 20),
            ]
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_ascii_partition_column_is_spelled_as_text_in_the_path() {
        let (root, mut handle) = lake("ascii");
        let field = StructType::from_fields([
            DataType::fixed_ascii(4).unwrap().required_field("ccy"),
            DataType::Int64.required_field("qty"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row")
        .with_partition_fields(&["ccy"])
        .unwrap();
        let incoming = RecordBatch::try_from_iter([
            (
                "ccy",
                std::sync::Arc::new(StringArray::from(vec!["USD", "EUR", "USD"]))
                    as arrow_array::ArrayRef,
            ),
            (
                "qty",
                std::sync::Arc::new(arrow_array::Int64Array::from(vec![1, 2, 3])),
            ),
        ])
        .unwrap();

        handle
            .overwrite_arrow_reader(
                yggdryl::arrow::batch_reader(incoming.schema(), [incoming]),
                &options(Some(field.clone())),
            )
            .unwrap();

        // The directory carries the trimmed text, never the padded storage's
        // hex, and the read restores the padded storage from it.
        assert!(root.join("ccy=USD").is_dir());
        assert!(root.join("ccy=EUR").is_dir());
        let mut found = Vec::new();
        for batch in handle.read_arrow_reader(&options(Some(field))).unwrap() {
            let batch = batch.unwrap();
            let ccy = batch
                .column_by_name("ccy")
                .unwrap()
                .as_any()
                .downcast_ref::<arrow_array::FixedSizeBinaryArray>()
                .unwrap()
                .clone();
            for row in 0..batch.num_rows() {
                found.push(ccy.value(row).to_vec());
            }
        }
        found.sort_unstable();
        assert_eq!(
            found,
            vec![b"EUR\0".to_vec(), b"USD\0".to_vec(), b"USD\0".to_vec()]
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn every_incoming_batch_spells_its_partition_the_same_way() {
        let (root, mut handle) = lake("ascii-batches");
        let field = StructType::from_fields([
            DataType::fixed_ascii(4).unwrap().required_field("ccy"),
            DataType::Int64.required_field("qty"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row")
        .with_partition_fields(&["ccy"])
        .unwrap();
        let batch = |ccy: Vec<&str>, qty: Vec<i64>| {
            RecordBatch::try_from_iter([
                (
                    "ccy",
                    std::sync::Arc::new(StringArray::from(ccy)) as arrow_array::ArrayRef,
                ),
                (
                    "qty",
                    std::sync::Arc::new(arrow_array::Int64Array::from(qty)),
                ),
            ])
            .unwrap()
        };
        let first = batch(vec!["USD", "EUR"], vec![1, 2]);
        let second = batch(vec!["EUR", "USD"], vec![3, 4]);

        handle
            .overwrite_arrow_reader(
                yggdryl::arrow::batch_reader(first.schema(), [first, second]),
                &options(Some(field.clone())),
            )
            .unwrap();

        // One directory per currency, whichever batch its rows arrived in.
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 2);
        let mut found = Vec::new();
        for batch in handle.read_arrow_reader(&options(Some(field))).unwrap() {
            let batch = batch.unwrap();
            let ccy = batch
                .column_by_name("ccy")
                .unwrap()
                .as_any()
                .downcast_ref::<arrow_array::FixedSizeBinaryArray>()
                .unwrap()
                .clone();
            let qty = batch
                .column_by_name("qty")
                .unwrap()
                .as_any()
                .downcast_ref::<arrow_array::Int64Array>()
                .unwrap()
                .clone();
            for row in 0..batch.num_rows() {
                found.push((ccy.value(row).to_vec(), qty.value(row)));
            }
        }
        found.sort_unstable();
        assert_eq!(
            found,
            vec![
                (b"EUR\0".to_vec(), 2),
                (b"EUR\0".to_vec(), 3),
                (b"USD\0".to_vec(), 1),
                (b"USD\0".to_vec(), 4),
            ]
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_code_partition_column_keeps_its_identity_through_the_path() {
        let (root, mut handle) = lake("code");
        let field = StructType::from_fields([
            DataType::Ccy.required_field("ccy"),
            DataType::Int64.required_field("qty"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row")
        .with_partition_fields(&["ccy"])
        .unwrap();
        let incoming = RecordBatch::try_from_iter([
            (
                "ccy",
                std::sync::Arc::new(StringArray::from(vec!["USD", "EUR", "USD"]))
                    as arrow_array::ArrayRef,
            ),
            (
                "qty",
                std::sync::Arc::new(arrow_array::Int64Array::from(vec![1, 2, 3])),
            ),
        ])
        .unwrap();

        handle
            .overwrite_arrow_reader(
                yggdryl::arrow::batch_reader(incoming.schema(), [incoming]),
                &options(Some(field.clone())),
            )
            .unwrap();

        // A code renders as its text in the path exactly as a width does.
        assert!(root.join("ccy=USD").is_dir());
        assert!(root.join("ccy=EUR").is_dir());

        let mut found = Vec::new();
        for batch in handle.read_arrow_reader(&options(Some(field))).unwrap() {
            let batch = batch.unwrap();
            // A code stores as the text it is, so the restored column holds the
            // path's own spelling and reads back a currency.
            let restored = yggdryl::Field::from_arrow_field(batch.schema().field(0)).unwrap();
            assert_eq!(restored.dtype(), &DataType::Ccy);
            let ccy = batch
                .column_by_name("ccy")
                .unwrap()
                .as_any()
                .downcast_ref::<arrow_array::StringArray>()
                .unwrap()
                .clone();
            for row in 0..batch.num_rows() {
                found.push(ccy.value(row).to_string());
            }
        }
        found.sort_unstable();
        assert_eq!(found, vec!["EUR", "USD", "USD"]);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_partitioned_folder_keeps_only_complete_commit_prefixes() {
        let (root, mut handle) = lake("commit-prefix");
        let field = schema().with_partition_fields(&["year", "month"]).unwrap();
        let full = with_partitions(&prices(), &partitions(), Some(&field)).unwrap();
        let reader = Box::new(RecordBatchIterator::new(
            [
                Ok(full.clone()),
                Err(ArrowError::ComputeError(
                    "later partition source failure".into(),
                )),
            ],
            full.schema(),
        ));
        let committed = options(Some(field.clone())).with_commit_row_size(2);

        let message = handle
            .overwrite_arrow_reader(reader, &committed)
            .unwrap_err()
            .to_string();

        assert!(
            message.contains("later partition source failure"),
            "{message}"
        );
        assert_eq!(
            rows(&handle, &field),
            vec![(2024, "01".to_owned(), 10), (2024, "01".to_owned(), 20)]
        );
        assert!(
            root.join("year=2024")
                .join("month=01")
                .join("part-0.arrows")
                .is_file()
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_empty_folder_overwrite_keeps_an_inferable_encoded_field() {
        let (root, mut handle) = lake("empty-overwrite-field");
        seed(&root, "year=2024/month=01", &prices());
        let field = schema();
        let arrow = field.clone().into_arrow_schema().unwrap();

        // No declared field is available on either side of this call: the
        // empty reader's Arrow schema must remain discoverable from a real
        // encoded leaf rather than a zero-byte remnant.
        handle
            .overwrite_arrow_reader(yggdryl::arrow::batch_reader(arrow, []), &options(None))
            .unwrap();

        let inferred = handle.read_arrow_field(&options(None)).unwrap();
        assert_eq!(
            inferred
                .fields()
                .iter()
                .map(|field| field.name())
                .collect::<Vec<_>>(),
            vec!["price", "year", "month"]
        );
        assert_eq!(handle.read_arrow_reader(&options(None)).unwrap().count(), 0);
        assert!(
            std::fs::metadata(
                root.join("year=2024")
                    .join("month=01")
                    .join("part-0.arrows")
            )
            .unwrap()
            .len()
                > 0
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn merge_keys_that_exist_only_in_partition_paths_are_refused() {
        for cadence in [None, Some(1)] {
            let label = cadence.map_or("unbounded", |_| "bounded");
            let (root, mut handle) = lake(&format!("path-only-merge-{label}"));
            seed(&root, "year=2024/month=01", &prices());
            let field = schema();
            let incoming = with_partitions(&prices(), &partitions(), Some(&field)).unwrap();
            let mut merging = options(Some(field.clone()))
                .with_merge_by(["year", "month"])
                .unwrap();
            merging.set_commit_row_size(cadence);

            let message = handle
                .merge_arrow_reader(
                    yggdryl::arrow::batch_reader(incoming.schema(), [incoming]),
                    &merging,
                )
                .unwrap_err()
                .to_string();

            assert!(message.contains("merge_by"), "{label}: {message}");
            assert!(message.contains("partition"), "{label}: {message}");
            assert_eq!(
                rows(&handle, &field),
                vec![
                    (2024, "01".to_owned(), 10),
                    (2024, "01".to_owned(), 20),
                    (2024, "01".to_owned(), 30),
                ],
                "{label}"
            );

            let _ = std::fs::remove_dir_all(&root);
        }
    }

    #[test]
    fn a_partition_filter_prunes_leaves_and_filters_rows() {
        use yggdryl::media::IORecordOptions;

        let (root, handle) = lake("filtered");
        seed(&root, "year=2024/month=01", &prices());
        seed(&root, "year=2024/month=02", &prices());
        seed(&root, "year=2025/month=01", &prices());

        let field = schema();
        // A path filter prunes: only the leaves whose directories name the
        // value are read at all, and the restored column carries it.
        let filtered = options(Some(field.clone()))
            .with_filter("month = '01' and year = '2024'")
            .unwrap();
        let mut found = Vec::new();
        for batch in handle.read_arrow_reader(&filtered).unwrap() {
            let batch = batch.unwrap();
            found.push(batch.num_rows());
            let month = batch
                .column_by_name("month")
                .unwrap()
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap()
                .clone();
            for row in 0..batch.num_rows() {
                assert_eq!(month.value(row), "01");
            }
        }
        assert_eq!(found.iter().sum::<usize>(), 3);

        // A data-carried column filters row by row through the same option:
        // the same lake, filtered on a column the paths do not spell.
        let by_price = options(Some(field.clone()))
            .with_filter("price = '20'")
            .unwrap();
        let mut matched = 0;
        for batch in handle.read_arrow_reader(&by_price).unwrap() {
            matched += batch.unwrap().num_rows();
        }
        assert_eq!(matched, 3);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_overwrite_empties_a_partition_the_incoming_rows_no_longer_name() {
        let (root, mut handle) = lake("empty-partition");
        seed(&root, "year=2024/month=01", &prices());
        seed(&root, "year=2024/month=02", &prices());

        let field = schema();
        let only_january = with_partitions(&prices(), &partitions(), Some(&field)).unwrap();
        handle
            .overwrite_arrow_reader(
                yggdryl::arrow::batch_reader(only_january.schema(), [only_january]),
                &options(Some(field.clone())),
            )
            .unwrap();

        // February still exists as a location and reads as no rows, which is
        // what the laziness contract says an empty resource is.
        assert!(root.join("year=2024").join("month=02").is_dir());
        assert_eq!(
            rows(&handle, &field),
            vec![
                (2024, "01".to_owned(), 10),
                (2024, "01".to_owned(), 20),
                (2024, "01".to_owned(), 30),
            ]
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_append_adds_to_one_partition_and_leaves_the_others_alone() {
        let (root, mut handle) = lake("append");
        seed(&root, "year=2024/month=01", &prices());
        seed(&root, "year=2024/month=02", &prices());

        let field = schema();
        let january = with_partitions(&prices(), &partitions(), Some(&field)).unwrap();
        handle
            .append_arrow_reader(
                yggdryl::arrow::batch_reader(january.schema(), [january]),
                &options(Some(field.clone())),
            )
            .unwrap();

        let found = rows(&handle, &field);
        assert_eq!(
            found.iter().filter(|(_, month, _)| month == "01").count(),
            6
        );
        assert_eq!(
            found.iter().filter(|(_, month, _)| month == "02").count(),
            3
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_merge_across_a_folder_updates_inside_each_partition() {
        let (root, mut handle) = lake("merge");
        seed(&root, "year=2024/month=01", &prices());
        seed(&root, "year=2024/month=02", &prices());

        let field = schema();
        // `price` is the key here, so 10 updates in place and 40 appends - and
        // only inside the partition the row's values name.
        let mut updates = with_partitions(&prices(), &partitions(), Some(&field)).unwrap();
        updates = RecordBatch::try_new(
            updates.schema(),
            vec![
                std::sync::Arc::new(arrow_array::Int64Array::from(vec![10, 40, 20])),
                updates.column(1).clone(),
                updates.column(2).clone(),
            ],
        )
        .unwrap();

        handle
            .merge_arrow_reader(
                yggdryl::arrow::batch_reader(updates.schema(), [updates]),
                &options(Some(field.clone()))
                    .with_merge_by(["price"])
                    .unwrap(),
            )
            .unwrap();

        let found = rows(&handle, &field);
        // January gained exactly one row; February was never addressed.
        assert_eq!(
            found.iter().filter(|(_, month, _)| month == "01").count(),
            4
        );
        assert_eq!(
            found.iter().filter(|(_, month, _)| month == "02").count(),
            3
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_empty_partition_directory_is_enough_to_declare_the_layout() {
        let (root, mut handle) = lake("declared");
        // Nothing has been written yet: the directories alone say which columns
        // belong in the path, which is how a tree comes into being.
        std::fs::create_dir_all(root.join("year=1900").join("month=12")).unwrap();

        let field = schema();
        let full = with_partitions(&prices(), &partitions(), Some(&field)).unwrap();
        handle
            .overwrite_arrow_reader(
                yggdryl::arrow::batch_reader(full.schema(), [full]),
                &options(Some(field.clone())),
            )
            .unwrap();

        // The rows landed in the partition their own values name, not in the
        // one that happened to exist.
        assert!(root.join("year=2024").join("month=01").is_dir());
        assert_eq!(
            rows(&handle, &field),
            vec![
                (2024, "01".to_owned(), 10),
                (2024, "01".to_owned(), 20),
                (2024, "01".to_owned(), 30),
            ]
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_declared_schema_lays_an_empty_folder_out_by_its_partition_fields() {
        let (root, mut handle) = lake("declared-schema");
        // Nothing is on disk, so nothing spells a layout. The schema does: it
        // marks the two columns a path is supposed to carry.
        let field = schema().with_partition_fields(&["year", "month"]).unwrap();
        let full = with_partitions(&prices(), &partitions(), Some(&field)).unwrap();

        handle
            .overwrite_arrow_reader(
                yggdryl::arrow::batch_reader(full.schema(), [full]),
                &options(Some(field.clone())),
            )
            .unwrap();

        // The directories were created from the declaration, and the leaf keeps
        // only the column the path does not carry.
        assert!(root.join("year=2024").join("month=01").is_dir());
        assert!(
            root.join("year=2024")
                .join("month=01")
                .join("part-0.arrows")
                .is_file()
        );
        assert_eq!(
            rows(&handle, &field),
            vec![
                (2024, "01".to_owned(), 10),
                (2024, "01".to_owned(), 20),
                (2024, "01".to_owned(), 30),
            ]
        );

        // Reading the tree back reports the same layout without being told it.
        let derived = handle.read_arrow_field(&options(None)).unwrap();
        assert_eq!(
            derived.partition_field_names().collect::<Vec<_>>(),
            ["year", "month"]
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_declared_layout_that_contradicts_the_stored_one_is_refused_by_name() {
        let (root, mut handle) = lake("contradicting-layout");
        seed(&root, "year=2024/month=01", &prices());

        // The tree is laid out by year and month; the schema says venue. One
        // write cannot mean both, so it says so instead of choosing.
        let field = schema()
            .try_with_dtype(
                StructType::from_fields([
                    DataType::Int64.required_field("price"),
                    DataType::Int32.required_field("year"),
                    DataType::utf8().required_field("month"),
                    DataType::utf8().required_field("venue"),
                ])
                .map(DataType::from)
                .unwrap(),
            )
            .unwrap()
            .with_partition_fields(&["venue"])
            .unwrap();
        let full = with_partitions(&prices(), &partitions(), Some(&field)).unwrap();
        let error = handle
            .overwrite_arrow_reader(
                yggdryl::arrow::batch_reader(full.schema(), [full]),
                &options(Some(field)),
            )
            .expect_err("two layouts in one tree");
        let message = error.to_string();
        assert!(message.contains("year, month"), "{message}");
        assert!(message.contains("venue"), "{message}");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_folder_with_no_partition_directories_is_one_table_in_one_leaf() {
        let (root, mut handle) = lake("flat");
        let field = schema();
        let full = with_partitions(&prices(), &partitions(), Some(&field)).unwrap();

        handle
            .overwrite_arrow_reader(
                yggdryl::arrow::batch_reader(full.schema(), [full]),
                &options(Some(field.clone())),
            )
            .unwrap();

        // Nothing said which columns belong in a path, so every column is
        // stored and the leaf is named after the encoding.
        assert!(root.join("part-0.arrows").is_file());
        assert_eq!(rows(&handle, &field).len(), 3);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_null_partition_value_is_spelled_out_in_the_path() {
        let (root, mut handle) = lake("null-partition");
        seed(&root, "year=2024/month=01", &prices());

        let field = yggdryl::StructType::from_fields([
            yggdryl::DataType::Int64.required_field("price"),
            yggdryl::DataType::Int32.nullable_field("year"),
            yggdryl::DataType::utf8().nullable_field("month"),
        ])
        .map(yggdryl::DataType::from)
        .unwrap()
        .required_field("row");
        let batch = RecordBatch::try_new(
            field.clone().into_arrow_schema().unwrap(),
            vec![
                std::sync::Arc::new(arrow_array::Int64Array::from(vec![99])),
                std::sync::Arc::new(Int32Array::from(vec![None::<i32>])),
                std::sync::Arc::new(StringArray::from(vec![None::<&str>])),
            ],
        )
        .unwrap();

        handle
            .overwrite_arrow_reader(
                yggdryl::arrow::batch_reader(batch.schema(), [batch]),
                &options(Some(field.clone())),
            )
            .unwrap();

        assert!(root.join("year=null").join("month=null").is_dir());
        // A path cannot say whether that is the absence or the three letters,
        // so a nullable declared column reads the text back as a null.
        let read: Vec<RecordBatch> = handle
            .read_arrow_reader(&options(Some(field)))
            .unwrap()
            .map(std::result::Result::unwrap)
            .collect();
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].column_by_name("year").unwrap().null_count(), 1);

        let _ = std::fs::remove_dir_all(&root);
    }
}

use std::sync::Arc;

use arrow_array::{Array, ArrayRef, Int32Array, Int64Array, RecordBatch, StringArray};

use yggdryl::media::RecordOptions;
use yggdryl::media::partition::{partitioned_reader, with_partitions, without_partitions};
use yggdryl::{ArrowCastOptions, DataType, Field, IOBase, Serie, StructType};

fn schema() -> Field {
    StructType::from_fields([
        DataType::Int64.required_field("price"),
        DataType::Int32.required_field("year"),
        DataType::utf8().required_field("month"),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("row")
}

fn prices() -> RecordBatch {
    RecordBatch::try_from_iter([(
        "price",
        Arc::new(Int64Array::from(vec![10, 20, 30])) as ArrayRef,
    )])
    .unwrap()
}

fn partitions() -> Vec<(String, String)> {
    vec![
        ("year".to_owned(), "2024".to_owned()),
        ("month".to_owned(), "01".to_owned()),
    ]
}

#[test]
fn restored_columns_take_the_type_the_schema_declares() {
    let restored = with_partitions(&prices(), &partitions(), Some(&schema())).unwrap();

    assert_eq!(restored.num_columns(), 3);
    assert_eq!(restored.num_rows(), 3);

    let year = restored
        .column_by_name("year")
        .unwrap()
        .as_any()
        .downcast_ref::<Int32Array>()
        .expect("an Int32 column, as the schema declares");
    assert_eq!(year.values(), &[2024, 2024, 2024]);

    let month = restored
        .column_by_name("month")
        .unwrap()
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("a Utf8 column");
    assert_eq!(month.value(0), "01");
    // A path value is spelled out, so it is never null.
    assert!(!restored.schema().field(1).is_nullable());
}

#[test]
fn a_literal_dotted_partition_name_keeps_its_declared_type() {
    let declared = StructType::from_fields([
        DataType::Int64.required_field("price"),
        DataType::Int32.required_field("a.b"),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("row");

    let restored = with_partitions(
        &prices(),
        &[("a.b".to_owned(), "2024".to_owned())],
        Some(&declared),
    )
    .unwrap();

    let dotted = restored
        .column_by_name("a.b")
        .unwrap()
        .as_any()
        .downcast_ref::<Int32Array>()
        .expect("an Int32 column, as the literal declaration says");
    assert_eq!(dotted.values(), &[2024, 2024, 2024]);
}

#[test]
fn an_ascii_partition_column_is_restored_padded_with_its_identity() {
    let declared = StructType::from_fields([
        DataType::Int64.required_field("price"),
        DataType::fixed_ascii(4).unwrap().required_field("ccy"),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("row");

    let restored = with_partitions(
        &prices(),
        &[("ccy".to_owned(), "USD".to_owned())],
        Some(&declared),
    )
    .unwrap();

    // The path spells the trimmed text; the column holds the padded storage
    // and keeps the extension identity the declaration carries.
    let ccy = restored
        .column_by_name("ccy")
        .unwrap()
        .as_any()
        .downcast_ref::<arrow_array::FixedSizeBinaryArray>()
        .expect("the ASCII storage, as the schema declares");
    assert_eq!(ccy.value(0), b"USD\0");
    let field = Field::from_arrow_field(restored.schema().field(1)).unwrap();
    assert_eq!(field.dtype(), &DataType::fixed_ascii(4).unwrap());
    assert!(field.is_partition());
}

#[test]
fn a_restored_column_is_text_when_no_schema_says_otherwise() {
    let restored = with_partitions(&prices(), &partitions(), None).unwrap();

    assert!(
        restored
            .column_by_name("year")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .is_some()
    );
}

#[test]
fn a_column_the_data_already_carries_is_left_alone() {
    let batch = RecordBatch::try_from_iter([
        ("price", Arc::new(Int64Array::from(vec![10])) as ArrayRef),
        ("year", Arc::new(Int32Array::from(vec![1999])) as ArrayRef),
    ])
    .unwrap();

    let restored = with_partitions(&batch, &partitions(), Some(&schema())).unwrap();

    assert_eq!(restored.num_columns(), 3);
    let year = restored
        .column_by_name("year")
        .unwrap()
        .as_any()
        .downcast_ref::<Int32Array>()
        .unwrap();
    // The stored value wins, so a mismatch stays visible instead of being
    // rewritten from the directory name.
    assert_eq!(year.values(), &[1999]);
}

#[test]
fn a_value_that_does_not_fit_its_declared_type_is_an_error() {
    let broken = vec![("year".to_owned(), "not-a-year".to_owned())];

    assert!(with_partitions(&prices(), &broken, Some(&schema())).is_err());
}

#[test]
fn nothing_changes_without_partitions() {
    let batch = prices();

    assert_eq!(with_partitions(&batch, &[], None).unwrap(), batch);
    assert_eq!(without_partitions(&batch, &[]).unwrap(), batch);
    // A partition the batch does not carry removes nothing.
    assert_eq!(without_partitions(&batch, &partitions()).unwrap(), batch);
}

#[test]
fn removing_every_column_keeps_the_row_count() {
    let batch = RecordBatch::try_from_iter([(
        "year",
        Arc::new(Int32Array::from(vec![2024, 2024])) as ArrayRef,
    )])
    .unwrap();

    let narrowed = without_partitions(&batch, &partitions()).unwrap();

    assert_eq!(narrowed.num_columns(), 0);
    assert_eq!(narrowed.num_rows(), 2);
}

#[test]
fn the_reader_reports_the_widened_schema_before_the_first_batch() {
    let inner: yggdryl::arrow::BatchReader = Box::new(arrow_array::RecordBatchIterator::new(
        [Ok(prices())],
        prices().schema(),
    ));

    let reader = partitioned_reader(inner, partitions(), Some(schema())).unwrap();

    assert_eq!(reader.schema().fields().len(), 3);
    let batches: Vec<_> = reader.collect::<std::result::Result<Vec<_>, _>>().unwrap();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].num_columns(), 3);
}

#[test]
fn every_batch_of_a_location_restores_its_columns_at_its_own_length() {
    let batch = |prices: Vec<i64>| {
        RecordBatch::try_from_iter([("price", Arc::new(Int64Array::from(prices)) as ArrayRef)])
            .unwrap()
    };
    let batches = vec![batch(vec![10, 20]), batch(vec![]), batch(vec![30, 40, 50])];
    let inner: yggdryl::arrow::BatchReader = Box::new(arrow_array::RecordBatchIterator::new(
        batches.clone().into_iter().map(Ok),
        prices().schema(),
    ));

    let restored = partitioned_reader(inner, partitions(), Some(schema()))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();

    // The path's value is read once and repeated into every batch, at that
    // batch's length and as the one-shot restoration of its rows would be.
    assert_eq!(restored.len(), batches.len());
    for (restored, batch) in restored.iter().zip(&batches) {
        assert_eq!(
            restored,
            &with_partitions(batch, &partitions(), Some(&schema())).unwrap()
        );
        let year = restored
            .column_by_name("year")
            .unwrap()
            .as_any()
            .downcast_ref::<Int32Array>()
            .unwrap();
        assert_eq!(year, &Int32Array::from(vec![2024; batch.num_rows()]));
        let month = restored
            .column_by_name("month")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(month, &StringArray::from(vec!["01"; batch.num_rows()]));
    }
}

#[test]
fn a_path_value_its_column_refuses_is_refused_as_the_whole_column_would_be() {
    let broken = vec![("year".to_owned(), "not-a-year".to_owned())];
    let year = DataType::Int32.required_field("year");
    let column: ArrayRef = Arc::new(StringArray::from(vec!["not-a-year"; 3]));
    let expected = Serie::from_arrow_array(
        Some(&year),
        column,
        ArrowCastOptions::new().with_safe(false),
    )
    .unwrap_err()
    .to_string();

    // The value is read once and repeated, and its refusal is the one the
    // batch's whole column of it would raise.
    let refused = with_partitions(&prices(), &broken, Some(&schema()))
        .unwrap_err()
        .to_string();
    assert!(refused.contains(&expected), "{refused}");

    let inner: yggdryl::arrow::BatchReader = Box::new(arrow_array::RecordBatchIterator::new(
        [Ok(RecordBatch::new_empty(prices().schema())), Ok(prices())],
        prices().schema(),
    ));
    let mut reader = partitioned_reader(inner, broken, Some(schema())).unwrap();
    // A batch with no rows holds nothing to refuse.
    assert_eq!(reader.next().unwrap().unwrap().num_rows(), 0);
    let refused = reader.next().unwrap().unwrap_err().to_string();
    assert!(refused.contains(&expected), "{refused}");
}

/// A declared folder shape must leave both its listing and its leaves lazy.
/// A root declaring `year` as `year(event)` beside the column it reads.
fn derived_schema() -> Field {
    let mut year = DataType::Int32.nullable_field("year");
    year.as_partition_mut().set_sources(["event"]).unwrap();
    year.as_partition_mut()
        .set_transform(yggdryl::expression::Function::Year)
        .unwrap();
    StructType::from_fields([DataType::date32().required_field("event"), year])
        .map(DataType::from)
        .unwrap()
        .required_field("row")
}

/// Two days, one in 2024 and one in 2025.
fn events() -> RecordBatch {
    RecordBatch::try_from_iter([(
        "event",
        Arc::new(arrow_array::Date32Array::from(vec![19_723, 20_089])) as ArrayRef,
    )])
    .unwrap()
}

#[test]
fn a_declared_derived_column_the_batch_lacks_is_computed_from_its_source() {
    let filled = derived_schema()
        .as_transform()
        .apply_arrow_batch(&events())
        .unwrap();

    assert_eq!(filled.num_columns(), 2);
    assert_eq!(filled.num_rows(), 2);
    let year = filled
        .column_by_name("year")
        .unwrap()
        .as_any()
        .downcast_ref::<Int32Array>()
        .expect("an Int32 column, as the schema declares");
    assert_eq!(year.values(), &[2024, 2025]);
}

#[test]
fn a_derived_column_is_not_marked_as_one_a_path_spells_out() {
    let filled = derived_schema()
        .as_transform()
        .apply_arrow_batch(&events())
        .unwrap();

    // `FIELD:partition` says a directory carries the column. This one is
    // computed from the rows, so the declaration travels unchanged.
    let declared = Field::from_arrow_field(filled.schema().field(1)).unwrap();
    assert!(!declared.is_partition());
    assert_eq!(
        declared.as_partition().sources().unwrap(),
        Some(vec!["event".to_owned()])
    );
}

#[test]
fn a_derived_column_carrying_values_is_left_alone() {
    let batch = RecordBatch::try_from_iter([
        (
            "event",
            Arc::new(arrow_array::Date32Array::from(vec![19_723])) as ArrayRef,
        ),
        ("year", Arc::new(Int32Array::from(vec![1999])) as ArrayRef),
    ])
    .unwrap();

    let filled = derived_schema()
        .as_transform()
        .apply_arrow_batch(&batch)
        .unwrap();

    // The stored value wins, exactly as it does against a directory name, so a
    // mismatch stays visible instead of being recomputed away.
    let year = filled
        .column_by_name("year")
        .unwrap()
        .as_any()
        .downcast_ref::<Int32Array>()
        .unwrap();
    assert_eq!(year.values(), &[1999]);
}

#[test]
fn a_column_holding_nothing_but_nulls_is_filled_from_the_batchs_own_schema() {
    // A batch cast to its root carries the declared column null-filled, which
    // is a column that was never written rather than one written null.
    let placeholder =
        Serie::from_arrow_batch(Some(&derived_schema()), &events(), ArrowCastOptions::new())
            .unwrap()
            .into_arrow_batch()
            .unwrap();
    assert_eq!(placeholder.column(1).null_count(), 2);

    let filled = Field::from_arrow_schema("row", placeholder.schema().as_ref())
        .unwrap()
        .as_transform()
        .apply_arrow_batch(&placeholder)
        .unwrap();

    let year = filled
        .column_by_name("year")
        .unwrap()
        .as_any()
        .downcast_ref::<Int32Array>()
        .unwrap();
    assert_eq!(year.values(), &[2024, 2025]);
    assert_eq!(filled.num_columns(), 2);
}

#[test]
fn an_absent_transform_copies_the_source_value_unchanged() {
    let mut day = DataType::date32().nullable_field("event_day");
    day.as_partition_mut().set_sources(["event"]).unwrap();
    let root = StructType::from_fields([DataType::date32().required_field("event"), day])
        .map(DataType::from)
        .unwrap()
        .required_field("row");

    let filled = root.as_transform().apply_arrow_batch(&events()).unwrap();

    assert_eq!(filled.column(1).as_ref(), filled.column(0).as_ref());
}

#[test]
fn a_source_path_reaches_a_struct_child() {
    let mut year = DataType::Int32.nullable_field("year");
    year.as_partition_mut()
        .set_sources(["trade.event"])
        .unwrap();
    year.as_partition_mut()
        .set_transform(yggdryl::expression::Function::Year)
        .unwrap();
    let trade = StructType::from_fields([DataType::date32().required_field("event")])
        .map(DataType::from)
        .unwrap()
        .required_field("trade");
    let root = StructType::from_fields([trade, year])
        .map(DataType::from)
        .unwrap()
        .required_field("row");

    let batch = RecordBatch::try_from_iter([(
        "trade",
        Arc::new(arrow_array::StructArray::from(vec![(
            Arc::new(arrow_schema::Field::new(
                "event",
                arrow_schema::DataType::Date32,
                false,
            )),
            Arc::new(arrow_array::Date32Array::from(vec![19_723])) as ArrayRef,
        )])) as ArrayRef,
    )])
    .unwrap();

    let filled = root.as_transform().apply_arrow_batch(&batch).unwrap();

    let year = filled
        .column_by_name("year")
        .unwrap()
        .as_any()
        .downcast_ref::<Int32Array>()
        .unwrap();
    assert_eq!(year.values(), &[2024]);
}

#[test]
fn the_batch_is_returned_unchanged_when_nothing_declares_a_derivation() {
    let filled = schema()
        .as_transform()
        .apply_arrow_batch(&prices())
        .unwrap();

    assert_eq!(filled.num_columns(), 1);
    assert_eq!(filled.schema(), prices().schema());
}

#[test]
fn a_widened_batch_keeps_the_schema_metadata_it_arrived_with() {
    let batch = events();
    let schema = Arc::new(
        batch.schema().as_ref().clone().with_metadata(
            [("origin".to_owned(), "tape".to_owned())]
                .into_iter()
                .collect(),
        ),
    );
    let batch = RecordBatch::try_new(schema, batch.columns().to_vec()).unwrap();

    let filled = derived_schema()
        .as_transform()
        .apply_arrow_batch(&batch)
        .unwrap();

    assert_eq!(
        filled.schema().metadata().get("origin"),
        Some(&"tape".to_owned())
    );
}

#[test]
fn a_transform_without_sources_beside_it_is_refused() {
    let mut year = DataType::Int32.nullable_field("year");
    year.as_partition_mut()
        .set_transform(yggdryl::expression::Function::Year)
        .unwrap();

    let error = year.as_partition().term().unwrap_err().to_string();
    assert!(error.contains("PARTITION:sources"), "{error}");
}

#[test]
fn a_transform_that_is_not_a_function_of_one_argument_is_refused() {
    let mut year = DataType::Int32.nullable_field("year");
    assert!(
        year.as_partition_mut()
            .set_transform(yggdryl::expression::Function::Truncate)
            .is_err()
    );
    // The refused write leaves the field untouched, and the generic mutation
    // path runs the same validator.
    assert!(year.as_partition().is_empty());
    let error = year
        .as_partition_mut()
        .insert("transform", "epoch")
        .unwrap_err()
        .to_string();
    assert!(error.contains("PARTITION:transform"), "{error}");
    assert!(year.as_partition().is_empty());
}

#[test]
fn a_transform_reading_more_than_one_source_is_refused_for_now() {
    let mut year = DataType::Int32.nullable_field("year");
    year.as_partition_mut()
        .set_sources(["event", "venue"])
        .unwrap();
    year.as_partition_mut()
        .set_transform(yggdryl::expression::Function::Year)
        .unwrap();

    // The list shape is stored, so the intent survives; only evaluating it is
    // refused, and by a message that says which shape reads today.
    assert_eq!(
        year.as_partition().sources().unwrap(),
        Some(vec!["event".to_owned(), "venue".to_owned()])
    );
    let error = year.as_partition().term().unwrap_err().to_string();
    assert!(error.contains("exactly one source"), "{error}");

    // The one shape every `sources` property refuses, whatever declares it.
    assert!(
        year.as_partition_mut()
            .set_sources(["event", "event"])
            .is_err()
    );
    assert!(year.as_partition_mut().set_sources(["*", "event"]).is_err());
}

#[test]
fn a_source_column_the_batch_does_not_carry_is_refused() {
    let error = derived_schema()
        .as_transform()
        .apply_arrow_batch(&prices())
        .unwrap_err()
        .to_string();
    assert!(error.contains("event"), "{error}");
}

#[test]
fn a_nested_declaration_is_filled_before_the_level_above_reads_it() {
    let mut inner_year = DataType::Int32.nullable_field("year");
    inner_year
        .as_partition_mut()
        .set_sources(["event"])
        .unwrap();
    inner_year
        .as_partition_mut()
        .set_transform(yggdryl::expression::Function::Year)
        .unwrap();
    // A source path is relative to the Struct that declares it, so the nested
    // column names `event`, and the level above names `trade.year`.
    let trade = StructType::from_fields([DataType::date32().required_field("event"), inner_year])
        .map(DataType::from)
        .unwrap()
        .required_field("trade");
    let mut top = DataType::Int32.nullable_field("top_year");
    top.as_partition_mut().set_sources(["trade.year"]).unwrap();
    let root = StructType::from_fields([trade, top])
        .map(DataType::from)
        .unwrap()
        .required_field("row");

    let batch = RecordBatch::try_from_iter([(
        "trade",
        Arc::new(arrow_array::StructArray::from(vec![(
            Arc::new(arrow_schema::Field::new(
                "event",
                arrow_schema::DataType::Date32,
                false,
            )),
            Arc::new(arrow_array::Date32Array::from(vec![19_723, 20_089])) as ArrayRef,
        )])) as ArrayRef,
    )])
    .unwrap();

    let filled = root.as_transform().apply_arrow_batch(&batch).unwrap();

    let trade = filled
        .column_by_name("trade")
        .unwrap()
        .as_any()
        .downcast_ref::<arrow_array::StructArray>()
        .expect("the struct the declaration widened");
    assert_eq!(trade.num_columns(), 2);
    assert_eq!(
        trade
            .column(1)
            .as_any()
            .downcast_ref::<Int32Array>()
            .unwrap()
            .values(),
        &[2024, 2025]
    );
    // The level above read what the nested declaration had just written.
    assert_eq!(
        filled
            .column_by_name("top_year")
            .unwrap()
            .as_any()
            .downcast_ref::<Int32Array>()
            .unwrap()
            .values(),
        &[2024, 2025]
    );
}

#[test]
fn a_nested_struct_keeps_its_own_null_mask_through_a_fill() {
    let mut inner_year = DataType::Int32.nullable_field("year");
    inner_year
        .as_partition_mut()
        .set_sources(["event"])
        .unwrap();
    inner_year
        .as_partition_mut()
        .set_transform(yggdryl::expression::Function::Year)
        .unwrap();
    let trade = StructType::from_fields([DataType::date32().required_field("event"), inner_year])
        .map(DataType::from)
        .unwrap()
        .nullable_field("trade");
    let root = StructType::from_fields([trade])
        .map(DataType::from)
        .unwrap()
        .required_field("row");

    let held = arrow_array::StructArray::try_new(
        vec![Arc::new(arrow_schema::Field::new(
            "event",
            arrow_schema::DataType::Date32,
            false,
        ))]
        .into(),
        vec![Arc::new(arrow_array::Date32Array::from(vec![19_723, 0])) as ArrayRef],
        Some(arrow_buffer::NullBuffer::from(vec![true, false])),
    )
    .unwrap();
    let batch = RecordBatch::try_from_iter([("trade", Arc::new(held) as ArrayRef)]).unwrap();

    let filled = root.as_transform().apply_arrow_batch(&batch).unwrap();

    let trade = filled
        .column_by_name("trade")
        .unwrap()
        .as_any()
        .downcast_ref::<arrow_array::StructArray>()
        .unwrap();
    assert_eq!(trade.null_count(), 1);
    assert!(trade.is_null(1));
}

#[test]
fn a_required_column_still_holding_its_canonical_default_is_filled() {
    let mut year = DataType::Int32.required_field("year");
    year.as_partition_mut().set_sources(["event"]).unwrap();
    year.as_partition_mut()
        .set_transform(yggdryl::expression::Function::Year)
        .unwrap();
    let root = StructType::from_fields([DataType::date32().required_field("event"), year])
        .map(DataType::from)
        .unwrap()
        .required_field("row");

    // A required column cannot be null, so one still holding its canonical
    // default is what "never written" looks like.
    let placeholder = RecordBatch::try_from_iter([
        (
            "event",
            Arc::new(arrow_array::Date32Array::from(vec![19_723, 20_089])) as ArrayRef,
        ),
        ("year", Arc::new(Int32Array::from(vec![0, 0])) as ArrayRef),
    ])
    .unwrap();

    let filled = root.as_transform().apply_arrow_batch(&placeholder).unwrap();

    assert_eq!(
        filled
            .column(1)
            .as_any()
            .downcast_ref::<Int32Array>()
            .unwrap()
            .values(),
        &[2024, 2025]
    );
}

/// A Hive layout addressed as a folder, so the three write intents have to
/// resolve its children themselves.

#[test]
fn every_temporal_family_survives_the_directory_name_it_spells() {
    use yggdryl::{Scalar, TimeUnit, Timezone};

    // A partition name is written by one renderer and read by the field cast,
    // so every temporal family has to make the round trip - a zoned instant
    // included, which Arrow's own formatter refuses to spell at all.
    let paris = Timezone::from_str("Europe/Paris").unwrap();
    for (dtype, value) in [
        (DataType::date32(), Scalar::date32(20_682)),
        (DataType::date64(), Scalar::date64(1_786_924_800_000)),
        (
            DataType::time32(TimeUnit::Second).unwrap(),
            Scalar::time32(37_425, TimeUnit::Second, Timezone::NAIVE).unwrap(),
        ),
        (
            DataType::time64(TimeUnit::Nanosecond).unwrap(),
            Scalar::time64(1, TimeUnit::Nanosecond, Timezone::NAIVE).unwrap(),
        ),
        (
            DataType::DateTime64 {
                unit: TimeUnit::Second,
                timezone: Timezone::NAIVE,
            },
            Scalar::datetime64(1_700_000_000, TimeUnit::Second, Timezone::NAIVE).unwrap(),
        ),
        (
            DataType::DateTime64 {
                unit: TimeUnit::Second,
                timezone: Timezone::UTC,
            },
            Scalar::datetime64(1_700_000_000, TimeUnit::Second, Timezone::UTC).unwrap(),
        ),
        (
            DataType::DateTime64 {
                unit: TimeUnit::Second,
                timezone: paris,
            },
            Scalar::datetime64(1_700_000_000, TimeUnit::Second, paris).unwrap(),
        ),
        (
            DataType::duration64(TimeUnit::Second).unwrap(),
            Scalar::duration64(90, TimeUnit::Second).unwrap(),
        ),
    ] {
        let spelled = yggdryl::media::partition::partition_text(&value)
            .unwrap_or_else(|error| panic!("{dtype} has no partition name: {error}"));
        let schema = StructType::from_fields([
            DataType::Int64.required_field("price"),
            Field::new("at", dtype.clone(), false),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
        let restored = with_partitions(
            &prices(),
            &[("at".to_owned(), spelled.to_string())],
            Some(&schema),
        )
        .unwrap_or_else(|error| panic!("{dtype} did not read {spelled:?}: {error}"));
        // One row out of the partition column, through the public boundary:
        // a one-row slice, already in the field's own layout - so nothing is
        // cast - read as the column the partition's field types.
        let at = dtype.clone().nullable_field("at");
        let column = restored.column_by_name("at").unwrap().slice(0, 1);
        assert_eq!(
            column.data_type(),
            at.as_arrow_field_ref().unwrap().data_type(),
            "{dtype}"
        );
        let read = Serie::from_arrow_array(Some(&at), column, ArrowCastOptions::default())
            .unwrap()
            .scalar(0)
            .unwrap();
        assert_eq!(
            read, value,
            "{dtype} did not round trip through {spelled:?}"
        );
    }
}

/// The lazy folder reader, which no caller can name.
#[cfg(feature = "internals")]
mod lazy_folder {
    use std::any::Any;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, OnceLock};

    use arrow_array::RecordBatch;
    use arrow_ipc::writer::StreamWriter;

    use yggdryl::fs::{
        ByteReader, ByteWriter, FileInfo, FileInfos, FileSelector, FileSystem, FsFile,
        MemoryFileSystem, OutputMetadata, RandomAccessReader,
    };
    use yggdryl::holder::Holder;
    use yggdryl::internals::media_partition::folder_reader;
    use yggdryl::media::{IORecordOptions, RecordOptions};
    use yggdryl::{DataType, Error, IOKind, MediaType, MimeType, Result, StructType, Url};
    use yggdryl::{IOBase, Listing};

    use super::prices;

    const LISTING_FAILURE: &str = "lazy folder listing failed";
    const READ_FAILURE: &str = "lazy leaf read failed";

    /// An in-memory foreign filesystem that records exactly which leaf bytes
    /// were requested and can refuse one leaf's reads.
    #[derive(Debug)]
    struct ProbeFilesystem {
        inner: MemoryFileSystem,
        reads: Mutex<Vec<String>>,
        failing_path: Option<String>,
    }

    impl ProbeFilesystem {
        fn new(failing_path: Option<String>) -> Self {
            Self {
                inner: MemoryFileSystem::new(),
                reads: Mutex::new(Vec::new()),
                failing_path,
            }
        }

        fn read_count(&self, path: &str) -> usize {
            self.reads
                .lock()
                .expect("the read probe lock")
                .iter()
                .filter(|read| read.as_str() == path)
                .count()
        }

        fn total_reads(&self) -> usize {
            self.reads.lock().expect("the read probe lock").len()
        }

        fn record_read(&self, path: &str) -> Result<()> {
            self.reads
                .lock()
                .expect("the read probe lock")
                .push(path.to_owned());
            if self.failing_path.as_deref() == Some(path) {
                return Err(Error::Io(std::io::Error::other(READ_FAILURE)));
            }
            Ok(())
        }
    }

    impl FileSystem for ProbeFilesystem {
        fn type_name(&self) -> &str {
            "probe"
        }

        fn equals(&self, other: &dyn FileSystem) -> bool {
            other
                .as_any()
                .downcast_ref::<Self>()
                .is_some_and(|other| std::ptr::eq(self, other))
        }

        fn normalize_path(&self, path: &str) -> Result<String> {
            self.inner.normalize_path(path)
        }

        fn file_info(&self, path: &str) -> Result<FileInfo> {
            self.inner.file_info(path)
        }

        fn list(&self, selector: &FileSelector) -> FileInfos {
            self.inner.list(selector)
        }

        fn create_dir(&self, path: &str, recursive: bool) -> Result<()> {
            self.inner.create_dir(path, recursive)
        }

        fn delete_dir(&self, path: &str) -> Result<()> {
            self.inner.delete_dir(path)
        }

        fn delete_dir_contents(&self, path: &str, missing_dir_ok: bool) -> Result<()> {
            self.inner.delete_dir_contents(path, missing_dir_ok)
        }

        fn delete_root_dir_contents(&self) -> Result<()> {
            self.inner.delete_root_dir_contents()
        }

        fn delete_file(&self, path: &str) -> Result<()> {
            self.inner.delete_file(path)
        }

        fn copy_file(&self, source: &str, target: &str) -> Result<()> {
            self.inner.copy_file(source, target)
        }

        fn move_file(&self, source: &str, target: &str) -> Result<()> {
            self.inner.move_file(source, target)
        }

        fn open_input_file(&self, path: &str) -> Result<Box<dyn RandomAccessReader>> {
            self.record_read(path)?;
            self.inner.open_input_file(path)
        }

        fn open_input_stream(&self, path: &str) -> Result<Box<dyn ByteReader>> {
            self.record_read(path)?;
            self.inner.open_input_stream(path)
        }

        fn open_output_stream(
            &self,
            path: &str,
            metadata: Option<&OutputMetadata>,
        ) -> Result<Box<dyn ByteWriter>> {
            self.inner.open_output_stream(path, metadata)
        }

        fn open_append_stream(
            &self,
            path: &str,
            metadata: Option<&OutputMetadata>,
        ) -> Result<Box<dyn ByteWriter>> {
            self.inner.open_append_stream(path, metadata)
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    /// A synthetic folder whose listing builds one Arrow-filesystem leaf per
    /// pull. The counter distinguishes a live listing from an eager vector.
    struct ProbeFolder {
        url: Url,
        filesystem: Arc<ProbeFilesystem>,
        width: usize,
        pulls: Arc<AtomicUsize>,
        failing_entry: Option<usize>,
    }

    impl ProbeFolder {
        fn new(width: usize, failing_entry: Option<usize>, failing_read: Option<usize>) -> Self {
            let failing_path = failing_read.map(part_path);
            let filesystem = Arc::new(ProbeFilesystem::new(failing_path));
            filesystem
                .create_dir("bucket/lake", true)
                .expect("create the fixture root");
            let encoded = encoded_batch(&prices());
            for index in 0..width {
                let mut writer = filesystem
                    .open_output_stream(&part_path(index), None)
                    .expect("open one IPC leaf");
                let mut position = 0;
                while position < encoded.len() {
                    let written = writer
                        .write(&encoded[position..])
                        .expect("seed one IPC leaf");
                    assert_ne!(written, 0, "the seed stream must make progress");
                    position += written;
                }
                writer.close().expect("publish one IPC leaf");
            }
            Self {
                url: Url::from_str("probe://bucket/lake/").expect("a valid folder URL"),
                filesystem,
                width,
                pulls: Arc::new(AtomicUsize::new(0)),
                failing_entry,
            }
        }

        fn pulls(&self) -> usize {
            self.pulls.load(Ordering::Relaxed)
        }
    }

    impl yggdryl::IOMedia for ProbeFolder {
        yggdryl::impl_default_iomedia!();
    }

    impl IOBase for ProbeFolder {
        fn pread(&self, _offset: u64, _buffer: &mut [u8]) -> Result<usize> {
            Ok(0)
        }

        fn pwrite(&mut self, _offset: u64, bytes: &[u8]) -> Result<usize> {
            Ok(bytes.len())
        }

        fn size(&self) -> u64 {
            0
        }

        fn capacity(&self) -> u64 {
            0
        }

        fn reserve(&mut self, _capacity: u64) -> Result<()> {
            Ok(())
        }

        fn truncate(&mut self, _size: u64) -> Result<()> {
            Ok(())
        }

        fn uri(&self) -> Option<&yggdryl::Uri> {
            Some(self.url.as_ref())
        }

        fn url(&self) -> Option<&Url> {
            Some(&self.url)
        }

        fn media_type(&self) -> &MediaType {
            static DIRECTORY: OnceLock<MediaType> = OnceLock::new();
            DIRECTORY.get_or_init(|| MediaType::from(MimeType::DIRECTORY))
        }

        fn set_media_type(&mut self, _media_type: MediaType) {}

        fn kind(&self) -> IOKind {
            IOKind::Directory
        }

        fn children_where(
            &self,
            filters: &[(&str, &str)],
            include_private: bool,
        ) -> Result<Listing> {
            assert!(filters.is_empty());
            assert!(!include_private);
            let filesystem: Arc<dyn FileSystem> = self.filesystem.clone();
            let pulls = Arc::clone(&self.pulls);
            let width = self.width;
            let failing_entry = self.failing_entry;
            Ok(Listing::new((0..width).map(move |index| {
                pulls.fetch_add(1, Ordering::Relaxed);
                if failing_entry == Some(index) {
                    return Err(Error::Io(std::io::Error::other(LISTING_FAILURE)));
                }
                FsFile::from_path(Arc::clone(&filesystem), part_path(index), None)
                    .map(Holder::FsFile)
            })))
        }
    }

    fn part_path(index: usize) -> String {
        format!("bucket/lake/part-{index}.arrows")
    }

    fn encoded_batch(batch: &RecordBatch) -> Vec<u8> {
        let mut encoded = Vec::new();
        {
            let mut writer = StreamWriter::try_new(&mut encoded, batch.schema().as_ref())
                .expect("an IPC writer");
            writer.write(batch).expect("one IPC batch");
            writer.finish().expect("the IPC end marker");
        }
        encoded
    }

    fn options() -> RecordOptions {
        let field = StructType::from_fields([DataType::Int64.required_field("price")])
            .map(DataType::from)
            .expect("one field")
            .required_field("row");
        RecordOptions::for_mime_type(&MimeType::ARROW_STREAM)
            .expect("IPC options")
            .with_field(field)
    }

    fn error_then_fused(mut reader: yggdryl::arrow::BatchReader) -> String {
        let error = reader
            .next()
            .expect("one failure")
            .expect_err("the read must fail")
            .to_string();
        assert!(
            reader.next().is_none(),
            "the reader must fuse after failure"
        );
        assert!(reader.next().is_none(), "the reader must stay fused");
        error
    }

    #[test]
    fn a_declared_schema_constructs_without_pulling_or_reading_a_leaf() {
        let folder = ProbeFolder::new(3, None, None);

        let reader = folder_reader(&folder, &options()).expect("a folder reader");

        assert_eq!(reader.schema().fields().len(), 1);
        assert_eq!(folder.pulls(), 0, "the leaf listing stays untouched");
        assert_eq!(
            folder.filesystem.total_reads(),
            0,
            "no leaf bytes are inspected to construct a declared schema"
        );
    }

    #[test]
    fn the_first_batch_opens_only_the_first_listed_leaf() {
        let folder = ProbeFolder::new(3, None, None);
        let mut reader = folder_reader(&folder, &options()).expect("a folder reader");

        let first = reader.next().expect("one batch").expect("a valid batch");

        assert_eq!(first.num_rows(), 3);
        assert_eq!(folder.pulls(), 1, "only one listing entry was demanded");
        assert!(folder.filesystem.read_count(&part_path(0)) > 0);
        assert_eq!(
            folder.filesystem.read_count(&part_path(1)),
            0,
            "the next leaf remains unopened"
        );
    }

    #[test]
    fn a_listing_failure_is_yielded_once_then_the_reader_fuses() {
        let folder = ProbeFolder::new(3, Some(0), None);
        let reader = folder_reader(&folder, &options()).expect("a lazy folder reader");

        let error = error_then_fused(reader);

        assert!(error.contains(LISTING_FAILURE), "{error}");
        assert_eq!(folder.pulls(), 1, "nothing was pulled past the failure");
        assert_eq!(folder.filesystem.total_reads(), 0);
    }

    #[test]
    fn a_leaf_read_failure_is_yielded_once_then_the_reader_fuses() {
        let folder = ProbeFolder::new(3, None, Some(0));
        let reader = folder_reader(&folder, &options()).expect("a lazy folder reader");

        let error = error_then_fused(reader);

        assert!(error.contains(READ_FAILURE), "{error}");
        assert_eq!(folder.pulls(), 1, "the next leaf was never requested");
        assert_eq!(folder.filesystem.read_count(&part_path(0)), 1);
        assert_eq!(folder.filesystem.read_count(&part_path(1)), 0);
    }
}
