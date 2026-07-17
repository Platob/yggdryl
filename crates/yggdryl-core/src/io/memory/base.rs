//! [`IOBase`] — positioned (random-access) byte read/write, the base of the I/O trait family.

use super::{IOCursor, IOSlice, IoError};
use crate::headers::Headers;
use crate::io::{IOKind, IOMode};
use crate::uri::Uri;

/// The **static default URI** of an in-memory source — the stable synthetic `mem://heap`
/// (deterministic; the real allocation address is deliberately not leaked). Parsed once into
/// this process-wide static; an accessor clones it (a couple of small string clones), never
/// re-parses.
pub(crate) static DEFAULT_URI: std::sync::LazyLock<Uri> = std::sync::LazyLock::new(|| {
    Uri::parse_str("mem://heap").expect("the static mem://heap URI parses")
});

/// The shared synthetic `mem://heap` address (parsed once) — what any in-memory source
/// reports from [`uri`](IOBase::uri) unless it has been re-addressed by [`join`](IOBase::join).
pub(crate) fn default_uri() -> &'static Uri {
    &DEFAULT_URI
}

/// The element count bulk operations stage per stack chunk — sized so the largest staged
/// chunk (`i64`: 256 × 8 = 2 KiB) stays comfortably on the stack while the per-chunk convert
/// loop is long enough for LLVM to vectorize.
const BULK_CHUNK: usize = 256;

/// The always-empty child stream of a **leaf** source — what a source with no children of
/// its own (a [`Heap`](super::Heap), a wrapper view, a raw mapped file) declares for
/// [`IOBase::Children`] / [`IOBase::Walk`] and returns from [`ls`](IOBase::ls).
pub type NoChildren<T> = std::iter::Empty<Result<T, IoError>>;

/// The guided error for a removal on a source with no removable backing.
fn unremovable(uri: &Uri, method: &str) -> IoError {
    IoError::FileIo {
        op: "remove",
        path: uri.to_string(),
        detail: format!(
            "{method} needs a removable backing; this source has none — address a \
             filesystem node (e.g. LocalIO) instead"
        ),
    }
}

/// Random-access byte storage addressed by absolute offset — no cursor. This is the base
/// every I/O **source** shares: [`IOCursor`](super::IOCursor) adds a moving position on top, and
/// [`IOSlice`](super::IOSlice) adds bounded sub-views.
///
/// # Shape
///
/// - **Size** — [`byte_size`](IOBase::byte_size) / [`bit_size`](IOBase::bit_size).
/// - **Capacity** — [`capacity`](IOBase::capacity) / [`reserve`](IOBase::reserve), the `Vec`-like
///   amortized-growth hooks (a source with no spare capacity, e.g. a memory-map, reports its size
///   and ignores `reserve`).
/// - **Byte-array primitives** — [`pread_byte_array`](IOBase::pread_byte_array) /
///   [`pwrite_byte_array`](IOBase::pwrite_byte_array): the two methods a source must implement.
/// - **Typed accessors** — `byte` / `bit` / `i32` / `i64`, positioned little-endian
///   read/write built on the primitives (`pread_byte`, `pread_bit`, `pread_i32`, `pread_i64`
///   and their `pwrite_*` twins).
///
/// DESIGN: the two **primitives** are *infallible* (`-> usize`), because the physical layer is
/// in-memory: a read past the end simply returns fewer bytes (0 at the end) and a write past the
/// end grows the storage, zero-filling any gap. The fallible surface is the **full** and **typed**
/// helpers built on them, whose contract — *fill exactly this many* — can be broken by the end of
/// the data. Signatures take `&[u8]` / `&mut [u8]` and native integers, never a foreign buffer
/// type, so the storage underneath stays an implementation detail. Bit addressing is **LSB-first**
/// (bit `i` is bit `i % 8` of byte `i / 8`, least-significant first), matching Arrow validity
/// bitmaps.
///
/// # The graph surface — every source is a node
///
/// `IOBase` is also the **central access path**: like [`uri`](IOBase::uri) addresses a
/// source, the graph methods place it in an IO graph. [`ls`](IOBase::ls) /
/// [`ls_recursive`](IOBase::ls_recursive) stream children **of the same type** (a leaf
/// source streams nothing — [`NoChildren`]), [`name`](IOBase::name) /
/// [`parent`](IOBase::parent) navigate, and [`rm`](IOBase::rm) / [`rmfile`](IOBase::rmfile)
/// / [`rmdir`](IOBase::rmdir) remove. A **container** node (a directory, an object-store
/// prefix) serves byte I/O as a **memory tree** over its children via the generic
/// [`tree_byte_size`](IOBase::tree_byte_size) /
/// [`tree_pread_byte_array`](IOBase::tree_pread_byte_array) /
/// [`tree_pwrite_byte_array`](IOBase::tree_pwrite_byte_array) — written once here so every
/// filesystem family (local today; s3 / azure later) inherits the same behavior.
pub trait IOBase: Sized {
    /// The total length in bytes.
    fn byte_size(&self) -> u64;

    /// The total length in bits — `byte_size() * 8`.
    fn bit_size(&self) -> u64 {
        self.byte_size() * 8
    }

    /// Whether the storage is empty (`byte_size() == 0`).
    fn is_empty(&self) -> bool {
        self.byte_size() == 0
    }

