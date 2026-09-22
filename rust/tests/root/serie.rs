//! `rust/src/serie.rs`: the root over a schema-free run and the column
//! leaves - one collection API, identity by rows, the wires it writes.

use std::borrow::Cow;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, Int32Array, Int64Array};
use yggdryl::{DataType, Field, FieldPath, Int32Serie, Run, Scalar, Serie, StructType, Value};

/// Three prices as a schema-free run.
fn run() -> Serie {
    Serie::new(vec![
        Scalar::from(125_i64),
        Scalar::from(126_i64),
        Scalar::from(127_i64),
    ])
}

/// The same three prices as an int64 column.
fn column() -> Serie {
    let array: ArrayRef = Arc::new(Int64Array::from(vec![125, 126, 127]));
    Serie::from_arrow_array(Field::new("price", DataType::Int64, false), array)
        .expect("an int64 column")
}

/// The int32 column of `values` under `name`.
fn int32_column(name: &str, values: Vec<i32>) -> Serie {
    let array: ArrayRef = Arc::new(Int32Array::from(values));
    Serie::from_arrow_array(Field::new(name, DataType::Int32, false), array)
        .expect("an int32 column")
}

/// A record column of two quotes: an identifier and a nested venue record.
fn quotes() -> Serie {
    let venue = Field::new(
        "venue",
        DataType::from(
            StructType::from_fields([
                Field::new("mic", DataType::utf8(), false),
                Field::new("tier", DataType::Int32, false),
            ])
            .unwrap(),
        ),
        false,
    );
    let root = Field::new(
        "row",
        DataType::from(
            StructType::from_fields([Field::new("id", DataType::Int64, false), venue]).unwrap(),
        ),
        false,
    );
    Serie::from_scalars(
        root,
        [
            Scalar::from_sequence([
                Scalar::from(1_i64),
                Scalar::from_sequence([Scalar::from("XNAS"), Scalar::from(1_i32)]),
            ]),
            Scalar::from_sequence([
                Scalar::from(2_i64),
                Scalar::from_sequence([Scalar::from("XNYS"), Scalar::from(2_i32)]),
            ]),
        ],
    )
    .expect("two quote rows")
}

/// A record column of two orders, each a list of legs (records of a price)
/// and a mapping of tags, so a schema path crosses all three nestings.
fn orders() -> Serie {
    let leg = Field::new(
        "item",
        DataType::from(
            StructType::from_fields([Field::new("price", DataType::Int64, false)]).unwrap(),
        ),
        false,
    );
    let entries = Field::new(
        "entries",
        DataType::from(
            StructType::from_fields([
                Field::new("key", DataType::utf8(), false),
                Field::new("value", DataType::Int64, true),
            ])
            .unwrap(),
        ),
        false,
    );
    let root = Field::new(
        "order",
        DataType::from(
            StructType::from_fields([
                Field::new("id", DataType::Int64, false),
                Field::new("legs", DataType::list(leg), false),
                Field::new("tags", DataType::map(entries, false).unwrap(), true),
            ])
            .unwrap(),
        ),
        false,
    );
    Serie::from_scalars(
        root,
        [
            Scalar::from_sequence([
                Scalar::from(1_i64),
                Scalar::from_sequence([
                    Scalar::from_sequence([Scalar::from(10_i64)]),
                    Scalar::from_sequence([Scalar::from(11_i64)]),
                ]),
                Scalar::from_mapping([(Scalar::from("venue"), Scalar::from(1_i64))]).unwrap(),
            ]),
            Scalar::from_sequence([
                Scalar::from(2_i64),
                Scalar::from_sequence([Scalar::from_sequence([Scalar::from(20_i64)])]),
                Scalar::Null,
            ]),
        ],
    )
    .expect("two order rows")
}

/// The hash a value writes into the default hasher.
fn hashed(value: &impl Hash) -> u64 {
    let mut state = DefaultHasher::new();
    value.hash(&mut state);
    state.finish()
}

