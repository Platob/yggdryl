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

/// A map key is spelled as text, a number or a boolean quoted, and reads
/// back under the key's datatype; an interval has no key JSON reads back.
#[test]
fn a_map_key_is_spelled_as_the_text_its_datatype_reads_back() {
    let counts = Scalar::from_mapping([(Scalar::from(1_i64), Scalar::from("a"))]).unwrap();
    let text = yggdryl::json::into_utf8(&counts).unwrap();
    assert_eq!(text, r#"{"1":"a"}"#);
    let field = Field::new("m", "map<int64, utf8>".parse().unwrap(), true);
    assert_eq!(
        yggdryl::json::from_utf8_with_field(&text, &field).unwrap(),
        counts
    );
    let flags = Scalar::from_mapping([(Scalar::from(true), Scalar::from(1_i64))]).unwrap();
    assert_eq!(yggdryl::json::into_utf8(&flags).unwrap(), r#"{"true":1}"#);
    let spans = Scalar::from_mapping([(
        Scalar::interval(12, 0, 0, yggdryl::TimeUnit::YearMonth).unwrap(),
        Scalar::from("a"),
    )])
    .unwrap();
    assert!(yggdryl::json::into_utf8(&spans).is_err());
}
