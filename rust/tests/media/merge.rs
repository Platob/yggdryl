//! `rust/src/media/merge.rs`: an explicit merge updates and appends by key.
//!
//! Everything but the last pin reaches the crate through `yggdryl::`. The
//! reader every `IOMode::Merge` write pulls through is crate-private, and that
//! it releases each incoming batch before pulling the next is what keeps a
//! merge bounded, so that one is reached through `yggdryl::internals`.

use std::sync::Arc;
#[cfg(feature = "internals")]
use std::sync::Weak;

#[cfg(feature = "internals")]
use arrow_array::ArrayRef;
use arrow_array::{Array, Int64Array, RecordBatch, StringArray};
#[cfg(feature = "internals")]
use arrow_schema::{ArrowError, SchemaRef};

use yggdryl::arrow::BatchReader;
use yggdryl::holder::Buffer;
use yggdryl::media::{IORecordOptions, RecordOptions};
use yggdryl::{DataType, Field, StructType, Url};
use yggdryl::{IOBase, IOMedia};

/// Two columns: one key and one payload, so an update is visible.
fn schema() -> Field {
    StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("symbol"),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("row")
}

fn rows(ids: Vec<i64>, symbols: Vec<Option<&str>>) -> RecordBatch {
    RecordBatch::try_new(
        schema().into_arrow_schema().unwrap(),
        vec![
            Arc::new(Int64Array::from(ids)),
            Arc::new(StringArray::from(symbols)),
        ],
    )
    .unwrap()
}

fn reader(batches: Vec<RecordBatch>) -> BatchReader {
    yggdryl::arrow::batch_reader(schema().into_arrow_schema().unwrap(), batches)
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
        .with_field(schema())
        .with_merge_by(["id"])
        .unwrap()
}