#[test]
fn a_run_declares_no_field_and_a_column_carries_one() {
    let run = run();
    assert_eq!(run.field(), None);
    assert_eq!(run.field_ref(), None);
    assert!(!run.is_column());
    assert!(run.as_run().is_some());
    assert_eq!(run.as_slice().map(<[Scalar]>::len), Some(3));
    assert!(run.into_arrow_array().is_none());
    let refusal = run.require_field().expect_err("a run declares no field");
    assert!(refusal.to_string().contains("schema-free run"), "{refusal}");
    let refusal = run
        .require_arrow_array()
        .expect_err("a run names no layout");
    assert!(refusal.to_string().contains("schema-free run"), "{refusal}");

    let column = column();
    assert_eq!(column.field().map(Field::name), Some("price"));
    assert_eq!(column.field_ref().map(|field| field.name()), Some("price"));
    assert!(column.is_column());
    assert!(column.as_run().is_none());
    assert_eq!(column.as_slice(), None);
    assert!(column.into_arrow_array().is_some());
    assert_eq!(column.require_field().unwrap().name(), "price");
    assert_eq!(column.require_arrow_array().unwrap().len(), 3);
    assert_eq!(Serie::default().len(), 0);
    assert!(!Serie::default().is_column());
}

#[test]
fn every_constructor_answers_the_leaf_it_names() {
    // A run from values in hand, from an iterator, and from the run type.
    assert_eq!(Serie::new(vec![Scalar::from(1_i64)]).len(), 1);
    let collected: Serie = [Scalar::from(1_i64), Scalar::from(2_i64)]
        .into_iter()
        .collect();
    assert_eq!(collected.as_slice().map(<[Scalar]>::len), Some(2));
    let held = Run::new(vec![Scalar::from(1_i64)]);
    assert_eq!(Serie::from(held.clone()).as_run(), Some(&held));

    // A column: empty, reserved, or laid out from proven rows.
    let empty = Serie::empty(Field::new("price", DataType::Int64, false)).unwrap();
    assert!(empty.is_empty());
    assert!(empty.is_column());
    assert_eq!(empty.field().map(Field::name), Some("price"));
    let reserved = Serie::with_capacity(Field::new("price", DataType::Int64, true), 16).unwrap();
    assert!(reserved.is_empty());
    assert!(reserved.as_int64().is_some());
    let laid_out = Serie::from_scalars(
        Field::new("price", DataType::Int64, false),
        [Scalar::from(125_i8), Scalar::from(126_i16)],
    )
    .expect("rows the field rewrites");
    assert_eq!(laid_out.as_int64().unwrap().values(), &[125, 126]);
    let refusal = Serie::from_scalars(
        Field::new("price", DataType::Int64, false),
        [Scalar::from("AAPL")],
    )
    .expect_err("text is not a price");
    assert!(refusal.to_string().contains("price"), "{refusal}");

    // A family widens into the root, and the root into a scalar.
    let family = laid_out.as_integer().expect("an integer column").clone();
    assert_eq!(Serie::from(family), laid_out);
    assert_eq!(Scalar::from(laid_out.clone()), Scalar::Sequence(laid_out));
}

