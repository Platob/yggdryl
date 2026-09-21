//! `rust/src/lib.rs`: what the crate root itself promises about its values.
//!
//! The core schema, identifier and metadata values cross thread boundaries,
//! so the auto traits they carry are part of the API rather than an accident
//! of what they happen to hold. Everything here is reached through
//! `yggdryl::` like any other caller.

use yggdryl::{
    DataType, Field, MediaType, Metadata, MimeType, OwnedDifferences, Scheme, StructType, Uri, Url,
    Urn,
};

fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn core_schema_values_are_send_and_sync() {
    assert_send_sync::<DataType>();
    assert_send_sync::<Field>();
    assert_send_sync::<StructType>();
    assert_send_sync::<Metadata>();
    assert_send_sync::<MimeType>();
    assert_send_sync::<MediaType>();
    assert_send_sync::<OwnedDifferences>();
    assert_send_sync::<Scheme>();
    assert_send_sync::<Uri>();
    assert_send_sync::<Url>();
    assert_send_sync::<Urn>();
}
