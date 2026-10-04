//! `rust/src/serie/order.rs`: the ordering, uniqueness and grouping verbs
//! every leaf answers, the reads leaving the serie as it was and the
//! `as_*` writes bringing it into the state in place.

use std::cmp::Ordering;
use std::sync::Arc;

use arrow_array::types::Int8Type;
use arrow_array::{
    Array, ArrayRef, BooleanArray, DictionaryArray, Int8Array, Int64Array, ListArray, StringArray,
    StringViewArray,
};
use arrow_buffer::{NullBuffer, OffsetBuffer};
use arrow_schema::{DataType as ArrowDataType, Field as ArrowField};
use yggdryl::expression::{IntoOrderings, Ordering as OrderBy, Selector};
use yggdryl::{
    ArrowCastOptions, DataType, Error, Field, FieldPath, Scalar, Serie, SerieReader, SerieWindows,
    SortOptions, StructType, TimeUnit, Timezone,
};

/// `field` without the order its root declares: what a sort leaves equal
/// to the field it sorted, the `SORT:by` it wrote aside.
fn without_order(field: Option<&Field>) -> Option<Field> {
    field.map(|field| field.clone().with_metadata_removed("SORT:by"))
}

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
        assert_eq!(
            without_order(sorted.field()),
            column.field().cloned(),
            "{what}"
        );
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
        assert_eq!(
            without_order(written.field()),
            column.field().cloned(),
            "{what}"
        );
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
fn a_record_with_a_value_ordered_leaf_compares_child_by_child_and_agrees_with_its_run() {
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

// ---------------------------------------------------------------------------
// window_by: the rows cut where the key changes
// ---------------------------------------------------------------------------

const MINUTE_NS: i64 = 60_000_000_000;

fn utc_ns() -> DataType {
    DataType::DateTime64 {
        unit: TimeUnit::Nanosecond,
        timezone: Timezone::UTC,
    }
}

/// The record `quote{venue, count, ts}`, its root nullable so a row may be
/// absent.
fn quote_field() -> Field {
    StructType::from_fields([
        DataType::utf8().nullable_field("venue"),
        DataType::Int64.required_field("count"),
        utc_ns().nullable_field("ts"),
    ])
    .map(DataType::from)
    .expect("three children")
    .nullable_field("quote")
}

/// One quote row: its venue, its count and its instant in minutes.
fn quote(venue: Option<&str>, count: i64, minutes: i64) -> Scalar {
    Scalar::from_sequence([
        venue.map_or(Scalar::Null, Scalar::from),
        Scalar::from(count),
        Scalar::from(minutes * MINUTE_NS),
    ])
}

fn quote_column(rows: Vec<Scalar>) -> Serie {
    let field = quote_field();
    let rows = rows
        .into_iter()
        .map(|row| field.scalar(row).expect("a quote row"))
        .collect::<Vec<_>>();
    Serie::from_scalars(field, rows).expect("quote rows")
}

/// The venues XNAS, XNAS, XNYS, XNAS at minutes 0, 14, 15 and 31.
fn venue_runs() -> Serie {
    quote_column(vec![
        quote(Some("XNAS"), 1, 0),
        quote(Some("XNAS"), 2, 14),
        quote(Some("XNYS"), 3, 15),
        quote(Some("XNAS"), 4, 31),
    ])
}

/// Each window as its key, its offset and its length.
fn cuts(windows: &SerieWindows<'_>) -> Vec<(Scalar, usize, usize)> {
    windows
        .iter()
        .map(|(key, window)| (key, window.offset(), window.len()))
        .collect()
}

fn one_cell(value: impl Into<Scalar>) -> Scalar {
    Scalar::from_sequence([value.into()])
}

/// The window refusal's path and reason.
fn invalid_record(error: Error) -> (String, String) {
    match error {
        Error::InvalidRecord { path, reason } => (path.to_string(), reason.to_string()),
        other => panic!("expected an invalid record, got {other}"),
    }
}

/// Pin every refusal `window_by` answers before a row is read, under
/// `sorted`: a sorted window is refused exactly where an unsorted one is.
fn window_by_refuses(sorted: bool) {
    let quotes = venue_runs();
    let empty = quote_column(Vec::new());
    for serie in [&quotes, &empty] {
        let what = format!("{} rows, sorted {sorted}", serie.len());
        // The text's own parse error, before anything else.
        let parsed = "venue,".parse::<Selector>().unwrap_err().to_string();
        assert_eq!(
            serie.window_by("venue,", sorted).unwrap_err().to_string(),
            parsed,
            "{what}"
        );
        // A key stating no column.
        for refused in [
            serie.window_by("*", sorted),
            serie.window_by(Selector::new(Vec::new()), sorted),
        ] {
            assert_eq!(
                invalid_record(refused.unwrap_err()),
                (
                    "quote".to_owned(),
                    "expected at least one column to window by, got an empty match key".to_owned()
                ),
                "{what}"
            );
        }
        // An unnest is one row per element, never one key per row.
        let refused = serie
            .window_by("unnest(items)", sorted)
            .unwrap_err()
            .to_string();
        assert!(refused.contains("in a key"), "{what}: {refused}");
        // A column the record does not hold, named; the binder's own text.
        let root = SerieReader::root_of(serie.field().expect("a column")).expect("a root");
        let unknown = "tier".parse::<Selector>().expect("a selector");
        let refused = serie.window_by(&unknown, sorted).unwrap_err().to_string();
        assert_eq!(refused, unknown.bind(&root).unwrap_err().to_string());
        assert!(refused.contains("tier"), "{what}: {refused}");
        // A period's step is a positive literal.
        for text in ["minutes(ts, 0)", "minutes(ts, count)"] {
            let selector = text.parse::<Selector>().expect("a selector");
            let refused = serie.window_by(&selector, sorted).unwrap_err().to_string();
            assert_eq!(
                refused,
                selector.bind(&root).unwrap_err().to_string(),
                "{what}: {text}"
            );
        }
    }

    // A run has no field for a term to read.
    for run in [Serie::new(i64s(&[1, 2])), Serie::new(Vec::new())] {
        assert_eq!(
            invalid_record(run.window_by("price", sorted).unwrap_err()),
            (
                "$".to_owned(),
                "a schema-free run windows by no term".to_owned()
            ),
            "sorted {sorted}"
        );
    }

    // Two columns that fold together are ambiguous to a folded name.
    let field = StructType::from_fields([
        DataType::Int64.required_field("a"),
        DataType::Int64.required_field("A"),
    ])
    .map(DataType::from)
    .expect("two children")
    .required_field("pair");
    let pair = Serie::from_scalars(
        field.clone(),
        [Scalar::from_sequence([
            Scalar::from(1_i64),
            Scalar::from(2_i64),
        ])],
    )
    .expect("a pair");
    let root = SerieReader::root_of(&field).expect("a root");
    let selector = "a".parse::<Selector>().expect("a selector");
    let refused = pair.window_by(&selector, sorted).unwrap_err().to_string();
    assert_eq!(refused, selector.bind(&root).unwrap_err().to_string());
    assert!(
        refused.contains("a and A; quote the one meant"),
        "{refused}"
    );
}

#[test]
fn window_by_refuses_a_run_an_empty_key_an_unnest_and_a_term_naming_no_column() {
    window_by_refuses(false);
}

#[test]
fn window_by_sorted_refuses_what_unsorted_refuses_before_any_row() {
    window_by_refuses(true);
}

#[test]
fn window_by_cuts_runs_of_equal_adjacent_keys_in_row_order() {
    let quotes = venue_runs();
    let windows = quotes.window_by("venue", false).expect("windows");
    assert_eq!(windows.len(), 3);
    let drained: Vec<_> = windows.iter().collect();
    assert_eq!(drained.len(), 3);
    assert_eq!(
        cuts(&windows),
        vec![
            (one_cell("XNAS"), 0, 2),
            (one_cell("XNYS"), 2, 1),
            (one_cell("XNAS"), 3, 1),
        ]
    );
    // Every window is a view over the serie; together they cover it, one
    // after another, and no two side by side share a key.
    let mut next = 0;
    for (key, window) in &drained {
        assert!(std::ptr::eq(window.serie(), &quotes));
        assert_eq!(window.offset(), next);
        assert!(!window.is_empty());
        next += window.len();
        assert_eq!(
            Scalar::from_sequence([window
                .scalar(0)
                .expect("a row")
                .get(0)
                .expect("venue")
                .into_owned()]),
            *key
        );
    }
    assert_eq!(next, quotes.len());
    for pair in drained.windows(2) {
        assert_ne!(pair[0].0, pair[1].0);
    }

    // Two terms key a two-cell run in selector order, the alias naming the
    // second; a period keys its number since the epoch.
    assert_eq!(
        cuts(
            &quotes
                .window_by("venue, minutes(ts, 15) as bucket", false)
                .expect("windows")
        ),
        vec![
            (
                Scalar::from_sequence([Scalar::from("XNAS"), Scalar::from(0_i32)]),
                0,
                2
            ),
            (
                Scalar::from_sequence([Scalar::from("XNYS"), Scalar::from(1_i32)]),
                2,
                1
            ),
            (
                Scalar::from_sequence([Scalar::from("XNAS"), Scalar::from(2_i32)]),
                3,
                1
            ),
        ]
    );
    assert_eq!(
        cuts(&quotes.window_by("minutes(ts, 15)", false).expect("windows")),
        vec![
            (one_cell(0_i32), 0, 2),
            (one_cell(1_i32), 2, 1),
            (one_cell(2_i32), 3, 1),
        ]
    );
    assert_eq!(
        cuts(&quotes.window_by("days(ts)", false).expect("windows")),
        vec![(one_cell(Scalar::date32(0)), 0, 4)]
    );
    // Names fold ASCII case.
    assert_eq!(
        cuts(&quotes.window_by("VENUE", false).expect("windows")),
        cuts(&quotes.window_by("venue", false).expect("windows"))
    );
    // One key on every row is one window; a key per row a window per row.
    assert_eq!(
        quotes.window_by("days(ts)", false).expect("windows").len(),
        1
    );
    assert_eq!(quotes.window_by("count", false).expect("windows").len(), 4);
    // No row, no window.
    assert_eq!(
        quote_column(Vec::new())
            .window_by("venue", false)
            .expect("windows")
            .len(),
        0
    );
}

#[test]
fn window_by_keys_consecutive_absent_rows_as_one_null_window() {
    let quotes = quote_column(vec![
        quote(Some("XNAS"), 1, 0),
        Scalar::Null,
        Scalar::Null,
        quote(None, 2, 1),
        quote(None, 3, 2),
    ]);
    assert_eq!(
        cuts(&quotes.window_by("venue", false).expect("windows")),
        vec![
            (one_cell("XNAS"), 0, 1),
            (Scalar::Null, 1, 2),
            (one_cell(Scalar::Null), 3, 2),
        ]
    );

    // A record landed off a foreign struct array: its children hold
    // different values under the two absent parents, and the identity key
    // still reads those rows as one absent key.
    let field = quote_field();
    let venue: ArrayRef = Arc::new(StringArray::from(vec![
        Some("XNAS"),
        Some("ghost"),
        Some("other"),
        Some("XNAS"),
    ]));
    let count: ArrayRef = Arc::new(Int64Array::from(vec![1, 98, 99, 1]));
    let ts: ArrayRef = Arc::new(
        arrow_array::TimestampNanosecondArray::from(vec![Some(0), Some(7), Some(8), Some(0)])
            .with_timezone("UTC"),
    );
    let ArrowDataType::Struct(fields) = field
        .as_arrow_field_ref()
        .expect("a projection")
        .data_type()
        .clone()
    else {
        unreachable!("a record field projects to a struct")
    };
    let records = arrow_array::StructArray::try_new(
        fields,
        vec![venue, count, ts],
        Some(NullBuffer::from(vec![true, false, false, true])),
    )
    .expect("a struct array");
    let landed = Serie::from_arrow_array(Some(&field), Arc::new(records), ArrowCastOptions::new())
        .expect("a record column");
    let identity = landed
        .window_by("venue, count, ts", false)
        .expect("windows");
    assert_eq!(
        cuts(&identity)
            .into_iter()
            .map(|(_, offset, len)| (offset, len))
            .collect::<Vec<_>>(),
        vec![(0, 1), (1, 2), (3, 1)]
    );
    let (key, _) = identity.iter().nth(1).expect("the absent window");
    assert_eq!(key, Scalar::Null);
}

#[test]
fn window_by_over_sorted_keys_answers_what_partition_by_paths_answers() {
    let negative_nan = f64::from_bits(f64::NAN.to_bits() | (1 << 63));
    let payload_nan = f64::from_bits(f64::NAN.to_bits() | 1);
    let venues: ArrayRef = Arc::new(StringArray::from(vec![
        "XNAS", "XNAS", "XNAS", "XNAS", "XNAS", "XNAS", "XNAS", "XNAS", "XNYS",
    ]));
    let prices: ArrayRef = Arc::new(arrow_array::Float64Array::from(vec![
        Some(-1.0),
        Some(-0.0),
        Some(0.0),
        Some(0.0),
        Some(f64::NAN),
        Some(payload_nan),
        Some(negative_nan),
        None,
        Some(1.0),
    ]));
    let records = arrow_array::StructArray::from(vec![
        (
            Arc::new(ArrowField::new("venue", ArrowDataType::Utf8, false)),
            venues,
        ),
        (
            Arc::new(ArrowField::new("px", ArrowDataType::Float64, true)),
            prices,
        ),
    ]);
    let quotes = Serie::from_arrow_array(None, Arc::new(records), ArrowCastOptions::new())
        .expect("a record column");
    let paths = [
        "venue".parse::<FieldPath>().expect("a path"),
        "px".parse().expect("a path"),
    ];
    let groups = quotes.partition_by_paths(&paths).expect("groups");
    let windows = quotes.window_by("venue, px", false).expect("windows");
    let windows: Vec<_> = windows.iter().collect();
    // Three NaN payloads are one key, -0.0 and 0.0 two, the absent price
    // one of its own.
    assert_eq!(windows.len(), 6);
    assert_eq!(groups.len(), windows.len());
    for ((group_key, group), (window_key, window)) in groups.iter().zip(&windows) {
        assert_eq!(group_key, window_key);
        assert_eq!(*group, window.into_serie());
    }
    // The whole serie keyed through the column alone agrees too.
    let px = quotes.child("px").expect("a child");
    let by_column = px.window_by("px", false).expect("windows");
    let grouped = px.partition_by(px).expect("groups");
    assert_eq!(by_column.len(), grouped.len());
    for ((group_key, group), (window_key, window)) in grouped.iter().zip(&by_column) {
        assert_eq!(&one_cell(group_key.clone()), &window_key);
        assert_eq!(*group, window.into_serie());
    }
}

#[test]
fn a_non_record_column_windows_by_its_own_name() {
    let day = 24 * 60 * MINUTE_NS;
    let ts = Serie::from_scalars(
        utc_ns().nullable_field("ts"),
        [0, 1, day, day + 1, -1]
            .map(|nanos| utc_ns().scalar(Scalar::from(nanos)).expect("an instant")),
    )
    .expect("instants");
    let windows = cuts(&ts.window_by("days(ts)", false).expect("windows"));
    assert_eq!(
        windows,
        vec![
            (one_cell(Scalar::date32(0)), 0, 2),
            (one_cell(Scalar::date32(1)), 2, 2),
            (one_cell(Scalar::date32(-1)), 4, 1),
        ]
    );
    // The offsets carry onto any serie aligned with the keys.
    let counts = int64_column((0..5).map(Some).collect());
    let carried: Vec<Vec<Scalar>> = windows
        .iter()
        .map(|(_, offset, len)| {
            counts
                .window(*offset, *len)
                .expect("an aligned window")
                .rows()
                .into_owned()
        })
        .collect();
    assert_eq!(carried, vec![i64s(&[0, 1]), i64s(&[2, 3]), i64s(&[4])]);
    // The column's own name is the one its key reads.
    assert!(ts.window_by("price", false).is_err());
    let none = ts.slice(0, 0).expect("no row");
    assert_eq!(none.window_by("days(ts)", false).expect("windows").len(), 0);
}

#[test]
fn a_star_beside_terms_keys_every_column_it_keeps() {
    // Same venue and count, the instant moving within a day: one window
    // modulo `ts`; the count changing opens the next.
    let quotes = quote_column(vec![
        quote(Some("XNAS"), 1, 0),
        quote(Some("XNAS"), 1, 5),
        quote(Some("XNAS"), 1, 60),
        quote(Some("XNAS"), 2, 61),
    ]);
    assert_eq!(
        cuts(
            &quotes
                .window_by("* exclude (ts), days(ts)", false)
                .expect("windows")
        ),
        vec![
            (
                Scalar::from_sequence([
                    Scalar::from("XNAS"),
                    Scalar::from(1_i64),
                    Scalar::date32(0)
                ]),
                0,
                3
            ),
            (
                Scalar::from_sequence([
                    Scalar::from("XNAS"),
                    Scalar::from(2_i64),
                    Scalar::date32(0)
                ]),
                3,
                1
            ),
        ]
    );
}

#[test]
fn a_rust_list_of_texts_names_columns() {
    // A list is exact column names, as every list door reads it: a term is
    // written as one clause text.
    let quotes = venue_runs();
    let refused = quotes
        .window_by(["minutes(ts, 15)"], false)
        .unwrap_err()
        .to_string();
    assert!(refused.contains("minutes(ts, 15)"), "{refused}");
    assert_eq!(
        cuts(&quotes.window_by(["venue"], false).expect("windows")),
        cuts(&quotes.window_by("venue", false).expect("windows"))
    );
}

// ---------------------------------------------------------------------------
// window_by sorted: each key once, in key order
// ---------------------------------------------------------------------------

/// The values' order the ordering verbs read, ascending with every
/// absence last, a row's and one nested in a sequence or a map alike: a
/// sequence and a map compare item by item, then the shorter first. The
/// reference a window cut is pinned against.
fn value_order(left: &Scalar, right: &Scalar) -> Ordering {
    match (left.is_null(), right.is_null()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => match (left.as_serie(), right.as_serie(), left, right) {
            (Some(left), Some(right), ..) => {
                let (left, right) = (left.rows(), right.rows());
                left.iter()
                    .zip(right.iter())
                    .map(|(left, right)| value_order(left, right))
                    .find(|step| *step != Ordering::Equal)
                    .unwrap_or_else(|| left.len().cmp(&right.len()))
            }
            (
                _,
                _,
                Scalar::Map(left) | Scalar::SortedMap(left),
                Scalar::Map(right) | Scalar::SortedMap(right),
            ) => {
                let (left, right) = (left.as_slice(), right.as_slice());
                left.iter()
                    .zip(right)
                    .map(|((left_key, left_value), (right_key, right_value))| {
                        value_order(left_key, right_key)
                            .then_with(|| value_order(left_value, right_value))
                    })
                    .find(|step| *step != Ordering::Equal)
                    .unwrap_or_else(|| left.len().cmp(&right.len()))
            }
            _ => left.cmp(right),
        },
    }
}

/// The windows `keys` - one key per row - cut into under `sorted`, as
/// `window_by` is pinned to cut them: maximal runs of equal adjacent keys in
/// row order; or, sorted and out of order, those runs sorted stably by key
/// and the runs of one key merged. Each window as its key, its offset and
/// its length, beside the rows, in order, of the serie the windows view.
fn reference_cuts(keys: &[Scalar], sorted: bool) -> (Vec<(Scalar, usize, usize)>, Vec<u32>) {
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for (row, key) in keys.iter().enumerate() {
        match runs.last_mut() {
            Some(run) if value_order(&keys[run.0], key) == Ordering::Equal => run.1 = row + 1,
            _ => runs.push((row, row + 1)),
        }
    }
    let in_order = keys
        .windows(2)
        .all(|pair| value_order(&pair[0], &pair[1]) != Ordering::Greater);
    if !sorted || in_order {
        return (
            runs.iter()
                .map(|(start, end)| (keys[*start].clone(), *start, end - start))
                .collect(),
            (0..keys.len() as u32).collect(),
        );
    }
    runs.sort_by(|left, right| value_order(&keys[left.0], &keys[right.0]));
    let mut order: Vec<u32> = Vec::new();
    let mut windows: Vec<(Scalar, usize, usize)> = Vec::new();
    for (start, end) in runs {
        let at = order.len();
        order.extend(start as u32..end as u32);
        match windows.last_mut() {
            Some(window) if value_order(&window.0, &keys[start]) == Ordering::Equal => {
                window.2 += end - start;
            }
            _ => windows.push((keys[start].clone(), at, end - start)),
        }
    }
    (windows, order)
}

/// Pin `serie.window_by(by, sorted)` under both flags against the
/// reference cut of `keys`, the key each row computes: the same keys,
/// offsets and lengths, and the rows the windows' serie holds - the serie
/// itself where nothing is gathered, else its rows taken once in key order.
fn cuts_as_its_values_do(serie: &Serie, by: &str, keys: &[Scalar]) {
    for sorted in [false, true] {
        let what = format!("{by} over {serie}, sorted {sorted}");
        let windows = serie.window_by(by, sorted).expect(&what);
        let (expected, order) = reference_cuts(keys, sorted);
        assert_eq!(cuts(&windows), expected, "{what}");
        assert_eq!(windows.len(), expected.len(), "{what}");
        let gathered = order.iter().enumerate().any(|(at, row)| at as u32 != *row);
        assert_eq!(
            std::ptr::eq(windows.serie(), serie),
            !gathered,
            "{what}: the serie is borrowed exactly when nothing is gathered"
        );
        assert_eq!(
            without_order(windows.serie().field()),
            serie.field().cloned(),
            "{what}"
        );
        let taken = serie
            .into_taken(&Serie::new(u32s(&order)))
            .expect("the reference order");
        assert_eq!(windows.serie().rows(), taken.rows(), "{what}");
        for (_, window) in &windows {
            assert!(std::ptr::eq(window.serie(), windows.serie()), "{what}");
        }
    }
}

/// One key per row of a column keyed by its own name: the one-cell run of
/// its row.
fn own_keys(column: &Serie) -> Vec<Scalar> {
    column.rows().iter().cloned().map(one_cell).collect()
}

/// One key per row of a record column keyed by the children at `cells`: the
/// run of those cells, or [`Scalar::Null`] where the row is absent.
fn record_keys(records: &Serie, cells: &[usize]) -> Vec<Scalar> {
    records
        .rows()
        .iter()
        .map(|row| match row.as_serie() {
            None => Scalar::Null,
            Some(run) => Scalar::from_sequence(
                cells
                    .iter()
                    .map(|cell| run.scalar(*cell).expect("a cell"))
                    .collect::<Vec<_>>(),
            ),
        })
        .collect()
}

#[test]
fn window_by_sorted_over_keys_in_order_answers_the_unsorted_windows_over_the_serie() {
    // Present venues ascending, then an absent venue, then an absent row:
    // already in key order.
    let quotes = quote_column(vec![
        quote(Some("XNAS"), 1, 0),
        quote(Some("XNAS"), 2, 14),
        quote(Some("XNYS"), 3, 15),
        quote(None, 4, 31),
        quote(None, 5, 32),
        Scalar::Null,
    ]);
    for by in ["venue", "venue, count", "minutes(ts, 15)"] {
        let unsorted = quotes.window_by(by, false).expect("windows");
        let sorted = quotes.window_by(by, true).expect("windows");
        assert_eq!(cuts(&sorted), cuts(&unsorted), "{by}");
        assert!(std::ptr::eq(sorted.serie(), &quotes), "{by}");
        for (_, window) in &sorted {
            assert!(std::ptr::eq(window.serie(), &quotes), "{by}");
        }
    }
    assert_eq!(
        cuts(&quotes.window_by("venue", true).expect("windows")),
        vec![
            (one_cell("XNAS"), 0, 2),
            (one_cell("XNYS"), 2, 1),
            (one_cell(Scalar::Null), 3, 2),
            (Scalar::Null, 5, 1),
        ]
    );

    // NaN payloads are one key above every number, -0.0 orders before 0.0,
    // the absent price last: in order, so nothing moves.
    let negative_nan = f64::from_bits(f64::NAN.to_bits() | (1 << 63));
    let payload_nan = f64::from_bits(f64::NAN.to_bits() | 1);
    let venues: ArrayRef = Arc::new(StringArray::from(vec![
        "XNAS", "XNAS", "XNAS", "XNAS", "XNAS", "XNAS", "XNAS", "XNAS", "XNYS",
    ]));
    let prices: ArrayRef = Arc::new(arrow_array::Float64Array::from(vec![
        Some(-1.0),
        Some(-0.0),
        Some(0.0),
        Some(0.0),
        Some(f64::NAN),
        Some(payload_nan),
        Some(negative_nan),
        None,
        Some(1.0),
    ]));
    let records = arrow_array::StructArray::from(vec![
        (
            Arc::new(ArrowField::new("venue", ArrowDataType::Utf8, false)),
            venues,
        ),
        (
            Arc::new(ArrowField::new("px", ArrowDataType::Float64, true)),
            prices,
        ),
    ]);
    let quotes = Serie::from_arrow_array(None, Arc::new(records), ArrowCastOptions::new())
        .expect("a record column");
    let unsorted = quotes.window_by("venue, px", false).expect("windows");
    let sorted = quotes.window_by("venue, px", true).expect("windows");
    assert_eq!(sorted.len(), 6);
    assert_eq!(cuts(&sorted), cuts(&unsorted));
    assert!(std::ptr::eq(sorted.serie(), &quotes));
    // The prices alone, before the XNYS row brings 1.0 after the absent one.
    let px = quotes
        .child("px")
        .expect("a child")
        .slice(0, 8)
        .expect("eight rows");
    let sorted = px.window_by("px", true).expect("windows");
    assert_eq!(
        cuts(&sorted),
        cuts(&px.window_by("px", false).expect("windows"))
    );
    assert!(std::ptr::eq(sorted.serie(), &px));
    assert_eq!(sorted.len(), 5);
}

#[test]
fn window_by_sorted_gathers_the_rows_once_in_stable_key_order() {
    let quotes = quote_column(vec![
        quote(Some("XNYS"), 1, 0),
        quote(Some("XNAS"), 2, 1),
        quote(Some("XNYS"), 3, 2),
        quote(None, 4, 3),
        quote(Some("XNAS"), 5, 4),
    ]);
    let windows = quotes.window_by("venue", true).expect("windows");
    assert_eq!(windows.len(), 3);
    assert!(!windows.is_empty());
    // XNAS first, its rows 1 and 4 in arrival order; XNYS; the absent venue
    // last.
    assert_eq!(
        cuts(&windows),
        vec![
            (one_cell("XNAS"), 0, 2),
            (one_cell("XNYS"), 2, 2),
            (one_cell(Scalar::Null), 4, 1),
        ]
    );
    // The rows are gathered once into a serie the windows own, under the
    // same field; every window views it.
    assert!(!std::ptr::eq(windows.serie(), &quotes));
    let stable = Serie::new(u32s(&[1, 4, 0, 2, 3]));
    let taken = quotes.into_taken(&stable).expect("taken");
    assert_eq!(
        without_order(windows.serie().field()),
        quotes.field().cloned()
    );
    assert_eq!(windows.serie().rows(), taken.rows());
    for (_, window) in &windows {
        assert!(std::ptr::eq(window.serie(), windows.serie()));
    }
    let (_, xnas) = windows.iter().next().expect("a first window");
    assert_eq!(
        xnas.rows().to_vec(),
        vec![
            quotes.scalar(1).expect("a row"),
            quotes.scalar(4).expect("a row")
        ]
    );
    // It is the stable order taken, then cut in row order.
    assert_eq!(
        cuts(&windows),
        cuts(&taken.window_by("venue", false).expect("windows"))
    );
    // The serie it was cut from is untouched.
    assert_eq!(
        quotes.scalar(0).expect("a row"),
        quote_field()
            .scalar(quote(Some("XNYS"), 1, 0))
            .expect("a quote row")
    );

    // An absent row keys `Null`, after a present row with an absent venue.
    let quotes = quote_column(vec![
        Scalar::Null,
        quote(Some("XNAS"), 1, 0),
        Scalar::Null,
        quote(None, 2, 1),
    ]);
    let windows = quotes.window_by("venue", true).expect("windows");
    assert_eq!(
        cuts(&windows),
        vec![
            (one_cell("XNAS"), 0, 1),
            (one_cell(Scalar::Null), 1, 1),
            (Scalar::Null, 2, 2),
        ]
    );

    // Two cells key in selector order: the venue first, then the bucket.
    let quotes = quote_column(vec![
        quote(Some("XNYS"), 1, 0),
        quote(Some("XNAS"), 2, 20),
        quote(Some("XNAS"), 3, 0),
        quote(Some("XNYS"), 4, 16),
    ]);
    let key = |venue: &str, bucket: i32| {
        Scalar::from_sequence([Scalar::from(venue), Scalar::from(bucket)])
    };
    let windows = quotes
        .window_by("venue, minutes(ts, 15) as bucket", true)
        .expect("windows");
    assert_eq!(
        cuts(&windows),
        vec![
            (key("XNAS", 0), 0, 1),
            (key("XNAS", 1), 1, 1),
            (key("XNYS", 0), 2, 1),
            (key("XNYS", 1), 3, 1),
        ]
    );
    assert_eq!(
        windows
            .serie()
            .rows()
            .iter()
            .map(|row| row.get(1).expect("count").into_owned())
            .collect::<Vec<_>>(),
        i64s(&[3, 2, 1, 4])
    );
}

/// The record field `{venue: mic, px: float64?, qty: int64}`, nullable.
fn coded_quote_field() -> Field {
    StructType::from_fields([
        DataType::Mic.required_field("venue"),
        DataType::Float64.nullable_field("px"),
        DataType::Int64.required_field("qty"),
    ])
    .map(DataType::from)
    .expect("three children")
    .nullable_field("quote")
}

/// [`coded_quote_field`] rows off Arrow buffers: a negative and a payload
/// NaN beside the canonical one, -0.0 beside 0.0, an absent price and an
/// absent row.
fn coded_quotes() -> Serie {
    let field = coded_quote_field();
    let ArrowDataType::Struct(fields) = field
        .as_arrow_field_ref()
        .expect("a projection")
        .data_type()
        .clone()
    else {
        unreachable!("a record field projects to a struct")
    };
    let negative_nan = f64::from_bits(f64::NAN.to_bits() | (1 << 63));
    let payload_nan = f64::from_bits(f64::NAN.to_bits() | 1);
    let venues: ArrayRef = Arc::new(StringArray::from(vec![
        "XNYS", "XNAS", "XNAS", "XNYS", "XNAS", "XNAS", "XNYS", "XNAS", "XNAS",
    ]));
    let prices: ArrayRef = Arc::new(arrow_array::Float64Array::from(vec![
        Some(0.0),
        Some(-0.0),
        Some(0.0),
        Some(f64::NAN),
        Some(negative_nan),
        Some(payload_nan),
        None,
        Some(1.0),
        Some(1.0),
    ]));
    let quantities: ArrayRef = Arc::new(Int64Array::from(vec![1, 2, 3, 4, 5, 6, 7, 8, 9]));
    let records = arrow_array::StructArray::try_new(
        fields,
        vec![venues, prices, quantities],
        Some(NullBuffer::from(vec![
            true, true, true, true, true, true, true, false, true,
        ])),
    )
    .expect("a struct array");
    Serie::from_arrow_array(Some(&field), Arc::new(records), ArrowCastOptions::new())
        .expect("a record column")
}

/// The record field `{order: {venue: mic, px: float64?}?, qty: int64}`.
fn ordered_quote_field() -> Field {
    let order = StructType::from_fields([
        DataType::Mic.required_field("venue"),
        DataType::Float64.nullable_field("px"),
    ])
    .map(DataType::from)
    .expect("two children")
    .nullable_field("order");
    StructType::from_fields([order, DataType::Int64.required_field("qty")])
        .map(DataType::from)
        .expect("two children")
        .required_field("fill")
}

/// Rows under [`ordered_quote_field`], `order` absent in two of them.
fn ordered_quotes() -> Serie {
    let field = ordered_quote_field();
    let order = |venue: &str, px: Option<f64>| {
        Scalar::from_sequence([Scalar::from(venue), px.map_or(Scalar::Null, Scalar::from)])
    };
    let rows = [
        (Some(order("XNYS", Some(2.0))), 1_i64),
        (None, 2),
        (Some(order("XNAS", None)), 3),
        (Some(order("XNAS", Some(1.0))), 4),
        (Some(order("XNAS", None)), 5),
        (None, 6),
        (Some(order("XNYS", Some(2.0))), 7),
    ]
    .into_iter()
    .map(|(order, qty)| {
        field
            .scalar(Scalar::from_sequence([
                order.unwrap_or(Scalar::Null),
                Scalar::from(qty),
            ]))
            .expect("a fill row")
    })
    .collect::<Vec<_>>();
    Serie::from_scalars(field, rows).expect("fill rows")
}

#[test]
fn window_by_cuts_every_nested_key_as_its_values_do() {
    for column in nested_columns() {
        if column.as_struct().is_some() {
            cuts_as_its_values_do(&column, "venue, price", &record_keys(&column, &[0, 1]));
        } else {
            let by = format!("\"{}\"", column.field().expect("a column").name());
            cuts_as_its_values_do(&column, &by, &own_keys(&column));
        }
    }

    // A union keys by its member and its payload.
    let member =
        |type_id: i64, payload: Scalar| Scalar::from_sequence([Scalar::from(type_id), payload]);
    let unions = Serie::from_scalars(
        Field::new(
            "quote",
            DataType::union(
                [
                    (0, Field::new("id", DataType::Int64, false)),
                    (1, Field::new("symbol", DataType::utf8(), true)),
                ],
                yggdryl::UnionMode::Sparse,
            )
            .expect("two members"),
            true,
        ),
        [
            member(1, Scalar::from("AAPL")),
            member(0, Scalar::from(2_i64)),
            member(0, Scalar::from(2_i64)),
            Scalar::Null,
            member(1, Scalar::Null),
            member(0, Scalar::from(1_i64)),
            member(1, Scalar::from("AAPL")),
        ],
    )
    .expect("union rows");
    cuts_as_its_values_do(&unions, "quote", &own_keys(&unions));

    // A map keys entry by entry, then the shorter first.
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
            entries(&[("a", Some(1)), ("b", Some(2))]),
            entries(&[("a", Some(1))]),
            entries(&[("a", Some(1))]),
            Scalar::Null,
            entries(&[("a", None)]),
            entries(&[("a", Some(1)), ("b", Some(2))]),
        ],
    )
    .expect("map rows");
    cuts_as_its_values_do(&maps, "tags", &own_keys(&maps));

    // A run-end column keys by the values its runs hold.
    let states_field = Field::new(
        "state",
        DataType::run_end_encoded(
            Field::new("run_ends", DataType::Int32, false),
            Field::new("values", DataType::utf8(), true),
        )
        .expect("a run-end datatype"),
        true,
    );
    let states: ArrayRef = Arc::new(
        arrow_array::RunArray::<arrow_array::types::Int32Type>::try_new(
            &arrow_array::Int32Array::from(vec![2, 3, 5, 6]),
            &StringArray::from(vec![Some("open"), None, Some("closed"), Some("open")]),
        )
        .expect("climbing run ends"),
    );
    let states = Serie::from_arrow_array(Some(&states_field), states, ArrowCastOptions::new())
        .expect("a run-end column");
    cuts_as_its_values_do(&states, "state", &own_keys(&states));

    // A record holding a registered code and a float with foreign NaN
    // payloads, keyed by each cell, by two and by all three.
    let quotes = coded_quotes();
    for (by, cells) in [
        ("venue", &[0][..]),
        ("px", &[1][..]),
        ("venue, px", &[0, 1][..]),
        ("px, venue", &[1, 0][..]),
        ("venue, px, qty", &[0, 1, 2][..]),
    ] {
        cuts_as_its_values_do(&quotes, by, &record_keys(&quotes, cells));
    }

    // A whole record cell keys item by item, and a path reaches through it;
    // an absent `order` keys a null cell either way.
    let fills = ordered_quotes();
    let orders: Vec<Scalar> = fills
        .rows()
        .iter()
        .map(|row| row.get(0).expect("order").into_owned())
        .collect();
    cuts_as_its_values_do(
        &fills,
        "order",
        &orders.iter().cloned().map(one_cell).collect::<Vec<_>>(),
    );
    let cell = |at: usize| -> Vec<Scalar> {
        orders
            .iter()
            .map(|order| {
                one_cell(match order.as_serie() {
                    None => Scalar::Null,
                    Some(run) => run.scalar(at).expect("a cell"),
                })
            })
            .collect()
    };
    cuts_as_its_values_do(&fills, "order.venue", &cell(0));
    cuts_as_its_values_do(&fills, "order.px", &cell(1));
}

