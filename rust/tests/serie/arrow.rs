//! `rust/src/serie/arrow.rs`: the one door buffers take into a column -
//! what it proves, what it refuses by name, and what crosses back out.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use arrow_array::{
    Array, ArrayRef, Int32Array, Int64Array, ListArray, RecordBatch, RecordBatchIterator,
    RecordBatchReader, StringArray, StructArray,
};
use arrow_buffer::{NullBuffer, OffsetBuffer};
use arrow_schema::{ArrowError, DataType as ArrowDataType, Field as ArrowField, Schema, SchemaRef};
use yggdryl::arrow::{BatchReader, batch_reader};
use yggdryl::{
    ArrowCastOptions, ChunkedSerie, DataType, Field, FieldPath, FieldRecord, Scalar, Selector,
    Serie, SerieReader, SerieReaderWindows, StructType, TimeUnit, Timezone,
};

/// The options a refusal is pinned under: a present value is never nulled.
fn strict() -> ArrowCastOptions {
    ArrowCastOptions::new().with_safe(false)
}

/// A public child must remain readable after it loses its parent context.
fn assert_every_extracted_child_is_readable(column: &Serie) {
    use std::hash::{Hash, Hasher};

    let expected = (0..column.len())
        .map(|row| {
            column
                .scalar(row)
                .expect("every physical child row is readable")
        })
        .collect::<Vec<_>>();
    assert_eq!(column.rows().as_ref(), expected.as_slice());
    let run = Serie::new(expected.clone());
    assert_eq!(column, &run);
    let mut actual_hash = std::collections::hash_map::DefaultHasher::new();
    let mut expected_hash = std::collections::hash_map::DefaultHasher::new();
    column.hash(&mut actual_hash);
    run.hash(&mut expected_hash);
    assert_eq!(actual_hash.finish(), expected_hash.finish());

    let field = column.require_field().unwrap();
    let identity = yggdryl::ArrowCastPlan::compile(field, field, ArrowCastOptions::new())
        .expect("the child's identity plan");
    assert_eq!(
        identity.apply(column).unwrap().rows().as_ref(),
        expected.as_slice()
    );
    let mut appended = Serie::empty(field.clone().with_nullable(true)).unwrap();
    appended
        .extend_from_serie(column)
        .expect("a borrowed child appends safely");
    assert_eq!(appended.rows().as_ref(), expected.as_slice());
    column
        .require_arrow_array()
        .unwrap()
        .to_data()
        .validate_full()
        .expect("required Arrow children remain physically valid");

    for child in column.children() {
        assert_every_extracted_child_is_readable(child);
    }
    if let Some(items) = column.items() {
        assert_every_extracted_child_is_readable(items);
    }
}

#[test]
fn hidden_narrow_values_remain_safe_through_every_public_child_surface() {
    use arrow_array::types::{Int8Type, Int32Type};
    use arrow_array::{
        DictionaryArray, FixedSizeListArray, Int8Array, MapArray, RunArray, UnionArray,
    };
    use arrow_buffer::ScalarBuffer;
    use arrow_schema::{FieldRef, Fields, UnionFields};

    let isin = || {
        DataType::Isin
            .required_field("item")
            .into_arrow_field_ref()
            .unwrap()
    };
    let values = || -> ArrayRef {
        Arc::new(StringArray::from(vec![
            "US0378331005",
            "BAD",
            "GB0002634946",
        ]))
    };
    let plain_field = |array: &ArrayRef| -> FieldRef {
        Arc::new(ArrowField::new("payload", array.data_type().clone(), false))
    };
    let mut cases: Vec<(&str, FieldRef, ArrayRef)> = vec![(
        "struct",
        DataType::Isin
            .required_field("payload")
            .into_arrow_field_ref()
            .unwrap(),
        values(),
    )];
    let fixed: ArrayRef = Arc::new(FixedSizeListArray::new(isin(), 1, values(), None));
    cases.push(("fixed", plain_field(&fixed), fixed));
    let list: ArrayRef = Arc::new(ListArray::new(
        isin(),
        OffsetBuffer::new(vec![0_i32, 1, 2, 3].into()),
        values(),
        None,
    ));
    cases.push(("list", plain_field(&list), list));
    let entry_fields: Fields = vec![
        Arc::new(ArrowField::new("key", ArrowDataType::Utf8, false)),
        DataType::Isin
            .required_field("value")
            .into_arrow_field_ref()
            .unwrap(),
    ]
    .into();
    let map: ArrayRef = Arc::new(MapArray::new(
        Arc::new(ArrowField::new(
            "entries",
            ArrowDataType::Struct(entry_fields.clone()),
            false,
        )),
        OffsetBuffer::new(vec![0_i32, 1, 2, 3].into()),
        StructArray::new(
            entry_fields,
            vec![
                Arc::new(StringArray::from(vec!["first", "hidden", "third"])),
                values(),
            ],
            None,
        ),
        None,
        false,
    ));
    cases.push(("map", plain_field(&map), map));
    for dense in [false, true] {
        let fields = UnionFields::try_new(
            vec![0_i8, 1],
            vec![
                isin(),
                Arc::new(ArrowField::new("number", ArrowDataType::Int64, false)),
            ],
        )
        .unwrap();
        let numbers: ArrayRef = if dense {
            Arc::new(Int64Array::from(vec![7_i64]))
        } else {
            Arc::new(Int64Array::from(vec![0_i64, 7, 0]))
        };
        let union: ArrayRef = Arc::new(
            UnionArray::try_new(
                fields,
                ScalarBuffer::from(vec![0_i8, 1, 0]),
                dense.then(|| ScalarBuffer::from(vec![0_i32, 0, 2])),
                vec![values(), numbers],
            )
            .unwrap(),
        );
        cases.push((
            if dense { "dense_union" } else { "sparse_union" },
            plain_field(&union),
            union,
        ));
    }
    let dictionary_field = DataType::dictionary(DataType::Int8, DataType::Isin)
        .unwrap()
        .required_field("payload")
        .into_arrow_field_ref()
        .unwrap();
    let dictionary: ArrayRef = Arc::new(
        DictionaryArray::<Int8Type>::try_new(Int8Array::from(vec![0_i8, 1, 2]), values()).unwrap(),
    );
    cases.push(("dictionary", dictionary_field, dictionary));
    let run_field = DataType::run_end_encoded(
        DataType::Int32.required_field("run_ends"),
        DataType::Isin.required_field("values"),
    )
    .unwrap()
    .required_field("payload")
    .into_arrow_field_ref()
    .unwrap();
    let runs = RunArray::<Int32Type>::try_new(&Int32Array::from(vec![1, 2, 3]), values().as_ref())
        .unwrap();
    let runs = arrow_array::make_array(
        runs.to_data()
            .into_builder()
            .data_type(run_field.data_type().clone())
            .build()
            .unwrap(),
    );
    cases.push(("run_end", run_field, runs));

    for (name, field, array) in cases {
        let source: ArrayRef = Arc::new(StructArray::new(
            vec![field].into(),
            vec![array],
            Some(NullBuffer::from(vec![true, false, true])),
        ));
        source
            .to_data()
            .validate_full()
            .expect("legal foreign Arrow buffers");
        let declared =
            Field::from_arrow_field(&ArrowField::new("root", source.data_type().clone(), true))
                .unwrap();
        for explicit in [false, true] {
            let column = Serie::from_arrow_array(
                explicit.then_some(&declared),
                Arc::clone(&source),
                ArrowCastOptions::new(),
            )
            .unwrap_or_else(|error| panic!("{name}, explicit={explicit}: {error}"));
            assert_eq!(column.scalar(1).unwrap(), Scalar::Null, "{name}");
            for (row, expected) in [(0, "US0378331005"), (2, "GB0002634946")] {
                let value = column.scalar(row).unwrap();
                assert!(
                    format!("{value:?}").contains(expected),
                    "{name} row {row}: {value:?}"
                );
            }
            let payload = column.child("payload").expect("the public child");
            if matches!(name, "list" | "map") {
                assert_eq!(
                    payload.items().unwrap().len(),
                    2,
                    "{name} removes the unsafe span"
                );
            } else {
                let physical_values = match name {
                    "struct" => payload,
                    "dense_union" | "sparse_union" => payload.child_at(0).unwrap(),
                    _ => payload.items().unwrap(),
                };
                assert_eq!(physical_values.scalar(1).unwrap(), Scalar::Null, "{name}");
            }
            assert_every_extracted_child_is_readable(&column);
        }
    }
}

#[test]
fn a_cast_ingest_proof_does_not_leave_hidden_narrow_bytes_in_a_public_child() {
    let source: ArrayRef = Arc::new(StructArray::new(
        vec![Arc::new(ArrowField::new(
            "code",
            ArrowDataType::Utf8,
            false,
        ))]
        .into(),
        vec![Arc::new(StringArray::from(vec!["US0378331005", "BAD"]))],
        Some(NullBuffer::from(vec![true, false])),
    ));
    let field = Field::new(
        "root",
        DataType::from(StructType::from_fields([DataType::Isin.required_field("code")]).unwrap()),
        true,
    );
    let column = Serie::from_arrow_array(Some(&field), source, ArrowCastOptions::new())
        .expect("a code ingest only validates visible rows");
    assert_eq!(column.scalar(1).unwrap(), Scalar::Null);
    assert_eq!(
        column.child("code").unwrap().scalar(1).unwrap(),
        Scalar::Null
    );
    assert_every_extracted_child_is_readable(&column);
}

/// A non-null record root over an identifier and a symbol.
fn quotes_root() -> Field {
    Field::new(
        "row",
        DataType::from(
            StructType::from_fields([
                Field::new("id", DataType::Int64, false),
                Field::new("symbol", DataType::utf8(), false),
            ])
            .expect("two named children"),
        ),
        false,
    )
}

/// Two quote rows as one Arrow batch.
fn quote_batch() -> RecordBatch {
    let schema = Arc::new(Schema::new(vec![
        ArrowField::new("id", ArrowDataType::Int64, false),
        ArrowField::new("symbol", ArrowDataType::Utf8, false),
    ]));
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Int64Array::from(vec![1, 2])),
            Arc::new(StringArray::from(vec!["AAPL", "MSFT"])),
        ],
    )
    .expect("two rows")
}

/// [`quote_batch`] with its identifiers laid out as int32, which the
/// quotes root declares as int64.
fn narrow_quote_batch() -> RecordBatch {
    let schema = Arc::new(Schema::new(vec![
        ArrowField::new("id", ArrowDataType::Int32, false),
        ArrowField::new("symbol", ArrowDataType::Utf8, false),
    ]));
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Int32Array::from(vec![1, 2])),
            Arc::new(StringArray::from(vec!["AAPL", "MSFT"])),
        ],
    )
    .expect("two rows")
}

/// A required int64 serie field, and three lists - `[1, 2]`, `[3]`,
/// `[4, 5]` - over one child of five.
fn legs() -> (Field, ListArray) {
    let field = Field::new(
        "legs",
        DataType::serie(Field::new("item", DataType::Int64, false)),
        false,
    );
    let lists = ListArray::new(
        Arc::new(ArrowField::new("item", ArrowDataType::Int64, false)),
        OffsetBuffer::new(vec![0_i32, 2, 3, 5].into()),
        Arc::new(Int64Array::from(vec![1_i64, 2, 3, 4, 5])),
        None,
    );
    (field, lists)
}

/// Three prices, as the int64 array a caller holds.
fn prices() -> ArrayRef {
    Arc::new(Int64Array::from(vec![125_i64, 126, 127]))
}

/// A stream of `count` copies of [`quote_batch`].
fn quote_stream(count: usize) -> BatchReader {
    let batch = quote_batch();
    batch_reader(batch.schema(), vec![batch; count])
}

/// The two rows of [`quote_batch`], as values.
fn quote_rows() -> [Scalar; 2] {
    [
        Scalar::from_sequence([Scalar::from(1_i64), Scalar::from("AAPL")]),
        Scalar::from_sequence([Scalar::from(2_i64), Scalar::from("MSFT")]),
    ]
}

