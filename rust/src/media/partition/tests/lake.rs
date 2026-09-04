use super::{Field, IOBase, RecordOptions, partitions, prices, schema, with_partitions};

use std::path::{Path, PathBuf};

use arrow_array::{Array, Int32Array, RecordBatch, RecordBatchIterator, StringArray};
use arrow_schema::ArrowError;

use crate::DataType;
use crate::IOMedia;
use crate::holder::Holder;
use crate::media::IORecordOptions;

/// Build an empty `lake/` under the temp directory and hold it as a folder.
fn lake(label: &str) -> (PathBuf, Holder) {
    let mut root = crate::holder::local::Folder::temporary()
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
    let options = RecordOptions::for_mime_type(&crate::MimeType::ARROW_STREAM).unwrap();
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
            crate::arrow::batch_reader(incoming.schema(), [incoming]),
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
    let field = DataType::from_fields([
        DataType::FixedAscii(4).required_field("ccy"),
        DataType::Int64.required_field("qty"),
    ])
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
            crate::arrow::batch_reader(incoming.schema(), [incoming]),
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
fn a_code_partition_column_keeps_its_identity_through_the_path() {
    let (root, mut handle) = lake("code");
    let field = DataType::from_fields([
        DataType::Currency.required_field("ccy"),
        DataType::Int64.required_field("qty"),
    ])
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
            crate::arrow::batch_reader(incoming.schema(), [incoming]),
            &options(Some(field.clone())),
        )
        .unwrap();

    // A code renders as its text in the path exactly as a width does.
    assert!(root.join("ccy=USD").is_dir());
    assert!(root.join("ccy=EUR").is_dir());

    let mut found = Vec::new();
    for batch in handle.read_arrow_reader(&options(Some(field))).unwrap() {
        let batch = batch.unwrap();
        // ISO 4217 is exactly three bytes, so the restored storage holds
        // no padding at all, and the column reads back a currency.
        let restored = crate::Field::from_arrow(batch.schema().field(0)).unwrap();
        assert_eq!(restored.dtype(), &DataType::Currency);
        let ccy = batch
            .column_by_name("ccy")
            .unwrap()
            .as_any()
            .downcast_ref::<arrow_array::FixedSizeBinaryArray>()
            .unwrap()
            .clone();
        assert_eq!(ccy.value_length(), 3);
        for row in 0..batch.num_rows() {
            found.push(ccy.value(row).to_vec());
        }
    }
    found.sort_unstable();
    assert_eq!(
        found,
        vec![b"EUR".to_vec(), b"USD".to_vec(), b"USD".to_vec()]
    );

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
    let arrow = crate::arrow::arrow_schema_from_field(&field).unwrap();

    // No declared field is available on either side of this call: the
    // empty reader's Arrow schema must remain discoverable from a real
    // encoded leaf rather than a zero-byte remnant.
    handle
        .overwrite_arrow_reader(crate::arrow::batch_reader(arrow, []), &options(None))
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
        let mut merging = options(Some(field.clone())).with_merge_by_names(["year", "month"]);
        merging.set_commit_row_size(cadence);

        let message = handle
            .merge_arrow_reader(
                crate::arrow::batch_reader(incoming.schema(), [incoming]),
                &merging,
            )
            .unwrap_err()
            .to_string();

        assert!(message.contains("merge_by_names"), "{label}: {message}");
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
    use crate::media::IORecordOptions;

    let (root, handle) = lake("filtered");
    seed(&root, "year=2024/month=01", &prices());
    seed(&root, "year=2024/month=02", &prices());
    seed(&root, "year=2025/month=01", &prices());

    let field = schema();
    // A path filter prunes: only the leaves whose directories name the
    // value are read at all, and the restored column carries it.
    let filtered =
        options(Some(field.clone())).with_filter_partitions([("month", "01"), ("year", "2024")]);
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
    let by_price = options(Some(field.clone())).with_filter_partitions([("price", "20")]);
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
        .append_arrow_reader(
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
        .merge_arrow_reader(
            crate::arrow::batch_reader(updates.schema(), [updates]),
            &options(Some(field.clone())).with_merge_by_names(["price"]),
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
        .overwrite_arrow_reader(
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
        .try_with_dtype(
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
        .overwrite_arrow_reader(
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
        .overwrite_arrow_reader(
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
        crate::arrow::arrow_schema_from_field(&field).unwrap(),
        vec![
            std::sync::Arc::new(arrow_array::Int64Array::from(vec![99])),
            std::sync::Arc::new(Int32Array::from(vec![None::<i32>])),
            std::sync::Arc::new(StringArray::from(vec![None::<&str>])),
        ],
    )
    .unwrap();

    handle
        .overwrite_arrow_reader(
            crate::arrow::batch_reader(batch.schema(), [batch]),
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
