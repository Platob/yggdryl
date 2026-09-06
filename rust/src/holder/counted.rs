//! Counting what a stack asks of the handle underneath it.
//!
//! [`IOBase`] is the one boundary every layer above it - codings, records,
//! media, page caches, table formats, the bindings - reaches storage through,
//! and on a remote store each of those calls is a round trip. [`Counted`]
//! turns that cost into a number: it wraps one handle, forwards every call to
//! it unchanged, and tallies each by name. Put it *under* the layer being
//! measured and the tally is exactly what that layer asked of storage.
//!
//! ```
//! use std::sync::Arc;
//!
//! use yggdryl::IOBase;
//! use yggdryl::holder::Buffer;
//! use yggdryl::holder::counted::{Call, Counted};
//!
//! # fn main() -> yggdryl::Result<()> {
//! let mut source = Buffer::new();
//! source.write_all_bytes(b"AAPL,187.23\n")?;
//!
//! let counted = Counted::new(source);
//! let calls = Arc::clone(counted.calls());
//!
//! // A whole read is one call, whatever the value's length.
//! assert_eq!(counted.read_all_bytes()?.len(), 12);
//! assert_eq!(calls.get(Call::ReadAllBytes), 1);
//! assert_eq!(calls.total(), 1);
//!
//! // And the tally says which call, so a read that became two is legible.
//! assert_eq!(counted.counts().to_string(), "read_all_bytes=1");
//! # Ok(())
//! # }
//! ```
//!
//! # Two instruments, two questions
//!
//! This counts **how many times a stack asks storage**. The S3 backend's own
//! `StatsSnapshot` (behind the `s3` feature) counts **how many requests
//! storage then makes**. A layer that asks twice for what one call
//! answers is caught here; a backend that answers one call with two requests
//! is caught there. Both are exact numbers, so both belong in assertions
//! rather than in intentions.
//!
//! # What it forwards, and what it leaves alone
//!
//! `Counted` mirrors the surface a *backend* implements: the positional
//! primitives, the whole and ranged reads and writes, the metadata answers,
//! the lifecycle pair, and the three navigation methods. Every one of those is
//! tallied and passed straight through, so the wrapper changes nothing.
//!
//! It deliberately does **not** forward the derived defaults - `glob`,
//! `partitions`, `children_where`, `copy_into`, `read_scalar`, `cursor`,
//! `reader_at`, `buffered`, and the rest. Those run against the wrapper, so
//! what the tally records is the calls they *decompose into*, which is the
//! thing worth knowing: a partition scan that costs one listing and a scan
//! that costs one call per file read the same from outside and differently
//! here.
//!
//! The one edge is navigation. [`IOBase::parent`] and
//! [`IOBase::child_by_path`] answer with a [`Holder`], which is the wrapped
//! backend's own handle rather than another `Counted`, so what a caller does
//! *through a child* is not in this tally. The call that produced the child is.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::holder::Holder;
use crate::{ByteStream, IOBase, IOKind, Listing, MediaType, Result, Url};

/// One [`IOBase`] method, as a tally names it.
///
/// The list is the surface a backend implements: everything else on the trait
/// is derived from these, and a derived call shows up as the calls it makes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub enum Call {
    /// [`IOBase::pread`].
    Pread,
    /// [`IOBase::pstream_bytes`].
    PstreamBytes,
    /// [`IOBase::read_all_bytes`].
    ReadAllBytes,
    /// [`IOBase::read_range_bytes`].
    ReadRangeBytes,
    /// [`IOBase::read_digest`].
    ReadDigest,
    /// [`IOBase::read_range_digest`].
    ReadRangeDigest,
    /// [`IOBase::pwrite`].
    Pwrite,
    /// [`IOBase::write_all_bytes`].
    WriteAllBytes,
    /// [`IOBase::append_bytes`].
    AppendBytes,
    /// [`IOBase::reserve`].
    Reserve,
    /// [`IOBase::truncate`].
    Truncate,
    /// [`IOBase::set_media_type`].
    SetMediaType,
    /// [`IOBase::clear`].
    Clear,
    /// [`IOBase::remove`].
    Remove,
    /// [`IOBase::size`].
    Size,
    /// [`IOBase::capacity`].
    Capacity,
    /// [`IOBase::url`].
    Url,
    /// [`IOBase::bound_location`].
    BoundLocation,
    /// [`IOBase::media_type`].
    MediaType,
    /// [`IOBase::kind`].
    Kind,
    /// [`IOBase::is_container`].
    IsContainer,
    /// [`IOBase::is_atomic`].
    IsAtomic,
    /// [`IOBase::is_tabular`].
    IsTabular,
    /// [`IOBase::parent`].
    Parent,
    /// [`IOBase::child_by_path`].
    ChildByPath,
    /// [`IOBase::ls`].
    Ls,
    /// [`IOBase::flush`].
    Flush,
    /// [`IOBase::open`].
    Open,
    /// [`IOBase::opened`].
    Opened,
    /// [`IOBase::closed`].
    Closed,
    /// [`IOBase::close`].
    Close,
}

