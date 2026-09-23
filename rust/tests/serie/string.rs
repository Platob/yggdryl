//! `rust/src/serie/string.rs`: the byte layouts read as text - what a text
//! leaf lends where it lies, what the field's contract refuses at a write,
//! and how a write moves the offsets.

use std::sync::Arc;

use arrow_array::{Array, ArrayRef, StringArray};
use yggdryl::{ArrowCastOptions, DataType, Field, Scalar, Serie, SerieValue, Utf8StringSerie};

/// A required utf8 column of three symbols, straight off Arrow buffers.
fn symbols() -> Serie {
    let array: ArrayRef = Arc::new(StringArray::from(vec!["AAPL", "MSFT", "NVDA"]));
    Serie::from_arrow_array(
        Some(&Field::new("symbol", DataType::utf8(), false)),
        array,
        ArrowCastOptions::new(),
    )
    .expect("a utf8 column")
}

/// A nullable utf8 column of three rows, one of them absent.
fn names() -> Serie {
    let array: ArrayRef = Arc::new(StringArray::from(vec![Some("ab"), None, Some("cde")]));
    Serie::from_arrow_array(
        Some(&Field::new("name", DataType::utf8(), true)),
        array,
        ArrowCastOptions::new(),
    )
    .expect("a utf8 column")
}

/// The text every present row of `column` holds, `None` for an absent one.
fn texts(column: &Serie) -> Vec<Option<String>> {
    column
        .rows()
        .iter()
        .map(|row| row.as_str().map(str::to_owned))
        .collect()
}

#[test]
fn a_required_column_refuses_an_absent_row_and_leaves_the_column_unchanged() {
    let mut column = symbols();

    let refusal = column
        .push(Scalar::Null)
        .expect_err("a required column admits no absent row");
    assert!(
        refusal.to_string().contains("symbol"),
        "the refusal names the column: {refusal}"
    );
    assert!(column.set(1, Scalar::Null).is_err());
    // A run of values is not text; a number coerces into some.
    assert!(
        column
            .set(1, Scalar::from_sequence([Scalar::from(1_i64)]))
            .is_err()
    );
    assert!(
        column
            .splice(0..1, vec![Scalar::from("A"), Scalar::Null])
            .is_err()
    );
    assert_eq!(column.len(), 3);
    let leaf = column.as_utf8().expect("a utf8 column");
    assert_eq!(leaf.offsets().as_ref(), &[0, 4, 8, 12]);
    assert_eq!(leaf.payload().as_slice(), b"AAPLMSFTNVDA");
    assert!(leaf.nulls().is_none());
}

#[test]
fn a_row_past_the_end_is_refused_naming_the_column_and_both_counts() {
    let column = symbols();
    let leaf = column.as_utf8().expect("a utf8 column");

    let refusal = leaf.scalar(3).expect_err("row 3 is past the end");
    let text = refusal.to_string();
    assert!(text.contains("symbol"), "names the column: {text}");
    assert!(text.contains('3'), "names the row: {text}");
    assert!(leaf.is_null(3).is_err());
    assert_eq!(leaf.value(3), None);
    assert!(leaf.slice(2, 2).is_err());

    let mut column = symbols();
    assert!(column.set(3, Scalar::from("X")).is_err());
    assert!(column.insert(4, Scalar::from("X")).is_err());
    let backwards = (2, 1);
    assert!(column.splice(backwards.0..backwards.1, vec![]).is_err());
    assert!(column.splice(1..4, vec![]).is_err());
    assert_eq!(column.len(), 3);
}

