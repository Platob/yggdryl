//! `rust/src/serie/runend.rs`: the run-end-encoded leaf - the run ends over
//! their values, read by one search over the runs and cut in place on a
//! write.

use std::sync::Arc;

use arrow_array::types::Int32Type;
use arrow_array::{Array, ArrayRef, Int32Array, RunArray, StringArray};
use yggdryl::{ArrowCastOptions, DataType, Field, RunEndEncodedSerie, Scalar, Serie, SerieValue};

/// The options a refusal is pinned under: a present value is never nulled.
fn strict() -> ArrowCastOptions {
    ArrowCastOptions::new().with_safe(false)
}

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
    Serie::from_arrow_array(
        Some(&states_field(DataType::Int32, true)),
        states_array(),
        ArrowCastOptions::new(),
    )
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
    let refusal = Serie::from_arrow_array(
        Some(&states_field(DataType::Int32, false)),
        states_array(),
        strict(),
    )
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
    let column = Serie::from_arrow_array(
        Some(&states_field(DataType::Int32, true)),
        Arc::clone(&array),
        ArrowCastOptions::new(),
    )
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
    let column = Serie::from_arrow_array(
        Some(&states_field(DataType::Int32, true)),
        sliced,
        ArrowCastOptions::new(),
    )
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
    let crossed = Serie::from_arrow_array(
        Some(&field),
        column.require_arrow_array().unwrap(),
        ArrowCastOptions::new(),
    )
    .expect("the column crosses back in");
    assert_eq!(crossed, column);
    assert_eq!(
        column.null_count(),
        expected.iter().filter(|row| **row == Scalar::Null).count()
    );
}

