//! `rust/src/arrow/size.rs`: the one memory estimate every byte bound reads.

use std::sync::Arc;

use arrow_array::builder::{Int8Builder, ListBuilder};
use arrow_array::{
    Array, ArrayRef, BooleanArray, DictionaryArray, Int8Array, Int64Array, RecordBatch,
    StringArray, StringViewArray, StructArray,
};
use arrow_schema::{DataType as ArrowType, Field as ArrowField, Schema};
use yggdryl::arrow::{array_memory_size, memory_size, scalar_memory_size};
use yggdryl::{DataType, Scalar};

fn ids(rows: i64) -> ArrayRef {
    Arc::new(Int64Array::from_iter_values(0..rows))
}

fn batch(columns: Vec<(&str, ArrayRef)>) -> RecordBatch {
    let fields: Vec<ArrowField> = columns
        .iter()
        .map(|(name, column)| {
            ArrowField::new(*name, column.data_type().clone(), column.null_count() > 0)
        })
        .collect();
    RecordBatch::try_new(
        Arc::new(Schema::new(fields)),
        columns.into_iter().map(|(_, column)| column).collect(),
    )
    .expect("a batch")
}

#[test]
fn a_slice_costs_its_own_rows_and_not_its_parents_buffers() {
    let whole = batch(vec![("id", ids(1024))]);
    assert_eq!(memory_size(&whole), 8 * 1024);

    // The parent's buffers are untouched by the slice, and Arrow's own
    // accounting reports them whole; this one reports the rows reached.
    let piece = whole.slice(100, 16);
    assert_eq!(memory_size(&piece), 8 * 16);
    assert_eq!(piece.get_array_memory_size(), whole.get_array_memory_size());

    // Cut a batch into slices and the slices sum to the batch, which is
    // what makes a byte bound spread over them count the batch once.
    let pieces: usize = (0..64)
        .map(|at| memory_size(&whole.slice(at * 16, 16)))
        .sum();
    assert_eq!(pieces, memory_size(&whole));
}

#[test]
fn a_byte_column_counts_its_sliced_offsets_and_the_bytes_they_span() {
    let text: ArrayRef = Arc::new(StringArray::from(vec!["ab", "cdef", "", "ghijkl"]));
    // Five offsets of four bytes, twelve bytes of text.
    assert_eq!(array_memory_size(&text), 5 * 4 + 12);
    // The middle two rows: three offsets, four bytes.
    assert_eq!(array_memory_size(&text.slice(1, 2)), 3 * 4 + 4);

    let flags: ArrayRef = Arc::new(BooleanArray::from(vec![true; 9]));
    assert_eq!(array_memory_size(&flags), 2);
    let narrow: ArrayRef = Arc::new(Int8Array::from(vec![Some(1), None, Some(3)]));
    // Three one-byte values and one validity byte.
    assert_eq!(array_memory_size(&narrow), 3 + 1);
}

#[test]
fn a_view_column_counts_its_views_and_the_out_of_line_bytes_they_reach() {
    let short = "inline";
    let long = "a value longer than twelve bytes lives out of line";
    let views: ArrayRef = Arc::new(StringViewArray::from(vec![short, long, short, long]));
    // Four sixteen-byte views; only the two long values reach a data buffer.
    assert_eq!(array_memory_size(&views), 4 * 16 + 2 * long.len());
    // A slice reaches one of each.
    assert_eq!(array_memory_size(&views.slice(1, 2)), 2 * 16 + long.len());
}

#[test]
fn a_struct_counts_its_children_and_a_list_the_child_range_it_spans() {
    let record: ArrayRef = Arc::new(StructArray::from(vec![
        (
            Arc::new(ArrowField::new("id", ArrowType::Int64, false)),
            ids(8),
        ),
        (
            Arc::new(ArrowField::new("flag", ArrowType::Boolean, false)),
            Arc::new(BooleanArray::from(vec![true; 8])) as ArrayRef,
        ),
    ]));
    assert_eq!(array_memory_size(&record), 8 * 8 + 1);
    assert_eq!(array_memory_size(&record.slice(2, 4)), 4 * 8 + 1);

    let mut lists = ListBuilder::new(Int8Builder::new());
    for run in [1_i8..4, 4..5, 5..9] {
        lists.values().append_slice(&run.collect::<Vec<_>>());
        lists.append(true);
    }
    let lists: ArrayRef = Arc::new(lists.finish());
    // Four offsets, eight child bytes.
    assert_eq!(array_memory_size(&lists), 4 * 4 + 8);
    // The last row alone: two offsets, the four children it spans.
    assert_eq!(array_memory_size(&lists.slice(2, 1)), 2 * 4 + 4);
}

#[test]
fn a_dictionary_counts_its_keys_sliced_and_its_values_whole() {
    let values = StringArray::from(vec!["XNAS", "XLON", "XNYS"]);
    let keys = Int8Array::from(vec![0_i8, 1, 2, 0, 1, 2, 0, 1]);
    let dictionary: ArrayRef = Arc::new(DictionaryArray::new(
        keys,
        Arc::new(values.clone()) as ArrayRef,
    ));
    let whole_values = values.get_array_memory_size();
    assert_eq!(array_memory_size(&dictionary), 8 + whole_values);
    // The keys slice; the values are shared by every row and count whole.
    assert_eq!(array_memory_size(&dictionary.slice(0, 2)), 2 + whole_values);
}

#[test]
fn a_scalar_costs_its_payload_and_a_fixed_row_width() {
    // A null is a validity bit, charged the row width alone.
    assert_eq!(scalar_memory_size(&Scalar::Null), 16);
    assert_eq!(scalar_memory_size(&Scalar::from("venue")), 16 + 5);
    assert_eq!(scalar_memory_size(&Scalar::from(7_i64)), 16 + 16);

    // A row is its cells plus its own width, a named row its keys too.
    let row = Scalar::from_sequence([Scalar::from(7_i64), Scalar::from("venue")]);
    assert_eq!(scalar_memory_size(&row), 16 + 16 + (16 + 5));
    let record = Scalar::from_struct([
        ("id", Scalar::from(7_i64)),
        ("venue", Scalar::from("venue")),
    ])
    .expect("a record");
    assert_eq!(scalar_memory_size(&record), 16 + 16 + (2 + 16) + (5 + 5));

    // A held column costs its buffers as the array estimate counts them.
    let field = DataType::Int64.required_field("id");
    let column = yggdryl::Serie::from_scalars(field, [Scalar::from(1_i64), Scalar::from(2_i64)])
        .expect("a column");
    let held = Scalar::from(column.clone());
    assert_eq!(
        scalar_memory_size(&held),
        16 + 16 + array_memory_size(&column.into_arrow_array().expect("an array"))
    );
}
