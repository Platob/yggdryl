//! `rust/src/serie/enums.rs`: the dictionary leaf - a key column over a
//! values column, read through the key and written by interning.

use std::sync::Arc;

use arrow_array::types::Int8Type;
use arrow_array::{Array, ArrayRef, DictionaryArray, Int8Array, StringArray};
use yggdryl::{
    ArrowCastOptions, DataType, DictionarySerie, Field, Nullability, Scalar, Serie, SerieValue,
    StructType,
};

/// The options a refusal is pinned under: a present value is never nulled
/// and an absent one never repaired.
fn strict() -> ArrowCastOptions {
    ArrowCastOptions::new()
        .with_safe(false)
        .with_nullability(Nullability::Strict)
}

/// An int8-keyed dictionary of symbols.
fn symbols_field(nullable: bool) -> Field {
    Field::new(
        "symbol",
        DataType::dictionary(DataType::Int8, DataType::utf8()).expect("an int8 key"),
        nullable,
    )
}

/// Four symbol rows, one with no key, over a two-word vocabulary.
fn symbols_array() -> ArrayRef {
    Arc::new(
        DictionaryArray::<Int8Type>::try_new(
            Int8Array::from(vec![Some(0), Some(1), None, Some(0)]),
            Arc::new(StringArray::from(vec!["AAPL", "MSFT"])),
        )
        .expect("keys within the vocabulary"),
    )
}

/// [`symbols_array`] as a nullable column.
fn symbols() -> Serie {
    Serie::from_arrow_array(
        Some(&symbols_field(true)),
        symbols_array(),
        ArrowCastOptions::new(),
    )
    .expect("a dictionary column")
}

/// Two rows whose keys are present, one of them pointing at an absent value.
fn absent_value_array() -> ArrayRef {
    Arc::new(
        DictionaryArray::<Int8Type>::try_new(
            Int8Array::from(vec![Some(0), Some(1)]),
            Arc::new(StringArray::from(vec![Some("AAPL"), None])),
        )
        .expect("keys within the vocabulary"),
    )
}

#[test]
fn a_required_dictionary_column_with_a_logically_null_row_is_refused_at_the_door_naming_the_column()
{
    let refusal = Serie::from_arrow_array(Some(&symbols_field(false)), symbols_array(), strict())
        .expect_err("a required column admits no absent key");
    assert!(
        refusal.to_string().contains("symbol"),
        "the refusal names the column: {refusal}"
    );

    // A key pointing at an absent value is as absent as a missing key.
    let refusal =
        Serie::from_arrow_array(Some(&symbols_field(false)), absent_value_array(), strict())
            .expect_err("a required column admits no key at an absent value");
    assert!(refusal.to_string().contains("symbol"));

    let admitted = Serie::from_arrow_array(
        Some(&symbols_field(true)),
        absent_value_array(),
        ArrowCastOptions::new(),
    )
    .expect("a nullable column admits it");
    assert_eq!(admitted.null_count(), 1);
    assert!(admitted.is_null(1).unwrap());
    assert_eq!(admitted.scalar(1).unwrap(), Scalar::Null);
    assert_eq!(admitted.scalar(0).unwrap(), Scalar::from("AAPL"));
}

#[test]
fn a_row_past_the_end_and_a_value_the_field_refuses_are_refused_and_nothing_moves() {
    let mut column = symbols();

    let refusal = column.scalar(4).expect_err("row 4 is past the end");
    let text = refusal.to_string();
    assert!(text.contains("symbol"), "names the column: {text}");
    assert!(text.contains('4'), "names the row: {text}");
    assert!(column.is_null(4).is_err());
    assert!(column.slice(3, 2).is_err());
    assert!(column.set(4, Scalar::from("GOOG")).is_err());

    // A text column reads a number as text; a sequence is no text at all.
    assert!(
        column
            .push(Scalar::from_sequence([Scalar::from(1_i64)]))
            .is_err()
    );
    assert!(
        column
            .set(0, Scalar::from_sequence([Scalar::from(true)]))
            .is_err()
    );
    assert_eq!(column.len(), 4);
    assert_eq!(column.items().map(Serie::len), Some(2));

    let mut required = Serie::from_scalars(
        symbols_field(false),
        [Scalar::from("AAPL"), Scalar::from("MSFT")],
    )
    .expect("two present rows");
    let refusal = required
        .push(Scalar::Null)
        .expect_err("a required column admits no absent row");
    assert!(refusal.to_string().contains("symbol"));
    assert_eq!(required.len(), 2);
}

