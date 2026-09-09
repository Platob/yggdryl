//! The fixed row: tag-named columns, derived facts, and the two closing lists.

use super::OneMessage;

use std::sync::Arc;

use yggdryl::{DataType, Field, FixCodec, FixRegistry, Scalar, fix_schema};

fn reader() -> (Arc<FixRegistry>, FixCodec) {
    let registry = super::committed_registry();
    let reader = FixCodec::new(Arc::clone(&registry));
    (registry, reader)
}

/// One column's value out of a fixed row, by the tag it is named for.
///
/// The column is found by the name the tag spells, because that is all a
/// fixed schema is: the tag never moves, so neither does the column.
fn at<'row>(row: &'row Scalar, schema: &Field, tag: i32) -> &'row Scalar {
    let index = schema
        .index_of(&tag.to_string())
        .unwrap_or_else(|| panic!("a column for tag {tag}"));
    &row.as_sequence().expect("a row")[index]
}

/// Where one tag's column sits in a fixed schema.
fn column_of(schema: &Field, tag: i32) -> usize {
    schema
        .index_of(&tag.to_string())
        .unwrap_or_else(|| panic!("a column for tag {tag}"))
}

#[test]
fn the_columns_are_the_tags_and_they_do_not_move() {
    let (registry, _) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();
    let names: Vec<&str> = schema.fields().iter().map(Field::name).collect();

    // A tag is the one name a field has in every version and dialect: 32 is
    // `LastShares` in 4.2 and `LastQty` in a newest one, and the column is
    // `32` in both.
    assert_eq!(&names[..3], ["8", "9", "35"]);
    assert_eq!(schema.index_of("35"), Some(2));
    assert_eq!(
        &names[names.len() - 2..],
        ["nofixentries", "nounmappedfixentries"],
    );

    // The dictionary's own typing reaches the column, so a currency column is
    // the packed currency and a side is the packed side.
    let fields = schema.fields();
    let typed = |tag: i32| fields[column_of(&schema, tag)].dtype().clone();
    assert_eq!(typed(15), DataType::Currency, "Currency(15)");
    assert_eq!(typed(120), DataType::Currency, "SettlCurrency(120)");
    assert_eq!(typed(54), DataType::Side, "Side(54)");
    assert_eq!(typed(35), DataType::Utf8, "MsgType(35)");
    assert!(
        matches!(typed(60), DataType::DateTime64 { .. }),
        "TransactTime"
    );
    assert_eq!(typed(44), DataType::Float64, "Price(44)");

    // Crate-owned columns follow the same contract as FIX's: the stable
    // identity is the folded name, while renderers receive the FIX-style
    // spelling the schema keeps once the column takes the tag's name.
    for (tag, display) in [
        (yggdryl::MSGHASH_TAG, "MsgHash"),
        (yggdryl::VERSION_TAG, "Version"),
        (yggdryl::SYMBOLTICKER_TAG, "SymbolTicker"),
        (yggdryl::TIMESTAMP_TAG, "Timestamp"),
        (yggdryl::UNIXPARTITION_TAG, "UnixPartition"),
        (yggdryl::PARENTCLORDID_TAG, "ParentClOrdID"),
        (yggdryl::PARENTORDERID_TAG, "ParentOrderID"),
    ] {
        let field = &fields[column_of(&schema, tag)];
        assert_eq!(field.display(), Some(display), "tag {tag}");
    }

    // Every column is nullable, because a message that carried nothing there
    // must answer null rather than shift its neighbours.
    assert!(fields.iter().all(Field::is_nullable));
}

#[test]
fn a_row_fills_every_column_by_tag_and_never_shifts() {
    let (registry, reader) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();

    let order = reader
        .one_line(b"8=FIX.4.4|35=D|11=ORDER-1|55=AAPL|54=1|44=12.5|38=100|15=USD|60=20240102-10:15:30.000|10=0|", false)
        .unwrap();
    let row = order.into_row(&schema).unwrap();
    assert_eq!(at(&row, &schema, 35).as_str(), Some("D"));
    assert_eq!(at(&row, &schema, 11).as_str(), Some("ORDER-1"));
    assert_eq!(at(&row, &schema, 55).as_str(), Some("AAPL"));
    assert_eq!(at(&row, &schema, 15).as_str(), Some("USD"));
    assert_eq!(at(&row, &schema, 44), &Scalar::from(12.5_f64));

    // A message that carried almost nothing has the same columns in the same
    // places, which is what makes two rows of one capture comparable.
    let bare = reader.one_line(b"8=FIX.4.4|35=0|10=0|", false).unwrap();
    let thin = bare.into_row(&schema).unwrap();
    assert_eq!(
        thin.as_sequence().map(<[Scalar]>::len),
        row.as_sequence().map(<[Scalar]>::len),
    );
    assert_eq!(at(&thin, &schema, 35).as_str(), Some("0"));
    assert!(at(&thin, &schema, 55).is_null(), "no symbol, not a shift");
}

