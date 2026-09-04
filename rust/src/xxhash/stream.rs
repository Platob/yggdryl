//! Digests over a moving stream of bytes.
//!
//! Three shapes share one rule: the payload is never held whole. A
//! [`DigestReader`] hashes what a caller is already reading, a
//! [`DigestWriter`] hashes what a caller is already writing, and
//! [`crate::IOBase::read_digest`] hashes a stored object through
//! [`crate::IOBase::pstream_bytes`], so a 64 GiB file costs one window.

use std::io::{Read, Write};

use crate::{DEFAULT_STREAM_BATCH_SIZE, IOBase};
use crate::{Digest, DigestAlgorithm, Digester, Error, Result};

/// Digest a whole handle without holding it in memory.
///
/// A missing resource digests as empty: construction is lazy everywhere else
/// in the project, so absence is emptiness here too rather than a third
/// answer a caller has to branch on.
pub(crate) fn read_digest<H: IOBase + ?Sized>(
    handle: &H,
    algorithm: DigestAlgorithm,
) -> Result<Digest> {
    let mut digester = algorithm.digester();
    feed_handle(handle, &mut digester)?;
    Ok(digester.as_digest())
}

/// Digest `length` bytes of a handle starting at `offset`.
///
/// The window is clamped the way [`crate::IOBase::read_range_bytes`]
/// clamps: a range starting or ending past the value digests only the bytes
/// that are there, and a range wholly past it digests nothing.
pub(crate) fn read_range_digest<H: IOBase + ?Sized>(
    handle: &H,
    offset: u64,
    length: usize,
    algorithm: DigestAlgorithm,
) -> Result<Digest> {
    reject_container(handle)?;
    let mut digester = algorithm.digester();
    let mut remaining = length;
    if remaining == 0 {
        return Ok(digester.as_digest());
    }
    for chunk in handle.pstream_bytes(offset, DEFAULT_STREAM_BATCH_SIZE)? {
        let chunk = chunk?;
        let taken = chunk.len().min(remaining);
        digester.write_bytes(&chunk[..taken]);
        remaining -= taken;
        if remaining == 0 {
            break;
        }
    }
    Ok(digester.as_digest())
}

/// Feed a whole handle into `digester`, returning the bytes consumed.
///
/// This is the one place a handle becomes digest input, so the streaming
/// contract - one bounded window, no whole-value read, a container refused by
/// kind - is stated once and inherited by everything that hashes a handle.
pub(crate) fn feed_handle<H: IOBase + ?Sized>(handle: &H, digester: &mut Digester) -> Result<u64> {
    reject_container(handle)?;
    let mut consumed = 0_u64;
    for chunk in handle.pstream_bytes(0, DEFAULT_STREAM_BATCH_SIZE)? {
        let chunk = chunk?;
        digester.write_bytes(&chunk);
        consumed += chunk.len() as u64;
    }
    Ok(consumed)
}

/// Refuse a resource that holds no bytes of its own.
///
/// Folder and recursive digests are a different question - which files, in
/// what order, under which name - and answering one here would invent a
/// convention no format specifies.
pub(crate) fn reject_container<H: IOBase + ?Sized>(handle: &H) -> Result<()> {
    if !handle.is_container() {
        return Ok(());
    }
    Err(Error::NotAtomic {
        operation: "digest",
        kind: handle.kind().as_str(),
        path: handle.url().map_or_else(Default::default, |url| {
            smol_str::SmolStr::new(url.to_string())
        }),
    })
}

/// A reader that digests the bytes it passes through.
///
/// Bytes reach the caller unchanged; the digest is a side effect of the read
/// that already had to happen, so a payload being moved is hashed without a
/// second pass over it.
///
/// ```
/// use std::io::Read;
///
/// use yggdryl::{DigestAlgorithm, xxhash};
///
/// # fn main() -> yggdryl::Result<()> {
/// let mut reader = xxhash::reader(b"AAPL,187.23".as_slice(), DigestAlgorithm::Xxh3_64);
/// let mut moved = Vec::new();
/// reader.read_to_end(&mut moved)?;
///
/// assert_eq!(moved, b"AAPL,187.23");
/// assert_eq!(reader.as_digest(), DigestAlgorithm::Xxh3_64.digest(&moved));
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct DigestReader<R: Read> {
    source: R,
    digester: Digester,
}

impl<R: Read> DigestReader<R> {
    /// Wrap a reader so every byte read is also digested.
    pub fn new(source: R, algorithm: DigestAlgorithm) -> Self {
        Self {
            source,
            digester: algorithm.digester(),
        }
    }

    /// Return the algorithm this reader computes.
    pub const fn algorithm(&self) -> DigestAlgorithm {
        self.digester.algorithm()
    }

    /// Answer the digest of everything read so far.
    pub fn as_digest(&self) -> Digest {
        self.digester.as_digest()
    }

    /// Give the wrapped reader back.
    pub fn into_inner(self) -> R {
        self.source
    }
}

impl<R: Read> Read for DigestReader<R> {
    fn read(&mut self, target: &mut [u8]) -> std::io::Result<usize> {
        let read = self.source.read(target)?;
        self.digester.write_bytes(&target[..read]);
        Ok(read)
    }
}

/// A writer that digests the bytes it tees through.
///
/// ```
/// use std::io::Write;
///
/// use yggdryl::{DigestAlgorithm, xxhash};
///
/// # fn main() -> yggdryl::Result<()> {
/// let mut writer = xxhash::writer(Vec::new(), DigestAlgorithm::Xxh64);
/// writer.write_all(b"AAPL,187.23")?;
///
/// assert_eq!(writer.as_digest(), DigestAlgorithm::Xxh64.digest(b"AAPL,187.23"));
/// assert_eq!(writer.into_inner(), b"AAPL,187.23");
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct DigestWriter<W: Write> {
    target: W,
    digester: Digester,
}

impl<W: Write> DigestWriter<W> {
    /// Wrap a writer so every byte written is also digested.
    pub fn new(target: W, algorithm: DigestAlgorithm) -> Self {
        Self {
            target,
            digester: algorithm.digester(),
        }
    }

    /// Return the algorithm this writer computes.
    pub const fn algorithm(&self) -> DigestAlgorithm {
        self.digester.algorithm()
    }

    /// Answer the digest of everything written so far.
    pub fn as_digest(&self) -> Digest {
        self.digester.as_digest()
    }

    /// Give the wrapped writer back.
    pub fn into_inner(self) -> W {
        self.target
    }
}

impl<W: Write> Write for DigestWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let written = self.target.write(bytes)?;
        self.digester.write_bytes(&bytes[..written]);
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.target.flush()
    }
}
