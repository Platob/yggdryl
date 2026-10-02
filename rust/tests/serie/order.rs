//! `rust/src/serie/order.rs`: the ordering, uniqueness and grouping verbs
//! every leaf answers, the reads leaving the serie as it was and the
//! `as_*` writes bringing it into the state in place.

use std::sync::Arc;

use arrow_array::types::Int8Type;
use arrow_array::{
    Array, ArrayRef, BooleanArray, DictionaryArray, Int8Array, Int64Array, ListArray, StringArray,
    StringViewArray,
};
use arrow_buffer::OffsetBuffer;
use arrow_schema::{DataType as ArrowDataType, Field as ArrowField};
use yggdryl::{
    ArrowCastOptions, DataType, Field, FieldPath, Scalar, Serie, SortOptions, StructType,
};

fn i64s(values: &[i64]) -> Vec<Scalar> {
    values.iter().map(|value| Scalar::from(*value)).collect()
}

fn u32s(values: &[u32]) -> Vec<Scalar> {
    values.iter().map(|value| Scalar::from(*value)).collect()
}

/// `values` as an int64 column, `None` an absent row, holding its buffer
/// alone.
fn int64_column(values: Vec<Option<i64>>) -> Serie {
    let nullable = values.iter().any(Option::is_none);
    let array: ArrayRef = Arc::new(Int64Array::from(values));
    let field = Field::new("price", DataType::Int64, nullable);
    Serie::from_arrow_array(Some(&field), array, ArrowCastOptions::new()).expect("an int64 column")
}

/// `values` as a utf8 column.
fn utf8_column(values: Vec<Option<&str>>) -> Serie {
    let nullable = values.iter().any(Option::is_none);
    let array: ArrayRef = Arc::new(StringArray::from(values));
    let field = Field::new("venue", DataType::utf8(), nullable);
    Serie::from_arrow_array(Some(&field), array, ArrowCastOptions::new()).expect("a utf8 column")
}

/// A record column of `(venue, price)` rows.
fn quotes(rows: &[(&str, i64)]) -> Serie {
    let root = Field::new(
        "quote",
        DataType::from(
            StructType::from_fields([
                Field::new("venue", DataType::utf8(), false),
                Field::new("price", DataType::Int64, false),
            ])
            .expect("two children"),
        ),
        false,
    );
    Serie::from_scalars(
        root,
        rows.iter().map(|(venue, price)| {
            Scalar::from_sequence([Scalar::from(*venue), Scalar::from(*price)])
        }),
    )
    .expect("quote rows")
}

#[test]
fn sort_indices_answers_a_uint32_index_column_stable_on_both_leaves() {
    let run = Serie::new(i64s(&[3, 1, 2, 1]));
    let column = int64_column(vec![Some(3), Some(1), Some(2), Some(1)]);
    for serie in [&run, &column] {
        let order = serie
            .sort_indices(SortOptions::default())
            .expect("an order");
        assert_eq!(order.field().map(Field::name), Some("index"));
        assert_eq!(order.field().map(Field::dtype), Some(&DataType::UInt32));
        // The two equal rows keep their order: stable.
        assert_eq!(order.rows().to_vec(), u32s(&[1, 3, 2, 0]));
        let order = serie
            .sort_indices(SortOptions::descending())
            .expect("an order");
        assert_eq!(order.rows().to_vec(), u32s(&[0, 2, 1, 3]));
    }
}

#[test]
fn absent_rows_gather_to_the_end_the_options_name() {
    let run = Serie::new(vec![
        Scalar::Null,
        Scalar::from(2_i64),
        Scalar::Null,
        Scalar::from(1_i64),
    ]);
    let column = int64_column(vec![None, Some(2), None, Some(1)]);
    let strings = utf8_column(vec![None, Some("b"), None, Some("a")]);
    for serie in [&run, &column, &strings] {
        let last = serie
            .sort_indices(SortOptions::default())
            .expect("an order");
        assert_eq!(last.rows().to_vec(), u32s(&[3, 1, 0, 2]));
        let first = serie
            .sort_indices(SortOptions::descending().with_nulls_first(true))
            .expect("an order");
        assert_eq!(first.rows().to_vec(), u32s(&[0, 2, 1, 3]));
    }
}

#[test]
fn is_sorted_reads_adjacent_rows_under_the_options_on_both_leaves() {
    let run = Serie::new(vec![
        Scalar::from(1_i64),
        Scalar::from(1_i64),
        Scalar::from(2_i64),
        Scalar::Null,
    ]);
    let column = int64_column(vec![Some(1), Some(1), Some(2), None]);
    for serie in [&run, &column] {
        assert!(serie.is_sorted(SortOptions::default()));
        assert!(!serie.is_sorted(SortOptions::descending()));
        assert!(!serie.is_sorted(SortOptions::ascending().with_nulls_first(true)));
        assert!(
            serie
                .into_reversed()
                .is_sorted(SortOptions::descending().with_nulls_first(true))
        );
    }
    assert!(Serie::new(Vec::<Scalar>::new()).is_sorted(SortOptions::default()));
    assert!(
        int64_column(vec![Some(1)]).is_sorted(SortOptions::descending().with_nulls_first(true))
    );
}

