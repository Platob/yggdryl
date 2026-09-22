//! `rust/src/serie/union.rs`: the union leaf - type ids, dense offsets and
//! one child column per member, read through the type id and rebuilt on a
//! write.

use std::sync::Arc;

use arrow_array::{Array, ArrayRef, Int64Array, StringArray, UnionArray};
use arrow_buffer::ScalarBuffer;
use arrow_schema::{DataType as ArrowDataType, UnionFields};
use yggdryl::{DataType, Field, Scalar, Serie, SerieValue, UnionMode, UnionSerie};

/// A union of a required identifier and a nullable symbol.
fn quote_field(mode: UnionMode, nullable: bool) -> Field {
    Field::new(
        "quote",
        DataType::union(
            [
                (0, Field::new("id", DataType::Int64, false)),
                (1, Field::new("symbol", DataType::utf8(), true)),
            ],
            mode,
        )
        .expect("two members"),
        nullable,
    )
}

/// The Arrow members [`quote_field`] projects to.
fn arrow_members(mode: UnionMode) -> UnionFields {
    let ArrowDataType::Union(fields, _) = quote_field(mode, true)
        .as_arrow_field_ref()
        .expect("a projection")
        .data_type()
        .clone()
    else {
        panic!("a union field projects to a union")
    };
    fields
}

/// One union row: the member's type id and its payload.
fn quote(type_id: i64, payload: Scalar) -> Scalar {
    Scalar::from_sequence([Scalar::from(type_id), payload])
}

/// The four rows both layouts hold: `[0, 1], [1, "AAPL"], [1, null], [0, 2]`.
fn quote_rows() -> Vec<Scalar> {
    vec![
        quote(0, Scalar::from(1_i64)),
        quote(1, Scalar::from("AAPL")),
        quote(1, Scalar::Null),
        quote(0, Scalar::from(2_i64)),
    ]
}

/// [`quote_rows`] laid out densely: each child holds only its rows.
fn dense_quotes() -> ArrayRef {
    Arc::new(
        UnionArray::try_new(
            arrow_members(UnionMode::Dense),
            ScalarBuffer::from(vec![0_i8, 1, 1, 0]),
            Some(ScalarBuffer::from(vec![0_i32, 0, 1, 1])),
            vec![
                Arc::new(Int64Array::from(vec![1, 2])),
                Arc::new(StringArray::from(vec![Some("AAPL"), None])),
            ],
        )
        .expect("aligned children"),
    )
}

/// [`quote_rows`] laid out sparsely: every child holds every row, with an
/// absent placeholder where another member is active - the required
/// identifier included.
fn sparse_quotes() -> ArrayRef {
    Arc::new(
        UnionArray::try_new(
            arrow_members(UnionMode::Sparse),
            ScalarBuffer::from(vec![0_i8, 1, 1, 0]),
            None,
            vec![
                Arc::new(Int64Array::from(vec![Some(1), None, None, Some(2)])),
                Arc::new(StringArray::from(vec![None, Some("AAPL"), None, None])),
            ],
        )
        .expect("children of the union's length"),
    )
}

/// Both layouts of [`quote_rows`] as nullable columns.
fn quotes() -> [Serie; 2] {
    [
        Serie::from_arrow_array(quote_field(UnionMode::Dense, true), dense_quotes())
            .expect("a dense union column"),
        Serie::from_arrow_array(quote_field(UnionMode::Sparse, true), sparse_quotes())
            .expect("a sparse union column"),
    ]
}

#[test]
fn a_required_union_column_with_an_absent_payload_is_refused_naming_the_column() {
    for (mode, array) in [
        (UnionMode::Dense, dense_quotes()),
        (UnionMode::Sparse, sparse_quotes()),
    ] {
        let refusal = Serie::from_arrow_array(quote_field(mode, false), array)
            .expect_err("a required column admits no absent payload");
        assert!(
            refusal.to_string().contains("quote"),
            "the refusal names the column: {refusal}"
        );
    }

    // A required member's inactive slots hold what Arrow leaves unspecified
    // and are never judged.
    let sparse = Serie::from_arrow_array(quote_field(UnionMode::Sparse, true), sparse_quotes())
        .expect("a required member judged where it is active");
    assert_eq!(sparse.null_count(), 1);
    assert_eq!(sparse.scalar(0).unwrap(), quote(0, Scalar::from(1_i64)));
}

#[test]
fn a_row_past_the_end_and_a_row_the_field_refuses_are_refused_and_nothing_moves() {
    for mut column in quotes() {
        let refusal = column.scalar(4).expect_err("row 4 is past the end");
        let text = refusal.to_string();
        assert!(text.contains("quote"), "names the column: {text}");
        assert!(text.contains('4'), "names the row: {text}");
        assert!(column.is_null(4).is_err());
        assert!(column.slice(3, 2).is_err());

        // A union spells absence inside a member: a bare null is no row.
        assert!(column.push(Scalar::Null).is_err());
        assert!(column.push(quote(7, Scalar::from(1_i64))).is_err());
        assert!(column.push(quote(0, Scalar::from("AAPL"))).is_err());
        assert!(column.push(quote(0, Scalar::Null)).is_err());
        assert!(column.push(Scalar::from(1_i64)).is_err());
        assert_eq!(column.len(), 4);
        assert_eq!(column.rows().into_owned(), quote_rows());
    }
}