/// What a call is for, so a tally reads as five numbers rather than thirty-one.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Group {
    /// Anything that transfers bytes out of the store.
    Read,
    /// Anything that changes what the store holds.
    Write,
    /// A question about the value that moves none of it.
    Metadata,
    /// Navigating to a neighbour, or listing one.
    Navigation,
    /// The open/close pair and the flush between them.
    Lifecycle,
}

impl Call {
    /// Every call a tally can hold, in the order a snapshot renders them.
    pub const ALL: [Self; Self::COUNT] = [
        Self::Pread,
        Self::PstreamBytes,
        Self::ReadAllBytes,
        Self::ReadRangeBytes,
        Self::ReadDigest,
        Self::ReadRangeDigest,
        Self::Pwrite,
        Self::WriteAllBytes,
        Self::AppendBytes,
        Self::Reserve,
        Self::Truncate,
        Self::SetMediaType,
        Self::Clear,
        Self::Remove,
        Self::Size,
        Self::Capacity,
        Self::Url,
        Self::BoundLocation,
        Self::MediaType,
        Self::Kind,
        Self::IsContainer,
        Self::IsAtomic,
        Self::IsTabular,
        Self::Parent,
        Self::ChildByPath,
        Self::Ls,
        Self::Flush,
        Self::Open,
        Self::Opened,
        Self::Closed,
        Self::Close,
    ];

    /// How many distinct calls a tally holds.
    pub const COUNT: usize = 31;

    /// The method's name, spelled as the trait spells it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Pread => "pread",
            Self::PstreamBytes => "pstream_bytes",
            Self::ReadAllBytes => "read_all_bytes",
            Self::ReadRangeBytes => "read_range_bytes",
            Self::ReadDigest => "read_digest",
            Self::ReadRangeDigest => "read_range_digest",
            Self::Pwrite => "pwrite",
            Self::WriteAllBytes => "write_all_bytes",
            Self::AppendBytes => "append_bytes",
            Self::Reserve => "reserve",
            Self::Truncate => "truncate",
            Self::SetMediaType => "set_media_type",
            Self::Clear => "clear",
            Self::Remove => "remove",
            Self::Size => "size",
            Self::Capacity => "capacity",
            Self::Url => "url",
            Self::BoundLocation => "bound_location",
            Self::MediaType => "media_type",
            Self::Kind => "kind",
            Self::IsContainer => "is_container",
            Self::IsAtomic => "is_atomic",
            Self::IsTabular => "is_tabular",
            Self::Parent => "parent",
            Self::ChildByPath => "child_by_path",
            Self::Ls => "ls",
            Self::Flush => "flush",
            Self::Open => "open",
            Self::Opened => "opened",
            Self::Closed => "closed",
            Self::Close => "close",
        }
    }

    /// What this call is for.
    #[must_use]
    pub const fn group(self) -> Group {
        match self {
            Self::Pread
            | Self::PstreamBytes
            | Self::ReadAllBytes
            | Self::ReadRangeBytes
            | Self::ReadDigest
            | Self::ReadRangeDigest => Group::Read,
            Self::Pwrite
            | Self::WriteAllBytes
            | Self::AppendBytes
            | Self::Reserve
            | Self::Truncate
            | Self::SetMediaType
            | Self::Clear
            | Self::Remove => Group::Write,
            Self::Size
            | Self::Capacity
            | Self::Url
            | Self::BoundLocation
            | Self::MediaType
            | Self::Kind
            | Self::IsContainer
            | Self::IsAtomic
            | Self::IsTabular => Group::Metadata,
            Self::Parent | Self::ChildByPath | Self::Ls => Group::Navigation,
            Self::Flush | Self::Open | Self::Opened | Self::Closed | Self::Close => {
                Group::Lifecycle
            }
        }
    }

    /// Where this call's counter lives.
    const fn slot(self) -> usize {
        self as usize
    }
}

impl std::fmt::Display for Call {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.name())
    }
}

/// The live counters a [`Counted`] writes into.
///
/// Shared behind an [`Arc`], so a caller keeps a reading handle after moving
/// the wrapper into whatever stack is being measured - which is the usual
/// shape, since the wrapper ends up several layers down.
pub struct Calls {
    counts: [AtomicU64; Call::COUNT],
}

impl Default for Calls {
    fn default() -> Self {
        Self {
            counts: std::array::from_fn(|_| AtomicU64::new(0)),
        }
    }
}

