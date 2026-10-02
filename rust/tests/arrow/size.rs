//! `rust/src/arrow/size.rs`: the one memory estimate every byte bound reads.

use std::sync::Arc;

use arrow_array::builder::{Int8Builder, ListBuilder};
use arrow_array::types::Int32Type;
use arrow_array::{
    Array, ArrayRef, BooleanArray, DictionaryArray, Int8Array, Int32Array, Int64Array,
    LargeListViewArray, ListViewArray, RecordBatch, RunArray, StringArray, StringViewArray,
    StructArray, UnionArray,
};
use arrow_buffer::ScalarBuffer;
use arrow_schema::{DataType as ArrowType, Field as ArrowField, Schema, UnionFields};
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

/// Cut a column into `pieces` equal slices and sum what each one costs.
fn pieces_cost(column: &ArrayRef, pieces: usize) -> usize {
    let width = column.len() / pieces;
    (0..pieces)
        .map(|piece| array_memory_size(&column.slice(piece * width, width)))
        .sum()
}

#[test]
fn a_run_end_slice_counts_only_the_runs_its_window_falls_in() {
    // 256 runs of four rows: four bytes of run end and eight of value each.
    let run_ends = Int32Array::from_iter_values((1..=256).map(|run| run * 4));
    let values = Int64Array::from_iter_values(0..256);
    let runs: ArrayRef = Arc::new(RunArray::<Int32Type>::try_new(&run_ends, &values).unwrap());
    assert_eq!(runs.len(), 1024);
    assert_eq!(array_memory_size(&runs), 256 * (4 + 8));

    // Sixteen rows aligned to the runs reach four of them, never the
    // parent's 256, so the slices sum to the column.
    assert_eq!(array_memory_size(&runs.slice(16, 16)), 4 * (4 + 8));
    assert_eq!(pieces_cost(&runs, 64), array_memory_size(&runs));
    // A window inside one run reaches that run; one across a boundary both.
    assert_eq!(array_memory_size(&runs.slice(1, 2)), 4 + 8);
    assert_eq!(array_memory_size(&runs.slice(3, 2)), 2 * (4 + 8));
    assert_eq!(array_memory_size(&runs.slice(5, 0)), 0);

    // The batch measure reads the same column arm.
    let whole = batch(vec![("price", Arc::clone(&runs))]);
    let piece = whole.slice(16, 16);
    assert_eq!(memory_size(&piece), 4 * (4 + 8));
    assert!(memory_size(&piece) < memory_size(&whole));
}

#[test]
fn a_dense_union_slice_counts_the_child_range_its_offsets_reach() {
    // Rows alternate an int64 and an int32 child, each child's offsets
    // counting up, so a row costs its type id, its offset and its value.
    let fields = UnionFields::try_new(
        vec![0_i8, 1],
        vec![
            ArrowField::new("wide", ArrowType::Int64, false),
            ArrowField::new("narrow", ArrowType::Int32, false),
        ],
    )
    .unwrap();
    let type_ids: Vec<i8> = (0..1024).map(|row| (row % 2) as i8).collect();
    let offsets: Vec<i32> = (0..1024).map(|row| row / 2).collect();
    let union: ArrayRef = Arc::new(
        UnionArray::try_new(
            fields,
            ScalarBuffer::from(type_ids),
            Some(ScalarBuffer::from(offsets)),
            vec![
                Arc::new(Int64Array::from_iter_values(0..512)) as ArrayRef,
                Arc::new(Int32Array::from_iter_values(0..512)) as ArrayRef,
            ],
        )
        .unwrap(),
    );
    let whole = 1024 * (1 + 4) + 512 * 8 + 512 * 4;
    assert_eq!(array_memory_size(&union), whole);

    // Sixteen rows: sixteen type ids and offsets, eight values of each child.
    let piece = array_memory_size(&union.slice(32, 16));
    assert_eq!(piece, 16 * (1 + 4) + 8 * 8 + 8 * 4);
    assert!(piece < whole);
    assert_eq!(pieces_cost(&union, 64), whole);
    // One row of the narrow child reaches that one value and no wide one.
    assert_eq!(array_memory_size(&union.slice(1, 1)), 1 + 4 + 4);
}

#[test]
fn a_sparse_union_slice_counts_its_children_as_the_slice_cut_them() {
    let fields = UnionFields::try_new(
        vec![0_i8, 1],
        vec![
            ArrowField::new("left", ArrowType::Int64, false),
            ArrowField::new("right", ArrowType::Int64, false),
        ],
    )
    .unwrap();
    let type_ids: Vec<i8> = (0..1024).map(|row| (row % 2) as i8).collect();
    let union: ArrayRef = Arc::new(
        UnionArray::try_new(
            fields,
            ScalarBuffer::from(type_ids),
            None,
            vec![ids(1024), ids(1024)],
        )
        .unwrap(),
    );
    let whole = 1024 + 2 * 1024 * 8;
    assert_eq!(array_memory_size(&union), whole);
    assert_eq!(array_memory_size(&union.slice(16, 16)), 16 + 2 * 16 * 8);
    assert_eq!(pieces_cost(&union, 64), whole);
}

#[test]
fn a_list_view_slice_counts_its_offsets_sizes_and_the_child_range_they_reach() {
    // Two values a row: an offset, a size and sixteen child bytes each.
    let offsets: Vec<i32> = (0..1024).map(|row| row * 2).collect();
    let field = Arc::new(ArrowField::new("item", ArrowType::Int64, false));
    let lists: ArrayRef = Arc::new(
        ListViewArray::try_new(
            Arc::clone(&field),
            ScalarBuffer::from(offsets),
            ScalarBuffer::from(vec![2_i32; 1024]),
            ids(2048),
            None,
        )
        .unwrap(),
    );
    let whole = 1024 * (4 + 4 + 2 * 8);
    assert_eq!(array_memory_size(&lists), whole);
    let piece = array_memory_size(&lists.slice(16, 16));
    assert_eq!(piece, 16 * (4 + 4 + 2 * 8));
    assert!(piece < whole);
    assert_eq!(pieces_cost(&lists, 64), whole);

    // Rows may share or reorder their values: the range is the lowest
    // offset to the highest end, and an empty row reaches nothing.
    let shuffled: ArrayRef = Arc::new(
        LargeListViewArray::try_new(
            field,
            ScalarBuffer::from(vec![6_i64, 0, 2, 9]),
            ScalarBuffer::from(vec![2_i64, 2, 2, 0]),
            ids(10),
            None,
        )
        .unwrap(),
    );
    // Four eight-byte offsets and sizes, child values 0 to 8.
    assert_eq!(array_memory_size(&shuffled), 4 * (8 + 8) + 8 * 8);
    // Rows two and three: values 2 and 3 alone, the empty row reaching none.
    assert_eq!(
        array_memory_size(&shuffled.slice(2, 2)),
        2 * (8 + 8) + 2 * 8
    );
}
