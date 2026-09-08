//! The value digest, the dedup adapter, and the crate's own two fields.

use std::path::PathBuf;
use std::sync::Arc;

use yggdryl::holder::local::Folder;
use yggdryl::types::MsgDirection;
use yggdryl::{DataType, FixCodec, FixDedup, FixId, FixRegistry, Scalar};

fn reader() -> FixCodec {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("config")
        .join("fix");
    let folder = Folder::new(root).expect("the seed folder is a local path");
    FixCodec::new(Arc::new(
        FixRegistry::from_handle(&folder).expect("the committed dictionary loads"),
    ))
}

#[test]
fn identical_entries_hash_equal_and_a_different_order_does_not() {
    let reader = reader();
    let one = reader
        .transform_line(b"8=FIX.4.4|35=D|11=A|55=AAPL|10=0|", false)
        .unwrap();
    let same = reader
        .transform_line(b"8=FIX.4.4|35=D|11=A|55=AAPL|10=0|", false)
        .unwrap();
    assert_eq!(one.digest(), same.digest());

    // Order carries meaning inside a repeating group, so it is never sorted
    // away: the same pairs in another order are another message.
    let reordered = reader
        .transform_line(b"8=FIX.4.4|35=D|55=AAPL|11=A|10=0|", false)
        .unwrap();
    assert_ne!(one.digest(), reordered.digest());

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
        .transform_line(b"8=FIX.4.4|35=D|9999=1|9998=23|10=0|", false)
        .unwrap();
    let other = reader
        .transform_line(b"8=FIX.4.4|35=D|9999=12|9998=3|10=0|", false)
        .unwrap();
    assert_ne!(split.digest(), other.digest());
}

#[test]
fn the_envelope_is_not_the_message() {
    let reader = reader();
    let row = "8=FIX.4.4|9=64|35=D|11=A|55=AAPL|10=203|";
    let one = reader.transform_line(row.as_bytes(), false).unwrap();

    // A recomputed body length and a different checksum describe how the
    // message was written down, not what it says.
    let rewritten = reader
        .transform_line(b"8=FIX.4.4|9=999|35=D|11=A|55=AAPL|10=000|", false)
        .unwrap();
    assert_eq!(one.digest(), rewritten.digest());

    // The same message re-serialized with another separator reads back equal.
    let soh = one.into_bytes(0x01);
    let again = reader.transform_fix_line(&soh, false).unwrap();
    assert_eq!(one.digest(), again.digest());

    // The session layer is not the message either: the same order sent a
    // second later, over another session, on a redelivery, is one message.
    let relayed = reader
        .transform_line(b"8=FIX.4.4|35=D|34=91|49=DESK|56=VENUE|52=20240102-10:15:31.000|43=Y|11=A|55=AAPL|10=0|", false)
        .unwrap();
    let original = reader
        .transform_line(
            b"8=FIX.4.4|35=D|34=7|49=OTHER|56=ELSEWHERE|52=20240102-10:15:30.000|11=A|55=AAPL|10=0|",
        false)
        .unwrap();
    assert_eq!(relayed.digest(), original.digest());

    // The whole standard header is the envelope rather than a curated part of
    // it: the application version, the encoding and the last sequence number
    // processed all describe how to read this delivery, not what it says.
    let annotated = reader
        .transform_line(
            b"8=FIX.4.4|35=D|1128=9|1129=X|1156=1|347=UTF-8|369=6|11=A|55=AAPL|10=0|",
            false,
        )
        .unwrap();
    assert_eq!(original.digest(), annotated.digest());

    // What the message says still separates it, and so does what it is.
    let other = reader
        .transform_line(b"8=FIX.4.4|35=D|11=A|55=MSFT|10=0|", false)
        .unwrap();
    assert_ne!(original.digest(), other.digest());
    let typed = reader
        .transform_line(b"8=FIX.4.4|35=F|11=A|55=AAPL|10=0|", false)
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
    let one = reader
        .transform_line(b"35=D|VenueOwnThing=x", false)
        .unwrap();
    let other = reader
        .transform_line(b"35=D|OtherVenueThing=x", false)
        .unwrap();
    assert_ne!(one.digest(), other.digest());

    // A message built from a schema and a value has no entries, so it
    // digests as the empty walk - correct, because none of them arrived.
    let empty = reader
        .transform_line(b"no level printed by this plugin", false)
        .unwrap();
    assert!(empty.entries().is_empty());
    assert_eq!(
        empty.digest(),
        reader
            .transform_line(b"also nothing here", false)
            .unwrap()
            .digest()
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
            .map(|row| reader.transform_line(row.as_bytes(), false).unwrap()),
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
            .map(|row| reader.transform_line(row.as_bytes(), false).unwrap()),
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
            .map(|row| reader.transform_line(row.as_bytes(), false).unwrap()),
    );
    assert_eq!(
        dedup.by_ref().count(),
        1,
        "one order, however often it arrives"
    );
    assert_eq!(dedup.dropped(), 2);

    // That a delivery *was* a replay is still readable, from the columns the
    // digest declined to fold in.
    let held = reader.transform_line(resend.as_bytes(), false).unwrap();
    assert!(held.lifted("resent").is_some());
    assert_eq!(held.lifted("seqnum"), Some(&Scalar::from(8_i32)));

    // A genuinely different order is a different order.
    let amended = reader
        .transform_line(
            b"8=FIX.4.4|35=D|34=7|52=20240102-10:15:30.000|11=A|55=AAPL|38=100|10=0|",
            false,
        )
        .unwrap();
    assert_ne!(
        reader
            .transform_line(original.as_bytes(), false)
            .unwrap()
            .digest(),
        amended.digest()
    );
}