#[test]
fn a_code_column_refuses_an_unregistered_value_at_push_and_keeps_what_it_had() {
    let mut column = Serie::from_scalars(
        Field::new("ccy", DataType::Currency, false),
        [Scalar::from("USD"), Scalar::from("EUR")],
    )
    .expect("two currencies");
    assert!(column.as_utf8().is_some());
    assert_eq!(
        column.field().map(|field| field.dtype()),
        Some(&DataType::Currency)
    );

    let refusal = column
        .push(Scalar::from("NOPE"))
        .expect_err("not a registered currency");
    assert!(
        refusal.to_string().contains("ccy"),
        "the refusal names the column: {refusal}"
    );
    assert!(column.set(0, Scalar::from("NOPE")).is_err());
    assert_eq!(column.len(), 2);

    // A registered one lands as the three characters the code rides on.
    column
        .push(Scalar::from("JPY"))
        .expect("a registered currency");
    let leaf = column.as_utf8().expect("a code rides utf8");
    assert_eq!(leaf.payload().as_slice(), b"USDEURJPY");
    assert_eq!(leaf.offsets().as_ref(), &[0, 3, 6, 9]);
    assert_eq!(column.scalar(2).unwrap().as_str(), Some("JPY"));
    assert!(column.scalar(2).unwrap().is_code());
}

#[test]
fn a_text_column_lends_its_offsets_and_its_characters_where_they_lie() {
    let column = symbols();

    let leaf: &Utf8StringSerie = column.as_utf8().expect("a utf8 column");
    assert_eq!(leaf.offsets().as_ref(), &[0, 4, 8, 12]);
    assert_eq!(leaf.payload().as_slice(), b"AAPLMSFTNVDA");
    assert_eq!(leaf.value(1), Some("MSFT"));
    assert_eq!(leaf.value(3), None);
    assert!(leaf.nulls().is_none());
    assert_eq!(leaf.array().len(), 3);

    // The value side still answers, and it answers the same characters.
    assert_eq!(column.scalar(2).unwrap(), Scalar::from("NVDA"));
    assert_eq!(
        leaf.slice(2, 1).unwrap().scalar(0).unwrap(),
        Scalar::from("NVDA")
    );
    assert!(column.as_binary().is_none());
    assert!(matches!(column, Serie::Utf8String(_)));
    assert_eq!(leaf.id(), yggdryl::DataTypeId::Utf8String);
    assert!(column.as_large_utf8().is_none());
}

#[test]
fn a_set_in_the_middle_moves_every_later_offset() {
    let mut column = symbols();

    column.set(1, Scalar::from("GOOGL")).expect("a longer run");
    let leaf = column.as_utf8().expect("a utf8 column");
    assert_eq!(leaf.offsets().as_ref(), &[0, 4, 9, 13]);
    assert_eq!(leaf.payload().as_slice(), b"AAPLGOOGLNVDA");
    assert_eq!(leaf.value(2), Some("NVDA"));

    column.set(0, Scalar::from("A")).expect("a shorter run");
    let leaf = column.as_utf8().expect("a utf8 column");
    assert_eq!(leaf.offsets().as_ref(), &[0, 1, 6, 10]);
    assert_eq!(leaf.payload().as_slice(), b"AGOOGLNVDA");

    // A nullable column empties a slot and fills it again.
    let mut column = names();
    column.set(0, Scalar::Null).expect("an emptied slot");
    column.set(1, Scalar::from("xy")).expect("a filled slot");
    let leaf = column.as_utf8().expect("a utf8 column");
    assert_eq!(leaf.offsets().as_ref(), &[0, 0, 2, 5]);
    assert_eq!(leaf.payload().as_slice(), b"xycde");
    assert_eq!(column.null_count(), 1);
    assert_eq!(
        texts(&column),
        vec![None, Some("xy".to_owned()), Some("cde".to_owned())]
    );
}

