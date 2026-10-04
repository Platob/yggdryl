//! `rust/src/fix/schema.rs`: the fixed row: columns spelled by name and
//! filled by tag, derived facts, and the one closing arrival record.

use super::SoleMessage;

use std::sync::Arc;

use yggdryl::graph::{Element, Event, Market};
use yggdryl::{
    DataType, Field, FixCodec, FixRegistry, IdKey, Scalar, StructType, fix_column_of, fix_schema,
};

fn reader() -> (Arc<FixRegistry>, FixCodec) {
    let registry = super::committed_registry();
    let reader = super::fixed_codec(Arc::clone(&registry));
    (registry, reader)
}

/// One column's value out of a fixed row, by the tag its field carries.
///
/// The column is found by its tag rather than its spelling, because the tag
/// is what the row is filled by: a venue renaming a field between versions
/// moves nothing.
fn at<'row>(row: &'row Scalar, schema: &Field, tag: i32) -> &'row Scalar {
    &row.as_sequence().expect("a row")[column_of(schema, tag)]
}

/// Where one tag's column sits in a fixed schema.
fn column_of(schema: &Field, tag: i32) -> usize {
    fix_column_of(schema, tag).unwrap_or_else(|| panic!("a column for tag {tag}"))
}

/// The residual record of a fixed row: each key under the text it holds.
fn residual(row: &Scalar, schema: &Field) -> Vec<(String, String)> {
    let at = schema.index_of("fixentries").expect("the residual record");
    row.as_sequence().expect("a row")[at]
        .as_mapping()
        .expect("the residual map")
        .iter()
        .map(|(key, value)| {
            (
                key.as_str().expect("a text key").to_owned(),
                value.as_str().expect("a text value").to_owned(),
            )
        })
        .collect()
}

