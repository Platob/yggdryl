//! `rust/src/logging/repeats.rs`: repeated records counted by hash - the
//! table every logger, data warning and binding filter counts through.

use std::sync::Arc;

use yggdryl::logging::{Counted, Level, Record, Repeat, Repeats};

#[test]
fn a_key_is_said_first_then_at_each_tenfold_count() {
    let repeats = Repeats::new();
    assert!(repeats.is_empty());
    let said: Vec<(u64, Repeat)> = (1..=1_000)
        .map(|count| (count, repeats.count(42)))
        .filter(|(_, repeat)| repeat.is_said())
        .collect();
    assert_eq!(
        said,
        [
            (1, Repeat::First),
            (10, Repeat::Tenfold(10)),
            (100, Repeat::Tenfold(100)),
            (1_000, Repeat::Tenfold(1_000)),
        ]
    );
    assert_eq!(repeats.seen(42), 1_000);
    assert_eq!(repeats.seen(7), 0, "a key never counted");
    assert_eq!(repeats.len(), 1);
}

#[test]
fn zero_is_a_key_like_any_other() {
    let repeats = Repeats::new();
    assert_eq!(repeats.count(0), Repeat::First);
    assert_eq!(repeats.count(0), Repeat::Repeated);
    assert_eq!(repeats.seen(0), 2);
}

#[test]
fn a_full_table_counts_no_new_key_and_keeps_counting_the_ones_it_holds() {
    let repeats = Repeats::new();
    for key in 1..=Repeats::CAPACITY as u64 {
        assert_eq!(repeats.count(key), Repeat::First);
    }
    assert_eq!(repeats.len(), Repeats::CAPACITY);
    assert_eq!(repeats.count(u64::MAX), Repeat::Untracked);
    assert!(
        Repeat::Untracked.is_said(),
        "a logger says an uncounted key in full"
    );
    assert_eq!(repeats.seen(u64::MAX), 0);
    assert_eq!(repeats.count(1), Repeat::Repeated);
    assert_eq!(repeats.seen(1), 2);
}

#[test]
fn threads_racing_for_the_last_room_never_take_the_table_past_its_capacity() {
    let repeats = Arc::new(Repeats::new());
    let firsts: usize = (0..8_u64)
        .map(|thread| {
            let repeats = Arc::clone(&repeats);
            std::thread::spawn(move || {
                (0..Repeats::CAPACITY as u64)
                    .filter(|key| repeats.count(key * 8 + thread + 1) == Repeat::First)
                    .count()
            })
        })
        .map(|counting| counting.join().expect("a thread"))
        .sum();
    assert_eq!(firsts, Repeats::CAPACITY, "one first per key held");
    assert_eq!(repeats.len(), Repeats::CAPACITY);
}

#[test]
fn threads_counting_one_key_see_one_first() {
    let repeats = Arc::new(Repeats::new());
    let firsts: usize = (0..8)
        .map(|_| {
            let repeats = Arc::clone(&repeats);
            std::thread::spawn(move || {
                (0..1_000)
                    .filter(|_| repeats.count(99) == Repeat::First)
                    .count()
            })
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|thread| thread.join().expect("a counting thread"))
        .sum();
    assert_eq!(firsts, 1);
    assert_eq!(repeats.seen(99), 8_000);
}

#[test]
fn a_record_is_keyed_by_its_logger_level_and_rendered_message() {
    let one = Record::new("trades.feed", Level::WARNING, &"late fill");
    let rendered = format!("late {}", "fill");
    let same = Record::new("trades.feed", Level::WARNING, &rendered);
    assert_eq!(
        one.stable_hash(),
        same.stable_hash(),
        "the text, however built"
    );
    for other in [
        Record::new("trades.book", Level::WARNING, &"late fill"),
        Record::new("trades.feed", Level::ERROR, &"late fill"),
        Record::new("trades.feed", Level::WARNING, &"late fills"),
        Record::new("trades.fee", Level::WARNING, &"dlate fill"),
    ] {
        assert_ne!(one.stable_hash(), other.stable_hash(), "{other:?}");
    }
    let dated = one.with_created(0).with_location("feed.rs", 9);
    assert_eq!(
        one.stable_hash(),
        dated.stable_hash(),
        "never the instant or the place"
    );

    let repeats = Repeats::new();
    assert_eq!(repeats.count_record(&one), Repeat::First);
    assert_eq!(repeats.count_record(&same), Repeat::Repeated);
}

#[test]
fn a_counted_message_says_its_count_after_it() {
    assert_eq!(
        Counted::new(10, &"late fill").to_string(),
        "late fill (seen 10 times)"
    );
}