#[test]
fn the_buffers_cross_in_and_out_shared_and_a_row_reads_through_its_key() {
    let array = symbols_array();
    let column = Serie::from_arrow_array(
        Some(&symbols_field(true)),
        Arc::clone(&array),
        ArrowCastOptions::new(),
    )
    .expect("a dictionary column");
    let leaf = column.as_dictionary().expect("a dictionary column");

    assert_eq!(
        <DictionarySerie as SerieValue>::from_serie(&column),
        Some(leaf)
    );
    assert_eq!(leaf.keys().len(), 4);
    assert_eq!(leaf.values().len(), 2);
    assert_eq!(leaf.keys().as_int8().expect("int8 keys").value(1), Some(1));
    assert_eq!(leaf.keys().as_int8().expect("int8 keys").value(2), None);
    assert_eq!(column.items().map(Serie::len), Some(2));
    assert_eq!(SerieValue::len(leaf), 4);
    assert_eq!(leaf.null_count(), 1);
    assert!(leaf.is_null(2).unwrap());
    assert_eq!(leaf.scalar(0).unwrap(), Scalar::from("AAPL"));
    assert_eq!(leaf.scalar(1).unwrap(), Scalar::from("MSFT"));
    assert_eq!(leaf.scalar(2).unwrap(), Scalar::Null);
    assert_eq!(leaf.scalar(3).unwrap(), Scalar::from("AAPL"));

    let back = column.into_arrow_array().expect("a column");
    assert_eq!(back.data_type(), array.data_type());
    assert!(
        back.to_data().ptr_eq(&array.to_data()),
        "the keys and the vocabulary cross back out shared"
    );
    assert!(leaf.array().to_data().ptr_eq(&array.to_data()));

    // A window shares the vocabulary and slices the keys.
    let window = leaf.slice(1, 2).expect("rows 1..3");
    assert_eq!(SerieValue::len(&window), 2);
    assert_eq!(window.values().len(), 2);
    assert_eq!(window.scalar(0).unwrap(), Scalar::from("MSFT"));
    assert!(window.is_null(1).unwrap());

    // The rows are the value, whichever leaf holds them.
    assert_eq!(
        column,
        Serie::new(vec![
            Scalar::from("AAPL"),
            Scalar::from("MSFT"),
            Scalar::Null,
            Scalar::from("AAPL"),
        ])
    );
}

#[test]
fn a_write_interns_into_the_vocabulary_and_the_rows_round_trip() {
    let mut column = symbols();

    column.push(Scalar::from("AAPL")).expect("a held value");
    assert_eq!(column.items().map(Serie::len), Some(2));
    column.push(Scalar::from("GOOG")).expect("a new value");
    assert_eq!(column.items().map(Serie::len), Some(3));
    column.set(0, Scalar::from("MSFT")).expect("one slot");
    column
        .insert(1, Scalar::Null)
        .expect("an absent row in front");
    assert_eq!(column.remove(2).unwrap(), Scalar::from("MSFT"));
    column.truncate(4).expect("four rows kept");
    assert_eq!(column.pop().unwrap(), Some(Scalar::from("AAPL")));

    let rows = vec![Scalar::from("MSFT"), Scalar::Null, Scalar::Null];
    assert_eq!(column, Serie::new(rows.clone()));
    assert_eq!(column.null_count(), 2);
    assert_eq!(column.items().map(Serie::len), Some(3));
    assert_eq!(
        column,
        Serie::from_scalars(symbols_field(true), rows).expect("three rows")
    );

    // Appending another column interns its vocabulary, no row read twice.
    let other = Serie::from_scalars(
        symbols_field(true),
        [
            Scalar::from("GOOG"),
            Scalar::from("AAPL"),
            Scalar::from("IBM"),
        ],
    )
    .expect("three rows");
    column.extend_from_serie(&other).expect("agreeing fields");
    assert_eq!(column.len(), 6);
    assert_eq!(column.items().map(Serie::len), Some(4));
    assert_eq!(column.scalar(5).unwrap(), Scalar::from("IBM"));
    assert_eq!(column.scalar(3).unwrap(), Scalar::from("GOOG"));

    // A column built by pushes reads as the column laid out at once.
    let laid_out = symbols();
    let mut pushed = Serie::empty(symbols_field(true)).expect("an empty column");
    for row in laid_out.rows().iter().cloned() {
        pushed.push(row).expect("a row the field accepts");
    }
    assert_eq!(pushed, laid_out);
    assert_eq!(pushed.null_count(), 1);
    assert_eq!(pushed.items().map(Serie::len), Some(2));
    assert_eq!(pushed.into_arrow_array().unwrap().len(), 4);
}