/// Every `(id, symbol)` pair a handle holds, in stored order.
fn stored(handle: &impl IOBase, options: &RecordOptions) -> Vec<(i64, Option<String>)> {
    let mut plain = options.clone();
    plain.set_merge_by(yggdryl::expression::Selector::all());
    let mut found = Vec::new();
    for batch in handle.read_arrow_reader(&plain).unwrap() {
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
        .merge_arrow_reader(
            reader(vec![rows(vec![1, 2], vec![Some("AAPL"), Some("MSFT")])]),
            &options,
        )
        .unwrap();

    handle
        .merge_arrow_reader(reader(vec![rows(vec![2], vec![Some("MSFT.O")])]), &options)
        .unwrap();

    // The row keeps its position; only its payload changed.
    assert_eq!(
        stored(&handle, &options),
        vec![(1, Some("AAPL".to_owned())), (2, Some("MSFT.O".to_owned())),]
    );
}

#[test]
fn a_key_resolves_the_way_every_other_name_resolves() {
    // The layout validation folds case and the cast that shaped these rows
    // matched their columns folded, so resolving the key exactly would refuse
    // a spelling the layers around it already accepted.
    let mut handle = handle("folded-key.arrows");
    let options = handle
        .record_options()
        .unwrap()
        .with_field(schema())
        .with_merge_by(["ID"])
        .unwrap();
    handle
        .merge_arrow_reader(
            reader(vec![rows(vec![1, 2], vec![Some("AAPL"), Some("MSFT")])]),
            &options,
        )
        .unwrap();
    handle
        .merge_arrow_reader(reader(vec![rows(vec![2], vec![Some("MSFT.O")])]), &options)
        .unwrap();

    assert_eq!(
        stored(&handle, &options),
        vec![(1, Some("AAPL".to_owned())), (2, Some("MSFT.O".to_owned()))]
    );
}

#[test]
fn a_key_naming_no_stored_column_is_still_refused() {
    let mut handle = handle("absent-key.arrows");
    let options = handle
        .record_options()
        .unwrap()
        .with_field(schema())
        .with_merge_by(["venue"])
        .unwrap();
    let error = handle
        .merge_arrow_reader(reader(vec![rows(vec![1], vec![Some("AAPL")])]), &options)
        .unwrap_err()
        .to_string();
    assert!(error.contains("venue"), "{error}");
}

#[test]
fn a_key_that_matches_nothing_is_appended() {
    let mut handle = handle("append-key.arrows");
    let options = merging(&handle);
    handle
        .merge_arrow_reader(reader(vec![rows(vec![1], vec![Some("AAPL")])]), &options)
        .unwrap();

    handle
        .merge_arrow_reader(
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
        .overwrite_arrow_reader(
            reader(vec![
                rows(vec![1], vec![Some("AAPL")]),
                rows(vec![1, 2], vec![Some("AAPL"), Some("MSFT")]),
            ]),
            &options
                .clone()
                .with_merge_by(yggdryl::expression::Selector::all())
                .unwrap(),
        )
        .unwrap();

    handle
        .merge_arrow_reader(reader(vec![rows(vec![1], vec![Some("AAPL.O")])]), &options)
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
        .merge_arrow_reader(reader(vec![rows(vec![1], vec![Some("AAPL")])]), &options)
        .unwrap();

    // Key 1 already exists and is updated twice; key 9 is new and must not be
    // appended twice, so its second arrival replaces the row the first claimed.
    handle
        .merge_arrow_reader(
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
        .merge_arrow_reader(
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
    let field = StructType::from_fields([
        DataType::Int64.nullable_field("id"),
        DataType::utf8().nullable_field("symbol"),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("row");
    let arrow = field.clone().into_arrow_schema().unwrap();
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
        .with_field(field)
        .with_merge_by(["id"])
        .unwrap();
    handle
        .merge_arrow_reader(
            yggdryl::arrow::batch_reader(
                Arc::clone(&arrow),
                [batch(vec![None], vec![Some("AAPL")])],
            ),
            &options,
        )
        .unwrap();
    handle
        .merge_arrow_reader(
            yggdryl::arrow::batch_reader(
                Arc::clone(&arrow),
                [batch(vec![None], vec![Some("MSFT")])],
            ),
            &options,
        )
        .unwrap();

    // Arrow's row encoding gives absence one exact spelling, so two null keys
    // are the same key rather than two rows that merely both lack a value.
    let total: usize = handle
        .read_arrow_reader(
            &options
                .with_merge_by(yggdryl::expression::Selector::all())
                .unwrap(),
        )
        .unwrap()
        .map(|batch| batch.unwrap().num_rows())
        .sum();
    assert_eq!(total, 1);
}

#[test]
fn a_composite_key_matches_on_every_column() {
    let field = StructType::from_fields([
        DataType::utf8().required_field("venue"),
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("symbol"),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("row");
    let arrow = field.clone().into_arrow_schema().unwrap();
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
        .with_field(field)
        .with_merge_by(["venue", "id"])
        .unwrap();
    handle
        .merge_arrow_reader(
            yggdryl::arrow::batch_reader(
                Arc::clone(&arrow),
                [batch(vec!["XNAS", "XNYS"], vec![1, 1], vec!["a", "b"])],
            ),
            &options,
        )
        .unwrap();
    // The same id at a different venue is a different row.
    handle
        .merge_arrow_reader(
            yggdryl::arrow::batch_reader(
                Arc::clone(&arrow),
                [batch(vec!["XNYS"], vec![1], vec!["b2"])],
            ),
            &options,
        )
        .unwrap();

    let read: Vec<RecordBatch> = handle
        .read_arrow_reader(
            &options
                .clone()
                .with_merge_by(yggdryl::expression::Selector::all())
                .unwrap(),
        )
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
        .merge_arrow_reader(
            reader(vec![rows(vec![1, 2], vec![Some("AAPL"), Some("MSFT")])]),
            &options,
        )
        .unwrap();

    // `id` arrives as text, `symbol` comes first, and `venue` is not declared
    // at all: the cast to the target reorders, converts, and drops before a
    // single key is compared.
    let loose = StructType::from_fields([
        DataType::utf8().nullable_field("symbol"),
        DataType::utf8().required_field("id"),
        DataType::utf8().nullable_field("venue"),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("row");
    let incoming = RecordBatch::try_new(
        loose.clone().into_arrow_schema().unwrap(),
        vec![
            Arc::new(StringArray::from(vec!["MSFT.O"])),
            Arc::new(StringArray::from(vec!["2"])),
            Arc::new(StringArray::from(vec!["XNAS"])),
        ],
    )
    .unwrap();

    handle
        .merge_arrow_reader(
            yggdryl::arrow::batch_reader(incoming.schema(), [incoming]),
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
        .with_field(schema())
        .with_merge_by(["nowhere"])
        .unwrap();

    let message = handle
        .merge_arrow_reader(reader(vec![rows(vec![1], vec![Some("AAPL")])]), &options)
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
        .merge_arrow_reader(
            reader(vec![rows(vec![1, 2], vec![Some("AAPL"), Some("MSFT")])]),
            &options,
        )
        .unwrap();
    handle
        .merge_arrow_reader(
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

#[test]
fn a_selection_narrows_a_read_to_the_named_columns_in_their_order() {
    let mut handle = handle("orders.arrows");
    let options = handle.record_options().unwrap().with_field(schema());
    handle
        .overwrite_arrow_reader(
            reader(vec![rows(vec![1, 2], vec![Some("AAPL"), Some("MSFT")])]),
            &options,
        )
        .unwrap();

    // Selecting one column yields exactly that column; the name matches the
    // way every cast matches, ASCII case-insensitively.
    let selecting = options.clone().with_select("SYMBOL").unwrap();
    let mut symbols = Vec::new();
    for batch in handle.read_arrow_reader(&selecting).unwrap() {
        let batch = batch.unwrap();
        assert_eq!(batch.num_columns(), 1);
        let column = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        for index in 0..column.len() {
            symbols.push(column.value(index).to_owned());
        }
    }
    assert_eq!(symbols, ["AAPL", "MSFT"]);

    // The selection also orders: naming both columns reversed yields them
    // reversed, which a plain read never does.
    let reversed = options.with_select("symbol, id").unwrap();
    let first = handle
        .read_arrow_reader(&reversed)
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    let names: Vec<String> = first
        .schema()
        .fields()
        .iter()
        .map(|field| field.name().clone())
        .collect();
    assert_eq!(names, ["symbol", "id"]);
}

#[test]
fn a_selection_narrows_a_write_and_a_missing_name_is_an_error() {
    let mut handle = handle("orders.arrows");
    // Writing with a selection keeps only the named columns of the incoming
    // rows: the payload column never lands, so it reads back absent.
    let narrowing = handle.record_options().unwrap().with_select("id").unwrap();
    handle
        .overwrite_arrow_reader(
            reader(vec![rows(vec![7, 8], vec![Some("AAPL"), Some("MSFT")])]),
            &narrowing,
        )
        .unwrap();

    let plain = handle.record_options().unwrap();
    let batch = handle
        .read_arrow_reader(&plain)
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(batch.num_columns(), 1);
    assert_eq!(batch.schema().field(0).name(), "id");

    // A name the rows do not have is an error naming what is there.
    let missing = plain.with_select("absent").unwrap();
    let error = handle
        .read_arrow_reader(&missing)
        .err()
        .expect("a missing selected column is an error")
        .to_string();
    assert!(error.contains("absent"), "unexpected error: {error}");
    assert!(error.contains("id"), "unexpected error: {error}");
}

#[test]
fn append_refuses_a_match_key_and_merge_uses_it_explicitly() {
    let mut handle = handle("append-merge.arrows");
    let options = merging(&handle);
    handle
        .merge_arrow_reader(
            reader(vec![rows(vec![1, 2], vec![Some("AAPL"), Some("MSFT")])]),
            &options,
        )
        .unwrap();

    // Intent comes from the method, never from an option. Append therefore
    // refuses a merge key before touching the resource.
    let before = handle.as_slice().to_vec();
    let error = handle
        .append_arrow_reader(
            reader(vec![rows(vec![2, 3], vec![Some("MSFT.O"), Some("NVDA")])]),
            &options,
        )
        .unwrap_err()
        .to_string();
    assert!(error.contains("write mode append"), "{error}");
    assert!(error.contains("merge_by"), "{error}");
    assert_eq!(handle.as_slice(), before.as_slice());

    handle
        .merge_arrow_reader(
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

#[test]
fn an_append_naming_no_match_key_still_appends_every_row() {
    let mut handle = handle("append-plain.arrows");
    let options = merging(&handle)
        .with_merge_by(yggdryl::expression::Selector::all())
        .unwrap();
    handle
        .overwrite_arrow_reader(reader(vec![rows(vec![1], vec![Some("AAPL")])]), &options)
        .unwrap();

    // Without a key nothing identifies a row, so a repeat is a second row -
    // the behaviour the merge branch must not have taken over.
    handle
        .append_arrow_reader(
            reader(vec![rows(vec![1, 2], vec![Some("AAPL"), Some("MSFT")])]),
            &options,
        )
        .unwrap();

    assert_eq!(
        stored(&handle, &options),
        vec![
            (1, Some("AAPL".to_owned())),
            (1, Some("AAPL".to_owned())),
            (2, Some("MSFT".to_owned())),
        ]
    );
}

/// Produce payload batches only when the preceding one has been released.
///
/// This turns incoming-stream retention into a deterministic error instead of
/// relying on an allocator or a process-wide memory watermark.
#[cfg(feature = "internals")]
struct ReleaseCheckedReader {
    schema: SchemaRef,
    next: i64,
    previous: Option<Weak<dyn Array>>,
}

#[cfg(feature = "internals")]
impl Iterator for ReleaseCheckedReader {
    type Item = std::result::Result<RecordBatch, ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self
            .previous
            .as_ref()
            .is_some_and(|array| array.strong_count() != 0)
        {
            return Some(Err(ArrowError::ComputeError(
                "the preceding incoming payload batch was retained".to_owned(),
            )));
        }
        if self.next == 2 {
            return None;
        }
        let id: ArrayRef = Arc::new(Int64Array::from(vec![self.next]));
        self.previous = Some(Arc::downgrade(&id));
        let symbol: ArrayRef = Arc::new(StringArray::from(vec![format!("symbol-{}", self.next)]));
        self.next += 1;
        Some(RecordBatch::try_new(
            Arc::clone(&self.schema),
            vec![id, symbol],
        ))
    }
}

#[cfg(feature = "internals")]
impl arrow_array::RecordBatchReader for ReleaseCheckedReader {
    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }
}

#[cfg(feature = "internals")]
#[test]
fn incoming_payload_batches_are_released_before_the_next_is_pulled() {
    let arrow = schema().into_arrow_schema().unwrap();
    let stored = yggdryl::arrow::batch_reader(Arc::clone(&arrow), []);
    let incoming: BatchReader = Box::new(ReleaseCheckedReader {
        schema: arrow,
        next: 0,
        previous: None,
    });

    let merged = yggdryl::internals::media_merge::merged(
        stored,
        incoming,
        &schema(),
        &yggdryl::Selector::from_columns(["id"]),
        true,
    )
    .unwrap();
    let rows: usize = merged.map(|batch| batch.unwrap().num_rows()).sum();
    assert_eq!(rows, 2);
}
