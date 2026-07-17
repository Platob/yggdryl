//! Tests for [`IOMode`](yggdryl_core::io::IOMode): the stable numeric values, the `parse_str`
//! tokens (canonical + aliases, case-insensitive), the checked `from_u8`, predicates, and the
//! guided error text.

use yggdryl_core::io::{IOMode, IoError};

#[test]
fn numeric_values_are_stable() {
    assert_eq!(IOMode::Read.to_u8(), 1);
    assert_eq!(IOMode::Write.to_u8(), 2);
    assert_eq!(IOMode::ReadWrite.to_u8(), 3); // Read | Write
    assert_eq!(IOMode::Append.to_u8(), 4);
    assert_eq!(IOMode::Overwrite.to_u8(), 5);
    for mode in [
        IOMode::Read,
        IOMode::Write,
        IOMode::ReadWrite,
        IOMode::Append,
        IOMode::Overwrite,
    ] {
        assert_eq!(IOMode::from_u8(mode.to_u8()).unwrap(), mode);
    }
    assert!(matches!(
        IOMode::from_u8(0),
        Err(IoError::UnknownName { .. })
    ));
    assert!(IOMode::from_u8(6).is_err());
}

#[test]
fn parse_str_accepts_names_and_aliases_case_insensitively() {
    for (token, mode) in [
        ("read", IOMode::Read),
        ("r", IOMode::Read),
        ("WRITE", IOMode::Write),
        ("w", IOMode::Write),
        ("read_write", IOMode::ReadWrite),
        ("rw", IOMode::ReadWrite),
        ("+", IOMode::ReadWrite),
        ("Append", IOMode::Append),
        ("a", IOMode::Append),
        ("overwrite", IOMode::Overwrite),
        ("o", IOMode::Overwrite),
        ("truncate", IOMode::Overwrite),
    ] {
        assert_eq!(IOMode::parse_str(token).unwrap(), mode, "token {token:?}");
    }
}

#[test]
fn parse_str_error_is_guided() {
    let err = IOMode::parse_str("bogus").unwrap_err();
    let text = err.to_string();
    assert!(text.contains("IOMode"));
    assert!(text.contains("bogus"));
    assert!(text.contains("read_write")); // lists the accepted tokens
}

#[test]
fn name_is_parse_str_inverse_and_display_matches() {
    for mode in [
        IOMode::Read,
        IOMode::Write,
        IOMode::ReadWrite,
        IOMode::Append,
        IOMode::Overwrite,
    ] {
        assert_eq!(IOMode::parse_str(mode.name()).unwrap(), mode);
        assert_eq!(mode.to_string(), mode.name());
    }
}

#[test]
fn predicates() {
    assert!(IOMode::Read.is_readable());
    assert!(!IOMode::Read.is_writable());
    assert!(IOMode::ReadWrite.is_readable());
    assert!(IOMode::ReadWrite.is_writable());
    for write_only in [IOMode::Write, IOMode::Append, IOMode::Overwrite] {
        assert!(!write_only.is_readable(), "{write_only}");
        assert!(write_only.is_writable(), "{write_only}");
    }
}