/// One inferred batch column whose nullable records contain required ISIN
/// values under Arrow's run-end encoding.
fn isin_run_batch(
    run_ends: Vec<i32>,
    values: Vec<&str>,
    present: Vec<bool>,
) -> arrow_array::RecordBatch {
    let isin = Field::new(
        "isin",
        DataType::run_end_encoded(
            Field::new("run_ends", DataType::Int32, false),
            Field::new("values", DataType::Isin, false),
        )
        .expect("int32 run ends"),
        false,
    )
    .into_arrow_field_ref()
    .expect("the run-end field projects");
    let plain =
        RunArray::<Int32Type>::try_new(&Int32Array::from(run_ends), &StringArray::from(values))
            .expect("climbing run ends");
    let encoded = arrow_array::make_array(
        plain
            .to_data()
            .into_builder()
            .data_type(isin.data_type().clone())
            .build()
            .expect("the projected value extension is legal Arrow metadata"),
    );
    let records = arrow_array::StructArray::new(
        vec![isin].into(),
        vec![encoded],
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
fn an_invalid_run_value_is_ignored_only_when_the_whole_run_is_under_a_null_struct() {
    let hidden = isin_run_batch(
        vec![1, 2, 3],
        vec!["US0378331005", "BAD", "US5949181045"],
        vec![true, false, true],
    );
    let landed = Serie::from_arrow_batch(None, &hidden, ArrowCastOptions::new())
        .expect("the invalid run is wholly under an absent record");
    let records = landed.child("wrapped").expect("the nullable records");
    assert!(records.is_null(1).unwrap());
    assert_eq!(records.scalar(1).unwrap(), Scalar::Null);
    let values = records.child("isin").expect("the run-end child");
    assert_eq!(values.scalar(0).unwrap().as_str(), Some("US0378331005"));
    assert_eq!(values.scalar(2).unwrap().as_str(), Some("US5949181045"));

    // The invalid run spans rows 1 and 2. The first is hidden, but the second
    // is visible, so the run's one physical value still has to be proved.
    let exposed = isin_run_batch(
        vec![1, 3, 4],
        vec!["US0378331005", "BAD", "US5949181045"],
        vec![true, false, true, true],
    );
    let refusal = Serie::from_arrow_batch(None, &exposed, ArrowCastOptions::new())
        .expect_err("a run crossing into a visible row must be proved");
    assert!(refusal.to_string().contains("values"), "{refusal}");
}

#[test]
fn list_view_gathers_sliced_isin_runs_in_reordered_overlapping_order() {
    const A: &str = "US0378331005";
    const B: &str = "US5949181045";
    const C: &str = "GB0002634946";

    let item = Field::new(
        "item",
        DataType::run_end_encoded(
            DataType::Int32.required_field("run_ends"),
            DataType::Isin.required_field("values"),
        )
        .expect("int32 run ends"),
        false,
    );
    let projected = item
        .clone()
        .into_arrow_field_ref()
        .expect("the run-end item projects");
    let plain = RunArray::<Int32Type>::try_new(
        &Int32Array::from(vec![2, 4, 6, 8, 10]),
        &StringArray::from(vec![A, B, "BAD", C, A]),
    )
    .expect("climbing run ends");
    let typed = arrow_array::make_array(
        plain
            .to_data()
            .into_builder()
            .data_type(projected.data_type().clone())
            .build()
            .expect("the ISIN extension is legal Arrow metadata"),
    );
    let sliced = typed.slice(1, 8);
    assert_eq!(
        sliced.offset(),
        1,
        "the gather starts from a sliced run array"
    );
    let views: ArrayRef = Arc::new(arrow_array::ListViewArray::new(
        projected,
        arrow_buffer::ScalarBuffer::from(vec![5_i32, 3, 0, 1, 5]),
        arrow_buffer::ScalarBuffer::from(vec![3_i32, 2, 3, 2, 2]),
        sliced,
        Some(arrow_buffer::NullBuffer::from(vec![
            true, false, true, true, true,
        ])),
    ));
    views
        .to_data()
        .validate_full()
        .expect("the sliced, overlapping source is legal Arrow");
    let field = Field::new("spans", DataType::serie_view(item), true);
    let column = Serie::from_arrow_array(Some(&field), views, ArrowCastOptions::new())
        .expect("the null view hides the only BAD run");

    let seq = |values: &[&str]| {
        Scalar::SerieView(Serie::new(
            values
                .iter()
                .map(|value| Scalar::Isin(yggdryl::Isin::new(value).unwrap()))
                .collect::<Vec<_>>(),
        ))
    };
    assert_eq!(
        column.rows().into_owned(),
        vec![
            seq(&[C, C, A]),
            Scalar::Null,
            seq(&[A, B, B]),
            seq(&[B, B]),
            seq(&[C, C]),
        ]
    );
    let items = column
        .items()
        .and_then(Serie::as_run_end_encoded)
        .expect("the gathered run-end items");
    assert_eq!(
        (0..items.len())
            .map(|row| items.scalar(row).unwrap())
            .collect::<Vec<_>>(),
        [C, C, A, A, B, B, B, B, C, C]
            .into_iter()
            .map(|value| Scalar::Isin(yggdryl::Isin::new(value).unwrap()))
            .collect::<Vec<_>>()
    );

    let exported = column.require_arrow_array().expect("an Arrow list view");
    exported
        .to_data()
        .validate_full()
        .expect("the gathered run ends and values remain aligned");
    let arrow_schema::DataType::ListView(item) = exported.data_type() else {
        panic!("the list-view layout changed: {}", exported.data_type());
    };
    let arrow_schema::DataType::RunEndEncoded(_, values) = item.data_type() else {
        panic!(
            "the item is no longer run-end encoded: {}",
            item.data_type()
        );
    };
    assert_eq!(item.name(), "item");
    assert_eq!(values.name(), "values");
    assert_eq!(
        values
            .metadata()
            .get("ARROW:extension:name")
            .map(String::as_str),
        Some("yggdryl.isin")
    );
}

#[test]
fn list_view_run_gather_refuses_an_i16_logical_total_before_building_arrow() {
    let item = Field::new(
        "item",
        DataType::run_end_encoded(
            DataType::Int16.required_field("run_ends"),
            DataType::Isin.required_field("values"),
        )
        .expect("int16 run ends"),
        false,
    );
    let projected = item
        .clone()
        .into_arrow_field_ref()
        .expect("the run-end item projects");
    let plain = arrow_array::RunArray::<arrow_array::types::Int16Type>::try_new(
        &arrow_array::Int16Array::from(vec![20_000_i16]),
        &StringArray::from(vec!["US0378331005"]),
    )
    .expect("one physical run within int16");
    let typed = arrow_array::make_array(
        plain
            .to_data()
            .into_builder()
            .data_type(projected.data_type().clone())
            .build()
            .expect("the ISIN extension is legal Arrow metadata"),
    );
    let views: ArrayRef = Arc::new(arrow_array::ListViewArray::new(
        projected,
        arrow_buffer::ScalarBuffer::from(vec![0_i32, 0, 0]),
        arrow_buffer::ScalarBuffer::from(vec![20_000_i32, 1, 20_000]),
        typed,
        Some(arrow_buffer::NullBuffer::from(vec![true, false, true])),
    ));
    views
        .to_data()
        .validate_full()
        .expect("each view fits the one physical source run");
    let field = Field::new("spans", DataType::serie_view(item), true);

    let refusal = Serie::from_arrow_array(Some(&field), views, ArrowCastOptions::new())
        .expect_err("two repeated spans total 40000 rows, past int16");
    let message = refusal.to_string();
    assert!(
        message.contains("spans"),
        "names the outer column: {message}"
    );
    assert!(
        message.contains("40000"),
        "names the logical selection: {message}"
    );
    assert!(
        message.contains("int16"),
        "names the run-end width: {message}"
    );
}

#[test]
fn list_view_run_gather_rejects_a_huge_single_run_at_the_budget() {
    const SPAN: i32 = 50_000_000;

    let item = Field::new(
        "item",
        DataType::run_end_encoded(
            DataType::Int32.required_field("run_ends"),
            DataType::Isin.required_field("values"),
        )
        .expect("int32 run ends"),
        false,
    );
    let projected = item
        .clone()
        .into_arrow_field_ref()
        .expect("the run-end item projects");
    let plain = RunArray::<Int32Type>::try_new(
        &Int32Array::from(vec![SPAN]),
        &StringArray::from(vec!["US0378331005"]),
    )
    .expect("one physical run represents fifty million rows");
    let typed = arrow_array::make_array(
        plain
            .to_data()
            .into_builder()
            .data_type(projected.data_type().clone())
            .build()
            .expect("the ISIN extension is legal Arrow metadata"),
    );
    let views: ArrayRef = Arc::new(arrow_array::ListViewArray::new(
        projected,
        arrow_buffer::ScalarBuffer::from(vec![0_i32, 0, 0]),
        arrow_buffer::ScalarBuffer::from(vec![SPAN, 1, SPAN]),
        typed,
        Some(arrow_buffer::NullBuffer::from(vec![true, false, true])),
    ));
    // `ListViewArray::new` has already checked the three view bounds. A full
    // Arrow validation would walk the fifty-million-row RunArray before the
    // landing can exercise its constant-time materialization-budget refusal.
    let refusal = Serie::from_arrow_array(None, views, ArrowCastOptions::new())
        .expect_err("one hundred million output rows exceed the materialization budget");
    let message = refusal.to_string();
    assert!(
        message.contains("item"),
        "names the inferred root item: {message}"
    );
    assert!(
        message.contains("expanded slots"),
        "names the resource: {message}"
    );
    assert!(
        message.contains("100000000"),
        "names the requested rows: {message}"
    );
    assert!(message.contains("1000000"), "names the limit: {message}");
}

#[test]
fn list_view_gathers_nested_run_values_with_null_and_zero_width_children() {
    const A: &str = "US0378331005";
    const B: &str = "US5949181045";
    const C: &str = "GB0002634946";

    let empty_field = Field::new(
        "empty",
        DataType::fixed_size_serie(DataType::Int64.required_field("item"), 0)
            .expect("zero is a fixed width"),
        false,
    );
    let values_field = Field::new(
        "values",
        DataType::from(
            yggdryl::StructType::from_fields([DataType::Isin.required_field("code"), empty_field])
                .expect("two record children"),
        ),
        true,
    );
    let arrow_values = values_field
        .clone()
        .into_arrow_field_ref()
        .expect("the run values project");
    let arrow_schema::DataType::Struct(value_fields) = arrow_values.data_type() else {
        panic!(
            "the values are no longer a record: {}",
            arrow_values.data_type()
        );
    };
    let empty: ArrayRef = Arc::new(
        arrow_array::FixedSizeListArray::try_new_with_length(
            DataType::Int64
                .required_field("item")
                .into_arrow_field_ref()
                .unwrap(),
            0,
            Arc::new(arrow_array::Int64Array::from(Vec::<i64>::new())),
            None,
            6,
        )
        .expect("six zero-width rows"),
    );
    let values = arrow_array::StructArray::new(
        value_fields.clone(),
        vec![
            Arc::new(StringArray::from(vec![A, "BAD", B, "BAD", C, A])),
            empty,
        ],
        Some(arrow_buffer::NullBuffer::from(vec![
            true, false, true, true, true, true,
        ])),
    );
    let item = Field::new(
        "item",
        DataType::run_end_encoded(DataType::Int32.required_field("run_ends"), values_field)
            .expect("int32 run ends"),
        true,
    );
    let projected = item
        .clone()
        .into_arrow_field_ref()
        .expect("the nested run-end item projects");
    let plain =
        RunArray::<Int32Type>::try_new(&Int32Array::from(vec![2, 4, 6, 8, 10, 12]), &values)
            .expect("six two-row runs");
    let typed = arrow_array::make_array(
        plain
            .to_data()
            .into_builder()
            .data_type(projected.data_type().clone())
            .build()
            .expect("the nested projected fields are legal Arrow metadata"),
    );
    let views: ArrayRef = Arc::new(arrow_array::ListViewArray::new(
        projected,
        arrow_buffer::ScalarBuffer::from(vec![8_i32, 1, 6, 0]),
        arrow_buffer::ScalarBuffer::from(vec![2_i32, 5, 2, 3]),
        typed,
        Some(arrow_buffer::NullBuffer::from(vec![
            true, true, false, true,
        ])),
    ));
    views
        .to_data()
        .validate_full()
        .expect("the noncontiguous nested views are legal Arrow");
    let field = Field::new("spans", DataType::serie_view(item), true);
    let column = Serie::from_arrow_array(Some(&field), views, ArrowCastOptions::new())
        .expect("the null view hides the only present BAD record");

    let list_lengths = [Some(2), Some(5), None, Some(3)];
    for (row, expected) in list_lengths.into_iter().enumerate() {
        let scalar = column.scalar(row).expect("a list-view row");
        assert_eq!(
            scalar.as_sequence().map(<[Scalar]>::len),
            expected,
            "outer row {row}"
        );
    }
    let items = column
        .items()
        .and_then(Serie::as_run_end_encoded)
        .expect("the gathered nested runs");
    let expected = [
        Some(C),
        Some(C),
        Some(A),
        None,
        None,
        Some(B),
        Some(B),
        Some(A),
        Some(A),
        None,
    ];
    assert_eq!(items.logical_len(), expected.len());
    for (row, expected) in expected.into_iter().enumerate() {
        let value = items.scalar(row).expect("a gathered run row");
        match expected {
            None => assert_eq!(value, Scalar::Null, "row {row}"),
            Some(code) => {
                let cells = value.as_sequence().expect("a present record value");
                assert_eq!(cells[0].as_str(), Some(code), "row {row}");
                assert_eq!(
                    cells[1].as_sequence().map(<[Scalar]>::len),
                    Some(0),
                    "row {row} keeps its zero-width child"
                );
            }
        }
    }

    let records = items.values();
    let codes = records.child("code").expect("the narrow child");
    let empty = records
        .child("empty")
        .and_then(Serie::as_fixed_size_serie)
        .expect("the zero-width child");
    assert_eq!(empty.width(), 0);
    assert_eq!(empty.len(), records.len());
    for row in 0..records.len() {
        assert_eq!(empty.scalar(row).unwrap(), Scalar::from_sequence([]));
        if records.is_null(row).unwrap() {
            assert_eq!(
                codes.scalar(row).unwrap(),
                Scalar::Null,
                "a null record owns a safe physical placeholder"
            );
        } else {
            assert_ne!(codes.scalar(row).unwrap().as_str(), Some("BAD"));
        }
    }

    let exported = column.require_arrow_array().expect("an Arrow list view");
    exported
        .to_data()
        .validate_full()
        .expect("all gathered descendants remain aligned");
    let arrow_schema::DataType::ListView(item) = exported.data_type() else {
        panic!("the outer layout changed: {}", exported.data_type());
    };
    let arrow_schema::DataType::RunEndEncoded(_, values) = item.data_type() else {
        panic!(
            "the item is no longer run-end encoded: {}",
            item.data_type()
        );
    };
    let arrow_schema::DataType::Struct(fields) = values.data_type() else {
        panic!(
            "the run values are no longer records: {}",
            values.data_type()
        );
    };
    assert_eq!(
        fields[0]
            .metadata()
            .get("ARROW:extension:name")
            .map(String::as_str),
        Some("yggdryl.isin")
    );
    assert!(matches!(
        fields[1].data_type(),
        arrow_schema::DataType::FixedSizeList(_, 0)
    ));
}

#[test]
fn list_view_gathers_struct_items_with_nested_isin_runs() {
    const A: &str = "US0378331005";
    const B: &str = "US5949181045";
    const C: &str = "GB0002634946";

    let run_field = Field::new(
        "encoded",
        DataType::run_end_encoded(
            DataType::Int32.required_field("run_ends"),
            DataType::Isin.required_field("values"),
        )
        .expect("int32 run ends"),
        false,
    );
    let projected_run = run_field
        .clone()
        .into_arrow_field_ref()
        .expect("the nested run-end field projects");
    let plain = RunArray::<Int32Type>::try_new(
        &Int32Array::from(vec![2, 4, 6, 8, 10, 12]),
        &StringArray::from(vec![A, B, "BAD", C, A, B]),
    )
    .expect("six two-row runs");
    let runs = arrow_array::make_array(
        plain
            .to_data()
            .into_builder()
            .data_type(projected_run.data_type().clone())
            .build()
            .expect("the ISIN extension is legal Arrow metadata"),
    );

    let item = Field::new(
        "item",
        DataType::from(
            yggdryl::StructType::from_fields([run_field, DataType::Int64.required_field("id")])
                .expect("two record children"),
        ),
        false,
    );
    let projected_item = item
        .clone()
        .into_arrow_field_ref()
        .expect("the record item projects");
    let arrow_schema::DataType::Struct(item_fields) = projected_item.data_type() else {
        panic!(
            "the item is no longer a record: {}",
            projected_item.data_type()
        );
    };
    let records: ArrayRef = Arc::new(arrow_array::StructArray::new(
        item_fields.clone(),
        vec![
            runs,
            Arc::new(arrow_array::Int64Array::from(vec![
                0_i64, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11,
            ])),
        ],
        None,
    ));
    let views: ArrayRef = Arc::new(arrow_array::ListViewArray::new(
        projected_item,
        arrow_buffer::ScalarBuffer::from(vec![8_i32, 0, 4, 6]),
        arrow_buffer::ScalarBuffer::from(vec![4_i32, 4, 2, 4]),
        records,
        Some(arrow_buffer::NullBuffer::from(vec![
            true, true, false, true,
        ])),
    ));
    let field = Field::new("spans", DataType::serie_view(item), true);
    let column = Serie::from_arrow_array(Some(&field), views, ArrowCastOptions::new())
        .expect("the null view hides the invalid nested run value");

    for (row, expected) in [Some(4), Some(4), None, Some(4)].into_iter().enumerate() {
        assert_eq!(
            column
                .scalar(row)
                .expect("a list-view row")
                .as_sequence()
                .map(<[Scalar]>::len),
            expected,
            "outer row {row}"
        );
    }
    let records = column.items().expect("the gathered record items");
    let encoded = records
        .child("encoded")
        .and_then(Serie::as_run_end_encoded)
        .expect("the nested run-end child");
    let expected_codes = [A, A, B, B, A, A, B, B, C, C, A, A];
    assert_eq!(encoded.logical_len(), expected_codes.len());
    for (row, expected) in expected_codes.into_iter().enumerate() {
        assert_eq!(
            encoded.scalar(row).unwrap().as_str(),
            Some(expected),
            "row {row}"
        );
    }
    assert_eq!(
        records
            .child("id")
            .and_then(Serie::as_int64)
            .expect("the aligned id child")
            .values(),
        &[8, 9, 10, 11, 0, 1, 2, 3, 6, 7, 8, 9]
    );

    let exported = column.require_arrow_array().expect("an Arrow list view");
    exported
        .to_data()
        .validate_full()
        .expect("the gathered record descendants remain aligned");
    let arrow_schema::DataType::ListView(item) = exported.data_type() else {
        panic!("the outer layout changed: {}", exported.data_type());
    };
    let arrow_schema::DataType::Struct(fields) = item.data_type() else {
        panic!("the item is no longer a record: {}", item.data_type());
    };
    let arrow_schema::DataType::RunEndEncoded(_, values) = fields[0].data_type() else {
        panic!("the nested child is no longer run-end encoded");
    };
    assert_eq!(
        values
            .metadata()
            .get("ARROW:extension:name")
            .map(String::as_str),
        Some("yggdryl.isin")
    );
}