#[test]
fn both_leaves_answer_the_same_reads_at_different_costs() {
    for serie in [run(), column()] {
        assert_eq!(serie.len(), 3);
        assert!(!serie.is_empty());
        assert_eq!(serie.null_count(), 0);
        assert!(!serie.is_null(0).unwrap());
        assert_eq!(serie.scalar(2).unwrap(), Scalar::from(127_i64));
        assert_eq!(serie.get(1).as_deref(), Some(&Scalar::from(126_i64)));
        assert_eq!(serie.get(3), None);
        assert_eq!(serie.rows().len(), 3);
        assert_eq!(serie.iter().len(), 3);
        assert_eq!(
            serie.iter().next_back().map(Cow::into_owned),
            Some(Scalar::from(127_i64))
        );
        assert_eq!(
            (&serie)
                .into_iter()
                .map(Cow::into_owned)
                .collect::<Vec<_>>(),
            serie.rows().as_ref()
        );
        assert_eq!(
            serie.dtype().unwrap(),
            DataType::list(Field::new("item", DataType::Int64, false))
        );
        assert_eq!(serie.clone().into_run().as_slice().len(), 3);
        assert!(serie.children().is_empty());
        assert!(serie.child_at(0).is_none());
        assert!(serie.items().is_none());
    }

    // A run lends its rows; a column builds them.
    assert!(matches!(run().get(0), Some(Cow::Borrowed(_))));
    assert!(matches!(column().get(0), Some(Cow::Owned(_))));
    assert!(matches!(run().iter().next(), Some(Cow::Borrowed(_))));
    assert!(matches!(column().iter().next(), Some(Cow::Owned(_))));
    assert!(matches!(run().rows(), Cow::Borrowed(_)));
    assert!(matches!(column().rows(), Cow::Owned(_)));

    // A run walks its values for a count a column reads off its bitmap.
    let sparse = Serie::new(vec![Scalar::Null, Scalar::from(1_i64), Scalar::Null]);
    assert_eq!(sparse.null_count(), 2);
    assert!(sparse.is_null(0).unwrap());
    let sparse = Serie::from_scalars(
        Field::new("price", DataType::Int64, true),
        sparse.rows().into_owned(),
    )
    .unwrap();
    assert_eq!(sparse.null_count(), 2);
    assert!(sparse.is_null(2).unwrap());
    assert_eq!(sparse.scalar(0).unwrap(), Scalar::Null);

    // An empty run names a null item; an empty column names its field's.
    assert_eq!(
        Serie::new(Vec::<Scalar>::new()).dtype().unwrap(),
        DataType::list(Field::new("item", DataType::Null, true))
    );
    assert_eq!(
        Serie::empty(Field::new("price", DataType::Int64, true))
            .unwrap()
            .dtype()
            .unwrap(),
        DataType::list(Field::new("item", DataType::Int64, true))
    );
}

#[test]
fn a_row_past_the_end_is_refused_naming_the_serie_and_both_counts() {
    let column = column();
    let refusal = column.scalar(3).expect_err("row 3 is past three rows");
    let text = refusal.to_string();
    assert!(text.contains("price"), "names the column: {text}");
    assert!(text.contains("row 3"), "names the row: {text}");
    assert!(text.contains("3 rows"), "names the length: {text}");
    assert!(column.is_null(3).is_err());
    assert_eq!(column.get(3), None);
    assert_eq!(column.get(usize::MAX), None);

    // A run has no field to name, so its refusal names the root path.
    let run = run();
    let refusal = run.scalar(3).expect_err("row 3 is past three rows");
    let text = refusal.to_string();
    assert!(text.contains("row 3"), "names the row: {text}");
    assert!(text.contains("3 rows"), "names the length: {text}");
    assert!(run.is_null(3).is_err());
    assert_eq!(run.get(3), None);
}

#[test]
fn a_run_and_a_column_of_equal_rows_are_one_value_and_hash_alike() {
    let (run, column) = (run(), column());
    assert_eq!(run, column);
    assert_eq!(column, run);
    assert_eq!(hashed(&run), hashed(&column));
    assert_eq!(run.cmp(&column), std::cmp::Ordering::Equal);

    // An int32 column of the same numbers is the same value again, exactly
    // as its scalars are.
    let narrow = int32_column("n", vec![125, 126, 127]);
    assert_eq!(narrow, column);
    assert_eq!(hashed(&narrow), hashed(&column));

    // Order is the rows, then the length: nothing else.
    let shorter = Serie::new(vec![Scalar::from(125_i64), Scalar::from(126_i64)]);
    assert!(shorter < run);
    assert!(shorter < column);
    let larger = Serie::new(vec![Scalar::from(126_i64)]);
    assert!(larger > column);
    assert_eq!(
        Scalar::Sequence(column.clone()),
        Scalar::from_sequence(run.rows().iter().cloned())
    );
    assert_eq!(
        hashed(&Scalar::Sequence(column)),
        hashed(&Scalar::Sequence(run))
    );
}