#[test]
fn the_buffers_cross_in_as_they_are_and_back_out_shared() {
    let array = Int64Array::from((0..1_024_i64).collect::<Vec<_>>());
    let values = array.values().as_ptr();
    let held: ArrayRef = Arc::new(array);
    let column = Serie::from_arrow_array(
        Some(&Field::new("price", DataType::Int64, false)),
        held,
        ArrowCastOptions::new(),
    )
    .expect("an int64 column");

    // The layout is the datatype's whole contract, so no row is read: the
    // column lends the very buffer the array held.
    assert_eq!(column.len(), 1_024);
    assert_eq!(column.as_int64().unwrap().values().as_ptr(), values);
    assert_eq!(column.scalar(1).unwrap(), Scalar::from(1_i64));

    let back = column.into_arrow_array().expect("a column");
    assert_eq!(back.data_type(), &ArrowDataType::Int64);
    assert_eq!(back.len(), 1_024);
    assert!(
        back.to_data().buffers()[0].as_ptr() == values.cast::<u8>(),
        "the way out shares the buffer too"
    );
    assert!(column.require_arrow_array().is_ok());
}

#[test]
fn a_layout_that_is_not_the_fields_is_cast_and_a_value_no_cast_reaches_is_refused() {
    // An int32 array under an int64 field is converted by the one plan the
    // door compiles: the column holds the field's layout, in buffers of its
    // own rather than the array's.
    let narrow = Int32Array::from(vec![1, 2]);
    let values = narrow.values().as_ptr().cast::<u8>();
    let column = Serie::from_arrow_array(
        Some(&Field::new("price", DataType::Int64, false)),
        Arc::new(narrow),
        strict(),
    )
    .expect("int32 widens into int64");
    assert_eq!(
        column.as_int64().expect("an int64 column").values(),
        &[1, 2]
    );
    let back = column.into_arrow_array().expect("a column");
    assert_eq!(back.data_type(), &ArrowDataType::Int64);
    assert!(
        back.to_data().buffers()[0].as_ptr() != values,
        "a converted column shares nothing with its input"
    );

    // Text no reading takes as an integer is refused under `safe = false`,
    // naming the value and the datatype it could not reach.
    let text: ArrayRef = Arc::new(StringArray::from(vec!["AAPL"]));
    let refusal = Serie::from_arrow_array(
        Some(&Field::new("price", DataType::Int64, false)),
        Arc::clone(&text),
        strict(),
    )
    .expect_err("AAPL is no int64");
    let shown = refusal.to_string();
    assert!(shown.contains("AAPL"), "names the value: {shown}");
    assert!(shown.contains("Int64"), "names the target: {shown}");

    // Under `safe` a required field still refuses it by the value, since
    // the null it would become has nowhere to stand; a nullable one takes
    // that null.
    let refusal = Serie::from_arrow_array(
        Some(&Field::new("price", DataType::Int64, false)),
        Arc::clone(&text),
        ArrowCastOptions::new(),
    )
    .expect_err("a required column holds no null");
    assert!(refusal.to_string().contains("AAPL"), "{refusal}");
    let column = Serie::from_arrow_array(
        Some(&Field::new("price", DataType::Int64, true)),
        text,
        ArrowCastOptions::new(),
    )
    .expect("nulled");
    assert_eq!(column.null_count(), 1);
}

#[test]
fn an_absent_row_is_refused_under_a_required_field_and_admitted_under_a_nullable_one() {
    let absent: ArrayRef = Arc::new(Int64Array::from(vec![Some(1), None]));
    // Whatever `safe` says: the absent row is never repaired to the
    // field's canonical default.
    for options in [strict(), ArrowCastOptions::new()] {
        let refusal = Serie::from_arrow_array(
            Some(&Field::new("price", DataType::Int64, false)),
            Arc::clone(&absent),
            options,
        )
        .expect_err("a required column admits no absent row");
        let text = refusal.to_string();
        assert!(text.contains("$.price"), "names the column: {text}");
        assert!(text.contains("1 null"), "counts the absent rows: {text}");
    }

    let column = Serie::from_arrow_array(
        Some(&Field::new("price", DataType::Int64, true)),
        absent,
        ArrowCastOptions::new(),
    )
    .expect("a nullable column admits it");
    assert_eq!(column.null_count(), 1);
    assert!(column.is_null(1).unwrap());
    assert_eq!(column.scalar(1).unwrap(), Scalar::Null);
}

#[test]
fn a_required_child_under_a_null_record_row_is_admitted() {
    // The child's absent slot is hidden by the record's null, so it is
    // judged only where the record is present.
    let child = ArrowField::new("price", ArrowDataType::Int64, false);
    let records: ArrayRef = Arc::new(StructArray::new(
        vec![Arc::new(child)].into(),
        vec![Arc::new(Int64Array::from(vec![Some(1), None]))],
        Some(NullBuffer::from(vec![true, false])),
    ));
    let root = Field::new(
        "row",
        DataType::from(
            StructType::from_fields([Field::new("price", DataType::Int64, false)]).unwrap(),
        ),
        true,
    );
    let column = Serie::from_arrow_array(Some(&root), Arc::clone(&records), strict())
        .expect("a hidden absent child, even strictly");
    assert_eq!(column.len(), 2);
    assert!(column.is_null(1).unwrap());
    assert_eq!(
        column.scalar(0).unwrap(),
        Scalar::from_sequence([Scalar::from(1_i64)])
    );

    // The same records under a required root: the hidden child is still
    // admitted, and it is the root's own absent row that is refused, at
    // the root's path - a required record is the `$` its children hang
    // from.
    let required = Field::new(
        "row",
        DataType::from(
            StructType::from_fields([Field::new("price", DataType::Int64, false)]).unwrap(),
        ),
        false,
    );
    let refusal = Serie::from_arrow_array(Some(&required), records, strict())
        .expect_err("the record row is absent");
    let text = refusal.to_string();
    assert!(text.contains("field $ "), "names the root: {text}");
    assert!(text.contains("1 null"), "counts the absent rows: {text}");
}

#[test]
fn a_value_the_fields_contract_refuses_is_refused_at_the_door_naming_the_row() {
    // A code rides Arrow's own text layout, so the layout admits any text
    // and the door reads each row once through the field's contract.
    let text: ArrayRef = Arc::new(StringArray::from(vec!["USD", "EUR", "TOOLONGCCY"]));
    let refusal = Serie::from_arrow_array(
        Some(&Field::new("ccy", DataType::Ccy, false)),
        Arc::clone(&text),
        strict(),
    )
    .expect_err("a currency is at most eight bytes");
    let shown = refusal.to_string();
    assert!(shown.contains("ccy"), "names the column: {shown}");
    assert!(shown.contains("row 2"), "names the row: {shown}");

    // Under `safe` the refused value is nulled rather than refused.
    let nulled = Serie::from_arrow_array(
        Some(&Field::new("ccy", DataType::Ccy, true)),
        Arc::clone(&text),
        ArrowCastOptions::new(),
    )
    .expect("EURO is nulled");
    assert_eq!(nulled.null_count(), 1);
    assert!(nulled.is_null(2).unwrap());

    // Under a nullable code field an absent row is not a value, and is
    // not read.
    let text: ArrayRef = Arc::new(StringArray::from(vec![Some("USD"), None]));
    let column = Serie::from_arrow_array(
        Some(&Field::new("ccy", DataType::Ccy, true)),
        text,
        strict(),
    )
    .expect("two rows, one absent");
    assert_eq!(column.null_count(), 1);
    assert!(column.scalar(0).unwrap().is_code());

    // The plain UTF-8 layout is its own contract: nothing is read, and the
    // same bytes cross in.
    let text: ArrayRef = Arc::new(StringArray::from(vec!["AB", "ABCD"]));
    let refusal = Serie::from_arrow_array(
        Some(&Field::new("code", DataType::sized_utf8(2).unwrap(), false)),
        Arc::clone(&text),
        strict(),
    )
    .expect_err("a value past the size is not one the field accepts");
    assert!(refusal.to_string().contains("row 1"), "{refusal}");
    assert!(
        Serie::from_arrow_array(
            Some(&Field::new("code", DataType::utf8(), false)),
            text,
            strict()
        )
        .is_ok()
    );
}

#[test]
fn from_scalars_proves_the_rows_once_and_lays_them_out_once() {
    let column = Serie::from_scalars(
        Field::new("price", DataType::Int64, false),
        [Scalar::from(1_i8), Scalar::from(2_i16)],
    )
    .expect("two rows the field rewrites");
    assert_eq!(
        column.as_int64().expect("an int64 column").values(),
        &[1, 2]
    );

    // A code column's rows are proven by the contract on the way in, and
    // the door does not read them again: the column reads back as codes.
    let codes = Serie::from_scalars(
        Field::new("ccy", DataType::Ccy, false),
        [Scalar::from("USD"), Scalar::from("EUR")],
    )
    .expect("two registered currencies");
    assert!(codes.scalar(1).unwrap().is_code());
    assert_eq!(
        codes
            .as_utf8()
            .expect("a code rides utf8")
            .payload()
            .as_slice(),
        b"USDEUR"
    );

    let refusal = Serie::from_scalars(
        Field::new("price", DataType::Int64, false),
        [Scalar::from(1_i64), Scalar::Null],
    )
    .expect_err("a required column admits no absent row");
    assert!(refusal.to_string().contains("price"));
    let refusal = Serie::from_scalars(
        Field::new("ccy", DataType::Ccy, false),
        [Scalar::from("USD"), Scalar::from("TOOLONGCCY")],
    )
    .expect_err("a currency is at most eight bytes");
    assert!(refusal.to_string().contains("ccy"), "{refusal}");
}

#[test]
fn a_sliced_list_array_is_rebased_onto_the_items_it_reaches() {
    let (field, lists) = legs();

    // Arrow slices a list by slicing its offsets and keeping the whole
    // child: the middle row is offsets [2, 3] over a child of five. The door
    // rebases the cut onto exactly the items it reaches, so a column that
    // grows knows where its items end.
    let middle: ArrayRef = Arc::new(lists.slice(1, 1));
    let mut column = Serie::from_arrow_array(Some(&field), middle, ArrowCastOptions::new())
        .expect("a sliced serie");
    assert_eq!(column.len(), 1);
    assert_eq!(
        column.scalar(0).unwrap(),
        Scalar::from_sequence([Scalar::from(3_i64)])
    );
    let leaf = column.as_serie().expect("a serie column");
    assert_eq!(leaf.offsets().as_ref(), &[0, 1]);
    assert_eq!(leaf.items().as_int64().unwrap().values(), &[3]);
    column
        .push(Scalar::from_sequence([Scalar::from(99_i64)]))
        .expect("one more row");
    assert_eq!(
        column.rows().as_ref(),
        &[
            Scalar::from_sequence([Scalar::from(3_i64)]),
            Scalar::from_sequence([Scalar::from(99_i64)]),
        ]
    );

    // A tail slice is rebased the same way, its items sliced to match.
    let tail: ArrayRef = Arc::new(lists.slice(1, 2));
    let column = Serie::from_arrow_array(Some(&field), tail, ArrowCastOptions::new())
        .expect("a sliced serie");
    let leaf = column.as_serie().expect("a serie column");
    assert_eq!(leaf.offsets().as_ref(), &[0, 1, 3]);
    assert_eq!(leaf.items().as_int64().unwrap().values(), &[3, 4, 5]);

    // An unsliced array is already in that shape and is taken untouched:
    // the offsets buffer is the array's own.
    let offsets = lists.offsets().inner().inner().clone();
    let whole: ArrayRef = Arc::new(lists);
    let column = Serie::from_arrow_array(Some(&field), whole, ArrowCastOptions::new())
        .expect("a serie column");
    let leaf = column.as_serie().expect("a serie column");
    assert_eq!(leaf.offsets().as_ref(), &[0, 2, 3, 5]);
    assert!(leaf.offsets().inner().inner().ptr_eq(&offsets));
    assert_eq!(leaf.items().len(), 5);
}

#[test]
fn an_empty_column_names_its_datatype_and_a_reserved_one_is_still_empty() {
    let empty = Serie::empty(Field::new("price", DataType::Int64, false)).unwrap();
    assert!(empty.is_empty());
    assert_eq!(
        empty.dtype().unwrap(),
        DataType::serie(Field::new("item", DataType::Int64, false))
    );

    let reserved = Serie::with_capacity(Field::new("price", DataType::Int64, true), 64).unwrap();
    assert!(reserved.is_empty());
    assert!(reserved.as_int64().is_some());

    let nested = Serie::with_capacity(quotes_root(), 8).unwrap();
    assert!(nested.is_empty());
    assert_eq!(nested.children().len(), 2);

    let (field, _) = legs();
    let lists = Serie::with_capacity(field, 8).unwrap();
    assert!(lists.is_empty());
    assert_eq!(lists.items().map(Serie::len), Some(0));
}

