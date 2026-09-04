//! A transparent handle that digests bytes as they are written.

use std::sync::{Mutex, PoisonError};

use crate::IOBase;
use crate::{Digest, DigestAlgorithm, Digester, Result};

/// A byte handle that hashes what is written through it.
///
/// Reads, writes, listing, media type, and every record surface are the
/// wrapped handle's, unchanged. What the wrapper adds is a running digest of
/// the value, so the common case - a file written once from the beginning, or
/// appended to in order - answers [`IOBase::read_digest`] without reading the
/// bytes back.
///
/// ```
/// use yggdryl::{IOBase, holder::Buffer};
/// use yggdryl::xxhash::Hashed;
/// use yggdryl::DigestAlgorithm;
///
/// # fn main() -> yggdryl::Result<()> {
/// let mut handle = Hashed::new(Buffer::new(), DigestAlgorithm::Xxh3_64);
/// handle.write_all_bytes(b"symbol,price\n")?;
/// handle.append_bytes(b"AAPL,187.23\n")?;
/// handle.flush()?;
///
/// // Answered from the running state; the bytes are never read back.
/// assert_eq!(
///     handle.read_digest(DigestAlgorithm::Xxh3_64)?,
///     DigestAlgorithm::Xxh3_64.digest(b"symbol,price\nAAPL,187.23\n"),
/// );
/// # Ok(())
/// # }
/// ```
///
/// # When the running state is used
///
/// The state covers writes that are strictly sequential from offset 0: a whole
/// write, repeated appends, a streamed record write. Any positional write that
/// is not the running append point, plus [`IOBase::clear`],
/// [`IOBase::remove`], and [`IOBase::truncate`], marks it stale.
///
/// Staleness is neither an error nor silent corruption. A stale state makes
/// [`IOBase::read_digest`] re-stream the handle and re-arm, so the answer is
/// identical either way and only the cost differs. The state is also only
/// consulted when it covers the whole value: a handle that stages writes until
/// [`IOBase::flush`] is streamed until the staged bytes are published, which is
/// what makes pending writes count only after a flush.
pub struct Hashed<H: IOBase> {
    handle: H,
    algorithm: DigestAlgorithm,
    seed: u64,
    running: Mutex<Running>,
}

/// The digest of a sequential prefix, and how far it reaches.
#[derive(Debug)]
struct Running {
    /// `None` once a write the state cannot follow has landed.
    digester: Option<Digester>,
    /// Bytes fed, always a prefix starting at offset 0.
    covered: u64,
}

impl<H: IOBase> Hashed<H> {
    /// Wrap a handle so writes through it are digested, without touching it.
    pub fn new(handle: H, algorithm: DigestAlgorithm) -> Self {
        Self {
            handle,
            algorithm,
            seed: 0,
            running: Mutex::new(Running {
                digester: Some(algorithm.digester()),
                covered: 0,
            }),
        }
    }

    /// Return this handle digesting under an explicit seed.
    ///
    /// The running state restarts, because a seed is part of the digest rather
    /// than a setting beside it.
    #[must_use]
    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self.running = Mutex::new(Running {
            digester: Some(self.algorithm.digester_with_seed(seed)),
            covered: 0,
        });
        self
    }

    /// Return the algorithm the running state computes.
    pub const fn algorithm(&self) -> DigestAlgorithm {
        self.algorithm
    }

    /// Return the seed writes are digested under.
    pub const fn seed(&self) -> u64 {
        self.seed
    }

    /// Borrow the wrapped handle, which holds the bytes.
    pub const fn handle(&self) -> &H {
        &self.handle
    }

    /// Consume this wrapper and return the handle it wraps.
    pub fn into_handle(self) -> H {
        self.handle
    }

    /// Build a state under this wrapper's algorithm and seed.
    fn fresh(&self) -> Digester {
        self.algorithm.digester_with_seed(self.seed)
    }

    /// Take the running state, whatever a poisoned lock left behind.
    fn state(&self) -> std::sync::MutexGuard<'_, Running> {
        self.running.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Forget the running state so the next digest re-streams and re-arms.
    fn invalidate(&mut self) {
        let mut running = self.state();
        running.digester = None;
        running.covered = 0;
    }
}

