//! `rust/src/serie/variant.rs`: the variant leaf - two byte runs per row,
//! lent undecoded, written as the pair the field's contract encodes.

use arrow_array::Array;
use yggdryl::{DataType, Field, Scalar, Serie, SerieValue, Variant};

/// The pair `value` encodes to.
fn encoded(value: Scalar) -> Variant {
    value.into_variant().expect("a value the encoding spells")
}

/// A nullable variant column of an integer, an absent row and a text.
fn payloads() -> Serie {
    Serie::from_scalars(
        Field::new("payload", DataType::Variant, true),
        [
            Scalar::Variant(encoded(Scalar::from(12_i64))),
            Scalar::Null,
            Scalar::Variant(encoded(Scalar::from("AAPL"))),
        ],
    )
    .expect("three variant rows")
}

/// The value every present row of `column` decodes to, `None` for an
/// absent one.
fn decoded(column: &Serie) -> Vec<Option<Scalar>> {
    column
        .rows()
        .iter()
        .map(|row| match row {
            Scalar::Variant(held) => Some(held.scalar().expect("a pair the codec reads")),
            _ => None,
        })
        .collect()
}

#[test]
fn a_required_column_refuses_an_absent_row_and_leaves_the_column_unchanged() {
    let mut column = Serie::from_scalars(
        Field::new("payload", DataType::Variant, false),
        [Scalar::Variant(encoded(Scalar::from(1_i64)))],
    )
    .expect("one variant row");

    let refusal = column
        .push(Scalar::Null)
        .expect_err("a required column admits no absent row");
    assert!(
        refusal.to_string().contains("payload"),
        "the refusal names the column: {refusal}"
    );
    assert!(column.set(0, Scalar::Null).is_err());
    assert!(
        column
            .splice(0..0, vec![Scalar::from(2_i64), Scalar::Null])
            .is_err()
    );
    assert_eq!(column.len(), 1);
    assert_eq!(column.null_count(), 0);
    assert!(column.as_variant().unwrap().nulls().is_none());
    assert_eq!(decoded(&column), vec![Some(Scalar::from(1_i64))]);
}

#[test]
fn a_row_past_the_end_is_refused_naming_the_column_and_both_counts() {
    let column = payloads();
    let leaf = column.as_variant().expect("a variant column");

    let refusal = leaf.scalar(3).expect_err("row 3 is past the end");
    let text = refusal.to_string();
    assert!(text.contains("payload"), "names the column: {text}");
    assert!(text.contains('3'), "names the row: {text}");
    assert!(leaf.is_null(3).is_err());
    assert_eq!(leaf.metadata(3), None);
    assert_eq!(leaf.value(3), None);
    assert!(leaf.slice(2, 2).is_err());

    let mut column = payloads();
    assert!(column.set(3, Scalar::from(1_i64)).is_err());
    assert!(column.insert(4, Scalar::from(1_i64)).is_err());
    let backwards = (2, 1);
    assert!(column.splice(backwards.0..backwards.1, vec![]).is_err());
    assert!(column.splice(1..4, vec![]).is_err());
    assert_eq!(column.len(), 3);
}

#[test]
fn a_variant_column_lends_the_pair_it_stored_and_reads_only_on_demand() {
    let integer = encoded(Scalar::from(12_i64));
    let column = payloads();
    let leaf = column.as_variant().expect("a variant column");

    assert_eq!(leaf.metadata(0), Some(integer.metadata()));
    assert_eq!(leaf.value(0), Some(integer.value()));
    assert_eq!(leaf.metadata(1), None);
    assert_eq!(leaf.value(1), None);
    assert_eq!(leaf.metadata_array().len(), 3);
    assert_eq!(leaf.value_array().len(), 3);
    assert_eq!(leaf.nulls().map(|nulls| nulls.null_count()), Some(1));
    assert_eq!(leaf.null_count(), 1);
    assert!(leaf.is_null(1).unwrap());
    assert_eq!(leaf.scalar(0).unwrap(), Scalar::Variant(integer));
    assert_eq!(leaf.scalar(1).unwrap(), Scalar::Null);
    assert_eq!(leaf.slice(1, 2).unwrap().null_count(), 1);
    assert_eq!(leaf.slice(2, 1).unwrap().value(0), leaf.value(2));
    assert_eq!(leaf.into_arrow_array().len(), 3);
    assert_eq!(SerieValue::field(leaf).name(), "payload");

    // Reading the pair is a second ask, and comes back what went in.
    assert_eq!(
        decoded(&column),
        vec![Some(Scalar::from(12_i64)), None, Some(Scalar::from("AAPL"))]
    );
}