    /// The number of bytes the storage can hold before it must reallocate — like
    /// [`Vec::capacity`]. A source with no distinct spare capacity (a fixed map, a view) returns
    /// [`byte_size`](IOBase::byte_size); a growable one (e.g. [`Heap`](super::Heap)) overrides it.
    fn capacity(&self) -> u64 {
        self.byte_size()
    }

    /// Reserves capacity for at least `additional` more bytes past the current
    /// [`byte_size`](IOBase::byte_size), amortizing later writes — like [`Vec::reserve`]. The
    /// default is a **no-op** (a fixed source cannot grow); a growable source overrides it.
    fn reserve(&mut self, additional: u64) {
        let _ = additional;
    }

    /// The spare room already allocated — `capacity() - byte_size()`, the bytes that can be
    /// appended before the next reallocation. The planning counterpart of
    /// [`reserve`](IOBase::reserve).
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let mut h = Heap::with_capacity(64);
    /// h.pwrite_byte_array(0, &[0; 16]);
    /// assert!(h.spare_capacity() >= 48);
    /// ```
    fn spare_capacity(&self) -> u64 {
        self.capacity().saturating_sub(self.byte_size())
    }

    /// Reserves capacity for **exactly** `additional` more bytes — like [`Vec::reserve_exact`]:
    /// no amortized over-allocation, for a caller that knows the final size and wants no spare
    /// tail. The default is a no-op (a fixed source cannot grow).
    fn reserve_exact(&mut self, additional: u64) {
        let _ = additional;
    }

    /// **Checked** reservation of at least `additional` more bytes — like [`Vec::try_reserve`]:
    /// where [`reserve`](IOBase::reserve) would **abort the process** on overflow or allocator
    /// failure, this returns the guided [`IoError::CapacityOverflow`] instead, so a hostile or
    /// miscomputed size is a recoverable error. The default is `Ok` (a fixed source reserves
    /// nothing); a growable source overrides it with a real checked reservation.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase, IoError};
    ///
    /// let mut h = Heap::new();
    /// h.try_reserve(1024).unwrap();               // fine — and now pre-allocated
    /// assert!(h.capacity() >= 1024);
    /// let err = h.try_reserve(u64::MAX).unwrap_err(); // recoverable, never an abort
    /// assert!(matches!(err, IoError::CapacityOverflow { .. }));
    /// ```
    fn try_reserve(&mut self, additional: u64) -> Result<(), IoError> {
        let _ = additional;
        Ok(())
    }

    /// **Checked exact** reservation — [`try_reserve`](IOBase::try_reserve) without the
    /// amortized over-allocation, like [`Vec::try_reserve_exact`]. The default is `Ok`.
    fn try_reserve_exact(&mut self, additional: u64) -> Result<(), IoError> {
        let _ = additional;
        Ok(())
    }

    /// Ensures the **total** capacity is at least `total` bytes — the absolute-target form of
    /// [`reserve`](IOBase::reserve), for a pipeline that knows how much data is coming. A
    /// no-op when the capacity already suffices; never shrinks.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let mut h = Heap::new();
    /// h.ensure_capacity(4096);
    /// assert!(h.capacity() >= 4096);
    /// h.ensure_capacity(16); // already satisfied — no-op, never shrinks
    /// assert!(h.capacity() >= 4096);
    /// ```
    fn ensure_capacity(&mut self, total: u64) {
        if total > self.capacity() {
            self.reserve(total.saturating_sub(self.byte_size()));
        }
    }

    /// **Checked** [`ensure_capacity`](IOBase::ensure_capacity) — errors with
    /// [`IoError::CapacityOverflow`] instead of aborting when `total` cannot be allocated.
    fn try_ensure_capacity(&mut self, total: u64) -> Result<(), IoError> {
        if total > self.capacity() {
            self.try_reserve(total.saturating_sub(self.byte_size()))?;
        }
        Ok(())
    }

    /// Releases spare capacity back to the allocator, shrinking the allocation toward
    /// [`byte_size`](IOBase::byte_size) — like [`Vec::shrink_to_fit`]. The default is a no-op
    /// (a fixed source has nothing to release).
    fn shrink_to_fit(&mut self) {}

    /// Shrinks the allocation toward `min_capacity` (never below
    /// [`byte_size`](IOBase::byte_size)) — like [`Vec::shrink_to`], keeping a known working
    /// headroom while releasing the rest. The default is a no-op.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let mut h = Heap::with_capacity(4096);
    /// h.pwrite_byte_array(0, &[0; 8]);
    /// h.shrink_to(64);
    /// assert!(h.capacity() >= 8 && h.capacity() <= 4096);
    /// ```
    fn shrink_to(&mut self, min_capacity: u64) {
        let _ = min_capacity;
    }

    /// Builds a source **pre-allocated** for `capacity` bytes — the fast path when the size is
    /// known up front, so the first writes never reallocate. Works on any source that is
    /// `Default` (an empty value plus one [`reserve`](IOBase::reserve)); a source with a cheaper
    /// exact allocation may override it.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let heap = <Heap as IOBase>::with_capacity(4096);
    /// assert!(heap.is_empty());
    /// assert!(heap.capacity() >= 4096);
    /// ```
    fn with_capacity(capacity: u64) -> Self
    where
        Self: Sized + Default,
    {
        let mut source = Self::default();
        source.reserve(capacity);
        source
    }