impl<H: IOBase> std::fmt::Debug for Hashed<H> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let running = self.state();
        formatter
            .debug_struct("Hashed")
            .field("algorithm", &self.algorithm)
            .field("seed", &self.seed)
            .field("live", &running.digester.is_some())
            .field("covered", &running.covered)
            .finish_non_exhaustive()
    }
}

impl<H: IOBase> crate::IOMedia for Hashed<H> {
    crate::delegate_iomedia!(handle);
}

impl<H: IOBase> IOBase for Hashed<H> {
    // Everything the running digest does not change is the wrapped handle's
    // answer. What the list leaves out is exactly what this wrapper owns: the
    // positional write that may extend the running prefix, the three
    // operations that can drop bytes the state has already folded in, and the
    // digest read itself.
    crate::delegate_iobase!(handle: pread, pstream_bytes, size, capacity, reserve, url,
        media_type, set_media_type, flush, open, opened, close, parent, child_by_path, ls,
        kind, is_atomic, is_tabular, is_io);

    /// Write through, extending the running digest when the write is the next
    /// sequential byte and dropping it when it is not.
    fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> Result<usize> {
        let restart = offset == 0;
        // The guard is taken from the field rather than through `self`, so the
        // write below borrows the handle independently of the state.
        let mut running = self.running.lock().unwrap_or_else(PoisonError::into_inner);
        let continues = running.digester.is_some() && running.covered == offset;
        // The write lands first: a failed write must not move the state.
        let written = self.handle.pwrite(offset, bytes)?;
        if restart || continues {
            let mut digester = match running.digester.take() {
                Some(digester) if !restart => digester,
                _ => self.algorithm.digester_with_seed(self.seed),
            };
            digester.write_bytes(&bytes[..written.min(bytes.len())]);
            running.covered = offset.saturating_add(written as u64);
            running.digester = Some(digester);
        } else {
            running.digester = None;
            running.covered = 0;
        }
        Ok(written)
    }

    /// Resize the value and drop the running digest.
    ///
    /// A truncation removes bytes the state has already folded in, and the
    /// fold cannot be undone.
    fn truncate(&mut self, size: u64) -> Result<()> {
        self.handle.truncate(size)?;
        self.invalidate();
        Ok(())
    }

    /// Empty the value and drop the running digest.
    fn clear(&mut self) -> Result<()> {
        self.handle.clear()?;
        self.invalidate();
        Ok(())
    }

    /// Delete the value and drop the running digest.
    fn remove(&mut self, recursive: bool) -> Result<()> {
        self.handle.remove(recursive)?;
        self.invalidate();
        Ok(())
    }

    /// Answer from the running state when it covers the whole value.
    ///
    /// A different algorithm than this wrapper computes, or a state that is
    /// stale or short of the value's size, streams the handle instead and
    /// re-arms. The answer is identical either way.
    fn read_digest(&self, algorithm: DigestAlgorithm) -> Result<Digest> {
        if algorithm != self.algorithm {
            return super::stream::read_digest(&self.handle, algorithm);
        }
        // Before the state, not after: a container's running state is live and
        // empty, and its size is zero, so it would otherwise answer the digest
        // of no bytes instead of naming the kind.
        super::stream::reject_container(&self.handle)?;
        let mut running = self.state();
        if let Some(digester) = &running.digester {
            if running.covered == self.handle.size() {
                return Ok(digester.as_digest());
            }
        }
        let mut digester = self.fresh();
        let covered = super::stream::feed_handle(&self.handle, &mut digester)?;
        let digest = digester.as_digest();
        running.digester = Some(digester);
        running.covered = covered;
        Ok(digest)
    }
}
