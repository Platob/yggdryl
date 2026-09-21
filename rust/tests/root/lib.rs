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

mod enums {

    use yggdryl::{
        DataTypeId, DataTypeKind, IOMode, MediaType, MimeType, Scheme, TimeUnit, UnionMode,
    };

    #[test]
    fn generic_reexports_the_public_vocabulary() {
        let id: yggdryl::DataTypeId = DataTypeId::Int32;
        let kind: yggdryl::DataTypeKind = DataTypeKind::Integer;
        let scheme: yggdryl::Scheme = Scheme::HTTPS;
        let unit: yggdryl::TimeUnit = TimeUnit::Nanosecond;
        let mode: yggdryl::UnionMode = UnionMode::Dense;
        let mime: yggdryl::MimeType = MimeType::JSON;
        let media: yggdryl::MediaType = MediaType::from(mime.clone());
        let write: yggdryl::IOMode = IOMode::Overwrite;

        assert_eq!(scheme, Scheme::HTTPS);
        assert_eq!(id, DataTypeId::Int32);
        assert_eq!(kind, DataTypeKind::Integer);
        assert_eq!(id.kind(), kind);
        assert_eq!(unit, TimeUnit::Nanosecond);
        assert_eq!(mode, UnionMode::Dense);
        assert_eq!(mime, MimeType::JSON);
        assert_eq!(media.base(), &MimeType::JSON);
        assert_eq!(write, IOMode::Overwrite);
    }
}
