//! Tests for the [`ByteCursor`] sequential cursor.

use yggdryl_core::{Buffer, ByteCursor, IoError, Whence};

#[test]
fn reads_advance_and_writes_extend() {
    let mut io = ByteCursor::new(Buffer::from_vec(b"hello world".to_vec()));
    assert_eq!(io.read_bytes(5).unwrap().as_slice(), b"hello");
    assert_eq!(io.position(), 5);
    assert_eq!(io.read_bytes(6).unwrap().as_slice(), b" world");
    assert_eq!(io.position(), 11);
    // At EOF the read is empty and the cursor holds.
    assert!(io.read_bytes(3).unwrap().as_slice().is_empty());
    assert_eq!(io.position(), 11);
    // Append at the end.
    io.write_bytes(b"!").unwrap();
    assert_eq!(io.position(), 12);
    assert_eq!(io.get_ref().as_slice(), b"hello world!");
}

#[test]
fn seek_resolves_all_whences_and_bounds() {
    let mut io = ByteCursor::new(Buffer::from_vec(b"0123456789".to_vec()));
    assert_eq!(io.seek(3, Whence::Start).unwrap(), 3);
    assert_eq!(io.seek(2, Whence::Current).unwrap(), 5);
    assert_eq!(io.seek(-1, Whence::End).unwrap(), 9);
    // Out-of-range seeks error, leaving the cursor where it was.
    assert_eq!(io.seek(-1, Whence::Start), Err(IoError::OutOfBounds));
    assert_eq!(io.seek(1, Whence::End), Err(IoError::OutOfBounds));
    assert_eq!(io.position(), 9);
}

#[cfg(feature = "serde")]
#[test]
fn whence_serde_round_trip() {
    for whence in [Whence::Start, Whence::Current, Whence::End] {
        let json = serde_json::to_string(&whence).unwrap();
        assert_eq!(serde_json::from_str::<Whence>(&json).unwrap(), whence);
    }
    // It serializes as its numeric discriminant.
    assert_eq!(serde_json::to_string(&Whence::End).unwrap(), "2");
    // An out-of-range discriminant is rejected.
    assert!(serde_json::from_str::<Whence>("3").is_err());
}
