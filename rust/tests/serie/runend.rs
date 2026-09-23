//! `rust/src/serie/runend.rs`: the run-end-encoded leaf - the run ends over
//! their values, read by one search over the runs and cut in place on a
//! write.

use std::sync::Arc;

use arrow_array::types::Int32Type;
use arrow_array::{Array, ArrayRef, Int32Array, RunArray, StringArray};
use yggdryl::{DataType, Field, RunEndEncodedSerie, Scalar, Serie, SerieValue};

/// A run-end field of states, with `run_ends` of `width` over nullable
/// UTF-8 values.
fn states_field(width: DataType, nullable: bool) -> Field {
    Field::new(
        "state",
        DataType::run_end_encoded(
            Field::new("run_ends", width, false),
            Field::new("values", DataType::utf8(), true),
        )
        .expect("an integer run-end width"),
        nullable,
    )
}

/// Five state rows in three runs, the middle run absent:
/// `open, open, null, closed, closed`.
fn states_array() -> ArrayRef {
    Arc::new(
        RunArray::<Int32Type>::try_new(
            &Int32Array::from(vec![2, 3, 5]),
            &StringArray::from(vec![Some("open"), None, Some("closed")]),
        )
        .expect("climbing run ends"),
    )
}

/// [`states_array`] as a nullable column.
fn states() -> Serie {
    Serie::from_arrow_array(states_field(DataType::Int32, true), states_array())
        .expect("a run-end column")
}

/// The five rows [`states_array`] encodes.
fn state_rows() -> Vec<Scalar> {
    vec![
        Scalar::from("open"),
        Scalar::from("open"),
        Scalar::Null,
        Scalar::from("closed"),
        Scalar::from("closed"),
    ]
}

#[test]
fn a_required_column_with_an_absent_run_is_refused_at_the_door_naming_the_column() {
    let refusal = Serie::from_arrow_array(states_field(DataType::Int32, false), states_array())
        .expect_err("a required column admits no absent run");
    assert!(
        refusal.to_string().contains("state"),
        "the refusal names the column: {refusal}"
    );

    let mut required = Serie::from_scalars(
        states_field(DataType::Int32, false),
        [Scalar::from("open"), Scalar::from("open")],
    )
    .expect("two present rows");
    assert!(required.push(Scalar::Null).is_err());
    assert_eq!(required.len(), 2);
}

#[test]
fn a_row_past_the_end_a_value_the_field_refuses_and_a_length_past_the_width_are_refused() {
    let mut column = states();

    let refusal = column.scalar(5).expect_err("row 5 is past the end");
    let text = refusal.to_string();
    assert!(text.contains("state"), "names the column: {text}");
    assert!(text.contains('5'), "names the row: {text}");
    assert!(column.is_null(5).is_err());
    assert!(column.slice(4, 2).is_err());
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
    assert_eq!(column.len(), 5);
    assert_eq!(column.items().map(Serie::len), Some(3));

    // An int16 run end reaches 32767 rows and not one more; nothing moved.
    let mut narrow = Serie::from_scalars(
        states_field(DataType::Int16, true),
        [Scalar::from("open"), Scalar::from("open"), Scalar::Null],
    )
    .expect("three rows");
    let refusal = narrow
        .extend(vec![Scalar::from("closed"); 32_765])
        .expect_err("32768 rows are past an int16 run end");
    let text = refusal.to_string();
    assert!(text.contains("state"), "names the column: {text}");
    assert!(text.contains("32768"), "names the length: {text}");
    assert_eq!(narrow.len(), 3);
    narrow
        .extend(vec![Scalar::from("closed"); 32_764])
        .expect("32767 rows fit an int16 run end");
    assert_eq!(narrow.len(), 32_767);
    assert_eq!(narrow.scalar(32_766).unwrap(), Scalar::from("closed"));
}

