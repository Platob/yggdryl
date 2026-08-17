//! Partition columns move between the path and the data without loss.

use std::sync::Arc;

use arrow_array::{ArrayRef, Int32Array, Int64Array, RecordBatch, StringArray};

use super::{partitioned_reader, with_partitions, without_partitions};
use crate::generic::RecordOptions;
use crate::io::IOBase;
use crate::{DataType, Field};

fn schema() -> Field {
    DataType::from_fields([
        DataType::Int64.required_field("price"),
        DataType::Int32.required_field("year"),
        DataType::Utf8.required_field("month"),
    ])
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
    let inner: crate::arrow::BatchReader = Box::new(arrow_array::RecordBatchIterator::new(
        [Ok(prices())],
        prices().schema(),
    ));

    let reader = partitioned_reader(inner, partitions(), Some(schema())).unwrap();

    assert_eq!(reader.schema().fields().len(), 3);
    let batches: Vec<_> = reader.collect::<std::result::Result<Vec<_>, _>>().unwrap();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].num_columns(), 3);
}

/// A Hive layout addressed as a folder, so the three record methods have to
/// resolve its children themselves.
mod lake {
    use super::{Field, IOBase, RecordOptions, partitions, prices, schema, with_partitions};

    use std::path::{Path, PathBuf};

    use arrow_array::{Array, Int32Array, RecordBatch, StringArray};

    use crate::DataType;
    use crate::generic::{Holder, IORecordOptions};

    /// Build an empty `lake/` under the temp directory and hold it as a folder.
    fn lake(label: &str) -> (PathBuf, Holder) {
        let mut root = std::env::temp_dir();
        root.push(format!("yggdryl-lake-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let holder = Holder::folder(&root).unwrap();
        (root, holder)
    }

    /// The options a partitioned Arrow IPC lake is read and written under.
    fn options(field: Option<Field>) -> RecordOptions {
        let options = RecordOptions::for_mime_type(&crate::MimeType::ARROW_STREAM).unwrap();
        match field {
            Some(field) => options.with_schema(field),
            None => options,
        }
    }

    /// Seed one partition directly, which is how a layout comes into being.
    fn seed(root: &Path, directory: &str, batch: &RecordBatch) {
        let mut leaf = Holder::folder(root.join(directory))
            .unwrap()
            .child_by("part-0.arrows")
            .unwrap();
        leaf.write_arrow_batch_reader(
            crate::arrow::batch_reader(batch.schema(), [batch.clone()]),
            &options(None),
        )
        .unwrap();
        leaf.flush().unwrap();
    }

    /// Every row a lake holds, as `(year, month, price)`.
    fn rows(handle: &Holder, field: &Field) -> Vec<(i32, String, i64)> {
        let mut found = Vec::new();
        for batch in handle
            .read_arrow_batch_reader(&options(Some(field.clone())))
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
        let read = handle
            .read_arrow_batch_reader(&options(Some(field)))
            .unwrap();
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
            .write_arrow_batch_reader(
                crate::arrow::batch_reader(incoming.schema(), [incoming]),
                &options(Some(field.clone())),
            )
            .unwrap();

        // The new partition was created and the old one was replaced, because
        // an empty match key overwrites the tree it addresses.
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
    fn an_overwrite_empties_a_partition_the_incoming_rows_no_longer_name() {
        let (root, mut handle) = lake("empty-partition");
        seed(&root, "year=2024/month=01", &prices());
        seed(&root, "year=2024/month=02", &prices());

        let field = schema();
        let only_january = with_partitions(&prices(), &partitions(), Some(&field)).unwrap();
        handle
            .write_arrow_batch_reader(
                crate::arrow::batch_reader(only_january.schema(), [only_january]),
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
            .append_arrow_batch_reader(
                crate::arrow::batch_reader(january.schema(), [january]),
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
            .write_arrow_batch_reader(
                crate::arrow::batch_reader(updates.schema(), [updates]),
                &options(Some(field.clone())).with_merge_by(["price"]),
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
            .write_arrow_batch_reader(
                crate::arrow::batch_reader(full.schema(), [full]),
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
            .write_arrow_batch_reader(
                crate::arrow::batch_reader(full.schema(), [full]),
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
            .try_with_data_type(
                DataType::from_fields([
                    DataType::Int64.required_field("price"),
                    DataType::Int32.required_field("year"),
                    DataType::Utf8.required_field("month"),
                    DataType::Utf8.required_field("venue"),
                ])
                .unwrap(),
            )
            .unwrap()
            .with_partition_fields(&["venue"])
            .unwrap();
        let full = with_partitions(&prices(), &partitions(), Some(&field)).unwrap();
        let error = handle
            .write_arrow_batch_reader(
                crate::arrow::batch_reader(full.schema(), [full]),
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
            .write_arrow_batch_reader(
                crate::arrow::batch_reader(full.schema(), [full]),
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

        let field = crate::DataType::from_fields([
            crate::DataType::Int64.required_field("price"),
            crate::DataType::Int32.nullable_field("year"),
            crate::DataType::Utf8.nullable_field("month"),
        ])
        .unwrap()
        .required_field("row");
        let batch = RecordBatch::try_new(
            crate::arrow::schema_from_field(&field).unwrap(),
            vec![
                std::sync::Arc::new(arrow_array::Int64Array::from(vec![99])),
                std::sync::Arc::new(Int32Array::from(vec![None::<i32>])),
                std::sync::Arc::new(StringArray::from(vec![None::<&str>])),
            ],
        )
        .unwrap();

        handle
            .write_arrow_batch_reader(
                crate::arrow::batch_reader(batch.schema(), [batch]),
                &options(Some(field.clone())),
            )
            .unwrap();

        assert!(root.join("year=null").join("month=null").is_dir());
        // A path cannot say whether that is the absence or the three letters,
        // so a nullable declared column reads the text back as a null.
        let read: Vec<RecordBatch> = handle
            .read_arrow_batch_reader(&options(Some(field)))
            .unwrap()
            .map(std::result::Result::unwrap)
            .collect();
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].column_by_name("year").unwrap().null_count(), 1);

        let _ = std::fs::remove_dir_all(&root);
    }
}
