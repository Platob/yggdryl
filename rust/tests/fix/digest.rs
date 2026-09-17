//! The value digest, the dedup adapter, and the crate definitions.

use super::SoleMessage;

use yggdryl::{DataType, FixCodec, FixDedup, FixId, FixRegistry, Scalar};

fn reader() -> FixCodec {
    super::fixed_codec(super::committed_registry())
}

#[test]
fn identical_entries_hash_equal_and_a_different_order_does_not() {
    let reader = reader();
    let one = reader
        .sole_line(b"8=FIX.4.4|35=D|11=A|55=AAPL|10=0|")
        .unwrap();
    let same = reader
        .sole_line(b"8=FIX.4.4|35=D|11=A|55=AAPL|10=0|")
        .unwrap();
    assert_eq!(one.digest(), same.digest());
    assert_eq!(one.stable_hash(), same.stable_hash());
    assert_eq!(one.stable_hash(), one.clone().stable_hash());

    // Order carries meaning inside a repeating group, so it is never sorted
    // away: the same pairs in another order are another message.
    let reordered = reader
        .sole_line(b"8=FIX.4.4|35=D|55=AAPL|11=A|10=0|")
        .unwrap();
    assert_ne!(one.digest(), reordered.digest());
    assert_ne!(one.stable_hash(), reordered.stable_hash());

    // Two calls are the same walk twice, because nothing was stored.
    assert_eq!(one.digest(), one.digest());
}

#[test]
fn a_length_prefix_is_what_keeps_two_split_values_apart() {
    let reader = reader();
    // The case a separator-framed digest gets wrong: concatenated, both read
    // `123`, and only the lengths tell them apart. A FIX value may hold any
    // byte at all, so no separator could have done it either.
    let split = reader
        .sole_line(b"8=FIX.4.4|35=D|9999=1|9998=23|10=0|")
        .unwrap();
    let other = reader
        .sole_line(b"8=FIX.4.4|35=D|9999=12|9998=3|10=0|")
        .unwrap();
    assert_ne!(split.digest(), other.digest());
}

#[test]
fn the_envelope_is_not_the_message() {
    let reader = reader();
    let row = "8=FIX.4.4|9=64|35=D|11=A|55=AAPL|10=203|";
    let one = reader.sole_line(row.as_bytes()).unwrap();

    // A recomputed body length and a different checksum describe how the
    // message was written down, not what it says.
    let rewritten = reader
        .sole_line(b"8=FIX.4.4|9=999|35=D|11=A|55=AAPL|10=000|")
        .unwrap();
    assert_eq!(one.digest(), rewritten.digest());

    // The same message re-serialized with another separator reads back equal.
    let soh = one.into_bytes(0x01);
    let again = reader.parse_fix_line(&soh).unwrap();
    assert_eq!(one.digest(), again.digest());

    // The session layer is not the message either: the same order sent a
    // second later, over another session, on a redelivery, is one message.
    let relayed = reader
        .sole_line(b"8=FIX.4.4|35=D|34=91|49=DESK|56=VENUE|52=20240102-10:15:31.000|43=Y|11=A|55=AAPL|10=0|")
        .unwrap();
    let original = reader
        .sole_line(
            b"8=FIX.4.4|35=D|34=7|49=OTHER|56=ELSEWHERE|52=20240102-10:15:30.000|11=A|55=AAPL|10=0|")
        .unwrap();
    assert_eq!(relayed.digest(), original.digest());

    // The whole standard header is the envelope rather than a curated part of
    // it: the application version, the encoding and the last sequence number
    // processed all describe how to read this delivery, not what it says.
    let annotated = reader
        .sole_line(b"8=FIX.4.4|35=D|1128=9|1129=X|1156=1|347=UTF-8|369=6|11=A|55=AAPL|10=0|")
        .unwrap();
    assert_eq!(original.digest(), annotated.digest());

    // What the message says still separates it, and so does what it is.
    let other = reader
        .sole_line(b"8=FIX.4.4|35=D|11=A|55=MSFT|10=0|")
        .unwrap();
    assert_ne!(original.digest(), other.digest());
    let typed = reader
        .sole_line(b"8=FIX.4.4|35=F|11=A|55=AAPL|10=0|")
        .unwrap();
    assert_ne!(
        original.digest(),
        typed.digest(),
        "a type is not an envelope"
    );
}