impl Calls {
    /// A fresh tally, with every counter at zero.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// How many times `call` has been made.
    #[must_use]
    pub fn get(&self, call: Call) -> u64 {
        self.counts[call.slot()].load(Ordering::Relaxed)
    }

    /// How many calls in `group` have been made.
    #[must_use]
    pub fn group(&self, group: Group) -> u64 {
        Call::ALL
            .iter()
            .filter(|call| call.group() == group)
            .map(|call| self.get(*call))
            .sum()
    }

    /// How many calls of any kind have been made.
    #[must_use]
    pub fn total(&self) -> u64 {
        Call::ALL.iter().map(|call| self.get(*call)).sum()
    }

    /// Take the counters as they stand.
    #[must_use]
    pub fn snapshot(&self) -> CallCounts {
        CallCounts {
            counts: Call::ALL
                .iter()
                .map(|call| (*call, self.get(*call)))
                .filter(|(_, count)| *count > 0)
                .collect(),
        }
    }

    /// Set every counter back to zero.
    ///
    /// What a benchmark does between the setup it does not want to count and
    /// the operation it does.
    pub fn reset(&self) {
        for counter in &self.counts {
            counter.store(0, Ordering::Relaxed);
        }
    }

    /// Record one call.
    fn record(&self, call: Call) {
        self.counts[call.slot()].fetch_add(1, Ordering::Relaxed);
    }
}

impl std::fmt::Debug for Calls {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.snapshot(), formatter)
    }
}

/// The counters at one moment, holding only what was actually called.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct CallCounts {
    counts: Vec<(Call, u64)>,
}

impl CallCounts {
    /// How many times `call` was made.
    #[must_use]
    pub fn get(&self, call: Call) -> u64 {
        self.counts
            .iter()
            .find(|(held, _)| *held == call)
            .map_or(0, |(_, count)| *count)
    }

    /// How many calls in `group` were made.
    #[must_use]
    pub fn group(&self, group: Group) -> u64 {
        self.counts
            .iter()
            .filter(|(call, _)| call.group() == group)
            .map(|(_, count)| *count)
            .sum()
    }

    /// How many calls of any kind were made.
    #[must_use]
    pub fn total(&self) -> u64 {
        self.counts.iter().map(|(_, count)| *count).sum()
    }

    /// Whether nothing was called at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.counts.is_empty()
    }

    /// Each call that was made, with its count, in trait order.
    pub fn iter(&self) -> impl Iterator<Item = (Call, u64)> + '_ {
        self.counts.iter().copied()
    }
}

/// `read_all_bytes=1 size=2`, or `none` when nothing was called.
impl std::fmt::Display for CallCounts {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.counts.is_empty() {
            return formatter.write_str("none");
        }
        for (index, (call, count)) in self.counts.iter().enumerate() {
            if index > 0 {
                formatter.write_str(" ")?;
            }
            write!(formatter, "{call}={count}")?;
        }
        Ok(())
    }
}

impl std::fmt::Debug for CallCounts {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "CallCounts({self})")
    }
}

/// One handle, plus a tally of everything asked of it.
///
/// See the [module documentation](self) for what it forwards and what it
/// leaves to decompose.
pub struct Counted<H> {
    handle: H,
    calls: Arc<Calls>,
}

impl<H> Counted<H> {
    /// Count everything asked of `handle`, from zero.
    #[must_use]
    pub fn new(handle: H) -> Self {
        Self {
            handle,
            calls: Arc::new(Calls::default()),
        }
    }

    /// Count into a tally that already exists.
    ///
    /// Two handles sharing one tally add up, which is how a stack spanning
    /// several objects - a table and its data files - is measured as one
    /// number.
    #[must_use]
    pub fn with_calls(handle: H, calls: Arc<Calls>) -> Self {
        Self { handle, calls }
    }

    /// The live counters, to keep a reading handle on.
    #[must_use]
    pub fn calls(&self) -> &Arc<Calls> {
        &self.calls
    }

    /// The counters as they stand.
    #[must_use]
    pub fn counts(&self) -> CallCounts {
        self.calls.snapshot()
    }

    /// Set every counter back to zero.
    pub fn reset(&self) {
        self.calls.reset();
    }

    /// The handle underneath.
    pub const fn handle(&self) -> &H {
        &self.handle
    }

    /// The handle underneath, to write through.
    pub const fn handle_mut(&mut self) -> &mut H {
        &mut self.handle
    }

    /// Give the handle back, dropping the tally.
    #[must_use]
    pub fn into_inner(self) -> H {
        self.handle
    }

    /// Record one call.
    fn record(&self, call: Call) -> &H {
        self.calls.record(call);
        &self.handle
    }

    /// Record one call that writes.
    fn record_mut(&mut self, call: Call) -> &mut H {
        self.calls.record(call);
        &mut self.handle
    }
}