#[test]
fn uniqueness_counts_an_absent_row_as_one_value_on_both_leaves() {
    let run = Serie::new(vec![
        Scalar::from(1_i64),
        Scalar::Null,
        Scalar::from(1_i64),
        Scalar::Null,
        Scalar::from(2_i64),
    ]);
    let column = int64_column(vec![Some(1), None, Some(1), None, Some(2)]);
    for serie in [&run, &column] {
        assert!(!serie.is_unique());
        assert_eq!(serie.unique_count(), 3);
        let unique = serie.into_unique().expect("the first occurrences");
        assert_eq!(
            unique.rows().to_vec(),
            vec![Scalar::from(1_i64), Scalar::Null, Scalar::from(2_i64)]
        );
        assert!(unique.is_unique());
        assert_eq!(unique.field(), serie.field());
        // The serie is as it was.
        assert_eq!(serie.len(), 5);
    }
    assert!(Serie::new(vec![Scalar::from(1_i64), Scalar::Null]).is_unique());
}

#[test]
fn into_sorted_answers_a_new_serie_under_the_same_field_and_leaves_this_one() {
    let column = int64_column(vec![Some(3), None, Some(1)]);
    let sorted = column.into_sorted(SortOptions::default()).expect("sorted");
    assert_eq!(
        sorted.rows().to_vec(),
        vec![Scalar::from(1_i64), Scalar::from(3_i64), Scalar::Null]
    );
    assert_eq!(sorted.field(), column.field());
    assert!(sorted.is_sorted(SortOptions::default()));
    assert_eq!(column.scalar(0).expect("in range"), Scalar::from(3_i64));

    let run = Serie::new(vec![Scalar::from("b"), Scalar::from("a")]);
    let sorted = run.into_sorted(SortOptions::default()).expect("sorted");
    assert_eq!(
        sorted.rows().to_vec(),
        vec![Scalar::from("a"), Scalar::from("b")]
    );
    assert!(sorted.field().is_none());
}

#[test]
fn into_reversed_reverses_both_leaves_with_their_absences() {
    let column = int64_column(vec![Some(1), None, Some(3)]);
    assert_eq!(
        column.into_reversed().rows().to_vec(),
        vec![Scalar::from(3_i64), Scalar::Null, Scalar::from(1_i64)]
    );
    let run = Serie::new(i64s(&[1, 2]));
    assert_eq!(run.into_reversed().rows().to_vec(), i64s(&[2, 1]));
    assert!(Serie::new(Vec::<Scalar>::new()).into_reversed().is_empty());
}

#[test]
fn into_taken_reads_indices_of_any_integer_width_and_refuses_what_names_no_row() {
    let column = int64_column(vec![Some(10), Some(20), Some(30)]);
    let run = Serie::new(i64s(&[10, 20, 30]));
    let by_uint32 = Serie::new(u32s(&[2, 0, 2]));
    let by_int64 = Serie::new(i64s(&[2, 0, 2]));
    let by_column = int64_column(vec![Some(2), Some(0), Some(2)]);
    for serie in [&column, &run] {
        for indices in [&by_uint32, &by_int64, &by_column] {
            let taken = serie.into_taken(indices).expect("taken");
            assert_eq!(taken.rows().to_vec(), i64s(&[30, 10, 30]));
            assert_eq!(taken.field(), serie.field());
        }
        assert!(
            serie
                .into_taken(&Serie::new(Vec::new()))
                .expect("none")
                .is_empty()
        );
        for (indices, what) in [
            (Serie::new(vec![Scalar::from(3_i64)]), "past the end"),
            (Serie::new(vec![Scalar::from(-1_i64)]), "negative"),
            (Serie::new(vec![Scalar::Null]), "absent"),
            (Serie::new(vec![Scalar::from("0")]), "text"),
            (int64_column(vec![None]), "an absent column row"),
        ] {
            let refused = serie.into_taken(&indices).unwrap_err().to_string();
            assert!(
                refused.contains("names no row of the 3"),
                "{what}: {refused}"
            );
        }
    }
}

#[test]
fn into_filtered_keeps_what_the_mask_keeps_and_an_absent_mask_row_keeps_nothing() {
    let column = int64_column(vec![Some(10), Some(20), Some(30)]);
    let run = Serie::new(i64s(&[10, 20, 30]));
    let mask_run = Serie::new(vec![Scalar::from(true), Scalar::Null, Scalar::from(true)]);
    let mask_column = Serie::from_arrow_array(
        None,
        Arc::new(BooleanArray::from(vec![Some(true), None, Some(true)])) as ArrayRef,
        ArrowCastOptions::new(),
    )
    .expect("a boolean column");
    for serie in [&column, &run] {
        for mask in [&mask_run, &mask_column] {
            let kept = serie.into_filtered(mask).expect("filtered");
            assert_eq!(kept.rows().to_vec(), i64s(&[10, 30]));
            assert_eq!(kept.field(), serie.field());
        }
        let refused = serie
            .into_filtered(&Serie::new(vec![Scalar::from(true)]))
            .unwrap_err()
            .to_string();
        assert!(
            refused.contains("a mask of 1 rows cannot filter the 3 rows"),
            "{refused}"
        );
        let refused = serie
            .into_filtered(&Serie::new(i64s(&[1, 1, 1])))
            .unwrap_err()
            .to_string();
        assert!(
            refused.contains("neither a boolean nor absent"),
            "{refused}"
        );
    }
}