#[test]
fn two_unknown_keys_carrying_one_value_are_two_messages() {
    let reader = reader();
    // Neither key names a field, so the tag is `0` for both and only the key
    // itself distinguishes them.
    let one = reader.sole_line(b"35=D|VenueOwnThing=x").unwrap();
    let other = reader.sole_line(b"35=D|OtherVenueThing=x").unwrap();
    assert_ne!(one.digest(), other.digest());

    // A message that recorded no arrival digests as the empty walk - correct,
    // because none of them arrived - whichever door built it: pairs a caller
    // handed in stating nothing, and a document stating no attribute. A line
    // that states no message states none at all now, so it is
    // no longer one of the doors that can answer an empty message.
    let empty = reader.parse_pairs(std::iter::empty()).unwrap();
    assert!(empty.entries().is_empty());
    let stated = reader.parse_fixml_line(b"<Order/>").unwrap();
    assert!(stated.entries().is_empty());
    assert_eq!(empty.digest(), stated.digest());
    assert!(
        reader
            .parse_line(b"also nothing here")
            .unwrap()
            .next()
            .is_none()
    );
}

#[test]
fn dedup_drops_the_adjacent_republication_and_counts_it() {
    let reader = reader();
    let rows = [
        "8=FIX.4.4|35=D|11=A|10=0|",
        "8=FIX.4.4|35=D|11=A|10=0|",
        "8=FIX.4.4|35=D|11=A|10=0|",
        "8=FIX.4.4|35=D|11=B|10=0|",
        "8=FIX.4.4|35=D|11=A|10=0|",
    ];
    let mut dedup = FixDedup::new(
        rows.iter()
            .map(|row| reader.sole_line(row.as_bytes()).unwrap()),
    );
    let kept: Vec<String> = dedup
        .by_ref()
        .map(|held| held.into_text('|').unwrap())
        .collect();
    // What each row emits, which an order's derived `TimeInForce` makes
    // more than the row itself.
    let emitted = |row: &str| {
        reader
            .sole_line(row.as_bytes())
            .unwrap()
            .into_text('|')
            .unwrap()
    };

    // Adjacent only: the `A` that returns after `B` is a third event.
    assert_eq!(kept.len(), 3);
    assert_eq!(kept[0], emitted(rows[0]));
    assert_eq!(kept[1], emitted(rows[3]));
    assert_eq!(kept[2], emitted(rows[4]));
    assert_eq!(dedup.dropped(), 2);

    // Nothing to drop drops nothing, and the count says so.
    let mut clean = FixDedup::new(
        ["8=FIX.4.4|35=D|11=A|10=0|", "8=FIX.4.4|35=D|11=B|10=0|"]
            .iter()
            .map(|row| reader.sole_line(row.as_bytes()).unwrap()),
    );
    assert_eq!(clean.by_ref().count(), 2);
    assert_eq!(clean.dropped(), 0);
}

#[test]
fn a_redelivery_of_one_order_is_one_order() {
    let reader = reader();
    let original = "8=FIX.4.4|35=D|34=7|52=20240102-10:15:30.000|11=A|55=AAPL|10=0|";
    // A replay carries a fresh sequence number, a fresh time and the resend
    // flag - all of them envelope - so it is the same order said twice.
    let resend = "8=FIX.4.4|35=D|34=8|52=20240102-10:16:00.000|43=Y|11=A|55=AAPL|10=0|";

    let replayed = [original, resend, resend];
    let mut dedup = FixDedup::new(
        replayed
            .iter()
            .map(|row| reader.sole_line(row.as_bytes()).unwrap()),
    );
    assert_eq!(
        dedup.by_ref().count(),
        1,
        "one order, however often it arrives"
    );
    assert_eq!(dedup.dropped(), 2);

    // That a delivery *was* a replay is still readable, from the header the
    // digest declined to fold in.
    let held = reader.sole_line(resend.as_bytes()).unwrap();
    assert_eq!(held.header().possdupflag(), Some(true));
    assert_eq!(held.header().msgseqnum(), Some(8));
    assert_eq!(held.by_tag(34).unwrap(), Scalar::from(8_u64));

    // A genuinely different order is a different order.
    let amended = reader
        .sole_line(b"8=FIX.4.4|35=D|34=7|52=20240102-10:15:30.000|11=A|55=AAPL|38=100|10=0|")
        .unwrap();
    assert_ne!(
        reader.sole_line(original.as_bytes()).unwrap().digest(),
        amended.digest()
    );
}