#[test]
fn the_record_rung_agrees_with_its_run_under_every_ordering() {
    // A registered code beside an enum and an integer: the code alone is
    // ordered by its value.
    let field = StructType::from_fields([
        DataType::Mic.required_field("venue"),
        DataType::Side.required_field("side"),
        DataType::Int64.required_field("count"),
    ])
    .map(DataType::from)
    .expect("three children")
    .required_field("quote");
    let row = |venue: &str, side: &str, count: i64| {
        field
            .scalar(Scalar::from_sequence([
                Scalar::from(venue),
                Scalar::from(side),
                Scalar::from(count),
            ]))
            .expect("a quote row")
    };
    let sided = Serie::from_scalars(
        field.clone(),
        [
            row("XNYS", "SELL", 2),
            row("XNAS", "BUYS", 1),
            row("XNYS", "BUYS", 2),
            row("XNAS", "BUYS", 1),
            row("XNAS", "SELL", 3),
        ],
    )
    .expect("quote rows");
    agrees_with_its_run(&sided);
    cuts_as_its_values_do(&sided, "venue, side", &record_keys(&sided, &[0, 1]));

    // Absent rows and absent cells of a code and of a float, every NaN
    // payload one value: placed where each ordering puts an absence.
    let quotes = coded_quotes();
    agrees_with_its_run(&quotes);
    let sorted = quotes
        .into_sorted(SortOptions::default())
        .expect("sorted")
        .rows()
        .into_owned();
    let qty = |row: &Scalar| row.get(2).map(|cell| cell.into_owned());
    assert_eq!(
        sorted.iter().map(qty).collect::<Vec<_>>(),
        // XNAS -0.0, 0.0, 1.0, then its two NaN prices, one value, by
        // quantity (2, 3, 9, 5, 6); XNYS 0.0, NaN, absent (1, 4, 7); then
        // the absent row.
        [2, 3, 9, 5, 6, 1, 4, 7]
            .map(|count: i64| Some(Scalar::from(count)))
            .into_iter()
            .chain([None])
            .collect::<Vec<_>>()
    );

    // A record nested in a record, absent in some rows: each level on its
    // own rung.
    let fills = ordered_quotes();
    agrees_with_its_run(&fills);
    let orders = fills.child("order").expect("a child");
    agrees_with_its_run(orders);
}