#[test]
fn a_direction_is_read_in_front_of_the_payload_and_never_inside_it() {
    // A verb inside a `Text(58)` value is payload, and the offset the reader
    // already computed is what keeps it out of the reading.
    let line = b"sending >> 8=FIX.4.2|35=D|58=received out of order|10=0|";
    let at = 11;
    assert_eq!(
        MsgDirection::at_payload(line, at, Some(MsgDirection::SENT)),
        Some(MsgDirection::SENT)
    );

    // A line the transport marked as arriving answers so, default or not.
    let arriving = b"receiving << 8=FIX.4.2|35=D|10=0|";
    assert_eq!(
        MsgDirection::at_payload(arriving, 13, Some(MsgDirection::SENT)),
        Some(MsgDirection::RECV),
        "a read verb always beats the default",
    );

    // Both verbs in one prefix is a line no reading can prefer one of, so it
    // falls to the default exactly as silence does.
    let both = b"sending a received copy >> 8=FIX.4.2|35=D|10=0|";
    assert_eq!(
        MsgDirection::at_payload(both, 26, Some(MsgDirection::SENT)),
        Some(MsgDirection::SENT)
    );
    assert_eq!(MsgDirection::at_payload(both, 26, None), None);

    // Silence takes the default, and no default is no answer.
    let bare = b"8=FIX.4.2|35=D|10=0|";
    assert_eq!(
        MsgDirection::at_payload(bare, 0, Some(MsgDirection::RECV)),
        Some(MsgDirection::RECV)
    );
    assert_eq!(MsgDirection::at_payload(bare, 0, None), None);
}

#[test]
fn the_crate_carries_fields_of_its_own_on_the_standard_branch_from_65000() {
    let held = yggdryl::fix_crate_fields().expect("the crate's own fields");
    let names: Vec<&str> = held.iter().map(yggdryl::Field::name).collect();
    assert_eq!(
        names,
        [
            "msghash",
            "version",
            "symbolticker",
            "timestamp",
            "unixpartition",
            "parentclordid",
            "parentorderid",
            "sessionid",
            "msgctxid",
            "senderpluginid",
            "targetpluginid",
            "senderpluginsession",
            "targetpluginsession",
            "isincode",
            "miccode",
            "state",
            "instid",
            "id",
            "persistentid",
        ],
    );
    let displays: Vec<Option<&str>> = held.iter().map(yggdryl::Field::display).collect();
    assert_eq!(
        displays,
        [
            Some("MsgHash"),
            Some("Version"),
            Some("SymbolTicker"),
            Some("Timestamp"),
            Some("UnixPartition"),
            Some("ParentClOrdID"),
            Some("ParentOrderID"),
            Some("SessionId"),
            Some("MsgCtxId"),
            Some("SenderPluginId"),
            Some("TargetPluginId"),
            Some("SenderPluginSession"),
            Some("TargetPluginSession"),
            Some("ISINCode"),
            Some("MICCode"),
            Some("State"),
            Some("InstId"),
            Some("Id"),
            Some("PersistentId"),
        ],
    );

    // Sixteen bytes, big-endian, because a digest is compared and ordered as
    // bytes and must not become a string.
    assert_eq!(
        held[0].dtype(),
        &DataType::fixed_size_binary(16).expect("a width")
    );
    // The columns a message answers from what it said are typed as the thing
    // they hold, not as the text a venue spelled it in; the three identities
    // a stream stamps are sixteen bytes each, as the digest is.
    assert_eq!(held[13].dtype(), &DataType::Isin);
    assert_eq!(held[14].dtype(), &DataType::Mic);
    assert_eq!(held[15].dtype(), &DataType::State);
    for identity in &held[16..19] {
        assert_eq!(
            identity.dtype(),
            &DataType::fixed_size_binary(16).expect("a width")
        );
    }

    // Every one is a standard field from 65000 up: one tag block, on the
    // branch every dictionary resolves through, so a bridge row spelling
    // `SESSIONID` or `ULFROMSESSIONNAME` reaches it by name.
    for (at, field) in held.iter().enumerate() {
        let view = field.as_fix();
        let tag = view.tag().unwrap().expect("a tag");
        let id = view.id().unwrap().expect("an identity");
        assert_eq!(tag, yggdryl::CRATE_TAG_MIN + i32::try_from(at).unwrap());
        assert_eq!(id, FixId::standard(tag), "the standard branch");
        assert!(yggdryl::is_crate_tag(tag));
    }
    assert_eq!(yggdryl::CRATE_TAG_MIN, 65_000);
    assert_eq!(yggdryl::MSGHASH_TAG, 65_000);
    assert_eq!(yggdryl::STATE_TAG, 65_015);
    assert_eq!(yggdryl::PERSISTENTID_TAG, 65_018);
    assert!(!yggdryl::is_crate_tag(yggdryl::CRATE_TAG_MIN - 1));
    let sessions = &held[11..13];
    assert_eq!(
        sessions
            .iter()
            .map(|field| field.as_fix().aliases().collect::<Vec<_>>())
            .collect::<Vec<_>>(),
        [vec!["ULFromSessionName"], vec!["ULToSessionName"]]
    );

    // `MsgDirection` is FIX's own, so it is not invented here.
    assert!(!names.contains(&"msgdirection"));
    assert_eq!(yggdryl::MSGDIRECTION_TAG, 385);

    // Every registry holds them from construction, and inserting them again
    // replaces rather than collides.
    assert_eq!(FixRegistry::new().len(), held.len());
    let registry = FixRegistry::from_fields(held.iter().cloned())
        .expect("the crate's own fields insert into a registry already holding them");
    assert_eq!(registry.len(), held.len());
    for field in held {
        let id = field.as_fix().id().unwrap().expect("an identity");
        assert_eq!(registry.field_by_id(id).unwrap().name(), field.name());
    }
}