/// The record surfaces run against the wrapper rather than through it.
///
/// This is the whole point of the instrument: `impl_default_iomedia!` makes
/// [`IOMedia::as_io_base`](crate::IOMedia::as_io_base) answer with the wrapper,
/// so a Parquet footer read, a schema question, or a row count reaches storage
/// through the tally and is counted. Delegating them to the handle underneath,
/// which is what a wrapper that wanted to be invisible would do, would put
/// every one of those calls below the counter where they cannot be seen.
impl<H: IOBase> crate::IOMedia for Counted<H> {
    crate::impl_default_iomedia!();
}

/// Every method here counts once and then answers exactly what the wrapped
/// handle answers, so the wrapper is invisible except for the tally.
impl<H: IOBase> IOBase for Counted<H> {
    fn pread(&self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        self.record(Call::Pread).pread(offset, buffer)
    }

    fn pstream_bytes(&self, position: u64, batch_size: usize) -> Result<ByteStream<'_>> {
        self.record(Call::PstreamBytes)
            .pstream_bytes(position, batch_size)
    }

    fn read_all_bytes(&self) -> Result<Vec<u8>> {
        self.record(Call::ReadAllBytes).read_all_bytes()
    }

    fn read_range_bytes(&self, offset: u64, length: usize) -> Result<Vec<u8>> {
        self.record(Call::ReadRangeBytes)
            .read_range_bytes(offset, length)
    }

    fn read_digest(&self, algorithm: crate::DigestAlgorithm) -> Result<crate::Digest> {
        self.record(Call::ReadDigest).read_digest(algorithm)
    }

    fn read_range_digest(
        &self,
        offset: u64,
        length: usize,
        algorithm: crate::DigestAlgorithm,
    ) -> Result<crate::Digest> {
        self.record(Call::ReadRangeDigest)
            .read_range_digest(offset, length, algorithm)
    }

    fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> Result<usize> {
        self.record_mut(Call::Pwrite).pwrite(offset, bytes)
    }

    fn write_all_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.record_mut(Call::WriteAllBytes).write_all_bytes(bytes)
    }

    fn append_bytes(&mut self, bytes: &[u8]) -> Result<u64> {
        self.record_mut(Call::AppendBytes).append_bytes(bytes)
    }

    fn size(&self) -> u64 {
        self.record(Call::Size).size()
    }

    fn capacity(&self) -> u64 {
        self.record(Call::Capacity).capacity()
    }

    fn reserve(&mut self, capacity: u64) -> Result<()> {
        self.record_mut(Call::Reserve).reserve(capacity)
    }

    fn truncate(&mut self, size: u64) -> Result<()> {
        self.record_mut(Call::Truncate).truncate(size)
    }

    fn url(&self) -> Option<&Url> {
        self.record(Call::Url).url()
    }

    fn bound_location(&self) -> Option<&crate::holder::fs::BoundLocation> {
        self.record(Call::BoundLocation).bound_location()
    }

    fn media_type(&self) -> &MediaType {
        self.record(Call::MediaType).media_type()
    }

    fn set_media_type(&mut self, media_type: MediaType) {
        self.record_mut(Call::SetMediaType)
            .set_media_type(media_type);
    }

    fn kind(&self) -> IOKind {
        self.record(Call::Kind).kind()
    }

    fn is_container(&self) -> bool {
        self.record(Call::IsContainer).is_container()
    }

    fn is_atomic(&self) -> bool {
        self.record(Call::IsAtomic).is_atomic()
    }

    fn is_tabular(&self) -> bool {
        self.record(Call::IsTabular).is_tabular()
    }

    fn parent(&self) -> Option<Holder> {
        self.record(Call::Parent).parent()
    }

    fn child_by_path(&self, path: &str) -> Result<Holder> {
        self.record(Call::ChildByPath).child_by_path(path)
    }

    fn ls(&self, recursive: bool, include_private: bool) -> Listing {
        self.record(Call::Ls).ls(recursive, include_private)
    }

    fn flush(&mut self) -> Result<()> {
        self.record_mut(Call::Flush).flush()
    }

    fn open(&mut self) -> Result<()> {
        self.record_mut(Call::Open).open()
    }

    fn opened(&self) -> bool {
        self.record(Call::Opened).opened()
    }

    fn closed(&self) -> bool {
        self.record(Call::Closed).closed()
    }

    fn close(&mut self) -> Result<()> {
        self.record_mut(Call::Close).close()
    }

    fn clear(&mut self) -> Result<()> {
        self.record_mut(Call::Clear).clear()
    }

    fn remove(&mut self, recursive: bool) -> Result<()> {
        self.record_mut(Call::Remove).remove(recursive)
    }
}

impl<H: std::fmt::Debug> std::fmt::Debug for Counted<H> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Counted")
            .field("handle", &self.handle)
            .field("calls", &self.calls.snapshot())
            .finish()
    }
}