#[test]
fn the_buffers_cross_in_and_out_shared_and_a_row_reads_through_its_run() {
    let array = states_array();
    let column = Serie::from_arrow_array(states_field(DataType::Int32, true), Arc::clone(&array))
        .expect("a run-end column");
    let leaf = column.as_run_end_encoded().expect("a run-end column");

    assert_eq!(
        <RunEndEncodedSerie as SerieValue>::from_serie(&column),
        Some(leaf)
    );
    assert_eq!(leaf.run_ends().len(), 3);
    assert_eq!(
        leaf.run_ends().as_int32().expect("int32 run ends").values(),
        &[2, 3, 5]
    );
    assert_eq!(leaf.values().len(), 3);
    assert_eq!(leaf.logical_len(), 5);
    assert_eq!(SerieValue::len(leaf), 5);
    assert_eq!(column.len(), 5);
    assert_eq!(column.items().map(Serie::len), Some(3));
    assert_eq!(leaf.null_count(), 1);
    assert!(leaf.is_null(2).unwrap());
    assert!(!leaf.is_null(3).unwrap());
    assert_eq!(column.rows().into_owned(), state_rows());
    assert_eq!(column, Serie::new(state_rows()));

    let back = column.into_arrow_array().expect("a column");
    assert_eq!(back.data_type(), array.data_type());
    assert!(
        back.to_data().ptr_eq(&array.to_data()),
        "the run ends and the values cross back out shared"
    );
}

#[test]
fn a_sliced_input_and_a_window_are_rebased_onto_the_runs_they_reach() {
    let sliced = states_array().slice(1, 3);
    let column = Serie::from_arrow_array(states_field(DataType::Int32, true), sliced)
        .expect("a sliced run array");
    assert_eq!(column.len(), 3);
    assert_eq!(
        column.rows().into_owned(),
        vec![Scalar::from("open"), Scalar::Null, Scalar::from("closed")]
    );
    let leaf = column.as_run_end_encoded().expect("a run-end column");
    assert_eq!(
        leaf.run_ends().as_int32().expect("int32 run ends").values(),
        &[1, 2, 3]
    );
    assert_eq!(leaf.values().len(), 3);

    let whole = states();
    assert_eq!(whole.slice(1, 3).unwrap(), column);
    let tail = whole.slice(3, 2).expect("the last run");
    assert_eq!(tail.items().map(Serie::len), Some(1));
    assert_eq!(
        tail.rows().into_owned(),
        vec![Scalar::from("closed"), Scalar::from("closed")]
    );
    let empty = whole.slice(5, 0).expect("an empty window");
    assert!(empty.is_empty());
    assert_eq!(empty.items().map(Serie::len), Some(0));
    assert_eq!(empty.null_count(), 0);

    // A window grows on its own rebased runs.
    let mut window = whole.slice(1, 3).unwrap();
    drop(whole);
    window.push(Scalar::from("closed")).expect("a present row");
    window.push(Scalar::Null).expect("an absent row");
    assert_eq!(
        window.rows().into_owned(),
        vec![
            Scalar::from("open"),
            Scalar::Null,
            Scalar::from("closed"),
            Scalar::from("closed"),
            Scalar::Null,
        ]
    );
    assert_eq!(window.null_count(), 2);
}

#[test]
fn the_writes_cut_the_runs_and_the_rows_round_trip() {
    let mut column = states();

    column.push(Scalar::from("closed")).expect("a row");
    column.set(0, Scalar::from("shut")).expect("one slot");
    column
        .insert(2, Scalar::Null)
        .expect("an absent row inside");
    assert_eq!(column.remove(1).unwrap(), Scalar::from("open"));
    column.truncate(4).expect("four rows kept");
    assert_eq!(column.pop().unwrap(), Some(Scalar::from("closed")));

    let rows = vec![Scalar::from("shut"), Scalar::Null, Scalar::Null];
    assert_eq!(column.rows().into_owned(), rows);
    assert_eq!(column.null_count(), 2);
    assert_eq!(
        column,
        Serie::from_scalars(states_field(DataType::Int32, true), rows).expect("three rows")
    );

    // Appending another column joins its runs, no row read.
    let other = Serie::from_scalars(
        states_field(DataType::Int32, true),
        [Scalar::from("open"), Scalar::from("open")],
    )
    .expect("two rows");
    column.extend_from_serie(&other).expect("agreeing fields");
    assert_eq!(column.len(), 5);
    assert_eq!(column.scalar(4).unwrap(), Scalar::from("open"));
    assert!(column.is_null(2).unwrap());

    // A column built by pushes reads as the column laid out at once.
    let laid_out = states();
    let mut pushed = Serie::empty(states_field(DataType::Int32, true)).expect("an empty column");
    for row in state_rows() {
        pushed.push(row).expect("a row the field accepts");
    }
    assert_eq!(pushed, laid_out);
    assert_eq!(pushed.null_count(), 1);
    assert_eq!(pushed.into_arrow_array().unwrap().len(), 5);
    pushed.clear().expect("cleared");
    assert!(pushed.is_empty());
    assert_eq!(pushed.items().map(Serie::len), Some(0));
}