#[test]
fn every_write_round_trips_through_the_rows() {
    let mut column = names();

    column
        .splice(
            0..2,
            vec![Scalar::from("1"), Scalar::Null, Scalar::from("22")],
        )
        .expect("two rows replaced by three");
    column
        .insert(1, Scalar::from("333"))
        .expect("a row inserted");
    assert_eq!(
        texts(&column),
        vec![
            Some("1".to_owned()),
            Some("333".to_owned()),
            None,
            Some("22".to_owned()),
            Some("cde".to_owned()),
        ]
    );
    assert_eq!(
        column.as_utf8().unwrap().offsets().as_ref(),
        &[0, 1, 4, 4, 6, 9]
    );

    assert_eq!(column.remove(1).unwrap(), Scalar::from("333"));
    assert_eq!(column.pop().unwrap(), Some(Scalar::from("cde")));
    column.truncate(2).expect("rows 2.. dropped");
    assert_eq!(texts(&column), vec![Some("1".to_owned()), None]);

    column
        .resize(4, Scalar::from("zz"))
        .expect("two clones appended");
    assert_eq!(column.as_utf8().unwrap().payload().as_slice(), b"1zzzz");
    column
        .extend(vec![Scalar::Null, Scalar::from("end")])
        .expect("two rows");
    assert_eq!(column.len(), 6);
    assert_eq!(column.null_count(), 2);
    assert_eq!(column.scalar(5).unwrap(), Scalar::from("end"));
    column.clear().expect("every row dropped");
    assert!(column.is_empty());
    assert_eq!(column.field().map(Field::name), Some("name"));
    assert_eq!(column.pop().unwrap(), None);
}

#[test]
fn a_slice_with_its_original_dropped_grows_into_a_freshly_built_array() {
    let whole = names();
    let mut window = whole.slice(1, 2).expect("rows 1..3");
    drop(whole);

    window.push(Scalar::from("f")).expect("a present row");
    window.push(Scalar::Null).expect("an absent row");
    window.set(0, Scalar::from("gh")).expect("a filled slot");

    let expected: ArrayRef = Arc::new(StringArray::from(vec![
        Some("gh"),
        Some("cde"),
        Some("f"),
        None,
    ]));
    let held = window.into_arrow_array().expect("a column");
    assert_eq!(held.as_ref(), expected.as_ref());
    assert_eq!(
        window.as_utf8().unwrap().offsets().as_ref(),
        &[0, 2, 5, 6, 6]
    );
}

#[test]
fn a_column_built_by_pushes_is_the_from_scalars_column_buffer_for_buffer() {
    let field = Field::new("name", DataType::utf8(), true);
    let rows = vec![Scalar::from("ab"), Scalar::Null, Scalar::from("cde")];

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
    assert_eq!(
        pushed,
        Serie::new(vec![Scalar::from("ab"), Scalar::Null, Scalar::from("cde")])
    );
}

#[test]
fn two_columns_of_one_field_append_buffer_to_buffer() {
    let mut column = names();
    let more = symbols();

    column.extend_from_serie(&more).expect("one layout");
    let leaf = column.as_utf8().expect("a utf8 column");
    assert_eq!(leaf.offsets().as_ref(), &[0, 2, 2, 5, 9, 13, 17]);
    assert_eq!(leaf.payload().as_slice(), b"abcdeAAPLMSFTNVDA");
    assert_eq!(column.null_count(), 1);

    // A required column takes a nullable one through the rows instead, and
    // refuses the absent row by name.
    let mut required = symbols();
    let refusal = required
        .extend_from_serie(&column)
        .expect_err("an absent row under a required field");
    assert!(refusal.to_string().contains("symbol"), "{refusal}");
    assert_eq!(required.len(), 3);
}

#[test]
fn a_charset_arrow_cannot_state_is_a_binary_leaf_read_as_text() {
    let mut column = Serie::from_scalars(
        Field::new("city", DataType::cp1252(), true),
        [Scalar::from("Lyon"), Scalar::Null],
    )
    .expect("two rows");
    assert!(column.as_binary().is_none());

    column
        .push(Scalar::from("Orléans"))
        .expect("one byte per character");
    let leaf = column.as_binary_string().expect("a windows-1252 column");
    // The stored length is the charset's answer: `é` is one byte here.
    assert_eq!(leaf.offsets().as_ref(), &[0, 4, 4, 11]);
    assert_eq!(&leaf.payload().as_slice()[4..], b"Orl\xe9ans");
    assert_eq!(column.scalar(2).unwrap().as_str(), Some("Orléans"));
    assert_eq!(
        texts(&column),
        vec![Some("Lyon".to_owned()), None, Some("Orléans".to_owned())]
    );
}