/// The record `book{venue: utf8?, price: int64?, qty: int64}`, its root
/// nullable so a row may be absent.
fn book_field(venue: DataType) -> Field {
    StructType::from_fields([
        venue.nullable_field("venue"),
        DataType::Int64.nullable_field("price"),
        DataType::Int64.required_field("qty"),
    ])
    .map(DataType::from)
    .expect("three children")
    .nullable_field("book")
}

/// The book's venues, prices and quantities, row 8 absent.
const BOOK: [(Option<&str>, Option<i64>, i64); 9] = [
    (Some("XNYS"), Some(5), 1),
    (Some("XNAS"), None, 2),
    (Some("XNAS"), Some(7), 3),
    (None, Some(3), 4),
    (Some("XNYS"), Some(5), 5),
    (Some("XNAS"), Some(7), 6),
    (Some("XNYS"), None, 7),
    (Some("XNAS"), Some(2), 8),
    (Some("AAAA"), Some(99), 9),
];

/// [`BOOK`] off Arrow buffers, its absent row holding a venue and a price
/// that would sort first were they read: an absent row is absent in every
/// cell.
fn book() -> Serie {
    let field = book_field(DataType::utf8());
    let ArrowDataType::Struct(fields) = field
        .as_arrow_field_ref()
        .expect("a projection")
        .data_type()
        .clone()
    else {
        unreachable!("a record field projects to a struct")
    };
    let venues: ArrayRef = Arc::new(StringArray::from(
        BOOK.iter().map(|row| row.0).collect::<Vec<_>>(),
    ));
    let prices: ArrayRef = Arc::new(Int64Array::from(
        BOOK.iter().map(|row| row.1).collect::<Vec<_>>(),
    ));
    let quantities: ArrayRef = Arc::new(Int64Array::from(
        BOOK.iter().map(|row| row.2).collect::<Vec<_>>(),
    ));
    let records = arrow_array::StructArray::try_new(
        fields,
        vec![venues, prices, quantities],
        Some(NullBuffer::from(
            (0..BOOK.len()).map(|row| row != 8).collect::<Vec<_>>(),
        )),
    )
    .expect("a struct array");
    Serie::from_arrow_array(Some(&field), Arc::new(records), ArrowCastOptions::new())
        .expect("a record column")
}