#[test]
fn partition_by_groups_in_first_occurrence_order_and_slices_sorted_keys_zero_copy() {
    let prices = int64_column(vec![Some(1), Some(2), Some(3), Some(4)]);
    let sorted_keys = utf8_column(vec![Some("a"), Some("a"), Some("b"), None]);
    let groups = prices.partition_by(&sorted_keys).expect("groups");
    assert_eq!(groups.len(), 3);
    assert_eq!(groups[0].0, Scalar::from("a"));
    assert_eq!(groups[0].1.rows().to_vec(), i64s(&[1, 2]));
    assert_eq!(groups[1].0, Scalar::from("b"));
    assert_eq!(groups[1].1.rows().to_vec(), i64s(&[3]));
    assert_eq!(groups[2].0, Scalar::Null);
    assert_eq!(groups[2].1.rows().to_vec(), i64s(&[4]));
    // Sorted keys: every group is a slice sharing the column's buffer.
    let whole = prices.require_arrow_array().expect("a column").to_data();
    let held = whole.buffers()[0].as_ptr() as usize..whole.buffers()[0].as_ptr() as usize + 32;
    for (_, group) in &groups {
        let group_data = group.require_arrow_array().expect("a column").to_data();
        assert!(held.contains(&(group_data.buffers()[0].as_ptr() as usize)));
    }

    let keys = utf8_column(vec![Some("b"), Some("a"), Some("b"), Some("a")]);
    let groups = prices.partition_by(&keys).expect("groups");
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].0, Scalar::from("b"));
    assert_eq!(groups[0].1.rows().to_vec(), i64s(&[1, 3]));
    assert_eq!(groups[1].0, Scalar::from("a"));
    assert_eq!(groups[1].1.rows().to_vec(), i64s(&[2, 4]));

    // A run partitions the same way, by a run of keys.
    let run = Serie::new(i64s(&[1, 2, 3, 4]));
    let run_keys = Serie::new(vec![
        Scalar::from("b"),
        Scalar::from("a"),
        Scalar::from("b"),
        Scalar::Null,
    ]);
    let groups = run.partition_by(&run_keys).expect("groups");
    assert_eq!(groups.len(), 3);
    assert_eq!(groups[2].0, Scalar::Null);
    assert_eq!(groups[2].1.rows().to_vec(), i64s(&[4]));

    let refused = prices
        .partition_by(&Serie::new(i64s(&[1])))
        .unwrap_err()
        .to_string();
    assert!(
        refused.contains("1 keys cannot partition the 4 rows"),
        "{refused}"
    );
    assert!(
        Serie::new(Vec::<Scalar>::new())
            .partition_by(&Serie::new(Vec::new()))
            .expect("no group")
            .is_empty()
    );
}

#[test]
fn partition_by_paths_keys_a_record_by_the_run_of_its_cells() {
    let quotes = quotes(&[("XNAS", 1), ("XNYS", 2), ("XNAS", 1), ("XNAS", 3)]);
    let venue: FieldPath = "venue".parse().expect("a path");
    let price: FieldPath = "price".parse().expect("a path");
    let groups = quotes
        .partition_by_paths(&[venue.clone(), price.clone()])
        .expect("groups");
    assert_eq!(groups.len(), 3);
    assert_eq!(
        groups[0].0,
        Scalar::from_sequence([Scalar::from("XNAS"), Scalar::from(1_i64)])
    );
    assert_eq!(groups[0].1.len(), 2);
    assert_eq!(groups[0].1.field(), quotes.field());
    assert_eq!(
        groups[1].0,
        Scalar::from_sequence([Scalar::from("XNYS"), Scalar::from(2_i64)])
    );
    let by_venue = quotes.partition_by_paths(&[venue]).expect("groups");
    assert_eq!(by_venue.len(), 2);
    assert_eq!(by_venue[0].0, Scalar::from_sequence([Scalar::from("XNAS")]));
    assert_eq!(by_venue[0].1.len(), 3);
    // One child is the same ask through `child`.
    let by_child = quotes
        .partition_by(quotes.child("venue").expect("a child"))
        .expect("groups");
    assert_eq!(by_child[0].0, Scalar::from("XNAS"));
    assert_eq!(by_child[0].1, by_venue[0].1);

    let refused = quotes.partition_by_paths(&[]).unwrap_err().to_string();
    assert!(refused.contains("partitions by no path"), "{refused}");
    let missing: FieldPath = "tier".parse().expect("a path");
    let refused = quotes
        .partition_by_paths(&[missing])
        .unwrap_err()
        .to_string();
    assert!(
        refused.contains("tier reaches no column of quote"),
        "{refused}"
    );
    let refused = Serie::new(i64s(&[1]))
        .partition_by_paths(&[price])
        .unwrap_err()
        .to_string();
    assert!(
        refused.contains("a schema-free run partitions by no path"),
        "{refused}"
    );
}

#[test]
fn memory_size_counts_a_column_as_its_slice_and_a_run_as_its_values() {
    let column = int64_column((0..1_000).map(Some).collect());
    let whole = column.memory_size();
    assert!(whole >= 8_000, "{whole}");
    let window = column.slice(0, 10).expect("a window");
    assert!(
        window.memory_size() < whole / 10,
        "{}",
        window.memory_size()
    );
    let run = Serie::new(i64s(&[1, 2, 3]));
    assert!(run.memory_size() > 0);
    assert_eq!(Serie::new(Vec::<Scalar>::new()).memory_size(), 0);
}