#[test]
fn a_record_root_crosses_from_a_batch_and_back_into_one() {
    let batch = quote_batch();
    let column =
        Serie::from_arrow_batch(None, &batch, ArrowCastOptions::new()).expect("a record column");
    assert_eq!(column.field().map(Field::name), Some("row"));
    assert_eq!(column.len(), 2);
    assert_eq!(
        column.scalar(1).unwrap(),
        Scalar::from_sequence([Scalar::from(2_i64), Scalar::from("MSFT")])
    );
    // The batch's own columns are this column's children, shared.
    assert!(
        column
            .child("id")
            .and_then(Serie::into_arrow_array)
            .is_some_and(
                |ids| ids.to_data().buffers()[0].ptr_eq(&batch.column(0).to_data().buffers()[0])
            )
    );

    let back = column.into_arrow_batch().expect("a batch");
    assert_eq!(back.num_rows(), 2);
    assert_eq!(back.num_columns(), 2);
    assert_eq!(back.schema().field(1).name(), "symbol");

    let mut reader = column.into_arrow_reader().expect("a reader");
    let first = reader.next().expect("one batch").expect("a batch");
    assert_eq!(first.num_rows(), 2);
    assert!(reader.next().is_none());

    let drained = Serie::from_arrow_reader(
        None,
        column.into_arrow_reader().unwrap(),
        ArrowCastOptions::new(),
    )
    .unwrap();
    assert_eq!(drained, column);
    assert_eq!(drained.field().map(Field::name), Some("row"));

    // A column that is not a record is the one column of a row root.
    let prices: ArrayRef = Arc::new(Int64Array::from(vec![1]));
    let prices = Serie::from_arrow_array(
        Some(&Field::new("price", DataType::Int64, false)),
        prices,
        ArrowCastOptions::new(),
    )
    .unwrap();
    let table = prices.into_arrow_batch().expect("a leaf is one column");
    assert_eq!(table.schema().field(0).name(), "price");
    assert_eq!(table.num_rows(), 1);
    assert!(
        Serie::new(vec![Scalar::from(1_i64)])
            .into_arrow_batch()
            .is_err()
    );
    assert!(
        Serie::new(vec![Scalar::from(1_i64)])
            .into_arrow_reader()
            .is_err()
    );
}

#[test]
fn a_record_column_holding_an_absent_row_is_not_a_table() {
    let child = Field::new("id", DataType::Int64, false);
    let root = Field::new(
        "row",
        DataType::from(StructType::from_fields([child]).expect("one child")),
        true,
    );
    let records = StructArray::new(
        vec![ArrowField::new("id", ArrowDataType::Int64, false)].into(),
        vec![Arc::new(Int64Array::from(vec![1, 2])) as ArrayRef],
        Some(NullBuffer::from(vec![true, false])),
    );
    let serie = Serie::from_arrow_array(Some(&root), Arc::new(records), ArrowCastOptions::new())
        .expect("a nullable record column");
    let refusal = serie
        .into_arrow_batch()
        .expect_err("a batch states no row validity");
    assert!(refusal.to_string().contains("1 absent rows"), "{refusal}");
}

#[test]
fn a_column_that_is_not_a_record_crosses_as_the_one_column_of_a_row_root() {
    let field = Field::new("price", DataType::Int64, true);
    let serie = Serie::from_scalars(field, [Scalar::from(1_i64), Scalar::Null]).expect("two rows");
    let batch = serie.into_arrow_batch().expect("one column");
    assert_eq!(batch.num_columns(), 1);
    assert_eq!(batch.schema().field(0).name(), "price");
    assert_eq!(batch.num_rows(), 2);
    let reader = serie.into_arrow_reader().expect("one batch");
    assert_eq!(reader.schema(), batch.schema());
}

#[test]
fn null_is_absence_under_a_required_field_except_where_it_is_the_datatypes_own_default() {
    // A required `null` column holds only nulls: null is its one value.
    let nothing = Field::new("nothing", DataType::Null, false);
    let nulls: ArrayRef = Arc::new(arrow_array::NullArray::new(3));
    let serie = Serie::from_arrow_array(Some(&nothing), nulls, ArrowCastOptions::new())
        .expect("null is null's default");
    assert_eq!(serie.len(), 3);

    // Every other required column still refuses one.
    let ids: ArrayRef = Arc::new(Int64Array::from(vec![Some(1), None]));
    let refusal = Serie::from_arrow_array(
        Some(&Field::new("id", DataType::Int64, false)),
        ids,
        strict(),
    )
    .expect_err("an int64 is not absent by default");
    assert!(refusal.to_string().contains("$.id"), "{refusal}");
    let symbols: ArrayRef = Arc::new(StringArray::from(vec![Some("AAPL"), None]));
    assert!(
        Serie::from_arrow_array(
            Some(&Field::new("symbol", DataType::utf8(), false)),
            symbols,
            strict()
        )
        .is_err()
    );
}

#[test]
fn the_default_column_repeats_one_row_the_field_lays_out() {
    let required =
        Serie::from_default(Field::new("id", DataType::Int64, false), 3).expect("zero three times");
    assert_eq!(
        required.as_int64().expect("an int64 column").values(),
        &[0, 0, 0]
    );
    let nullable =
        Serie::from_default(Field::new("id", DataType::Int64, true), 2).expect("null twice");
    assert_eq!(nullable.null_count(), 2);
    assert_eq!(
        Serie::from_default(Field::new("id", DataType::Int64, false), 0)
            .expect("no rows")
            .len(),
        0
    );
}

#[test]
fn one_row_is_an_arrow_scalar_and_any_other_length_is_refused() {
    let field = Field::new("id", DataType::Int64, false);
    let one = Serie::from_scalars(field.clone(), [Scalar::from(7_i64)]).expect("one row");
    let datum = one.into_arrow_scalar().expect("one row is a scalar");
    let (array, is_scalar) = arrow_array::Datum::get(&datum);
    assert!(is_scalar);
    assert_eq!(array.len(), 1);
    let two = Serie::from_scalars(field, [Scalar::from(1_i64), Scalar::from(2_i64)]).expect("two");
    let refusal = two
        .into_arrow_scalar()
        .expect_err("two rows are not a scalar");
    assert!(refusal.to_string().contains("got 2"), "{refusal}");
    assert!(
        Serie::new(vec![Scalar::from(1_i64)])
            .into_arrow_scalar()
            .is_err()
    );
}

#[test]
fn a_batch_casts_into_a_declared_root_and_one_already_in_it_shares_its_columns() {
    let root = quotes_root();
    let batch = quote_batch();
    let exact = Serie::from_arrow_batch(Some(&root), &batch, strict()).expect("the root's layout");
    assert_eq!(exact.field(), Some(&root));
    assert!(
        exact
            .child("id")
            .and_then(Serie::into_arrow_array)
            .is_some_and(
                |ids| ids.to_data().buffers()[0].ptr_eq(&batch.column(0).to_data().buffers()[0])
            ),
        "an exact batch lands on its own buffers"
    );

    // Int32 identifiers are widened by the one plan into the declared
    // int64, and the rows are the rows of the exact batch.
    let widened = Serie::from_arrow_batch(Some(&root), &narrow_quote_batch(), strict())
        .expect("int32 widens into int64");
    assert_eq!(widened, exact);
    assert_eq!(
        widened
            .child("id")
            .and_then(Serie::as_int64)
            .map(|ids| ids.values().to_vec()),
        Some(vec![1, 2])
    );

    // A table has no row validity, so a nullable root is refused before a
    // row is read.
    let refusal = Serie::from_arrow_batch(
        Some(&root.clone().with_nullable(true)),
        &batch,
        ArrowCastOptions::new(),
    )
    .expect_err("a batch lands under a non-null record");
    assert!(refusal.to_string().contains("non-nullable"), "{refusal}");
}

#[test]
fn a_stream_is_one_record_column_per_batch_under_one_plan() {
    let root = quotes_root();
    let narrow = narrow_quote_batch();
    let stream = || batch_reader(narrow.schema(), [narrow.clone(), narrow.slice(1, 1)]);

    let series = SerieReader::from_arrow_reader(Some(&root), stream(), strict()).expect("one plan");
    assert_eq!(series.field(), &root);
    let landed = series
        .collect::<Result<Vec<Serie>, _>>()
        .expect("two batches");
    assert_eq!(landed.iter().map(Serie::len).collect::<Vec<_>>(), [2, 1]);
    assert_eq!(
        landed[1].scalar(0).unwrap(),
        Scalar::from_sequence([Scalar::from(2_i64), Scalar::from("MSFT")])
    );

    // Drained, the stream is one column of every row it carried.
    let drained = Serie::from_arrow_reader(Some(&root), stream(), strict()).expect("three rows");
    assert_eq!(drained.len(), 3);
    assert_eq!(drained.field(), Some(&root));

    // The transport face reconciles each batch to the root, landing none.
    let mut transport = SerieReader::from_arrow_reader(Some(&root), stream(), strict())
        .expect("one plan")
        .into_arrow_reader();
    assert_eq!(
        transport.schema().field(0).data_type(),
        &ArrowDataType::Int64
    );
    let first = transport
        .next()
        .expect("a batch")
        .expect("a reconciled batch");
    assert_eq!(first.column(0).data_type(), &ArrowDataType::Int64);
    assert_eq!(first.num_rows(), 2);

    // A stream no plan reaches the root from is refused by the constructor,
    // before a batch is pulled: no source column carries `id`.
    let symbols = quote_batch().project(&[1]).expect("the symbol column");
    let refusal = SerieReader::from_arrow_reader(
        Some(&root),
        batch_reader(symbols.schema(), [symbols]),
        strict(),
    )
    .expect_err("a required identifier no column carries");
    assert!(refusal.to_string().contains("id"), "{refusal}");
}

#[test]
fn a_column_in_hand_casts_once_and_one_under_its_own_field_is_itself() {
    let field = Field::new("id", DataType::Int32, true);
    let ids =
        Serie::from_scalars(field.clone(), [Scalar::from(1_i32), Scalar::Null]).expect("two rows");
    let values = ids.as_int32().expect("an int32 column").values().as_ptr();

    let same = ids.cast(&field, strict()).expect("its own field");
    assert_eq!(same.as_int32().unwrap().values().as_ptr(), values);

    let wide = ids
        .cast(&Field::new("id", DataType::Int64, true), strict())
        .expect("int32 widens into int64");
    assert_eq!(wide.as_int64().expect("an int64 column").values()[0], 1);
    assert!(wide.is_null(1).unwrap());

    // The absent row is judged again under a required field and refused,
    // whatever `safe` says: no default is invented for it.
    let required = Field::new("id", DataType::Int32, false);
    for options in [strict(), ArrowCastOptions::new()] {
        let refusal = ids
            .cast(&required, options)
            .expect_err("a required column admits no absent row");
        assert!(refusal.to_string().contains("id"), "{refusal}");
    }

    // A run lays out no buffers for a plan to read.
    assert!(
        Serie::new(vec![Scalar::from(1_i32)])
            .cast(&field, strict())
            .is_err()
    );
}

#[test]
fn an_extension_label_is_not_a_proof() {
    // The column claims to be a URL; the landing still reads what it holds.
    let url = Field::new("u", DataType::Url, false);
    let labelled = url
        .clone()
        .into_arrow_field_ref()
        .expect("a url projects")
        .as_ref()
        .clone();
    let schema = Arc::new(Schema::new(vec![labelled]));
    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(StringArray::from(vec!["not a url"])) as ArrayRef],
    )
    .expect("one row");
    let root = Field::new(
        "row",
        DataType::from(StructType::from_fields([url]).expect("one child")),
        false,
    );
    let refusal =
        Serie::from_arrow_batch(Some(&root), &batch, strict()).expect_err("a label proves nothing");
    // The refusal names the row the batch holds it at, and the column.
    let message = refusal.to_string();
    assert!(message.contains("$[0].u"), "{message}");
}

#[test]
fn a_kernel_into_a_rule_governed_leaf_lands_no_row_its_field_refuses() {
    // Arrow reads any int64 as a date64; a date64 here is a whole day.
    let millis: ArrayRef = Arc::new(Int64Array::from(vec![1_i64]));
    let day = Field::new("day", DataType::Date64, false);
    assert!(Serie::from_arrow_array(Some(&day), millis, strict()).is_err());
    let midnight: ArrayRef = Arc::new(Int64Array::from(vec![86_400_000_i64]));
    let serie = Serie::from_arrow_array(Some(&day), midnight, strict()).expect("a whole day lands");
    assert_eq!(serie.len(), 1);
}