/// [`BOOK`] laid out row by row under `venue`, row 8 absent.
fn book_of(venue: DataType) -> Serie {
    let field = book_field(venue);
    let rows = BOOK
        .iter()
        .enumerate()
        .map(|(row, (venue, price, qty))| {
            if row == 8 {
                return Scalar::Null;
            }
            field
                .scalar(Scalar::from_sequence([
                    venue.map_or(Scalar::Null, Scalar::from),
                    price.map_or(Scalar::Null, Scalar::from),
                    Scalar::from(*qty),
                ]))
                .expect("a book row")
        })
        .collect::<Vec<_>>();
    Serie::from_scalars(field, rows).expect("book rows")
}

/// The order `cells` - per row, one leaf cell per key - sort into under
/// `options`, one per key: a stable sort by the first key whose cells
/// differ, each absent cell where its key puts an absence. The reference
/// every `order by` rung is pinned to.
fn reference_order(cells: &[Vec<Scalar>], options: &[SortOptions]) -> Vec<u32> {
    let mut order: Vec<u32> = (0..cells.len() as u32).collect();
    order.sort_by(|left, right| {
        let (left, right) = (&cells[*left as usize], &cells[*right as usize]);
        left.iter()
            .zip(right)
            .zip(options)
            .map(|((left, right), options)| {
                let absent = if options.is_nulls_first() {
                    Ordering::Less
                } else {
                    Ordering::Greater
                };
                match (left.is_null(), right.is_null()) {
                    (true, true) => Ordering::Equal,
                    (true, false) => absent,
                    (false, true) => absent.reverse(),
                    (false, false) if options.is_descending() => value_order(left, right).reverse(),
                    (false, false) => value_order(left, right),
                }
            })
            .find(|step| *step != Ordering::Equal)
            .unwrap_or(Ordering::Equal)
    });
    order
}

