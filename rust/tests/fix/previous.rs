//! What a message takes from the one before it in its chain, at the edges.
//!
//! [`FixMsg::with_previous`] is the one place that says it, and
//! [`FixLifecycle::fill`] writes the same stamps beside its own. These are
//! the cases where the two must agree and where "nothing moved" has to mean
//! exactly that.

use std::sync::Arc;

use yggdryl::{
    CREATEDAT_TAG_NAME, FixCodec, FixLifecycle, FixMsg, FixRegistry, PREVMSGHASH_TAG_NAME,
    PREVUPDATEDAT_TAG_NAME, SNAPSHOTAT_TAG_NAME, Scalar, TimeUnit, Timezone, UPDATEDAT_TAG_NAME,
};

fn registry() -> Arc<FixRegistry> {
    super::committed_registry()
}

fn codec() -> FixCodec {
    super::fixed_codec(registry())
}

fn clock(count: i64) -> Scalar {
    Scalar::datetime64(count, TimeUnit::Nanosecond, Timezone::UTC).expect("a clock")
}

/// One order, dated by the sending time its line spells.
fn order(key: &str, sending: &str) -> FixMsg {
    let line = format!("8=FIX.4.4|35=D|11={key}|55=AAPL|52={sending}|10=0|");
    codec().parse_fix_line(line.as_bytes()).expect("parses")
}

#[test]
fn a_message_with_no_previous_still_says_so_and_says_it_once() {
    let held = order("A1", "20240102-10:15:30.000");
    // The pair columns are the row's way of saying "nothing came before",
    // so the first message of a chain gains them empty rather than lacking
    // them: a reader asking for the previous clock gets a null, not an error.
    let opened = held
        .clone()
        .with_previous(None)
        .expect("no previous is a legal previous")
        .expect("the columns arrive");
    for tag in [PREVUPDATEDAT_TAG_NAME.0, PREVMSGHASH_TAG_NAME.0] {
        assert!(opened.by_tag(tag).expect("a column").is_null());
    }
    // And once: the second call has nothing left to write.
    assert!(
        opened
            .clone()
            .with_previous(None)
            .expect("still legal")
            .is_none()
    );
}

#[test]
fn a_pair_is_the_previous_messages_own_three_facts() {
    let first = order("A1", "20240102-10:15:30.000");
    let second = order("A1", "20240102-10:15:31.000");
    let paired = second
        .with_previous(Some(&first))
        .expect("a pair")
        .expect("it moved");
    assert_eq!(
        paired.by_tag(PREVUPDATEDAT_TAG_NAME.0).unwrap(),
        first.updatedat()
    );
    assert_eq!(
        paired.by_tag(PREVMSGHASH_TAG_NAME.0).unwrap(),
        first.msghash()
    );
    // The creation instant is the chain's, which is the predecessor's: a
    // message joining a chain was not created when it arrived.
    assert_eq!(paired.createdat(), first.createdat());
    assert_ne!(paired.updatedat(), first.updatedat());
}

#[test]
fn a_message_that_states_its_own_pair_keeps_it_and_moves_nothing_else() {
    let first = order("A1", "20240102-10:15:30.000");
    let mut second = order("A1", "20240102-10:15:31.000");
    // A stated reading is never landed over, so a replayed row carrying the
    // pair its capture wrote is the row that comes back out.
    let stated = clock(7);
    second
        .set(PREVUPDATEDAT_TAG_NAME.0, stated.clone())
        .unwrap();
    second
        .set(PREVMSGHASH_TAG_NAME.0, first.msghash().clone())
        .unwrap();
    second
        .set(CREATEDAT_TAG_NAME.0, first.createdat().clone())
        .unwrap();
    assert!(
        second
            .clone()
            .with_previous(Some(&first))
            .expect("a pair")
            .is_none(),
        "everything it would write is already there"
    );
    // A different predecessor still only moves what is not stated, which is
    // the creation instant alone.
    let other = order("A2", "20240102-10:15:29.000");
    let paired = second
        .with_previous(Some(&other))
        .expect("a pair")
        .expect("the creation instant moved");
    assert_eq!(paired.by_tag(PREVUPDATEDAT_TAG_NAME.0).unwrap(), &stated);
    assert_eq!(
        paired.by_tag(PREVMSGHASH_TAG_NAME.0).unwrap(),
        first.msghash()
    );
    assert_eq!(paired.createdat(), other.createdat());
}

#[test]
fn the_lifecycle_pairs_exactly_as_the_message_does() {
    // Two doors, one answer: what `fill` writes beside its own stamps is
    // what `with_previous` writes on its own.
    let mut life = FixLifecycle::new(registry());
    let first = life.fill(order("A1", "20240102-10:15:30.000")).unwrap();
    let second = life.fill(order("A1", "20240102-10:15:31.000")).unwrap();
    assert_eq!(
        second.by_tag(PREVMSGHASH_TAG_NAME.0).unwrap(),
        first.msghash()
    );
    assert_eq!(
        second.by_tag(PREVUPDATEDAT_TAG_NAME.0).unwrap(),
        first.updatedat()
    );
    assert_eq!(second.createdat(), first.createdat());
    // And a message already stamped that way moves nothing when paired again.
    assert!(
        second
            .clone()
            .with_previous(Some(&first))
            .expect("a pair")
            .is_none()
    );
}

#[test]
fn a_snapshot_stamps_when_it_was_taken_and_a_fill_never_does() {
    let mut life = FixLifecycle::new(registry())
        .try_with_interval_ns(1_000_000_000)
        .unwrap();
    let filled = life.fill(order("A1", "20240102-10:15:30.250")).unwrap();
    assert!(
        filled.by_tag(SNAPSHOTAT_TAG_NAME.0).unwrap().is_null(),
        "a fill is not a reading"
    );
    let mut life = FixLifecycle::new(registry())
        .try_with_interval_ns(1_000_000_000)
        .unwrap();
    let taken = life
        .snapshot(order("A1", "20240102-10:15:30.250"))
        .unwrap()
        .expect("an off-grid arrival emits");
    // The grid cut `updatedat`; the reading says what it was cut from.
    assert_eq!(
        taken.by_tag(SNAPSHOTAT_TAG_NAME.0).unwrap(),
        taken.createdat()
    );
    assert_ne!(
        taken.by_tag(SNAPSHOTAT_TAG_NAME.0).unwrap(),
        taken.updatedat()
    );
    assert_eq!(
        taken.updatedat().temporal_count_at(TimeUnit::Millisecond),
        Some(1_704_190_530_000)
    );
}

#[test]
fn a_stated_snapshot_clock_is_the_rows_own_and_a_reading_never_lands_over_it() {
    let mut life = FixLifecycle::new(registry())
        .try_with_interval_ns(1_000_000_000)
        .unwrap();
    let mut held = order("A1", "20240102-10:15:30.250");
    let stated = clock(99);
    held.set(SNAPSHOTAT_TAG_NAME.0, stated.clone()).unwrap();
    let taken = life.snapshot(held).unwrap().expect("it emits");
    assert_eq!(taken.by_tag(SNAPSHOTAT_TAG_NAME.0).unwrap(), &stated);
    // And the row still says which bucket it was cut to.
    assert_eq!(
        taken.by_tag(UPDATEDAT_TAG_NAME.0).unwrap(),
        taken.updatedat()
    );
}