#[test]
fn a_serie_reader_casts_each_batch_by_one_plan_and_fuses_after_a_failure() {
    let schema = Arc::new(Schema::new(vec![ArrowField::new(
        "id",
        ArrowDataType::Int32,
        true,
    )]));
    let good = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![Arc::new(arrow_array::Int32Array::from(vec![Some(1)])) as ArrayRef],
    )
    .unwrap();
    let absent = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![Arc::new(arrow_array::Int32Array::from(vec![None::<i32>])) as ArrayRef],
    )
    .unwrap();
    let root = Field::new(
        "row",
        DataType::from(
            StructType::from_fields([Field::new("id", DataType::Int64, false)]).unwrap(),
        ),
        false,
    );
    let reader = yggdryl::arrow::batch_reader(Arc::clone(&schema), [good.clone(), absent, good]);
    let mut series =
        yggdryl::SerieReader::from_arrow_reader(Some(&root), reader, strict()).expect("one plan");
    assert_eq!(series.field(), &root);
    let first = series
        .next()
        .expect("a batch")
        .expect("the first batch casts");
    assert_eq!(first.len(), 1);
    assert!(series.next().expect("a batch").is_err());
    assert!(series.next().is_none(), "fused after the failure");
}

#[test]
fn an_identity_serie_reader_hands_its_inner_reader_back() {
    let batch = quote_batch();
    let root = Field::from_arrow_schema("row", batch.schema().as_ref()).expect("the root");
    let reader = yggdryl::arrow::batch_reader(batch.schema(), [batch.clone()]);
    let transport =
        yggdryl::SerieReader::from_arrow_reader(Some(&root), reader, ArrowCastOptions::new())
            .expect("an identity plan")
            .into_arrow_reader();
    let back: Vec<RecordBatch> = transport.map(Result::unwrap).collect();
    assert_eq!(back.len(), 1);
    assert!(Arc::ptr_eq(back[0].column(0), batch.column(0)));
}

#[test]
fn a_nullable_record_over_an_uninhabited_child_defaults_to_an_absent_row() {
    // The child can hold nothing, so it has no default and no null default;
    // the record's own absence is what its default row is.
    let inner = Field::new(
        "inner",
        DataType::from(
            StructType::from_fields([Field::new("required_null", DataType::Null, false)])
                .expect("one child"),
        ),
        false,
    );
    let outer = Field::new(
        "outer",
        DataType::from(StructType::from_fields([inner]).expect("one child")),
        true,
    );
    let serie = Serie::from_default(outer.clone(), 2).expect("two absent rows");
    assert_eq!(serie.null_count(), 2);
    let rows = Serie::from_scalars(outer, [Scalar::Null]).expect("an absent row");
    assert_eq!(rows.null_count(), 1);
}

#[test]
fn a_zero_width_serie_default_keeps_its_row_count() {
    let empty = Field::new(
        "empty",
        DataType::fixed_size_serie(Field::new("item", DataType::Int32, true), 0).expect("width 0"),
        false,
    );
    let rows = Serie::from_default(empty, 3).expect("three rows");
    assert_eq!(rows.len(), 3);
    assert_eq!(rows.require_arrow_array().expect("an array").len(), 3);
    let one = rows.slice(0, 1).expect("one row");
    assert!(one.into_arrow_scalar().is_ok());
}

#[test]
fn a_held_column_counts_its_rows_and_its_table_counts_its_columns() {
    let price = Field::new("price", DataType::Int64, false);
    let one = Serie::from_scalars(price.clone(), [Scalar::from(125_i64)]).expect("one row");
    assert_eq!(one.len(), 1);
    assert_eq!(one.into_arrow_batch().expect("one column").num_columns(), 1);

    let many =
        Serie::from_arrow_array(Some(&price), prices(), ArrowCastOptions::new()).expect("a column");
    assert_eq!(many.len(), 3);

    let quotes = Serie::from_arrow_batch(None, &quote_batch(), ArrowCastOptions::new())
        .expect("a record column");
    assert_eq!(quotes.len(), 2);
    assert_eq!(quotes.children().len(), 2);
    assert_eq!(quotes.field().map(Field::name), Some("row"));
    assert_eq!(quotes.into_arrow_batch().expect("a table").num_columns(), 2);

    // A batch narrows to the struct column its rows already are.
    let array = quotes.into_arrow_array().expect("a column");
    let records = array
        .as_any()
        .downcast_ref::<StructArray>()
        .expect("rows are a struct column");
    assert_eq!(records.len(), 2);
    assert_eq!(records.num_columns(), 2);
}

#[test]
fn a_stream_states_its_root_before_a_batch_is_pulled_and_drains_every_batch() {
    let pulls = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&pulls);
    let batch = quote_batch();
    let schema = batch.schema();
    let reader: BatchReader = Box::new(RecordBatchIterator::new(
        std::iter::repeat_n(batch, 2).map(move |batch| {
            counted.fetch_add(1, Ordering::Relaxed);
            Ok(batch)
        }),
        schema,
    ));
    let series = SerieReader::from_arrow_reader(None, reader, ArrowCastOptions::new())
        .expect("the reader names its root");
    assert_eq!(series.field(), &quotes_root());
    assert_eq!(pulls.load(Ordering::Relaxed), 0, "nothing was read");

    // A stream has no length until it is drained, and draining it pulls
    // every batch it holds.
    let rows: usize = series
        .map(|column| column.expect("a batch lands").len())
        .sum();
    assert_eq!(rows, 4);
    assert_eq!(pulls.load(Ordering::Relaxed), 2);

    // Drained into one column, the stream concatenates every batch.
    let drained = Serie::from_arrow_reader(None, quote_stream(2), ArrowCastOptions::new())
        .expect("the stream drains");
    assert_eq!(drained.len(), 4);
    assert_eq!(
        Scalar::from(drained),
        Scalar::from_sequence([quote_rows(), quote_rows()].concat())
    );
}

#[test]
fn a_record_column_with_every_row_present_is_its_own_table_whatever_its_nullability() {
    let records = Serie::from_arrow_batch(None, &quote_batch(), ArrowCastOptions::new())
        .expect("a record column")
        .require_arrow_array()
        .expect("a record array");
    // A record column's children are the columns of its table, and a
    // nullable one that holds no absent row states nothing a batch cannot -
    // so it is not wrapped as the one column of another root.
    for nullable in [false, true] {
        let field = quotes_root().with_nullable(nullable);
        let column = Serie::from_arrow_array(Some(&field), Arc::clone(&records), strict())
            .expect("the records pair");
        let table = column.into_arrow_batch().expect("a table");
        assert_eq!(table.num_columns(), 2, "nullable {nullable}");
        assert_eq!(table.schema().field(0).name(), "id", "nullable {nullable}");
    }
}

#[test]
fn a_foreign_layout_is_cast_into_the_declared_field_rather_than_misread() {
    // Int64 values under a text field are converted by the one plan, each
    // to its text, and never read as the bytes of a string.
    let text = Field::new("price", DataType::utf8(), false);
    let column =
        Serie::from_arrow_array(Some(&text), prices(), strict()).expect("int64 casts to text");
    assert_eq!(column.field(), Some(&text));
    assert_eq!(
        column.rows().as_ref(),
        &[
            Scalar::from("125"),
            Scalar::from("126"),
            Scalar::from("127")
        ]
    );
}

#[test]
fn a_declared_root_types_the_record_column_down_to_its_nullability() {
    let widened = Field::new(
        "row",
        DataType::from(
            StructType::from_fields([
                Field::new("id", DataType::Int64, false),
                Field::new("symbol", DataType::utf8(), true),
            ])
            .expect("two named children"),
        ),
        false,
    );
    // Nullability is part of the root, so the column carries the declared
    // one - the batch's required symbol widens into a nullable one - and the
    // rows are the rows of the batch's own schema.
    let exact = Serie::from_arrow_batch(Some(&quotes_root()), &quote_batch(), strict())
        .expect("the batch's own root");
    let column = Serie::from_arrow_batch(Some(&widened), &quote_batch(), strict())
        .expect("a required column widens into a nullable one");
    assert_eq!(exact.field(), Some(&quotes_root()));
    assert_eq!(column.field(), Some(&widened));
    assert_eq!(column.rows(), exact.rows());
}

#[test]
fn rows_cross_into_a_record_column_and_back_as_the_same_value() {
    let column = Serie::from_scalars(quotes_root(), quote_rows()).expect("the rows materialize");
    assert_eq!(column.len(), 2);
    assert_eq!(
        Scalar::from(column.clone()),
        Scalar::from_sequence(quote_rows())
    );

    // The column streamed out and drained back in is the same one sequence.
    let drained = Serie::from_arrow_reader(
        None,
        column.into_arrow_reader().expect("one batch"),
        ArrowCastOptions::new(),
    )
    .expect("the stream drains");
    assert_eq!(Scalar::from(drained), Scalar::from_sequence(quote_rows()));
}

#[test]
fn every_held_column_widens_to_the_one_reader_a_record_write_takes() {
    let price = Field::new("price", DataType::Int64, false);
    let cases = [
        (
            Serie::from_arrow_array(Some(&price), prices(), ArrowCastOptions::new())
                .expect("a column"),
            3,
        ),
        (
            Serie::from_arrow_batch(None, &quote_batch(), ArrowCastOptions::new())
                .expect("a record column"),
            2,
        ),
    ];
    for (column, rows) in cases {
        let direct = column
            .into_arrow_reader()
            .expect("a held column is one batch");
        assert_eq!(
            direct
                .map(|batch| batch.expect("a batch reads").num_rows())
                .sum::<usize>(),
            rows
        );
        let series = SerieReader::from_serie(column).expect("a held column is one record column");
        assert_eq!(
            series
                .into_arrow_reader()
                .map(|batch| batch.expect("a batch reads").num_rows())
                .sum::<usize>(),
            rows
        );
    }
}

#[test]
fn an_integer_column_casts_into_a_float_one_row_for_row() {
    let source = Field::new("price", DataType::Int64, false);
    let target = Field::new("price", DataType::Float64, false);
    let column =
        Serie::from_arrow_array(Some(&source), prices(), ArrowCastOptions::new()).expect("int64");

    let cast = column
        .cast(&target, ArrowCastOptions::new())
        .expect("int64 widens to float64");
    assert_eq!(cast.field(), Some(&target));
    assert_eq!(cast.len(), 3);
    assert_eq!(
        cast.as_float64().expect("a float64 column").values(),
        &[125.0, 126.0, 127.0]
    );
}

#[test]
fn one_plan_casts_every_batch_a_stream_yields() {
    let target = Field::new(
        "row",
        DataType::from(
            StructType::from_fields([
                Field::new("id", DataType::Float64, false),
                Field::new("symbol", DataType::utf8(), false),
            ])
            .expect("two named children"),
        ),
        false,
    );
    let expected = target
        .clone()
        .into_arrow_schema()
        .expect("the root projects to Arrow");
    let batches: Vec<RecordBatch> =
        SerieReader::from_arrow_reader(Some(&target), quote_stream(3), ArrowCastOptions::new())
            .expect("int64 widens to float64")
            .into_arrow_reader()
            .map(|batch| batch.expect("a batch reads"))
            .collect();

    // The plan is compiled once from the reader schema, so every batch - not
    // only the first - arrives under the declared root.
    assert_eq!(batches.len(), 3);
    for batch in &batches {
        assert_eq!(batch.schema(), expected);
        let rows =
            Serie::from_arrow_batch(None, batch, ArrowCastOptions::new()).expect("a record column");
        assert_eq!(
            Scalar::from(rows),
            Scalar::from_sequence([
                Scalar::from_sequence([Scalar::from(1.0_f64), Scalar::from("AAPL")]),
                Scalar::from_sequence([Scalar::from(2.0_f64), Scalar::from("MSFT")]),
            ])
        );
    }
}

#[test]
fn a_held_record_column_reads_as_a_stream_of_itself() {
    let records = Serie::from_arrow_batch(None, &quote_batch(), ArrowCastOptions::new())
        .expect("a record column");
    let series = SerieReader::from_serie(records.clone()).expect("a record column");
    assert_eq!(series.field(), &quotes_root());
    let yielded = series
        .collect::<Result<Vec<Serie>, _>>()
        .expect("one column");
    assert_eq!(yielded, std::slice::from_ref(&records));

    // Its transport face is the one batch the column is.
    let mut transport = SerieReader::from_serie(records)
        .expect("a record column")
        .into_arrow_reader();
    let batch = transport.next().expect("a batch").expect("a table");
    assert_eq!(batch.num_rows(), 2);
    assert_eq!(batch.schema(), quote_batch().schema());
    assert!(transport.next().is_none());
}

