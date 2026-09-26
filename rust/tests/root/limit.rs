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

/// What `Limit::from_scalar` answers, asserted to be what the one value door
/// `Limit::field().scalar` answers: its refusal as it stands, or the limit
/// its canonical row reads as.
fn read_as_the_door(value: &Scalar) -> Result<Limit, String> {
    let read = Limit::from_scalar(value).map_err(|error| error.to_string());
    match Limit::field().scalar(value.clone()) {
        Err(door) => assert_eq!(read, Err(door.to_string())),
        Ok(row) => assert_eq!(
            read,
            Limit::from_scalar(&row).map_err(|error| error.to_string())
        ),
    }
    read
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
fn a_cell_is_refused_exactly_as_the_value_door_refuses_it() {
    let wrong = Scalar::Uuid(Uuid::from_v8(9));
    let mixed = Scalar::from_sequence([Scalar::Uuid(Uuid::from_v8(1)), Scalar::from(1_i64)]);
    for (name, index, cell, path) in [
        ("price", 0, wrong.clone(), "$.limit.price"),
        ("quantity", 1, wrong.clone(), "$.limit.quantity"),
        ("quantity", 1, Scalar::Null, "$.limit.quantity"),
        ("uuids", 2, Scalar::Null, "$.limit.uuids"),
        ("uuids", 2, Scalar::from("O-1"), "$.limit.uuids"),
        ("uuids", 2, mixed, "$.limit.uuids[1].uuid"),
    ] {
        for value in [
            struct_with(name, cell.clone()),
            row_with(index, cell.clone()),
        ] {
            assert!(read_as_the_door(&value).is_err(), "{name} = {cell:?}");
            assert_eq!(refused(&value).0, path, "{name} = {cell:?}");
        }
    }
}

#[test]
fn an_empty_text_is_never_a_price_or_a_quantity_of_zero() {
    let empty = Scalar::from("");
    for (name, index) in [("price", 0), ("quantity", 1)] {
        for value in [
            struct_with(name, empty.clone()),
            row_with(index, empty.clone()),
        ] {
            match read_as_the_door(&value) {
                // The door refuses the text where it stands ...
                Err(_) => assert_eq!(refused(&value).0, format!("$.limit.{name}")),
                // ... or reads it as a null: no price, and a quantity is required.
                Ok(limit) => {
                    assert_eq!(name, "price", "a null quantity is refused");
                    assert_eq!(limit.price, None);
                }
            }
        }
    }
}

#[test]
fn a_float_grouped_digits_or_a_nineteenth_fractional_digit_is_refused() {
    for (name, index, cell) in [
        ("quantity", 1, Scalar::from(0.1_f64)),
        ("price", 0, Scalar::from(101.5_f64)),
        ("price", 0, Scalar::from("1,250.50")),
        ("quantity", 1, Scalar::from("1,250")),
        ("price", 0, Scalar::from("0.1234567890123456789")),
        ("quantity", 1, Scalar::from("7.0000000000000000001")),
    ] {
        for value in [
            struct_with(name, cell.clone()),
            row_with(index, cell.clone()),
        ] {
            assert!(read_as_the_door(&value).is_err(), "{name} = {cell:?}");
            assert_eq!(refused(&value).0, format!("$.limit.{name}"));
        }
    }
}

#[test]
fn what_the_value_door_accepts_reads_as_the_limit_it_canonicalizes_to() {
    let limit = read_as_the_door(&struct_with("price", Scalar::from("101.25"))).unwrap();
    assert_eq!(limit.price, Some("101.25".parse().unwrap()));
    let limit = read_as_the_door(&row_with(1, Scalar::from(5_i64))).unwrap();
    assert_eq!(limit.quantity, Decimal::from_int(5));
    // A name the struct lacks is a null: a missing price is none.
    let missing = Scalar::from_struct([
        ("quantity", Scalar::from(Decimal::ONE)),
        ("uuids", Scalar::from_sequence([])),
    ])
    .unwrap();
    assert_eq!(read_as_the_door(&missing).unwrap().price, None);
}

#[test]
fn a_name_the_struct_lacks_is_a_null_never_the_default_a_required_cell_takes() {
    for name in ["quantity", "uuids"] {
        let fields = priced().into_scalar();
        let lacking = Scalar::from_struct(
            fields
                .as_struct()
                .unwrap()
                .iter()
                .filter(|(key, _)| key.as_str() != name)
                .map(|(key, value)| (key.clone(), value.clone())),
        )
        .unwrap();
        // The door alone fills a required cell with its default - a quantity
        // of zero - which a limit never states.
        assert!(Limit::field().scalar(lacking.clone()).is_ok());
        let stated_null = read_as_the_door(&struct_with(name, Scalar::Null)).unwrap_err();
        assert_eq!(
            Limit::from_scalar(&lacking).map_err(|error| error.to_string()),
            Err(stated_null),
            "{name}"
        );
        assert_eq!(refused(&lacking).0, format!("$.limit.{name}"));
    }
}

#[test]
fn a_row_of_another_width_an_unknown_name_or_another_shape_is_refused() {
    let named = Scalar::from_struct([
        ("price", Scalar::Null),
        ("quantity", Scalar::from(Decimal::ONE)),
        ("uuids", Scalar::from_sequence([])),
        ("venue", Scalar::from("XNAS")),
    ])
    .unwrap();
    for value in [
        Scalar::from_sequence([Scalar::Null, Scalar::Null]),
        named,
        Scalar::from("101"),
        Scalar::Null,
    ] {
        assert!(read_as_the_door(&value).is_err(), "{value:?}");
        assert!(
            refused(&value).0.starts_with("$.limit"),
            "{value:?} is refused under the field"
        );
    }
    let (_, reason) = refused(
        &Scalar::from_struct([
            ("price", Scalar::Null),
            ("quantity", Scalar::from(Decimal::ONE)),
            ("uuids", Scalar::from_sequence([])),
            ("venue", Scalar::from("XNAS")),
        ])
        .unwrap(),
    );
    assert!(reason.contains("venue"), "{reason}");
}

#[test]
fn equality_and_hash_read_the_entries_in_order() {
    let mut swapped = priced();
    swapped.uuids.reverse();
    assert_ne!(swapped, priced());
    assert_ne!(hashed(&swapped), hashed(&priced()));
    assert_eq!(hashed(&priced()), hashed(&priced().clone()));
}