#[test]
fn as_sorted_rewrites_a_primitive_column_in_place_gathering_its_absences() {
    for (options, expected) in [
        (
            SortOptions::default(),
            vec![Some(1), Some(2), Some(3), None, None],
        ),
        (
            SortOptions::descending(),
            vec![Some(3), Some(2), Some(1), None, None],
        ),
        (
            SortOptions::ascending().with_nulls_first(true),
            vec![None, None, Some(1), Some(2), Some(3)],
        ),
        (
            SortOptions::descending().with_nulls_first(true),
            vec![None, None, Some(3), Some(2), Some(1)],
        ),
    ] {
        let mut column = int64_column(vec![Some(3), None, Some(1), None, Some(2)]);
        let field = column.field().cloned();
        column.as_sorted(options).expect("sorted in place");
        let leaf = column.as_int64().expect("still an int64 column");
        let read: Vec<Option<i64>> = (0..5).map(|index| leaf.value(index)).collect();
        assert_eq!(read, expected, "{options}");
        assert_eq!(column.null_count(), 2);
        assert_eq!(column.field().cloned(), field);
        assert!(column.is_sorted(options));
        assert_eq!(column.into_arrow_array().expect("a column").null_count(), 2);
    }
    // No absence: the slice alone.
    let mut column = int64_column(vec![Some(3), Some(1), Some(2)]);
    column
        .as_sorted(SortOptions::default())
        .expect("sorted")
        .as_reversed()
        .expect("reversed");
    assert_eq!(column.rows().to_vec(), i64s(&[3, 2, 1]));
}

#[test]
fn as_sorted_on_a_shared_column_copies_once_and_leaves_the_other_holder_alone() {
    let column = int64_column(vec![Some(2), Some(1)]);
    let mut other = column.clone();
    other.as_sorted(SortOptions::default()).expect("sorted");
    assert_eq!(other.rows().to_vec(), i64s(&[1, 2]));
    assert_eq!(column.rows().to_vec(), i64s(&[2, 1]));
}

#[test]
fn every_as_write_brings_a_run_or_a_kernel_leaf_into_the_state_and_chains() {
    let mut run = Serie::new(vec![
        Scalar::from("b"),
        Scalar::Null,
        Scalar::from("a"),
        Scalar::from("b"),
    ]);
    run.as_sorted(SortOptions::default())
        .expect("sorted")
        .as_unique()
        .expect("unique")
        .as_reversed()
        .expect("reversed");
    assert_eq!(
        run.rows().to_vec(),
        vec![Scalar::Null, Scalar::from("b"), Scalar::from("a")]
    );
    run.as_taken(&Serie::new(u32s(&[2, 1])))
        .expect("taken")
        .as_filtered(&Serie::new(vec![Scalar::from(false), Scalar::from(true)]))
        .expect("filtered");
    assert_eq!(run.rows().to_vec(), vec![Scalar::from("b")]);

    let mut strings = utf8_column(vec![Some("b"), None, Some("a"), Some("b")]);
    let field = strings.field().cloned();
    strings
        .as_sorted(SortOptions::default())
        .expect("sorted")
        .as_unique()
        .expect("unique")
        .as_reversed()
        .expect("reversed");
    assert_eq!(
        strings.rows().to_vec(),
        vec![Scalar::Null, Scalar::from("b"), Scalar::from("a")]
    );
    assert_eq!(strings.field().cloned(), field);
    strings
        .as_taken(&Serie::new(u32s(&[2, 1])))
        .expect("taken")
        .as_filtered(&Serie::new(vec![Scalar::from(false), Scalar::from(true)]))
        .expect("filtered");
    assert_eq!(strings.rows().to_vec(), vec![Scalar::from("b")]);
    assert_eq!(strings.field().cloned(), field);
}

#[test]
fn a_refused_write_leaves_the_serie_as_it_was() {
    let mut column = int64_column(vec![Some(2), Some(1)]);
    assert!(column.as_taken(&Serie::new(u32s(&[5]))).is_err());
    assert!(column.as_filtered(&Serie::new(Vec::new())).is_err());
    assert_eq!(column.rows().to_vec(), i64s(&[2, 1]));
}

#[test]
fn as_reversed_reverses_a_primitive_column_in_place_with_its_validity() {
    let mut column = int64_column(vec![Some(1), None, None, Some(4), Some(5)]);
    column.as_reversed().expect("reversed");
    let leaf = column.as_int64().expect("an int64 column");
    let read: Vec<Option<i64>> = (0..5).map(|index| leaf.value(index)).collect();
    assert_eq!(read, vec![Some(5), Some(4), None, None, Some(1)]);
    assert_eq!(column.null_count(), 2);
}

/// Every nested, encoded and viewed layout answers the ladder's lower
/// rungs: the row format, and the values' order.
fn nested_columns() -> Vec<Serie> {
    let records = quotes(&[("XNYS", 2), ("XNAS", 2), ("XNAS", 1), ("XNYS", 2)]);
    let item = Arc::new(ArrowField::new("item", ArrowDataType::Int64, true));
    let lists: ArrayRef = Arc::new(ListArray::new(
        item,
        OffsetBuffer::from_lengths([2, 1, 2, 1]),
        Arc::new(Int64Array::from(vec![2, 3, 1, 2, 3, 2])),
        None,
    ));
    let lists =
        Serie::from_arrow_array(None, lists, ArrowCastOptions::new()).expect("a list column");
    let keys = Int8Array::from(vec![Some(1), Some(0), Some(1), Some(2)]);
    let values: ArrayRef = Arc::new(StringArray::from(vec!["XNAS", "XNYS", "XPAR"]));
    let dictionary: ArrayRef =
        Arc::new(DictionaryArray::<Int8Type>::try_new(keys, values).expect("a dictionary"));
    let dictionary = Serie::from_arrow_array(None, dictionary, ArrowCastOptions::new())
        .expect("a dictionary column");
    let views: ArrayRef = Arc::new(StringViewArray::from(vec![
        "XNYS",
        "XNAS",
        "a view longer than twelve bytes",
        "XNYS",
    ]));
    let views =
        Serie::from_arrow_array(None, views, ArrowCastOptions::new()).expect("a view column");
    let booleans: ArrayRef = Arc::new(BooleanArray::from(vec![
        Some(true),
        Some(false),
        None,
        Some(true),
    ]));
    let booleans =
        Serie::from_arrow_array(None, booleans, ArrowCastOptions::new()).expect("a boolean column");
    vec![records, lists, dictionary, views, booleans]
}