#[test]
fn a_held_column_that_is_not_a_record_is_the_one_child_of_a_record() {
    let price = Field::new("price", DataType::Int64, false);
    let prices =
        Serie::from_arrow_array(Some(&price), prices(), ArrowCastOptions::new()).expect("a column");
    let series = SerieReader::from_serie(prices.clone()).expect("a leaf is one column");
    assert_eq!(series.field().name(), yggdryl::media::DEFAULT_ROOT_NAME);
    assert!(!series.field().is_nullable());
    assert_eq!(
        series.field().dtype().as_fields().map(<[Field]>::to_vec),
        Some(vec![price])
    );
    let yielded = series
        .collect::<Result<Vec<Serie>, _>>()
        .expect("one column");
    assert_eq!(yielded.len(), 1);
    assert_eq!(yielded[0].len(), 3);
    assert_eq!(yielded[0].child("price"), Some(&prices));
}

#[test]
fn a_run_and_a_record_column_holding_an_absent_row_are_no_stream() {
    assert!(SerieReader::from_serie(Serie::new(vec![Scalar::from(1_i64)])).is_err());

    let root = quotes_root().with_nullable(true);
    let records = StructArray::new(
        vec![
            ArrowField::new("id", ArrowDataType::Int64, false),
            ArrowField::new("symbol", ArrowDataType::Utf8, false),
        ]
        .into(),
        vec![
            Arc::new(Int64Array::from(vec![1, 2])) as ArrayRef,
            Arc::new(StringArray::from(vec!["AAPL", "MSFT"])),
        ],
        Some(NullBuffer::from(vec![true, false])),
    );
    let serie = Serie::from_arrow_array(Some(&root), Arc::new(records), ArrowCastOptions::new())
        .expect("a nullable record column");
    let refusal = SerieReader::from_serie(serie).expect_err("a table states no row validity");
    assert!(refusal.to_string().contains("1 absent rows"), "{refusal}");
}

/// The quotes of [`quote_batch`] held as two chunks - both rows, then the
/// second again - under the root the batch names.
fn chunked_quotes() -> ChunkedSerie {
    let batch = quote_batch();
    ChunkedSerie::from_arrow_reader(
        None,
        batch_reader(batch.schema(), vec![batch.clone(), batch.slice(1, 1)]),
        ArrowCastOptions::new(),
    )
    .expect("two batches")
}

#[test]
fn a_held_chunked_record_column_reads_as_the_stream_of_its_chunks() {
    let chunked = chunked_quotes();
    let series = SerieReader::from_chunked(chunked.clone()).expect("a record column");
    assert_eq!(series.field(), &quotes_root());
    let yielded = series
        .collect::<Result<Vec<Serie>, _>>()
        .expect("every chunk");
    assert_eq!(yielded, chunked.chunks());
    for (record, chunk) in yielded.iter().zip(chunked.chunks()) {
        let (Some(record), Some(chunk)) = (record.child("id"), chunk.child("id")) else {
            panic!("every chunk has its ids");
        };
        assert!(
            record
                .require_arrow_array()
                .expect("ids")
                .to_data()
                .buffers()[0]
                .ptr_eq(
                    &chunk
                        .require_arrow_array()
                        .expect("ids")
                        .to_data()
                        .buffers()[0]
                ),
            "nothing is copied"
        );
    }

    // Its transport face is one batch per chunk, in order.
    let batches = SerieReader::from_chunked(chunked)
        .expect("a record column")
        .into_arrow_reader()
        .collect::<Result<Vec<RecordBatch>, _>>()
        .expect("every batch reads");
    assert_eq!(
        batches
            .iter()
            .map(RecordBatch::num_rows)
            .collect::<Vec<_>>(),
        [2, 1]
    );
    assert!(
        batches
            .iter()
            .all(|batch| batch.schema() == quote_batch().schema())
    );
    assert_eq!(batches[1], quote_batch().slice(1, 1));
}

#[test]
fn a_held_chunked_leaf_column_is_the_one_child_of_a_record_per_chunk() {
    let price = Field::new("price", DataType::Int64, false);
    let chunked = ChunkedSerie::from_arrow_arrays(
        Some(&price),
        [
            prices(),
            Arc::new(Int64Array::from(vec![128_i64])) as ArrayRef,
        ],
        ArrowCastOptions::new(),
    )
    .expect("two chunks");
    let series = SerieReader::from_chunked(chunked.clone()).expect("a leaf is one column");
    assert_eq!(series.field().name(), yggdryl::media::DEFAULT_ROOT_NAME);
    assert!(!series.field().is_nullable());
    assert_eq!(
        series.field().dtype().as_fields().map(<[Field]>::to_vec),
        Some(vec![price])
    );
    let yielded = series
        .collect::<Result<Vec<Serie>, _>>()
        .expect("every chunk");
    assert_eq!(yielded.len(), 2);
    for (record, chunk) in yielded.iter().zip(chunked.chunks()) {
        assert_eq!(record.len(), chunk.len());
        assert_eq!(record.child("price"), Some(chunk));
    }

    let rows = SerieReader::from_chunked(chunked)
        .expect("a leaf is one column")
        .into_arrow_reader()
        .map(|batch| batch.expect("a batch reads").num_rows())
        .collect::<Vec<_>>();
    assert_eq!(rows, [3, 1]);
}

#[test]
fn a_chunked_record_column_holding_an_absent_row_is_no_stream() {
    let root = quotes_root().with_nullable(true);
    let records = StructArray::new(
        vec![
            ArrowField::new("id", ArrowDataType::Int64, false),
            ArrowField::new("symbol", ArrowDataType::Utf8, false),
        ]
        .into(),
        vec![
            Arc::new(Int64Array::from(vec![1, 2])) as ArrayRef,
            Arc::new(StringArray::from(vec!["AAPL", "MSFT"])),
        ],
        Some(NullBuffer::from(vec![true, false])),
    );
    let present: ArrayRef = Arc::new(StructArray::from(quote_batch()));
    let chunked = ChunkedSerie::from_arrow_arrays(
        Some(&root),
        [Arc::clone(&present), Arc::new(records) as ArrayRef],
        ArrowCastOptions::new(),
    )
    .expect("a nullable record column");
    let refusal = SerieReader::from_chunked(chunked).expect_err("a table states no row validity");
    assert!(refusal.to_string().contains("1 absent rows"), "{refusal}");

    // Every row present, a nullable root streams as the required record
    // its batches are.
    let present = ChunkedSerie::from_arrow_arrays(Some(&root), [present], ArrowCastOptions::new())
        .expect("one chunk");
    assert_eq!(present.field(), &root);
    let series = SerieReader::from_chunked(present).expect("every row present");
    assert_eq!(series.field(), &quotes_root());
    assert_eq!(series.count(), 1);
}

#[test]
fn an_empty_chunked_column_is_the_empty_stream_of_its_root() {
    let empty = ChunkedSerie::empty(quotes_root()).expect("a record field");
    let series = SerieReader::from_chunked(empty.clone()).expect("no chunk");
    assert_eq!(series.field(), &quotes_root());
    assert_eq!(series.count(), 0);
    let transport = SerieReader::from_chunked(empty)
        .expect("no chunk")
        .into_arrow_reader();
    assert_eq!(transport.schema(), quote_batch().schema());
    assert_eq!(transport.count(), 0);

    let price = Field::new("price", DataType::Int64, false);
    let series =
        SerieReader::from_chunked(ChunkedSerie::empty(price.clone()).expect("a leaf field"))
            .expect("no chunk");
    assert_eq!(series.field().name(), yggdryl::media::DEFAULT_ROOT_NAME);
    assert_eq!(
        series.field().dtype().as_fields().map(<[Field]>::to_vec),
        Some(vec![price])
    );
    assert_eq!(series.count(), 0);
}

#[test]
fn a_duration32_column_reads_back_as_duration32_values() {
    // Arrow lays out one duration width, so the column's storage is 64-bit
    // and the reading resolved at the landing is what keeps the width.
    let field = Field::new(
        "elapsed",
        DataType::duration32(yggdryl::TimeUnit::Second).unwrap(),
        true,
    );
    let array: ArrayRef = Arc::new(arrow_array::DurationSecondArray::from(vec![Some(90), None]));
    let landed = Serie::from_arrow_array(Some(&field), array, strict()).unwrap();
    let laid = Serie::from_scalars(
        field,
        [
            Scalar::duration32(90, yggdryl::TimeUnit::Second).unwrap(),
            Scalar::Null,
        ],
    )
    .unwrap();
    for column in [&landed, &laid] {
        let first = column.scalar(0).unwrap();
        assert_eq!(first.kind(), "duration32");
        assert_eq!(
            first,
            Scalar::duration32(90, yggdryl::TimeUnit::Second).unwrap()
        );
        assert_eq!(column.scalar(1).unwrap(), Scalar::Null);
    }
}

#[test]
fn the_held_root_is_one_rule_every_held_door_names_its_field_by() {
    // A record keeps its own name, required; any other field is the one
    // child of the `row` record, named as it is.
    let root = quotes_root();
    let optional = root.clone().with_nullable(true);
    assert_eq!(SerieReader::root_of(&root).unwrap(), root);
    assert_eq!(SerieReader::root_of(&optional).unwrap(), root);
    let price = Field::new("price", DataType::Int64, false);
    let wrapped = SerieReader::root_of(&price).unwrap();
    assert_eq!(wrapped.name(), "row");
    assert!(!wrapped.is_nullable());
    assert_eq!(wrapped.fields().len(), 1);
    assert_eq!(wrapped.get_field_at(0), Some(&price));

    let column = Serie::from_scalars(price.clone(), [Scalar::from(1_i64)]).unwrap();
    assert_eq!(
        SerieReader::from_serie(column.clone()).unwrap().field(),
        &wrapped
    );
    let chunked = ChunkedSerie::from_serie(column).unwrap();
    assert_eq!(
        SerieReader::from_chunked(chunked).unwrap().field(),
        &wrapped
    );
}

/// A quotes root whose `id` is `dtype`, beside the `symbol` text column.
fn quotes_root_with_id(dtype: DataType) -> Field {
    Field::new(
        "row",
        DataType::from(
            StructType::from_fields([
                Field::new("id", dtype, false),
                Field::new("symbol", DataType::utf8(), false),
            ])
            .expect("two named children"),
        ),
        false,
    )
}

#[test]
fn a_held_reader_cast_under_its_own_root_is_itself_and_under_another_casts_each_record() {
    let batch = quote_batch();
    let root = Field::from_arrow_schema("row", batch.schema().as_ref()).expect("the root");
    let held =
        Serie::from_arrow_batch(Some(&root), &batch, ArrowCastOptions::new()).expect("lands");
    // Its own root: the reader as it stands, its records untouched.
    let same = SerieReader::from_serie(held.clone())
        .expect("a held stream")
        .cast(&root, ArrowCastOptions::new())
        .expect("its own root");
    assert_eq!(same.field(), &root);
    let records: Vec<Serie> = same.map(Result::unwrap).collect();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].rows(), held.rows());
    // A wider root: every record cast once, by one plan.
    let wide = quotes_root_with_id(DataType::Float64);
    let widened = SerieReader::from_serie(held.clone())
        .expect("a held stream")
        .cast(&wide, ArrowCastOptions::new())
        .expect("int64 widens");
    assert_eq!(widened.field(), &wide);
    let records: Vec<Serie> = widened.map(Result::unwrap).collect();
    assert_eq!(
        records[0].children()[0].field().map(Field::dtype),
        Some(&DataType::Float64)
    );
    assert_eq!(
        records[0].scalar(1).expect("a row"),
        Scalar::from_sequence([Scalar::from(2.0_f64), Scalar::from("MSFT")])
    );
    // A root a record cannot fill is refused where the cast happens,
    // naming the column.
    let numeric_symbol = Field::new(
        "row",
        DataType::from(
            StructType::from_fields([
                Field::new("id", DataType::Int64, false),
                Field::new("symbol", DataType::Int64, false),
            ])
            .expect("two named children"),
        ),
        false,
    );
    let refused = SerieReader::from_serie(held)
        .expect("a held stream")
        .cast(&numeric_symbol, strict())
        .expect_err("text is not a number");
    assert!(refused.to_string().contains("symbol"), "{refused}");
}

