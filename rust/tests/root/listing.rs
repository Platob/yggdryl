//! `rust/src/listing.rs`: the fused listing an entry stream is.

use yggdryl::Error;
use yggdryl::Listing;

#[test]
fn a_listing_is_fused_after_the_first_failure() {
    let mut listing = Listing::new(
        [
            Err(Error::absent("file", "a")),
            Err(Error::absent("file", "b")),
        ]
        .into_iter(),
    );
    assert!(listing.next().is_some_and(|entry| entry.is_err()));
    assert!(listing.next().is_none());
    assert!(listing.next().is_none());
}

#[test]
fn an_empty_listing_yields_nothing() {
    assert_eq!(Listing::empty().count(), 0);
    assert_eq!(Listing::default().count(), 0);
}

#[test]
fn a_failing_listing_reports_once_and_ends() {
    let entries: Vec<_> = Listing::failing(Error::absent("folder", "gone")).collect();
    assert_eq!(entries.len(), 1);
    assert!(entries[0].as_ref().is_err_and(Error::is_absent));
}