    /// The [`Uri`] that **addresses** this source — every source is locatable. The default is
    /// the stable synthetic in-memory address `mem://heap` (the `mem` scheme addresses
    /// in-memory sources; deterministic, so tests and logs can rely on it) — an anonymous
    /// in-memory source ([`Heap`](super::Heap)) stores no address and keeps this default. A
    /// source with a real address (a future file/network source) overrides it to return its
    /// own.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// // An in-memory source reports the synthetic mem:// address.
    /// assert_eq!(Heap::new().uri().to_string(), "mem://heap");
    /// assert_eq!(Heap::new().uri().scheme(), Some("mem"));
    /// ```
    fn uri(&self) -> Uri {
        DEFAULT_URI.clone()
    }

    /// The metadata attached to this source — the project-wide [`Headers`] map (HTTP headers,
    /// schema/field metadata, source annotations all live here; never a second map type).
    fn headers(&self) -> &Headers;

    /// Mutable access to the source's [`Headers`] metadata.
    fn headers_mut(&mut self) -> &mut Headers;

    /// How this source may be accessed — see [`IOMode`]. In-memory sources default to
    /// [`IOMode::ReadWrite`]; a source opened otherwise overrides it.
    fn mode(&self) -> IOMode {
        IOMode::ReadWrite
    }

    /// What this source **is** — see [`IOKind`] ([`Heap`](super::Heap) reports
    /// [`IOKind::Heap`]; a file source reports [`IOKind::File`] / [`IOKind::Directory`], or
    /// [`IOKind::Missing`] when nothing exists at its address).
    fn kind(&self) -> IOKind;

    /// Whether this source is a regular **file** — derived from [`kind`](IOBase::kind).
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// assert!(!Heap::new().is_file()); // a heap is IOKind::Heap, not a file
    /// ```
    fn is_file(&self) -> bool {
        self.kind() == IOKind::File
    }

    /// Whether this source is a **directory** — derived from [`kind`](IOBase::kind).
    fn is_dir(&self) -> bool {
        self.kind() == IOKind::Directory
    }

    /// Whether this source's kind is [`Unknown`](IOKind::Unknown) — it exists, but of a type
    /// that is not file / directory / heap.
    fn is_unknown(&self) -> bool {
        self.kind() == IOKind::Unknown
    }

    /// Whether something **exists** at this source's address — anything except
    /// [`Missing`](IOKind::Missing) (so a `File`, `Directory`, live `Heap`, or `Unknown` node
    /// all exist). Leverages [`IOKind::exists`](IOKind::exists).
    fn exists(&self) -> bool {
        self.kind().exists()
    }

    // ---------------------------------------------------------------------------------
    // The graph surface — every source is a node in an IO graph
    // ---------------------------------------------------------------------------------

    /// The streamed one-level child iterator of [`ls`](IOBase::ls) — items are the **same
    /// source type**, so graphs stay homogeneous whatever node you start from. A leaf
    /// source declares [`NoChildren`].
    type Children: Iterator<Item = Result<Self, IoError>>;

    /// The streamed recursive walker of [`ls_recursive`](IOBase::ls_recursive) — same item
    /// type as [`Children`](IOBase::Children).
    type Walk: Iterator<Item = Result<Self, IoError>>;

    /// The node's own name — by default the last segment of its address's path (empty when
    /// the address has none, e.g. the synthetic `mem://heap`); a filesystem node overrides
    /// it with its real file name. The segment is **percent-decoded** so a wrapper over a
    /// spaced path reports `my file.txt`, never the encoded `my%20file.txt`.
    fn name(&self) -> String {
        let uri = self.uri();
        let segment = uri
            .path()
            .rsplit('/')
            .find(|segment| !segment.is_empty())
            .unwrap_or("");
        crate::uri::percent::decode(segment).into_owned()
    }

    /// The parent node, or `None` — the default for a leaf source or a root.
    fn parent(&self) -> Option<Self> {
        None
    }

    /// An iterator over this node's **ancestors**, nearest first — the repeated
    /// [`parent`](IOBase::parent) chain up to the root. The node-graph counterpart of
    /// [`Uri::parents`](crate::uri::Uri::parents).
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let node = Heap::new().join("a/b/c.bin").unwrap();
    /// let uris: Vec<String> = node.parents().map(|p| p.uri().to_string()).collect();
    /// assert_eq!(uris, vec!["mem://heap/a/b", "mem://heap/a", "mem://heap"]);
    /// ```
    fn parents(&self) -> impl Iterator<Item = Self> {
        std::iter::successors(self.parent(), Self::parent)
    }

    /// The child node at `segment` — a **new node of the same kind**, addressed by joining
    /// `segment` onto this node's address with [`Uri::joinpath`](crate::uri::Uri::joinpath),
    /// so navigation composes through the URI: the child's [`parent`](IOBase::parent)
    /// addresses this node again. `segment` may be multi-segment (`"a/b/c"`); an **absolute**
    /// segment (leading `/`) re-roots (the URI merge/join algebra, shared with `Uri`/`Url`).
    ///
    /// Constructing a child touches nothing — it is pure address algebra. Reading or writing
    /// the returned node is what actually creates or opens its backing (e.g. a `LocalIO`
    /// child auto-creates on first write). The default reports that this source has no
    /// navigable child space (a bare byte view or wrapper); [`Heap`](super::Heap) and the
    /// local family ([`LocalIO`](crate::io::local::LocalIO)) build a real child.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let root = Heap::new();
    /// let child = root.join("logs/app.bin").unwrap();
    /// assert_eq!(child.uri().to_string(), "mem://heap/logs/app.bin");
    /// assert_eq!(child.parent().unwrap().uri().to_string(), "mem://heap/logs");
    /// ```
    fn join(&self, segment: &str) -> Result<Self, IoError> {
        let _ = segment;
        Err(IoError::FileIo {
            op: "join",
            path: self.uri().to_string(),
            detail: "this source has no child path space to join onto; address a filesystem \
                     node (LocalIO) or an in-heap node (Heap)"
                .to_string(),
        })
    }

