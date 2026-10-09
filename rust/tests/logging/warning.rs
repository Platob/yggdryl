//! `rust/src/logging/warning.rs`: deduplicated warnings, keyed by where one is
//! raised, what went wrong and what it is about - never by the value.

use yggdryl::internals::logging_warning::{count, is_tenfold, warn};

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

/// A site is keyed by the logger name it carries, so a module keeps its
/// count whatever crate of the workspace holds it - the FIX crate's
/// `tests::moved` is the core's `fix::tests::moved`, the market crate's
/// modules the core's own - and a crate outside the workspace is a site of
/// its own.
#[test]
fn a_site_is_keyed_by_its_logger_name_whatever_crate_holds_it() {
    const WHAT: &str = "a test value moved crates";
    let before = count("yggdryl::fix::tests::moved", WHAT, "price");
    warn("yggdryl_fix::tests::moved", WHAT, "price", "row 0");
    warn("yggdryl::fix::tests::moved", WHAT, "price", "row 1");
    assert_eq!(
        count("yggdryl::fix::tests::moved", WHAT, "price") - before,
        2
    );
    assert_eq!(
        count("yggdryl_fix::tests::moved", WHAT, "price"),
        count("yggdryl::fix::tests::moved", WHAT, "price")
    );

    let market = count("yggdryl::tests::moved::market", WHAT, "price");
    warn(
        "yggdryl_market::tests::moved::market",
        WHAT,
        "price",
        "row 0",
    );
    assert_eq!(
        count("yggdryl::tests::moved::market", WHAT, "price") - market,
        1
    );

    let foreign = count("yggdryl_cli::tests::moved", WHAT, "price");
    warn("yggdryl_cli::tests::moved", WHAT, "price", "row 0");
    assert_eq!(
        count("yggdryl_cli::tests::moved", WHAT, "price") - foreign,
        1
    );
    assert_eq!(count("yggdryl::cli::tests::moved", WHAT, "price"), 0);
}