#[test]
fn a_stream_cast_again_lands_what_its_first_plan_cast() {
    // A stream opened under an identity plan and cast again is one plan
    // from its own schema; one opened under a real cast keeps that cast and
    // lands its output under the second, so an int32 column widened to
    // int64 by the first plan reaches the second as int64 and leaves it as
    // float64 - in order, never skipping the middle.
    let wide = quotes_root_with_id(DataType::Float64);
    let batch = quote_batch();
    let root = Field::from_arrow_schema("row", batch.schema().as_ref()).expect("the root");
    let direct =
        SerieReader::from_arrow_reader(Some(&root), quote_stream(2), ArrowCastOptions::new())
            .expect("an identity plan")
            .cast(&wide, ArrowCastOptions::new())
            .expect("int64 widens");
    assert_eq!(direct.field(), &wide);
    let records: Vec<Serie> = direct.map(Result::unwrap).collect();
    assert_eq!(records.len(), 2);
    assert_eq!(
        records[1].scalar(0).expect("a row"),
        Scalar::from_sequence([Scalar::from(1.0_f64), Scalar::from("AAPL")])
    );

    let narrow = narrow_quote_batch();
    let middle = quotes_root_with_id(DataType::Int64);
    let composed = SerieReader::from_arrow_reader(
        Some(&middle),
        batch_reader(narrow.schema(), vec![narrow.clone(); 2]),
        ArrowCastOptions::new(),
    )
    .expect("int32 widens to int64")
    .cast(&wide, ArrowCastOptions::new())
    .expect("int64 widens to float64");
    assert_eq!(composed.field(), &wide);
    let records: Vec<Serie> = composed.map(Result::unwrap).collect();
    assert_eq!(records.len(), 2);
    assert_eq!(
        records[0].scalar(1).expect("a row"),
        Scalar::from_sequence([Scalar::from(2.0_f64), Scalar::from("MSFT")])
    );
    // The transport face applies both, in the same order.
    let transport = SerieReader::from_arrow_reader(
        Some(&middle),
        batch_reader(narrow.schema(), vec![narrow; 1]),
        ArrowCastOptions::new(),
    )
    .expect("int32 widens to int64")
    .cast(&wide, ArrowCastOptions::new())
    .expect("int64 widens to float64")
    .into_arrow_reader();
    assert_eq!(
        transport.schema().field(0).data_type(),
        &ArrowDataType::Float64
    );
    let batches: Vec<RecordBatch> = transport.map(Result::unwrap).collect();
    assert_eq!(batches[0].column(0).data_type(), &ArrowDataType::Float64);
}

#[test]
fn a_batch_that_lays_out_as_its_root_lands_as_it_stands_and_a_narrower_one_takes_the_plan() {
    let batch = quote_batch();
    let root = Field::from_arrow_schema("row", batch.schema().as_ref()).expect("the root");
    // Every leaf's layout is its contract, so the batch's own columns are
    // the record's children: the same buffers, no plan compiled.
    let exact = Serie::from_arrow_batch(Some(&root), &batch, strict()).expect("an exact batch");
    let landed = exact.children()[0].into_arrow_array().expect("a column");
    assert_eq!(
        landed.to_data().buffers()[0].as_ptr(),
        batch.column(0).to_data().buffers()[0].as_ptr(),
        "an exact batch was copied"
    );
    // A batch of another layout is cast into the root, so its buffers are
    // the cast's own and its values are the root's.
    let narrow = narrow_quote_batch();
    let cast = Serie::from_arrow_batch(Some(&root), &narrow, ArrowCastOptions::new())
        .expect("int32 widens");
    assert_eq!(
        cast.children()[0].field().map(Field::dtype),
        Some(&DataType::Int64)
    );
    assert_ne!(
        cast.children()[0]
            .into_arrow_array()
            .expect("a column")
            .to_data()
            .buffers()[0]
            .as_ptr(),
        narrow.column(0).to_data().buffers()[0].as_ptr()
    );
    // A layout the root does not state - a nullable column under a required
    // child - takes the plan's answer: the absent row is refused by path.
    let schema = Arc::new(Schema::new(vec![
        ArrowField::new("id", ArrowDataType::Int64, true),
        ArrowField::new("symbol", ArrowDataType::Utf8, false),
    ]));
    let absent = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            Arc::new(Int64Array::from(vec![Some(1_i64), None])) as ArrayRef,
            Arc::new(StringArray::from(vec!["AAPL", "MSFT"])) as ArrayRef,
        ],
    )
    .expect("a batch");
    for options in [strict(), ArrowCastOptions::new()] {
        let refused =
            Serie::from_arrow_batch(Some(&root), &absent, options).expect_err("an absent id");
        assert!(refused.to_string().contains("id"), "{refused}");
    }
}

/// One day in nanoseconds: what `days(ts)` cuts a quote's instant by.
const DAY_NS: i64 = 86_400_000_000_000;

/// `quote{venue: utf8, price: int64, ts: timestamp(ns, UTC)}`: the root
/// every windowed stream below yields.
fn window_root() -> Field {
    DataType::from(
        StructType::from_fields([
            DataType::utf8().required_field("venue"),
            DataType::Int64.required_field("price"),
            DataType::DateTime64 {
                unit: TimeUnit::Nanosecond,
                timezone: Timezone::UTC,
            }
            .required_field("ts"),
        ])
        .expect("three children"),
    )
    .required_field("quote")
}

/// One quote: its venue, its price, and the day it was quoted on - the
/// row [`window_root`] holds.
fn window_quote(venue: &str, price: i64, day: i64) -> Scalar {
    window_root()
        .scalar(Scalar::from_sequence([
            Scalar::from(venue),
            Scalar::from(price),
            Scalar::from(day * DAY_NS),
        ]))
        .expect("a quote row")
}

/// One Arrow batch of quotes under [`window_root`].
fn window_batch(rows: &[(&str, i64, i64)]) -> RecordBatch {
    Serie::from_scalars(
        window_root(),
        rows.iter()
            .map(|(venue, price, day)| window_quote(venue, *price, *day)),
    )
    .expect("quote rows")
    .into_arrow_batch()
    .expect("a batch")
}

/// A stream of batches that counts what it hands over and flags its drop.
struct Probed {
    batches: std::vec::IntoIter<Result<RecordBatch, ArrowError>>,
    schema: SchemaRef,
    pulls: Arc<AtomicUsize>,
    dropped: Arc<AtomicBool>,
}

impl Iterator for Probed {
    type Item = Result<RecordBatch, ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        let next = self.batches.next();
        if next.is_some() {
            self.pulls.fetch_add(1, Ordering::SeqCst);
        }
        next
    }
}

impl RecordBatchReader for Probed {
    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }
}

impl Drop for Probed {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

/// What a [`Probed`] stream reports: the batches pulled from it and
/// whether it was dropped.
#[derive(Default)]
struct Probe {
    pulls: Arc<AtomicUsize>,
    dropped: Arc<AtomicBool>,
}

impl Probe {
    fn pulls(&self) -> usize {
        self.pulls.load(Ordering::SeqCst)
    }

    fn dropped(&self) -> bool {
        self.dropped.load(Ordering::SeqCst)
    }

    /// The quotes of `batches`, one Arrow batch each, read under
    /// [`window_root`] through this probe.
    fn reader(&self, batches: &[&[(&str, i64, i64)]]) -> SerieReader {
        self.stream(batches.iter().map(|rows| Ok(window_batch(rows))).collect())
    }