#[test]
fn the_derived_columns_are_computed_and_never_stored() {
    let (registry, reader) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();
    let order = reader
        .one_line(
            b"8=FIX.4.4|35=D|11=A|55=AAPL|207=XNAS|60=20240102-10:15:30.000|10=0|",
            false,
        )
        .unwrap();
    let row = order.into_row(&schema).unwrap();

    // The digest is sixteen bytes of value, not a rendered string.
    let digest = at(&row, &schema, yggdryl::MSGHASH_TAG);
    assert_eq!(digest.as_bytes().map(<[u8]>::len), Some(16));

    // One ticker for one instrument, qualified by the venue that named it.
    assert_eq!(
        at(&row, &schema, yggdryl::SYMBOLTICKER_TAG).as_str(),
        Some("AAPL@XNAS"),
    );

    // The clock, and the partition it falls in - an hour, floored, so a row
    // lands in the partition that contains it.
    assert!(!at(&row, &schema, yggdryl::TIMESTAMP_TAG).is_null());
    let partition = at(&row, &schema, yggdryl::UNIXPARTITION_TAG);
    let seconds = 1_704_190_530_i64; // 2024-01-02T10:15:30Z
    assert_eq!(partition, &Scalar::from(seconds - seconds % 3_600));

    // The version it was read at, which is not always what the frame claimed.
    assert_eq!(
        at(&row, &schema, yggdryl::VERSION_TAG).as_str(),
        Some("4.4")
    );

    // And nothing of it was stored: the message is what it was.
    assert!(order.get_by_tag(yggdryl::MSGHASH_TAG).is_none());
}

#[test]
fn an_identifier_carries_its_scheme_and_a_ticker_does_not() {
    let (_, reader) = reader();
    // A plain ticker is itself; an identifier is qualified by the scheme that
    // numbers it, because `US0378331005` does not say it is an ISIN.
    let ticker = reader
        .one_line(b"8=FIX.4.4|35=D|55=AAPL|10=0|", false)
        .unwrap();
    assert_eq!(ticker.symbol_ticker().as_str(), Some("AAPL"));

    let identified = reader
        .one_line(b"8=FIX.4.4|35=D|48=US0378331005|22=4|207=XNAS|10=0|", false)
        .unwrap();
    assert_eq!(
        identified.symbol_ticker().as_str(),
        Some("4:US0378331005@XNAS"),
    );

    // A message naming no instrument answers nothing rather than a guess.
    let none = reader.one_line(b"8=FIX.4.4|35=0|10=0|", false).unwrap();
    assert!(none.symbol_ticker().is_null());
}

#[test]
fn a_lane_a_message_never_wrote_is_still_true_of_it() {
    let (registry, reader) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();

    // A buy order at a price is a party willing to pay it, so the bid lane it
    // never wrote is filled and the ask lane is not.
    let buy = reader
        .one_line(b"8=FIX.4.4|35=D|11=A|54=1|44=12.5|38=100|10=0|", false)
        .unwrap();
    let row = buy.into_row(&schema).unwrap();
    assert_eq!(at(&row, &schema, 132), &Scalar::from(12.5_f64));
    assert_eq!(at(&row, &schema, 134), &Scalar::from(100.0_f64));
    assert!(at(&row, &schema, 133).is_null(), "no ask lane on a buy");

    // A one-sided quote implies the side it never wrote.
    let quote = reader
        .one_line(b"8=FIX.4.4|35=S|117=Q|132=12.4|10=0|", false)
        .unwrap();
    let row = quote.into_row(&schema).unwrap();
    assert_eq!(at(&row, &schema, 54).as_str(), Some("1"));

    // And a stated column is never overwritten by a derivation.
    let stated = reader
        .one_line(b"8=FIX.4.4|35=D|11=A|54=1|44=12.5|132=99.0|10=0|", false)
        .unwrap();
    let row = stated.into_row(&schema).unwrap();
    assert_eq!(at(&row, &schema, 132), &Scalar::from(99.0_f64));
}

#[test]
fn the_row_stays_lossless_and_says_what_nothing_explained() {
    let (registry, reader) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();
    let row = reader
        .one_line(b"8=FIX.4.4|35=D|11=A|9999=x|VenueOwnThing=y|10=0|", false)
        .unwrap()
        .into_row(&schema)
        .unwrap();
    let held = row.as_sequence().expect("a row");
    let entries = held[held.len() - 2].as_sequence().expect("the record");
    let unmapped = held[held.len() - 1].as_sequence().expect("the unmapped");

    // The record is everything that arrived, in arrival order, so the wire is
    // rebuilt from it and never from the columns.
    assert_eq!(entries.len(), 6);
    // The view is the part of it nothing explained, and holds nothing the
    // record does not.
    assert_eq!(unmapped.len(), 2);
    for entry in unmapped {
        assert!(entries.contains(entry));
    }
}

/// The two documents a datatype writes name it the same way.
///
/// A datatype is written twice by this crate: as a `Scalar` record, which is
/// what a `FixMsg` schema and a registry shard carry, and by the serde derive
/// behind `into_json`. Whoever holds a table reads one with the other, so a
/// type the two spell differently is a schema that crosses in only one
/// direction. `MsgDirection` was that -- `msgdirection` as a record and
/// `msg_direction` from the derive -- and no `fix_schema` carrying tag 385
/// survived the crossing.
#[test]
fn a_datatype_is_named_the_same_by_both_documents() {
    use yggdryl::{DataType, DataTypeId, Field, Scalar};

    for id in DataTypeId::ALL {
        // Only the parameterless ones are nameable without a shape; the
        // parameterized families are covered by their own suites.
        if id.is_parameterized() {
            continue;
        }
        let Ok(dtype) = DataType::from_str(id.as_str()) else {
            continue;
        };
        let record = dtype.clone().into_value();
        let stated = record
            .get_key_str("type")
            .and_then(Scalar::as_utf8)
            .map(str::to_owned);
        let document = dtype
            .clone()
            .nullable_field("held")
            .into_json()
            .expect("a field renders");
        let held: serde_json::Value =
            serde_json::from_str(&document).expect("a field document is JSON");
        assert_eq!(
            held["dtype"]["type"].as_str(),
            stated.as_deref(),
            "{} is written under two spellings",
            id.as_str()
        );
        let read = Field::from_json(&document)
            .unwrap_or_else(|error| panic!("{} does not read back: {error}", id.as_str()));
        assert_eq!(read.dtype().id(), id, "{} changed identity", id.as_str());
    }
}