/// The cells at `at` of every row of `records`, an absent row's all null.
fn cells_at(records: &Serie, at: &[usize]) -> Vec<Vec<Scalar>> {
    records
        .rows()
        .iter()
        .map(|row| {
            at.iter()
                .map(|cell| {
                    row.as_serie()
                        .map_or(Scalar::Null, |run| run.scalar(*cell).expect("a cell"))
                })
                .collect()
        })
        .collect()
}

fn order_of(serie: &Serie, by: &str) -> Vec<Scalar> {
    serie
        .sort_indices_by(by)
        .unwrap_or_else(|error| panic!("{by}: {error}"))
        .rows()
        .into_owned()
}

#[test]
fn sort_by_orders_a_record_by_every_key_in_turn_stable_over_ties() {
    let book = book();
    // XNAS by price descending, absent first; then XNYS; then the absent
    // venue and the absent row, which reads absent in every cell.
    assert_eq!(
        order_of(&book, "venue, price desc nulls first"),
        u32s(&[1, 2, 5, 7, 6, 0, 4, 8, 3])
    );
    assert_eq!(
        order_of(&book, "venue desc nulls first, price"),
        u32s(&[3, 8, 0, 4, 6, 7, 2, 5, 1])
    );
    // An index column, as `sort_indices` answers.
    let order = book.sort_indices_by("venue, price").expect("an order");
    assert_eq!(order.field().map(Field::name), Some("index"));
    assert_eq!(order.field().map(Field::dtype), Some(&DataType::UInt32));

    // Every pair of orderings against the reference, on the row format's
    // rung (utf8 and int64 cells) and on the values' own (a windows-1252
    // venue): one order, rows of equal keys kept in arrival order.
    let cp1252 = book_of(DataType::cp1252());
    for first in ORDERINGS {
        for second in ORDERINGS {
            let by = format!("venue{first}, price{second}");
            let expected = u32s(&reference_order(
                &cells_at(&book, &[0, 1]),
                &[first, second],
            ));
            assert_eq!(order_of(&book, &by), expected, "{by}");
            assert_eq!(order_of(&cp1252, &by), expected, "{by} over windows-1252");
            // Two stable sorts, the least significant key first, are the
            // same order.
            let mut held = book.clone();
            held.as_sort_by(format!("price{second}"))
                .expect("by price")
                .as_sort_by(format!("venue{first}"))
                .expect("by venue");
            assert_eq!(
                held.rows(),
                book.into_sort_by(by.as_str()).expect("sorted").rows(),
                "{by}"
            );
        }
    }
}

#[test]
fn into_sort_by_takes_the_rows_under_the_same_field_and_leaves_the_serie() {
    let book = book();
    let before = book.rows().into_owned();
    let order = book
        .sort_indices_by("venue, price desc nulls first")
        .expect("an order");
    let sorted = book
        .into_sort_by("venue, price desc nulls first")
        .expect("sorted");
    assert_eq!(without_order(sorted.field()), book.field().cloned());
    assert_eq!(
        sorted.rows(),
        book.into_taken(&order).expect("taken").rows()
    );
    assert_eq!(book.rows().into_owned(), before);
}

#[test]
fn a_plain_column_sorts_by_its_own_name_as_into_sorted_does() {
    let prices = int64_column(vec![Some(3), None, Some(1), Some(2), Some(1)]);
    for options in ORDERINGS {
        let by = format!("price{options}");
        assert_eq!(
            prices
                .sort_indices_by(by.as_str())
                .expect("an order")
                .rows(),
            prices.sort_indices(options).expect("an order").rows(),
            "{by}"
        );
        assert_eq!(
            prices.into_sort_by(by.as_str()).expect("sorted").rows(),
            prices.into_sorted(options).expect("sorted").rows(),
            "{by}"
        );
    }
    let values = Serie::from_scalars(
        Field::new("value", DataType::Int64, false),
        i64s(&[2, 9, 4]),
    )
    .expect("a column");
    assert_eq!(
        values
            .into_sort_by("value desc")
            .expect("sorted")
            .rows()
            .to_vec(),
        i64s(&[9, 4, 2])
    );
    // A computed term over the column's own name.
    assert_eq!(order_of(&values, "value * -1"), u32s(&[1, 2, 0]));
}

#[test]
fn a_computed_key_orders_by_what_it_computes() {
    let book = book();
    assert_eq!(
        order_of(&book, "price * 2 desc"),
        order_of(&book, "price desc")
    );
    assert_eq!(
        order_of(&book, "venue, price * 2 desc nulls first"),
        u32s(&[1, 2, 5, 7, 6, 0, 4, 8, 3])
    );
    let field = StructType::from_fields([
        DataType::utf8().required_field("venue"),
        DataType::Int64.required_field("qty"),
    ])
    .map(DataType::from)
    .expect("two children")
    .required_field("fill");
    let fills = Serie::from_scalars(
        field,
        [("XNYS", 1_i64), ("xnas", 2), ("XNAS", 3), ("xnys", 4)]
            .map(|(venue, qty)| Scalar::from_sequence([Scalar::from(venue), Scalar::from(qty)])),
    )
    .expect("fills");
    // Folded, the two spellings of a venue tie and the quantity decides;
    // as stored, upper case sorts first.
    assert_eq!(
        order_of(&fills, "lower(venue), qty desc"),
        u32s(&[2, 1, 3, 0])
    );
    assert_eq!(order_of(&fills, "venue"), u32s(&[2, 0, 1, 3]));
}

#[test]
fn a_key_off_the_row_format_sorts_by_its_values_beside_the_others() {
    // A version orders by its numbers: the values' own rung, beside an
    // integer on the comparator's.
    let field = StructType::from_fields([
        DataType::Version.required_field("release"),
        DataType::Int64.required_field("build"),
    ])
    .map(DataType::from)
    .expect("two children")
    .required_field("ship");
    let ship = |release: &str, build: i64| {
        field
            .scalar(Scalar::from_sequence([
                Scalar::from(release),
                Scalar::from(build),
            ]))
            .expect("a ship row")
    };
    let ships = Serie::from_scalars(
        field.clone(),
        [
            ship("1.10.0", 1),
            ship("1.9.0", 2),
            ship("1.10.0", 3),
            ship("1.2.0", 4),
            ship("1.9.0", 5),
        ],
    )
    .expect("ships");
    assert_eq!(
        order_of(&ships, "release desc, build"),
        u32s(&[0, 2, 1, 4, 3])
    );
    for first in ORDERINGS {
        for second in ORDERINGS {
            let by = format!("release{first}, build{second}");
            assert_eq!(
                order_of(&ships, &by),
                u32s(&reference_order(
                    &cells_at(&ships, &[0, 1]),
                    &[first, second]
                )),
                "{by}"
            );
        }
    }

    // A registered code and a float holding foreign NaN payloads, under a
    // root with an absent row whose cells hold values: each cell on its own
    // rung, the absent row absent in every key.
    let quotes = coded_quotes();
    assert_eq!(
        order_of(&quotes, "venue desc, qty"),
        u32s(&[0, 3, 6, 1, 2, 4, 5, 8, 7])
    );
    assert_eq!(
        order_of(&quotes, "qty desc"),
        u32s(&[8, 6, 5, 4, 3, 2, 1, 0, 7])
    );
    assert_eq!(
        order_of(&quotes, "qty desc nulls first"),
        u32s(&[7, 8, 6, 5, 4, 3, 2, 1, 0])
    );
    // Every NaN one value above every number, -0.0 below 0.0, and the
    // absent row's 1.0 never read.
    assert_eq!(
        order_of(&quotes, "px desc, qty"),
        u32s(&[3, 4, 5, 8, 0, 2, 1, 6, 7])
    );
    for options in ORDERINGS {
        let by = format!("px{options}, qty desc");
        let mut held = quotes.clone();
        held.as_sort_by("qty desc")
            .expect("by qty")
            .as_sort_by(format!("px{options}"))
            .expect("by px");
        assert_eq!(
            quotes.into_sort_by(by.as_str()).expect("sorted").rows(),
            held.rows(),
            "{by}"
        );
    }

    // A record key compares child by child, absent where its row is.
    let fills = ordered_quotes();
    for first in ORDERINGS {
        for second in ORDERINGS {
            let by = format!("order{first}, qty{second}");
            let mut held = fills.clone();
            held.as_sort_by(format!("qty{second}"))
                .expect("by qty")
                .as_sort_by(format!("order{first}"))
                .expect("by order");
            assert_eq!(
                fills.into_sort_by(by.as_str()).expect("sorted").rows(),
                held.rows(),
                "{by}"
            );
            assert_eq!(
                fills
                    .into_sort_by(format!("order{first}"))
                    .expect("sorted")
                    .rows(),
                fills
                    .child("order")
                    .expect("a child")
                    .sort_indices(first)
                    .and_then(|order| fills.into_taken(&order))
                    .expect("sorted")
                    .rows(),
                "{by}"
            );
        }
    }
}

