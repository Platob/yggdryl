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
        .sole_line(b"8=FIX.4.4|35=D|11=A|55=AAPL|10=0|", false)
        .unwrap();
    let same = reader
        .sole_line(b"8=FIX.4.4|35=D|11=A|55=AAPL|10=0|", false)
        .unwrap();
    assert_eq!(one.digest(), same.digest());
    assert_eq!(one.stable_hash(), same.stable_hash());
    assert_eq!(one.stable_hash(), one.clone().stable_hash());

    // Order carries meaning inside a repeating group, so it is never sorted
    // away: the same pairs in another order are another message.
    let reordered = reader
        .sole_line(b"8=FIX.4.4|35=D|55=AAPL|11=A|10=0|", false)
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
        .sole_line(b"8=FIX.4.4|35=D|9999=1|9998=23|10=0|", false)
        .unwrap();
    let other = reader
        .sole_line(b"8=FIX.4.4|35=D|9999=12|9998=3|10=0|", false)
        .unwrap();
    assert_ne!(split.digest(), other.digest());
}

#[test]
fn the_envelope_is_not_the_message() {
    let reader = reader();
    let row = "8=FIX.4.4|9=64|35=D|11=A|55=AAPL|10=203|";
    let one = reader.sole_line(row.as_bytes(), false).unwrap();

    // A recomputed body length and a different checksum describe how the
    // message was written down, not what it says.
    let rewritten = reader
        .sole_line(b"8=FIX.4.4|9=999|35=D|11=A|55=AAPL|10=000|", false)
        .unwrap();
    assert_eq!(one.digest(), rewritten.digest());

    // The same message re-serialized with another separator reads back equal.
    let soh = one.into_bytes(0x01);
    let again = reader.parse_fix_line(&soh).unwrap();
    assert_eq!(one.digest(), again.digest());

    // The session layer is not the message either: the same order sent a
    // second later, over another session, on a redelivery, is one message.
    let relayed = reader
        .sole_line(b"8=FIX.4.4|35=D|34=91|49=DESK|56=VENUE|52=20240102-10:15:31.000|43=Y|11=A|55=AAPL|10=0|", false)
        .unwrap();
    let original = reader
        .sole_line(
            b"8=FIX.4.4|35=D|34=7|49=OTHER|56=ELSEWHERE|52=20240102-10:15:30.000|11=A|55=AAPL|10=0|",
        false)
        .unwrap();
    assert_eq!(relayed.digest(), original.digest());

    // The whole standard header is the envelope rather than a curated part of
    // it: the application version, the encoding and the last sequence number
    // processed all describe how to read this delivery, not what it says.
    let annotated = reader
        .sole_line(
            b"8=FIX.4.4|35=D|1128=9|1129=X|1156=1|347=UTF-8|369=6|11=A|55=AAPL|10=0|",
            false,
        )
        .unwrap();
    assert_eq!(original.digest(), annotated.digest());

    // What the message says still separates it, and so does what it is.
    let other = reader
        .sole_line(b"8=FIX.4.4|35=D|11=A|55=MSFT|10=0|", false)
        .unwrap();
    assert_ne!(original.digest(), other.digest());
    let typed = reader
        .sole_line(b"8=FIX.4.4|35=F|11=A|55=AAPL|10=0|", false)
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
    let one = reader.sole_line(b"35=D|VenueOwnThing=x", false).unwrap();
    let other = reader.sole_line(b"35=D|OtherVenueThing=x", false).unwrap();
    assert_ne!(one.digest(), other.digest());

    // A message that recorded no arrival digests as the empty walk - correct,
    // because none of them arrived - whichever door built it: pairs a caller
    // handed in stating nothing, and a document stating no attribute. A line
    // that states no message states none at all now (decision 16), so it is
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
            .map(|row| reader.sole_line(row.as_bytes(), false).unwrap()),
    );
    let kept: Vec<String> = dedup
        .by_ref()
        .map(|held| held.into_text('|').unwrap())
        .collect();

    // Adjacent only: the `A` that returns after `B` is a third event.
    assert_eq!(kept.len(), 3);
    assert_eq!(kept[0], rows[0]);
    assert_eq!(kept[1], rows[3]);
    assert_eq!(kept[2], rows[4]);
    assert_eq!(dedup.dropped(), 2);

    // Nothing to drop drops nothing, and the count says so.
    let mut clean = FixDedup::new(
        ["8=FIX.4.4|35=D|11=A|10=0|", "8=FIX.4.4|35=D|11=B|10=0|"]
            .iter()
            .map(|row| reader.sole_line(row.as_bytes(), false).unwrap()),
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
            .map(|row| reader.sole_line(row.as_bytes(), false).unwrap()),
    );
    assert_eq!(
        dedup.by_ref().count(),
        1,
        "one order, however often it arrives"
    );
    assert_eq!(dedup.dropped(), 2);

    // That a delivery *was* a replay is still readable, from the columns the
    // digest declined to fold in.
    let held = reader.sole_line(resend.as_bytes(), false).unwrap();
    assert!(held.lifted("resent").is_some());
    assert_eq!(held.lifted("seqnum"), Some(&Scalar::from(8_i32)));

    // A genuinely different order is a different order.
    let amended = reader
        .sole_line(
            b"8=FIX.4.4|35=D|34=7|52=20240102-10:15:30.000|11=A|55=AAPL|38=100|10=0|",
            false,
        )
        .unwrap();
    assert_ne!(
        reader
            .sole_line(original.as_bytes(), false)
            .unwrap()
            .digest(),
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
            "version",
            "symbolticker",
            "updatedat",
            "parentclordid",
            "parentorderid",
            "sendersessionid",
            "msgctxid",
            "pluginid",
            "prevpluginid",
            "sendersessionname",
            "targetsessionname",
            "isincode",
            "miccode",
            "state",
            "msghash",
            "msgphash",
            "targetsessionid",
            "altids",
            "prevupdatedat",
            "prevmsghash",
            "createdat",
            "code",
            "snapshotat",
            "sourceurl",
            "nofixentries",
            "recordedat",
            "expiredat",
            "bidcurrency",
            "offercurrency",
            "bridgesessionid",
            "bloombergcode",
            "cusipcode",
            "sedolcode",
            "instids",
            "sessionmsgid",
            "sessionmsgseqid",
        ],
    );
    let displays: Vec<Option<&str>> = held.iter().map(yggdryl::Field::display).collect();
    assert_eq!(
        displays,
        [
            Some("Version"),
            Some("SymbolTicker"),
            Some("UpdatedAt"),
            Some("ParentClOrdID"),
            Some("ParentOrderID"),
            Some("SenderSessionId"),
            Some("MsgCtxId"),
            Some("PluginId"),
            Some("PrevPluginId"),
            Some("SenderSessionName"),
            Some("TargetSessionName"),
            Some("ISINCode"),
            Some("MICCode"),
            Some("State"),
            Some("MsgHash"),
            Some("MsgPHash"),
            Some("TargetSessionId"),
            Some("AltIds"),
            Some("PrevUpdatedAt"),
            Some("PrevMsgHash"),
            Some("CreatedAt"),
            Some("Code"),
            Some("SnapshotAt"),
            Some("SourceUrl"),
            Some("NoFixEntries"),
            Some("RecordedAt"),
            Some("ExpiredAt"),
            Some("BidCurrency"),
            Some("OfferCurrency"),
            Some("BridgeSessionId"),
            Some("BloombergCode"),
            Some("CUSIPCode"),
            Some("SEDOLCode"),
            Some("InstIds"),
            Some("SessionMsgId"),
            Some("SessionMsgSeqId"),
        ],
    );

    // The columns a message answers from what it said are typed as the thing
    // they hold, not as the text a venue spelled it in; the three identity
    // columns are sixteen fixed bytes, which is what a lake engine reads, and
    // `msghash` opens the two that sit together.
    let typed = |name: &str| {
        held.iter()
            .find(|field| field.name() == name)
            .unwrap_or_else(|| panic!("{name}"))
            .dtype()
    };
    assert_eq!(typed("isincode"), &DataType::Isin);
    assert_eq!(typed("miccode"), &DataType::Mic);
    assert_eq!(typed("state"), &DataType::State);
    let identities = held
        .iter()
        .position(|field| field.name() == "msghash")
        .expect("msghash");
    for identity in &held[identities..identities + 2] {
        assert_eq!(identity.dtype(), &super::identity_dtype());
        assert_eq!(identity.as_fix().aliases().count(), 0);
    }
    assert_eq!(typed("prevupdatedat"), typed("updatedat"));
    assert_eq!(
        typed("prevupdatedat"),
        &DataType::DateTime64 {
            unit: yggdryl::TimeUnit::Nanosecond,
            timezone: yggdryl::Timezone::UTC,
        }
    );
    assert_eq!(typed("prevmsghash"), &super::identity_dtype());
    let previous_at = held
        .iter()
        .position(|field| field.name() == "prevupdatedat")
        .expect("prevupdatedat");
    for previous in &held[previous_at..previous_at + 2] {
        assert!(previous.is_nullable());
        assert_eq!(previous.as_fix().aliases().count(), 0);
    }
    let field = |name: &str| {
        held.iter()
            .find(|field| field.name() == name)
            .unwrap_or_else(|| panic!("{name}"))
    };
    for name in ["updatedat", "createdat"] {
        assert_eq!(typed(name), typed("prevupdatedat"), "{name}");
        assert!(!field(name).is_nullable(), "{name}");
    }
    // A snapshot's clock is the one thing only a snapshot has, so it carries
    // the same instant as the others and is null on every row that is not one.
    assert_eq!(typed("snapshotat"), typed("prevupdatedat"));
    assert!(field("snapshotat").is_nullable());
    assert_eq!(typed("code"), &DataType::utf8());
    assert!(!field("code").is_nullable());
    // Where a line was read from is the URL it is, so a row joins on it and
    // a reader resolves it rather than parsing text back into one.
    assert_eq!(typed(yggdryl::SOURCEURL_TAG_NAME.1), &DataType::Url);
    assert!(field(yggdryl::SOURCEURL_TAG_NAME.1).is_nullable());
    // The arrival record is a group, so it has a counter like any other.
    assert_eq!(typed(yggdryl::NOFIXENTRIES_TAG_NAME.1), &DataType::Int32);
    assert!(field(yggdryl::NOFIXENTRIES_TAG_NAME.1).is_nullable());
    // No partition column: how a layout is cut is the target's - an Iceberg
    // table takes an `hour` transform over `updatedat` - and a materialized
    // copy of that instant was a second owner of it.
    assert!(held.iter().all(|field| !field.is_partition()));
    assert!(held.iter().all(|field| field.name() != "timepartition"));
    assert_eq!(held[3].get_metadata("iceberg:transform"), None);
    assert_eq!(held[3].get_metadata("partition:transform"), None);

    // Every definition has a tag from 65000 up: one block, in the one namespace
    // every dictionary resolves through, so a bridge row spelling `SESSIONID`
    // or `ULFROMSESSIONNAME` reaches it by name; its identity is its tag and
    // its name, and a dictionary member it is not.
    // Strictly increasing rather than contiguous: a retired slot is never
    // reused, so the block has holes where one was. 65000 held the original
    // `msghash` (decision 26), 65004 held `timepartition`, which went when
    // how a layout is cut became the target's, and 65016 held `instuuid`,
    // which went when the instrument became a scope rather than a column.
    let tags: Vec<i32> = held
        .iter()
        .filter_map(|field| field.as_fix().tag().ok().flatten())
        .collect();
    for retired in [65_000, 65_004, 65_016] {
        assert!(!tags.contains(&retired), "{retired} stays retired");
    }
    let mut last = yggdryl::CRATE_TAG_MIN;
    for field in held {
        let view = field.as_fix();
        let tag = view.tag().unwrap().expect("a tag");
        let id = view.id().unwrap().expect("an identity");
        assert!(tag > last, "{tag} follows {last}");
        assert!(tag <= yggdryl::CRATE_TAG_MAX, "{tag} is in the block");
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
    assert!(
        !held
            .iter()
            .any(|field| field.as_fix().tag().unwrap() == Some(65_000))
    );
    assert_eq!(yggdryl::STATE_TAG_NAME.0, 65_015);
    assert_eq!(
        [yggdryl::MSGHASH_TAG_NAME, yggdryl::MSGPHASH_TAG_NAME],
        [(65_017, "msghash"), (65_018, "msgphash")]
    );
    assert_eq!(
        [
            yggdryl::PREVUPDATEDAT_TAG_NAME,
            yggdryl::PREVMSGHASH_TAG_NAME
        ],
        [(65_021, "prevupdatedat"), (65_022, "prevmsghash")]
    );
    assert!(!yggdryl::is_crate_tag(yggdryl::CRATE_TAG_MIN - 1));
    assert_eq!(
        ["sendersessionname", "targetsessionname"]
            .map(|name| field(name).as_fix().aliases().collect::<Vec<_>>()),
        [vec!["ULFromSessionName"], vec!["ULToSessionName"]]
    );

    // `MsgDirection` is FIX's own, so it is not invented here.
    assert!(!names.contains(&"msgdirection"));
    assert_eq!(yggdryl::MSGDIRECTION_TAG_NAME.0, 385);

    // Every registry holds them from construction, and inserting them again
    // replaces rather than collides.
    let scalar_count = held
        .iter()
        .filter(|field| !field.dtype().is_nested())
        .count();
    assert_eq!(held.len(), 36);
    assert_eq!(scalar_count, 34);
    let (mut registry, warnings) = super::warned::during(FixRegistry::new);
    assert!(warnings.is_empty(), "builtin registration: {warnings:?}");
    assert_eq!(registry.len(), scalar_count + 2);
    // `msghash` is a live name again - on 65017, not on the 65000 decision 26
    // retired and this crate still does not reuse - and the spellings it
    // replaced are the retired ones now.
    for retired in [
        "instid",
        "id",
        "persistentid",
        "timestamp",
        "uuid",
        "puuid",
        "prevuuid",
        "instuuid",
    ] {
        assert!(registry.get_field_by_name(retired).is_none(), "{retired}");
    }
    // A definition is filed by the shape it has: a Map is a group, a Struct
    // is a component, and everything else is a scalar field.
    for field in held {
        let category = match field.dtype() {
            DataType::Struct(_) => yggdryl::FixCategory::Components,
            dtype if dtype.is_nested() => yggdryl::FixCategory::Groups,
            _ => yggdryl::FixCategory::Fields,
        };
        assert_eq!(registry.definition(category, field.name()).unwrap(), field);
        registry.insert_definition(category, field.clone()).unwrap();
    }
    assert_eq!(
        registry.len(),
        scalar_count + 2,
        "standard clock seeds remain"
    );
    let map = registry
        .get_group_by_tag(yggdryl::ALTIDS_TAG_NAME.0)
        .unwrap();
    assert_eq!(map.name(), yggdryl::ALTIDS_TAG_NAME.1);
    assert_eq!(
        map.dtype(),
        &DataType::map_of(DataType::utf8(), DataType::utf8(), true).unwrap()
    );
    assert!(
        registry
            .get_field_by_tag(yggdryl::ALTIDS_TAG_NAME.0)
            .is_none()
    );
}

