//! Tests for the [`Io`] byte abstraction and its [`BytesIo`] backend.

use yggdryl_core::{Buffer, BytesIo, Io, IoError, Whence};

#[test]
fn positional_read_is_zero_copy_and_leaves_the_cursor() {
    let io = BytesIo::from_bytes(b"hello world".to_vec());
    let world = io.positional_read_bytes(6, 5).unwrap();
    assert_eq!(world.as_slice(), b"world");
    // Shares the backing allocation — same address, no copy.
    assert_eq!(world.as_slice().as_ptr(), io.as_slice()[6..].as_ptr());
    // A positional read never moves the cursor.
    assert_eq!(io.position(), 0);
}

#[test]
fn positional_read_clamps_at_eof_and_errors_past_the_end() {
    let io = BytesIo::from_bytes(b"abc".to_vec());
    // Asking for more than is available returns only what is there.
    assert_eq!(io.positional_read_bytes(1, 10).unwrap().as_slice(), b"bc");
    // Reading exactly at the end yields nothing.
    assert!(io.positional_read_bytes(3, 4).unwrap().is_empty());
    // Starting past the end errors.
    assert_eq!(io.positional_read_bytes(4, 1), Err(IoError::OutOfBounds));
}

#[test]
fn sequential_read_advances_the_cursor() {
    let mut io = BytesIo::from_bytes(b"hello world".to_vec());
    assert_eq!(io.read_bytes(5).unwrap().as_slice(), b"hello");
    assert_eq!(io.position(), 5);
    assert_eq!(io.read_bytes(6).unwrap().as_slice(), b" world");
    assert_eq!(io.position(), 11);
    // At EOF the read is empty and the cursor holds.
    assert!(io.read_bytes(4).unwrap().is_empty());
    assert_eq!(io.position(), 11);
}

#[test]
fn write_overwrites_extends_and_advances_the_cursor() {
    let mut io = BytesIo::from_bytes(b"abc".to_vec());
    // Overwrite in place from the cursor (0).
    assert_eq!(io.write_bytes(b"AB").unwrap(), 2);
    assert_eq!(io.position(), 2);
    assert_eq!(io.as_slice(), b"ABc");
    // Seek to the end and extend.
    io.seek(0, Whence::End).unwrap();
    io.write_bytes(b"XYZ").unwrap();
    assert_eq!(io.as_slice(), b"ABcXYZ");
    assert_eq!(io.position(), 6);
}

#[test]
fn positional_write_leaves_the_cursor_and_errors_past_the_end() {
    let mut io = BytesIo::from_bytes(b"abcd".to_vec());
    io.seek(2, Whence::Start).unwrap();
    // A positional write does not move the cursor.
    assert_eq!(io.positional_write_bytes(0, b"Z").unwrap(), 1);
    assert_eq!(io.as_slice(), b"Zbcd");
    assert_eq!(io.position(), 2);
    // Writing with a gap past the end is rejected.
    assert_eq!(
        io.positional_write_bytes(5, b"x"),
        Err(IoError::OutOfBounds)
    );
}

#[test]
fn seek_resolves_all_whences_and_bounds() {
    let mut io = BytesIo::from_bytes(b"0123456789".to_vec());
    assert_eq!(io.seek(3, Whence::Start).unwrap(), 3);
    assert_eq!(io.seek(2, Whence::Current).unwrap(), 5);
    assert_eq!(io.seek(-1, Whence::End).unwrap(), 9);
    // Out-of-range seeks error, leaving the cursor where it was.
    assert_eq!(io.seek(-1, Whence::Start), Err(IoError::OutOfBounds));
    assert_eq!(io.seek(1, Whence::End), Err(IoError::OutOfBounds));
    assert_eq!(io.position(), 9);
}

#[test]
fn empty_source() {
    let io = BytesIo::new();
    assert_eq!(io.len().unwrap(), 0);
    assert!(io.is_empty().unwrap());
}

/// A second, read-only backend: it serves bytes but refuses every write, so the
/// default [`Io::write_bytes`] surfaces [`IoError::ReadOnly`].
#[derive(Default)]
struct ReadOnly {
    data: Vec<u8>,
    position: u64,
}

impl Io for ReadOnly {
    fn len(&self) -> Result<u64, IoError> {
        Ok(self.data.len() as u64)
    }

    fn position(&self) -> u64 {
        self.position
    }

    fn set_position(&mut self, position: u64) {
        self.position = position;
    }

    fn positional_read_bytes(&self, offset: u64, len: usize) -> Result<Buffer, IoError> {
        let offset = usize::try_from(offset).map_err(|_| IoError::OutOfBounds)?;
        if offset > self.data.len() {
            return Err(IoError::OutOfBounds);
        }
        let end = offset.saturating_add(len).min(self.data.len());
        Ok(Buffer::from_vec(self.data[offset..end].to_vec()))
    }

    fn positional_write_bytes(&mut self, _offset: u64, _bytes: &[u8]) -> Result<usize, IoError> {
        Err(IoError::ReadOnly)
    }
}

#[test]
fn read_only_backend_rejects_writes_through_the_default() {
    let mut ro = ReadOnly {
        data: b"data".to_vec(),
        position: 0,
    };
    assert_eq!(ro.read_bytes(4).unwrap().as_slice(), b"data");
    assert_eq!(ro.write_bytes(b"x"), Err(IoError::ReadOnly));
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