    /// Streams this node's **direct children**, lazily — each item is produced as the
    /// caller pulls, never a pre-collected tree. A leaf source (or a missing / file node)
    /// streams nothing; a real listing failure is a guided [`IoError::FileIo`].
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// assert_eq!(Heap::new().ls().unwrap().count(), 0); // a heap is a leaf
    /// ```
    fn ls(&self) -> Result<Self::Children, IoError>;

    /// Streams the node's **entire subtree** (depth-first), lazily — the recursive
    /// counterpart of [`ls`](IOBase::ls); the bindings expose both through one generic
    /// `ls(recursive=…)` entry point.
    fn ls_recursive(&self) -> Result<Self::Walk, IoError>;

    /// The direct children, collected — the convenience over the streamed
    /// [`ls`](IOBase::ls).
    fn children(&self) -> Result<Vec<Self>, IoError> {
        self.ls()?.collect()
    }

    /// Removes **whatever exists** at this node — a file is unlinked, a directory removed
    /// with its subtree, a missing node is a no-op. The default is the guided refusal of a
    /// source with no removable backing; filesystem families override it.
    fn rm(&self) -> Result<(), IoError> {
        Err(unremovable(&self.uri(), "rm"))
    }

    /// Removes this node **as a file** — a guided error when it is a directory (use
    /// [`rmdir`](IOBase::rmdir)), a no-op when missing. Default: the guided refusal.
    fn rmfile(&self) -> Result<(), IoError> {
        Err(unremovable(&self.uri(), "rmfile"))
    }

    /// Removes this node **as a directory**, recursively — a guided error when it is a
    /// file (use [`rmfile`](IOBase::rmfile)), a no-op when missing. Default: the guided
    /// refusal.
    fn rmdir(&self) -> Result<(), IoError> {
        Err(unremovable(&self.uri(), "rmdir"))
    }

    // ---------------------------------------------------------------------------------
    // The memory tree — a container node served as one contiguous byte region
    // ---------------------------------------------------------------------------------

    /// The **memory-tree size** of a container node: the lazy, streamed sum of every child
    /// block's [`byte_size`](IOBase::byte_size) (a child container recurses through its own
    /// `byte_size`). Nothing is collected and nothing is cached — the size is recomputed
    /// per call from the live tree. Written once here so every filesystem family serves
    /// container sizes identically.
    ///
    /// DESIGN: an **erroring** child (one whose listing yields `Err`) is skipped, so a tree
    /// with an unreadable entry reports the size of the readable remainder rather than
    /// failing. Cycle safety (a directory symlink pointing at an ancestor) is the family's
    /// concern: it belongs to [`blocks`](IOBase::blocks), which a family overrides to keep
    /// the layout acyclic (the local family drops symlinked directories there).
    fn tree_byte_size(&self) -> u64 {
        match self.ls() {
            Ok(children) => children
                .filter_map(Result::ok)
                .map(|child| child.byte_size())
                .sum(),
            Err(_) => 0,
        }
    }

    /// The node's direct children as **name-sorted blocks** — the deterministic order the
    /// memory-tree byte layout uses (listing order is OS-dependent; names are not). One
    /// level is collected and sorted per call, never the whole tree.
    fn blocks(&self) -> Vec<Self> {
        let mut blocks: Vec<Self> = match self.ls() {
            Ok(children) => children.filter_map(Result::ok).collect(),
            Err(_) => Vec::new(),
        };
        blocks.sort_by_key(|block| block.name());
        blocks
    }

    /// **Memory-tree read**: serves [`pread_byte_array`](IOBase::pread_byte_array) for a
    /// container node by reading across its name-sorted child [`blocks`](IOBase::blocks)
    /// as one contiguous byte region — a child container recurses through its own
    /// `pread_byte_array`, so the whole subtree reads as one lazily-computed buffer.
    /// Blocks before `offset` are skipped by size alone; nothing is materialized.
    fn tree_pread_byte_array(&self, offset: u64, buf: &mut [u8]) -> usize {
        let mut done = 0usize;
        let mut block_start = 0u64;
        for block in self.blocks() {
            if done == buf.len() {
                break;
            }
            let block_end = block_start + block.byte_size();
            let read_at = offset + done as u64;
            // Only read where the target lands inside this block. `read_at < block_start`
            // means an earlier block short-read and left a hole — the region is no longer
            // contiguous, so stop (never underflow `read_at - block_start`).
            if read_at < block_start {
                break;
            }
            if read_at < block_end {
                done += block.pread_byte_array(read_at - block_start, &mut buf[done..]);
            }
            block_start = block_end;
        }
        done
    }