#[test]
fn every_layout_answers_every_verb_through_the_ladder() {
    for column in nested_columns() {
        let what = format!("{column:?}");
        let order = column.sort_indices(SortOptions::default()).expect(&what);
        assert_eq!(order.len(), 4, "{what}");
        let sorted = column.into_sorted(SortOptions::default()).expect(&what);
        assert!(sorted.is_sorted(SortOptions::default()), "{what}: {sorted}");
        assert_eq!(sorted.field(), column.field(), "{what}");
        // The sort agrees with the values' own order, absences last.
        let mut by_value = column.rows().to_vec();
        by_value.sort_by(|left, right| match (left.is_null(), right.is_null()) {
            (true, true) => std::cmp::Ordering::Equal,
            (true, false) => std::cmp::Ordering::Greater,
            (false, true) => std::cmp::Ordering::Less,
            (false, false) => left.cmp(right),
        });
        assert_eq!(sorted.rows().to_vec(), by_value, "{what}");
        let descending = column.into_sorted(SortOptions::descending()).expect(&what);
        assert!(descending.is_sorted(SortOptions::descending()), "{what}");
        assert!(!column.is_unique(), "{what}");
        assert_eq!(column.unique_count(), 3, "{what}");
        let unique = column.into_unique().expect(&what);
        assert_eq!(unique.len(), 3, "{what}");
        assert!(unique.is_unique(), "{what}");
        assert_eq!(
            unique.scalar(0).expect("a row"),
            column.scalar(0).expect("a row")
        );
        assert_eq!(
            column.into_reversed().scalar(0).expect("a row"),
            column.scalar(3).expect("a row"),
            "{what}"
        );
        let groups = column.partition_by(&column).expect(&what);
        assert_eq!(groups.len(), 3, "{what}");
        assert_eq!(groups[0].1.len(), 2, "{what}");
        let mut written = column.clone();
        written
            .as_sorted(SortOptions::default())
            .expect(&what)
            .as_unique()
            .expect(&what)
            .as_reversed()
            .expect(&what);
        assert_eq!(written.len(), 3, "{what}");
        assert_eq!(written.field(), column.field(), "{what}");
        // Ascending with absences last, reversed, is descending with
        // absences first.
        assert!(
            written.is_sorted(SortOptions::descending().with_nulls_first(true)),
            "{what}: {written}"
        );
        assert!(column.memory_size() > 0, "{what}");
    }
}

/// The four orderings `SortOptions` states.
const ORDERINGS: [SortOptions; 4] = [
    SortOptions::ascending(),
    SortOptions::ascending().with_nulls_first(true),
    SortOptions::descending(),
    SortOptions::descending().with_nulls_first(true),
];

/// Pin that `column` and the run of its own rows answer every ordering,
/// uniqueness and grouping verb alike under all four orderings: one order,
/// whichever rung of the ladder reads it.
fn agrees_with_its_run(column: &Serie) {
    let field = column.field().expect("a column").clone();
    let run = Serie::new(column.rows().into_owned());
    let what = format!("{column}");
    for options in ORDERINGS {
        let order = column.sort_indices(options).expect(&what);
        let run_order = run.sort_indices(options).expect(&what);
        assert_eq!(
            order.rows(),
            run_order.rows(),
            "{what}{options}: sort_indices"
        );
        let sorted = column.into_sorted(options).expect(&what);
        let run_sorted = run.into_sorted(options).expect(&what);
        assert_eq!(
            sorted.rows(),
            run_sorted.rows(),
            "{what}{options}: into_sorted"
        );
        assert!(sorted.is_sorted(options), "{what}{options}: {sorted}");
        assert!(
            run_sorted.is_sorted(options),
            "{what}{options}: {run_sorted}"
        );
        assert_eq!(
            column.is_sorted(options),
            run.is_sorted(options),
            "{what}{options}: is_sorted"
        );
        // The run's order, laid out as a column, is one the column calls
        // sorted.
        let relaid = Serie::from_scalars(field.clone(), run_sorted.rows().into_owned())
            .expect("the run's rows under the column's field");
        assert!(relaid.is_sorted(options), "{what}{options}: {relaid}");
        let mut in_place = column.clone();
        in_place.as_sorted(options).expect(&what);
        assert_eq!(
            in_place.rows(),
            run_sorted.rows(),
            "{what}{options}: as_sorted"
        );
        let mut run_in_place = run.clone();
        run_in_place.as_sorted(options).expect(&what);
        assert_eq!(run_in_place.rows(), run_sorted.rows(), "{what}{options}");
    }
    assert_eq!(column.unique_count(), run.unique_count(), "{what}");
    assert_eq!(column.is_unique(), run.is_unique(), "{what}");
    assert_eq!(
        column.into_unique().expect(&what).rows(),
        run.into_unique().expect(&what).rows(),
        "{what}: into_unique"
    );
    let groups = |serie: &Serie| -> Vec<(Scalar, Vec<Scalar>)> {
        serie
            .partition_by(serie)
            .expect("grouped by itself")
            .into_iter()
            .map(|(key, rows)| (key, rows.rows().into_owned()))
            .collect()
    };
    assert_eq!(groups(column), groups(&run), "{what}: partition_by");
}

/// `values` laid out under `field`, each through the field's own contract.
fn column_of(field: Field, values: &[Option<&str>]) -> Serie {
    let rows: Vec<Scalar> = values
        .iter()
        .map(|value| match value {
            Some(text) => field.scalar(*text).expect("a value the field accepts"),
            None => Scalar::Null,
        })
        .collect();
    Serie::from_scalars(field, rows).expect("rows the field accepts")
}

