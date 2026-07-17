//! Tests for [`IOKind`](yggdryl_core::io::IOKind): the stable numeric values, `parse_str`
//! tokens, the checked `from_u8`, the `exists` predicate, and the guided error text.

use yggdryl_core::io::{IOKind, IoError};

#[test]
fn numeric_values_are_stable() {
    assert_eq!(IOKind::Missing.to_u8(), 0);
    assert_eq!(IOKind::File.to_u8(), 1);
    assert_eq!(IOKind::Directory.to_u8(), 2);
    assert_eq!(IOKind::Heap.to_u8(), 3);
    for kind in [
        IOKind::Missing,
        IOKind::File,
        IOKind::Directory,
        IOKind::Heap,
    ] {
        assert_eq!(IOKind::from_u8(kind.to_u8()).unwrap(), kind);
    }
    assert!(matches!(
        IOKind::from_u8(4),
        Err(IoError::UnknownName { .. })
    ));
}

#[test]
fn parse_str_accepts_names_case_insensitively() {
    for (token, kind) in [
        ("missing", IOKind::Missing),
        ("FILE", IOKind::File),
        ("directory", IOKind::Directory),
        ("dir", IOKind::Directory),
        ("Heap", IOKind::Heap),
    ] {
        assert_eq!(IOKind::parse_str(token).unwrap(), kind, "token {token:?}");
    }
    let err = IOKind::parse_str("bogus").unwrap_err();
    assert!(err.to_string().contains("IOKind"));
    assert!(err.to_string().contains("directory/dir"));
}

#[test]
fn name_display_and_exists() {
    for kind in [
        IOKind::Missing,
        IOKind::File,
        IOKind::Directory,
        IOKind::Heap,
    ] {
        assert_eq!(IOKind::parse_str(kind.name()).unwrap(), kind);
        assert_eq!(kind.to_string(), kind.name());
    }
    assert!(!IOKind::Missing.exists());
    assert!(IOKind::File.exists());
    assert!(IOKind::Directory.exists());
    assert!(IOKind::Heap.exists());
}
