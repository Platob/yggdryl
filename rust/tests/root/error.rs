//! `rust/src/error.rs`: the failures every boundary answers with.

#[cfg(all(feature = "internals", feature = "iceberg"))]
use std::error::Error as _;

use yggdryl::Error;

#[cfg(all(feature = "internals", feature = "iceberg"))]
#[test]
fn official_iceberg_failures_keep_their_source_behind_the_core_error() {
    let error = yggdryl::internals::error::invalid_iceberg_metadata("invalid metadata");
    assert!(matches!(
        error,
        Error::Iceberg {
            source: Some(_),
            ..
        }
    ));
    assert!(error.source().is_some());
}

#[test]
fn an_absence_names_what_was_expected_and_where() {
    let error = Error::absent("table", "warehouse/sales/orders");
    assert_eq!(
        error.to_string(),
        "expected a table at \"warehouse/sales/orders\", got nothing"
    );
    assert!(error.is_absent());
    assert!(!error.is_conflict());
}

#[test]
fn a_conflict_names_both_sides_and_where() {
    let error = Error::conflict("table", "namespace", "warehouse/sales");
    assert_eq!(
        error.to_string(),
        "expected to create a table at \"warehouse/sales\", got an existing namespace"
    );
    assert!(error.is_conflict());
    assert!(!error.is_absent());
}

#[test]
fn a_backend_spelling_normalizes_into_the_typed_variant() {
    let absent = Error::from_io_at(
        std::io::Error::from(std::io::ErrorKind::NotFound),
        "file",
        "/tmp/missing",
    );
    assert!(matches!(absent, Error::Absent { .. }));

    let conflict = Error::from_io_at(
        std::io::Error::from(std::io::ErrorKind::AlreadyExists),
        "file",
        "/tmp/taken",
    );
    assert!(matches!(conflict, Error::Conflict { .. }));
}

#[test]
fn a_remote_refusal_names_the_store_the_operation_and_the_verdict() {
    let error = Error::remote(
        "s3",
        "GetObject",
        403,
        "AccessDenied",
        "Access Denied\nsecond line is dropped",
        "s3://trades/part.parquet",
    );
    assert_eq!(
        error.to_string(),
        "s3 GetObject at \"s3://trades/part.parquet\" failed with 403 AccessDenied: Access Denied"
    );
    assert!(!error.is_absent());
    assert!(!error.is_conflict());
}

#[test]
fn a_permission_failure_is_neither_an_absence_nor_a_conflict() {
    let denied = Error::from_io_at(
        std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        "file",
        "/tmp/locked",
    );
    assert!(matches!(denied, Error::Io(_)));
    assert!(!denied.is_absent());
    assert!(!denied.is_conflict());
}

#[test]
fn an_unnormalized_backend_answer_still_branches_the_same_way() {
    // A boundary that has not been normalized yet must not make a caller
    // repair something that was never missing, nor miss a real absence.
    let absent = Error::Io(std::io::Error::from(std::io::ErrorKind::NotFound));
    assert!(absent.is_absent());
    let conflict = Error::Io(std::io::Error::from(std::io::ErrorKind::AlreadyExists));
    assert!(conflict.is_conflict());
}
