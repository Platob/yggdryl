//! `rust/src/graph/element_column.rs`: the six columns every generated
//! schema opens with, each stating back exactly the fact it read.

use yggdryl::graph::{Element, ElementColumn, OrderEvent};
use yggdryl::{Scalar, Uuid};

#[test]
fn every_column_states_back_what_it_read() {
    let mut element = OrderEvent::at(1_700_000_000_000_000_000);
    element.set_curruuid(Uuid::from_v8(1));
    element.set_crossuuid(Uuid::from_v8(2));
    element.set_crosscode("O-1".to_owned());
    element.set_currhashcode(3);
    element.set_crosshashcode(4);
    element.set_srcuuids(vec![Uuid::from_v8(9)]);
    // Dated as the source is: a stated digest re-derives the identity off
    // the instant.
    let mut again = OrderEvent::at(1_700_000_000_000_000_000);
    for column in ElementColumn::ALL {
        let fact = column.fact(&element).expect("every fact is stated");
        column
            .datatype()
            .required_field(column.name())
            .scalar(fact.clone())
            .expect("the fact fits the column");
        column.record(&mut again, &fact);
    }
    assert_eq!(again.get_curruuid(), element.get_curruuid());
    assert_eq!(again.get_crossuuid(), element.get_crossuuid());
    // The leaf stores the code under its category and side, and the column
    // states that stored code back: recording it again is the identity.
    assert_eq!(element.get_crosscode(), "10:0:O-1");
    assert_eq!(again.get_crosscode(), "10:0:O-1");
    assert_eq!(again.get_currhashcode(), element.get_currhashcode());
    assert_eq!(again.get_crosshashcode(), element.get_crosshashcode());
    assert_eq!(again.get_srcuuids(), [Uuid::from_v8(9)]);
}

#[test]
fn a_digest_a_table_stored_as_a_whole_decimal_reads_back_as_the_number_it_was() {
    // An Iceberg table has no unsigned type, so a `uint64` digest is stored
    // as `decimal(20, 0)`: the column reads the cell through its own value
    // door, so the digest returns as it was - read back as zero, the row
    // was another message's delivery - and a cell the door refuses states
    // nothing.
    let mut element = OrderEvent::at(7);
    ElementColumn::CurrHashCode.record(&mut element, &Scalar::decimal128(i128::from(u64::MAX), 0));
    ElementColumn::CrossHashCode.record(&mut element, &Scalar::decimal128(400, 2));
    assert_eq!(element.get_currhashcode(), u64::MAX);
    assert_eq!(element.get_crosshashcode(), 4);
    ElementColumn::CurrHashCode.record(&mut element, &Scalar::decimal128(-1, 0));
    ElementColumn::CrossHashCode.record(&mut element, &Scalar::from("digest"));
    assert_eq!(element.get_currhashcode(), u64::MAX);
    assert_eq!(element.get_crosshashcode(), 4);
}

#[test]
fn a_digest_a_table_stored_as_a_long_reads_back_as_its_bits() {
    // A table that stores a digest as the `long` of its width holds its
    // bits: a negative cell is no other `u64`, and a cell both readings
    // agree on is the number it is.
    let mut element = OrderEvent::at(7);
    ElementColumn::CurrHashCode.record(&mut element, &Scalar::from(-1_i64));
    ElementColumn::CrossHashCode.record(&mut element, &Scalar::from(i64::MIN));
    assert_eq!(element.get_currhashcode(), u64::MAX);
    assert_eq!(element.get_crosshashcode(), 1 << 63);
    ElementColumn::CurrHashCode.record(&mut element, &Scalar::from(5_i64));
    assert_eq!(element.get_currhashcode(), 5);
    // Only the width of the digest carries its bits.
    ElementColumn::CrossHashCode.record(&mut element, &Scalar::from(-1_i32));
    assert_eq!(element.get_crosshashcode(), 1 << 63);
}

#[test]
fn a_null_clears_and_nothing_stated_is_none() {
    let mut element = OrderEvent::at(7);
    element.set_crosscode("X".to_owned());
    element.set_srcuuids(vec![Uuid::from_v8(1)]);
    ElementColumn::CrossCode.record(&mut element, &Scalar::Null);
    ElementColumn::SrcUuids.record(&mut element, &Scalar::Null);
    // An identity is never absent: a null states nothing.
    let identity = element.get_curruuid();
    ElementColumn::CurrUuid.record(&mut element, &Scalar::Null);
    assert_eq!(element.get_crosscode(), "");
    assert!(element.get_srcuuids().is_empty());
    assert_eq!(element.get_curruuid(), identity);
    assert_eq!(
        ElementColumn::CrossCode.fact(&element),
        Some(Scalar::from("")),
        "the code is never absent: a null reads as the empty text"
    );
    assert_eq!(ElementColumn::SrcUuids.fact(&element), None);
    assert!(ElementColumn::CurrUuid.fact(&element).is_some());
}

#[test]
fn the_columns_are_the_element_trait_s_in_one_order() -> yggdryl::Result<()> {
    let names: Vec<&str> = ElementColumn::ALL
        .iter()
        .map(|column| column.name())
        .collect();
    assert_eq!(
        names,
        [
            "curruuid",
            "crossuuid",
            "crosscode",
            "currhashcode",
            "crosshashcode",
            "srcuuids",
        ]
    );
    let fields = ElementColumn::fields()?;
    let nullable: Vec<&str> = fields
        .iter()
        .filter(|field| field.is_nullable())
        .map(|field| field.name())
        .collect();
    assert_eq!(nullable, ["srcuuids"], "only the sources may be absent");
    for field in &fields {
        assert!(field.display().is_some(), "{}", field.name());
        assert!(field.description().is_some(), "{}", field.name());
    }
    assert_eq!(
        ElementColumn::of_name("CrossCode"),
        Some(ElementColumn::CrossCode)
    );
    // When an element happened is an event's fact.
    assert_eq!(ElementColumn::of_name("currunix"), None);
    Ok(())
}

#[test]
fn a_column_of_identities_records_like_the_run_of_them() -> yggdryl::Result<()> {
    let column = Scalar::from(yggdryl::Serie::from_scalars(
        yggdryl::Field::new("item", yggdryl::DataType::Uuid, false),
        [
            Scalar::Uuid(Uuid::from_v8(7)),
            Scalar::Uuid(Uuid::from_v8(8)),
        ],
    )?);
    assert_eq!(column.as_sequence(), None, "the fixture holds a column");
    let mut element = OrderEvent::default();
    ElementColumn::SrcUuids.record(&mut element, &column);
    assert_eq!(element.get_srcuuids(), [Uuid::from_v8(7), Uuid::from_v8(8)]);
    Ok(())
}