#[test]
fn a_float_column_reads_every_nan_payload_as_the_one_nan_its_rows_hold() {
    let negative_nan = f64::from_bits(f64::NAN.to_bits() | (1 << 63));
    let payload_nan = f64::from_bits(f64::NAN.to_bits() | 1);
    let array: ArrayRef = Arc::new(arrow_array::Float64Array::from(vec![
        Some(1.0),
        Some(negative_nan),
        Some(f64::NAN),
        None,
        Some(-1.0),
        Some(payload_nan),
        Some(-0.0),
        Some(0.0),
    ]));
    let column =
        Serie::from_arrow_array(None, array, ArrowCastOptions::new()).expect("a float column");
    agrees_with_its_run(&column);
    // Three NaN payloads are one value, as the rows read them.
    assert_eq!(column.unique_count(), 6);
    let sorted = column
        .into_sorted(SortOptions::default())
        .expect("sorted")
        .rows()
        .into_owned();
    assert!(
        sorted[4..7]
            .iter()
            .all(|row| row.as_f64().is_some_and(f64::is_nan))
    );
    assert_eq!(sorted[7], Scalar::Null);
    let groups = column.partition_by(&column).expect("grouped");
    assert_eq!(groups.len(), 6);
    // A uniquely held float column sorts in place to the same order.
    let mut held = column.clone();
    held.as_sorted(SortOptions::default()).expect("sorted");
    assert_eq!(held.rows().to_vec(), sorted);

    let narrow: ArrayRef = Arc::new(arrow_array::Float32Array::from(vec![
        f32::from_bits(f32::NAN.to_bits() | 1),
        2.0,
        f32::NAN,
        -f32::NAN,
    ]));
    let narrow =
        Serie::from_arrow_array(None, narrow, ArrowCastOptions::new()).expect("a float column");
    agrees_with_its_run(&narrow);
    assert_eq!(narrow.unique_count(), 2);
}

#[test]
fn a_leaf_whose_value_order_is_not_its_stored_order_sorts_as_its_values() {
    // A version orders by its numbers, never its text.
    let releases = column_of(
        DataType::Version.required_field("release"),
        &[Some("1.10.0"), Some("1.9.0"), Some("1.2.0"), Some("1.9.0")],
    );
    agrees_with_its_run(&releases);
    let sorted = releases
        .into_sorted(SortOptions::default())
        .expect("sorted");
    assert_eq!(
        sorted.rows().to_vec(),
        ["1.2.0", "1.9.0", "1.9.0", "1.10.0"]
            .map(|text| DataType::Version.scalar(text).expect("a version"))
            .to_vec()
    );
    // windows-1252 text orders by its characters, never its bytes: `€` is
    // 0x80 and `é` 0xE9, but U+20AC follows U+00E9.
    let names = column_of(
        Field::new("name", DataType::cp1252(), true),
        &[Some("€"), Some("é"), None, Some("a"), Some("é")],
    );
    agrees_with_its_run(&names);
    assert_eq!(
        names
            .into_sorted(SortOptions::default())
            .expect("sorted")
            .rows()
            .to_vec(),
        vec![
            Scalar::from("a"),
            Scalar::from("é"),
            Scalar::from("é"),
            Scalar::from("€"),
            Scalar::Null
        ]
    );
    for (field, values) in [
        (
            Field::new("urn", DataType::Urn, true),
            vec![
                Some("urn:isbn:0451450523"),
                Some("urn:example:a"),
                None,
                Some("urn:ietf:rfc:2648"),
                Some("urn:example:a"),
            ],
        ),
        (
            Field::new("media", DataType::MediaType, true),
            vec![
                Some("text/plain"),
                Some("application/json"),
                Some("text/csv"),
                None,
                Some("text/plain"),
            ],
        ),
        (
            Field::new("mime", DataType::MimeType, false),
            vec![
                Some("text/plain"),
                Some("application/json"),
                Some("image/png"),
                Some("text/plain"),
            ],
        ),
        (
            Field::new("zone", DataType::Timezone, false),
            vec![
                Some("Europe/Paris"),
                Some("America/New_York"),
                Some("UTC"),
                Some("Asia/Tokyo"),
            ],
        ),
        (
            Field::new("currency", DataType::Ccy, false),
            vec![Some("USD"), Some("EUR"), Some("JPY"), Some("EUR")],
        ),
        (
            Field::new("state", DataType::State, true),
            vec![Some("FILLED"), Some("NEW"), None, Some("CANCELED")],
        ),
        (
            Field::new("id", DataType::Uuid, false),
            vec![
                Some("ffffffff-0000-0000-0000-000000000000"),
                Some("00000000-0000-0000-0000-0000000000ff"),
                Some("0f000000-0000-0000-0000-000000000000"),
            ],
        ),
        (
            Field::new("code", DataType::fixed_utf8(3).expect("a width"), false),
            vec![Some("ab"), Some("a"), Some("abc"), Some("b")],
        ),
        (
            Field::new("venue", DataType::utf8(), true),
            vec![Some("é"), Some("€"), None, Some("z")],
        ),
    ] {
        agrees_with_its_run(&column_of(field, &values));
    }
}

