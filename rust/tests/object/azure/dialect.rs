//! `rust/src/object/azure/dialect.rs`: the block naming no caller can name.
//!
//! A committed block list is only readable if every id of one blob is the same
//! length and the ids sort the way the blocks are ordered, which is a property
//! of the naming rather than of any request.

use yggdryl::internals::object_azure_dialect::{batch_boundary, block_id};

#[test]
fn every_block_id_of_a_blob_is_the_same_length() {
    let first = block_id(1);
    let last = block_id(9_999_999);
    assert_eq!(first.len(), last.len());
    assert_ne!(first, last);
    // Ids sort the way the blocks are ordered, which is what makes a
    // committed list readable.
    assert!(block_id(2) > block_id(1));
}

#[test]
fn a_batch_boundary_is_the_prefix_azure_demands() {
    assert!(batch_boundary(b"lake").starts_with("batch_"));
}