    /// **Memory-tree write**: routes [`pwrite_byte_array`](IOBase::pwrite_byte_array) for a
    /// container node across its name-sorted child [`blocks`](IOBase::blocks). A write
    /// inside a block stays **capped at that block's end** (a middle block never grows —
    /// the layout would shift); bytes past the last block grow the **last** block. A
    /// container with no blocks writes nothing (the full writes report the guided fix).
    fn tree_pwrite_byte_array(&mut self, offset: u64, data: &[u8]) -> usize {
        let mut blocks = self.blocks();
        let Some(last) = blocks.len().checked_sub(1) else {
            return 0;
        };
        let mut done = 0usize;
        let mut block_start = 0u64;
        for (i, block) in blocks.iter_mut().enumerate() {
            if done == data.len() {
                break;
            }
            let block_end = block_start + block.byte_size();
            let write_at = offset + done as u64;
            // A hole from an earlier short write breaks contiguity — stop before underflow.
            if write_at < block_start {
                break;
            }
            if write_at < block_end || i == last {
                let chunk_end = if i == last {
                    data.len()
                } else {
                    // `write_at <= block_end` here (write_at < block_end), so the cap is safe.
                    done + ((block_end - write_at) as usize).min(data.len() - done)
                };
                let written =
                    block.pwrite_byte_array(write_at - block_start, &data[done..chunk_end]);
                done += written;
                if done < chunk_end {
                    break; // the block refused (e.g. read-only) — stop, report the shortfall
                }
            }
            block_start = block_end;
        }
        done
    }

    /// **Positioned read** (primitive). Copies up to `buf.len()` bytes starting at `offset` into
    /// `buf`, returning the number copied — `0` at or past the end, a short count near it. Never
    /// moves a cursor.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let data = Heap::from_slice(b"hello world");
    /// let mut buf = [0u8; 5];
    /// assert_eq!(data.pread_byte_array(6, &mut buf), 5);
    /// assert_eq!(&buf, b"world");
    /// assert_eq!(data.pread_byte_array(11, &mut buf), 0); // at the end -> nothing
    /// ```
    fn pread_byte_array(&self, offset: u64, buf: &mut [u8]) -> usize;

    /// **Positioned write** (primitive). Copies `data` in at `offset`, growing the storage (and
    /// zero-filling any gap between the old end and `offset`) as needed. Returns the number of
    /// bytes written — always `data.len()`.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let mut data = Heap::from_slice(b"abc");
    /// assert_eq!(data.pwrite_byte_array(5, b"Z"), 1); // writes past the end, zero-filling the gap
    /// assert_eq!(data.as_slice(), b"abc\0\0Z");
    /// ```
    fn pwrite_byte_array(&mut self, offset: u64, data: &[u8]) -> usize;

    /// **Full positioned read.** Fills *all* of `buf` starting at `offset`, or errors with
    /// [`IoError::UnexpectedEof`] naming the shortfall if fewer bytes remain.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let data = Heap::from_slice(b"hello");
    /// let mut buf = [0u8; 3];
    /// data.pread_exact(1, &mut buf).unwrap();
    /// assert_eq!(&buf, b"ell");
    /// assert!(data.pread_exact(3, &mut [0u8; 5]).is_err()); // only 2 remain
    /// ```
    fn pread_exact(&self, offset: u64, buf: &mut [u8]) -> Result<(), IoError> {
        let read = self.pread_byte_array(offset, buf);
        if read == buf.len() {
            Ok(())
        } else {
            Err(IoError::UnexpectedEof {
                offset: offset + read as u64,
                requested: buf.len(),
                available: read,
            })
        }
    }

    /// **Full positioned write** of *all* of `data` at `offset` — the counterpart of
    /// [`pread_exact`](IOBase::pread_exact). Errors with [`IoError::UnexpectedEof`] (naming the
    /// shortfall) when the sink could not take every byte — a bounded window
    /// ([`IOSlice`](super::IOSlice)) clamps at its edge, and even a growable source refuses an
    /// offset so large the write would overflow the address space. For an ordinary in-heap write
    /// this always succeeds (the storage grows to fit).
    fn pwrite_all(&mut self, offset: u64, data: &[u8]) -> Result<(), IoError> {
        let written = self.pwrite_byte_array(offset, data);
        if written == data.len() {
            Ok(())
        } else {
            Err(IoError::UnexpectedEof {
                offset: offset + written as u64,
                requested: data.len(),
                available: written,
            })
        }
    }

    /// Reads up to `len` bytes at `offset` into a fresh `Vec` (short near the end) — the owning
    /// read for callers that do not bring their own buffer. One allocation, **pre-sized to what
    /// is actually available** (never to the raw request), so a hostile or corrupt `len` cannot
    /// trigger a runaway allocation.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let data = Heap::from_slice(b"hello world");
    /// assert_eq!(data.pread_vec(6, 100), b"world"); // clamped to what remains
    /// assert_eq!(data.pread_vec(6, usize::MAX), b"world"); // no giant up-front allocation
    /// ```
    fn pread_vec(&self, offset: u64, len: usize) -> Vec<u8> {
        let available = self.byte_size().saturating_sub(offset).min(len as u64) as usize;
        let mut buf = vec![0u8; available];
        let read = self.pread_byte_array(offset, &mut buf);
        buf.truncate(read);
        buf
    }