#[test]
fn a_registered_code_whose_column_pads_it_is_one_value_with_the_trimmed_code() {
    // A column stamped as currencies by a foreign writer lands as it came,
    // padding and all, and its value is the code the padding trims to.
    let field = ArrowField::new("currency", ArrowDataType::Utf8, false).with_metadata(
        [
            ("ARROW:extension:name".to_owned(), "yggdryl.ccy".to_owned()),
            ("ARROW:extension:metadata".to_owned(), String::new()),
        ]
        .into(),
    );
    let schema = Arc::new(arrow_schema::Schema::new(vec![field]));
    let codes: ArrayRef = Arc::new(StringArray::from(vec!["USD\0", "EUR", "USD"]));
    let batch = arrow_array::RecordBatch::try_new(schema, vec![codes]).expect("a batch");
    let records =
        Serie::from_arrow_batch(None, &batch, ArrowCastOptions::new()).expect("a record column");
    let currencies = records
        .child("currency")
        .expect("the currency column")
        .clone();
    let stored = currencies.into_arrow_array().expect("a column");
    let stored = stored.as_any().downcast_ref::<StringArray>().expect("text");
    assert_eq!(stored.value(0), "USD\0");
    agrees_with_its_run(&currencies);
    assert_eq!(currencies.unique_count(), 2);
    assert_eq!(
        currencies.partition_by(&currencies).expect("grouped").len(),
        2
    );
}

/// The record field `q{a: int64?, b: utf8}`.
fn nullable_record() -> Field {
    Field::new(
        "q",
        DataType::from(
            StructType::from_fields([
                Field::new("a", DataType::Int64, true),
                Field::new("b", DataType::utf8(), false),
            ])
            .expect("two children"),
        ),
        true,
    )
}

#[test]
fn a_nested_absence_goes_where_the_options_put_a_top_level_one_on_every_rung() {
    let record = |a: Option<i64>, b: &str| {
        Scalar::from_sequence([a.map_or(Scalar::Null, Scalar::from), Scalar::from(b)])
    };
    let records = Serie::from_scalars(
        nullable_record(),
        [
            record(None, "x"),
            record(Some(1), "y"),
            Scalar::Null,
            record(Some(1), "x"),
            record(None, "y"),
        ],
    )
    .expect("record rows");
    agrees_with_its_run(&records);
    // Ascending with nulls last puts the absent child after the present one.
    let sorted = records
        .into_sorted(SortOptions::default())
        .expect("sorted")
        .rows()
        .into_owned();
    assert_eq!(
        sorted,
        vec![
            record(Some(1), "x"),
            record(Some(1), "y"),
            record(None, "x"),
            record(None, "y"),
            Scalar::Null
        ]
    );
    let sorted = records
        .into_sorted(SortOptions::descending().with_nulls_first(true))
        .expect("sorted")
        .rows()
        .into_owned();
    assert_eq!(
        sorted,
        vec![
            Scalar::Null,
            record(None, "y"),
            record(None, "x"),
            record(Some(1), "y"),
            record(Some(1), "x")
        ]
    );

    let item = Arc::new(Field::new("item", DataType::Int64, true));
    let lists = Serie::from_scalars(
        Field::new("legs", DataType::Serie(Arc::clone(&item)), true),
        [
            Scalar::from_sequence([Scalar::from(1_i64), Scalar::Null]),
            Scalar::from_sequence([Scalar::from(1_i64), Scalar::from(2_i64)]),
            Scalar::from_sequence([]),
            Scalar::Null,
            Scalar::from_sequence([Scalar::from(1_i64)]),
            Scalar::from_sequence([Scalar::Null]),
        ],
    )
    .expect("list rows");
    agrees_with_its_run(&lists);

    let entries = |pairs: &[(&str, Option<i64>)]| {
        Scalar::from_mapping(
            pairs
                .iter()
                .map(|(key, value)| (Scalar::from(*key), value.map_or(Scalar::Null, Scalar::from))),
        )
        .expect("a mapping")
    };
    let maps = Serie::from_scalars(
        Field::new(
            "tags",
            DataType::map_of(DataType::utf8(), DataType::Int64, false).expect("a map"),
            true,
        ),
        [
            entries(&[("a", Some(1))]),
            entries(&[("a", None)]),
            entries(&[("a", Some(1)), ("b", Some(2))]),
            Scalar::Null,
            entries(&[("a", None)]),
        ],
    )
    .expect("map rows");
    agrees_with_its_run(&maps);
}

#[test]
fn a_nested_leaf_that_orders_by_value_sends_its_whole_column_to_the_values_order() {
    let root = Field::new(
        "release",
        DataType::from(
            StructType::from_fields([
                Field::new("version", DataType::Version, false),
                Field::new("score", DataType::Float64, true),
            ])
            .expect("two children"),
        ),
        false,
    );
    let row = |version: &str, score: Option<f64>| {
        Scalar::from_sequence([
            DataType::Version.scalar(version).expect("a version"),
            score.map_or(Scalar::Null, Scalar::from),
        ])
    };
    let releases = Serie::from_scalars(
        root,
        [
            row("1.10.0", Some(1.0)),
            row("1.9.0", None),
            row("1.9.0", Some(f64::NAN)),
            row("1.2.0", Some(2.0)),
        ],
    )
    .expect("record rows");
    agrees_with_its_run(&releases);

    // A list of floats holding a foreign NaN deep in its items.
    let item = Arc::new(ArrowField::new("item", ArrowDataType::Float64, true));
    let negative_nan = f64::from_bits(f64::NAN.to_bits() | (1 << 63));
    let lists: ArrayRef = Arc::new(ListArray::new(
        item,
        OffsetBuffer::from_lengths([1, 1, 2]),
        Arc::new(arrow_array::Float64Array::from(vec![
            negative_nan,
            f64::NAN,
            1.0,
            negative_nan,
        ])),
        None,
    ));
    let lists = Serie::from_arrow_array(None, lists, ArrowCastOptions::new()).expect("a list");
    agrees_with_its_run(&lists);
    assert_eq!(lists.unique_count(), 2);
}

