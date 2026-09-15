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
            "timepartition",
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
            "instuuid",
            "uuid",
            "puuid",
            "targetsessionid",
            "altids",
            "prevupdatedat",
            "prevuuid",
            "createdat",
            "code",
            "snapshotat",
            "sourceurl",
            "nofixentries",
        ],
    );
    let displays: Vec<Option<&str>> = held.iter().map(yggdryl::Field::display).collect();
    assert_eq!(
        displays,
        [
            Some("Version"),
            Some("SymbolTicker"),
            Some("UpdatedAt"),
            Some("TimePartition"),
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
            Some("InstUuid"),
            Some("Uuid"),
            Some("PUuid"),
            Some("TargetSessionId"),
            Some("AltIds"),
            Some("PrevUpdatedAt"),
            Some("PrevUuid"),
            Some("CreatedAt"),
            Some("Code"),
            Some("SnapshotAt"),
            Some("SourceUrl"),
            Some("NoFixEntries"),
        ],
    );

    // The columns a message answers from what it said are typed as the thing
    // they hold, not as the text a venue spelled it in; the three lifecycle
    // identities are sixteen fixed bytes, which is what a lake engine reads.
    assert_eq!(held[12].dtype(), &DataType::Isin);
    assert_eq!(held[13].dtype(), &DataType::Mic);
    assert_eq!(held[14].dtype(), &DataType::State);
    for identity in &held[15..18] {
        assert_eq!(identity.dtype(), &super::identity_dtype());
        assert_eq!(identity.as_fix().aliases().count(), 0);
    }
    assert_eq!(held[20].dtype(), held[2].dtype());
    assert_eq!(
        held[20].dtype(),
        &DataType::DateTime64 {
            unit: yggdryl::TimeUnit::Nanosecond,
            timezone: yggdryl::Timezone::UTC,
        }
    );
    assert_eq!(held[21].dtype(), &super::identity_dtype());
    for previous in &held[20..22] {
        assert!(previous.is_nullable());
        assert_eq!(previous.as_fix().aliases().count(), 0);
    }
    for at in [2, 22, 24] {
        assert_eq!(held[at].dtype(), held[20].dtype());
        assert!(!held[at].is_nullable());
    }
    assert_eq!(held[23].dtype(), &DataType::utf8());
    assert!(!held[23].is_nullable());
    // Where a line was read from is the URL it is, so a row joins on it and
    // a reader resolves it rather than parsing text back into one.
    assert_eq!(held[25].name(), yggdryl::SOURCEURL_TAG_NAME.1);
    assert_eq!(held[25].dtype(), &DataType::Url);
    assert!(held[25].is_nullable());
    // The arrival record is a group, so it has a counter like any other.
    assert_eq!(held[26].name(), yggdryl::NOFIXENTRIES_TAG_NAME.1);
    assert_eq!(held[26].dtype(), &DataType::Int32);
    assert!(held[26].is_nullable());
    // The partition is the hour `updatedat` falls in, typed as that clock
    // is; it is marked as the column a layout is cut on, names the clock it
    // reads, and declares its derivation in the expression layer's own
    // vocabulary rather than in an Iceberg transform of its own.
    assert_eq!(held[3].dtype(), held[2].dtype());
    assert!(held[3].is_partition());
    assert_eq!(
        held[3].as_partition().sources().unwrap(),
        Some(vec!["updatedat".to_owned()])
    );
    assert_eq!(
        held[3].get_metadata("transform:expression"),
        Some("truncate(updatedat, 'hour')")
    );
    assert_eq!(
        held[3]
            .as_transform()
            .term()
            .unwrap()
            .map(|term| term.to_string()),
        Some("truncate(updatedat, 'hour')".to_owned())
    );
    assert_eq!(held[3].get_metadata("iceberg:transform"), None);
    assert_eq!(held[3].get_metadata("partition:transform"), None);

    // Every definition has a tag from 65000 up: one block, in the one namespace
    // every dictionary resolves through, so a bridge row spelling `SESSIONID`
    // or `ULFROMSESSIONNAME` reaches it by name; its identity is its tag and
    // its name, and a dictionary member it is not.
    for (at, field) in held.iter().enumerate() {
        let view = field.as_fix();
        let tag = view.tag().unwrap().expect("a tag");
        let id = view.id().unwrap().expect("an identity");
        assert_eq!(tag, yggdryl::CRATE_TAG_MIN + 1 + i32::try_from(at).unwrap());
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
        [
            yggdryl::INSTUUID_TAG_NAME,
            yggdryl::UUID_TAG_NAME,
            yggdryl::PUUID_TAG_NAME,
        ],
        [(65_016, "instuuid"), (65_017, "uuid"), (65_018, "puuid")]
    );
    assert_eq!(
        [yggdryl::PREVUPDATEDAT_TAG_NAME, yggdryl::PREVUUID_TAG_NAME],
        [(65_021, "prevupdatedat"), (65_022, "prevuuid")]
    );
    assert!(!yggdryl::is_crate_tag(yggdryl::CRATE_TAG_MIN - 1));
    let sessions = &held[10..12];
    assert_eq!(
        sessions
            .iter()
            .map(|field| field.as_fix().aliases().collect::<Vec<_>>())
            .collect::<Vec<_>>(),
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
    assert_eq!(held.len(), 27);
    assert_eq!(scalar_count, 26);
    let (mut registry, warnings) = super::warned::during(FixRegistry::new);
    assert!(warnings.is_empty(), "builtin registration: {warnings:?}");
    assert_eq!(registry.len(), scalar_count + 2);
    for retired in ["instid", "id", "persistentid", "timestamp", "msghash"] {
        assert!(registry.get_field_by_name(retired).is_none(), "{retired}");
    }
    for field in held {
        let category = if field.dtype().is_nested() {
            yggdryl::FixCategory::Groups
        } else {
            yggdryl::FixCategory::Fields
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
    let uuid_at = yggdryl::fix_column_of(&schema, yggdryl::UUID_TAG_NAME.0).expect("a uuid column");
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
        identities.push(held[uuid_at].clone());
    }
    assert_eq!(
        identities[0], identities[1],
        "one message read from two objects is one message",
    );
}