#[test]
fn every_spelling_of_the_keys_answers_one_order() {
    let book = book();
    let expected = u32s(&[1, 2, 5, 7, 6, 0, 4, 8, 3]);
    let keys: Vec<OrderBy> = ["venue", "price desc nulls first"]
        .into_iter()
        .map(|text| text.parse().expect("a key"))
        .collect();
    let record = |entries: Vec<(&str, Scalar)>| Scalar::from_struct(entries).expect("a record");
    let orders = [
        book.sort_indices_by("venue, price desc nulls first"),
        book.sort_indices_by(String::from("venue, price desc nulls first")),
        book.sort_indices_by(["venue", "price desc nulls first"]),
        book.sort_indices_by(vec!["venue", "price desc nulls first"]),
        book.sort_indices_by(keys.clone()),
        book.sort_indices_by(keys.as_slice()),
        book.sort_indices_by(Scalar::from("venue, price desc nulls first")),
        book.sort_indices_by(Scalar::from_sequence([
            Scalar::from("venue"),
            Scalar::from("price desc nulls first"),
        ])),
        book.sort_indices_by(Scalar::from_sequence([
            record(vec![("term", Scalar::from("venue"))]),
            record(vec![
                ("term", Scalar::from("price")),
                ("descending", Scalar::from(true)),
                ("nulls_first", Scalar::from(true)),
            ]),
        ])),
    ];
    for (spelling, order) in orders.into_iter().enumerate() {
        assert_eq!(
            order.expect("an order").rows().into_owned(),
            expected,
            "spelling {spelling}"
        );
    }
    // A selector keys every projection ascending.
    assert_eq!(
        book.sort_indices_by("venue, price".parse::<Selector>().expect("a selector"))
            .expect("an order")
            .rows(),
        book.sort_indices_by("venue, price")
            .expect("an order")
            .rows()
    );
    assert_eq!(
        "venue, price".into_orderings().expect("keys"),
        ["venue", "price"].into_orderings().expect("keys")
    );
}

#[test]
fn sort_by_refuses_by_name_before_any_row() {
    let book = book();
    let empty = book_of(DataType::utf8())
        .into_filtered(&Serie::new(vec![Scalar::from(false); BOOK.len()]))
        .expect("no row");
    for serie in [&book, &empty] {
        let what = format!("{} rows", serie.len());
        // No key at all.
        for refused in [
            serie.sort_indices_by(Vec::<OrderBy>::new()),
            serie.sort_indices_by(Selector::new(Vec::new())),
        ] {
            assert_eq!(
                invalid_record(refused.unwrap_err()),
                (
                    "book".to_owned(),
                    "expected at least one `order by` key to sort book by, got none".to_owned()
                ),
                "{what}"
            );
        }
        // Text that is not a list of keys: the parse error itself.
        assert_eq!(
            serie.sort_indices_by("venue,").unwrap_err().to_string(),
            "venue,".into_orderings().unwrap_err().to_string(),
            "{what}"
        );
        // An unnest is one row per element, never one key per row.
        let refused = serie
            .sort_indices_by("unnest(items)")
            .unwrap_err()
            .to_string();
        assert!(refused.contains("unnest"), "{what}: {refused}");
        // A column the record does not hold: the binder's own text.
        let root = SerieReader::root_of(serie.field().expect("a column")).expect("a root");
        let unknown = "tier".parse::<Selector>().expect("a selector");
        assert_eq!(
            serie.sort_indices_by("tier desc").unwrap_err().to_string(),
            unknown.bind(&root).unwrap_err().to_string(),
            "{what}"
        );
        // Two keys publishing one name.
        let refused = serie
            .sort_indices_by("price, price desc")
            .unwrap_err()
            .to_string();
        assert!(refused.contains("twice"), "{what}: {refused}");
        // Every refusal of the write leaves the serie as it was.
        let mut held = serie.clone();
        for by in ["venue,", "tier", "price, price desc", "unnest(items)"] {
            assert!(held.as_sort_by(by).is_err(), "{what}: {by}");
            assert_eq!(held.rows(), serie.rows(), "{what}: {by}");
        }
    }
    // A run has no field for a term to read.
    for run in [Serie::new(i64s(&[1, 2])), Serie::new(Vec::new())] {
        assert_eq!(
            invalid_record(run.sort_indices_by("price").unwrap_err()),
            (
                "$".to_owned(),
                "a schema-free run sorts by no term".to_owned()
            )
        );
    }
}

#[test]
fn as_sort_by_sorts_in_place_and_chains() {
    let book = book();
    let mut held = book.clone();
    held.as_sort_by("venue, price desc nulls first")
        .expect("sorted")
        .as_reversed()
        .expect("reversed");
    assert_eq!(
        held.rows(),
        book.into_sort_by("venue, price desc nulls first")
            .expect("sorted")
            .into_reversed()
            .rows()
    );
    assert_eq!(without_order(held.field()), book.field().cloned());
}

// ---------------------------------------------------------------------------
// The declared order: `SORT:by` on a record's root, written by the sorts,
// kept by what keeps the order, flipped by a reversal, cleared by a write
// that breaks it, and read before any row is compared.
// ---------------------------------------------------------------------------

/// The `SORT:by` text a serie's root carries.
fn sort_by(serie: &Serie) -> Option<String> {
    serie
        .field()
        .and_then(|field| field.get_metadata("SORT:by"))
        .map(str::to_owned)
}

/// The keys a serie declares, each as the grammar writes it.
fn declared_keys(serie: &Serie) -> Option<Vec<String>> {
    serie
        .declared_order()
        .expect("a well-formed declaration")
        .map(|keys| keys.iter().map(ToString::to_string).collect())
}

/// Whether two records lend the very same buffers, child by child.
fn shares_children(left: &Serie, right: &Serie) -> bool {
    let buffers = |child: &Serie| {
        child
            .into_arrow_array()
            .expect("a column")
            .to_data()
            .buffers()
            .to_vec()
    };
    left.children().len() == right.children().len()
        && left
            .children()
            .iter()
            .zip(right.children())
            .all(|(mine, theirs)| {
                let (mine, theirs) = (buffers(mine), buffers(theirs));
                !mine.is_empty()
                    && mine.len() == theirs.len()
                    && mine
                        .iter()
                        .zip(&theirs)
                        .all(|(left, right)| left.ptr_eq(right))
            })
}

fn quote_row(venue: &str, price: i64) -> Scalar {
    Scalar::from_sequence([Scalar::from(venue), Scalar::from(price)])
}

/// What [`declared_quotes`] declares.
const DECLARED: &str = r#"["venue","price desc"]"#;

/// XNAS 3, XNAS 1, XNYS 2, XNYS 1: the quotes sorted by `venue, price
/// desc`, the root declaring it.
fn declared_quotes() -> Serie {
    let sorted = quotes(&[("XNYS", 1), ("XNAS", 1), ("XNYS", 2), ("XNAS", 3)])
        .into_sort_by("venue, price desc")
        .expect("sorted");
    assert_eq!(
        sorted.rows().to_vec(),
        vec![
            quote_row("XNAS", 3),
            quote_row("XNAS", 1),
            quote_row("XNYS", 2),
            quote_row("XNYS", 1),
        ]
    );
    sorted
}

/// Whether `rows` land under `field` - which a declaring root verifies.
fn lands_under(field: &Field, rows: Vec<Scalar>) -> bool {
    Serie::from_scalars(field.clone(), rows).is_ok()
}