#[test]
fn two_int32_columns_of_equal_rows_under_fields_a_and_b_are_one_value_and_hash_alike() {
    let under_a = int32_column("a", vec![1, 2, 3]);
    let under_b = int32_column("b", vec![1, 2, 3]);
    assert_ne!(
        under_a.field().map(Field::name),
        under_b.field().map(Field::name)
    );

    assert_eq!(under_a, under_b);
    assert_eq!(hashed(&under_a), hashed(&under_b));
    assert_eq!(under_a.cmp(&under_b), std::cmp::Ordering::Equal);

    // The rows are all that counts: one row apart and they are not.
    let under_c = int32_column("a", vec![1, 2, 4]);
    assert_ne!(under_a, under_c);
    assert!(under_a < under_c);
}

#[test]
fn a_sorted_vec_of_int32_leaves_agrees_with_the_sorted_vec_of_series_holding_them() {
    let columns: Vec<Serie> = [vec![3, 1], vec![1, 2, 3], vec![], vec![1, 2]]
        .into_iter()
        .map(|values| int32_column("n", values))
        .collect();
    let mut leaves: Vec<Int32Serie> = columns
        .iter()
        .map(|column| column.as_int32().expect("an int32 column").clone())
        .collect();
    let mut roots = columns;

    leaves.sort();
    roots.sort();

    let by_leaf: Vec<Vec<i32>> = leaves.iter().map(|leaf| leaf.values().to_vec()).collect();
    let by_root: Vec<Vec<i32>> = roots
        .iter()
        .map(|root| root.as_int32().expect("an int32 column").values().to_vec())
        .collect();
    assert_eq!(by_leaf, by_root);
    assert_eq!(by_leaf, vec![vec![], vec![1, 2], vec![1, 2, 3], vec![3, 1]]);

    // And the leaf orders as the root does: the rows, then the length.
    for (leaf, root) in leaves.iter().zip(&roots) {
        assert_eq!(leaf.cmp(&leaves[0]), root.cmp(&roots[0]));
    }
}

#[test]
fn a_run_and_a_column_render_differently_and_a_column_debugs_as_its_shape() {
    assert_eq!(
        run().to_string(),
        format!("{:?}", run().as_slice().unwrap())
    );
    let column = column();
    let shown = column.to_string();
    assert!(shown.starts_with("price["), "{shown}");
    assert!(shown.ends_with(']'), "{shown}");
    let debugged = format!("{column:?}");
    assert!(debugged.starts_with("Int64Serie {"), "{debugged}");
    assert!(debugged.contains("len: 3"), "{debugged}");
    assert!(debugged.contains("nulls: 0"), "{debugged}");
}