#[test]
fn the_crate_carries_fields_of_its_own_from_65000() {
    let held = yggdryl::fix_crate_fields().expect("the crate's own fields");
    let names: Vec<&str> = held.iter().map(yggdryl::Field::name).collect();
    assert_eq!(
        names,
        [
            "currunix",
            "msgctxid",
            "pluginid",
            "isincode",
            "miccode",
            "state",
            "currhashcode",
            "crosshashcode",
            "identifiers",
            "prevunix",
            "prevuuid",
            "creatunix",
            "snapunix",
            "sourceurl",
            "nofixentries",
            "recordedat",
            "expirunix",
            "bidcurrency",
            "askcurrency",
            "msgsessionid",
            "bloombergcode",
            "cusipcode",
            "sedolcode",
            "curruuid",
            "crossuuid",
            "parentuuids",
            "seqnum",
            "px",
            "qty",
            "unit",
            "bidunit",
            "askunit",
            "crosscode",
            "metadata",
            "prevpx",
            "prevqty",
            "tradable",
            "symbolticker",
        ],
    );
    let displays: Vec<Option<&str>> = held.iter().map(yggdryl::Field::display).collect();
    assert_eq!(
        displays,
        [
            Some("CurrUnix"),
            Some("MsgCtxId"),
            Some("PluginId"),
            Some("ISINCode"),
            Some("MICCode"),
            Some("State"),
            Some("CurrHashCode"),
            Some("CrossHashCode"),
            Some("Identifiers"),
            Some("PrevUnix"),
            Some("PrevUuid"),
            Some("CreatUnix"),
            Some("SnapUnix"),
            Some("SourceUrl"),
            Some("NoFixEntries"),
            Some("RecordedAt"),
            Some("ExpirUnix"),
            Some("BidCurrency"),
            Some("AskCurrency"),
            Some("MsgSessionId"),
            Some("BloombergCode"),
            Some("CUSIPCode"),
            Some("SEDOLCode"),
            Some("CurrUuid"),
            Some("CrossUuid"),
            Some("ParentUuids"),
            Some("SeqNum"),
            Some("Px"),
            Some("Qty"),
            Some("Unit"),
            Some("BidUnit"),
            Some("AskUnit"),
            Some("CrossCode"),
            Some("Metadata"),
            Some("PrevPx"),
            Some("PrevQty"),
            Some("Tradable"),
            Some("SymbolTicker"),
        ],
    );

    // The columns a message answers from what it said are typed as the thing
    // they hold, not as the text a venue spelled it in: the codes are
    // sixty-four-bit digests, the identities UUIDs, the clocks one instant.
    let field = |name: &str| {
        held.iter()
            .find(|field| field.name() == name)
            .unwrap_or_else(|| panic!("{name}"))
    };
    let typed = |name: &str| field(name).dtype();
    let clock = DataType::DateTime64 {
        unit: yggdryl::TimeUnit::Nanosecond,
        timezone: yggdryl::Timezone::UTC,
    };
    assert_eq!(typed("isincode"), &DataType::Isin);
    assert_eq!(typed("miccode"), &DataType::Mic);
    assert_eq!(typed("state"), &DataType::State);
    for name in ["currhashcode", "crosshashcode", "seqnum"] {
        assert_eq!(typed(name), &DataType::UInt64, "{name}");
    }
    for name in ["curruuid", "crossuuid", "prevuuid"] {
        assert_eq!(typed(name), &DataType::Uuid, "{name}");
        assert_eq!(field(name).as_fix().names().count(), 0, "{name}");
    }
    for name in ["currunix", "creatunix"] {
        assert_eq!(typed(name), &clock, "{name}");
        assert!(!field(name).is_nullable(), "{name}");
    }
    // The clocks only a walk fills - the predecessor's instant and the grid
    // instant a snapshot was read as - are null on every row that is not one.
    for name in ["prevunix", "snapunix", "expirunix", "recordedat"] {
        assert_eq!(typed(name), &clock, "{name}");
        assert!(field(name).is_nullable(), "{name}");
    }
    for name in ["currhashcode", "crosshashcode", "curruuid", "crossuuid"] {
        assert!(!field(name).is_nullable(), "{name}");
    }
    assert_eq!(typed("crosscode"), &DataType::utf8());
    assert!(field("crosscode").is_nullable());
    // Where a line was read from is the URL it is, so a row joins on it and
    // a reader resolves it rather than parsing text back into one.
    assert_eq!(typed(yggdryl::SOURCEURL_TAG_NAME.1), &DataType::Url);
    assert!(field(yggdryl::SOURCEURL_TAG_NAME.1).is_nullable());
    // The arrival record is a group, so it has a counter like any other.
    assert_eq!(typed(yggdryl::NOFIXENTRIES_TAG_NAME.1), &DataType::Int32);
    assert!(field(yggdryl::NOFIXENTRIES_TAG_NAME.1).is_nullable());
    // The names a message goes by and what a bridge stated under its own
    // namespaces are the two Map groups, each its own counter.
    for (tag, name) in [yggdryl::IDENTIFIERS_TAG_NAME, yggdryl::METADATA_TAG_NAME] {
        assert_eq!(
            typed(name),
            &DataType::map_of(DataType::utf8(), DataType::utf8(), true).unwrap()
        );
        assert_eq!(field(name).as_fix().counter().unwrap(), Some(tag));
    }
    // No partition column: how a layout is cut is the target's - an Iceberg
    // table takes an `hour` transform over `currunix` - and a materialized copy
    // of that instant was a second owner of it.
    assert!(held.iter().all(|field| !field.is_partition()));
    assert!(held.iter().all(|field| field.name() != "timepartition"));

    // Every definition has a tag from 65000 up: one block, in the one namespace
    // every dictionary resolves through, so a bridge row spelling `PLUGINID`
    // reaches it by name; its identity is its tag and its name, and a
    // dictionary member it is not. Strictly increasing rather than
    // contiguous: a retired slot is never reused, so the block has holes
    // where one was.
    let tags: Vec<i32> = held
        .iter()
        .filter_map(|field| field.as_fix().tag().ok().flatten())
        .collect();
    for retired in [65_000, 65_004, 65_016, 65_019, 65_024, 65_036] {
        assert!(!tags.contains(&retired), "{retired} stays retired");
    }
    let mut last = yggdryl::CRATE_TAG_MIN;
    for field in held {
        let view = field.as_fix();
        let tag = view.tag().unwrap().expect("a tag");
        let id = view.id().unwrap().expect("an identity");
        assert!(tag > last, "{tag} follows {last}");
        assert!(tag < yggdryl::CRATE_TAG_MAX, "{tag} is in the block");
        last = tag;
        assert_eq!(id, FixId::of(tag, field.name()).unwrap(), "tag and name");
        assert!(yggdryl::is_crate_tag(tag));
        assert_eq!(
            view.branches().count(),
            0,
            "{} is no dictionary's",
            field.name()
        );
    }
    assert_eq!(yggdryl::CRATE_TAG_MIN, 65_000);
    assert_eq!(yggdryl::STATE_TAG_NAME.0, 65_015);
    assert_eq!(
        [
            yggdryl::CURRHASHCODE_TAG_NAME,
            yggdryl::CROSSHASHCODE_TAG_NAME
        ],
        [(65_017, "currhashcode"), (65_018, "crosshashcode")]
    );
    assert_eq!(
        [yggdryl::PREVUNIX_TAG_NAME, yggdryl::PREVUUID_TAG_NAME],
        [(65_021, "prevunix"), (65_022, "prevuuid")]
    );
    assert_eq!(
        [
            yggdryl::CROSSCODE_TAG_NAME,
            yggdryl::METADATA_TAG_NAME,
            yggdryl::FIXMSG_TAG_NAME
        ],
        [
            (65_048, "crosscode"),
            (65_049, "metadata"),
            (65_050, "fixmsg")
        ]
    );
    // The fixed row's own name is a tag of the block and not a field of it:
    // a store dumps the row under it, and nothing reads it back.
    assert!(!tags.contains(&yggdryl::FIXMSG_TAG_NAME.0));
    assert!(!yggdryl::is_crate_tag(yggdryl::CRATE_TAG_MIN - 1));
    assert!(!yggdryl::is_crate_tag(yggdryl::CRATE_TAG_MAX));

    // `MsgDirection` is FIX's own, so it is not invented here.
    assert!(!names.contains(&"msgdirection"));
    assert_eq!(yggdryl::MSGDIRECTION_TAG_NAME.0, 385);

    // The spellings the columns replaced are gone, in every registry.
    let registry = FixRegistry::new();
    for retired in [
        "version",
        "updatedat",
        "createdat",
        "msghash",
        "msgphash",
        "altids",
        "instids",
        "expiredat",
        "offercurrency",
        "snapshotat",
        "prevupdatedat",
        "prevmsghash",
        "code",
        "bridgesessionid",
        "sendersessionid",
    ] {
        assert!(registry.get_field_by_name(retired).is_none(), "{retired}");
    }
}

