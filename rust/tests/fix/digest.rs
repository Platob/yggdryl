//! The value digest, the dedup adapter, and the crate's own two fields.

use std::path::PathBuf;
use std::sync::Arc;

use yggdryl::holder::local::Folder;
use yggdryl::types::Direction;
use yggdryl::{DataType, FixDedup, FixId, FixReader, FixRegistry};

fn reader() -> FixReader {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("config")
        .join("fix");
    let folder = Folder::new(root).expect("the seed folder is a local path");
    FixReader::new(Arc::new(
        FixRegistry::from_handle(&folder).expect("the committed dictionary loads"),
    ))
}

#[test]
fn identical_entries_hash_equal_and_a_different_order_does_not() {
    let reader = reader();
    let one = reader.text("8=FIX.4.4|35=D|11=A|55=AAPL|10=0|").unwrap();
    let same = reader.text("8=FIX.4.4|35=D|11=A|55=AAPL|10=0|").unwrap();
    assert_eq!(one.digest(), same.digest());

    // Order carries meaning inside a repeating group, so it is never sorted
    // away: the same pairs in another order are another message.
    let reordered = reader.text("8=FIX.4.4|35=D|55=AAPL|11=A|10=0|").unwrap();
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
    let split = reader.text("8=FIX.4.4|35=D|9999=1|9998=23|10=0|").unwrap();
    let other = reader.text("8=FIX.4.4|35=D|9999=12|9998=3|10=0|").unwrap();
    assert_ne!(split.digest(), other.digest());
}

#[test]
fn the_frame_is_not_the_message() {
    let reader = reader();
    let row = "8=FIX.4.4|9=64|35=D|11=A|55=AAPL|10=203|";
    let one = reader.text(row).unwrap();

    // A recomputed body length and a different checksum describe how the
    // message was written down, not what it says.
    let rewritten = reader
        .text("8=FIX.4.4|9=999|35=D|11=A|55=AAPL|10=000|")
        .unwrap();
    assert_eq!(one.digest(), rewritten.digest());

    // The same message re-serialized with another separator reads back equal.
    let soh = one.into_bytes(0x01);
    let again = reader.fixtext(&soh, 0x01).unwrap();
    assert_eq!(one.digest(), again.digest());

    // But the session fields stay in: two heartbeats a second apart are two
    // messages, which is exactly what a monitor needs them to be.
    let early = reader
        .text("8=FIX.4.4|35=0|34=1|52=20240102-10:15:30.000|10=0|")
        .unwrap();
    let later = reader
        .text("8=FIX.4.4|35=0|34=2|52=20240102-10:15:31.000|10=0|")
        .unwrap();
    assert_ne!(early.digest(), later.digest());
}

#[test]
fn two_unknown_keys_carrying_one_value_are_two_messages() {
    let reader = reader();
    // Neither key names a field, so the tag is `0` for both and only the key
    // itself distinguishes them.
    let one = reader.text("35=D|VenueOwnThing=x").unwrap();
    let other = reader.text("35=D|OtherVenueThing=x").unwrap();
    assert_ne!(one.digest(), other.digest());

    // A message built from a schema and a value has no entries, so it
    // digests as the empty walk - correct, because none of them arrived.
    let empty = reader.text("no level printed by this plugin").unwrap();
    assert!(empty.entries().is_empty());
    assert_eq!(
        empty.digest(),
        reader.text("also nothing here").unwrap().digest()
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
    let mut dedup = FixDedup::new(rows.iter().map(|row| reader.text(row).unwrap()));
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
            .map(|row| reader.text(row).unwrap()),
    );
    assert_eq!(clean.by_ref().count(), 2);
    assert_eq!(clean.dropped(), 0);
}

#[test]
fn a_resend_survives_dedup_and_its_exact_republication_does_not() {
    let reader = reader();
    let original = "8=FIX.4.4|35=D|34=7|52=20240102-10:15:30.000|11=A|10=0|";
    // A replay carries a fresh `SendingTime`, so it is a different message -
    // deliberately, because dropping it would remove exactly the recovery
    // traffic a sequence-gap check reads.
    let resend = "8=FIX.4.4|35=D|34=7|52=20240102-10:16:00.000|43=Y|11=A|10=0|";

    let replayed_twice = [original, resend, resend];
    let mut dedup = FixDedup::new(replayed_twice.iter().map(|row| reader.text(row).unwrap()));
    assert_eq!(
        dedup.by_ref().count(),
        2,
        "the resend lives, its twin does not"
    );
    assert_eq!(dedup.dropped(), 1);

    // The replay is caught by the facet that exists for it, not by the digest.
    let replayed = reader.text(resend).unwrap();
    assert!(replayed.lifted("resent").is_some());
}

#[test]
fn a_direction_is_read_in_front_of_the_payload_and_never_inside_it() {
    // A verb inside a `Text(58)` value is payload, and the offset the reader
    // already computed is what keeps it out of the reading.
    let line = b"sending >> 8=FIX.4.2|35=D|58=received out of order|10=0|";
    let at = 11;
    assert_eq!(
        Direction::at_payload(line, at, Some(Direction::SENT)),
        Some(Direction::SENT)
    );

    // A line the transport marked as arriving answers so, default or not.
    let arriving = b"receiving << 8=FIX.4.2|35=D|10=0|";
    assert_eq!(
        Direction::at_payload(arriving, 13, Some(Direction::SENT)),
        Some(Direction::RECV),
        "a read verb always beats the default",
    );

    // Both verbs in one prefix is a line no reading can prefer one of, so it
    // falls to the default exactly as silence does.
    let both = b"sending a received copy >> 8=FIX.4.2|35=D|10=0|";
    assert_eq!(
        Direction::at_payload(both, 26, Some(Direction::SENT)),
        Some(Direction::SENT)
    );
    assert_eq!(Direction::at_payload(both, 26, None), None);

    // Silence takes the default, and no default is no answer.
    let bare = b"8=FIX.4.2|35=D|10=0|";
    assert_eq!(
        Direction::at_payload(bare, 0, Some(Direction::RECV)),
        Some(Direction::RECV)
    );
    assert_eq!(Direction::at_payload(bare, 0, None), None);
}

#[test]
fn the_crate_carries_two_fields_of_its_own_on_a_branch_of_its_own() {
    let held = yggdryl::fix_crate_fields().expect("the crate's own fields");
    assert_eq!(held[0].name(), "msghash");
    assert_eq!(held[1].name(), "direction");

    // Sixteen bytes, big-endian, because a digest is compared and ordered as
    // bytes and must not become a string.
    assert_eq!(
        held[0].dtype(),
        &DataType::fixed_size_binary(16).expect("a width")
    );
    assert_eq!(held[1].dtype(), &DataType::Direction);

    // Same tags a venue's own could be, and different identities.
    for (field, tag) in held
        .iter()
        .zip([yggdryl::MSGHASH_TAG, yggdryl::DIRECTION_TAG])
    {
        let id = field.as_fix().id().unwrap().expect("an identity");
        assert_eq!(field.as_fix().tag().unwrap(), Some(tag));
        assert_ne!(id, FixId::standard(tag), "not the standard branch");
        assert_eq!(
            field.as_fix().branch().unwrap().name(),
            yggdryl::CRATE_BRANCH
        );
    }

    // A dictionary that has them resolves them like any other field, and one
    // that does not is unchanged.
    let registry = FixRegistry::from_fields(held.iter().cloned())
        .expect("the crate's own fields make a dictionary")
        .with_crate_fields()
        .expect("adding what is already there is not a collision");
    assert_eq!(registry.len(), 2);
}