#[test]
fn every_write_is_spelled_over_splice_on_both_leaves() {
    for mut serie in [run(), column()] {
        serie.push(Scalar::from(128_i64)).unwrap();
        serie.insert(0, Scalar::from(124_i64)).unwrap();
        serie.set(1, Scalar::from(1_i64)).unwrap();
        assert_eq!(serie.remove(1).unwrap(), Scalar::from(1_i64));
        assert_eq!(serie.pop().unwrap(), Some(Scalar::from(128_i64)));
        serie
            .extend(vec![Scalar::from(200_i64), Scalar::from(201_i64)])
            .unwrap();
        serie.truncate(4).unwrap();
        serie.truncate(9).unwrap();
        serie.splice(1..3, vec![Scalar::from(0_i64)]).unwrap();
        serie.resize(5, Scalar::from(9_i64)).unwrap();
        assert_eq!(
            serie.rows().as_ref(),
            &[
                Scalar::from(124_i64),
                Scalar::from(0_i64),
                Scalar::from(200_i64),
                Scalar::from(9_i64),
                Scalar::from(9_i64),
            ]
        );
        serie.resize(2, Scalar::Null).unwrap();
        assert_eq!(serie.len(), 2);

        // Past the end, reversed, or before a row that is not there.
        assert!(serie.set(2, Scalar::from(1_i64)).is_err());
        assert!(serie.insert(3, Scalar::from(1_i64)).is_err());
        assert!(serie.remove(2).is_err());
        let backwards = (2, 1);
        assert!(serie.splice(backwards.0..backwards.1, vec![]).is_err());
        assert!(serie.splice(1..3, vec![]).is_err());
        assert_eq!(serie.len(), 2);

        serie.clear().unwrap();
        assert!(serie.is_empty());
        assert_eq!(serie.pop().unwrap(), None);
    }

    // A column proves through its field and keeps what it had; a run
    // accepts anything.
    let mut column = column();
    let refusal = column
        .push(Scalar::from("AAPL"))
        .expect_err("text is not a price");
    assert!(refusal.to_string().contains("price"), "{refusal}");
    assert!(column.resize(5, Scalar::Null).is_err());
    assert!(
        column
            .extend(vec![Scalar::from(1_i64), Scalar::Null])
            .is_err()
    );
    assert_eq!(column.as_int64().unwrap().values(), &[125, 126, 127]);
    let mut run = run();
    run.push(Scalar::from("AAPL")).unwrap();
    run.resize(6, Scalar::Null).unwrap();
    assert_eq!(run.len(), 6);
    assert_eq!(run.null_count(), 2);
}

#[test]
fn a_write_through_a_shared_column_copies_it_once_and_leaves_the_other_holder_alone() {
    let column = column();
    let mut written = column.clone();

    // The root's writer and the typed writer both go through `make_mut`.
    written.push(Scalar::from(128_i64)).unwrap();
    written
        .get_int64_mut()
        .expect("an int64 column")
        .set_value(0, Some(1))
        .unwrap();
    assert_eq!(written.as_int64().unwrap().values(), &[1, 126, 127, 128]);
    assert_eq!(column.as_int64().unwrap().values(), &[125, 126, 127]);
    assert!(written.get_int32_mut().is_none());
    assert!(run().get_int64_mut().is_none());
}

#[test]
fn extending_from_another_serie_appends_buffers_where_the_fields_agree() {
    let mut column = column();
    let other = column.clone();
    let before = other.as_int64().unwrap().values().as_ptr();
    column.extend_from_serie(&other).unwrap();
    assert_eq!(
        column.as_int64().unwrap().values(),
        &[125, 126, 127, 125, 126, 127]
    );
    // The source keeps the buffers it lent, exactly where they were.
    assert_eq!(other.as_int64().unwrap().values().as_ptr(), before);
    assert_eq!(other.len(), 3);

    // A nullable source appends into a nullable target buffer to buffer,
    // absence included.
    let mut sparse = Serie::from_scalars(
        Field::new("price", DataType::Int64, true),
        [Scalar::from(1_i64), Scalar::Null],
    )
    .unwrap();
    sparse.extend_from_serie(&sparse.clone()).unwrap();
    assert_eq!(sparse.len(), 4);
    assert_eq!(sparse.null_count(), 2);
    assert_eq!(sparse.as_int64().unwrap().values(), &[1, 0, 1, 0]);

    // A run's rows are read; a column of another datatype is read too.
    column.extend_from_serie(&run()).unwrap();
    assert_eq!(column.len(), 9);
    column
        .extend_from_serie(&int32_column("n", vec![1]))
        .unwrap();
    assert_eq!(column.scalar(9).unwrap(), Scalar::from(1_i64));

    // A nullable source under a required target is read, and refused by
    // the target's own contract.
    let absent =
        Serie::from_scalars(Field::new("price", DataType::Int64, true), [Scalar::Null]).unwrap();
    assert!(column.extend_from_serie(&absent).is_err());
    assert_eq!(column.len(), 10);

    let mut run = run();
    run.extend_from_serie(&column).unwrap();
    assert_eq!(run.len(), 13);
}

