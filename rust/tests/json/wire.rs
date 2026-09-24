//! `rust/src/json/wire.rs`: the natural JSON projection of one value.
//!
//! A sequence is written as the rows it holds, so these pin that a column
//! writes the same text as the run of its rows, nested or not, through
//! `yggdryl::json`.

use yggdryl::{DataType, Field, Scalar, Serie};

/// A `serie<int64>` value held as a column, and the run of its rows.
fn int64s() -> (Scalar, Scalar) {
    let item = Field::new("item", DataType::Int64, false);
    let rows = [Scalar::from(1_i64), Scalar::from(2_i64)];
    let column = Serie::from_scalars(item, rows.clone()).unwrap();
    (Scalar::from(column), Scalar::from_sequence(rows))
}

/// A `serie<serie<int64>>` value held as a column, and the run of its rows.
fn nested() -> (Scalar, Scalar) {
    let (column, run) = int64s();
    let item = Field::new(
        "item",
        DataType::serie(Field::new("item", DataType::Int64, false)),
        false,
    );
    let outer = Serie::from_scalars(item, [column.clone(), column]).unwrap();
    (
        Scalar::from(outer),
        Scalar::from_sequence([run.clone(), run]),
    )
}

#[test]
fn a_serie_column_writes_as_the_run_of_its_rows() {
    for (column, run) in [int64s(), nested()] {
        assert_eq!(
            yggdryl::json::into_utf8(&column).unwrap(),
            yggdryl::json::into_utf8(&run).unwrap()
        );
        assert_eq!(
            yggdryl::into_json_scalar(&column).unwrap(),
            yggdryl::into_json_scalar(&run).unwrap()
        );
    }
    assert_eq!(
        yggdryl::json::into_utf8(&nested().0).unwrap(),
        "[[1,2],[1,2]]"
    );
}
