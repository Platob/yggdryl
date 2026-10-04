//! `rust/src/serie/lit.rs`: the constant column - one field, one value, a
//! length - laid out only when something exports it.

use yggdryl::{
    ArrowCastOptions, ArrowCastPlan, DataType, Field, Scalar, Serie, SortOptions, SpillOptions,
    StructType,
};

fn venue() -> Field {
    DataType::utf8().required_field("venue")
}

fn lit(rows: usize) -> Serie {
    Serie::lit(venue(), Scalar::from("XNAS"), rows).expect("a constant column")
}

fn laid_out(rows: usize) -> Serie {
    Serie::from_scalars(venue(), std::iter::repeat_n(Scalar::from("XNAS"), rows)).expect("rows")
}

#[test]
fn a_lit_holds_one_value_once_and_reads_every_row_as_it() {
    let column = lit(1_000_000);
    assert_eq!(column.len(), 1_000_000);
    assert_eq!(column.null_count(), 0);
    assert_eq!(column.scalar(0).unwrap(), Scalar::from("XNAS"));
    assert_eq!(column.scalar(999_999).unwrap(), Scalar::from("XNAS"));
    assert!(column.scalar(1_000_000).is_err(), "past the end");
    assert_eq!(column.field(), Some(&venue()));
    let held = column.as_lit().expect("a lit");
    assert_eq!(held.value(), &Scalar::from("XNAS"));
    assert!(!held.is_built(), "nothing is laid out until exported");
    assert!(
        column.resident_size() < 1_024,
        "one row resident, not a million: {}",
        column.resident_size()
    );
    assert!(
        column.memory_size() >= 1_000_000,
        "the laid-out estimate counts every row"
    );
    assert!(
        !column.as_lit().unwrap().is_built(),
        "sizing the column builds nothing"
    );
    assert!(
        column.resident_size() < 1_024,
        "and the one row is still all that is resident: {}",
        column.resident_size()
    );
    assert!(!column.is_spilled());
    // A typed narrowing lays the column out once, as the leaf it exports
    // as, so a reader of buffers never sees the constant.
    let small = lit(16);
    assert!(!small.as_lit().unwrap().is_built());
    assert_eq!(
        small.as_utf8().expect("laid out on demand").value(3),
        Some("XNAS")
    );
    assert!(small.as_lit().unwrap().is_built());
    assert!(small.as_lit().is_some(), "and it stays a constant column");
    assert_eq!(lit(16), laid_out(16), "identity is the rows alone");
}

#[test]
fn the_value_is_proven_by_the_field() {
    assert!(
        Serie::lit(venue(), Scalar::Null, 3).is_err(),
        "a required field holds no null"
    );
    let absent = Serie::lit(venue().with_nullable(true), Scalar::Null, 3).expect("a nullable null");
    assert_eq!(absent.null_count(), 3);
    assert!(absent.is_null(1).unwrap());
    // A text field reads an integer as its text; a sequence it cannot read.
    assert_eq!(
        Serie::lit(venue(), Scalar::from(1_i64), 3)
            .unwrap()
            .scalar(0)
            .unwrap(),
        Scalar::from("1")
    );
    assert!(
        Serie::lit(venue(), Scalar::from_sequence([Scalar::from(1_i64)]), 3).is_err(),
        "a value no reading accepts is refused"
    );
    // The value lands canonical under the field: a narrower integer widens.
    let count = Serie::lit(DataType::Int64.required_field("n"), Scalar::from(7_i32), 2).unwrap();
    assert_eq!(count.scalar(1).unwrap(), Scalar::from(7_i64));
    assert_eq!(
        Serie::lit(venue(), Scalar::from("XNAS"), 0).unwrap().len(),
        0
    );
}

