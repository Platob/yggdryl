//! The fixed row: columns spelled by name and filled by tag, derived facts,
//! and the one closing arrival record.

use super::SoleMessage;

use std::sync::Arc;

use yggdryl::{DataType, Field, FixCodec, FixRegistry, Scalar, fix_column_of, fix_schema};

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

/// The `parties` occurrences out of a fixed row.
///
/// The group is reached by its name and not by tag 453, which is the
/// counter's column: a List group and its counter are two columns.
fn group<'row>(row: &'row Scalar, schema: &Field) -> &'row [Scalar] {
    let at = schema.index_of("parties").expect("a parties column");
    row.as_sequence().expect("a row")[at]
        .as_sequence()
        .expect("the parties")
}

#[test]
fn the_fixed_schema_keeps_existing_tags_and_appends_the_settled_identity_fields() {
    use yggdryl::fix::{BODY_TAGS, GROUP_TAGS, HEADER_TAGS, TRAILER_TAGS};

    let tags = yggdryl::fix_schema_tags();
    assert_eq!(tags.len(), 119);
    // The row is read in bands rather than by tag number: when it happened,
    // which event it is, which message carried it, which instrument it is
    // about, which order it belongs to, what it states, how it went, the
    // groups kept whole, and last the frame.
    assert_eq!(
        &tags[..12],
        [
            yggdryl::UNIX_TAG_NAME.0,
            yggdryl::CREATUNIX_TAG_NAME.0,
            yggdryl::PREVUNIX_TAG_NAME.0,
            yggdryl::EXPIRUNIX_TAG_NAME.0,
            yggdryl::SNAPUNIX_TAG_NAME.0,
            yggdryl::RECORDEDAT_TAG_NAME.0,
            52,
            122,
            60,
            64,
            75,
            126,
        ],
        "when it happened"
    );
    assert_eq!(
        &tags[12..21],
        [
            yggdryl::CURRUUID_TAG_NAME.0,
            yggdryl::CROSSUUID_TAG_NAME.0,
            yggdryl::CROSSCODE_TAG_NAME.0,
            yggdryl::HASHCODE_TAG_NAME.0,
            yggdryl::CROSSHASHCODE_TAG_NAME.0,
            yggdryl::PREVUUID_TAG_NAME.0,
            yggdryl::SEQNUM_TAG_NAME.0,
            yggdryl::PARENTUUIDS_TAG_NAME.0,
            yggdryl::IDENTIFIERS_TAG_NAME.0,
        ],
        "which event"
    );
    assert_eq!(
        &tags[21..28],
        [8, 35, 34, 49, 56, 43, yggdryl::MSGDIRECTION_TAG_NAME.0],
        "which message"
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
    // The bridge's own keys and the arrival record's counter close the row.
    assert_eq!(
        &tags[tags.len() - 2..],
        [
            yggdryl::METADATA_TAG_NAME.0,
            yggdryl::NOFIXENTRIES_TAG_NAME.0
        ]
    );

    let (registry, _) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();
    let names: Vec<_> = schema.fields().iter().map(Field::name).collect();
    // The frame closes the row: the trailer, then the bridge's own keys, then
    // the arrival record and the counter that counts it.
    assert_eq!(
        &names[names.len() - 5..],
        [
            "signature",
            "checksum",
            "metadata",
            "nofixentries",
            "fixentries"
        ]
    );
    for tag in [yggdryl::PREVUNIX_TAG_NAME.0, yggdryl::PREVUUID_TAG_NAME.0] {
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
}

#[test]
fn the_columns_are_named_by_fold_and_filled_by_tag() {
    let (registry, _) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();
    let names: Vec<&str> = schema.fields().iter().map(Field::name).collect();

    // A column is spelled by the dictionary's folded name and found by the
    // tag its field carries: 32 is `LastShares` in 4.2 and `LastQty` in a
    // newest one, and the column is the dictionary's one `lastqty` in both.
    // The crate's own clocks lead the row, then its identities, then the
    // standard header - a table is read by time and joined by identity.
    // The row reads the way a message reads: when it happened first, then
    // which event it is, then which message and session, and the header
    // after them. Each column is found by its name rather than by an offset,
    // so a band that gains one does not move this assertion.
    let at = |name: &str| {
        schema
            .index_of(name)
            .unwrap_or_else(|| panic!("a {name} column"))
    };
    for pair in ["unix", "creatunix", "prevunix", "expirunix", "snapunix"].windows(2) {
        assert!(at(pair[0]) < at(pair[1]), "{pair:?} in {names:?}");
    }
    assert!(at("snapunix") < at("curruuid"), "the clocks open the row");
    assert!(
        at("curruuid") < at("beginstring"),
        "the event before the header"
    );
    let header = schema.index_of("beginstring").expect("the header opens");
    assert_eq!(
        &names[header..header + 3],
        ["beginstring", "msgtype", "msgseqnum"]
    );
    assert_eq!(schema.index_of("msgtype"), Some(header + 1));
    assert_eq!(column_of(&schema, 35), header + 1);
    assert_eq!(names[column_of(&schema, 32)], "lastqty");
    assert_eq!(names.last(), Some(&"fixentries"));

    // The dictionary's own typing reaches the column, so a currency column is
    // the packed currency and a side is the packed side.
    let fields = schema.fields();
    let typed = |tag: i32| fields[column_of(&schema, tag)].dtype().clone();
    assert_eq!(typed(15), DataType::Currency, "Currency(15)");
    assert_eq!(typed(120), DataType::Currency, "SettlCurrency(120)");
    assert_eq!(typed(54), DataType::Side, "Side(54)");
    assert_eq!(typed(35), DataType::utf8(), "MsgType(35)");
    assert!(
        matches!(typed(60), DataType::DateTime64 { .. }),
        "TransactTime"
    );
    // The price and the quantity a message is about are the crate's own
    // columns at the crate's own scale, because the event holds one fact
    // whichever of FIX's fields a message spelled it in.
    assert_eq!(
        typed(yggdryl::PX_TAG_NAME.0),
        DataType::decimal128(38, 18).unwrap()
    );
    assert_eq!(
        typed(yggdryl::QTY_TAG_NAME.0),
        typed(yggdryl::PX_TAG_NAME.0)
    );
    assert_eq!(
        typed(yggdryl::PREVUNIX_TAG_NAME.0),
        typed(yggdryl::UNIX_TAG_NAME.0)
    );
    assert_eq!(typed(yggdryl::PREVUUID_TAG_NAME.0), DataType::Uuid);
    assert_eq!(typed(yggdryl::CURRUUID_TAG_NAME.0), DataType::Uuid);
    assert_eq!(typed(yggdryl::HASHCODE_TAG_NAME.0), DataType::UInt64);

    // Crate-owned columns follow the same contract as FIX's: the stable
    // identity is the folded name, while renderers receive the FIX-style
    // spelling the field keeps as its display.
    for (tag, display) in [
        (yggdryl::UNIX_TAG_NAME.0, "Unix"),
        (yggdryl::MSGCTXID_TAG_NAME.0, "MsgCtxId"),
        (yggdryl::PLUGINID_TAG_NAME.0, "PluginId"),
        (yggdryl::MSGSESSIONID_TAG_NAME.0, "MsgSessionId"),
        (yggdryl::ISINCODE_TAG_NAME.0, "ISINCode"),
        (yggdryl::MICCODE_TAG_NAME.0, "MICCode"),
        (yggdryl::STATE_TAG_NAME.0, "State"),
        (yggdryl::HASHCODE_TAG_NAME.0, "HashCode"),
        (yggdryl::CROSSHASHCODE_TAG_NAME.0, "CrossHashCode"),
        (yggdryl::CROSSCODE_TAG_NAME.0, "CrossCode"),
        (yggdryl::PREVUNIX_TAG_NAME.0, "PrevUnix"),
        (yggdryl::PREVUUID_TAG_NAME.0, "PrevUuid"),
    ] {
        let field = &fields[column_of(&schema, tag)];
        assert_eq!(field.display(), Some(display), "tag {tag}");
    }

    // The replay bundle and BeginString are required, and nothing else:
    // `snapshotat` is only what a snapshot stamps, so it is nullable like
    // every other column a message may not state.
    let required: Vec<&str> = fields
        .iter()
        .filter(|field| !field.is_nullable())
        .map(Field::name)
        .collect();
    assert_eq!(
        required,
        [
            "unix",
            "creatunix",
            "curruuid",
            "crossuuid",
            "hashcode",
            "crosshashcode",
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
    use yggdryl::media::RecordOptions;
    use yggdryl::media::ipc::{Ipc, IpcOptions};
    use yggdryl::{CROSSHASHCODE_TAG_NAME, FixMsg, HASHCODE_TAG_NAME, IOMedia, PREVUUID_TAG_NAME};

    let (registry, codec) = reader();
    let codec = codec.with_separator(b'|');
    let wire = b"8=FIX.4.4|35=D|11=UUID-ORDER-1|55=AAPL|10=0|";
    let mut message = codec.sole_line(wire).unwrap();
    let digest = message.digest();
    let previous = DataType::Uuid
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
        b"8=FIX.4.4|35=D|59=0|11=UUID-ORDER-1|55=AAPL|10=0|"
    );

    let schema = fix_schema(&registry, "fix").unwrap();
    let row = message.into_row(&schema).unwrap();
    for (tag, _) in [HASHCODE_TAG_NAME, CROSSHASHCODE_TAG_NAME] {
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
    assert_eq!(field.dtype(), &DataType::Uuid);
    assert_eq!(at(&row, &schema, PREVUUID_TAG_NAME.0), &previous);

    let restored = FixMsg::from_row(Arc::clone(&registry), &schema, &row).unwrap();
    assert_eq!(restored.into_row(&schema).unwrap(), row);
    assert_eq!(restored.digest(), digest);
    assert_eq!(restored.into_bytes(b'|'), message.into_bytes(b'|'));

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
    for (_, name) in [HASHCODE_TAG_NAME, CROSSHASHCODE_TAG_NAME] {
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

    let mut encoded = Vec::new();
    assert_eq!(
        codec
            .write_arrow_reader(stored.read_arrow_reader(&options).unwrap(), &mut encoded)
            .unwrap(),
        1
    );
    assert_eq!(
        encoded,
        b"8=FIX.4.4|35=D|59=0|11=UUID-ORDER-1|55=AAPL|10=0|\n"
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
        assert_eq!(at(&row, schema, 54).as_str(), Some("BUY"));
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
    let row = order.into_row(&schema).unwrap();
    assert_eq!(at(&row, &schema, 35).as_str(), Some("D"));
    assert_eq!(at(&row, &schema, 11).as_str(), Some("ORDER-1"));
    assert_eq!(at(&row, &schema, 55).as_str(), Some("AAPL"));
    assert_eq!(at(&row, &schema, 15).as_str(), Some("USD"));
    // `Price(44)` has no column of its own: the price a message is about is
    // the crate's `px`, at the crate column's own scale.
    assert_eq!(
        at(&row, &schema, yggdryl::PX_TAG_NAME.0).as_decimal(),
        Some((yggdryl::i256::from_i128(12_500_000_000_000_000_000), 18))
    );

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
    assert!(!at(&row, &schema, yggdryl::HASHCODE_TAG_NAME.0).is_null());

    // The clock the row is cut by - how a layout is cut from it is the
    // target's, not a column of this crate's.
    assert!(!at(&row, &schema, yggdryl::UNIX_TAG_NAME.0).is_null());

    // The version it was read at, which the header holds and the row states.
    assert_eq!(at(&row, &schema, 8).as_str(), Some("FIX.4.4"));

    // Hard identities and clocks are stored mirrors, never invented arrivals.
    assert!(order.get_by_tag(65_000).is_none());
    assert!(order.get_by_tag(yggdryl::HASHCODE_TAG_NAME.0).is_some());
    assert!(order.get_by_tag(yggdryl::UNIX_TAG_NAME.0).is_some());
    assert!(
        order
            .entries()
            .iter()
            .all(|entry| entry.tag() != yggdryl::UNIX_TAG_NAME.0)
    );
}

#[test]
fn a_lane_a_message_never_wrote_is_still_true_of_it() {
    let (registry, reader) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();

    // A buy order at a price is a party willing to pay it, so the bid lane it
    // never wrote is filled and the ask lane is not.
    let buy = reader
        .sole_line(b"8=FIX.4.4|35=D|11=A|54=1|44=12.5|38=100|10=0|")
        .unwrap();
    let row = buy.into_row(&schema).unwrap();
    assert_eq!(at(&row, &schema, 132), &Scalar::from(12.5_f64));
    assert_eq!(at(&row, &schema, 134), &Scalar::from(100.0_f64));
    assert!(at(&row, &schema, 133).is_null(), "no ask lane on a buy");

    // And a stated column is never overwritten by a derivation.
    let stated = reader
        .sole_line(b"8=FIX.4.4|35=D|11=A|54=1|44=12.5|132=99.0|10=0|")
        .unwrap();
    let row = stated.into_row(&schema).unwrap();
    assert_eq!(at(&row, &schema, 132), &Scalar::from(99.0_f64));
}

#[test]
fn the_row_stays_lossless_and_says_what_nothing_explained() {
    let (registry, reader) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();
    let row = reader
        .sole_line(b"8=FIX.4.4|35=D|11=A|9999=x|VenueOwnThing=y|10=0|")
        .unwrap()
        .into_row(&schema)
        .unwrap();
    let held = row.as_sequence().expect("a row");
    let entries = held.last().unwrap().as_sequence().expect("the record");

    // The record is the content the message holds, in its order, so the wire
    // is rebuilt from it and never from the columns: the version, the type
    // and the day order the dictionary derives for an order are facts the
    // message holds typed, so none of the three is an entry.
    assert_eq!(entries.len(), 4);
    // A key no dictionary explains is named after itself, folded as every
    // name is: the name cannot be null, and the key is the only one it has.
    let named: Vec<_> = entries
        .iter()
        .filter(|entry| entry.get(0).and_then(Scalar::as_i128) == Some(0))
        .map(|entry| entry.get(1).and_then(Scalar::as_str).unwrap())
        .collect();
    assert_eq!(named, ["9999", "venueownthing"]);

    // A key one does explain carries the dictionary's canonical name, so a
    // consumer groups by name without a dictionary of its own.
    let clordid = entries
        .iter()
        .find(|entry| entry.get(0).and_then(Scalar::as_i128) == Some(11))
        .expect("the ClOrdID arrival");
    assert_eq!(clordid.get(1).and_then(Scalar::as_str), Some("clordid"));
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
/// which Arrow reads back as `fixed_size_binary(16)`.
///
/// The round trip writes stamped messages through the widened schema, reads
/// them back, and compares each value to what the projected row stated.
#[cfg(feature = "iceberg")]
#[test]
fn the_identity_columns_cross_an_iceberg_table_in_the_storage_their_width_asks_for() {
    use yggdryl::holder::local::Folder;
    use yggdryl::media::iceberg::{
        FormatVersion, PartitionSpec, PrimitiveType, Table, assign_field_ids,
    };
    use yggdryl::{CROSSHASHCODE_TAG_NAME, HASHCODE_TAG_NAME, PREVUUID_TAG_NAME, Scheme};

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
    let refused = PrimitiveType::from_dtype(fixed.get_field(HASHCODE_TAG_NAME.1).unwrap().dtype())
        .map(|held| held.to_string())
        .unwrap_err()
        .to_string();
    assert!(refused.contains("uint64"), "{refused}");
    assert!(refused.contains("into_scheme_compat"), "{refused}");

    // Read off the projected row, because the fixed schema is what the
    // messages state and the table's business is to carry those values back:
    // `msghash` digests the row it lands in (see
    // `message.md#clocks-and-identity`).
    let digests = [HASHCODE_TAG_NAME, CROSSHASHCODE_TAG_NAME];
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

    let path = Folder::temporary()
        .unwrap()
        .path()
        .unwrap()
        .join(format!("yggdryl-fix-identity-lake-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    // Unpartitioned: what is pinned here is the identity columns' storage.
    // How a layout is cut is the target's - an Iceberg table takes an `hour`
    // transform over `updatedat` - and no longer a column of this crate's.
    let mut table = Table::create(
        Folder::new(&path).unwrap(),
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

    // A message spelling a crate column's name with a value its datatype
    // cannot hold: five bytes under `miccode`, which is a four-byte MIC, and
    // a spelling no ISIN check digit closes under `isincode`.
    let root = DataType::from_fields([
        DataType::utf8().nullable_field("miccode"),
        DataType::utf8().nullable_field("isincode"),
    ])
    .unwrap()
    .required_field("NewOrderSingle");
    let message = yggdryl::FixMsg::with_registry(
        Arc::clone(&registry),
        root,
        Scalar::from_record([
            ("miccode", Scalar::from("XLONX")),
            ("isincode", Scalar::from("NOTANISIN12")),
        ])
        .unwrap(),
    )
    .unwrap();

    let row = message.into_row(&schema).unwrap();
    assert!(at(&row, &schema, yggdryl::MICCODE_TAG_NAME.0).is_null());
    assert!(at(&row, &schema, yggdryl::ISINCODE_TAG_NAME.0).is_null());
    // The row is still a row: the columns beside the unreadable ones are
    // filled, and the identity bundle still settled.
    assert!(!at(&row, &schema, yggdryl::HASHCODE_TAG_NAME.0).is_null());
}

/// A market spelled wider than a MIC derives nothing rather than refusing.
///
/// `miccode` derives from `SecurityExchange`, which FIX types as free text:
/// a venue writing more than four bytes there has not named a MIC, and the
/// honest column is empty.
#[test]
fn a_derivation_wider_than_its_column_stays_silent() {
    let (registry, reader) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();

    let narrow = reader
        .sole_line(b"8=FIX.4.4|35=D|11=A|55=AAPL|54=1|207=XLON|10=0|")
        .unwrap()
        .into_row(&schema)
        .unwrap();
    assert_eq!(
        at(&narrow, &schema, yggdryl::MICCODE_TAG_NAME.0).as_str(),
        Some("XLON"),
    );

    let wide = reader
        .sole_line(b"8=FIX.4.4|35=D|11=A|55=AAPL|54=1|207=XLONX|10=0|")
        .unwrap()
        .into_row(&schema)
        .unwrap();
    assert!(at(&wide, &schema, yggdryl::MICCODE_TAG_NAME.0).is_null());
}

/// A group keeps every member that reads, whatever one of them turned out to be.
///
/// A bridge packs an occurrence into one value and a venue writes a member
/// the dictionary does not declare, so a group arrives one member short or
/// one member long often enough to matter. Nulling the whole group over it
/// would throw away the parties that did read, and refusing would throw away
/// the capture, so the row keeps what reads and says the rest is absent.
#[test]
fn a_group_keeps_the_members_that_read() {
    let (registry, reader) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();

    // A packed occurrence stating only the identifier: the members it never
    // wrote are null and the identifier it did write is kept.
    let packed = reader
        .sole_line(b"MSGTYPE=D|453=2|453[0]=448=BUYSIDE|453[1]=448=VENUE")
        .unwrap()
        .into_row(&schema)
        .unwrap();
    let occurrences = group(&packed, &schema);
    let identifiers: Vec<Option<&str>> = occurrences
        .iter()
        .map(|party| party.as_sequence().expect("a party")[0].as_str())
        .collect();
    assert_eq!(identifiers, [Some("BUYSIDE"), Some("VENUE")]);
    assert!(occurrences[0].as_sequence().unwrap()[1].is_null());
    // The counter column is the group's own tag and still counts them.
    assert_eq!(at(&packed, &schema, 453).as_i128(), Some(2));

    // And one member the row cannot read costs that member alone: the
    // occurrence around it and the occurrences beside it stay.
    let marked = reader
        .sole_line(b"MSGTYPE=ZMIN|#453=1|#453[0]=PARTYID=BUYSIDEPARTYROLE=1")
        .unwrap()
        .into_row(&schema)
        .unwrap();
    let members = group(&marked, &schema)[0].as_sequence().expect("a party");
    assert_eq!(members[0].as_str(), Some("BUYSIDE"));
    assert_eq!(members[2].as_i128(), Some(1));
}