#[test]
fn a_record_column_lends_its_children_by_name_position_and_path() {
    let quotes = quotes();
    assert_eq!(quotes.children().len(), 2);
    assert_eq!(
        quotes.child("id").and_then(Serie::field).map(Field::name),
        Some("id")
    );
    assert_eq!(
        quotes.child_at(1).and_then(Serie::field).map(Field::name),
        Some("venue")
    );
    assert!(quotes.child("volume").is_none());
    assert!(quotes.child_at(2).is_none());
    assert!(quotes.items().is_none());

    let tier = quotes
        .get_child_by_path(&FieldPath::from_str("venue.tier").unwrap())
        .expect("two levels down");
    assert_eq!(tier.as_int32().unwrap().values(), &[1, 2]);
    assert!(
        quotes
            .get_child_by_path(&FieldPath::from_str("venue.size").unwrap())
            .is_none()
    );
    assert!(
        quotes
            .get_child_by_path(&FieldPath::from_str("[0]").unwrap())
            .is_none()
    );
    assert!(quotes.get_child_by_path(&FieldPath::root()).is_none());

    // A run and a leaf column have no children.
    assert!(run().children().is_empty());
    assert!(column().child("id").is_none());
    assert!(
        run()
            .get_child_by_path(&FieldPath::from_str("id").unwrap())
            .is_none()
    );
}

#[test]
fn a_schema_path_sees_through_a_list_and_addresses_a_mapping_through_its_entries() {
    let orders = orders();
    let legs = orders.child("legs").expect("a list child");
    let legs_items = legs.items().expect("a list column has items");
    assert_eq!(legs_items.len(), 3);
    assert_eq!(
        legs_items.field().map(Field::name),
        Some("item"),
        "the item column carries the item field"
    );

    // A list is transparent to its item: `legs.price` and `legs.item.price`
    // reach the one price column under the item record.
    let prices = orders
        .get_child_by_path(&FieldPath::from_str("legs.price").unwrap())
        .expect("through the list");
    assert_eq!(prices.as_int64().unwrap().values(), &[10, 11, 20]);
    let spelled = orders
        .get_child_by_path(&FieldPath::from_str("legs.item.price").unwrap())
        .expect("the item named");
    assert!(std::ptr::eq(prices, spelled));
    assert!(std::ptr::eq(
        orders
            .get_child_by_path(&FieldPath::from_str("legs.item").unwrap())
            .unwrap(),
        legs_items
    ));
    assert!(
        orders
            .get_child_by_path(&FieldPath::from_str("legs[0]").unwrap())
            .is_none()
    );

    // A mapping is addressed through its entries field, and nothing else.
    let tags = orders.child("tags").expect("a mapping child");
    let entries = tags.items().expect("a mapping column has entries");
    assert_eq!(entries.field().map(Field::name), Some("entries"));
    assert!(std::ptr::eq(
        orders
            .get_child_by_path(&FieldPath::from_str("tags.entries").unwrap())
            .unwrap(),
        entries
    ));
    let keys = orders
        .get_child_by_path(&FieldPath::from_str("tags.entries.key").unwrap())
        .expect("the keys under the entries");
    assert_eq!(keys.scalar(0).unwrap(), Scalar::from("venue"));
    assert!(
        orders
            .get_child_by_path(&FieldPath::from_str("tags.key").unwrap())
            .is_none()
    );
    assert!(
        orders
            .get_child_by_path(&FieldPath::from_str("tags['venue']").unwrap())
            .is_none()
    );
    assert_eq!(tags.null_count(), 1);
    // A record row reads as an ordered run, so the absent mapping is the
    // third cell of the second row.
    assert_eq!(
        orders.scalar(1).unwrap().get(2).as_deref(),
        Some(&Scalar::Null)
    );
}

