//! A write whose options name a match key updates and appends by key.

use std::sync::Arc;

use arrow_array::{Array, Int64Array, RecordBatch, StringArray};

use crate::arrow::BatchReader;
use crate::generic::{IORecordOptions, RecordOptions};
use crate::io::{Buffer, IOBase};
use crate::{DataType, Field, Url};

/// Two columns: one key and one payload, so an update is visible.
fn schema() -> Field {
    DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
    ])
    .unwrap()
    .required_field("row")
}

fn rows(ids: Vec<i64>, symbols: Vec<Option<&str>>) -> RecordBatch {
    RecordBatch::try_new(
        crate::arrow::schema_from_field(&schema()).unwrap(),
        vec![
            Arc::new(Int64Array::from(ids)),
            Arc::new(StringArray::from(symbols)),
        ],
    )
    .unwrap()
}

fn reader(batches: Vec<RecordBatch>) -> BatchReader {
    crate::arrow::batch_reader(crate::arrow::schema_from_field(&schema()).unwrap(), batches)
}

fn handle(name: &str) -> Buffer {
    Buffer::new().with_media_type(
        Url::from_str(&format!("file:///{name}"))
            .unwrap()
            .media_type(),
    )
}

/// Merging options over a handle, keyed on `id`.
fn merging(handle: &Buffer) -> RecordOptions {
    handle
        .record_options()
        .unwrap()
        .with_schema(schema())
        .with_merge_by(["id"])
}

/// Every `(id, symbol)` pair a handle holds, in stored order.
fn stored(handle: &impl IOBase, options: &RecordOptions) -> Vec<(i64, Option<String>)> {
    let mut plain = options.clone();
    plain.set_merge_by(Vec::new());
    let mut found = Vec::new();
    for batch in handle.read_arrow_batch_reader(&plain).unwrap() {
        let batch = batch.unwrap();
        let ids = batch
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .expect("the declared Int64 key");
        let symbols = batch
            .column(1)
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("the declared Utf8 payload");
        for row in 0..batch.num_rows() {
            found.push((
                ids.value(row),
                symbols.is_valid(row).then(|| symbols.value(row).to_owned()),
            ));
        }
    }
    found
}

#[test]
fn a_matching_key_updates_the_row_it_names() {
    let mut handle = handle("update.arrows");
    let options = merging(&handle);
    handle
        .write_arrow_batch_reader(
            reader(vec![rows(vec![1, 2], vec![Some("AAPL"), Some("MSFT")])]),
            &options,
        )
        .unwrap();

    handle
        .write_arrow_batch_reader(reader(vec![rows(vec![2], vec![Some("MSFT.O")])]), &options)
        .unwrap();

    // The row keeps its position; only its payload changed.
    assert_eq!(
        stored(&handle, &options),
        vec![(1, Some("AAPL".to_owned())), (2, Some("MSFT.O".to_owned())),]
    );
}

#[test]
fn a_key_that_matches_nothing_is_appended() {
    let mut handle = handle("append-key.arrows");
    let options = merging(&handle);
    handle
        .write_arrow_batch_reader(reader(vec![rows(vec![1], vec![Some("AAPL")])]), &options)
        .unwrap();

    handle
        .write_arrow_batch_reader(
            reader(vec![rows(vec![1, 3], vec![Some("AAPL.O"), Some("NVDA")])]),
            &options,
        )
        .unwrap();

    assert_eq!(
        stored(&handle, &options),
        vec![(1, Some("AAPL.O".to_owned())), (3, Some("NVDA".to_owned()))]
    );
}

#[test]
fn a_key_stored_twice_has_every_occurrence_updated() {
    let mut handle = handle("duplicate-stored.arrows");
    let options = merging(&handle);
    // Two batches, both carrying key 1, because a match key is a rule and not
    // a constraint the stored side was ever checked against.
    handle
        .write_arrow_batch_reader(
            reader(vec![
                rows(vec![1], vec![Some("AAPL")]),
                rows(vec![1, 2], vec![Some("AAPL"), Some("MSFT")]),
            ]),
            &options.clone().with_merge_by(Vec::<String>::new()),
        )
        .unwrap();

    handle
        .write_arrow_batch_reader(reader(vec![rows(vec![1], vec![Some("AAPL.O")])]), &options)
        .unwrap();

    assert_eq!(
        stored(&handle, &options),
        vec![
            (1, Some("AAPL.O".to_owned())),
            (1, Some("AAPL.O".to_owned())),
            (2, Some("MSFT".to_owned())),
        ]
    );
}