/// Where a line was read from is not what the message says.
///
/// A capture is re-cut, replayed and copied, and the same message comes back
/// out of a different object every time. Its sixteen identity bytes are the
/// message's content, so they must not move when only the object does -
/// otherwise a replay deduplicates against nothing and every archived day
/// re-enters a table as new rows.
#[test]
fn the_object_a_line_was_read_from_is_not_part_of_the_message() {
    let registry = super::committed_registry();
    let reader = super::fixed_codec(std::sync::Arc::clone(&registry));
    let schema = yggdryl::fix_schema(&registry, "fix").unwrap();
    let line = b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|10=0|";

    let at =
        yggdryl::fix_column_of(&schema, yggdryl::SOURCEURL_TAG_NAME.0).expect("a sourceurl column");
    let msghash_at =
        yggdryl::fix_column_of(&schema, yggdryl::MSGHASH_TAG_NAME.0).expect("a msghash column");
    let mut identities = Vec::new();
    for url in [
        "file:///capture/2026-08-14/part-0.txt.gz",
        "s3://replay/2026-08-14/part-0.txt.gz",
    ] {
        let stated = yggdryl::Scalar::from(yggdryl::Url::from_str(url).unwrap());
        let mut message = reader.sole_line(line, false).unwrap();
        message
            .set(yggdryl::SOURCEURL_TAG_NAME.0, stated.clone())
            .unwrap();
        let row = message.into_row(&schema).unwrap();
        let held = row.as_sequence().expect("a row");
        assert_eq!(held[at], stated, "the column still states it");
        identities.push(held[msghash_at].clone());
    }
    assert_eq!(
        identities[0], identities[1],
        "one message read from two objects is one message",
    );
}