#[test]
fn a_cell_three_levels_deep_is_written_in_place_and_a_child_is_replaced_whole() {
    let mut quotes = quotes();
    quotes
        .set_cell(
            &FieldPath::from_str("venue.tier").unwrap(),
            1,
            Scalar::from(3_i8),
        )
        .expect("one cell, proven by the leaf's field");
    assert_eq!(
        quotes
            .get_child_by_path(&FieldPath::from_str("venue.tier").unwrap())
            .unwrap()
            .as_int32()
            .unwrap()
            .values(),
        &[1, 3]
    );
    assert!(
        quotes
            .set_cell(
                &FieldPath::from_str("venue.tier").unwrap(),
                2,
                Scalar::from(3_i8)
            )
            .is_err()
    );
    assert!(
        quotes
            .set_cell(
                &FieldPath::from_str("venue").unwrap(),
                0,
                Scalar::from(3_i8)
            )
            .is_err()
    );
    assert!(
        quotes
            .set_cell(&FieldPath::from_str("id.x").unwrap(), 0, Scalar::from(3_i8))
            .is_err()
    );

    let volumes: ArrayRef = Arc::new(Int64Array::from(vec![10, 20]));
    let volume =
        Serie::from_arrow_array(Field::new("volume", DataType::Int64, false), volumes).unwrap();
    quotes.set_child(volume).expect("a child added");
    assert_eq!(quotes.children().len(), 3);
    assert_eq!(quotes.field().unwrap().fields().len(), 3);
    assert_eq!(
        quotes.scalar(0).unwrap(),
        Scalar::from_sequence([
            Scalar::from(1_i64),
            Scalar::from_sequence([Scalar::from("XNAS"), Scalar::from(1_i32)]),
            Scalar::from(10_i64),
        ])
    );

    let short: ArrayRef = Arc::new(Int64Array::from(vec![1]));
    let short = Serie::from_arrow_array(Field::new("bid", DataType::Int64, false), short).unwrap();
    assert!(quotes.set_child(short).is_err());
    assert!(quotes.set_child(run()).is_err());
    assert!(column().set_child(quotes.clone()).is_err());
    assert!(run().set_child(quotes.clone()).is_err());
    assert!(
        run()
            .set_cell(&FieldPath::from_str("id").unwrap(), 0, Scalar::Null)
            .is_err()
    );
    assert_eq!(quotes.children().len(), 3);
}

#[test]
fn a_slice_is_zero_copy_for_a_column_and_a_copy_for_a_run() {
    let column = column();
    let window = column.slice(1, 2).unwrap();
    assert_eq!(window.as_int64().unwrap().values(), &[126, 127]);
    // The window lends the original's buffer, one row in.
    let whole = column.as_int64().unwrap().values().as_ptr();
    let start = window.as_int64().unwrap().values().as_ptr();
    assert_eq!(start, whole.wrapping_add(1));
    assert_eq!(window.field().map(Field::name), Some("price"));
    assert!(column.slice(2, 2).is_err());
    assert!(column.slice(usize::MAX, 1).is_err());
    assert!(column.slice(3, 0).unwrap().is_empty());

    // A sliced record column slices every child to the window.
    let quotes = quotes();
    let second = quotes.slice(1, 1).unwrap();
    assert_eq!(second.len(), 1);
    assert_eq!(second.child("id").map(Serie::len), Some(1));
    assert_eq!(second.scalar(0).unwrap(), quotes.scalar(1).unwrap());

    let run = run();
    let window = run.slice(1, 2).unwrap();
    assert_eq!(window.rows().as_ref(), &run.rows()[1..]);
    assert!(run.slice(3, 1).is_err());
}

#[test]
fn a_sliced_column_grows_on_its_own_once_the_original_is_dropped() {
    let column = column();
    let mut window = column.slice(1, 2).unwrap();
    drop(column);

    window.push(Scalar::from(128_i64)).expect("a fourth price");
    window.set(0, Scalar::from(1_i64)).expect("a slot");
    assert_eq!(window.as_int64().unwrap().values(), &[1, 127, 128]);
    assert_eq!(window.len(), 3);
    let expected: ArrayRef = Arc::new(Int64Array::from(vec![1, 127, 128]));
    assert_eq!(
        window.into_arrow_array().unwrap().as_ref(),
        expected.as_ref()
    );
}