#[test]
fn a_vocabulary_past_what_the_key_indexes_is_refused_naming_the_column() {
    let words = (0..128).map(|index| Scalar::from(format!("v{index}").as_str()));
    let mut column = Serie::from_scalars(symbols_field(true), words).expect("128 words fit int8");
    assert_eq!(column.items().map(Serie::len), Some(128));

    let refusal = column
        .push(Scalar::from("v128"))
        .expect_err("a 129th word is past what an int8 key indexes");
    let text = refusal.to_string();
    assert!(text.contains("symbol"), "names the column: {text}");
    assert!(text.contains("129"), "names the count: {text}");
    assert_eq!(column.len(), 128);
    assert_eq!(column.items().map(Serie::len), Some(128));

    // Words already held intern without growing the vocabulary.
    column
        .extend(vec![Scalar::from("v0"), Scalar::from("v127"), Scalar::Null])
        .expect("held words");
    column.resize(140, Scalar::from("v5")).expect("a held word");
    assert_eq!(column.len(), 140);
    assert_eq!(column.items().map(Serie::len), Some(128));
    assert_eq!(column.scalar(139).unwrap(), Scalar::from("v5"));
}

#[test]
fn a_required_dictionary_child_hidden_under_an_absent_record_crosses_and_is_written() {
    let root = Field::new(
        "row",
        DataType::from(StructType::from_fields([symbols_field(false)]).expect("one named child")),
        true,
    );
    let rows = [
        Scalar::from_sequence([Scalar::from("AAPL")]),
        Scalar::Null,
        Scalar::from_sequence([Scalar::from("MSFT")]),
    ];
    let laid_out = Serie::from_scalars(root.clone(), rows.iter().cloned()).expect("a hidden key");
    assert_eq!(laid_out.len(), 3);
    assert!(laid_out.is_null(1).unwrap());
    assert_eq!(laid_out.scalar(1).unwrap(), Scalar::Null);
    assert_eq!(
        laid_out.scalar(2).unwrap(),
        Scalar::from_sequence([Scalar::from("MSFT")])
    );
    let child = laid_out.child("symbol").expect("the dictionary child");
    assert!(child.as_dictionary().is_some());
    assert_eq!(child.len(), 3);

    let mut pushed = Serie::empty(root).expect("an empty record column");
    for row in rows {
        pushed.push(row).expect("a row the record accepts");
    }
    assert_eq!(pushed, laid_out);
    assert_eq!(
        pushed.into_arrow_array().unwrap().as_ref(),
        laid_out.into_arrow_array().unwrap().as_ref()
    );
}

/// A required dictionary of ISIN values whose vocabulary includes one invalid
/// value. Arrow permits the extension storage; landing decides which values
/// are referenced and therefore need proving.
fn isin_dictionary_column(keys: Vec<i8>) -> (arrow_schema::FieldRef, ArrayRef) {
    let isin = Field::new(
        "isin",
        DataType::dictionary(DataType::Int8, DataType::IsinCode).expect("an int8 key"),
        false,
    )
    .into_arrow_field_ref()
    .expect("the dictionary projects");
    let dictionary: ArrayRef = Arc::new(
        DictionaryArray::<Int8Type>::try_new(
            Int8Array::from(keys),
            Arc::new(StringArray::from(vec![
                "US0378331005",
                "BAD",
                "US5949181045",
            ])),
        )
        .expect("keys within the vocabulary"),
    );
    (isin, dictionary)
}