/// The run ends and the values a run-end column holds, as plain rows.
fn runs_of(column: &Serie) -> (Vec<i32>, Vec<Scalar>) {
    let leaf = column.as_run_end_encoded().expect("a run-end column");
    (
        leaf.run_ends()
            .as_int32()
            .expect("int32 run ends")
            .values()
            .to_vec(),
        leaf.values().rows().into_owned(),
    )
}

#[test]
fn a_push_extends_the_last_run_or_appends_one_and_a_write_folds_into_equal_neighbours() {
    let mut column = states();
    let open = || Scalar::from("open");
    let closed = || Scalar::from("closed");

    // The value the last run holds lengthens it: one run end moves.
    column.push(closed()).expect("a row");
    assert_eq!(
        runs_of(&column),
        (vec![2, 3, 6], vec![open(), Scalar::Null, closed()])
    );

    // Another value is one run more.
    column.push(open()).expect("a row");
    assert_eq!(
        runs_of(&column),
        (
            vec![2, 3, 6, 7],
            vec![open(), Scalar::Null, closed(), open()]
        )
    );

    // A row set to its left neighbour's value joins that run, and the run it
    // emptied is gone.
    column.set(2, open()).expect("one slot");
    assert_eq!(
        runs_of(&column),
        (vec![3, 6, 7], vec![open(), closed(), open()])
    );

    // A row set inside a run splits it in three.
    column.set(4, Scalar::Null).expect("one slot");
    assert_eq!(
        runs_of(&column),
        (
            vec![3, 4, 5, 6, 7],
            vec![open(), closed(), Scalar::Null, closed(), open()]
        )
    );

    // Removing what parts two equal runs joins them.
    assert_eq!(column.remove(4).unwrap(), Scalar::Null);
    assert_eq!(
        runs_of(&column),
        (vec![3, 5, 6], vec![open(), closed(), open()])
    );

    // An insert on a boundary equal to the run after it extends that run.
    column.insert(3, closed()).expect("a row on a boundary");
    assert_eq!(
        runs_of(&column),
        (vec![3, 6, 7], vec![open(), closed(), open()])
    );

    // Truncating inside a run shortens it; truncating on a boundary drops
    // the runs past it.
    column.truncate(5).expect("five rows kept");
    assert_eq!(runs_of(&column), (vec![3, 5], vec![open(), closed()]));
    column.truncate(3).expect("three rows kept");
    assert_eq!(runs_of(&column), (vec![3], vec![open()]));

    // Pushed rows fold into the same runs the door reads a laid-out array as.
    let mut pushed = Serie::empty(states_field(DataType::Int32, true)).expect("an empty column");
    for row in state_rows() {
        pushed.push(row).expect("a row the field accepts");
    }
    assert_eq!(runs_of(&pushed), runs_of(&states()));
}

#[test]
fn a_long_walk_of_splices_reads_as_the_same_splices_over_a_plain_run() {
    let field = states_field(DataType::Int32, true);
    let vocabulary = [Scalar::from("a"), Scalar::from("b"), Scalar::Null];
    let mut column = Serie::empty(field.clone()).expect("an empty column");
    let mut expected: Vec<Scalar> = Vec::new();
    // A fixed linear congruential walk: reproducible, and wide enough to
    // start, end and straddle runs of every length.
    let mut state: u64 = 0x5eed;
    let mut next = |bound: usize| {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        usize::try_from(state >> 33).unwrap() % bound
    };
    for _ in 0..600 {
        let len = expected.len();
        let start = next(len + 1);
        let end = start + next(len - start + 1).min(3);
        let count = next(4);
        let rows: Vec<Scalar> = (0..count)
            .map(|_| vocabulary[next(vocabulary.len())].clone())
            .collect();
        column
            .splice(start..end, rows.clone())
            .expect("rows the field accepts");
        expected.splice(start..end, rows);

        assert_eq!(column.rows().into_owned(), expected);
        let (ends, values) = runs_of(&column);
        assert_eq!(ends.len(), values.len());
        assert_eq!(
            ends.last().map_or(0, |end| usize::try_from(*end).unwrap()),
            expected.len()
        );
        assert!(
            values.windows(2).all(|pair| pair[0] != pair[1]),
            "writes alone never leave two equal runs side by side: {values:?}"
        );
    }
    let crossed = Serie::from_arrow_array(field, column.require_arrow_array().unwrap())
        .expect("the column crosses back in");
    assert_eq!(crossed, column);
    assert_eq!(
        column.null_count(),
        expected.iter().filter(|row| **row == Scalar::Null).count()
    );
}