    /// Reads up to `len` bytes at `offset` into `dst`, **reusing `dst`'s existing allocation** —
    /// the allocation-free bulk read for a transfer loop that reads chunk after chunk into one
    /// scratch buffer. `dst` is cleared, grown once to fit only if its capacity is too small
    /// (its spare capacity is reused otherwise), and filled; returns the number of bytes read
    /// (short near the end). Unlike [`pread_vec`](IOBase::pread_vec) — a fresh `Vec` every call —
    /// this keeps a caller's buffer hot across a whole transfer.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let src = Heap::from_slice(b"hello world");
    /// let mut scratch = Vec::new();
    /// assert_eq!(src.pread_into(0, 5, &mut scratch), 5);
    /// assert_eq!(&scratch, b"hello");
    /// let cap = scratch.capacity();
    /// assert_eq!(src.pread_into(6, 5, &mut scratch), 5); // reuses the allocation
    /// assert_eq!(&scratch, b"world");
    /// assert_eq!(scratch.capacity(), cap);
    /// ```
    fn pread_into(&self, offset: u64, len: usize, dst: &mut Vec<u8>) -> usize {
        // Size to what is actually available (never the raw request), so a hostile `len`
        // cannot force a runaway grow; reuses `dst`'s capacity when it already fits.
        let available = self.byte_size().saturating_sub(offset).min(len as u64) as usize;
        dst.clear();
        dst.resize(available, 0);
        let read = self.pread_byte_array(offset, dst);
        dst.truncate(read);
        read
    }

    /// Reads the single byte at `offset`, or errors with [`IoError::UnexpectedEof`] if it is past
    /// the end.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let data = Heap::from_slice(b"abc");
    /// assert_eq!(data.pread_byte(1).unwrap(), b'b');
    /// assert!(data.pread_byte(3).is_err());
    /// ```
    fn pread_byte(&self, offset: u64) -> Result<u8, IoError> {
        let mut buf = [0u8; 1];
        self.pread_exact(offset, &mut buf)?;
        Ok(buf[0])
    }

    /// Writes the single byte `value` at `offset`, growing the storage as needed.
    fn pwrite_byte(&mut self, offset: u64, value: u8) -> Result<(), IoError> {
        self.pwrite_all(offset, &[value])
    }

    /// Reads the bit at absolute **bit** `offset` (LSB-first: bit `offset % 8` of byte
    /// `offset / 8`), or errors with [`IoError::UnexpectedEof`] if its byte is past the end.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let data = Heap::from_slice(&[0b0000_0101]);
    /// assert!(data.pread_bit(0).unwrap());  // bit 0 set
    /// assert!(!data.pread_bit(1).unwrap()); // bit 1 clear
    /// assert!(data.pread_bit(2).unwrap());  // bit 2 set
    /// ```
    fn pread_bit(&self, offset: u64) -> Result<bool, IoError> {
        let byte = self.pread_byte(offset / 8)?;
        Ok((byte >> (offset % 8)) & 1 != 0)
    }

    /// Sets or clears the bit at absolute **bit** `offset` (LSB-first), read-modify-writing its
    /// byte and growing the storage (zero-filled) if the bit is past the end.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let mut data = Heap::new();
    /// data.pwrite_bit(10, true).unwrap();      // grows to 2 bytes, sets bit 2 of byte 1
    /// assert_eq!(data.as_slice(), &[0b0000_0000, 0b0000_0100]);
    /// assert!(data.pread_bit(10).unwrap());
    /// ```
    fn pwrite_bit(&mut self, offset: u64, value: bool) -> Result<(), IoError> {
        let byte_offset = offset / 8;
        let mask = 1u8 << (offset % 8);
        let mut buf = [0u8; 1];
        self.pread_byte_array(byte_offset, &mut buf); // 0 if past the end (about to grow)
        if value {
            buf[0] |= mask;
        } else {
            buf[0] &= !mask;
        }
        self.pwrite_all(byte_offset, &buf)
    }

    /// Reads a little-endian `i32` (4 bytes) at `offset`, or errors with
    /// [`IoError::UnexpectedEof`].
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let data = Heap::from_slice(&(-42i32).to_le_bytes());
    /// assert_eq!(data.pread_i32(0).unwrap(), -42);
    /// ```
    fn pread_i32(&self, offset: u64) -> Result<i32, IoError> {
        let mut buf = [0u8; 4];
        self.pread_exact(offset, &mut buf)?;
        Ok(i32::from_le_bytes(buf))
    }

    /// Writes `value` as a little-endian `i32` (4 bytes) at `offset`, growing as needed.
    fn pwrite_i32(&mut self, offset: u64, value: i32) -> Result<(), IoError> {
        self.pwrite_all(offset, &value.to_le_bytes())
    }

    /// Reads a little-endian `i64` (8 bytes) at `offset`, or errors with
    /// [`IoError::UnexpectedEof`].
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let data = Heap::from_slice(&(1234567890123i64).to_le_bytes());
    /// assert_eq!(data.pread_i64(0).unwrap(), 1234567890123);
    /// ```
    fn pread_i64(&self, offset: u64) -> Result<i64, IoError> {
        let mut buf = [0u8; 8];
        self.pread_exact(offset, &mut buf)?;
        Ok(i64::from_le_bytes(buf))
    }

    /// Writes `value` as a little-endian `i64` (8 bytes) at `offset`, growing as needed.
    fn pwrite_i64(&mut self, offset: u64, value: i64) -> Result<(), IoError> {
        self.pwrite_all(offset, &value.to_le_bytes())
    }

