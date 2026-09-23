//! `rust/src/text/typed.rs`: the natural shape a field restates its value in,
//! and the one spelling a document substitutes before the value contract.
//!
//! A sequence reaches both walks as a run or as a column, so these pin that
//! the column reads exactly as the run of its rows, through
//! `Field::into_natural_value` and `Field::from_natural_value`.

use yggdryl::{DataType, Field, Scalar, Serie};

/// The column of `rows` under a required `item` of `dtype`, and their run.
fn column(dtype: DataType, rows: &[Scalar]) -> (Scalar, Scalar) {
    let item = Field::new("item", dtype, false);
    let column = Serie::from_scalars(item, rows.iter().cloned()).unwrap();
    (
        Scalar::from(column),
        Scalar::from_sequence(rows.iter().cloned()),
    )
}

#[test]
fn a_column_restates_as_the_run_of_its_rows() {
    let int64s = [Scalar::from(1_i64), Scalar::from(2_i64)];
    let (values, run) = column(DataType::Int64, &int64s);
    for spelling in [
        "row: struct<a: int64, b: int64>",
        "xs: serie<int64 not null>",
    ] {
        let field = Field::from_str(spelling).unwrap();
        assert_eq!(
            field.into_natural_value(values.clone()).unwrap(),
            field.into_natural_value(run.clone()).unwrap(),
            "{spelling}"
        );
    }
    let named = Field::from_str("row: struct<a: int64, b: int64>")
        .unwrap()
        .into_natural_value(values)
        .unwrap();
    assert_eq!(named.get_key_str("b").and_then(Scalar::as_i64), Some(2));
}

#[test]
fn a_base64_column_reads_as_the_run_of_its_rows() {
    let texts = [Scalar::from("AQI="), Scalar::from("AwQ=")];
    let (values, run) = column(DataType::utf8(), &texts);
    for spelling in [
        "xs: serie<binary not null>",
        "row: struct<a: binary, b: binary>",
    ] {
        let field = Field::from_str(spelling).unwrap();
        assert_eq!(
            field.from_natural_value(values.clone()).unwrap(),
            field.from_natural_value(run.clone()).unwrap(),
            "{spelling}"
        );
    }
}