#[test]
fn the_array_is_built_once_and_shared_by_the_slices_of_a_built_column() {
    let column = lit(5);
    let before = column.slice(1, 2).unwrap();
    assert!(
        !before.as_lit().unwrap().is_built(),
        "a slice of an unbuilt column builds nothing"
    );
    let first = column.into_arrow_array().unwrap();
    let second = column.into_arrow_array().unwrap();
    assert!(column.as_lit().unwrap().is_built());
    assert_eq!(
        first.to_data().buffers()[0].as_ptr(),
        second.to_data().buffers()[0].as_ptr(),
        "laid out once, every export shares its buffers"
    );
    let slice = column.slice(1, 3).unwrap();
    assert!(
        slice.as_lit().unwrap().is_built(),
        "a slice of a built column shares its buffers"
    );
    assert_eq!(slice.into_arrow_array().unwrap().len(), 3);
    assert_eq!(&*first, &*laid_out(5).into_arrow_array().unwrap());
    assert_eq!(
        &*lit(0).into_arrow_array().unwrap(),
        &*laid_out(0).into_arrow_array().unwrap()
    );
}

#[test]
fn a_spill_forgets_the_built_array_and_writes_nothing() {
    let mut column = lit(1_000);
    let _ = column.into_arrow_array();
    let built = column.resident_size();
    column
        .spill(&SpillOptions::new().with_byte_size(0))
        .unwrap();
    assert!(!column.as_lit().unwrap().is_built());
    assert!(column.resident_size() < built);
    assert!(!column.is_spilled(), "a constant is never on disk");
    assert_eq!(column.scalar(3).unwrap(), Scalar::from("XNAS"));
    // Under a bound the one row fits, nothing is forgotten.
    let mut small = lit(4);
    let _ = small.into_arrow_array();
    small
        .spill(&SpillOptions::new().with_byte_size(1 << 20))
        .unwrap();
    assert!(small.as_lit().unwrap().is_built());
}

#[test]
fn a_write_of_the_value_moves_the_count_and_another_value_lays_the_column_out() {
    let mut column = lit(3);
    column.push(Scalar::from("XNAS")).unwrap();
    assert_eq!(column.len(), 4);
    assert!(column.as_lit().is_some(), "the same value is a count");
    column.remove(0).unwrap();
    assert_eq!(column.len(), 3);
    assert!(column.as_lit().is_some(), "a removal is a count");
    assert!(
        column
            .set(0, Scalar::from_sequence([Scalar::from(1_i64)]))
            .is_err(),
        "a value the field refuses"
    );
    assert!(column.as_lit().is_some(), "a refused write changes nothing");
    column.set(1, Scalar::from("XLON")).unwrap();
    assert!(
        column.as_lit().is_none(),
        "another value lays the column out"
    );
    assert!(column.as_utf8().is_some(), "as the field's own leaf");
    assert_eq!(
        column.rows().to_vec(),
        [
            Scalar::from("XNAS"),
            Scalar::from("XLON"),
            Scalar::from("XNAS")
        ]
    );
}

#[test]
fn extending_with_the_same_constant_stays_a_lit() {
    let mut column = lit(2);
    column.extend_from_serie(&lit(3)).unwrap();
    assert_eq!(column.len(), 5);
    assert!(column.as_lit().is_some());
    column
        .extend_from_serie(&Serie::lit(venue(), Scalar::from("XLON"), 1).unwrap())
        .unwrap();
    assert_eq!(column.len(), 6);
    assert!(column.as_lit().is_none());
    assert_eq!(column.scalar(5).unwrap(), Scalar::from("XLON"));
}

#[test]
fn from_default_answers_a_lit() {
    let column = Serie::from_default(DataType::Int64.required_field("n"), 7).unwrap();
    assert!(column.as_lit().is_some());
    assert_eq!(column.len(), 7);
    assert_eq!(column.scalar(6).unwrap(), Scalar::from(0_i64));
}