#[test]
fn the_buffers_cross_in_and_out_shared_and_a_row_reads_through_its_type_id() {
    let array = dense_quotes();
    let dense = Serie::from_arrow_array(quote_field(UnionMode::Dense, true), Arc::clone(&array))
        .expect("a dense union column");
    let [_, sparse] = quotes();
    let leaf = dense.as_union().expect("a union column");

    assert_eq!(<UnionSerie as SerieValue>::from_serie(&dense), Some(leaf));
    assert_eq!(leaf.type_ids(), &[0, 1, 1, 0]);
    assert_eq!(leaf.offsets(), Some(&[0, 0, 1, 1][..]));
    assert_eq!(leaf.mode(), UnionMode::Dense);
    assert_eq!(leaf.children().len(), 2);
    assert_eq!(dense.children().len(), 2);
    assert_eq!(leaf.child_of(0).map(Serie::len), Some(2));
    assert_eq!(
        leaf.child_of(1).and_then(Serie::field).map(Field::name),
        Some("symbol")
    );
    assert!(leaf.child_of(7).is_none());
    assert_eq!(leaf.null_count(), 1);
    assert!(leaf.is_null(2).unwrap());
    assert!(!leaf.is_null(1).unwrap());
    assert_eq!(dense.rows().into_owned(), quote_rows());
    assert_eq!(dense, Serie::new(quote_rows()));
    assert_eq!(dense, sparse);

    let back = dense.into_arrow_array().expect("a column");
    assert_eq!(back.data_type(), array.data_type());
    assert!(
        back.to_data().ptr_eq(&array.to_data()),
        "the type ids, the offsets and the children cross back out shared"
    );

    let leaf = sparse.as_union().expect("a union column");
    assert_eq!(leaf.offsets(), None);
    assert_eq!(leaf.mode(), UnionMode::Sparse);
    assert!(leaf.children().iter().all(|child| child.len() == 4));
    assert_eq!(leaf.scalar(3).unwrap(), quote(0, Scalar::from(2_i64)));
    let array = sparse_quotes();
    let sparse = Serie::from_arrow_array(quote_field(UnionMode::Sparse, true), Arc::clone(&array))
        .expect("a sparse union column");
    let back = sparse.into_arrow_array().expect("a column");
    assert!(back.to_data().ptr_eq(&array.to_data()));
}

#[test]
fn a_window_slices_the_type_ids_and_what_each_layout_reaches() {
    let [dense, sparse] = quotes();

    let window = dense.slice(1, 2).expect("rows 1..3");
    let leaf = window.as_union().expect("a union column");
    assert_eq!(leaf.type_ids(), &[1, 1]);
    assert_eq!(leaf.offsets(), Some(&[0, 1][..]));
    assert_eq!(
        window.rows().into_owned(),
        vec![quote(1, Scalar::from("AAPL")), quote(1, Scalar::Null)]
    );
    assert_eq!(window.null_count(), 1);

    let window = sparse.slice(1, 2).expect("rows 1..3");
    let leaf = window.as_union().expect("a union column");
    assert_eq!(leaf.type_ids(), &[1, 1]);
    assert!(leaf.children().iter().all(|child| child.len() == 2));
    assert_eq!(
        window.rows().into_owned(),
        vec![quote(1, Scalar::from("AAPL")), quote(1, Scalar::Null)]
    );
    assert!(dense.slice(4, 0).unwrap().is_empty());
}

#[test]
fn the_writes_rebuild_the_layout_and_the_rows_round_trip() {
    for (mode, mut column) in [UnionMode::Dense, UnionMode::Sparse]
        .into_iter()
        .zip(quotes())
    {
        column.push(quote(0, Scalar::from(3_i64))).expect("a row");
        column
            .set(1, quote(1, Scalar::from("MSFT")))
            .expect("one slot");
        column
            .insert(0, quote(1, Scalar::Null))
            .expect("an absent payload in front");
        assert_eq!(column.remove(2).unwrap(), quote(1, Scalar::from("MSFT")));
        column.truncate(3).expect("three rows kept");

        let rows = vec![
            quote(1, Scalar::Null),
            quote(0, Scalar::from(1_i64)),
            quote(1, Scalar::Null),
        ];
        assert_eq!(column.rows().into_owned(), rows);
        assert_eq!(column.null_count(), 2);
        assert_eq!(column.as_union().expect("a union column").mode(), mode);
        assert_eq!(
            column,
            Serie::from_scalars(quote_field(mode, true), rows).expect("three rows")
        );
        assert_eq!(column.pop().unwrap(), Some(quote(1, Scalar::Null)));

        // Appending another column joins the layouts, no row read.
        let other = Serie::from_scalars(
            quote_field(mode, true),
            [quote(0, Scalar::from(9_i64)), quote(1, Scalar::from("IBM"))],
        )
        .expect("two rows");
        column.extend_from_serie(&other).expect("agreeing fields");
        assert_eq!(column.len(), 4);
        assert_eq!(column.scalar(3).unwrap(), quote(1, Scalar::from("IBM")));
        assert_eq!(column.scalar(2).unwrap(), quote(0, Scalar::from(9_i64)));

        // A column built by pushes reads as the column laid out at once.
        let laid_out = Serie::from_scalars(quote_field(mode, true), quote_rows()).expect("rows");
        let mut pushed = Serie::empty(quote_field(mode, true)).expect("an empty column");
        for row in quote_rows() {
            pushed.push(row).expect("a row the field accepts");
        }
        assert_eq!(pushed, laid_out);
        assert_eq!(pushed.null_count(), 1);
        assert_eq!(pushed.into_arrow_array().unwrap().len(), 4);
        pushed.clear().expect("cleared");
        assert!(pushed.is_empty());
        assert!(pushed.children().iter().all(Serie::is_empty));
    }
}