/// Every registry holds the crate's own fields from construction, and
/// inserting one again replaces rather than collides.
///
/// A definition is filed by the shape it has: a Map is a group and
/// everything else is a scalar field, and the two Map groups are reached by
/// the counter that is their own tag.
#[test]
fn every_registry_holds_the_crates_fields_and_takes_them_again() {
    let held = yggdryl::fix_crate_fields().expect("the crate's own fields");
    let mut registry = FixRegistry::new();
    let registered = held
        .iter()
        .filter(|field| registry.get_field_by_name(field.name()).is_some())
        .count();
    // The two standard clock seeds stand beside the crate's own.
    assert_eq!(registry.len(), registered + 2);
    for field in held {
        let Some(known) = registry.get_field_by_name(field.name()) else {
            continue;
        };
        assert_eq!(known, field);
        registry.insert(field.clone()).unwrap();
    }
    assert_eq!(
        registry.len(),
        registered + 2,
        "standard clock seeds remain"
    );
    for (tag, name) in [yggdryl::IDENTIFIERS_TAG_NAME, yggdryl::METADATA_TAG_NAME] {
        let map = registry.get_field_by_counter(tag).unwrap();
        assert_eq!(map.name(), name);
        assert_eq!(
            map.dtype(),
            &DataType::map_of(DataType::utf8(), DataType::utf8(), true).unwrap()
        );
        assert_eq!(registry.get_field_by_name(name), Some(map));
        // A Map group is its own counter: no scalar stands beside it.
        assert!(registry.get_field_by_tag(tag).is_none());
    }
}