    /// Reads up to `len` **bytes** at `offset` and decodes them as UTF-8 text (clamped near the
    /// end, like [`pread_vec`](IOBase::pread_vec)), or errors with [`IoError::InvalidUtf8`]
    /// naming the offending byte — including a multi-byte character cut by the range. Built on
    /// the byte primitives, so every source inherits it.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let mut data = Heap::new();
    /// data.pwrite_utf8(0, "héllo");
    /// assert_eq!(data.pread_utf8(0, 6).unwrap(), "héllo"); // é is 2 bytes
    /// assert!(data.pread_utf8(0, 2).is_err());             // cuts é in half
    /// ```
    fn pread_utf8(&self, offset: u64, len: usize) -> Result<String, IoError> {
        String::from_utf8(self.pread_vec(offset, len)).map_err(|error| IoError::InvalidUtf8 {
            position: error.utf8_error().valid_up_to(),
        })
    }

    /// Writes `text`'s UTF-8 bytes at `offset` (growing as needed); returns the number of
    /// **bytes** written. The exact writing counterpart of [`pread_utf8`](IOBase::pread_utf8).
    fn pwrite_utf8(&mut self, offset: u64, text: &str) -> usize {
        self.pwrite_byte_array(offset, text.as_bytes())
    }

    /// **Bulk typed read.** Fills *all* of `dst` with little-endian `i32`s starting at `offset`,
    /// or errors with [`IoError::UnexpectedEof`]. Stages through a fixed stack chunk — zero heap
    /// allocation — and converts each chunk in a dense, branch-free loop the compiler
    /// vectorizes.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let mut data = Heap::new();
    /// data.pwrite_i32_array(0, &[1, -2, 3]).unwrap();
    /// let mut values = [0i32; 3];
    /// data.pread_i32_array(0, &mut values).unwrap();
    /// assert_eq!(values, [1, -2, 3]);
    /// ```
    fn pread_i32_array(&self, offset: u64, dst: &mut [i32]) -> Result<(), IoError> {
        stage_pread_i32_array(self, offset, dst)
    }

    /// **Bulk typed write.** Writes all of `src` as little-endian `i32`s at `offset` (growing
    /// as needed). Stages through a fixed stack chunk — zero heap allocation, vectorizable.
    fn pwrite_i32_array(&mut self, offset: u64, src: &[i32]) -> Result<(), IoError> {
        stage_pwrite_i32_array(self, offset, src)
    }

    /// **Bulk typed read** of little-endian `i64`s — the wide counterpart of
    /// [`pread_i32_array`](IOBase::pread_i32_array).
    fn pread_i64_array(&self, offset: u64, dst: &mut [i64]) -> Result<(), IoError> {
        stage_pread_i64_array(self, offset, dst)
    }

    /// **Bulk typed write** of little-endian `i64`s — the wide counterpart of
    /// [`pwrite_i32_array`](IOBase::pwrite_i32_array).
    fn pwrite_i64_array(&mut self, offset: u64, src: &[i64]) -> Result<(), IoError> {
        stage_pwrite_i64_array(self, offset, src)
    }

    /// **Repeated-value fill.** Writes `count` copies of the byte `value` starting at `offset`
    /// (growing as needed) — without ever materializing the full array: a fixed stack chunk is
    /// filled once and written repeatedly. The byte-level `memset` of the family.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let mut data = Heap::new();
    /// data.pwrite_byte_repeat(0, 0xAB, 5).unwrap();
    /// assert_eq!(data.as_slice(), &[0xAB; 5]);
    /// ```
    fn pwrite_byte_repeat(&mut self, offset: u64, value: u8, count: usize) -> Result<(), IoError> {
        stage_pwrite_byte_repeat(self, offset, value, count)
    }

    /// **Repeated-value fill** of `count` little-endian `i32` copies of `value` at `offset` —
    /// no full array is built (one stack chunk, filled once, written repeatedly).
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let mut data = Heap::new();
    /// data.pwrite_i32_repeat(0, -1, 3).unwrap();
    /// let mut values = [0i32; 3];
    /// data.pread_i32_array(0, &mut values).unwrap();
    /// assert_eq!(values, [-1, -1, -1]);
    /// ```
    fn pwrite_i32_repeat(&mut self, offset: u64, value: i32, count: usize) -> Result<(), IoError> {
        stage_pwrite_i32_repeat(self, offset, value, count)
    }

    /// **Repeated-value fill** of `count` little-endian `i64` copies of `value` at `offset` —
    /// the wide counterpart of [`pwrite_i32_repeat`](IOBase::pwrite_i32_repeat).
    fn pwrite_i64_repeat(&mut self, offset: u64, value: i64, count: usize) -> Result<(), IoError> {
        stage_pwrite_i64_repeat(self, offset, value, count)
    }

    /// Wraps this source in an [`IOCursor`] positioned at the start — the standard way to add a
    /// moving read/write position to any source. Consumes the source (zero-copy); wrap a clone to
    /// keep the original.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let mut cur = Heap::from_slice(b"hi").cursor();
    /// assert_eq!(cur.read_byte().unwrap(), b'h');
    /// ```
    fn cursor(self) -> IOCursor<Self>
    where
        Self: Sized,
    {
        IOCursor::new(self)
    }

    /// Wraps this source in an [`IOSlice`] — the bounded window `[offset, offset + len)` addressed
    /// from its own `0`. Errors with [`IoError::SliceOutOfBounds`] if it runs past the end.
    /// Consumes the source (zero-copy); wrap a clone to keep the original.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let win = Heap::from_slice(b"hello world").window(6, 5).unwrap();
    /// assert_eq!(win.pread_vec(0, 5), b"world");
    /// ```
    fn window(self, offset: u64, len: u64) -> Result<IOSlice<Self>, IoError>
    where
        Self: Sized,
    {
        IOSlice::new(self, offset, len)
    }
}