#[test]
fn a_key_arriving_twice_lets_the_last_arrival_win() {
    let mut handle = handle("duplicate-incoming.arrows");
    let options = merging(&handle);
    handle
        .write_arrow_batch_reader(reader(vec![rows(vec![1], vec![Some("AAPL")])]), &options)
        .unwrap();

    // Key 1 already exists and is updated twice; key 9 is new and must not be
    // appended twice, so its second arrival replaces the row the first claimed.
    handle
        .write_arrow_batch_reader(
            reader(vec![rows(
                vec![1, 9, 1, 9],
                vec![Some("a"), Some("b"), Some("c"), Some("d")],
            )]),
            &options,
        )
        .unwrap();

    assert_eq!(
        stored(&handle, &options),
        vec![(1, Some("c".to_owned())), (9, Some("d".to_owned()))]
    );
}

#[test]
fn an_empty_target_appends_every_row() {
    let mut handle = handle("empty-target.arrows");
    let options = merging(&handle);

    // Nothing is stored yet, so the merge has nothing to match and the whole
    // incoming side lands as it stands.
    handle
        .write_arrow_batch_reader(
            reader(vec![rows(vec![5, 6], vec![None, Some("MSFT")])]),
            &options,
        )
        .unwrap();

    assert_eq!(
        stored(&handle, &options),
        vec![(5, None), (6, Some("MSFT".to_owned()))]
    );
}

#[test]
fn a_null_key_matches_another_null_key() {
    let field = DataType::from_fields([
        DataType::Int64.nullable_field("id"),
        DataType::Utf8.nullable_field("symbol"),
    ])
    .unwrap()
    .required_field("row");
    let arrow = crate::arrow::schema_from_field(&field).unwrap();
    let batch = |ids: Vec<Option<i64>>, symbols: Vec<Option<&str>>| {
        RecordBatch::try_new(
            Arc::clone(&arrow),
            vec![
                Arc::new(Int64Array::from(ids)),
                Arc::new(StringArray::from(symbols)),
            ],
        )
        .unwrap()
    };

    let mut handle = handle("null-key.arrows");
    let options = handle
        .record_options()
        .unwrap()
        .with_schema(field)
        .with_merge_by(["id"]);
    handle
        .write_arrow_batch_reader(
            crate::arrow::batch_reader(Arc::clone(&arrow), [batch(vec![None], vec![Some("AAPL")])]),
            &options,
        )
        .unwrap();
    handle
        .write_arrow_batch_reader(
            crate::arrow::batch_reader(Arc::clone(&arrow), [batch(vec![None], vec![Some("MSFT")])]),
            &options,
        )
        .unwrap();

    // Arrow's row encoding gives absence one exact spelling, so two null keys
    // are the same key rather than two rows that merely both lack a value.
    let total: usize = handle
        .read_arrow_batch_reader(&options.with_merge_by(Vec::<String>::new()))
        .unwrap()
        .map(|batch| batch.unwrap().num_rows())
        .sum();
    assert_eq!(total, 1);
}