#[test]
fn a_cast_keeps_a_constant_constant() {
    let target = DataType::large_utf8().required_field("venue");
    let plan = ArrowCastPlan::compile(&venue(), &target, ArrowCastOptions::new()).unwrap();
    let cast = plan.apply(&lit(4)).unwrap();
    assert!(cast.as_lit().is_some(), "cast once, repeated");
    assert_eq!(cast.field(), Some(&target));
    assert_eq!(cast.len(), 4);
    assert_eq!(cast.scalar(3).unwrap().as_str(), Some("XNAS"));
    assert_eq!(lit(4).cast(&target, ArrowCastOptions::new()).unwrap(), cast);
    // A value the target refuses is refused once, before any row.
    let refused = ArrowCastPlan::compile(
        &venue(),
        &DataType::Int64.required_field("venue"),
        ArrowCastOptions::new(),
    )
    .unwrap()
    .apply(&lit(4));
    assert!(refused.is_err());
}

#[test]
fn the_ordering_verbs_read_a_lit_without_a_pass() {
    let column = lit(5);
    assert!(column.is_sorted(SortOptions::default()));
    let order = column.sort_indices(SortOptions::default()).unwrap();
    assert_eq!(order.len(), 5);
    assert!(
        order.is_sorted(SortOptions::default()),
        "the order rows stand in"
    );
    assert!(!column.is_unique());
    assert!(lit(1).is_unique());
    assert_eq!(column.unique_count(), 1);
    assert_eq!(lit(0).unique_count(), 0);
    assert_eq!(column.into_unique().unwrap().len(), 1);
    assert_eq!(column.into_reversed(), column);
    let taken = column
        .into_taken(&Serie::new(vec![Scalar::from(4_u32), Scalar::from(0_u32)]))
        .unwrap();
    assert!(taken.as_lit().is_some());
    assert_eq!(taken.len(), 2);
    let mask = Serie::new(
        [true, false, true, false, true]
            .into_iter()
            .map(Scalar::from)
            .collect::<Vec<_>>(),
    );
    let filtered = column.into_filtered(&mask).unwrap();
    assert!(filtered.as_lit().is_some());
    assert_eq!(filtered.len(), 3);
    assert_eq!(column.into_sorted(SortOptions::default()).unwrap().len(), 5);
}

#[test]
fn a_lit_digests_and_serializes_as_the_rows_it_is() {
    let constant = lit(4);
    let laid = laid_out(4);
    assert_eq!(
        Scalar::from(constant.clone()).stable_hash(),
        Scalar::from(laid.clone()).stable_hash(),
        "one value whichever layout holds the rows"
    );
    let written = serde_json::to_string(&constant).unwrap();
    assert_eq!(written, serde_json::to_string(&laid).unwrap());
    let read: Serie = serde_json::from_str(&written).unwrap();
    assert_eq!(read, constant);
}

#[test]
fn a_record_child_may_be_a_lit() {
    let root = StructType::from_fields([DataType::Int64.required_field("id"), venue()])
        .map(DataType::from)
        .unwrap()
        .required_field("trade");
    let row = |id: i64| Scalar::from_sequence([Scalar::from(id), Scalar::from("XLON")]);
    let mut record = Serie::from_scalars(root, [row(1), row(2), row(3)]).unwrap();
    record.set_child(lit(3)).unwrap();
    assert!(record.child("venue").unwrap().as_lit().is_some());
    assert_eq!(
        record.scalar(2).unwrap(),
        Scalar::from_sequence([Scalar::from(3_i64), Scalar::from("XNAS")])
    );
    let batch = record.into_arrow_batch().unwrap();
    assert_eq!(batch.num_rows(), 3);
    assert_eq!(batch.num_columns(), 2);
    // A cell write of another value lays the child out.
    record
        .set_cell(&"venue".parse().unwrap(), 0, Scalar::from("XPAR"))
        .unwrap();
    assert!(record.child("venue").unwrap().as_lit().is_none());
    assert_eq!(
        record.child("venue").unwrap().scalar(0).unwrap(),
        Scalar::from("XPAR")
    );
}