#[test]
fn a_run_serializes_as_its_values_and_a_column_as_its_field_beside_its_rows() {
    // The root's own serde: a run is a list, a column is the wire.
    let listed = serde_json::to_string(&run()).unwrap();
    assert!(listed.starts_with('['), "{listed}");
    let wire = serde_json::to_string(&column()).unwrap();
    assert!(wire.contains("\"field\""), "{wire}");
    assert!(wire.contains("\"rows\""), "{wire}");
    let back: Serie = serde_json::from_str(&wire).unwrap();
    assert_eq!(back, column());
    assert_eq!(back.field().map(Field::name), Some("price"));
    assert!(back.is_column());

    // A list is not a column wire: a run is never spelled through the
    // root's own serde, and the refusal names the tag.
    let refusal = serde_json::from_str::<Serie>(&listed).expect_err("a list is a run");
    assert!(refusal.to_string().contains("`serie`"), "{refusal}");
    assert!(serde_json::from_str::<Serie>("[]").is_err());
    let refusal = serde_json::from_str::<Scalar>(r#"{"type":"serie","value":[125,126]}"#)
        .expect_err("a list under the column tag");
    assert!(refusal.to_string().contains("`serie`"), "{refusal}");

    // Under a scalar, the tag tells the two apart both ways.
    let run = Scalar::Sequence(run());
    let document = serde_json::to_string(&run).unwrap();
    assert!(document.contains("\"sequence\""), "{document}");
    let back: Scalar = serde_json::from_str(&document).unwrap();
    assert_eq!(back, run);
    assert!(back.as_serie().is_some_and(|serie| !serie.is_column()));

    let column = Scalar::Sequence(column());
    let document = serde_json::to_string(&column).unwrap();
    assert!(document.contains("\"serie\""), "{document}");
    let back: Scalar = serde_json::from_str(&document).unwrap();
    assert_eq!(back, column);
    assert!(back.as_serie().is_some_and(Serie::is_column));
    assert_eq!(back.kind(), "sequence");
    assert!(serde_json::from_str::<Scalar>(r#"{"serie":[125,126]}"#).is_err());

    // The wire carries the field's contract: a row it refuses does not read.
    let mut forged = serde_json::to_value(column.as_serie().unwrap()).unwrap();
    forged["rows"] = serde_json::to_value([Scalar::from("AAPL")]).unwrap();
    let refusal = serde_json::from_value::<Serie>(forged).expect_err("text is not a price");
    assert!(refusal.to_string().contains("price"), "{refusal}");
}

#[test]
fn the_root_is_a_value_and_converts_from_and_into_its_neighbours() {
    let serie = column();
    assert_eq!(Value::dtype(&serie).unwrap(), serie.dtype().unwrap());
    assert_eq!(Scalar::from(serie.clone()), Scalar::Sequence(serie.clone()));
    assert_eq!(
        <Serie as Value>::from_scalar(&Scalar::Sequence(serie.clone())),
        Some(&serie)
    );
    assert_eq!(<Serie as Value>::from_scalar(&Scalar::Null), None);
    assert_eq!(Scalar::Sequence(serie.clone()).as_serie(), Some(&serie));
    assert_eq!(Scalar::Sequence(serie.clone()).len(), 3);
    assert_eq!(
        Scalar::Sequence(serie.clone()).get(2).as_deref(),
        Some(&Scalar::from(127_i64))
    );

    // A leaf narrows and widens through the root.
    let leaf = serie.as_int64().expect("an int64 column").clone();
    assert_eq!(
        <yggdryl::Int64Serie as yggdryl::SerieValue>::from_serie(&serie),
        Some(&leaf)
    );
    assert_eq!(yggdryl::SerieValue::into_serie(leaf), serie);
    assert!(serie.as_integer().is_some());
    assert!(serie.as_floating().is_none());
    assert!(serie.as_utf8().is_none());
    assert!(serie.as_struct().is_none());
    assert!(run().as_integer().is_none());
    assert_eq!(Serie::from(serie.as_integer().unwrap().clone()), serie);
}
