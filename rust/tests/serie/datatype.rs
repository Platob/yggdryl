//! `rust/src/serie/datatype.rs`: the schema-free run a row is, and the serie
//! family's datatype it sits beside.

use std::borrow::Cow;
use std::sync::Arc;

use yggdryl::{DataType, DataTypeId, Field, NestedValue, Run, Scalar, Serie, Value};

/// Two prices as a run.
fn prices() -> Run {
    Run::new(vec![Scalar::from(1_i64), Scalar::from(2_i64)])
}

#[test]
fn a_run_is_one_shared_slice_it_lends_as_it_is() {
    let run = prices();
    assert_eq!(run.as_slice(), &[Scalar::from(1_i64), Scalar::from(2_i64)]);
    assert_eq!(NestedValue::len(&run), 2);
    assert!(!run.is_empty());
    assert_eq!(run.children().count(), 2);
    assert!(matches!(run.children().next(), Some(Cow::Borrowed(_))));
    assert_eq!(run.to_string(), format!("{:?}", run.as_slice()));

    // The slice a run is built over is the slice it holds: nothing between
    // the two, so a run costs one allocation and a run over shared storage
    // costs none.
    let shared: Arc<[Scalar]> = run.clone().into_inner();
    assert_eq!(shared.len(), 2);
    let over = Run::new(Arc::clone(&shared));
    assert_eq!(over, run);
    assert!(Arc::ptr_eq(&over.into_inner(), &shared));
    assert!(std::ptr::eq(
        Run::new(Arc::clone(&shared)).as_slice().as_ptr(),
        shared.as_ptr()
    ));
}

#[test]
fn the_empty_run_holds_nothing_and_every_empty_sequence_is_the_one_shared_slice() {
    assert_eq!(Run::default().as_slice().len(), 0);
    assert!(Run::default().is_empty());
    assert_eq!(Run::default(), Run::new(Vec::<Scalar>::new()));

    // `Scalar::from_sequence` of nothing is one static slice, shared by
    // every empty sequence rather than allocated per row.
    let first = Scalar::from_sequence([]);
    let second = Scalar::from_sequence(std::iter::empty());
    let (Some(first), Some(second)) = (Run::from_scalar(&first), Run::from_scalar(&second)) else {
        panic!("an empty sequence is a run");
    };
    assert!(first.is_empty());
    assert!(Arc::ptr_eq(
        &first.clone().into_inner(),
        &second.clone().into_inner()
    ));
    assert_eq!(first, &Run::default());
}

#[test]
fn a_run_widens_to_a_sequence_scalar_and_narrows_back_only_from_a_run() {
    let run = prices();
    let scalar = run.clone().into_scalar();
    assert_eq!(
        scalar,
        Scalar::from_sequence([Scalar::from(1_i64), Scalar::from(2_i64)])
    );
    assert_eq!(Run::from_scalar(&scalar), Some(&run));
    assert_eq!(scalar.as_sequence(), Some(run.as_slice()));
    assert_eq!(
        Value::dtype(&run).unwrap(),
        DataType::serie(Field::new("item", DataType::Int64, false))
    );
    assert_eq!(Value::dtype(&run).unwrap().id(), DataTypeId::Serie);

    // A column is the same scalar variant and not this leaf.
    let column = Serie::from_scalars(
        Field::new("n", DataType::Int64, false),
        [Scalar::from(1_i64), Scalar::from(2_i64)],
    )
    .unwrap();
    let held = Scalar::Serie(column);
    assert_eq!(Run::from_scalar(&held), None);
    assert_eq!(held.as_sequence(), None);
    assert_eq!(Run::from_scalar(&Scalar::from(1_i64)), None);

    // The serie root holds it as its schema-free leaf, and answers it back.
    let serie = Serie::from(run.clone());
    assert_eq!(serie.as_run(), Some(&run));
    assert_eq!(serie.as_slice(), Some(run.as_slice()));
    assert_eq!(serie.field(), None);
    assert_eq!(serie.into_run(), run);
}

#[test]
fn a_run_serializes_as_a_transparent_list_and_reads_back_as_one() {
    let run = prices();
    let document = serde_json::to_string(&run).unwrap();
    assert_eq!(document, serde_json::to_string(run.as_slice()).unwrap());
    let back: Run = serde_json::from_str(&document).unwrap();
    assert_eq!(back, run);
}

#[test]
fn a_write_through_the_serie_root_copies_the_run_once_and_the_shared_slice_stays() {
    let run = prices();
    let mut serie = Serie::from(run.clone());
    serie.push(Scalar::from(3_i64)).unwrap();
    assert_eq!(serie.len(), 3);
    assert_eq!(run.as_slice().len(), 2, "the run copied, not grown");
    assert_eq!(serie.as_run().map(|held| held.as_slice().len()), Some(3));
}

#[test]
fn a_sequence_datatype_holds_one_item_field_in_five_layouts() {
    let item = Field::new("item", DataType::Int32, true);
    let sequence = DataType::serie(item.clone())
        .as_serie_type()
        .expect("a serie");
    assert_eq!(sequence.item(), &item);
    assert_eq!(sequence.fixed_length(), None);
    assert!(!sequence.is_view());
    assert!(!sequence.is_large());
    assert!(
        DataType::large_serie_view(item.clone())
            .as_serie_type()
            .unwrap()
            .is_view()
    );
    assert_eq!(
        DataType::fixed_size_serie(item, 3)
            .unwrap()
            .as_serie_type()
            .unwrap()
            .fixed_length(),
        Some(3)
    );
}