    /// `batches` - failures included - read under [`window_root`].
    fn stream(&self, batches: Vec<Result<RecordBatch, ArrowError>>) -> SerieReader {
        let stream = Probed {
            batches: batches.into_iter(),
            schema: window_batch(&[]).schema(),
            pulls: Arc::clone(&self.pulls),
            dropped: Arc::clone(&self.dropped),
        };
        SerieReader::from_arrow_reader(Some(&window_root()), Box::new(stream), strict())
            .expect("an identity plan")
    }
}

/// A window's static values, as the cells of their row.
fn static_cells(window: &SerieReader) -> Vec<Scalar> {
    window
        .static_values()
        .expect("a window states its static values")
        .value()
        .sequence_rows()
        .expect("a record row")
        .into_owned()
}

/// Every row a window serves, each piece read as it comes.
fn window_rows(window: SerieReader) -> Vec<Scalar> {
    window
        .flat_map(|piece| piece.expect("a piece").rows().into_owned())
        .collect()
}

/// A one-cell key, as a window's key cells read.
fn venue_key(venue: &str) -> Scalar {
    Scalar::from_sequence([Scalar::from(venue)])
}

/// A refusal's path and reason.
fn refusal(error: yggdryl::arrow::Error) -> (String, String) {
    match error {
        yggdryl::arrow::Error::Core(yggdryl::Error::InvalidRecord { path, reason }) => {
            (path.to_string(), reason.to_string())
        }
        other => panic!("expected an invalid record, got {other}"),
    }
}

/// A path a static value is read at.
fn path(text: &str) -> FieldPath {
    text.parse().expect("a path")
}

#[test]
fn window_by_on_a_reader_refuses_before_any_pull() {
    let probe = Probe::default();
    let rows: &[&[(&str, i64, i64)]] = &[&[("XNAS", 1, 0)]];
    let root = window_root();
    for sorted in [false, true] {
        // The text's own parse error, before anything else.
        let parsed = "venue,".parse::<Selector>().unwrap_err().to_string();
        let refused = probe.reader(rows).window_by("venue,", sorted).unwrap_err();
        assert_eq!(refused.to_string(), parsed);
        // A key stating no column, named by the reader's root.
        for refused in [
            probe.reader(rows).window_by("*", sorted),
            probe
                .reader(rows)
                .window_by(Selector::new(Vec::new()), sorted),
        ] {
            assert_eq!(
                refusal(refused.unwrap_err()),
                (
                    "quote".to_owned(),
                    "expected at least one column to window by, got an empty match key".to_owned()
                )
            );
        }
        let refused = probe
            .reader(rows)
            .window_by("unnest(items)", sorted)
            .unwrap_err()
            .to_string();
        assert!(refused.contains("in a key"), "{refused}");
        // The binder's own refusals, word for word.
        for text in ["tier", "minutes(ts, 0)"] {
            let selector = text.parse::<Selector>().expect("a selector");
            let refused = probe
                .reader(rows)
                .window_by(&selector, sorted)
                .unwrap_err()
                .to_string();
            assert_eq!(refused, selector.bind(&root).unwrap_err().to_string());
        }
        // A key cell named as a reserved static value, or folding onto one
        // the reader states, is refused naming both.
        let (path, reason) = refusal(
            probe
                .reader(rows)
                .window_by("venue as RowNum", sorted)
                .unwrap_err(),
        );
        assert_eq!(path, "quote");
        assert!(
            reason.contains("\"RowNum\"")
                && reason.contains("\"rownum\"")
                && reason.contains("alias"),
            "{reason}"
        );
        let part = DataType::from(
            StructType::from_fields([DataType::utf8().required_field("Venue")]).expect("one child"),
        )
        .required_field("part");
        let stated = || {
            probe.reader(rows).with_static_values(
                FieldRecord::new(&part, Scalar::from_sequence([Scalar::from("all")]))
                    .expect("a row"),
            )
        };
        let (_, reason) = refusal(stated().window_by("venue", sorted).unwrap_err());
        assert!(
            reason.contains("\"venue\"") && reason.contains("\"Venue\""),
            "{reason}"
        );
        assert!(stated().window_by("venue as desk", sorted).is_ok());
        // A reserved static value at another datatype.
        let misdeclared = DataType::from(
            StructType::from_fields([DataType::utf8().required_field("rownum")])
                .expect("one child"),
        )
        .required_field("part");
        let (path, reason) = refusal(
            probe
                .reader(rows)
                .with_static_values(
                    FieldRecord::new(&misdeclared, Scalar::from_sequence([Scalar::from("7")]))
                        .expect("a row"),
                )
                .window_by("venue", sorted)
                .unwrap_err(),
        );
        assert_eq!(path, "quote");
        assert!(
            reason.contains("\"rownum\"") && reason.contains("uint64") && reason.contains("utf8"),
            "{reason}"
        );
    }
    assert_eq!(probe.pulls(), 0, "no refusal pulled a batch");

    // A refused key spends nothing but the reader; an accepted one names
    // what every window yields and states before the first pull.
    let windows = probe
        .reader(rows)
        .window_by("venue, days(ts) as day", true)
        .expect("a key");
    assert_eq!(windows.field(), &root);
    let names: Vec<&str> = windows
        .static_field()
        .fields()
        .iter()
        .map(Field::name)
        .collect();
    assert_eq!(names, ["venue", "day", "windownum", "rownum"]);
    assert_eq!(windows.static_field().name(), "quote");
    assert!(!windows.static_field().is_nullable());
    assert_eq!(probe.pulls(), 0);
}

#[test]
fn a_reader_windows_lazily_holding_one_batch() {
    fn assert_send_sync<T: Send + Sync>() {}
    fn assert_send<T: Send>() {}
    assert_send_sync::<SerieReaderWindows>();
    assert_send::<SerieReader>();

    let probe = Probe::default();
    let batches: &[&[(&str, i64, i64)]] = &[
        &[("XNAS", 1, 0), ("XNAS", 2, 0)],
        &[("XNAS", 3, 0), ("XNYS", 4, 0)],
        &[],
        &[("XNYS", 5, 0), ("XLON", 6, 0)],
    ];
    let first = window_batch(batches[0]);
    let mut windows = probe
        .stream(vec![
            Ok(first.clone()),
            Ok(window_batch(batches[1])),
            Ok(window_batch(batches[2])),
            Ok(window_batch(batches[3])),
        ])
        .window_by("venue", false)
        .expect("a key");
    assert_eq!(
        probe.pulls(),
        0,
        "nothing is pulled before the first window"
    );
    assert!(format!("{windows:?}").contains("SerieReaderWindows"));

    // XNAS spans the first batch whole and the second's first row.
    let mut xnas = windows.next().expect("a window").expect("XNAS");
    assert_eq!(probe.pulls(), 1);
    assert_eq!(xnas.field(), &window_root());
    assert!(format!("{xnas:?}").contains("static_values"));
    let whole = xnas.next().expect("a piece").expect("the first batch");
    assert_eq!(whole.len(), 2);
    // A batch a window spans whole is the landed batch itself: its buffers
    // are the stream's own.
    assert_eq!(
        whole.children()[1]
            .into_arrow_array()
            .expect("prices")
            .to_data()
            .buffers()[0]
            .as_ptr(),
        first.column(1).to_data().buffers()[0].as_ptr(),
    );
    assert_eq!(probe.pulls(), 1, "a whole batch is served as it is held");
    let edge = xnas
        .next()
        .expect("a piece")
        .expect("the second batch's first row");
    assert_eq!(edge.rows().into_owned(), [window_quote("XNAS", 3, 0)]);
    assert_eq!(probe.pulls(), 2);
    assert!(xnas.next().is_none());
    assert!(xnas.next().is_none(), "fused");
    assert_eq!(probe.pulls(), 2, "the window ended inside the held batch");

    // XNYS opens in the held batch and reaches across the empty one.
    let xnys = windows.next().expect("a window").expect("XNYS");
    assert_eq!(probe.pulls(), 2, "the next window opens in the held batch");
    assert_eq!(
        static_cells(&xnys),
        [
            Scalar::from("XNYS"),
            Scalar::from(1_u64),
            Scalar::from(3_u64)
        ]
    );
    assert_eq!(
        window_rows(xnys),
        [window_quote("XNYS", 4, 0), window_quote("XNYS", 5, 0)]
    );
    assert_eq!(probe.pulls(), 4, "the empty batch was pulled and skipped");

    let xlon = windows.next().expect("a window").expect("XLON");
    assert_eq!(
        static_cells(&xlon),
        [
            Scalar::from("XLON"),
            Scalar::from(2_u64),
            Scalar::from(5_u64)
        ]
    );
    assert_eq!(window_rows(xlon), [window_quote("XLON", 6, 0)]);
    assert!(windows.next().is_none());
    assert!(windows.next().is_none(), "fused");
    assert!(probe.dropped(), "the stream is dropped at its end");
}

#[test]
fn a_window_states_its_key_windownum_and_rownum() {
    // The flat static record: the key cells, then the window's place and
    // its first row's number.
    let probe = Probe::default();
    let mut windows = probe
        .reader(&[&[("XNAS", 1, 0), ("XNYS", 2, 0)]])
        .window_by("venue", false)
        .expect("a key");
    let expected = DataType::from(
        StructType::from_fields([
            DataType::utf8().required_field("venue"),
            DataType::UInt64.required_field("windownum"),
            DataType::UInt64.required_field("rownum"),
        ])
        .expect("three children"),
    )
    .required_field("quote");
    assert_eq!(windows.static_field(), &expected);
    let xnas = windows.next().expect("a window").expect("XNAS");
    let statics = xnas.static_values().expect("stated");
    assert_eq!(statics.field(), &expected);
    assert_eq!(
        statics.value(),
        &Scalar::from_sequence([
            Scalar::from("XNAS"),
            Scalar::from(0_u64),
            Scalar::from(0_u64)
        ])
    );

    // A held serie windowed as a stream states exactly what its held
    // windows answer: the key cells, the place and the offset.
    let mixed = [
        ("XNAS", 1, 0),
        ("XNAS", 2, 0),
        ("XNYS", 3, 0),
        ("XNAS", 4, 0),
    ];
    let ordered = [
        ("XLON", 1, 0),
        ("XNAS", 2, 0),
        ("XNAS", 3, 0),
        ("XNYS", 4, 0),
    ];
    for (sorted, rows) in [(false, mixed), (true, ordered)] {
        let rows = Serie::from_scalars(
            window_root(),
            rows.iter()
                .map(|(venue, price, day)| window_quote(venue, *price, *day)),
        )
        .expect("quotes");
        let held: Vec<Vec<Scalar>> = rows
            .window_by("venue", sorted)
            .expect("held windows")
            .iter()
            .enumerate()
            .map(|(place, (key, window))| {
                let mut cells = key.sequence_rows().expect("a key run").into_owned();
                cells.extend([
                    Scalar::from(place as u64),
                    Scalar::from(window.offset() as u64),
                ]);
                cells
            })
            .collect();
        let streamed: Vec<Vec<Scalar>> = SerieReader::from_serie(rows)
            .expect("a held stream")
            .window_by("venue", sorted)
            .expect("stream windows")
            .map(|window| static_cells(&window.expect("a window")))
            .collect();
        assert_eq!(streamed, held, "sorted {sorted}");
    }

    // A window of a window keeps the outer key cells, states its own place,
    // and numbers its first row in the stream the outer windows were cut
    // from.
    let probe = Probe::default();
    let days = probe
        .reader(&[
            &[("XNAS", 1, 0), ("XNYS", 2, 0)],
            &[("XNYS", 3, 0), ("XNAS", 4, 1)],
            &[("XNAS", 5, 1)],
        ])
        .window_by("days(ts) as day", false)
        .expect("a key");
    let mut seen = Vec::new();
    for day in days {
        let day = day.expect("a day");
        let date = day
            .get_static_value(&path("day"))
            .expect("the outer key cell")
            .value()
            .clone();
        let venues = day.window_by("venue", false).expect("an inner key");
        let names: Vec<&str> = venues
            .static_field()
            .fields()
            .iter()
            .map(Field::name)
            .collect();
        assert_eq!(names, ["day", "venue", "windownum", "rownum"]);
        for venue in venues {
            let venue = venue.expect("a venue");
            let cells = static_cells(&venue);
            assert_eq!(cells[0], date);
            let rows = window_rows(venue);
            seen.push((
                cells[1].clone(),
                cells[2].clone(),
                cells[3].clone(),
                rows.len(),
            ));
        }
    }
    assert_eq!(
        seen,
        [
            (
                Scalar::from("XNAS"),
                Scalar::from(0_u64),
                Scalar::from(0_u64),
                1
            ),
            (
                Scalar::from("XNYS"),
                Scalar::from(1_u64),
                Scalar::from(1_u64),
                2
            ),
            (
                Scalar::from("XNAS"),
                Scalar::from(0_u64),
                Scalar::from(3_u64),
                2
            ),
        ]
    );
}

#[test]
fn a_sorted_reader_refuses_a_key_going_backwards_naming_batch_and_row() {
    let text = |venue: &str| venue_key(venue).as_serie().expect("a run").to_string();
    let expected = |batch: usize, row: usize, key: &str, previous: &str| {
        format!(
            "window by expects keys in order, ascending with absent keys last: batch {batch} row \
             {row} keys {} after {}; window it unsorted, or hold it \
             (ChunkedSerie::from_serie_reader) and window it sorted",
            text(key),
            text(previous)
        )
    };

    // A descent inside a batch: the windows before it are delivered whole,
    // the last one ending where the key went back.
    let probe = Probe::default();
    let mut windows = probe
        .reader(&[&[("XLON", 1, 0), ("XNYS", 2, 0), ("XNAS", 3, 0)]])
        .window_by("venue", true)
        .expect("a key");
    let xlon = windows.next().expect("a window").expect("XLON");
    assert_eq!(window_rows(xlon), [window_quote("XLON", 1, 0)]);
    let xnys = windows.next().expect("a window").expect("XNYS");
    assert_eq!(window_rows(xnys), [window_quote("XNYS", 2, 0)]);
    let refused = windows.next().expect("the refusal").unwrap_err();
    assert_eq!(
        refusal(refused),
        ("$[2]".to_owned(), expected(0, 2, "XNAS", "XNYS"))
    );
    assert!(windows.next().is_none(), "fused after the refusal");
    assert!(probe.dropped(), "the stream is dropped at the refusal");

    // A descent at an edge, an empty batch between: empty batches count.
    let probe = Probe::default();
    let mut windows = probe
        .reader(&[
            &[("XLON", 1, 0), ("XNYS", 2, 0)],
            &[],
            &[("XNAS", 3, 0), ("XNAS", 4, 0)],
        ])
        .window_by("venue", true)
        .expect("a key");
    let xlon = windows.next().expect("a window").expect("XLON");
    assert_eq!(window_rows(xlon).len(), 1);
    let xnys = windows.next().expect("a window").expect("XNYS");
    // XNYS ends at the edge: its reader pulls the next batch, which opens
    // another key, and ends.
    assert_eq!(window_rows(xnys).len(), 1);
    assert!(!probe.dropped());
    let refused = windows.next().expect("the refusal").unwrap_err();
    assert_eq!(
        refusal(refused),
        ("$[2]".to_owned(), expected(2, 0, "XNAS", "XNYS"))
    );
    assert!(windows.next().is_none());
    assert!(probe.dropped());

    // Unsorted, the same stream windows every key where it arrives.
    let probe = Probe::default();
    let venues: Vec<Scalar> = probe
        .reader(&[&[("XLON", 1, 0), ("XNYS", 2, 0)], &[], &[("XNAS", 3, 0)]])
        .window_by("venue", false)
        .expect("a key")
        .map(|window| static_cells(&window.expect("a window"))[0].clone())
        .collect();
    assert_eq!(
        venues,
        [
            Scalar::from("XLON"),
            Scalar::from("XNYS"),
            Scalar::from("XNAS")
        ]
    );
}

#[test]
fn a_window_passed_by_the_walk_refuses_to_be_read() {
    let batches: &[&[(&str, i64, i64)]] = &[
        &[("XNAS", 1, 0), ("XNAS", 2, 0)],
        &[("XNYS", 3, 0), ("XLON", 4, 0)],
    ];
    let passed = "window 0 was passed by its walk with rows unread; read each window before \
                  taking the next";

    // Taken past with rows unread: the window refuses once, then ends.
    let probe = Probe::default();
    let mut windows = probe
        .reader(batches)
        .window_by("venue", false)
        .expect("a key");
    let mut xnas = windows.next().expect("a window").expect("XNAS");
    let xnys = windows.next().expect("a window").expect("XNYS");
    assert_eq!(
        refusal(xnas.next().expect("the refusal").unwrap_err()),
        ("quote".to_owned(), passed.to_owned())
    );
    assert!(xnas.next().is_none(), "fused after the refusal");
    assert_eq!(window_rows(xnys), [window_quote("XNYS", 3, 0)]);

    // Every row served, though the walk moved on before it said so: it ends.
    let probe = Probe::default();
    let mut windows = probe
        .reader(batches)
        .window_by("venue", false)
        .expect("a key");
    let mut xnas = windows.next().expect("a window").expect("XNAS");
    assert_eq!(
        xnas.next()
            .expect("a piece")
            .expect("the first batch")
            .len(),
        2
    );
    let xnys = windows.next().expect("a window").expect("XNYS");
    assert!(xnas.next().is_none());
    assert_eq!(window_rows(xnys).len(), 1);

    // Dropped unread: it costs only the pull of its rows.
    let probe = Probe::default();
    let mut windows = probe
        .reader(batches)
        .window_by("venue", false)
        .expect("a key");
    drop(windows.next().expect("a window").expect("XNAS"));
    let xnys = windows.next().expect("a window").expect("XNYS");
    assert_eq!(window_rows(xnys), [window_quote("XNYS", 3, 0)]);

    // Collected before any is read, every window was passed - loud, never
    // a silent loss.
    let probe = Probe::default();
    let collected: Vec<SerieReader> = probe
        .reader(batches)
        .window_by("venue", false)
        .expect("a key")
        .collect::<Result<_, _>>()
        .expect("three windows");
    assert_eq!(collected.len(), 3);
    for (place, mut window) in collected.into_iter().enumerate() {
        let (_, reason) = refusal(window.next().expect("the refusal").unwrap_err());
        assert!(
            reason.starts_with(&format!("window {place} was passed")),
            "{reason}"
        );
        assert!(window.next().is_none());
    }

    // A window outliving its walk reads to its end.
    let probe = Probe::default();
    let mut windows = probe
        .reader(&[&[("XNAS", 1, 0)], &[("XNAS", 2, 0)], &[("XNYS", 3, 0)]])
        .window_by("venue", false)
        .expect("a key");
    let xnas = windows.next().expect("a window").expect("XNAS");
    drop(windows);
    assert_eq!(
        window_rows(xnas),
        [window_quote("XNAS", 1, 0), window_quote("XNAS", 2, 0)]
    );
}

#[test]
fn a_reader_error_mid_window_is_the_pullers_item_and_fuses() {
    let failing = |probe: &Probe| {
        probe.stream(vec![
            Ok(window_batch(&[("XNAS", 1, 0), ("XNAS", 2, 0)])),
            Err(ArrowError::ComputeError("the wire was cut".to_owned())),
            Ok(window_batch(&[("XNYS", 3, 0)])),
        ])
    };

    // The window pulls the failure: it is the window's item, once.
    let probe = Probe::default();
    let mut windows = failing(&probe).window_by("venue", false).expect("a key");
    let mut xnas = windows.next().expect("a window").expect("XNAS");
    assert_eq!(
        xnas.next()
            .expect("a piece")
            .expect("the first batch")
            .len(),
        2
    );
    let failure = xnas.next().expect("the failure").unwrap_err().to_string();
    assert!(failure.contains("the wire was cut"), "{failure}");
    assert!(xnas.next().is_none(), "the window fused");
    assert!(windows.next().is_none(), "the walk fused");
    assert!(
        probe.dropped(),
        "the stream is dropped where the failure arrived"
    );
    assert_eq!(probe.pulls(), 2, "nothing is pulled past the failure");

    // The walk pulls the failure while it skips a window: the walk's item,
    // and the window it was skipping is never presented as complete.
    let probe = Probe::default();
    let mut windows = failing(&probe).window_by("venue", false).expect("a key");
    let mut xnas = windows.next().expect("a window").expect("XNAS");
    let failure = windows
        .next()
        .expect("the failure")
        .unwrap_err()
        .to_string();
    assert!(failure.contains("the wire was cut"), "{failure}");
    assert!(windows.next().is_none(), "the walk fused");
    let (_, reason) = refusal(xnas.next().expect("the refusal").unwrap_err());
    assert!(reason.starts_with("window 0 was passed"), "{reason}");
    assert!(xnas.next().is_none());
    assert!(probe.dropped());

    // A batch the stream's plan refuses is a failure like any other.
    let probe = Probe::default();
    let schema = Arc::new(Schema::new(vec![
        ArrowField::new("venue", ArrowDataType::Utf8, true),
        ArrowField::new("price", ArrowDataType::Int64, false),
        ArrowField::new(
            "ts",
            ArrowDataType::Timestamp(arrow_schema::TimeUnit::Nanosecond, Some("UTC".into())),
            false,
        ),
    ]));
    let absent = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            Arc::new(StringArray::from(vec![None::<&str>])) as ArrayRef,
            Arc::new(Int64Array::from(vec![1_i64])) as ArrayRef,
            Arc::new(arrow_array::TimestampNanosecondArray::from(vec![0_i64]).with_timezone("UTC"))
                as ArrayRef,
        ],
    )
    .expect("a batch");
    let stream = Probed {
        batches: vec![Ok(absent)].into_iter(),
        schema,
        pulls: Arc::clone(&probe.pulls),
        dropped: Arc::clone(&probe.dropped),
    };
    let mut windows =
        SerieReader::from_arrow_reader(Some(&window_root()), Box::new(stream), strict())
            .expect("a plan")
            .window_by("venue", false)
            .expect("a key");
    let failure = windows
        .next()
        .expect("the failure")
        .unwrap_err()
        .to_string();
    assert!(failure.contains("venue"), "{failure}");
    assert!(windows.next().is_none());
    assert!(probe.dropped());
}