// -------------------------------------------------------------------------------------
// Stack-staged bulk kernels, as free functions — the single source of truth for the trait
// defaults. A source with a faster **contiguous** backing (`Heap` over its `Vec`, `Mmap`
// over its mapping) overrides the trait methods with a direct conversion; a source that
// **composes** another (`LocalIO`'s non-mapped branch, over its ad-hoc / memory-tree byte
// methods) reuses these staged kernels verbatim rather than duplicating them. Each stages
// through one fixed stack chunk (zero heap allocation) and converts in a dense, branch-free
// loop the compiler vectorizes.
// -------------------------------------------------------------------------------------

pub(crate) fn stage_pread_i32_array<S: IOBase>(
    src: &S,
    offset: u64,
    dst: &mut [i32],
) -> Result<(), IoError> {
    let mut bytes = [0u8; BULK_CHUNK * 4];
    let mut position = offset;
    for chunk in dst.chunks_mut(BULK_CHUNK) {
        let staged = &mut bytes[..chunk.len() * 4];
        src.pread_exact(position, staged)?;
        for (value, raw) in chunk.iter_mut().zip(staged.chunks_exact(4)) {
            *value = i32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
        }
        position += staged.len() as u64;
    }
    Ok(())
}

pub(crate) fn stage_pwrite_i32_array<S: IOBase>(
    dst: &mut S,
    offset: u64,
    src: &[i32],
) -> Result<(), IoError> {
    let mut bytes = [0u8; BULK_CHUNK * 4];
    let mut position = offset;
    for chunk in src.chunks(BULK_CHUNK) {
        let staged = &mut bytes[..chunk.len() * 4];
        for (raw, value) in staged.chunks_exact_mut(4).zip(chunk) {
            raw.copy_from_slice(&value.to_le_bytes());
        }
        dst.pwrite_all(position, staged)?;
        position += staged.len() as u64;
    }
    Ok(())
}

pub(crate) fn stage_pread_i64_array<S: IOBase>(
    src: &S,
    offset: u64,
    dst: &mut [i64],
) -> Result<(), IoError> {
    let mut bytes = [0u8; BULK_CHUNK * 8];
    let mut position = offset;
    for chunk in dst.chunks_mut(BULK_CHUNK) {
        let staged = &mut bytes[..chunk.len() * 8];
        src.pread_exact(position, staged)?;
        for (value, raw) in chunk.iter_mut().zip(staged.chunks_exact(8)) {
            *value = i64::from_le_bytes(raw.try_into().expect("chunks_exact yields 8"));
        }
        position += staged.len() as u64;
    }
    Ok(())
}

pub(crate) fn stage_pwrite_i64_array<S: IOBase>(
    dst: &mut S,
    offset: u64,
    src: &[i64],
) -> Result<(), IoError> {
    let mut bytes = [0u8; BULK_CHUNK * 8];
    let mut position = offset;
    for chunk in src.chunks(BULK_CHUNK) {
        let staged = &mut bytes[..chunk.len() * 8];
        for (raw, value) in staged.chunks_exact_mut(8).zip(chunk) {
            raw.copy_from_slice(&value.to_le_bytes());
        }
        dst.pwrite_all(position, staged)?;
        position += staged.len() as u64;
    }
    Ok(())
}

pub(crate) fn stage_pwrite_byte_repeat<S: IOBase>(
    dst: &mut S,
    offset: u64,
    value: u8,
    count: usize,
) -> Result<(), IoError> {
    let chunk = [value; BULK_CHUNK * 4];
    let mut position = offset;
    let mut remaining = count;
    while remaining > 0 {
        let take = remaining.min(chunk.len());
        dst.pwrite_all(position, &chunk[..take])?;
        position += take as u64;
        remaining -= take;
    }
    Ok(())
}

pub(crate) fn stage_pwrite_i32_repeat<S: IOBase>(
    dst: &mut S,
    offset: u64,
    value: i32,
    count: usize,
) -> Result<(), IoError> {
    let mut chunk = [0u8; BULK_CHUNK * 4];
    for raw in chunk.chunks_exact_mut(4) {
        raw.copy_from_slice(&value.to_le_bytes());
    }
    let mut position = offset;
    let mut remaining = count;
    while remaining > 0 {
        let take = remaining.min(BULK_CHUNK);
        dst.pwrite_all(position, &chunk[..take * 4])?;
        position += (take * 4) as u64;
        remaining -= take;
    }
    Ok(())
}

pub(crate) fn stage_pwrite_i64_repeat<S: IOBase>(
    dst: &mut S,
    offset: u64,
    value: i64,
    count: usize,
) -> Result<(), IoError> {
    let mut chunk = [0u8; BULK_CHUNK * 8];
    for raw in chunk.chunks_exact_mut(8) {
        raw.copy_from_slice(&value.to_le_bytes());
    }
    let mut position = offset;
    let mut remaining = count;
    while remaining > 0 {
        let take = remaining.min(BULK_CHUNK);
        dst.pwrite_all(position, &chunk[..take * 8])?;
        position += (take * 8) as u64;
        remaining -= take;
    }
    Ok(())
}
