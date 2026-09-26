//! `rust/src/limit.rs`: one price limit of a book side, its datatype, its
//! field and its scalar.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use yggdryl::{Decimal, Error, Limit, Scalar, Uuid};

fn priced() -> Limit {
    Limit {
        price: Some("101.5".parse().unwrap()),
        quantity: Decimal::from_int(300),
        uuids: vec![Uuid::from_v8(1), Uuid::from_v8(2)],
    }
}

fn unpriced() -> Limit {
    Limit {
        price: None,
        quantity: Decimal::from_int(7),
        uuids: vec![Uuid::from_v8(3)],
    }
}

/// The path and the reason a refusal names.
fn refused(value: &Scalar) -> (String, String) {
    match Limit::from_scalar(value).unwrap_err() {
        Error::InvalidRecord { path, reason } => (path.to_string(), reason.to_string()),
        other => panic!("expected a located record refusal, got {other}"),
    }
}

/// A struct scalar under the three names, `edit` replacing one cell.
fn struct_with(name: &str, cell: Scalar) -> Scalar {
    let limit = priced().into_scalar();
    let fields = limit.as_struct().unwrap();
    Scalar::from_struct(fields.iter().map(|(key, value)| {
        let value = if key == name {
            cell.clone()
        } else {
            value.clone()
        };
        (key.clone(), value)
    }))
    .unwrap()
}

/// The canonical ordered row, the cell at `index` replaced.
fn row_with(index: usize, cell: Scalar) -> Scalar {
    let row = Limit::dtype().scalar(priced().into_scalar()).unwrap();
    let mut cells = row.sequence_rows().unwrap().into_owned();
    cells[index] = cell;
    Scalar::from_sequence(cells)
}

fn hashed(limit: &Limit) -> u64 {
    let mut hasher = DefaultHasher::new();
    limit.hash(&mut hasher);
    hasher.finish()
}

#[test]
fn the_datatype_is_a_nullable_price_a_quantity_and_the_entries() {
    assert_eq!(
        Limit::dtype().to_string(),
        "struct(\
         field(\"price\",decimal,nullable=true,metadata={}),\
         field(\"quantity\",decimal,nullable=false,metadata={}),\
         field(\"uuids\",serie(field(\"uuid\",uuid,nullable=false,metadata={})),nullable=false,metadata={}))"
    );
}

#[test]
fn the_field_is_the_required_item_of_a_limits_column() {
    let field = Limit::field();
    assert_eq!(field.name(), "limit");
    assert!(!field.is_nullable());
    assert_eq!(field.dtype(), &Limit::dtype());
}

#[test]
fn into_scalar_is_the_struct_of_the_three_names() {
    let scalar = priced().into_scalar();
    let fields = scalar.as_struct().expect("a struct scalar");
    assert_eq!(
        fields.keys().map(|key| key.as_str()).collect::<Vec<_>>(),
        ["price", "quantity", "uuids"]
    );
    assert_eq!(fields["price"], Scalar::from(priced().price.unwrap()));
    assert_eq!(fields["quantity"], Scalar::from(Decimal::from_int(300)));
    assert_eq!(
        fields["uuids"].sequence_rows().unwrap().into_owned(),
        [
            Scalar::Uuid(Uuid::from_v8(1)),
            Scalar::Uuid(Uuid::from_v8(2))
        ]
    );
    let scalar = unpriced().into_scalar();
    assert_eq!(scalar.as_struct().unwrap()["price"], Scalar::Null);
}

#[test]
fn from_scalar_reads_the_struct_and_the_canonical_row() {
    for limit in [
        priced(),
        unpriced(),
        Limit {
            price: Some(Decimal::ZERO),
            quantity: Decimal::ZERO,
            uuids: Vec::new(),
        },
    ] {
        assert_eq!(Limit::from_scalar(&limit.into_scalar()).unwrap(), limit);
        let row = Limit::dtype().scalar(limit.into_scalar()).unwrap();
        assert!(row.as_struct().is_none(), "the canonical shape is a row");
        assert_eq!(Limit::from_scalar(&row).unwrap(), limit);
        let row = Limit::field().scalar(limit.into_scalar()).unwrap();
        assert_eq!(Limit::from_scalar(&row).unwrap(), limit);
    }
}

#[test]
fn a_price_that_is_no_decimal_is_refused_at_its_path() {
    let wrong = Scalar::Uuid(Uuid::from_v8(9));
    for value in [
        struct_with("price", wrong.clone()),
        row_with(0, wrong.clone()),
    ] {
        let (path, reason) = refused(&value);
        assert_eq!(path, "$.price");
        assert_eq!(
            reason,
            format!("expected a decimal or null, got {}", wrong.kind())
        );
    }
}

#[test]
fn a_null_or_non_decimal_quantity_is_refused_at_its_path() {
    let wrong = Scalar::Uuid(Uuid::from_v8(9));
    for (cell, got) in [(Scalar::Null, "null"), (wrong.clone(), wrong.kind())] {
        for value in [
            struct_with("quantity", cell.clone()),
            row_with(1, cell.clone()),
        ] {
            let (path, reason) = refused(&value);
            assert_eq!(path, "$.quantity");
            assert_eq!(reason, format!("expected a decimal, got {got}"));
        }
    }
}

#[test]
fn uuids_that_are_no_serie_are_refused_at_their_path() {
    for cell in [Scalar::Null, Scalar::from("O-1")] {
        let got = cell.kind();
        for value in [
            struct_with("uuids", cell.clone()),
            row_with(2, cell.clone()),
        ] {
            let (path, reason) = refused(&value);
            assert_eq!(path, "$.uuids");
            assert_eq!(reason, format!("expected a serie of uuids, got {got}"));
        }
    }
}

#[test]
fn a_uuid_that_is_no_uuid_is_refused_at_its_index() {
    let wrong = Scalar::from(1_i64);
    let uuids = Scalar::from_sequence([Scalar::Uuid(Uuid::from_v8(1)), wrong.clone()]);
    for value in [
        struct_with("uuids", uuids.clone()),
        row_with(2, uuids.clone()),
    ] {
        let (path, reason) = refused(&value);
        assert_eq!(path, "$.uuids[1]");
        assert_eq!(reason, format!("expected a uuid, got {}", wrong.kind()));
    }
}

#[test]
fn a_row_of_another_width_an_unknown_name_or_another_shape_is_refused() {
    let (path, reason) = refused(&Scalar::from_sequence([Scalar::Null, Scalar::Null]));
    assert_eq!(path, "$");
    assert_eq!(reason, "expected a row of 3 cells, got 2 cells");

    let named = Scalar::from_struct([
        ("price", Scalar::Null),
        ("quantity", Scalar::from(Decimal::ONE)),
        ("uuids", Scalar::from_sequence([])),
        ("venue", Scalar::from("XNAS")),
    ])
    .unwrap();
    let (path, reason) = refused(&named);
    assert_eq!(path, "$.venue");
    assert_eq!(
        reason,
        "expected price, quantity or uuids, got an unknown field"
    );

    let text = Scalar::from("101");
    let (path, reason) = refused(&text);
    assert_eq!(path, "$");
    assert_eq!(
        reason,
        format!("expected a limit struct or its row, got {}", text.kind())
    );
}

#[test]
fn equality_and_hash_read_the_entries_in_order() {
    let mut swapped = priced();
    swapped.uuids.reverse();
    assert_ne!(swapped, priced());
    assert_ne!(hashed(&swapped), hashed(&priced()));
    assert_eq!(hashed(&priced()), hashed(&priced().clone()));
}