#[test]
fn a_composite_key_matches_on_every_column() {
    let field = DataType::from_fields([
        DataType::Utf8.required_field("venue"),
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
    ])
    .unwrap()
    .required_field("row");
    let arrow = crate::arrow::schema_from_field(&field).unwrap();
    let batch = |venues: Vec<&str>, ids: Vec<i64>, symbols: Vec<&str>| {
        RecordBatch::try_new(
            Arc::clone(&arrow),
            vec![
                Arc::new(StringArray::from(venues)),
                Arc::new(Int64Array::from(ids)),
                Arc::new(StringArray::from(symbols)),
            ],
        )
        .unwrap()
    };

    let mut handle = handle("composite.arrows");
    let options = handle
        .record_options()
        .unwrap()
        .with_schema(field)
        .with_merge_by(["venue", "id"]);
    handle
        .write_arrow_batch_reader(
            crate::arrow::batch_reader(
                Arc::clone(&arrow),
                [batch(vec!["XNAS", "XNYS"], vec![1, 1], vec!["a", "b"])],
            ),
            &options,
        )
        .unwrap();
    // The same id at a different venue is a different row.
    handle
        .write_arrow_batch_reader(
            crate::arrow::batch_reader(
                Arc::clone(&arrow),
                [batch(vec!["XNYS"], vec![1], vec!["b2"])],
            ),
            &options,
        )
        .unwrap();

    let read: Vec<RecordBatch> = handle
        .read_arrow_batch_reader(&options.clone().with_merge_by(Vec::<String>::new()))
        .unwrap()
        .map(std::result::Result::unwrap)
        .collect();
    let total: usize = read.iter().map(RecordBatch::num_rows).sum();
    assert_eq!(total, 2);
    let symbols = read[0]
        .column(2)
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert_eq!(symbols.value(0), "a");
    assert_eq!(symbols.value(1), "b2");
}

#[test]
fn an_incoming_schema_that_disagrees_is_cast_to_the_target_first() {
    let mut handle = handle("disagree.arrows");
    let options = merging(&handle);
    handle
        .write_arrow_batch_reader(
            reader(vec![rows(vec![1, 2], vec![Some("AAPL"), Some("MSFT")])]),
            &options,
        )
        .unwrap();

    // `id` arrives as text, `symbol` comes first, and `venue` is not declared
    // at all: the cast to the target reorders, converts, and drops before a
    // single key is compared.
    let loose = DataType::from_fields([
        DataType::Utf8.nullable_field("symbol"),
        DataType::Utf8.required_field("id"),
        DataType::Utf8.nullable_field("venue"),
    ])
    .unwrap()
    .required_field("row");
    let incoming = RecordBatch::try_new(
        crate::arrow::schema_from_field(&loose).unwrap(),
        vec![
            Arc::new(StringArray::from(vec!["MSFT.O"])),
            Arc::new(StringArray::from(vec!["2"])),
            Arc::new(StringArray::from(vec!["XNAS"])),
        ],
    )
    .unwrap();

    handle
        .write_arrow_batch_reader(
            crate::arrow::batch_reader(incoming.schema(), [incoming]),
            &options,
        )
        .unwrap();

    assert_eq!(
        stored(&handle, &options),
        vec![(1, Some("AAPL".to_owned())), (2, Some("MSFT.O".to_owned())),]
    );
}

#[test]
fn a_match_key_naming_an_unknown_column_is_refused_by_name() {
    let mut handle = handle("unknown-key.arrows");
    let options = handle
        .record_options()
        .unwrap()
        .with_schema(schema())
        .with_merge_by(["nowhere"]);

    let message = handle
        .write_arrow_batch_reader(reader(vec![rows(vec![1], vec![Some("AAPL")])]), &options)
        .unwrap_err()
        .to_string();
    assert!(message.contains("nowhere"), "{message}");
    assert!(message.contains("id, symbol"), "{message}");
}

#[cfg(feature = "parquet")]
#[test]
fn merging_works_the_same_way_on_parquet() {
    let mut handle = handle("merge.parquet");
    let options = merging(&handle);
    handle
        .write_arrow_batch_reader(
            reader(vec![rows(vec![1, 2], vec![Some("AAPL"), Some("MSFT")])]),
            &options,
        )
        .unwrap();
    handle
        .write_arrow_batch_reader(
            reader(vec![rows(vec![2, 3], vec![Some("MSFT.O"), Some("NVDA")])]),
            &options,
        )
        .unwrap();

    assert_eq!(
        stored(&handle, &options),
        vec![
            (1, Some("AAPL".to_owned())),
            (2, Some("MSFT.O".to_owned())),
            (3, Some("NVDA".to_owned())),
        ]
    );
}
