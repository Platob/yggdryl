//! `rust/src/serie/union.rs`: the union leaf - type ids, dense offsets and
//! one child column per member, read through the type id; written in place
//! when sparse or when a dense column is appended to, rebuilt otherwise.

use std::sync::Arc;

use arrow_array::{Array, ArrayRef, Int64Array, StringArray, UnionArray};
use arrow_buffer::ScalarBuffer;
use arrow_schema::{DataType as ArrowDataType, UnionFields};
use yggdryl::{
    ArrowCastOptions, DataType, Field, Scalar, Serie, SerieValue, UnionMode, UnionSerie,
};

/// The options a refusal is pinned under: a present value is never nulled.
fn strict() -> ArrowCastOptions {
    ArrowCastOptions::new().with_safe(false)
}

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
        Serie::from_arrow_array(
            Some(&quote_field(UnionMode::Dense, true)),
            dense_quotes(),
            ArrowCastOptions::new(),
        )
        .expect("a dense union column"),
        Serie::from_arrow_array(
            Some(&quote_field(UnionMode::Sparse, true)),
            sparse_quotes(),
            ArrowCastOptions::new(),
        )
        .expect("a sparse union column"),
    ]
}

#[test]
fn a_required_union_column_with_an_absent_payload_is_refused_naming_the_column() {
    for (mode, array) in [
        (UnionMode::Dense, dense_quotes()),
        (UnionMode::Sparse, sparse_quotes()),
    ] {
        let refusal = Serie::from_arrow_array(Some(&quote_field(mode, false)), array, strict())
            .expect_err("a required column admits no absent payload");
        assert!(
            refusal.to_string().contains("quote"),
            "the refusal names the column: {refusal}"
        );
    }

    // A required member's inactive slots hold what Arrow leaves unspecified
    // and are never judged.
    let sparse = Serie::from_arrow_array(
        Some(&quote_field(UnionMode::Sparse, true)),
        sparse_quotes(),
        ArrowCastOptions::new(),
    )
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

        assert!(column.push(quote(7, Scalar::from(1_i64))).is_err());
        assert!(column.push(quote(0, Scalar::from("AAPL"))).is_err());
        assert!(column.push(quote(0, Scalar::Null)).is_err());
        // A bare value no member accepts names the members.
        let record = Scalar::from_struct([("px", Scalar::from(1_i64))]).unwrap();
        let refusal = column.push(record).expect_err("no member holds a record");
        assert!(
            refusal.to_string().contains("one union member accepts"),
            "{refusal}"
        );
        assert_eq!(column.len(), 4);
        assert_eq!(column.rows().into_owned(), quote_rows());
    }
}

#[test]
fn a_bare_value_lands_in_the_member_its_datatype_names() {
    for mut column in quotes() {
        column
            .push(Scalar::from(9_i64))
            .expect("the id member's own value");
        column
            .push(Scalar::from("MSFT"))
            .expect("the symbol member's own value");
        // A union spells absence inside a member, so a bare null is the
        // payload of the one member that takes a null.
        column
            .push(Scalar::Null)
            .expect("the nullable symbol member holds absence");

        assert_eq!(column.scalar(4).unwrap(), quote(0, Scalar::from(9_i64)));
        assert_eq!(column.scalar(5).unwrap(), quote(1, Scalar::from("MSFT")));
        assert_eq!(column.scalar(6).unwrap(), quote(1, Scalar::Null));
        assert!(column.is_null(6).unwrap());
    }
}

#[test]
fn a_null_member_holds_absence_and_crosses_arrow_unchanged() {
    for mode in [UnionMode::Dense, UnionMode::Sparse] {
        let field = Field::new(
            "choice",
            DataType::union(
                [
                    (0, Field::new("int", DataType::Int64, false)),
                    (1, Field::new("str", DataType::utf8(), false)),
                    (2, Field::new("NoneType", DataType::Null, true)),
                ],
                mode,
            )
            .expect("three members"),
            true,
        );
        let column = Serie::from_scalars(
            field.clone(),
            [Scalar::from(1_i64), Scalar::from("a"), Scalar::Null],
        )
        .expect("each value names its member");
        let rows = vec![
            quote(0, Scalar::from(1_i64)),
            quote(1, Scalar::from("a")),
            quote(2, Scalar::Null),
        ];
        assert_eq!(column.rows().into_owned(), rows);
        assert_eq!(column.null_count(), 1);

        let array = column.clone().into_arrow_array().expect("an Arrow union");
        let again = Serie::from_arrow_array(Some(&field), array, strict())
            .expect("the null member's absence is its own");
        assert_eq!(again.rows().into_owned(), rows);
    }
}

