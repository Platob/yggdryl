//! The metadata sharing an integration test cannot reach.
//!
//! A metadata map is one shared pointer, and that it is shared - by an empty
//! map, by a clone - is the whole reason a field clone costs nothing. The
//! pointer is private, so the pin is here; everything a caller can observe
//! lives in `tests/types/metadata.rs`.

use std::sync::Arc;

use crate::Metadata;

#[test]
fn empty_and_cloned_metadata_share_their_backing_map() {
    let empty = Metadata::new();
    let other_empty = Metadata::new();
    assert!(Arc::ptr_eq(&empty.0, &other_empty.0));

    let metadata = Metadata::from_entries([("source", "orders")]).unwrap();
    let clone = metadata.clone();
    assert!(Arc::ptr_eq(&metadata.0, &clone.0));
}

/// Removal takes a folded protocol key, which is what a field's own
/// `without_*` reaches through. The method is crate-private, so the pin is
/// here.
#[test]
fn removing_by_any_spelling_takes_the_one_folded_entry() {
    let mut metadata =
        Metadata::from_entries([("HTTPS:Content-Type", "text/plain; charset=utf-8")]).unwrap();
    assert_eq!(
        metadata.remove("HTTPS:CONTENT-TYPE").as_deref(),
        Some("text/plain; charset=utf-8")
    );
    assert!(!metadata.contains_key("HTTP:content-type"));
    assert_eq!(metadata.remove("HTTP:content-type"), None);
}