/// The `metadata` of a fixed row: each key under the text it holds.
fn metadata(row: &Scalar, schema: &Field) -> Vec<(String, String)> {
    let at = schema.index_of("metadata").expect("the metadata column");
    row.as_sequence().expect("a row")[at]
        .as_mapping()
        .map(|entries| {
            entries
                .iter()
                .map(|(key, value)| {
                    (
                        key.as_str().expect("a text key").to_owned(),
                        value.as_str().expect("a text value").to_owned(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The `regulatorytradeids` occurrences out of a fixed row.
///
/// The group is reached by its name, which is the one column it is: its
/// counter's tag 1907 names it and is no column of its own.
fn group<'row>(row: &'row Scalar, schema: &Field) -> &'row [Scalar] {
    let at = schema
        .index_of("regulatorytradeids")
        .expect("a regulatorytradeids column");
    row.as_sequence().expect("a row")[at]
        .as_sequence()
        .expect("the regulatory trade identifiers")
}

#[test]
fn the_fixed_schema_keeps_existing_tags_and_appends_the_settled_identity_fields() {
    use yggdryl::fix::{BODY_TAGS, GROUP_TAGS, HEADER_TAGS, TRAILER_TAGS};

    // `SecurityID(48)`, `SecurityIDSource(22)`, `Parties(453)` and
    // `SecAltIDGrp(454)` are no columns: the prefix's `secaltids` and
    // `parties` state what they name, and a message stating them keeps them
    // among its entries.
    let tags = yggdryl::fix_schema_tags();
    assert_eq!(tags.len(), 149);
    for tag in [22, 48, 453, 454] {
        assert!(!tags.contains(&tag), "{tag} is no column");
    }
    // The row opens as every generated schema does: the element's, the
    // event's, the market's and the operation's facts, the crate's own tag
    // or - for a market or an operation column FIX already names alike -
    // that field's.
    let shared = 6 + 9 + 34 + 5;
    assert_eq!(
        &tags[..15],
        [
            yggdryl::CURRUUID_TAG_NAME.0,
            yggdryl::CROSSUUID_TAG_NAME.0,
            yggdryl::CROSSCODE_TAG_NAME.0,
            yggdryl::CURRHASHCODE_TAG_NAME.0,
            yggdryl::CROSSHASHCODE_TAG_NAME.0,
            yggdryl::SRCUUIDS_TAG_NAME.0,
            yggdryl::CURRUNIX_TAG_NAME.0,
            yggdryl::CREAUNIX_TAG_NAME.0,
            yggdryl::RECDUNIX_TAG_NAME.0,
            yggdryl::EXPRUNIX_TAG_NAME.0,
            yggdryl::PREVUNIX_TAG_NAME.0,
            yggdryl::SNAPUNIX_TAG_NAME.0,
            yggdryl::PREVUUID_TAG_NAME.0,
            yggdryl::SEQNUM_TAG_NAME.0,
            yggdryl::STATE_TAG_NAME.0,
        ],
        "the element, then the event"
    );
    assert_eq!(
        &tags[15..25],
        [
            yggdryl::MARKETDATAKIND_TAG_NAME.0,
            yggdryl::MARKETDATATYPE_TAG_NAME.0,
            44,
            99,
            15,
            53,
            1138,
            yggdryl::HIDDENQTY_TAG_NAME.0,
            yggdryl::UNIT_TAG_NAME.0,
            54,
        ],
        "the market's category and type, prices and quantities, FIX's own fields where FIX names them alike"
    );
    assert_eq!(
        &tags[49..shared],
        [
            yggdryl::ORDQTY_TAG_NAME.0,
            59,
            yggdryl::TRADABLE_TAG_NAME.0,
            yggdryl::IDENTIFIERS_TAG_NAME.0,
            yggdryl::PARTYIDS_TAG_NAME.0,
        ],
        "the operation, TimeInForce(59) under its own name"
    );
    assert_eq!(
        &tags[shared..shared + 8],
        [52, 122, 60, 64, 75, 126, 62, 432],
        "then the clocks FIX states"
    );
    assert_eq!(
        &tags[shared + 8..shared + 21],
        [
            8,
            35,
            34,
            49,
            56,
            43,
            yggdryl::MSGDIRECTION_TAG_NAME.0,
            yggdryl::MSGPLUGINID_TAG_NAME.0,
            yggdryl::MSGORIGINATOR_TAG_NAME.0,
            yggdryl::MSGCTXID_TAG_NAME.0,
            yggdryl::MSGSESSIONID_TAG_NAME.0,
            yggdryl::MSGSESSEVENTID_TAG_NAME.0,
            yggdryl::CONVERSATIONID_TAG_NAME.0,
        ],
        "which message, over which session - the plugin it came from right \
         after the bridge's own, the session event it was delivered as right \
         after the session that delivered it, then the conversation"
    );
    // And not where the capture read it: the object a line came out of is
    // the reader's word about the line, carried beside the row with the body
    // and the row number, so no column of this row states it.
    assert!(
        !tags.contains(&yggdryl::SOURCEURL_TAG_NAME.0),
        "the capture's own column is no column of the fixed row"
    );
    // Every tag the four standard lists name still has its column, each
    // exactly once, and a band claiming one early does not repeat it.
    for held in [
        HEADER_TAGS.as_slice(),
        BODY_TAGS.as_slice(),
        GROUP_TAGS.as_slice(),
        TRAILER_TAGS.as_slice(),
    ] {
        for tag in held {
            assert_eq!(
                tags.iter().filter(|held| *held == tag).count(),
                1,
                "tag {tag} once"
            );
        }
    }
    // The frame closes the tagged row.
    assert_eq!(tags.last(), Some(&10));

    let (registry, _) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();
    // The two identifier fields and the two groups - each its list alone,
    // its length the count - are four columns no row holds.
    assert_eq!(schema.fields().len(), 150);
    assert!(
        schema
            .fields()
            .iter()
            .all(|field| !["notrdregtimestamps", "noregulatorytradeids"].contains(&field.name())),
        "no counter column"
    );
    let names: Vec<_> = schema.fields().iter().map(Field::name).collect();
    let expected: Vec<&str> = yggdryl::graph::ElementColumn::ALL
        .map(yggdryl::graph::ElementColumn::name)
        .into_iter()
        .chain(yggdryl::graph::EventColumn::ALL.map(yggdryl::graph::EventColumn::name))
        .chain(yggdryl::graph::MarketColumn::ALL.map(yggdryl::graph::MarketColumn::name))
        .chain(yggdryl::graph::OperationColumn::ALL.map(yggdryl::graph::OperationColumn::name))
        .collect();
    assert_eq!(
        names[..shared],
        expected[..],
        "the shared columns open the row"
    );
    // The frame closes the row: the trailer, then the residual record.
    assert_eq!(
        &names[names.len() - 3..],
        ["signature", "checksum", "fixentries"]
    );
    for tag in [
        yggdryl::EXECUNIX_TAG_NAME.0,
        yggdryl::RECDUNIX_TAG_NAME.0,
        yggdryl::MSGSESSEVENTID_TAG_NAME.0,
        yggdryl::PREVUNIX_TAG_NAME.0,
        yggdryl::PREVUUID_TAG_NAME.0,
    ] {
        assert_eq!(
            schema
                .fields()
                .iter()
                .filter(|field| field.as_fix().tag().unwrap() == Some(tag))
                .count(),
            1
        );
        assert!(schema.fields()[column_of(&schema, tag)].is_nullable());
    }
    let clock = DataType::datetime64(yggdryl::TimeUnit::Nanosecond, yggdryl::Timezone::UTC)
        .expect("the event clock");
    for tag in [yggdryl::EXECUNIX_TAG_NAME.0, yggdryl::RECDUNIX_TAG_NAME.0] {
        assert_eq!(schema.fields()[column_of(&schema, tag)].dtype(), &clock);
    }
    // The session event is the text its four parts join to.
    assert_eq!(
        schema.fields()[column_of(&schema, yggdryl::MSGSESSEVENTID_TAG_NAME.0)].dtype(),
        &DataType::utf8()
    );
    // The merge reference's recording clock is no column: the reference is
    // the latest `recdunix`, which the row already states.
    assert!(schema.index_of("refrecdunix").is_none());
    assert!(!tags.contains(&65_064));
}

#[test]
fn the_columns_are_named_by_fold_and_filled_by_tag() {
    let (registry, _) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();
    let names: Vec<&str> = schema.fields().iter().map(Field::name).collect();

    // A column is spelled by the dictionary's folded name and found by the
    // tag its field carries: 32 is `LastShares` in 4.2 and `LastQty` in a
    // newest one, and the column is the dictionary's one `lastqty` in both.
    // The row opens as every generated schema does - the element's facts,
    // the event's, the market's, the operation's - and the message's own
    // columns follow: its clocks, then which message and session, the
    // header first. Each column is found by its name rather than by an
    // offset, so a band that gains one does not move this assertion.
    let at = |name: &str| {
        schema
            .index_of(name)
            .unwrap_or_else(|| panic!("a {name} column"))
    };
    for pair in [
        "curruuid",
        "srcuuids",
        "currunix",
        "creaunix",
        "prevunix",
        "snapunix",
        "state",
        "marketdatakind",
        "price",
        "partyids",
        "sendingtime",
        "beginstring",
    ]
    .windows(2)
    {
        assert!(at(pair[0]) < at(pair[1]), "{pair:?} in {names:?}");
    }
    let header = schema.index_of("beginstring").expect("the header opens");
    assert_eq!(
        &names[header..header + 4],
        ["beginstring", "msgtype", "msgseqnum", "sendercompid"]
    );
    assert_eq!(schema.index_of("msgtype"), Some(header + 1));
    assert_eq!(column_of(&schema, 35), header + 1);
    assert_eq!(names[column_of(&schema, 32)], "lastqty");
    assert_eq!(names.last(), Some(&"fixentries"));

    // The dictionary's own typing reaches the column, so a currency column is
    // the packed currency and a side is the packed side.
    let fields = schema.fields();
    let typed = |tag: i32| fields[column_of(&schema, tag)].dtype().clone();
    assert_eq!(typed(15), DataType::Ccy, "Currency(15)");
    assert_eq!(typed(120), DataType::Ccy, "SettlCurrency(120)");
    assert_eq!(typed(54), DataType::Side, "Side(54)");
    assert_eq!(typed(35), DataType::utf8(), "MsgType(35)");
    assert!(
        matches!(typed(60), DataType::DateTime64 { .. }),
        "TransactTime"
    );
    // Every price and quantity is FIX's own field, exact at the one width
    // this crate keeps a number at.
    assert_eq!(typed(44), DataType::decimal128(38, 18).unwrap());
    assert_eq!(typed(38), typed(44));
    assert_eq!(typed(53), typed(44));
    assert_eq!(
        typed(yggdryl::PREVUNIX_TAG_NAME.0),
        typed(yggdryl::CURRUNIX_TAG_NAME.0)
    );
    assert_eq!(typed(yggdryl::PREVUUID_TAG_NAME.0), DataType::Uuid);
    assert_eq!(typed(yggdryl::CURRUUID_TAG_NAME.0), DataType::Uuid);
    assert_eq!(typed(yggdryl::CURRHASHCODE_TAG_NAME.0), DataType::UInt64);

    // Crate-owned columns follow the same contract as FIX's: the stable
    // identity is the folded name, while renderers receive the readable
    // spelling the field keeps as its display.
    for (tag, display) in [
        (yggdryl::CURRUNIX_TAG_NAME.0, "Current Time"),
        (yggdryl::MSGCTXID_TAG_NAME.0, "Message Context ID"),
        (yggdryl::MSGPLUGINID_TAG_NAME.0, "Message Plugin ID"),
        (yggdryl::MSGSESSIONID_TAG_NAME.0, "Message Session ID"),
        (
            yggdryl::MSGSESSEVENTID_TAG_NAME.0,
            "Message Session Event ID",
        ),
        (yggdryl::CURRHASHCODE_TAG_NAME.0, "Current Hash Code"),
        (yggdryl::CROSSHASHCODE_TAG_NAME.0, "Cross Hash Code"),
        (yggdryl::CROSSCODE_TAG_NAME.0, "Cross Code"),
        (yggdryl::PREVUNIX_TAG_NAME.0, "Previous Time"),
        (yggdryl::PREVUUID_TAG_NAME.0, "Previous UUID"),
        (yggdryl::STATE_TAG_NAME.0, "State"),
        (yggdryl::EXPRUNIX_TAG_NAME.0, "Expiry Time"),
    ] {
        let field = &fields[column_of(&schema, tag)];
        assert_eq!(field.display(), Some(display), "tag {tag}");
    }

    // The replay bundle, the place at the instant - zero for the first, so
    // never absent - and BeginString are required, and nothing else:
    // `snapunix` is only what a snapshot stamps, and the state a message
    // reached, stated on every row a message writes, has no neutral member
    // to fill an empty cell with, so both are nullable like every other
    // column a message may not state.
    let required: Vec<&str> = fields
        .iter()
        .filter(|field| !field.is_nullable())
        .map(Field::name)
        .collect();
    assert_eq!(
        required,
        [
            "curruuid",
            "crossuuid",
            "crosscode",
            "currhashcode",
            "crosshashcode",
            "currunix",
            "creaunix",
            "seqnum",
            "beginstring",
        ]
    );
}

/// A settled identity is a value a table carries and gives back.
///
/// The hash codes are `uint64` and the identities are `uuid`, which Arrow
/// stores as sixteen bytes under `arrow.uuid`: what a row holds is what the
/// message settled, and a writer, a reader and a second row all give it back.
#[test]
fn identity_columns_keep_their_values_through_rows_and_record_writers() {
    use yggdryl::holder::Buffer;
    use yggdryl::ipc::{Ipc, IpcOptions};
    use yggdryl::media::RecordOptions;
    use yggdryl::{
        CROSSHASHCODE_TAG_NAME, CURRHASHCODE_TAG_NAME, FixMsg, IOMedia, PREVUUID_TAG_NAME,
    };

    let (registry, codec) = reader();
    let codec = codec.with_separator(b'|');
    let wire = b"8=FIX.4.4|35=D|11=UUID-ORDER-1|55=AAPL|10=0|";
    let mut message = codec.sole_line(wire).unwrap();
    let digest = message.digest();
    let previous = DataType::uuid()
        .scalar(Scalar::from(
            &[
                0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x86, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
                0xee, 0xff,
            ][..],
        ))
        .unwrap();
    let previous_clock = Scalar::from_datetime(
        1_700_000_000_000_000_123,
        yggdryl::TimeUnit::Nanosecond,
        yggdryl::Timezone::UTC,
    )
    .unwrap();
    message
        .set_many([
            (PREVUUID_TAG_NAME.0, previous.clone()),
            (yggdryl::PREVUNIX_TAG_NAME.0, previous_clock.clone()),
        ])
        .unwrap();
    // The identity is the message's own fact, so writing one changes neither
    // the wire it re-emits nor the code that wire digests to.
    assert_eq!(message.digest(), digest);
    assert_eq!(
        message.into_bytes(b'|'),
        b"8=FIX.4.4|35=D|11=UUID-ORDER-1|55=AAPL|59=0|10=0|"
    );

    let schema = fix_schema(&registry, "fix").unwrap();
    let row = message.into_row(&schema).unwrap();
    for (tag, _) in [CURRHASHCODE_TAG_NAME, CROSSHASHCODE_TAG_NAME] {
        assert!(
            at(&row, &schema, tag).as_u64().is_some(),
            "tag {tag} is a settled code"
        );
    }
    assert_eq!(
        at(&row, &schema, yggdryl::PREVUNIX_TAG_NAME.0),
        &previous_clock
    );
    let field = &schema.fields()[column_of(&schema, PREVUUID_TAG_NAME.0)];
    assert_eq!(field.name(), PREVUUID_TAG_NAME.1);
    assert_eq!(field.dtype(), &DataType::uuid());
    assert_eq!(at(&row, &schema, PREVUUID_TAG_NAME.0), &previous);

    let restored = FixMsg::from_row(Arc::clone(&registry), &schema, &row).unwrap();
    assert_eq!(restored.into_row(&schema).unwrap(), row);
    for tag in [11, 55] {
        assert_eq!(
            restored.by_tag(tag).unwrap(),
            message.by_tag(tag).unwrap(),
            "tag {tag}"
        );
    }
    // A complete row carries the supplied event identity rather than deriving
    // another identity from the reconstructed content order.
    assert_eq!(restored.get_curruuid(), message.get_curruuid());
    assert_eq!(restored.get_crossuuid(), message.get_crossuuid());
    assert_eq!(restored.get_currhashcode(), message.get_currhashcode());
    assert_eq!(restored.get_crosshashcode(), message.get_crosshashcode());
    assert_eq!(restored.get_prevuuid(), message.get_prevuuid());
    assert_eq!(restored.get_prevunix(), message.get_prevunix());

    let identity = (
        message.get_curruuid(),
        message.get_crossuuid(),
        message.get_prevuuid(),
        message.get_prevunix(),
    );
    let outgoing = codec.arrow_reader(schema.clone(), [Ok(message)]).unwrap();
    let arrow_schema = outgoing.schema();
    assert_eq!(
        arrow_schema
            .field_with_name(yggdryl::PREVUNIX_TAG_NAME.1)
            .unwrap()
            .data_type(),
        &arrow_schema::DataType::Timestamp(arrow_schema::TimeUnit::Nanosecond, Some("UTC".into()))
    );
    // A code is a plain `uint64`; an identity is sixteen bytes under the
    // canonical Arrow extension name for one, which is what a lake engine
    // reads it back as.
    for (_, name) in [CURRHASHCODE_TAG_NAME, CROSSHASHCODE_TAG_NAME] {
        let field = arrow_schema.field_with_name(name).unwrap();
        assert_eq!(field.data_type(), &arrow_schema::DataType::UInt64);
        assert!(!field.metadata().contains_key("ARROW:extension:name"));
    }
    for (_, name) in [PREVUUID_TAG_NAME, yggdryl::CURRUUID_TAG_NAME] {
        let field = arrow_schema.field_with_name(name).unwrap();
        assert_eq!(
            field.data_type(),
            &arrow_schema::DataType::FixedSizeBinary(16)
        );
        assert_eq!(
            field
                .metadata()
                .get("ARROW:extension:name")
                .map(String::as_str),
            Some("arrow.uuid")
        );
    }

    let options: RecordOptions = IpcOptions::default().into();
    let mut stored = Ipc::new(Buffer::new());
    stored.overwrite_arrow_reader(outgoing, &options).unwrap();
    let incoming = stored.read_arrow_reader(&options).unwrap();
    assert_eq!(incoming.schema(), arrow_schema);
    let mut messages = codec.messages(incoming);
    let restored = messages.next().unwrap().unwrap();
    assert!(messages.next().is_none());
    assert_eq!(restored.into_row(&schema).unwrap(), row);
    assert_eq!(restored.get_curruuid(), identity.0);
    assert_eq!(restored.get_crossuuid(), identity.1);
    assert_eq!(restored.get_prevuuid(), identity.2);
    assert_eq!(restored.get_prevunix(), identity.3);

    let mut encoded = Vec::new();
    assert_eq!(
        codec
            .write_arrow_reader(stored.read_arrow_reader(&options).unwrap(), &mut encoded)
            .unwrap(),
        1
    );
    // The body follows the row's columns: `TimeInForce(59)` is the
    // operation's `timeinforce`, so it leads the instrument band.
    assert_eq!(
        encoded,
        b"8=FIX.4.4|35=D|11=UUID-ORDER-1|59=0|55=AAPL|10=0|\n"
    );
}

#[test]
fn a_row_read_against_one_schema_then_another_answers_each_schema_s_own_columns() {
    let (registry, reader) = reader();
    // The tags a schema's columns answer for are remembered from one row to
    // the next, and the memory is the schema's own: a narrower schema, a
    // rebuilt one and the first again each fill their own columns.
    let wide = fix_schema(&registry, "fix").unwrap();
    let mut symbol = DataType::utf8().nullable_field("symbol");
    symbol.as_fix_mut().set_tag(55).unwrap();
    let mut side = DataType::utf8().nullable_field("side");
    side.as_fix_mut().set_tag(54).unwrap();
    let narrow = fix_schema(&FixRegistry::from_fields([side, symbol]).unwrap(), "fix").unwrap();
    let rebuilt = fix_schema(&registry, "fix").unwrap();
    assert_eq!(rebuilt, wide, "one dictionary, one schema");

    let order = reader
        .sole_line(b"8=FIX.4.4|35=D|11=ORDER-1|55=AAPL|54=1|10=0|")
        .unwrap();
    for schema in [&wide, &narrow, &rebuilt, &wide, &narrow] {
        let row = order.into_row(schema).unwrap();
        assert_eq!(
            row.as_sequence().map(<[Scalar]>::len),
            Some(schema.fields().len()),
            "one value per column"
        );
        assert_eq!(at(&row, schema, 55).as_str(), Some("AAPL"));
        assert_eq!(at(&row, schema, 54).as_str(), Some("BUYS"));
    }
    let wide_row = order.into_row(&wide).unwrap();
    assert_eq!(at(&wide_row, &wide, 11).as_str(), Some("ORDER-1"));
    assert!(
        fix_column_of(&narrow, 11).is_none(),
        "the narrow schema has no column for the order id"
    );
}

#[test]
fn a_row_fills_every_column_by_tag_and_never_shifts() {
    let (registry, reader) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();

    let order = reader
        .sole_line(b"8=FIX.4.4|35=D|11=ORDER-1|55=AAPL|54=1|44=12.5|38=100|15=USD|60=20240102-10:15:30.000|10=0|")
        .unwrap();
    let order_px = order.get_price();
    let row = order.into_row(&schema).unwrap();
    assert_eq!(at(&row, &schema, 35).as_str(), Some("D"));
    assert_eq!(at(&row, &schema, 11).as_str(), Some("ORDER-1"));
    assert_eq!(at(&row, &schema, 55).as_str(), Some("AAPL"));
    assert_eq!(at(&row, &schema, 15).as_str(), Some("USD"));
    // `Price(44)` is a column of its own, exact, and what the message is
    // *about* is what the trait reads off it.
    assert_eq!(
        at(&row, &schema, 44).as_decimal(),
        Some((yggdryl::i256::from_i128(12_500_000_000_000_000_000), 18))
    );
    assert_eq!(order_px.map(|px| px.to_string()).as_deref(), Some("12.5"));

    // A message that carried almost nothing has the same columns in the same
    // places, which is what makes two rows of one capture comparable.
    let bare = reader.sole_line(b"8=FIX.4.4|35=D|10=0|").unwrap();
    let thin = bare.into_row(&schema).unwrap();
    assert_eq!(
        thin.as_sequence().map(<[Scalar]>::len),
        row.as_sequence().map(<[Scalar]>::len),
    );
    assert_eq!(at(&thin, &schema, 35).as_str(), Some("D"));
    assert!(at(&thin, &schema, 55).is_null(), "no symbol, not a shift");
}

#[test]
fn projections_derive_facets_but_keep_the_hard_identity_bundle() {
    let (registry, reader) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();
    let order = reader
        .sole_line(b"8=FIX.4.4|35=D|11=A|55=AAPL|207=XNAS|60=20240102-10:15:30.000|10=0|")
        .unwrap();
    let row = order.into_row(&schema).unwrap();

    // The code the content digests to, a sixty-four-bit number the row holds.
    assert!(!at(&row, &schema, yggdryl::CURRHASHCODE_TAG_NAME.0).is_null());

    // The clock the row is cut by - how a layout is cut from it is the
    // target's, not a column of this crate's.
    assert!(!at(&row, &schema, yggdryl::CURRUNIX_TAG_NAME.0).is_null());

    // The version it was read at, which the header holds and the row states.
    assert_eq!(at(&row, &schema, 8).as_str(), Some("FIX.4.4"));

    // Hard identities and clocks are stored mirrors, never invented arrivals.
    assert!(order.get_by_tag(65_000).is_none());
    assert!(order.get_by_tag(yggdryl::CURRHASHCODE_TAG_NAME.0).is_some());
    assert!(order.get_by_tag(yggdryl::CURRUNIX_TAG_NAME.0).is_some());
    assert!(
        order
            .entries()
            .iter()
            .all(|entry| entry.tag() != yggdryl::CURRUNIX_TAG_NAME.0)
    );
}

#[test]
fn a_bid_is_a_column_only_where_the_message_states_it() {
    let (registry, reader) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();

    // A buy order at a price states no bid: nothing derives one, so the
    // row's `BidPx(132)` and `OfferPx(133)` stay null.
    let buy = reader
        .sole_line(b"8=FIX.4.4|35=D|11=A|54=1|44=12.5|38=100|10=0|")
        .unwrap();
    let row = buy.into_row(&schema).unwrap();
    assert!(at(&row, &schema, 132).is_null(), "no bid was stated");
    assert!(at(&row, &schema, 133).is_null());

    // A bid the message did state is a column like any other.
    let stated = reader
        .sole_line(b"8=FIX.4.4|35=D|11=A|54=1|44=12.5|132=99.0|10=0|")
        .unwrap();
    let row = stated.into_row(&schema).unwrap();
    assert_eq!(at(&row, &schema, 132), &super::decimal("99"));
}

#[test]
fn the_row_keeps_unrepresented_content_and_projects_explained_values() {
    let (registry, reader) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();
    let message = reader
        .sole_line(b"8=FIX.4.4|35=D|11=A|107=HOLCIM N|9999=x|VenueOwnThing=y|10=0|")
        .unwrap();
    let row = message.into_row(&schema).unwrap();

    // Only content no column explained remains in the residual, under its
    // field's `tag:name`. The frame, order identifier and derived
    // `TimeInForce` each have a column; `SecurityDesc(107)` has none.
    assert_eq!(
        residual(&row, &schema),
        [("107:securitydesc".to_owned(), "HOLCIM N".to_owned())]
    );
    // A key no dictionary resolves is no field: the metadata states it under
    // its own spelling, folded as every name is, with the text it arrived as.
    assert_eq!(
        metadata(&row, &schema),
        [
            ("9999".to_owned(), "x".to_owned()),
            ("venueownthing".to_owned(), "y".to_owned())
        ]
    );

    // A dictionary value the schema represents is read from its named
    // column, and rebuilding preserves it beside the residual; an unmapped
    // key comes back as the tag-zero entry the parse held it as, so the
    // message read back re-emits it and its metadata keeps bridge keys only.
    let restored = yggdryl::FixMsg::from_row(Arc::clone(&registry), &schema, &row).unwrap();
    assert_eq!(restored.by_tag(59).unwrap(), message.by_tag(59).unwrap());
    assert_eq!(restored.by_tag(107).unwrap(), message.by_tag(107).unwrap());
    assert!(restored.metadata().is_empty());
    let unmapped = |held: &yggdryl::FixMsg| {
        held.entries()
            .iter()
            .filter(|entry| entry.tag() == 0)
            .map(|entry| (entry.name().to_owned(), entry.value().map(str::to_owned)))
            .collect::<Vec<_>>()
    };
    assert_eq!(unmapped(&restored), unmapped(&message));
    assert_eq!(restored.digest(), message.digest());
    assert_eq!(restored.into_row(&schema).unwrap(), row);
}

/// A key no dictionary resolved whose name spells an identifier the message
/// holds with its value - a bridge's `OMSINSTRUMENTID`, its dotted
/// `ULLINK.INSTRUMENTID`, a `#`-kept `#ORDERID`, a `VENUEUSERID` - is
/// captured: it leaves the row's `metadata` cell, which then holds only what
/// nothing resolved, and rides `fixentries` under `0:<key>` as it arrived, so
/// the row read back restates the wire, the message's own metadata, its
/// tag-zero entries, its digest and its identity. A second value under a
/// captured key, which the set refused, stays in `metadata` as it arrived.
#[test]
fn a_captured_key_leaves_the_metadata_cell_rides_the_residual_and_comes_back() {
    use yggdryl::graph::Operation;

    let (registry, reader) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();
    // The unmapped keys stand in the order a row rebuilds them - the
    // captured ones by key, then the metadata's by name - so the
    // order-sensitive digest coincides beside the name-sorted identity.
    let message = reader
        .sole_line(
            b"8=FIX.4.4|35=D|11=A|##ORDERID=345|OMSINSTRUMENTID=dbi;CH0012214059_XSWX_CHF|\
              ULLINK.INSTRUMENTID=dbi;CH0012214059_XSWX_CHF|VENUEUSERID=t1|9999=x|\
              ULLINKINSTRUMENTID=dbi;ZZ|10=0|",
        )
        .unwrap();
    let row = message.into_row(&schema).unwrap();
    assert_eq!(
        metadata(&row, &schema),
        [
            ("9999".to_owned(), "x".to_owned()),
            ("ullinkinstrumentid".to_owned(), "dbi;ZZ".to_owned()),
        ]
    );
    let captured: Vec<(String, String)> = residual(&row, &schema)
        .into_iter()
        .filter(|(key, _)| key.starts_with("0:"))
        .collect();
    assert_eq!(
        captured,
        [
            ("0:#orderid".to_owned(), "345".to_owned()),
            (
                "0:omsinstrumentid".to_owned(),
                "dbi;CH0012214059_XSWX_CHF".to_owned()
            ),
            (
                "0:ullink.instrumentid".to_owned(),
                "dbi;CH0012214059_XSWX_CHF".to_owned()
            ),
            ("0:venueuserid".to_owned(), "t1".to_owned()),
        ]
    );

    let restored = yggdryl::FixMsg::from_row(Arc::clone(&registry), &schema, &row).unwrap();
    assert_eq!(restored.metadata(), message.metadata());
    let unmapped = |held: &yggdryl::FixMsg| {
        held.entries()
            .iter()
            .filter(|entry| entry.tag() == 0)
            .map(|entry| (entry.name().to_owned(), entry.value().map(str::to_owned)))
            .collect::<Vec<_>>()
    };
    assert_eq!(unmapped(&restored), unmapped(&message));
    let tokens = |held: &yggdryl::FixMsg| {
        let mut tokens: Vec<String> = String::from_utf8(held.into_bytes(b'|'))
            .unwrap()
            .split('|')
            .map(str::to_owned)
            .collect();
        tokens.sort();
        tokens
    };
    assert_eq!(tokens(&restored), tokens(&message));
    assert_eq!(restored.digest(), message.digest());
    assert_eq!(restored.get_currhashcode(), message.get_currhashcode());
    assert_eq!(restored.get_curruuid(), message.get_curruuid());
    assert_eq!(restored.get_securityids(), message.get_securityids());
    assert_eq!(restored.get_identifiers(), message.get_identifiers());
    assert_eq!(restored.get_partyids(), message.get_partyids());
    assert_eq!(restored.anomalies().len(), message.anomalies().len());
    assert_eq!(restored.into_row(&schema).unwrap(), row);

    // A row with a `metadata` column and no `fixentries` keeps a captured
    // key in `metadata`: nothing a row holds is lost for want of a column.
    let narrow = Field::new(
        "fix",
        DataType::from(
            StructType::from_fields(
                ["msgtype", "clordid", "metadata"]
                    .map(|name| schema.fields()[schema.index_of(name).expect(name)].clone())
                    .to_vec(),
            )
            .unwrap(),
        ),
        false,
    );
    let row = message.into_row(&narrow).unwrap();
    let kept = metadata(&row, &narrow);
    for key in [
        "#orderid",
        "omsinstrumentid",
        "ullink.instrumentid",
        "venueuserid",
        "9999",
        "ullinkinstrumentid",
    ] {
        assert!(
            kept.iter().any(|(held, _)| held == key),
            "{key} in {kept:?}"
        );
    }
}

/// An unmapped key stated twice is one metadata key holding the JSON array
/// of its values in arrival order, and a text opening the way JSON does its
/// JSON string; the row read back holds both occurrences again.
#[test]
fn a_repeated_unmapped_key_is_one_metadata_array_and_reads_back_whole() {
    let (registry, reader) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();
    let message = reader
        .sole_line(b"8=FIX.4.4|35=D|11=A|VenueOwnThing=b|VenueOwnThing=a|VenueList=[1]|10=0|")
        .unwrap();
    let row = message.into_row(&schema).unwrap();
    assert_eq!(
        metadata(&row, &schema),
        [
            ("venuelist".to_owned(), "\"[1]\"".to_owned()),
            ("venueownthing".to_owned(), "[\"b\",\"a\"]".to_owned())
        ]
    );
    let restored = yggdryl::FixMsg::from_row(Arc::clone(&registry), &schema, &row).unwrap();
    let wire = String::from_utf8(restored.into_bytes(b'|')).unwrap();
    assert!(
        wire.contains("venueownthing=b|venueownthing=a|") && wire.contains("venuelist=[1]|"),
        "{wire}"
    );
    assert_eq!(restored.into_row(&schema).unwrap(), row);
}

/// A residual value is text where a scalar states it and the JSON of what
/// a group holds - its occurrences an array, each an object keyed
/// `tag:name` - and a scalar whose text opens the way JSON does is its JSON
/// string, so every value reads back as what it was.
#[test]
fn a_residual_group_is_its_json_and_a_json_looking_scalar_its_string() {
    let (registry, reader) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();
    let message = reader
        .sole_line(b"8=FIX.4.4|35=AE|571=T1|107=[1]|552=2|54=1|453=1|448=P1|452=1|54=2|10=0|")
        .unwrap();
    let row = message.into_row(&schema).unwrap();
    let held = residual(&row, &schema);
    let text = |key: &str| {
        held.iter()
            .find(|(held, _)| held == key)
            .map(|(_, value)| value.as_str())
            .unwrap_or_else(|| panic!("{key} in {held:?}"))
    };
    assert_eq!(text("107:securitydesc"), r#""[1]""#);
    let sides = held
        .iter()
        .find(|(key, _)| key.starts_with("552:"))
        .map(|(_, value)| value.as_str())
        .unwrap_or_else(|| panic!("the sides in {held:?}"));
    assert!(sides.starts_with("[{"), "{sides}");
    assert!(sides.contains(r#""54:side":"1""#), "{sides}");
    assert!(sides.contains(r#""54:side":"2""#), "{sides}");
    assert!(
        sides.contains(r#"[{"448:partyid":"P1","452:partyrole":"1"}]"#),
        "{sides}"
    );

    let restored = yggdryl::FixMsg::from_row(Arc::clone(&registry), &schema, &row).unwrap();
    assert_eq!(restored.by_tag(107).unwrap(), message.by_tag(107).unwrap());
    assert_eq!(restored.into_row(&schema).unwrap(), row);
}

/// A key the record holds is a resolved field's `tag:name`, or a key no
/// dictionary resolved under tag zero - which the row read back restores as
/// the tag-zero entry a parse holds it as, whatever its name; a key that is
/// neither is refused where it is spelled.
#[test]
fn a_residual_key_that_is_no_resolved_field_is_refused_by_name() {
    let (registry, reader) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();
    let row = reader
        .sole_line(b"8=FIX.4.4|35=D|11=A|10=0|")
        .unwrap()
        .into_row(&schema)
        .unwrap();
    let at = schema.index_of("fixentries").unwrap();
    let mut cells = row.as_sequence().unwrap().to_vec();
    cells[at] = Scalar::from_mapping([(Scalar::from("0:venueownthing"), Scalar::from("y"))])
        .expect("a map");
    let restored = yggdryl::FixMsg::from_row(
        Arc::clone(&registry),
        &schema,
        &Scalar::from_sequence(cells),
    )
    .expect("a tag-zero key is a key no dictionary resolved");
    assert_eq!(
        restored
            .entries()
            .iter()
            .find(|entry| entry.tag() == 0)
            .map(|entry| (entry.name().to_owned(), entry.value().map(str::to_owned))),
        Some(("venueownthing".to_owned(), Some("y".to_owned())))
    );
    for key in ["venueownthing", "x:symbol", "55:", "0:"] {
        let mut cells = row.as_sequence().unwrap().to_vec();
        cells[at] = Scalar::from_mapping([(Scalar::from(key), Scalar::from("y"))]).expect("a map");
        let error = yggdryl::FixMsg::from_row(
            Arc::clone(&registry),
            &schema,
            &Scalar::from_sequence(cells),
        )
        .expect_err("a key that is no resolved field");
        assert!(error.to_string().contains("fixentries"), "{key}: {error}");
    }
}

/// A residual key's tag and name are one field: the name a field is spelled
/// by beside its own tag, a group's name beside the tag of the counter
/// heading it, or a name no field has beside the tag of the child held under
/// it. A tag beside another field's name is refused naming the key, never
/// read as the field the name reaches while the tag's own column is dropped.
#[test]
fn a_residual_key_whose_tag_and_name_are_not_one_field_is_refused_by_name() {
    let (registry, reader) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();
    let row = reader
        .sole_line(b"8=FIX.4.4|35=D|11=C-1|55=AAPL|54=1|38=1|40=2|10=0|")
        .unwrap()
        .into_row(&schema)
        .unwrap();
    let at = schema.index_of("fixentries").unwrap();
    let with_entry = |key: &str, value: &str| {
        let mut cells = row.as_sequence().unwrap().to_vec();
        cells[at] =
            Scalar::from_mapping([(Scalar::from(key), Scalar::from(value))]).expect("a map");
        yggdryl::FixMsg::from_row(
            Arc::clone(&registry),
            &schema,
            &Scalar::from_sequence(cells),
        )
    };
    // `55` is Symbol's tag and `securityid` SecurityID(48)'s name, and the
    // other way round.
    for key in ["55:securityid", "48:symbol"] {
        let error = with_entry(key, "US0378331005").expect_err("two fields");
        assert!(
            error
                .to_string()
                .contains(&format!("$.fixentries[\"{key}\"]")),
            "{key}: {error}"
        );
    }
    // A field under its own tag and name, a group under its counter's tag.
    for (key, value, stated) in [
        ("21:handlinst", "1", "|21=1|"),
        ("9999:custom", "x", "|9999=x|"),
        (
            "453:parties",
            r#"[{"448:partyid":"P1","452:partyrole":"1"}]"#,
            "|453=1|448=P1|452=1|",
        ),
    ] {
        let wire = with_entry(key, value)
            .expect("one field")
            .into_text('|')
            .unwrap();
        assert!(
            wire.contains(stated) && wire.contains("|55=AAPL|"),
            "{wire}"
        );
    }
}
/// The identifier columns are sorted maps from the key `src:type` to the
/// identifier's `struct<src, type, value>`, one per set a market element
/// states: its `securityids`, its `identifiers` and its `partyids`, the last
/// of the columns every generated schema opens with. A message's cell is its
/// set's own map - each key the one its row spells, the keys in the order
/// they are spelled - and reads back to the set it was.
#[test]
fn the_identifier_columns_are_sorted_maps_from_the_key_to_the_identifier_row() {
    use yggdryl::graph::Operation;
    use yggdryl::{Identifier, Identifiers};

    let (registry, codec) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();
    for name in ["securityids", "identifiers", "partyids"] {
        let column = &schema.fields()[schema.index_of(name).expect(name)];
        assert_eq!(column.dtype(), &Identifiers::dtype(), "{name}");
        assert!(column.is_nullable(), "{name}");
    }
    let message = codec
        .sole_line(
            b"8=FIX.4.4|35=D|11=A1|55=AAPL|48=US0378331005|22=4|54=1|38=1|40=2|1=ACC|\
              OMSUSERID=trader1|ParentOrderID=P1|10=0|",
        )
        .unwrap();
    let row = message.into_row(&schema).unwrap();
    for (name, expected) in [
        ("securityids", message.get_securityids().clone()),
        ("identifiers", message.get_identifiers().clone()),
        ("partyids", message.get_partyids().clone()),
    ] {
        let cell = &row.as_sequence().expect("a row")[schema.index_of(name).unwrap()];
        let entries = cell.as_mapping().expect(name);
        assert!(!entries.is_empty(), "{name} states something");
        assert_eq!(entries.len(), expected.len(), "{name}");
        let mut previous: Option<&str> = None;
        for (key, value) in entries {
            let key = key.as_str().expect("a text key");
            // Each key is an identifier key as it is spelled - a base key as
            // its type alone - over its text value, in key order.
            let read: IdKey = key.parse().expect("an identifier key");
            assert_eq!(read.to_string(), key, "{name}");
            assert!(value.as_str().is_some(), "{name}: {key} holds text");
            assert!(
                previous.is_none_or(|before| before < key),
                "{name}: {previous:?} {key}"
            );
            previous = Some(key);
        }
        assert_eq!(
            &Identifiers::from_scalar(cell).expect(name),
            &expected,
            "{name}"
        );
    }
    // What the fields do not state of its sets and a row does stands: the
    // message a row is read back as holds the identifier its map states.
    let at = schema.index_of("identifiers").unwrap();
    let mut cells = row.as_sequence().unwrap().to_vec();
    let mut held = Identifiers::from_scalar(&cells[at]).unwrap();
    held.insert(Identifier::new(IdKey::base("foreignid".parse().unwrap()), "F-1").unwrap());
    cells[at] = held.into_scalar();
    let restored = yggdryl::FixMsg::from_row(
        Arc::clone(&registry),
        &schema,
        &Scalar::from_sequence(cells),
    )
    .unwrap();
    assert_eq!(
        restored
            .get_identifiers()
            .get_from(&IdKey::base("foreignid".parse().unwrap())),
        Some("F-1")
    );
    assert_eq!(restored.get_identifiers(), &held);
}

/// A row's identifier map is its own word, so one holding a key that reads
/// as no key, a value its type refuses by shape - eleven characters where an
/// ISIN is twelve; a check digit that does not close is a value of rank 0,
/// not a refusal - or two values under two spellings of one key is refused
/// on its column - by `FixMsg::from_row`, and by `FixCodec::messages`, which
/// excludes the row - never read as the set the fields alone state.
#[test]
fn an_identifier_column_holding_what_no_map_holds_is_refused_on_its_column() {
    let (registry, codec) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();
    let row = codec
        .sole_line(b"8=FIX.4.4|35=D|11=C-1|55=AAPL|48=US0378331005|22=4|54=1|38=1|40=2|10=0|")
        .unwrap()
        .into_row(&schema)
        .unwrap();
    for (name, entries, key, reason) in [
        (
            "securityids",
            &[("isin", "US037833100")][..],
            "isin",
            "expected twelve characters",
        ),
        (
            "identifiers",
            &[("fix:", "C-1")][..],
            "fix:",
            "expected an identifier key src:type or type",
        ),
        (
            "partyids",
            &[("BASE:EXECUTINGFIRM", "B"), ("executingfirm", "A")][..],
            "executingfirm",
            "expected one value under executingfirm",
        ),
    ] {
        let mut cells = row.as_sequence().unwrap().to_vec();
        let Scalar::Map(entries) = Scalar::from_mapping(
            entries
                .iter()
                .map(|(key, value)| (Scalar::from(*key), Scalar::from(*value))),
        )
        .unwrap() else {
            panic!("a map")
        };
        cells[schema.index_of(name).unwrap()] = Scalar::SortedMap(entries);
        let refused_row = Scalar::from_sequence(cells);

        let refused = yggdryl::FixMsg::from_row(Arc::clone(&registry), &schema, &refused_row)
            .expect_err("no map holds it");
        let refused = refused.to_string();
        assert!(refused.contains(&format!("$.{name}['{key}']")), "{refused}");
        assert!(refused.contains(reason), "{refused}");

        let batch = yggdryl::Serie::from_scalars(schema.clone(), [row.clone(), refused_row])
            .unwrap()
            .into_arrow_batch()
            .unwrap();
        let read = codec
            .messages(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
            .collect::<yggdryl::Result<Vec<_>>>()
            .expect("an excluded row is no error");
        assert_eq!(read.len(), 1, "{name}: the refused row is excluded");
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
            .and_then(Scalar::as_str)
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
        assert_eq!(read.id(), id, "{} changed identity", id.as_str());
    }
}

/// The identity columns cross a lake in the storage their own width asks for.
///
/// Two of the three are XXH3-64 digests, so they are `uint64`, which Iceberg
/// has no type for at all - the spec's integers are signed. The widening the
/// refusal names is the door: a `u64` is at most twenty digits, so
/// `decimal(20, 0)` holds every one of them losslessly. Only the previous
/// message's UUID is sixteen bytes, and it crosses as the spec's `uuid`,
/// which Arrow reads back as `fixed_binary(16)`.
///
/// The round trip writes stamped messages through the widened schema, reads
/// them back, and compares each value to what the projected row stated.
#[cfg(feature = "iceberg")]
#[test]
fn the_identity_columns_cross_an_iceberg_table_in_the_storage_their_width_asks_for() {
    use yggdryl::iceberg::{
        FormatVersion, IcebergTable, PartitionSpec, PrimitiveType, assign_field_ids,
    };
    use yggdryl::local::LocalFolder;
    use yggdryl::{CROSSHASHCODE_TAG_NAME, CURRHASHCODE_TAG_NAME, PREVUUID_TAG_NAME, Scheme};

    let (registry, codec) = reader();
    let codec = codec.with_separator(b'|');
    // Two messages of one order, so the chain stamps `msgphash` on both and
    // `prevmsghash` on the second.
    let lines: [&[u8]; 2] = [
        b"8=FIX.4.4|35=D|11=LAKE-1|55=AAPL|207=XNAS|15=USD|54=1|38=100|52=20260102-10:15:30.000|10=0|",
        b"8=FIX.4.4|35=8|11=LAKE-1|37=O-1|17=E-1|39=2|150=F|55=AAPL|207=XNAS|15=USD|14=100|52=20260102-10:15:31.000|10=0|",
    ];
    let messages: Vec<_> = lines
        .iter()
        .map(|line| Ok(codec.sole_line(line).unwrap()))
        .collect();
    let fixed = fix_schema(&registry, "fix").unwrap();
    let stamped: Vec<_> = codec
        .lifecycle(messages)
        .map(|held| held.unwrap())
        .collect();

    // The digests are unsigned, so the fixed schema is refused before a table
    // exists, and the refusal names both the type and the way out.
    let refused =
        PrimitiveType::from_dtype(fixed.get_field(CURRHASHCODE_TAG_NAME.1).unwrap().dtype())
            .map(|held| held.to_string())
            .unwrap_err()
            .to_string();
    assert!(refused.contains("uint64"), "{refused}");
    assert!(refused.contains("into_scheme_compat"), "{refused}");

    // Read off the projected row, because the fixed schema is what the
    // messages state and the table's business is to carry those values back:
    // `msghash` digests the row it lands in (see
    // `message.md#clocks-and-identity`).
    let digests = [CURRHASHCODE_TAG_NAME, CROSSHASHCODE_TAG_NAME];
    let expected_digests: Vec<Vec<Option<u64>>> = stamped
        .iter()
        .map(|held| {
            let row = held.into_row(&fixed).unwrap();
            digests
                .iter()
                .map(|(tag, _)| at(&row, &fixed, *tag).as_u64())
                .collect()
        })
        .collect();
    let expected_uuids: Vec<Option<[u8; 16]>> = stamped
        .iter()
        .map(|held| {
            let row = held.into_row(&fixed).unwrap();
            match at(&row, &fixed, PREVUUID_TAG_NAME.0) {
                Scalar::Uuid(uuid) => Some(uuid.into_bytes()),
                Scalar::Null => None,
                other => panic!("a uuid or nothing, got {other:?}"),
            }
        })
        .collect();
    assert!(
        expected_digests[1].iter().all(Option::is_some),
        "the second message states both digests"
    );
    assert!(
        expected_uuids[1].is_some(),
        "the second message follows the first, so it names its uuid"
    );

    let mut schema = fixed
        .clone()
        .into_scheme_compat(&Scheme::ICEBERG)
        .expect("the widening the refusal names");
    assign_field_ids(&mut schema, 1).unwrap();
    for (_, name) in digests {
        assert_eq!(
            PrimitiveType::from_dtype(schema.get_field(name).unwrap().dtype())
                .unwrap()
                .to_string(),
            "decimal(20, 0)",
            "{name}"
        );
    }
    assert_eq!(
        PrimitiveType::from_dtype(schema.get_field(PREVUUID_TAG_NAME.1).unwrap().dtype())
            .unwrap()
            .to_string(),
        "uuid",
    );

    let path = LocalFolder::temporary()
        .unwrap()
        .path()
        .unwrap()
        .join(format!("yggdryl-fix-identity-lake-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    // Unpartitioned: what is pinned here is the identity columns' storage.
    // How a layout is cut is the target's - an Iceberg table takes an `hour`
    // transform over `updatedat` - and no longer a column of this crate's.
    let mut table = IcebergTable::create(
        LocalFolder::new(&path).unwrap(),
        FormatVersion::V2,
        schema.clone(),
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    let reader = codec
        .arrow_reader(schema.clone(), stamped.into_iter().map(Ok))
        .unwrap();
    table.commit_append(reader).unwrap();

    let read = table.scan(None).unwrap();
    for (_, name) in digests {
        assert_eq!(
            read.schema().field_with_name(name).unwrap().data_type(),
            &arrow_schema::DataType::Decimal128(20, 0),
            "{name}"
        );
    }
    assert_eq!(
        read.schema()
            .field_with_name(PREVUUID_TAG_NAME.1)
            .unwrap()
            .data_type(),
        &arrow_schema::DataType::FixedSizeBinary(16),
    );

    let mut digest_rows: Vec<Vec<Option<u64>>> = Vec::new();
    let mut uuid_rows: Vec<Option<[u8; 16]>> = Vec::new();
    for batch in read {
        let batch = batch.unwrap();
        let columns: Vec<&arrow_array::Decimal128Array> = digests
            .iter()
            .map(|(_, name)| {
                batch
                    .column_by_name(name)
                    .unwrap()
                    .as_any()
                    .downcast_ref()
                    .expect("twenty digits, read back as they were written")
            })
            .collect();
        let uuids: &arrow_array::FixedSizeBinaryArray = batch
            .column_by_name(PREVUUID_TAG_NAME.1)
            .unwrap()
            .as_any()
            .downcast_ref()
            .expect("sixteen fixed bytes, read back as they were written");
        for row in 0..batch.num_rows() {
            digest_rows.push(
                columns
                    .iter()
                    .map(|column| {
                        (!arrow_array::Array::is_null(*column, row))
                            .then(|| u64::try_from(column.value(row)).expect("a digest, unchanged"))
                    })
                    .collect(),
            );
            uuid_rows.push(
                (!arrow_array::Array::is_null(uuids, row))
                    .then(|| <[u8; 16]>::try_from(uuids.value(row)).unwrap()),
            );
        }
    }
    assert_eq!(
        digest_rows, expected_digests,
        "the digests come back exactly as they went"
    );
    assert_eq!(
        uuid_rows, expected_uuids,
        "the uuid comes back exactly as it went"
    );
    let _ = std::fs::remove_dir_all(&path);
}

/// A value no column could read is that column's null, never the row's end.
///
/// A capture is written by systems that disagree with the dictionary about
/// what a field is, and the disagreement arrives one row in ten million: a
/// five-byte MIC under a four-byte column, an ISIN whose check digit does not
/// close. A row that refused would end a run over a day of traffic, and the
/// arrival record already carries what arrived, so nothing is lost by the
/// null and everything is lost by the refusal.
#[test]
fn a_value_a_column_will_not_hold_is_that_columns_null() {
    let (registry, _) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();

    // A message spelling a column's name with a value its datatype cannot
    // hold: five bytes under `SecurityExchange(207)`, which is a four-byte
    // MIC, and ten under `Currency(15)`, which is held to eight.
    let root = StructType::from_fields([
        DataType::utf8().nullable_field("securityexchange"),
        DataType::utf8().nullable_field("currency"),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("NewOrderSingle");
    let message = yggdryl::FixMsg::with_registry(
        Arc::clone(&registry),
        root,
        Scalar::from_struct([
            ("securityexchange", Scalar::from("XLONX")),
            ("currency", Scalar::from("TOOLONGCCY")),
        ])
        .unwrap(),
    )
    .unwrap();

    let row = message.into_row(&schema).unwrap();
    assert!(at(&row, &schema, 207).is_null());
    assert!(at(&row, &schema, 15).is_null());
    // The row is still a row: the columns beside the unreadable ones are
    // filled, and the identity bundle still settled.
    assert!(!at(&row, &schema, yggdryl::CURRHASHCODE_TAG_NAME.0).is_null());
}

/// A market spelled wider than a MIC is no market, and the trait says so.
///
/// The market the traits answer is read off `SecurityExchange`, which FIX
/// types as free text: a venue writing more than four bytes there has not
/// named a MIC, so the trait answers nothing while the column keeps exactly
/// what the venue wrote.
#[test]
fn a_market_wider_than_a_mic_is_read_as_none() {
    let (registry, reader) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();

    let narrow = reader
        .sole_line(b"8=FIX.4.4|35=D|11=A|55=AAPL|54=1|207=XLON|10=0|")
        .unwrap();
    assert_eq!(narrow.get_miccode().map(|held| held.as_str()), Some("XLON"));
    assert_eq!(
        at(&narrow.into_row(&schema).unwrap(), &schema, 207).as_str(),
        Some("XLON"),
    );

    let wide = reader
        .sole_line(b"8=FIX.4.4|35=D|11=A|55=AAPL|54=1|207=XLONX|10=0|")
        .unwrap();
    assert_eq!(wide.get_miccode(), None, "five bytes name no MIC");
    assert!(at(&wide.into_row(&schema).unwrap(), &schema, 207).is_null());
}

/// A group keeps every member that reads, whatever one of them turned out to be.
///
/// A bridge packs an occurrence into one value and a venue writes a member
/// the dictionary does not declare, so a group arrives one member short or
/// one member long often enough to matter. Nulling the whole group over it
/// would throw away the identifiers that did read, and refusing would throw
/// away the capture, so the row keeps what reads and says the rest is absent.
#[test]
fn a_group_keeps_the_members_that_read() {
    let (registry, reader) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();

    // A packed occurrence stating only the identifier: the members it never
    // wrote are null and the identifier it did write is kept.
    let packed = reader
        .sole_line(b"MSGTYPE=D|1907=2|1907[0]=1903=TVT-1|1907[1]=1903=UTI-1")
        .unwrap()
        .into_row(&schema)
        .unwrap();
    let occurrences = group(&packed, &schema);
    let identifiers: Vec<Option<&str>> = occurrences
        .iter()
        .map(|occurrence| occurrence.as_sequence().expect("an occurrence")[0].as_str())
        .collect();
    assert_eq!(identifiers, [Some("TVT-1"), Some("UTI-1")]);
    assert!(occurrences[0].as_sequence().unwrap()[1].is_null());
    // The group's column is found by the counter tag naming it, and its
    // length is the count.
    assert_eq!(
        at(&packed, &schema, 1907)
            .as_sequence()
            .map(<[Scalar]>::len),
        Some(2)
    );

    // And one member the row cannot read costs that member alone: the
    // occurrence around it and the occurrences beside it stay.
    let marked = reader
        .sole_line(b"MSGTYPE=ZMIN|#1907=1|#1907[0]=REGULATORYTRADEID=UTI-1REGULATORYTRADEIDTYPE=0")
        .unwrap()
        .into_row(&schema)
        .unwrap();
    let members = group(&marked, &schema)[0]
        .as_sequence()
        .expect("an occurrence");
    assert_eq!(members[0].as_str(), Some("UTI-1"));
    assert_eq!(members[3].as_i128(), Some(0));
}

/// The regulatory identifiers are one typed Serie column beside their FIX
/// counter, and a projected occurrence leaves no duplicate in the residual.
#[test]
fn regulatory_trade_ids_are_lifted_whole_into_the_fixed_schema() {
    let (registry, reader) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();
    let at_group = schema
        .index_of("regulatorytradeids")
        .expect("the regulatory trade identifiers column");
    let field = &schema.fields()[at_group];
    assert_eq!(field.display(), Some("RegulatoryTradeIDs"));
    assert_eq!(field.as_fix().tag().unwrap(), Some(497_401));
    assert_eq!(field.as_fix().counter().unwrap(), Some(1907));
    assert!(registry.get_field_by_name("regulatorytradeidgrp").is_none());
    let DataType::Serie(item) = field.dtype() else {
        panic!("regulatorytradeids is a Serie, got {}", field.dtype());
    };
    assert_eq!(item.name(), "regulatorytradeidcomponent");
    let member_names: Vec<_> = item.fields().iter().map(Field::name).collect();
    assert_eq!(
        member_names,
        [
            "regulatorytradeid",
            "regulatorytradeidsource",
            "regulatorytradeidevent",
            "regulatorytradeidtype",
            "regulatorylegrefid",
            "regulatorytradeidscope",
        ]
    );
    let member_tags: Vec<_> = item
        .fields()
        .iter()
        .map(|member| member.as_fix().tag().unwrap())
        .collect();
    assert_eq!(
        member_tags,
        [
            Some(1903),
            Some(1905),
            Some(1904),
            Some(1906),
            Some(2411),
            Some(2397)
        ]
    );

    let message = reader
        .sole_line(b"8=FIX.4.4|35=8|17=E1|1907=1|1903=042K38MWK5817YZZ|1906=5|10=0|")
        .unwrap();
    assert!(message.get_by_name("regulatorytradeidgrp").is_none());
    assert!(message.get_by_name("regulatorytradeids").is_some());
    let row = message.into_row(&schema).unwrap();
    assert_eq!(column_of(&schema, 1907), at_group);
    let occurrences = row.as_sequence().unwrap()[at_group]
        .as_sequence()
        .expect("the regulatory trade identifier occurrences");
    assert_eq!(occurrences.len(), 1);
    let members = occurrences[0].as_sequence().expect("one occurrence");
    assert_eq!(members[0].as_str(), Some("042K38MWK5817YZZ"));
    assert!(members[1].is_null());
    assert!(members[2].is_null());
    assert_eq!(members[3].as_i128(), Some(5));
    assert!(members[4].is_null());
    assert!(members[5].is_null());

    assert!(
        residual(&row, &schema)
            .iter()
            .all(|(key, _)| !key.starts_with("1907:")),
        "the projected group is not duplicated in fixentries"
    );
    let rebuilt = yggdryl::FixMsg::from_row(Arc::clone(&registry), &schema, &row).unwrap();
    assert_eq!(rebuilt.into_row(&schema).unwrap(), row);

    // A venue may reuse the standard counter while packing a proprietary
    // occurrence whose members are not RegulatoryTradeIDGrp members. The
    // fixed Serie must not claim that shape: its Serie and scalar counter stay
    // null together, while the complete group remains in the residual.
    let proprietary = reader
        .sole_line(
            b"MSGTYPE=tradecapturereport|NOREGULATORYTRADEIDS=1|NOREGULATORYTRADEIDS[0]=TRADEID=R1\x04\x03TRADEIDTYPE=custom\x04\x03",
        )
        .unwrap();
    let row = proprietary.into_row(&schema).unwrap();
    assert!(at(&row, &schema, 1907).is_null());
    assert!(row.as_sequence().unwrap()[at_group].is_null());
    assert!(
        residual(&row, &schema)
            .iter()
            .any(|(key, _)| key.starts_with("1907:")),
        "the proprietary group remains whole in fixentries"
    );
    let rebuilt = yggdryl::FixMsg::from_row(Arc::clone(&registry), &schema, &row).unwrap();
    assert_eq!(rebuilt.into_row(&schema).unwrap(), row);
}

/// A row read out of Arrow holds its residual record as the map Arrow
/// answers; it rebuilds the message the record does.
#[test]
fn a_row_read_out_of_arrow_rebuilds_its_message_from_the_residual_map() {
    let (registry, reader) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();
    let order = reader
        .sole_line(b"8=FIX.4.4|35=AE|571=T1|55=AAPL|107=HOLCIM N|552=1|54=1|9999=x|10=0|")
        .unwrap();
    let row = order.into_row(&schema).unwrap();
    let keys: Vec<String> = residual(&row, &schema)
        .into_iter()
        .map(|(key, _)| key)
        .collect();
    assert!(keys.iter().any(|key| key == "107:securitydesc"), "{keys:?}");
    assert!(keys.iter().any(|key| key.starts_with("552:")), "{keys:?}");
    assert!(keys.iter().all(|key| !key.starts_with("0:")), "{keys:?}");
    let batch = yggdryl::Serie::from_scalars(schema.clone(), [row.clone()])
        .expect("the row lands")
        .into_arrow_batch()
        .expect("a batch");
    let landed =
        yggdryl::Serie::from_arrow_batch(Some(&schema), &batch, yggdryl::ArrowCastOptions::new())
            .expect("the batch lands again")
            .scalar(0)
            .expect("its row");
    let held = yggdryl::FixMsg::from_row(Arc::clone(&registry), &schema, &landed)
        .expect("the record read out of Arrow");
    assert_eq!(held.into_row(&schema).unwrap(), row);
}

/// Children one fold names rebuild as the first of them, ASCII or not: a
/// fixed row's root is wider than a level the rebuild scans, so its fold
/// index decides which name a level already holds, and it decides as a scan
/// of every child would - `Foo_Bar` before `foobar`, `ÉTAT` before `état`,
/// and a residual entry of that fold, which rebuilds ahead of every column,
/// before both.
#[test]
fn children_one_fold_names_rebuild_as_the_first_of_them_ascii_or_not() {
    let (registry, reader) = reader();
    let fixed = fix_schema(&registry, "fix").unwrap();
    let tagged = |name: &str, tag: i32| {
        let mut field = DataType::utf8().nullable_field(name);
        field.as_fix_mut().set_tag(tag).unwrap();
        field
    };
    let mut children = fixed.fields().to_vec();
    let closing = children.len() - 1;
    assert_eq!(children[closing].name(), "fixentries");
    let extra = [
        ("Foo_Bar", 9001, "a"),
        ("foobar", 9002, "b"),
        ("ÉTAT", 9004, "c"),
        ("état", 9005, "d"),
    ];
    children.splice(
        closing..closing,
        extra.iter().map(|(name, tag, _)| tagged(name, *tag)),
    );
    let mut schema = fixed.clone();
    schema
        .set_dtype(
            StructType::from_fields(children)
                .map(DataType::from)
                .unwrap(),
        )
        .unwrap();
    let stated = |tag: i32, held: &yggdryl::FixMsg| {
        held.get_by_tag(tag)
            .and_then(|value| value.as_str().map(str::to_owned))
    };
    // `TimeInForce(59)` is the default the parse states; the bridge key
    // rebuilds ahead of the columns as the unmapped entry it was parsed as.
    for (line, foobar, children) in [
        (
            b"8=FIX.4.4|35=D|11=A|10=0|".as_slice(),
            Some("a"),
            ["timeinforce", "Foo_Bar", "ÉTAT"],
        ),
        (
            b"8=FIX.4.4|35=D|11=A|FOO-BAR=z|10=0|".as_slice(),
            None,
            ["foobar", "timeinforce", "ÉTAT"],
        ),
    ] {
        let message = reader.sole_line(line).unwrap();
        let row = message.into_row(&schema).unwrap();
        let filled = Scalar::from_sequence(row.as_sequence().unwrap().iter().enumerate().map(
            |(index, cell)| match index.checked_sub(closing) {
                Some(offset) if offset < extra.len() => Scalar::from(extra[offset].2),
                _ => cell.clone(),
            },
        ));
        let restored = yggdryl::FixMsg::from_row(Arc::clone(&registry), &schema, &filled).unwrap();
        assert_eq!(stated(9001, &restored).as_deref(), foobar, "{line:?}");
        assert_eq!(stated(9002, &restored), None, "{line:?}");
        assert_eq!(stated(9004, &restored).as_deref(), Some("c"), "{line:?}");
        assert_eq!(stated(9005, &restored), None, "{line:?}");
        let names: Vec<&str> = restored
            .entries()
            .iter()
            .map(yggdryl::FixEntry::name)
            .collect();
        assert_eq!(names, children, "{line:?}");
    }
}

/// A group column states its occurrences, its length the count, whether the
/// row holds it as a run or as a column.
#[test]
fn a_group_column_is_its_occurrences_as_a_run_and_as_a_column() {
    let (registry, reader) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();
    let order = reader
        .sole_line(b"8=FIX.4.4|35=D|11=A1|1907=1|1903=UTI-1|1906=0|10=0|")
        .unwrap();
    let row = order.into_row(&schema).unwrap();
    assert_eq!(
        column_of(&schema, 1907),
        schema.index_of("regulatorytradeids").unwrap()
    );
    let at = schema
        .index_of("regulatorytradeids")
        .expect("a regulatorytradeids column");
    let column = super::with_column_at(&row, at, &super::item_of(&schema.fields()[at]));

    let run = yggdryl::FixMsg::from_row(Arc::clone(&registry), &schema, &row).unwrap();
    let held = yggdryl::FixMsg::from_row(Arc::clone(&registry), &schema, &column).unwrap();
    for message in [&run, &held] {
        assert!(message.get_by_tag(1907).is_none(), "no counter child");
        assert_eq!(
            message
                .by_name("regulatorytradeids")
                .unwrap()
                .as_serie()
                .map(yggdryl::Serie::len),
            Some(1)
        );
    }
}

/// A message holding a group as a column whose occurrences state their
/// members in another order than the fixed row declares lands each member
/// under its own name, and the fixed group covers the entry it states.
#[test]
fn a_group_column_is_regrouped_by_name_into_the_fixed_row() {
    let (registry, reader) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();
    let parsed = reader
        .sole_line(b"8=FIX.4.4|35=D|11=A1|1907=1|1903=UTI-1|1905=SRC|1906=0|10=0|")
        .unwrap();
    let (mut root, row) = super::restatable(&registry, &parsed, &[35, 52]);
    let at = root
        .index_of("regulatorytradeids")
        .expect("the group's column");
    let declared = super::item_of(&root.fields()[at]);
    let mut item = declared.clone();
    item.set_dtype(
        StructType::from_fields(declared.fields().iter().rev().cloned())
            .map(DataType::from)
            .unwrap(),
    )
    .unwrap();
    let names = |field: &Field| {
        field
            .fields()
            .iter()
            .map(|member| member.name().to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        names(&item).last().map(String::as_str),
        Some("regulatorytradeid"),
        "the message states its members in another order than the fixed row"
    );
    assert_ne!(
        names(&item),
        names(&super::item_of(
            &schema.fields()[schema
                .index_of("regulatorytradeids")
                .expect("a regulatorytradeids column")]
        ))
    );
    let mut group = root.fields()[at].clone();
    group.set_dtype(DataType::serie(item.clone())).unwrap();
    let mut children = root.fields().to_vec();
    children[at] = group;
    root.set_dtype(
        StructType::from_fields(children)
            .map(DataType::from)
            .unwrap(),
    )
    .unwrap();
    let cells = row.as_sequence().unwrap();
    let reordered = Scalar::from_sequence(cells.iter().enumerate().map(|(index, cell)| {
        if index != at {
            return cell.clone();
        }
        Scalar::from_sequence(cell.as_sequence().unwrap().iter().map(|occurrence| {
            Scalar::from_sequence(occurrence.as_sequence().unwrap().iter().rev().cloned())
        }))
    }));
    let run =
        yggdryl::FixMsg::with_registry(Arc::clone(&registry), root.clone(), reordered.clone())
            .unwrap();
    let message = yggdryl::FixMsg::with_registry(
        Arc::clone(&registry),
        root,
        super::with_column_at(&reordered, at, &item),
    )
    .unwrap();
    assert!(super::holds_column(&message, "regulatorytradeids"));

    let row = message.into_row(&schema).unwrap();
    let cells = row.as_sequence().unwrap();
    let group = schema
        .index_of("regulatorytradeids")
        .expect("a regulatorytradeids column");
    let declared = super::item_of(&schema.fields()[group]);
    let occurrences = cells[group].sequence_rows().expect("the fixed group");
    assert_eq!(occurrences.len(), 1);
    let members = occurrences[0].sequence_rows().expect("one occurrence");
    let member = |name: &str| members[declared.index_of(name).expect(name)].as_str();
    assert_eq!(member("regulatorytradeid"), Some("UTI-1"));
    assert_eq!(member("regulatorytradeidsource"), Some("SRC"));
    assert!(
        residual(&row, &schema)
            .iter()
            .all(|(key, _)| !key.starts_with("1907:")),
        "the fixed group is not duplicated in fixentries"
    );
    assert_eq!(row, run.into_row(&schema).unwrap());
}

// ---------------------------------------------------------------------------
// Moved out of `rust/src/fix/schema.rs`, which is the file this one mirrors:
// the member order every stated occurrence of a group agrees with. It is a
// step inside the fixed row rather than a door of its own, so it is reached
// through `yggdryl::internals`.
// ---------------------------------------------------------------------------

#[cfg(feature = "internals")]
mod group_member_order {
    use yggdryl::internals::fix_schema::ordered_group_union;
    use yggdryl::{DataType, Error, Field, Scalar};

    fn occurrence(names: &[&str]) -> Vec<(String, Scalar)> {
        names
            .iter()
            .map(|name| ((*name).to_owned(), Scalar::Null))
            .collect()
    }

    #[test]
    fn group_member_order_respects_every_occurrence() {
        let union: Vec<Field> = ["a", "c", "b", "d"]
            .map(|name| DataType::utf8().nullable_field(name))
            .into();
        let stated = [
            occurrence(&["a", "c"]),
            occurrence(&["b", "d"]),
            occurrence(&["a", "b", "c"]),
        ];
        let ordered = ordered_group_union(union, &stated, "example").expect("consistent order");
        assert_eq!(
            ordered.iter().map(Field::name).collect::<Vec<_>>(),
            ["a", "b", "c", "d"]
        );

        let contradictory = [occurrence(&["a", "b"]), occurrence(&["b", "a"])];
        assert!(matches!(
            ordered_group_union(ordered, &contradictory, "example"),
            Err(Error::InvalidRecord { .. })
        ));
    }
}

/// An iceberg stop order states its terms in FIX's own fields, and the
/// fixed row opens with them under the names every market row states them
/// by: `stoppx`, `displayqty` and `cxlqty` are FIX's own fields, named
/// alike, as `timeinforce` is `TimeInForce(59)`, and `marketdatatype`,
/// `hiddenqty` and `ticker` are derived from the fields FIX states them in,
/// while `OrdType(40)` and `Symbol(55)` stay columns of their own beside
/// them. The message is recorded when it was sent, where no carrier says
/// otherwise. A row read back is the message.
#[test]
fn an_iceberg_stop_order_states_its_terms_in_the_shared_columns() {
    use yggdryl::{Decimal, FixMsg, MarketDataType};

    let (registry, reader) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();
    let message = reader
        .sole_line(
            b"8=FIX.4.4|35=D|52=20260921-10:00:00|11=C1|55=AAPL|54=1|40=4|44=100|99=98|38=10|1138=4|59=0|10=0|",
        )
        .unwrap();
    assert_eq!(message.get_stoppx(), Some(Decimal::from_int(98)));
    assert_eq!(message.get_displayqty(), Some(Decimal::from_int(4)));
    assert_eq!(message.get_hiddenqty(), Some(Decimal::from_int(6)));
    assert_eq!(message.get_marketdatatype(), MarketDataType::OrdStopLimit);
    assert_eq!(message.get_ticker(), Some("AAPL"));
    assert_eq!(message.get_recdunix(), Some(message.header().sendingtime()));

    let row = message.clone().into_row(&schema).unwrap();
    let cell =
        |name: &str| row.as_sequence().expect("a row")[schema.index_of(name).expect(name)].clone();
    assert_eq!(
        Decimal::from_scalar(&cell("stoppx")),
        Some(Decimal::from_int(98))
    );
    assert_eq!(
        Decimal::from_scalar(&cell("displayqty")),
        Some(Decimal::from_int(4))
    );
    assert_eq!(
        Decimal::from_scalar(&cell("hiddenqty")),
        Some(Decimal::from_int(6))
    );
    assert_eq!(
        cell("marketdatatype"),
        yggdryl::Scalar::MarketDataType(MarketDataType::OrdStopLimit)
    );
    assert_eq!(
        cell("ordtype").as_str(),
        Some("4"),
        "FIX's own column stays"
    );
    assert_eq!(cell("ticker").as_str(), Some("AAPL"));
    assert_eq!(
        cell("symbol").as_str(),
        Some("AAPL"),
        "FIX's own column stays"
    );
    assert!(!cell("timeinforce").is_null());
    assert_eq!(cell("recdunix"), cell("sendingtime"));
    let again = FixMsg::from_row(Arc::clone(&registry), &schema, &row).unwrap();
    assert_eq!(again.into_row(&schema).unwrap(), row);

    // FIX 4.4's `MaxFloor(111)` states the peak where `DisplayQty(1138)` is
    // not - the newest specification's name for it - and the row's
    // `displayqty` reads it so.
    let floor = reader
        .sole_line(b"8=FIX.4.4|35=D|11=C2|55=AAPL|54=1|40=2|44=100|38=10|111=3|10=0|")
        .unwrap();
    assert_eq!(floor.get_displayqty(), Some(Decimal::from_int(3)));
    assert_eq!(floor.get_hiddenqty(), Some(Decimal::from_int(7)));
    let row = floor.into_row(&schema).unwrap();
    assert_eq!(
        Decimal::from_scalar(&row.as_sequence().unwrap()[schema.index_of("displayqty").unwrap()]),
        Some(Decimal::from_int(3))
    );

    // A report states what it canceled.
    let canceled = reader
        .sole_line(
            b"8=FIX.4.4|35=8|11=C3|37=O3|39=4|150=4|55=AAPL|54=1|38=10|84=6|14=4|151=0|10=0|",
        )
        .unwrap();
    assert_eq!(canceled.get_cxlqty(), Some(Decimal::from_int(6)));
}

/// What the setters fill off a message's fields - what is left of the order
/// and so its quantity, the canceled rest, the quote - is a column of the
/// fixed row, and the row read back is the message.
#[test]
fn a_reports_filled_quantities_are_columns_and_read_back() {
    use yggdryl::{Decimal, FixMsg};

    let (registry, reader) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();
    let decimal = |row: &Scalar, name: &str| {
        Decimal::from_scalar(&row.as_sequence().expect("a row")[schema.index_of(name).expect(name)])
    };
    let int = |value: i64| Some(Decimal::from_int(value));

    // Part filled: `LeavesQty(151)` is what was ordered less what traded.
    let working = reader
        .sole_line(
            b"8=FIX.4.4|35=8|37=O1|17=E1|39=1|150=F|55=AAPL|54=1|38=100|14=40|32=40|31=10|10=0|",
        )
        .unwrap();
    assert_eq!(working.get_leavesqty(), int(60));
    let row = working.clone().into_row(&schema).unwrap();
    assert_eq!(decimal(&row, "ordqty"), int(100));
    assert_eq!(
        decimal(&row, "quantity"),
        None,
        "Quantity(53) is FIX's own and unstated"
    );
    assert_eq!(
        decimal(&row, "leavesqty"),
        int(60),
        "LeavesQty(151), which the enrichment states"
    );
    let again = FixMsg::from_row(Arc::clone(&registry), &schema, &row).unwrap();
    assert_eq!(again.get_leavesqty(), int(60));
    assert_eq!(again.get_quantity(), int(60));
    assert_eq!(again.into_row(&schema).unwrap(), row);

    // Canceled: nothing left, the rest canceled.
    let canceled = reader
        .sole_line(b"8=FIX.4.4|35=8|37=O2|17=E2|39=4|150=4|55=AAPL|54=1|38=100|14=40|10=0|")
        .unwrap();
    assert_eq!(
        (canceled.get_leavesqty(), canceled.get_cxlqty()),
        (int(0), int(60))
    );
    let row = canceled.clone().into_row(&schema).unwrap();
    let again = FixMsg::from_row(Arc::clone(&registry), &schema, &row).unwrap();
    assert_eq!(
        (again.get_leavesqty(), again.get_cxlqty()),
        (int(0), int(60))
    );
    assert_eq!(again.into_row(&schema).unwrap(), row);
}
