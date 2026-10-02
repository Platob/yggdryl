//! `rust/src/boolean.rs`.

use yggdryl::DataType;
use yggdryl::boolean;

use super::typed::assert_typed_marker;

#[test]
fn scalar_markers_cover_null_and_boolean() {
    assert_typed_marker::<boolean::NullType>(DataType::Null);
    assert_typed_marker::<boolean::BooleanType>(DataType::Boolean);
}

#[test]
fn a_boolean_cell_reads_every_spelling_its_column_cast_reads() {
    use yggdryl::{ArrowCastOptions, Field, Scalar, Serie};

    // A declared boolean reads FIX's `Y`/`N` and a bridge's `no` as a
    // column cast does; text no boolean spells stays a refusal.
    let spellings = [
        "true",
        "TRUE",
        " t ",
        "tr",
        "tru",
        "yes",
        "Y",
        "ye",
        "on",
        "1",
        "false",
        "F",
        "fa",
        "fal",
        "fals",
        "no",
        "N",
        "off",
        "of",
        "0",
        "n/a",
        "aggressor",
        "2",
        "",
    ];
    let text = Field::new("flag", DataType::utf8(), true);
    let column = Serie::from_scalars(text, spellings.map(Scalar::from)).expect("a text column");
    let cast = column
        .cast(
            &Field::new("flag", DataType::Boolean, true),
            ArrowCastOptions::new().with_safe(true),
        )
        .expect("a boolean column");
    for (row, spelling) in spellings.iter().enumerate() {
        let cell = DataType::Boolean
            .scalar(Scalar::from(*spelling))
            .unwrap_or(Scalar::Null);
        assert_eq!(cell, cast.scalar(row).expect("a row"), "{spelling:?}");
    }
    assert_eq!(
        DataType::Boolean
            .scalar(Scalar::from("no"))
            .expect("a reading"),
        Scalar::from(false)
    );
    assert!(DataType::Boolean.scalar(Scalar::from("n/a")).is_err());
}