/// The dictionary as a top-level inferred batch column, with no parent
/// validity handed to its landing.
fn flat_isin_dictionary_batch(keys: Vec<i8>) -> arrow_array::RecordBatch {
    let (isin, dictionary) = isin_dictionary_column(keys);
    let schema = Arc::new(arrow_schema::Schema::new(vec![isin]));
    arrow_array::RecordBatch::try_new(schema, vec![dictionary]).expect("one dictionary column")
}

/// One inferred batch column whose nullable records contain the required
/// dictionary.
fn isin_dictionary_batch(keys: Vec<i8>, present: Vec<bool>) -> arrow_array::RecordBatch {
    let (isin, dictionary) = isin_dictionary_column(keys);
    let records = arrow_array::StructArray::new(
        vec![isin].into(),
        vec![dictionary],
        Some(arrow_buffer::NullBuffer::from(present)),
    );
    let schema = Arc::new(arrow_schema::Schema::new(vec![arrow_schema::Field::new(
        "wrapped",
        records.data_type().clone(),
        true,
    )]));
    arrow_array::RecordBatch::try_new(schema, vec![Arc::new(records)])
        .expect("one nullable record column")
}

#[test]
fn unreferenced_invalid_dictionary_values_are_not_proved() {
    // Slicing away the only key keeps Arrow's shared vocabulary, including
    // BAD. With no logical row referencing it, that storage is not a value.
    let source = flat_isin_dictionary_batch(vec![1]);
    let empty = source.slice(0, 0);
    let dictionary = empty
        .column(0)
        .as_any()
        .downcast_ref::<DictionaryArray<Int8Type>>()
        .expect("the sliced dictionary");
    let vocabulary = dictionary
        .values()
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("the string vocabulary");
    assert_eq!(vocabulary.value(1), "BAD");
    let landed = Serie::from_arrow_batch(None, &empty, ArrowCastOptions::new())
        .expect("an empty dictionary references no vocabulary value");
    assert_eq!(landed.len(), 0);
    assert_eq!(landed.child("isin").map(Serie::len), Some(0));

    // A nonempty dictionary proves only the vocabulary entries its keys use.
    let used = flat_isin_dictionary_batch(vec![0, 2, 0]);
    let landed = Serie::from_arrow_batch(None, &used, ArrowCastOptions::new())
        .expect("the unreferenced invalid value is not a row");
    let values = landed.child("isin").expect("the dictionary column");
    assert_eq!(values.scalar(0).unwrap().as_str(), Some("US0378331005"));
    assert_eq!(values.scalar(1).unwrap().as_str(), Some("US5949181045"));
}

#[test]
fn an_invalid_dictionary_value_is_ignored_only_when_every_key_is_under_a_null_struct() {
    let hidden = isin_dictionary_batch(vec![0, 1, 2], vec![true, false, true]);
    let landed = Serie::from_arrow_batch(None, &hidden, ArrowCastOptions::new())
        .expect("the invalid value is referenced only by an absent record");
    let records = landed.child("wrapped").expect("the nullable records");
    assert!(records.is_null(1).unwrap());
    assert_eq!(records.scalar(1).unwrap(), Scalar::Null);
    let values = records.child("isin").expect("the dictionary child");
    assert_eq!(values.scalar(0).unwrap().as_str(), Some("US0378331005"));
    assert_eq!(values.scalar(2).unwrap().as_str(), Some("US5949181045"));

    // Both rows use the same invalid vocabulary entry. The null record hides
    // one key, but the visible key still makes that shared value observable.
    let exposed = isin_dictionary_batch(vec![0, 1, 1, 2], vec![true, false, true, true]);
    let refusal = Serie::from_arrow_batch(None, &exposed, ArrowCastOptions::new())
        .expect_err("one visible reference must prove the shared value");
    assert!(refusal.to_string().contains("isin"), "{refusal}");
}