#[test]
fn a_foreign_nan_at_a_batch_edge_moves_no_window_boundary() {
    let root = DataType::from(
        StructType::from_fields([DataType::Float64.nullable_field("px")]).expect("one child"),
    )
    .required_field("tick");
    let schema = Arc::new(Schema::new(vec![ArrowField::new(
        "px",
        ArrowDataType::Float64,
        true,
    )]));
    let batch = |values: Vec<f64>| {
        RecordBatch::try_new(
            Arc::clone(&schema),
            vec![Arc::new(arrow_array::Float64Array::from(values)) as ArrayRef],
        )
        .expect("a batch")
    };
    let foreign = f64::from_bits(0x7ff8_0000_0000_0001);
    for sorted in [false, true] {
        let reader = batch_reader(
            Arc::clone(&schema),
            [batch(vec![1.0, f64::NAN]), batch(vec![foreign, f64::NAN])],
        );
        let windows: Vec<(Vec<Scalar>, usize)> =
            SerieReader::from_arrow_reader(Some(&root), reader, strict())
                .expect("an identity plan")
                .window_by("px", sorted)
                .expect("a key")
                .map(|window| {
                    let window = window.expect("a window");
                    let cells = static_cells(&window);
                    (cells, window_rows(window).len())
                })
                .collect();
        assert_eq!(windows.len(), 2, "sorted {sorted}: {windows:?}");
        assert_eq!(windows[0].1, 1);
        assert_eq!(windows[1].1, 3, "every NaN is one key, across the edge");
        assert_eq!(
            windows[1].0[1..],
            [Scalar::from(1_u64), Scalar::from(1_u64)]
        );
    }
}

#[test]
fn static_values_are_carried_by_cast_and_dropped_at_the_transport_face() {
    let part = DataType::from(
        StructType::from_fields([
            DataType::utf8().required_field("desk"),
            DataType::from(
                StructType::from_fields([DataType::Int32.nullable_field("day")])
                    .expect("one child"),
            )
            .nullable_field("session"),
        ])
        .expect("two children"),
    )
    .required_field("part");
    let values = || {
        FieldRecord::new(
            &part,
            Scalar::from_sequence([
                Scalar::from("rates"),
                Scalar::from_sequence([Scalar::from(3_i32)]),
            ]),
        )
        .expect("a row")
    };
    let probe = Probe::default();
    let reader = probe
        .reader(&[&[("XNAS", 1, 0)]])
        .with_static_values(values());
    assert_eq!(reader.field(), &window_root(), "never a column");
    let statics = reader.static_values().expect("stated");
    assert_eq!(statics.field(), &part);
    assert_eq!(statics.value(), &values().into_scalar());
    let stated = statics.value().clone();

    // Exact names through record steps; anything else reaches nothing.
    assert_eq!(
        reader
            .get_static_value(&path("desk"))
            .expect("a cell")
            .value(),
        &Scalar::from("rates")
    );
    let day = reader
        .get_static_value(&path("session.day"))
        .expect("a nested cell");
    assert_eq!(day.name(), "day");
    assert_eq!(day.value(), &Scalar::from(3_i32));
    assert_eq!(
        reader
            .get_static_value(&FieldPath::root())
            .expect("the record")
            .value(),
        &stated
    );
    for missing in ["DESK", "session.DAY", "tier", "desk.day", "session[0]"] {
        assert!(
            reader.get_static_value(&path(missing)).is_none(),
            "{missing}"
        );
    }

    // A cast keeps them: they say where the rows come from.
    let wider = DataType::from(
        StructType::from_fields([
            DataType::utf8().required_field("venue"),
            DataType::Float64.required_field("price"),
            DataType::DateTime64 {
                unit: TimeUnit::Nanosecond,
                timezone: Timezone::UTC,
            }
            .required_field("ts"),
        ])
        .expect("three children"),
    )
    .required_field("quote");
    let mut cast = reader
        .cast(&wider, ArrowCastOptions::new())
        .expect("int64 widens");
    assert_eq!(cast.static_values().expect("kept").value(), &stated);
    cast.clear_static_values();
    assert!(cast.static_values().is_none());
    cast.set_static_values(values());
    assert!(cast.static_values().is_some());

    // A held table keeps rows only, and the transport face is a schema.
    let held = ChunkedSerie::from_serie_reader(cast).expect("a held table");
    assert_eq!(held.len(), 1);
    let transport = SerieReader::from_arrow_reader(
        None,
        probe
            .reader(&[&[("XNAS", 1, 0)]])
            .with_static_values(values())
            .into_arrow_reader(),
        ArrowCastOptions::new(),
    )
    .expect("an identity plan");
    assert!(transport.static_values().is_none());
}

#[test]
fn a_window_sub_reader_casts_and_crosses_as_batches_lazily() {
    let wider = DataType::from(
        StructType::from_fields([
            DataType::utf8().required_field("venue"),
            DataType::Float64.required_field("price"),
            DataType::DateTime64 {
                unit: TimeUnit::Nanosecond,
                timezone: Timezone::UTC,
            }
            .required_field("ts"),
        ])
        .expect("three children"),
    )
    .required_field("quote");
    let probe = Probe::default();
    let mut windows = probe
        .reader(&[
            &[("XNAS", 1, 0), ("XNAS", 2, 0)],
            &[("XNAS", 3, 0), ("XNYS", 4, 0)],
        ])
        .window_by("venue", false)
        .expect("a key");

    // Cast: each piece is cast as it is served, the static values kept.
    let xnas = windows.next().expect("a window").expect("XNAS");
    let stated = static_cells(&xnas);
    let mut cast = xnas
        .cast(&wider, ArrowCastOptions::new())
        .expect("int64 widens");
    assert_eq!(cast.field(), &wider);
    assert_eq!(static_cells(&cast), stated);
    assert_eq!(probe.pulls(), 1, "a cast pulls nothing");
    let piece = cast.next().expect("a piece").expect("cast");
    assert_eq!(piece.field(), Some(&wider));
    assert_eq!(
        piece.rows()[1],
        wider
            .scalar(Scalar::from_sequence([
                Scalar::from("XNAS"),
                Scalar::from(2.0_f64),
                Scalar::from(0_i64),
            ]))
            .expect("a wider quote")
    );
    let rest: Vec<Serie> = cast.map(|piece| piece.expect("cast")).collect();
    assert_eq!(rest.len(), 1);
    assert_eq!(rest[0].field(), Some(&wider));

    // Transport: one batch per piece, pulled as the batches are.
    let xnys = windows.next().expect("a window").expect("XNYS");
    let pulls = probe.pulls();
    let mut batches = xnys.into_arrow_reader();
    assert_eq!(batches.schema(), window_batch(&[]).schema());
    assert_eq!(
        probe.pulls(),
        pulls,
        "the transport face pulls nothing up front"
    );
    let batch = batches.next().expect("a batch").expect("the piece");
    assert_eq!(batch.num_rows(), 1);
    assert!(batches.next().is_none());
    assert!(windows.next().is_none());
}

#[test]
fn a_pull_that_panics_poisons_the_walk_into_one_internal_error() {
    let first = window_batch(&[("XNAS", 1, 0)]);
    let batches = [Some(first), None].into_iter().map(|batch| match batch {
        Some(batch) => Ok(batch),
        None => panic!("the stream panicked"),
    });
    let reader: BatchReader = Box::new(RecordBatchIterator::new(
        batches,
        window_batch(&[]).schema(),
    ));
    let mut windows = SerieReader::from_arrow_reader(Some(&window_root()), reader, strict())
        .expect("an identity plan")
        .window_by("venue", false)
        .expect("a key");
    let mut xnas = windows.next().expect("a window").expect("XNAS");
    assert_eq!(xnas.next().expect("a piece").expect("the batch").len(), 1);
    // The window's next pull panics while it holds the walk.
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| xnas.next()));
    assert!(panicked.is_err());
    let failure = windows
        .next()
        .expect("the failure")
        .unwrap_err()
        .to_string();
    assert!(
        failure.contains("SerieReaderWindows: a pull panicked while holding its walk"),
        "{failure}"
    );
    assert!(windows.next().is_none(), "fused");
    // The window open when it panicked is never presented as complete.
    let (_, reason) = refusal(xnas.next().expect("the refusal").unwrap_err());
    assert!(reason.starts_with("window 0 was passed"), "{reason}");
    assert!(xnas.next().is_none());
}
