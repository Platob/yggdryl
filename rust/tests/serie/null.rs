//! `rust/src/serie/null.rs`: the null leaf - a length, and no buffer at all.

use std::sync::Arc;

use arrow_array::{Array, ArrayRef, NullArray};
use yggdryl::{DataType, Field, Scalar, Serie, SerieValue};

#[test]
fn a_null_column_is_a_length_whose_every_row_is_absent() {
    let array: ArrayRef = Arc::new(NullArray::new(3));
    let column = Serie::from_arrow_array(Field::new("nothing", DataType::Null, true), array)
        .expect("a null column");
    let leaf = column.as_null().expect("a null column");

    assert_eq!(SerieValue::len(leaf), 3);
    assert_eq!(leaf.null_count(), 3);
    assert!(leaf.is_null(2).unwrap());
    assert_eq!(leaf.scalar(0).unwrap(), Scalar::Null);
    assert_eq!(leaf.array().len(), 3);
    assert_eq!(leaf.into_arrow_array().len(), 3);
    assert_eq!(
        leaf.into_arrow_array().data_type(),
        &arrow_schema::DataType::Null
    );
}

#[test]
fn a_row_past_the_end_is_refused_naming_the_column_and_both_counts() {
    let column = Serie::from_scalars(
        Field::new("nothing", DataType::Null, true),
        [Scalar::Null, Scalar::Null],
    )
    .expect("two null rows");
    let leaf = column.as_null().expect("a null column");

    let refusal = leaf.scalar(2).expect_err("row 2 is past the end");
    let text = refusal.to_string();
    assert!(text.contains("nothing"), "names the column: {text}");
    assert!(text.contains('2'), "names the counts: {text}");
    assert!(leaf.is_null(2).is_err());
    assert!(leaf.slice(1, 2).is_err());
    assert_eq!(leaf.slice(1, 1).unwrap().len(), 1);
}

#[test]
fn the_typed_writers_move_the_count_and_refuse_a_range_past_the_end() {
    let mut column = Serie::empty(Field::new("nothing", DataType::Null, true)).unwrap();
    let leaf = column.get_null_mut().expect("a null column");

    leaf.push_value();
    leaf.push_value();
    assert_eq!(SerieValue::len(leaf), 2);

    leaf.splice_values(1..2, 3)
        .expect("one row replaced by three");
    assert_eq!(SerieValue::len(leaf), 4);
    assert!(leaf.splice_values(3..5, 1).is_err());
    let backwards = (2, 1);
    assert!(leaf.splice_values(backwards.0..backwards.1, 1).is_err());
    assert_eq!(SerieValue::len(leaf), 4);

    leaf.truncate(1).expect("a shorter column");
    assert_eq!(SerieValue::len(leaf), 1);
    leaf.push(Scalar::Null).expect("an absent row");
    assert_eq!(leaf.null_count(), 2);
}