/// A fresh registry registers every one of the crate's own fields, and
/// says nothing while doing it: a column of the fixed row that no registry
/// holds is a column no dictionary can type.
#[test]
fn every_registry_registers_the_crates_fields_without_warning() {
    let held = yggdryl::fix_crate_fields().expect("the crate's own fields");
    let (registry, warnings) = super::warned::during_all(FixRegistry::new);
    assert!(warnings.is_empty(), "builtin registration: {warnings:?}");
    for field in held {
        assert!(
            registry.get_field_by_name(field.name()).is_some(),
            "{} is every registry's",
            field.name()
        );
    }
}

/// Where a line was read from is not what the message says.
///
/// A capture is re-cut, replayed and copied, and the same message comes back
/// out of a different object every time. Its code is the message's content,
/// so it must not move when only the object does - otherwise a replay
/// deduplicates against nothing and every archived day re-enters a table as
/// new rows.
///
/// The object is not a fact a message can hold at all: writing one is
/// refused, the row a message writes states none, and a row a reader stated
/// one on reads back as the same message. So the code cannot move, rather
/// than being kept from moving.
#[test]
fn the_object_a_line_was_read_from_is_not_part_of_the_message() {
    let registry = super::committed_registry();
    let reader = super::fixed_codec(std::sync::Arc::clone(&registry));
    let schema = yggdryl::fix_schema(&registry, "fix").unwrap();
    let line = b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|10=0|";

    let at =
        yggdryl::fix_column_of(&schema, yggdryl::SOURCEURL_TAG_NAME.0).expect("a sourceurl column");
    let hashcode_at = yggdryl::fix_column_of(&schema, yggdryl::CURRHASHCODE_TAG_NAME.0)
        .expect("a hashcode column");
    let mut identities = Vec::new();
    for url in [
        "file:///capture/2026-08-14/part-0.txt.gz",
        "s3://replay/2026-08-14/part-0.txt.gz",
    ] {
        let stated = yggdryl::Scalar::from(yggdryl::Url::from_str(url).unwrap());
        let mut message = reader.sole_line(line).unwrap();
        // The message will not hold it: the column is the reader's.
        assert!(
            message
                .set(yggdryl::SOURCEURL_TAG_NAME.0, stated.clone())
                .is_err(),
            "a message states no source object"
        );
        let row = message.into_row(&schema).unwrap();
        let mut held = row.as_sequence().expect("a row").to_vec();
        assert!(held[at].is_null(), "and its row states none either");
        // As a reader states it, on the row.
        held[at] = stated.clone();
        let carried = yggdryl::Scalar::from_sequence(held.clone());
        let again =
            yggdryl::FixMsg::from_row(std::sync::Arc::clone(&registry), &schema, &carried).unwrap();
        assert_eq!(
            held[hashcode_at].as_u64(),
            Some(yggdryl::graph::Element::get_currhashcode(&again)),
            "the row read back is the message that wrote it",
        );
        identities.push(held[hashcode_at].clone());
    }
    assert_eq!(
        identities[0], identities[1],
        "one message read from two objects is one message",
    );
}
