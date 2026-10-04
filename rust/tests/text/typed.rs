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

#[test]
fn a_base64_cell_reads_through_the_one_bytes_reader() {
    let field = Field::new("payload", DataType::binary(), false);
    let expected = Scalar::from(vec![0_u8, 255]);
    // The standard alphabet with its pad, whitespace between the digits
    // ignored as a wrapped or indented document writes it.
    for text in ["AP8=", " AP8= ", "AP\n8=", "A P 8 =", "\tAP8=\r\n"] {
        assert_eq!(
            field.from_natural_value(Scalar::from(text)).unwrap(),
            expected,
            "{text:?}"
        );
    }
}

#[test]
fn a_base64_cell_that_is_not_standard_base64_is_refused_by_its_field() {
    let field = Field::new("payload", DataType::binary(), false);
    // The URL-safe alphabet and a missing pad are not RFC 4648 section 4, and
    // a string entering a byte column is never read as its own bytes here.
    for text in ["not base64!", "AP8", "-_8=", "AP8==", "="] {
        let error = field
            .from_natural_value(Scalar::from(text))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("expected base64 text, got"),
            "{text}: {error}"
        );
        assert!(error.contains("$.payload"), "{text}: {error}");
    }
}