#[test]
fn the_buffers_cross_in_and_out_shared_and_a_row_reads_through_its_type_id() {
    let array = dense_quotes();
    let dense = Serie::from_arrow_array(
        Some(&quote_field(UnionMode::Dense, true)),
        Arc::clone(&array),
        ArrowCastOptions::new(),
    )
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
    let sparse = Serie::from_arrow_array(
        Some(&quote_field(UnionMode::Sparse, true)),
        Arc::clone(&array),
        ArrowCastOptions::new(),
    )
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
fn the_writes_keep_the_layout_and_the_rows_round_trip() {
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

#[test]
fn a_sparse_write_splices_every_member_and_a_dense_push_lands_on_its_member() {
    let [dense, sparse] = quotes();

    // Sparse: every member is spliced over the rows written, its payload
    // where the row is its own and a placeholder elsewhere.
    let mut sparse = sparse;
    sparse
        .set(1, quote(0, Scalar::from(7_i64)))
        .expect("one slot");
    sparse.push(quote(1, Scalar::from("IBM"))).expect("a row");
    let leaf = sparse.as_union().expect("a union column");
    assert_eq!(leaf.type_ids(), &[0, 0, 1, 0, 1]);
    assert_eq!(leaf.offsets(), None);
    assert!(leaf.children().iter().all(|child| child.len() == 5));
    assert_eq!(
        leaf.child_of(0).expect("the id member").scalar(1).unwrap(),
        Scalar::from(7_i64)
    );
    assert_eq!(
        leaf.child_of(1)
            .expect("the symbol member")
            .scalar(4)
            .unwrap(),
        Scalar::from("IBM")
    );

    // Dense: a push lands at the end of its own member, and the offset
    // records where.
    let mut dense = dense;
    dense.push(quote(1, Scalar::from("IBM"))).expect("a row");
    dense.push(quote(0, Scalar::from(3_i64))).expect("a row");
    let leaf = dense.as_union().expect("a union column");
    assert_eq!(leaf.type_ids(), &[0, 1, 1, 0, 1, 0]);
    assert_eq!(leaf.offsets(), Some(&[0, 0, 1, 1, 2, 2][..]));
    assert_eq!(leaf.child_of(0).map(Serie::len), Some(3));
    assert_eq!(leaf.child_of(1).map(Serie::len), Some(3));

    // Any other dense write rebuilds, and the rebuild keeps only the member
    // slots a row still reaches.
    dense.remove(1).expect("a row");
    dense
        .set(0, quote(1, Scalar::from("MSFT")))
        .expect("one slot");
    let leaf = dense.as_union().expect("a union column");
    assert_eq!(leaf.type_ids(), &[1, 1, 0, 1, 0]);
    assert_eq!(leaf.child_of(0).map(Serie::len), Some(2));
    assert_eq!(leaf.child_of(1).map(Serie::len), Some(3));
    assert_eq!(
        dense.rows().into_owned(),
        vec![
            quote(1, Scalar::from("MSFT")),
            quote(1, Scalar::Null),
            quote(0, Scalar::from(2_i64)),
            quote(1, Scalar::from("IBM")),
            quote(0, Scalar::from(3_i64)),
        ]
    );
}

#[test]
fn a_long_walk_of_splices_reads_as_the_same_splices_over_a_plain_run() {
    let vocabulary = [
        quote(0, Scalar::from(1_i64)),
        quote(0, Scalar::from(2_i64)),
        quote(1, Scalar::from("AAPL")),
        quote(1, Scalar::Null),
    ];
    for mode in [UnionMode::Dense, UnionMode::Sparse] {
        let field = quote_field(mode, true);
        let mut column = Serie::empty(field.clone()).expect("an empty column");
        let mut expected: Vec<Scalar> = Vec::new();
        // A fixed linear congruential walk, weighted towards appends so a
        // dense column exercises both of its paths.
        let mut state: u64 = 0xd1ce;
        let mut next = |bound: usize| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            usize::try_from(state >> 33).unwrap() % bound
        };
        for _ in 0..400 {
            let len = expected.len();
            let start = if next(2) == 0 { len } else { next(len + 1) };
            let end = start + next(len - start + 1).min(2);
            let rows: Vec<Scalar> = (0..next(4))
                .map(|_| vocabulary[next(vocabulary.len())].clone())
                .collect();
            column
                .splice(start..end, rows.clone())
                .expect("rows the field accepts");
            expected.splice(start..end, rows);

            assert_eq!(column.rows().into_owned(), expected, "{mode:?}");
            let leaf = column.as_union().expect("a union column");
            match leaf.offsets() {
                None => assert!(
                    leaf.children()
                        .iter()
                        .all(|child| child.len() == expected.len())
                ),
                Some(offsets) => {
                    for (type_id, offset) in leaf.type_ids().iter().zip(offsets) {
                        let child = leaf.child_of(*type_id).expect("a declared member");
                        assert!(usize::try_from(*offset).unwrap() < child.len());
                    }
                }
            }
        }
        let crossed = Serie::from_arrow_array(
            Some(&field),
            column.require_arrow_array().unwrap(),
            ArrowCastOptions::new(),
        )
        .expect("the column crosses back in");
        assert_eq!(crossed, column, "{mode:?}");
        assert_eq!(
            column.null_count(),
            expected
                .iter()
                .filter(|row| **row == quote(1, Scalar::Null))
                .count()
        );
    }
}