#[test]
fn every_other_leaf_agrees_with_the_run_of_its_rows() {
    for column in nested_columns() {
        agrees_with_its_run(&column);
    }
    agrees_with_its_run(&int64_column(vec![
        Some(3),
        None,
        Some(1),
        Some(3),
        None,
        Some(-2),
    ]));
    agrees_with_its_run(&utf8_column(vec![Some("b"), None, Some("a"), Some("b")]));
    let flags: ArrayRef = Arc::new(BooleanArray::from(vec![
        Some(true),
        None,
        Some(false),
        Some(true),
        None,
        Some(false),
    ]));
    agrees_with_its_run(
        &Serie::from_arrow_array(None, flags, ArrowCastOptions::new()).expect("booleans"),
    );
}

/// A boolean column of `values`, holding its bitmaps alone.
fn boolean_column(values: Vec<Option<bool>>) -> Serie {
    let array: ArrayRef = Arc::new(BooleanArray::from(values));
    Serie::from_arrow_array(
        Some(&Field::new("flag", DataType::Boolean, true)),
        array,
        ArrowCastOptions::new(),
    )
    .expect("a boolean column")
}

/// Where a boolean column's values bitmap starts.
fn bits_at(column: &Serie) -> *const u8 {
    column
        .as_boolean()
        .expect("a boolean column")
        .values()
        .inner()
        .as_ptr()
}

/// Where an int64 column's values buffer starts.
fn values_at(column: &Serie) -> *const i64 {
    column
        .as_int64()
        .expect("an int64 column")
        .values()
        .as_ptr()
}

#[test]
fn a_boolean_column_held_alone_sorts_and_reverses_its_bitmaps_where_they_stand() {
    let values = vec![
        Some(true),
        None,
        Some(false),
        Some(true),
        None,
        Some(false),
        Some(true),
    ];
    for options in ORDERINGS {
        let mut column = boolean_column(values.clone());
        let before = bits_at(&column);
        column.as_sorted(options).expect("sorted in place");
        assert_eq!(bits_at(&column), before, "{options}: the bitmap moved");
        let run = Serie::new(boolean_column(values.clone()).rows().into_owned())
            .into_sorted(options)
            .expect("sorted");
        assert_eq!(column.rows(), run.rows(), "{options}");
        assert_eq!(column.null_count(), 2);
        assert!(column.is_sorted(options), "{options}: {column}");
    }
    let mut column = boolean_column(values.clone());
    let before = bits_at(&column);
    column.as_reversed().expect("reversed in place");
    assert_eq!(bits_at(&column), before);
    let mut reversed = values;
    reversed.reverse();
    assert_eq!(
        column.rows().to_vec(),
        reversed
            .into_iter()
            .map(|value| value.map_or(Scalar::Null, Scalar::from))
            .collect::<Vec<_>>()
    );
    // A shared bitmap is copied once, and the other holder keeps its rows.
    let original = boolean_column(vec![Some(true), Some(false)]);
    let mut shared = original.clone();
    shared.as_sorted(SortOptions::default()).expect("sorted");
    assert_eq!(
        shared.rows().to_vec(),
        vec![Scalar::from(false), Scalar::from(true)]
    );
    assert_eq!(
        original.rows().to_vec(),
        vec![Scalar::from(true), Scalar::from(false)]
    );
}

#[test]
fn a_window_of_a_boolean_column_sorts_and_reverses_in_place() {
    let mut column = boolean_column(vec![Some(true), Some(true), None, Some(false), Some(true)]);
    let before = bits_at(&column);
    column
        .window_mut(1, 3)
        .expect("a window")
        .as_sorted(SortOptions::descending().with_nulls_first(true))
        .expect("sorted");
    assert_eq!(bits_at(&column), before);
    assert_eq!(
        column.rows().to_vec(),
        vec![
            Scalar::from(true),
            Scalar::Null,
            Scalar::from(true),
            Scalar::from(false),
            Scalar::from(true)
        ]
    );
    column
        .window_mut(0, 4)
        .expect("a window")
        .as_reversed()
        .expect("reversed");
    assert_eq!(bits_at(&column), before);
    assert_eq!(
        column.rows().to_vec(),
        vec![
            Scalar::from(false),
            Scalar::from(true),
            Scalar::Null,
            Scalar::from(true),
            Scalar::from(true)
        ]
    );
}

#[test]
fn a_primitive_column_with_absent_rows_held_alone_sorts_where_it_stands() {
    for options in ORDERINGS {
        let mut column = int64_column(vec![Some(3), None, Some(1), None, Some(2)]);
        let before = values_at(&column);
        column.as_sorted(options).expect("sorted in place");
        assert_eq!(values_at(&column), before, "{options}: the buffer moved");
        assert!(column.is_sorted(options), "{options}");
        let mut column = int64_column(vec![Some(9), Some(3), None, Some(1), None, Some(0)]);
        let before = values_at(&column);
        column
            .window_mut(1, 4)
            .expect("a window")
            .as_sorted(options)
            .expect("sorted in place");
        assert_eq!(
            values_at(&column),
            before,
            "{options}: the window's buffer moved"
        );
        assert!(
            column.window(1, 4).expect("a window").is_sorted(options),
            "{options}: {column}"
        );
        assert_eq!(column.scalar(0).expect("a row"), Scalar::from(9_i64));
        assert_eq!(column.scalar(5).expect("a row"), Scalar::from(0_i64));
    }
}