#[test]
fn a_whole_row_sort_of_a_record_declares_every_child_under_its_options() {
    let unsorted = quotes(&[("XNYS", 1), ("XNAS", 1), ("XNYS", 2), ("XNAS", 3)]);
    assert!(unsorted.declared_order().expect("none").is_none());
    for (options, text) in [
        (SortOptions::default(), r#"["venue","price"]"#),
        (SortOptions::descending(), r#"["venue desc","price desc"]"#),
        (
            SortOptions::ascending().with_nulls_first(true),
            r#"["venue nulls first","price nulls first"]"#,
        ),
        (
            SortOptions::descending().with_nulls_first(true),
            r#"["venue desc nulls first","price desc nulls first"]"#,
        ),
    ] {
        let sorted = unsorted.into_sorted(options).expect("sorted");
        assert_eq!(sort_by(&sorted).as_deref(), Some(text), "{options:?}");
        let keys = sorted
            .declared_order()
            .expect("well formed")
            .expect("declared");
        assert_eq!(
            keys.iter()
                .map(|key| (key.term().to_string(), key.options()))
                .collect::<Vec<_>>(),
            vec![("venue".to_owned(), options), ("price".to_owned(), options)],
            "{options:?}"
        );
        // In place, the same declaration and the same rows.
        let mut held = unsorted.clone();
        held.as_sorted(options).expect("sorted");
        assert_eq!(sort_by(&held).as_deref(), Some(text), "{options:?}");
        assert_eq!(held.rows(), sorted.rows(), "{options:?}");
        // The rows sorted are what the declaration says, and the serie
        // sorted is left declaring nothing.
        assert!(sorted.is_sorted(options), "{options:?}");
        assert!(
            lands_under(
                sorted.field().expect("a record"),
                sorted.rows().into_owned()
            ),
            "{options:?}"
        );
        assert!(sort_by(&unsorted).is_none());
    }
}

#[test]
fn a_sort_by_keys_declares_exactly_its_keys_as_the_grammar_spells_them() {
    let unsorted = quotes(&[("XNYS", 1), ("XNAS", 1), ("XNYS", 2), ("XNAS", 3)]);
    for (by, text) in [
        ("venue, price desc", r#"["venue","price desc"]"#),
        ("price desc nulls first", r#"["price desc nulls first"]"#),
        (
            "venue desc nulls last, price asc",
            r#"["venue desc","price"]"#,
        ),
        ("price * -1", r#"["price * -1"]"#),
    ] {
        let sorted = unsorted.into_sort_by(by).expect("sorted");
        assert_eq!(sort_by(&sorted).as_deref(), Some(text), "{by}");
        let mut held = unsorted.clone();
        held.as_sort_by(by).expect("sorted");
        assert_eq!(sort_by(&held).as_deref(), Some(text), "{by}");
        assert_eq!(held.rows(), sorted.rows(), "{by}");
        assert!(
            lands_under(
                sorted.field().expect("a record"),
                sorted.rows().into_owned()
            ),
            "{by}"
        );
    }
    // The keys a declaration reads back are the ones the sort was given.
    let sorted = unsorted.into_sort_by("venue, price desc").expect("sorted");
    assert_eq!(
        declared_keys(&sorted),
        Some(vec!["venue".to_owned(), "price desc".to_owned()])
    );
    assert_eq!(
        sorted.declared_order().expect("well formed"),
        Some("venue, price desc".into_orderings().expect("two keys"))
    );
}

#[test]
fn a_column_that_is_not_a_record_and_a_run_declare_nothing() {
    let prices = int64_column(vec![Some(3), None, Some(1)]);
    let mut held = prices.clone();
    held.as_sorted(SortOptions::default())
        .expect("sorted")
        .as_sort_by("price desc")
        .expect("sorted")
        .as_reversed()
        .expect("reversed");
    for (what, sorted) in [
        (
            "into_sorted",
            prices.into_sorted(SortOptions::default()).expect("sorted"),
        ),
        (
            "into_sort_by",
            prices.into_sort_by("price desc").expect("sorted"),
        ),
        ("into_reversed", prices.into_reversed()),
        ("the writes in place", held),
    ] {
        assert!(sorted.declared_order().expect("none").is_none(), "{what}");
        assert_eq!(sorted.field(), prices.field(), "{what}");
    }
    let run = Serie::new(i64s(&[3, 1, 2]));
    for sorted in [
        run.into_sorted(SortOptions::default()).expect("sorted"),
        run.into_reversed(),
    ] {
        assert!(sorted.declared_order().expect("none").is_none());
        assert!(sorted.field().is_none());
    }
    // A leaf field stating `SORT:by` is no record: it declares nothing, so
    // nothing reads its rows against it.
    let stated = Field::new("price", DataType::Int64, false)
        .try_with_metadata("SORT:by", r#"["price"]"#)
        .expect("the key is well formed");
    let leaf = Serie::from_scalars(stated, i64s(&[3, 1, 2])).expect("no order is read");
    assert!(leaf.declared_order().expect("none").is_none());
    assert_eq!(
        leaf.sort_indices(SortOptions::default())
            .expect("an order")
            .rows()
            .to_vec(),
        u32s(&[1, 2, 0])
    );
}

#[test]
fn what_the_declaration_states_is_answered_with_the_identity_and_the_same_buffers() {
    let sorted = declared_quotes();
    let identity = u32s(&[0, 1, 2, 3]);
    // The whole declaration, and every prefix of it.
    for by in ["venue, price desc", "venue"] {
        assert_eq!(
            sorted
                .sort_indices_by(by)
                .expect("an order")
                .rows()
                .to_vec(),
            identity,
            "{by}"
        );
        let again = sorted.into_sort_by(by).expect("sorted");
        assert!(shares_children(&again, &sorted), "{by}: the same buffers");
        // The clone keeps the declaration it had, the longer one.
        assert_eq!(sort_by(&again).as_deref(), Some(DECLARED), "{by}");
        let mut held = sorted.clone();
        held.as_sort_by(by).expect("sorted");
        assert!(shares_children(&held, &sorted), "{by}: in place, untouched");
    }
    // What the declaration does not begin with is sorted, and declared.
    let by_price = sorted.into_sort_by("price desc").expect("sorted");
    assert!(!shares_children(&by_price, &sorted));
    assert_eq!(sort_by(&by_price).as_deref(), Some(r#"["price desc"]"#));
    assert_eq!(
        sorted
            .sort_indices_by("price desc")
            .expect("an order")
            .rows()
            .to_vec(),
        u32s(&[0, 2, 1, 3])
    );
    let longer = sorted
        .into_sort_by("venue, price desc, venue desc")
        .err()
        .map(|error| error.to_string());
    assert!(
        longer.is_some(),
        "a key named twice is the binder's refusal"
    );

    // A whole-row declaration answers `is_sorted`, `sort_indices` and
    // `into_sorted` under its options; any other options are read.
    let whole = quotes(&[("XNYS", 1), ("XNAS", 1), ("XNYS", 2)])
        .into_sorted(SortOptions::default())
        .expect("sorted");
    assert!(whole.is_sorted(SortOptions::default()));
    assert!(!whole.is_sorted(SortOptions::descending()));
    assert_eq!(
        whole
            .sort_indices(SortOptions::default())
            .expect("an order")
            .rows()
            .to_vec(),
        u32s(&[0, 1, 2])
    );
    let again = whole.into_sorted(SortOptions::default()).expect("sorted");
    assert!(shares_children(&again, &whole));
    let mut held = whole.clone();
    held.as_sorted(SortOptions::default()).expect("sorted");
    assert!(shares_children(&held, &whole));
    let descending = whole
        .into_sorted(SortOptions::descending())
        .expect("sorted");
    assert_eq!(
        sort_by(&descending).as_deref(),
        Some(r#"["venue desc","price desc"]"#)
    );
    // `venue, price desc` is no whole-row order: `is_sorted` reads it.
    assert!(!sorted.is_sorted(SortOptions::default()));
}

#[test]
fn the_verbs_that_keep_the_order_keep_the_declaration() {
    let sorted = declared_quotes();
    let mask = Serie::new([true, false, true, true].map(Scalar::from).to_vec());
    let increasing = Serie::new(u32s(&[0, 2, 3]));
    let mut filtered = sorted.clone();
    filtered.as_filtered(&mask).expect("filtered");
    let mut unique = sorted.clone();
    unique.as_unique().expect("unique");
    let mut taken = sorted.clone();
    taken.as_taken(&increasing).expect("taken");
    let mut spilled = sorted.clone();
    spilled
        .spill(&yggdryl::SpillOptions::new().with_byte_size(0))
        .expect("spilled");
    assert!(spilled.is_spilled());
    for (what, kept) in [
        ("slice", sorted.slice(1, 2).expect("a slice")),
        (
            "window",
            sorted.window(1, 3).expect("a window").into_serie(),
        ),
        (
            "into_filtered",
            sorted.into_filtered(&mask).expect("filtered"),
        ),
        ("as_filtered", filtered),
        ("into_unique", sorted.into_unique().expect("unique")),
        ("as_unique", unique),
        ("clone", sorted.clone()),
        ("spill", spilled),
        (
            "into_taken increasing",
            sorted.into_taken(&increasing).expect("taken"),
        ),
        ("as_taken increasing", taken),
    ] {
        assert_eq!(sort_by(&kept).as_deref(), Some(DECLARED), "{what}");
        assert!(
            lands_under(kept.field().expect("a record"), kept.rows().into_owned()),
            "{what}: the rows kept are in the order kept"
        );
    }
    // A pick that is not strictly increasing - out of order, or a row
    // twice - could break the order, so the declaration goes.
    for indices in [u32s(&[2, 0]), u32s(&[0, 0, 1]), u32s(&[3, 2, 1, 0])] {
        let picks = Serie::new(indices.clone());
        let taken = sorted.into_taken(&picks).expect("taken");
        assert!(sort_by(&taken).is_none(), "{indices:?}");
        let mut held = sorted.clone();
        held.as_taken(&picks).expect("taken");
        assert!(sort_by(&held).is_none(), "{indices:?}");
        assert_eq!(held.rows(), taken.rows(), "{indices:?}");
    }
}

#[test]
fn a_reversal_flips_every_key_its_direction_and_its_nulls() {
    let sorted = declared_quotes();
    let flipped = r#"["venue desc nulls first","price nulls first"]"#;
    let reversed = sorted.into_reversed();
    assert_eq!(sort_by(&reversed).as_deref(), Some(flipped));
    let mut held = sorted.clone();
    held.as_reversed().expect("reversed");
    assert_eq!(sort_by(&held).as_deref(), Some(flipped));
    assert_eq!(held.rows(), reversed.rows());
    // The flipped declaration is true of the reversed rows, and a second
    // reversal is the first declaration again.
    assert!(lands_under(
        reversed.field().expect("a record"),
        reversed.rows().into_owned()
    ));
    assert_eq!(
        sort_by(&reversed.into_reversed()).as_deref(),
        Some(DECLARED)
    );
    // A whole-row declaration flips to the whole-row order the other way.
    let whole = quotes(&[("XNYS", 1), ("XNAS", 1)])
        .into_sorted(SortOptions::default())
        .expect("sorted")
        .into_reversed();
    assert_eq!(
        sort_by(&whole).as_deref(),
        Some(r#"["venue desc nulls first","price desc nulls first"]"#)
    );
    assert!(whole.is_sorted(SortOptions::descending().with_nulls_first(true)));
}

/// Run `write` on [`declared_quotes`] and pin whether the declaration
/// stayed - and that it stays exactly where the rows still land under it.
fn after_write(what: &str, keeps: bool, write: impl FnOnce(&mut Serie) -> yggdryl::Result<()>) {
    let mut held = declared_quotes();
    write(&mut held).unwrap_or_else(|error| panic!("{what}: {error}"));
    assert_eq!(
        sort_by(&held).as_deref(),
        keeps.then_some(DECLARED),
        "{what}: {held}"
    );
    // The root as written, declaring the order again: its rows land under
    // it exactly where the write kept the declaration.
    let declaring = without_order(held.field())
        .expect("a record")
        .try_with_metadata("SORT:by", DECLARED)
        .expect("the declaration");
    assert_eq!(
        lands_under(&declaring, held.rows().into_owned()),
        keeps,
        "{what}: the rows written are in the declared order exactly when it is kept"
    );
}

#[test]
fn a_row_write_keeps_the_declaration_where_the_order_holds_and_clears_it_where_it_breaks() {
    after_write("set, between its neighbours", true, |serie| {
        serie.set(1, quote_row("XNAS", 2))
    });
    after_write("set, past its neighbour", false, |serie| {
        serie.set(1, quote_row("XNAS", 4))
    });
    after_write("set, the last row", false, |serie| {
        serie.set(3, quote_row("XNYS", 3))
    });
    after_write("push, after the last", true, |serie| {
        serie.push(quote_row("XNYS", 0))
    });
    after_write("push, before the last", false, |serie| {
        serie.push(quote_row("XNAS", 9))
    });
    after_write("insert, in its place", true, |serie| {
        serie.insert(2, quote_row("XNAS", 0))
    });
    after_write("insert, out of its place", false, |serie| {
        serie.insert(0, quote_row("XNYS", 9))
    });
    after_write("splice, in order", true, |serie| {
        serie.splice(1..3, vec![quote_row("XNAS", 2), quote_row("XNYS", 5)])
    });
    after_write("splice, out of order", false, |serie| {
        serie.splice(0..1, vec![quote_row("ZZZZ", 1)])
    });
    after_write(
        "splice, out of order within the rows written",
        false,
        |serie| serie.splice(4..4, vec![quote_row("XPAR", 1), quote_row("XPAR", 2)]),
    );
    after_write("extend, in order", true, |serie| {
        serie.extend(vec![quote_row("XNYS", 1), quote_row("XPAR", 9)])
    });
    after_write("extend, out of order", false, |serie| {
        serie.extend(vec![quote_row("XPAR", 1), quote_row("AAAA", 1)])
    });
    after_write("resize, growing with an equal row", true, |serie| {
        serie.resize(6, quote_row("XNYS", 1))
    });
    after_write("resize, growing out of order", false, |serie| {
        serie.resize(6, quote_row("AAAA", 1))
    });
    after_write("resize, shrinking", true, |serie| {
        serie.resize(2, quote_row("AAAA", 1))
    });
    let price = "price".parse::<FieldPath>().expect("a path");
    after_write("set_cell, in order", true, |serie| {
        serie.set_cell(&price, 0, Scalar::from(5_i64))
    });
    after_write("set_cell, out of order", false, |serie| {
        serie.set_cell(&price, 1, Scalar::from(9_i64))
    });
    let prices = |values: &[i64]| {
        Serie::from_scalars(DataType::Int64.required_field("price"), i64s(values))
            .expect("a price column")
    };
    after_write("set_child, the same key values", true, |serie| {
        serie.set_child(prices(&[9, 3, 7, 0]))
    });
    after_write("set_child, a key out of order", false, |serie| {
        serie.set_child(prices(&[1, 3, 2, 1]))
    });
    after_write("set_child, a child no key reads", true, |serie| {
        serie.set_child(
            Serie::from_scalars(DataType::Int64.required_field("qty"), i64s(&[4, 1, 3, 2]))
                .expect("a qty column"),
        )
    });
    // Removals leave rows in the order they were.
    after_write("remove", true, |serie| serie.remove(1).map(drop));
    after_write("pop", true, |serie| serie.pop().map(drop));
    after_write("truncate", true, |serie| serie.truncate(1));
    after_write("clear", true, Serie::clear);
}

#[test]
fn a_refused_write_leaves_the_declaration_as_it_was() {
    let mut held = declared_quotes();
    let before = held.rows().into_owned();
    assert!(held.set(0, Scalar::from(1_i64)).is_err());
    assert!(held.push(Scalar::from("XNAS")).is_err());
    assert_eq!(held.rows().into_owned(), before);
    assert_eq!(sort_by(&held).as_deref(), Some(DECLARED));
}

#[test]
fn extend_from_serie_trusts_an_order_the_other_declares_and_reads_any_other() {
    let declared = |rows: &[(&str, i64)]| {
        quotes(rows)
            .into_sort_by("venue, price desc")
            .expect("sorted")
    };
    // A serie declaring the same order: in order across the edge, and out
    // of it.
    after_write("a declaring serie, its edge in order", true, |serie| {
        serie.extend_from_serie(&declared(&[("XPAR", 5), ("XNYS", 0)]))
    });
    after_write("a declaring serie, its edge out of order", false, |serie| {
        serie.extend_from_serie(&declared(&[("AAAA", 1), ("ZZZZ", 1)]))
    });
    // A plain serie is read row by row.
    after_write("a plain serie in order", true, |serie| {
        serie.extend_from_serie(&quotes(&[("XNYS", 0), ("XPAR", 5)]))
    });
    after_write("a plain serie out of order", false, |serie| {
        serie.extend_from_serie(&quotes(&[("XPAR", 5), ("XNYS", 0)]))
    });
    // A serie declaring another order proves nothing about this one.
    after_write("a serie declaring another order", false, |serie| {
        serie.extend_from_serie(
            &quotes(&[("XPAR", 9), ("XPAR", 5)])
                .into_sort_by("price")
                .expect("sorted"),
        )
    });
    // A record declaring nothing takes a declaring serie's rows and still
    // declares nothing.
    let mut plain = quotes(&[("XNAS", 1)]);
    plain
        .extend_from_serie(&declared(&[("XNYS", 2)]))
        .expect("appended");
    assert!(sort_by(&plain).is_none());
}

#[test]
fn a_computed_key_is_cleared_by_any_row_write() {
    let quotes = quote_column(vec![
        quote(Some("XNAS"), 1, 0),
        quote(Some("XNAS"), 2, 14),
        quote(Some("XNYS"), 3, 15),
    ])
    .into_sort_by("minutes(ts, 15)")
    .expect("sorted");
    assert_eq!(sort_by(&quotes).as_deref(), Some(r#"["minutes(ts, 15)"]"#));
    // A row in the order the key states still clears it: a write does not
    // evaluate a computed key.
    let mut held = quotes.clone();
    held.push(quote(Some("XNYS"), 4, 31)).expect("pushed");
    assert!(sort_by(&held).is_none());
    let mut held = quotes.clone();
    held.set(2, quote(Some("XNYS"), 3, 16)).expect("set");
    assert!(sort_by(&held).is_none());
    // The verbs that move no row keep it.
    assert_eq!(
        sort_by(&quotes.slice(1, 2).expect("a slice")),
        sort_by(&quotes)
    );
    assert_eq!(
        sort_by(&quotes.into_reversed()).as_deref(),
        Some(r#"["minutes(ts, 15) desc nulls first"]"#)
    );
}

#[test]
fn a_removal_keeps_even_a_computed_key() {
    // Rows taken out of rows in order leave them in order, whatever the key
    // computes: a removal is no row write.
    let quotes = quote_column(vec![
        quote(Some("XNAS"), 1, 0),
        quote(Some("XNAS"), 2, 14),
        quote(Some("XNYS"), 3, 15),
    ])
    .into_sort_by("minutes(ts, 15)")
    .expect("sorted");
    type Removal = fn(&mut Serie) -> yggdryl::Result<()>;
    let removals: [(&str, Removal); 4] = [
        ("remove", |serie| serie.remove(1).map(drop)),
        ("pop", |serie| serie.pop().map(drop)),
        ("truncate", |serie| serie.truncate(1)),
        ("clear", Serie::clear),
    ];
    for (what, removal) in removals {
        let mut held = quotes.clone();
        removal(&mut held).expect(what);
        assert_eq!(sort_by(&held), sort_by(&quotes), "{what}");
    }
}

#[test]
fn an_absent_row_is_written_where_the_declaration_puts_absence() {
    // Absent rows sort last under the default: an absent row pushed after
    // every present one keeps the declaration, one inserted first breaks it.
    let quotes = quote_column(vec![
        quote(Some("XNYS"), 1, 0),
        quote(Some("XNAS"), 2, 14),
        quote(None, 3, 15),
    ])
    .into_sort_by("venue, count")
    .expect("sorted");
    assert_eq!(sort_by(&quotes).as_deref(), Some(r#"["venue","count"]"#));
    let declaring = quotes.field().expect("a record").clone();
    for (what, keeps, write) in [
        (
            "an absent row last",
            true,
            Box::new(|serie: &mut Serie| serie.push(Scalar::Null))
                as Box<dyn Fn(&mut Serie) -> yggdryl::Result<()>>,
        ),
        (
            "an absent row first",
            false,
            Box::new(|serie: &mut Serie| serie.insert(0, Scalar::Null)),
        ),
        (
            "an absent venue last",
            true,
            Box::new(|serie: &mut Serie| serie.push(quote(None, 9, 0))),
        ),
        (
            "an absent venue first",
            false,
            Box::new(|serie: &mut Serie| serie.insert(0, quote(None, 0, 0))),
        ),
    ] {
        let mut held = quotes.clone();
        write(&mut held).expect(what);
        assert_eq!(sort_by(&held).is_some(), keeps, "{what}: {held}");
        assert_eq!(
            lands_under(&declaring, held.rows().into_owned()),
            keeps,
            "{what}: kept exactly where the rows land under it"
        );
    }
}
