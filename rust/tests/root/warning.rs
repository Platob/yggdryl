//! `rust/src/warning.rs`: deduplicated warnings, keyed by where one is
//! raised, what went wrong and what it is about - never by the value.

use yggdryl::internals::warning::{count, is_tenfold, warn};

/// One key counts every occurrence, and another subject or reason is a key
/// of its own: a stream repeating one fault is one warning.
#[test]
fn a_warning_is_counted_under_its_site_reason_and_subject() {
    const SITE: &str = "yggdryl::tests::warning";
    const WHAT: &str = "a test value was unreadable and is null";
    let before = count(SITE, WHAT, "price");
    for row in 0..25 {
        warn(SITE, WHAT, "price", &format!("row {row} states 'x'"));
    }
    assert_eq!(count(SITE, WHAT, "price") - before, 25);
    let other = count(SITE, WHAT, "quantity");
    warn(SITE, WHAT, "quantity", "row 0 states 'y'");
    assert_eq!(count(SITE, WHAT, "quantity") - other, 1);
    assert_eq!(count(SITE, "never raised", "price"), 0);
}

/// A repeated warning is logged again only at each tenfold count, so a
/// million rows log seven lines rather than a million.
#[test]
fn a_repeated_warning_is_logged_again_at_each_tenfold_count() {
    let logged: Vec<u64> = (1..=1_000_000).filter(|count| is_tenfold(*count)).collect();
    assert_eq!(logged, [10, 100, 1_000, 10_000, 100_000, 1_000_000]);
    assert!(!is_tenfold(1) && !is_tenfold(20) && !is_tenfold(110));
}
