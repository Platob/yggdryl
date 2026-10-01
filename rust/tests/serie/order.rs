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