#[test]
fn the_large_and_view_layouts_write_through_the_same_verbs() {
    let mut large = Serie::from_scalars(
        Field::new("name", DataType::large_utf8(), true),
        [Scalar::from("ab"), Scalar::Null],
    )
    .expect("two rows");
    large.push(Scalar::from("cde")).expect("a row");
    large.set(1, Scalar::from("x")).expect("a filled slot");
    let leaf = large.as_large_utf8().expect("a large utf8 column");
    assert_eq!(leaf.offsets().as_ref(), &[0_i64, 2, 3, 6]);
    assert_eq!(leaf.payload().as_slice(), b"abxcde");
    assert!(leaf.nulls().is_none());

    let mut views = Serie::from_scalars(
        Field::new("name", DataType::utf8_view(), true),
        [Scalar::from("ab"), Scalar::Null],
    )
    .expect("two rows");
    views
        .push(Scalar::from("a run longer than twelve bytes"))
        .expect("a run held out of line");
    views.set(1, Scalar::from("x")).expect("a filled slot");
    views.remove(0).expect("the first row");
    views
        .insert(0, Scalar::Null)
        .expect("an absent row in front");
    let leaf = views.as_utf8_view().expect("a utf8 view column");
    assert_eq!(leaf.views().len(), 3);
    assert_eq!(leaf.value(0), None);
    assert_eq!(leaf.value(1), Some("x"));
    assert_eq!(leaf.value(2), Some("a run longer than twelve bytes"));
    assert_eq!(leaf.nulls().map(|nulls| nulls.null_count()), Some(1));
    assert_eq!(leaf.array().len(), 3);
    assert_eq!(
        texts(&views),
        vec![
            None,
            Some("x".to_owned()),
            Some("a run longer than twelve bytes".to_owned())
        ]
    );
    assert!(leaf.slice(1, 2).unwrap().payloads().len() <= leaf.payloads().len());
}

#[test]
fn a_fixed_width_text_column_pads_its_slot_and_reads_the_text_back() {
    let mut column = Serie::from_scalars(
        Field::new("tag", DataType::fixed_utf8(4).unwrap(), true),
        [Scalar::from("AB"), Scalar::Null],
    )
    .expect("two rows");

    column.push(Scalar::from("CDEF")).expect("a full slot");
    column.set(0, Scalar::from("X")).expect("one memcpy");
    let leaf = column.as_fixed_string().expect("a fixed-width text column");
    assert_eq!(leaf.width(), 4);
    assert_eq!(leaf.payload().as_slice(), b"X\0\0\0\0\0\0\0CDEF");
    assert_eq!(leaf.value(0), Some(b"X\0\0\0".as_slice()));
    assert_eq!(leaf.value(1), None);
    assert_eq!(column.scalar(0).unwrap().as_str(), Some("X"));
    assert_eq!(column.scalar(2).unwrap().as_str(), Some("CDEF"));
    assert_eq!(column.null_count(), 1);
    assert!(column.as_fixed_bytes().is_none());

    let refusal = column
        .push(Scalar::from("too long"))
        .expect_err("eight bytes do not fit four");
    assert!(refusal.to_string().contains("tag"), "{refusal}");
    assert_eq!(column.len(), 3);
    assert_eq!(column.remove(1).unwrap(), Scalar::Null);
    assert_eq!(
        column.as_fixed_string().unwrap().payload().as_slice(),
        b"X\0\0\0CDEF"
    );
}