#[test]
fn a_write_encodes_a_native_value_once_and_moves_both_runs() {
    let mut column = payloads();

    // A native value is encoded by the field's contract; a pair goes in as
    // it is.
    column.push(Scalar::from(3.5_f64)).expect("a native row");
    column
        .set(1, Scalar::Variant(encoded(Scalar::from(true))))
        .expect("a filled slot");
    column
        .insert(0, Scalar::from("first"))
        .expect("a row in front");
    assert_eq!(
        decoded(&column),
        vec![
            Some(Scalar::from("first")),
            Some(Scalar::from(12_i64)),
            Some(Scalar::from(true)),
            Some(Scalar::from("AAPL")),
            Some(Scalar::from(3.5_f64)),
        ]
    );
    assert_eq!(column.null_count(), 0);
    assert!(column.as_variant().unwrap().nulls().is_none());

    assert_eq!(
        column.remove(0).unwrap(),
        Scalar::Variant(encoded(Scalar::from("first")))
    );
    column.set(0, Scalar::Null).expect("an emptied slot");
    assert_eq!(
        column.pop().unwrap(),
        Some(Scalar::Variant(encoded(Scalar::from(3.5_f64))))
    );
    column.truncate(2).expect("rows 2.. dropped");
    assert_eq!(decoded(&column), vec![None, Some(Scalar::from(true))]);
    let leaf = column.as_variant().expect("a variant column");
    assert_eq!(leaf.metadata_array().len(), 2);
    assert_eq!(leaf.value_array().len(), 2);
    // An absent row occupies one empty slot in each run.
    assert_eq!(leaf.metadata_array().value(0), b"");
    assert_eq!(leaf.value_array().value(0), b"");

    column
        .resize(4, Scalar::from(7_i64))
        .expect("two clones appended");
    assert_eq!(
        decoded(&column),
        vec![
            None,
            Some(Scalar::from(true)),
            Some(Scalar::from(7_i64)),
            Some(Scalar::from(7_i64)),
        ]
    );
    column.clear().expect("every row dropped");
    assert!(column.is_empty());
    assert_eq!(column.field().map(Field::name), Some("payload"));
}

#[test]
fn a_slice_with_its_original_dropped_grows_into_a_freshly_built_array() {
    let whole = payloads();
    let mut window = whole.slice(1, 2).expect("rows 1..3");
    drop(whole);

    window.push(Scalar::from(4_i64)).expect("a present row");
    window.push(Scalar::Null).expect("an absent row");
    window
        .set(0, Scalar::from("filled"))
        .expect("a filled slot");

    let expected = Serie::from_scalars(
        Field::new("payload", DataType::Variant, true),
        [
            Scalar::from("filled"),
            Scalar::from("AAPL"),
            Scalar::from(4_i64),
            Scalar::Null,
        ],
    )
    .expect("four variant rows");
    assert_eq!(
        window.into_arrow_array().unwrap().as_ref(),
        expected.into_arrow_array().unwrap().as_ref()
    );
    assert_eq!(window, expected);
}

#[test]
fn a_column_built_by_pushes_is_the_from_scalars_column_buffer_for_buffer() {
    let field = Field::new("payload", DataType::Variant, true);
    let rows = vec![
        Scalar::Variant(encoded(Scalar::from(12_i64))),
        Scalar::Null,
        Scalar::from("AAPL"),
    ];

    let laid_out = Serie::from_scalars(field.clone(), rows.clone()).unwrap();
    let mut pushed = Serie::empty(field).unwrap();
    for row in rows {
        pushed.push(row).expect("a row the field accepts");
    }

    assert_eq!(
        pushed.into_arrow_array().unwrap().as_ref(),
        laid_out.into_arrow_array().unwrap().as_ref()
    );
    assert_eq!(pushed, laid_out);
}

#[test]
fn two_columns_of_one_field_append_run_to_run() {
    let mut column = payloads();
    let more = Serie::from_scalars(
        Field::new("more", DataType::Variant, false),
        [Scalar::from(1_i64), Scalar::from("b")],
    )
    .expect("two variant rows");

    column.extend_from_serie(&more).expect("one layout");
    assert_eq!(column.len(), 5);
    assert_eq!(column.null_count(), 1);
    assert_eq!(
        decoded(&column),
        vec![
            Some(Scalar::from(12_i64)),
            None,
            Some(Scalar::from("AAPL")),
            Some(Scalar::from(1_i64)),
            Some(Scalar::from("b")),
        ]
    );

    // A required column takes a nullable one through the rows instead, and
    // refuses the absent row by name.
    let mut required = Serie::from_scalars(
        Field::new("payload", DataType::Variant, false),
        [Scalar::from(0_i64)],
    )
    .unwrap();
    let refusal = required
        .extend_from_serie(&column)
        .expect_err("an absent row under a required field");
    assert!(refusal.to_string().contains("payload"), "{refusal}");
    assert_eq!(required.len(), 1);
}
