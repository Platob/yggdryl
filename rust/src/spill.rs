//! Disk spill: a column too big to hold writes its buffers once to a private
//! file, maps it read-only, and reads back as the same Arrow buffers over the
//! mapping.
//!
//! The spill unit is one leaf's buffers, written as they are. The unit's
//! Arrow data tree is walked once, depth first - the validity bitmap, then
//! each data buffer, then the children - and every buffer is written whole
//! at a 64-byte-aligned offset through one buffered writer of
//! [`DEFAULT_STREAM_BATCH_SIZE`]. Nothing describes the file on disk: the
//! process keeps the skeleton - each node's datatype, length and offset,
//! where each buffer lies and the validity's bit offset, the children - and
//! rebuilds the tree over slices of one read-only mapping before the column
//! is handed back. Every read, slice, cast and Arrow export of the rebuilt
//! column then reaches the mapping and copies nothing; the first write to a
//! mapped buffer copies it once, because a mapping is never written.
//!
//! The file is created new under [`SpillOptions::folder`] - the platform
//! temporary folder unless stated - as `yggdryl-spill-<pid>-<seq>`, unlinked
//! right after it is opened on Unix and opened to delete on close on Windows,
//! so a crash leaves nothing behind and nothing outside the process can
//! truncate it while it is mapped: the SIGBUS hazard a shared mapping of a
//! named file carries ([`crate::local::LocalFile`]) does not arise here.
//!
//! [`SpillOptions`] is the bound and the folder. [`SpillOptions::from_env`]
//! is the process default, read once from `YGGDRYL_SPILL_BYTE_SIZE` and
//! `YGGDRYL_SPILL_FOLDER`, which every door the crate lays a column out at
//! settles under ([`Serie::settled`](crate::Serie)); a per-call
//! [`SpillOptions`] is what [`Serie::spill`](crate::Serie::spill) takes.
//!
//! # Unsafe
//!
//! Three uses, each named where it stands:
//!
//! * `memmap2::Mmap::map`, for the reason [`crate::local::LocalFile`] gives:
//!   a mapping aliases file bytes. The file here is private to the process
//!   and already unlinked, so no other process can shorten it;
//! * `Buffer::from_custom_allocation`, which trusts a pointer and a length:
//!   both come from the mapping that owns the bytes, which the buffer keeps
//!   alive through its `Allocation` and which [`Mapping::contains`] checks;
//! * `ArrayDataBuilder::build_unchecked`, which trusts the buffers to be
//!   valid Arrow data: they are the very bytes of an array this process
//!   held, written and mapped back byte for byte, so a checked rebuild would
//!   only re-scan what the landing already proved.

#![allow(unsafe_code)]

use std::ffi::OsString;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write as _};
use std::path::PathBuf;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use arrow_array::{Array, ArrayRef, make_array};
use arrow_buffer::alloc::Allocation;
use arrow_buffer::{BooleanBuffer, Buffer, NullBuffer};
use arrow_data::{ArrayData, ArrayDataBuilder};
use arrow_schema::DataType as ArrowDataType;
use memmap2::Mmap;
use smol_str::format_smolstr;

use crate::local::LocalFolder;
use crate::{DEFAULT_STREAM_BATCH_SIZE, Error, Result};

/// The default bound one column stays resident under: 64 MiB.
pub const DEFAULT_SPILL_BYTE_SIZE: u64 = 64 * 1024 * 1024;

/// The environment variable naming the process-wide bound: a byte count, or
/// `never` in any case.
const BYTE_SIZE_VARIABLE: &str = "YGGDRYL_SPILL_BYTE_SIZE";

/// The environment variable naming the folder spill files are created in.
const FOLDER_VARIABLE: &str = "YGGDRYL_SPILL_FOLDER";

/// Every buffer starts on this boundary in the file, so it does in the
/// mapping too: Arrow's widest native is sixteen bytes and its kernels read
/// sixty-four-byte lanes.
const ALIGNMENT: usize = 64;

/// The process default, once resolved. Unset until a parse succeeds or a
/// value is installed, so a refused environment is retried by the next call.
static GLOBAL: OnceLock<SpillOptions> = OnceLock::new();

/// The number of the next spill file this process creates.
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// How many names are tried before a collision is reported.
const NAME_ATTEMPTS: u64 = 16;

/// The bound a column stays resident under, and the folder it spills to.
///
/// `byte_size` is the resident bytes a column may hold before it spills;
/// [`Self::NEVER`] spills nothing and `0` everything. `folder` is where the
/// files are created, the platform temporary folder when `None`.
///
/// ```
/// use yggdryl::{DEFAULT_SPILL_BYTE_SIZE, SpillOptions};
///
/// let options = SpillOptions::new();
/// assert_eq!(options.byte_size(), DEFAULT_SPILL_BYTE_SIZE);
/// assert!(options.folder().is_none());
/// assert!(SpillOptions::new().with_byte_size(SpillOptions::NEVER).is_never());
/// ```
#[derive(Clone, Debug)]
pub struct SpillOptions {
    byte_size: u64,
    folder: Option<LocalFolder>,
}

impl SpillOptions {
    /// The bound under which nothing spills.
    pub const NEVER: u64 = u64::MAX;

    /// The default bound ([`DEFAULT_SPILL_BYTE_SIZE`]) over the platform
    /// temporary folder.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            byte_size: DEFAULT_SPILL_BYTE_SIZE,
            folder: None,
        }
    }

    /// The same folder under another bound.
    #[must_use]
    pub const fn with_byte_size(mut self, byte_size: u64) -> Self {
        self.byte_size = byte_size;
        self
    }

    /// The same bound over another folder.
    #[must_use]
    pub fn with_folder(mut self, folder: LocalFolder) -> Self {
        self.folder = Some(folder);
        self
    }

    /// The resident bytes a column may hold before it spills.
    #[must_use]
    pub const fn byte_size(&self) -> u64 {
        self.byte_size
    }

    /// The folder spill files are created in, `None` for the platform
    /// temporary folder.
    #[must_use]
    pub const fn folder(&self) -> Option<&LocalFolder> {
        self.folder.as_ref()
    }

    /// Whether the bound is [`Self::NEVER`].
    #[must_use]
    pub const fn is_never(&self) -> bool {
        self.byte_size == Self::NEVER
    }

    /// The options the process environment states, read on the first call.
    ///
    /// `YGGDRYL_SPILL_BYTE_SIZE` is the bound - a byte count, or `never` in
    /// any case - and `YGGDRYL_SPILL_FOLDER` the folder, a path; either unset
    /// or empty is [`Self::new`]'s answer for it. Nothing reads at module
    /// init: the first call resolves the default on the calling thread,
    /// reading the environment exactly once, and every later call answers
    /// the same value. [`Self::install_env`] installs one before anything
    /// resolves it.
    ///
    /// # Errors
    ///
    /// Returns an error naming the variable when the bound is neither a
    /// byte count nor `never`, or is not UTF-8, and the folder's own refusal
    /// for a path that is no local folder. The default stays unresolved, so
    /// the next call reads the environment again.
    pub fn from_env() -> Result<&'static Self> {
        if let Some(options) = GLOBAL.get() {
            return Ok(options);
        }
        let parsed = Self::parse_env(
            std::env::var_os(BYTE_SIZE_VARIABLE),
            std::env::var_os(FOLDER_VARIABLE),
        )?;
        Ok(GLOBAL.get_or_init(|| parsed))
    }

    /// Installs the options every later [`Self::from_env`] answers, before
    /// anything resolves them.
    ///
    /// # Errors
    ///
    /// Returns a typed conflict when the default has already been resolved
    /// or installed, so the value every caller saw cannot change underneath
    /// them.
    pub fn install_env(options: Self) -> Result<()> {
        GLOBAL.set(options).map_err(|_| {
            Error::conflict(
                "spill options",
                "spill options",
                "the process default, already resolved",
            )
        })
    }

    /// Resolve the default from its two inputs: what `YGGDRYL_SPILL_BYTE_SIZE`
    /// and `YGGDRYL_SPILL_FOLDER` held.
    ///
    /// Pure in both, so the rule is tested with explicit inputs and never
    /// through the process-wide environment.
    pub(crate) fn parse_env(byte_size: Option<OsString>, folder: Option<OsString>) -> Result<Self> {
        let mut options = Self::new();
        if let Some(value) = byte_size {
            let text = value.into_string().map_err(|value| Error::Codec {
                format: "text",
                position: 0,
                reason: format_smolstr!("expected UTF-8 in {BYTE_SIZE_VARIABLE}, got {value:?}"),
            })?;
            let trimmed = text.trim();
            if trimmed.eq_ignore_ascii_case("never") {
                options.byte_size = Self::NEVER;
            } else if !trimmed.is_empty() {
                options.byte_size = trimmed.parse::<u64>().map_err(|error| Error::Codec {
                    format: "text",
                    position: 0,
                    reason: format_smolstr!(
                        "expected a byte count or `never` in {BYTE_SIZE_VARIABLE}, got {trimmed:?}: {error}"
                    ),
                })?;
            }
        }
        if let Some(value) = folder
            && !value.is_empty()
        {
            options.folder = Some(LocalFolder::new(PathBuf::from(value))?);
        }
        Ok(options)
    }
}

impl Default for SpillOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl PartialEq for SpillOptions {
    /// Two folders are one when they name one location.
    fn eq(&self, other: &Self) -> bool {
        self.byte_size == other.byte_size
            && self.folder.as_ref().map(LocalFolder::url)
                == other.folder.as_ref().map(LocalFolder::url)
    }
}

impl Eq for SpillOptions {}

/// Where a leaf's buffers live: the one fact a leaf keeps about them.
///
/// A write that replaces a leaf's buffers sets [`Self::Heap`]; a slice and a
/// clone carry what they were cut from; a spill sets [`Self::Mapped`]. The
/// mapping itself is owned by the buffers over it, each through the
/// [`Allocation`] it was built with, so nothing here has to hold it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum Backing {
    /// The buffers are the allocator's.
    #[default]
    Heap,
    /// The buffers are slices of one read-only mapping of a spill file.
    Mapped,
}

impl Backing {
    /// Whether the buffers are a spill file's.
    pub(crate) const fn is_mapped(self) -> bool {
        matches!(self, Self::Mapped)
    }
}

/// One spill file, mapped read-only: the [`Allocation`] every buffer of a
/// spilled unit shares, so the mapping outlives the last buffer over it.
pub(crate) struct Mapping {
    map: Mmap,
    /// Held so the descriptor outlives the mapping; already unlinked.
    _file: File,
}

#[cfg(feature = "internals")]
impl Mapping {
    /// The bytes the mapping covers.
    pub(crate) fn len(&self) -> usize {
        self.map.len()
    }

    /// Whether `len` bytes from `ptr` lie inside the mapping.
    pub(crate) fn contains(&self, ptr: *const u8, len: usize) -> bool {
        let start = self.map.as_ptr() as usize;
        let at = ptr as usize;
        at >= start
            && at
                .checked_add(len)
                .is_some_and(|end| end <= start + self.map.len())
    }
}

impl fmt::Debug for Mapping {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Mapping")
            .field("len", &self.map.len())
            .finish()
    }
}

/// Where one buffer's bytes lie in the file.
struct Slot {
    place: usize,
    len: usize,
}

/// The validity bitmap's place, with the bit offset and length the array
/// reads it at.
struct Validity {
    slot: Slot,
    bit_offset: usize,
    bits: usize,
}

/// One node of the skeleton the process keeps while the buffers are on disk.
struct Node {
    dtype: ArrowDataType,
    len: usize,
    offset: usize,
    nulls: Option<Validity>,
    buffers: Vec<Slot>,
    children: Vec<Node>,
}

/// The one pass that writes a tree's buffers and records where each landed.
struct Writer<'a> {
    sink: BufWriter<&'a File>,
    position: usize,
}

impl Writer<'_> {
    /// Write `bytes` at the next aligned place and answer where they lie.
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<Slot> {
        let padding = (ALIGNMENT - self.position % ALIGNMENT) % ALIGNMENT;
        if padding != 0 {
            self.sink.write_all(&[0_u8; ALIGNMENT][..padding])?;
            self.position += padding;
        }
        let slot = Slot {
            place: self.position,
            len: bytes.len(),
        };
        self.sink.write_all(bytes)?;
        self.position += bytes.len();
        Ok(slot)
    }

    /// Write every buffer of `data`'s tree, depth first, as the skeleton.
    fn walk(&mut self, data: &ArrayData) -> std::io::Result<Node> {
        let nulls = match data.nulls() {
            Some(nulls) => {
                let bits = nulls.inner();
                Some(Validity {
                    slot: self.write(bits.inner().as_slice())?,
                    bit_offset: bits.offset(),
                    bits: bits.len(),
                })
            }
            None => None,
        };
        let mut buffers = Vec::with_capacity(data.buffers().len());
        for buffer in data.buffers() {
            buffers.push(self.write(buffer.as_slice())?);
        }
        let mut children = Vec::with_capacity(data.child_data().len());
        for child in data.child_data() {
            children.push(self.walk(child)?);
        }
        Ok(Node {
            dtype: data.data_type().clone(),
            len: data.len(),
            offset: data.offset(),
            nulls,
            buffers,
            children,
        })
    }
}

/// The bytes a tree's buffers hold, summed: zero for a tree with nothing to
/// write.
fn tree_bytes(data: &ArrayData) -> usize {
    data.nulls().map_or(0, |nulls| nulls.inner().inner().len())
        + data.buffers().iter().map(Buffer::len).sum::<usize>()
        + data.child_data().iter().map(tree_bytes).sum::<usize>()
}

/// Rebuild `node`'s tree over slices of `mapping`.
fn rebuild(node: &Node, mapping: &Arc<Mapping>) -> ArrayData {
    let buffer = |slot: &Slot| -> Buffer {
        // SAFETY: `slot` was recorded by the writer that laid the file out,
        // so `place + len` lies inside the mapping, whose bytes stay valid
        // and immutable for as long as the mapping lives - and the buffer
        // keeps it alive through the `Allocation` it is handed.
        unsafe {
            let base = mapping.map.as_ptr().cast_mut();
            let ptr = NonNull::new_unchecked(base.add(slot.place));
            Buffer::from_custom_allocation(
                ptr,
                slot.len,
                Arc::clone(mapping) as Arc<dyn Allocation>,
            )
        }
    };
    let mut builder = ArrayDataBuilder::new(node.dtype.clone())
        .len(node.len)
        .offset(node.offset)
        .add_buffers(node.buffers.iter().map(buffer));
    if let Some(validity) = &node.nulls {
        builder = builder.nulls(Some(NullBuffer::new(BooleanBuffer::new(
            buffer(&validity.slot),
            validity.bit_offset,
            validity.bits,
        ))));
    }
    builder = builder.child_data(
        node.children
            .iter()
            .map(|child| rebuild(child, mapping))
            .collect(),
    );
    // SAFETY: every buffer is the very bytes of an array this process held,
    // written and mapped back byte for byte under the same datatype, length,
    // offset and children, so the data the landing already proved valid is
    // valid here; a checked build would only scan it again.
    unsafe { builder.build_unchecked() }
}

/// Name a spill failure by the folder it happened under.
fn under(folder: &LocalFolder, error: std::io::Error) -> Error {
    Error::Io(std::io::Error::new(
        error.kind(),
        format!("spill under {}: {error}", folder.url()),
    ))
}

/// Create the next spill file under `folder`: new, private to this process,
/// and already gone from the folder's listing.
fn create_file(folder: &LocalFolder) -> Result<File> {
    let root = folder.path()?;
    let pid = std::process::id();
    for _ in 0..NAME_ATTEMPTS {
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = root.join(format!("yggdryl-spill-{pid}-{sequence}"));
        match open_private(&path) {
            Ok(file) => return Ok(file),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(under(folder, error)),
        }
    }
    Err(under(
        folder,
        std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("{NAME_ATTEMPTS} spill file names were taken"),
        ),
    ))
}

/// Open `path` new for reading and writing, and make it vanish on close:
/// unlinked at once on Unix, flagged to delete on close on Windows.
#[cfg(unix)]
fn open_private(path: &std::path::Path) -> std::io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)?;
    std::fs::remove_file(path)?;
    Ok(file)
}

/// Open `path` new for reading and writing, and make it vanish on close:
/// unlinked at once on Unix, flagged to delete on close on Windows.
#[cfg(windows)]
fn open_private(path: &std::path::Path) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt as _;
    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_SHARE_WRITE: u32 = 0x0000_0002;
    const FILE_SHARE_DELETE: u32 = 0x0000_0004;
    const FILE_FLAG_DELETE_ON_CLOSE: u32 = 0x0400_0000;
    OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_DELETE_ON_CLOSE)
        .open(path)
}

/// Write `array`'s buffers to a new spill file under `folder` - the platform
/// temporary folder when `None` - and answer the same array rebuilt over the
/// file's read-only mapping, with the mapping every buffer of it shares.
///
/// `None` for a tree with no byte to write, which spills nothing.
///
/// # Errors
///
/// Returns an error naming the folder when the file cannot be created,
/// written or mapped; the array is untouched.
pub(crate) fn spill_array(
    array: &ArrayRef,
    folder: Option<&LocalFolder>,
) -> Result<Option<(ArrayRef, Arc<Mapping>)>> {
    let data = array.to_data();
    if tree_bytes(&data) == 0 {
        return Ok(None);
    }
    let folder = match folder {
        Some(folder) => folder.clone(),
        None => LocalFolder::temporary()?,
    };
    let file = create_file(&folder)?;
    let node = {
        let mut writer = Writer {
            sink: BufWriter::with_capacity(DEFAULT_STREAM_BATCH_SIZE, &file),
            position: 0,
        };
        let node = writer.walk(&data).map_err(|error| under(&folder, error))?;
        writer.sink.flush().map_err(|error| under(&folder, error))?;
        node
    };
    // SAFETY: the file is private to this process and already unlinked, so
    // nothing else can shorten it while the mapping is live; the mapping is
    // read-only and the file handle is kept beside it for as long as any
    // buffer reaches it.
    let map = unsafe { Mmap::map(&file) }.map_err(|error| under(&folder, error))?;
    let mapping = Arc::new(Mapping { map, _file: file });
    let rebuilt = make_array(rebuild(&node, &mapping));
    Ok(Some((rebuilt, mapping)))
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/root/spill.rs` pins and a caller cannot reach.
    //!
    //! [`SpillOptions::from_env`](crate::SpillOptions::from_env) resolves
    //! once per process and reads the environment, so the rule it reads by
    //! is pinned through the pure step underneath it; the walk is pinned
    //! over the mapping it answers.

    use std::ffi::OsString;
    use std::sync::Arc;

    use arrow_array::ArrayRef;

    use crate::local::LocalFolder;
    use crate::{Result, SpillOptions};

    /// One spill file mapped read-only, as the buffers over it see it.
    pub struct Mapping(pub(crate) Arc<super::Mapping>);

    impl Mapping {
        /// The bytes the mapping covers.
        pub fn len(&self) -> usize {
            self.0.len()
        }

        /// Whether the mapping covers no byte: never, for a mapping that
        /// was created.
        pub fn is_empty(&self) -> bool {
            self.0.len() == 0
        }

        /// Whether `len` bytes from `ptr` lie inside the mapping.
        pub fn contains(&self, ptr: *const u8, len: usize) -> bool {
            self.0.contains(ptr, len)
        }
    }

    /// Write `array`'s buffers to a spill file and read them back mapped.
    ///
    /// # Errors
    ///
    /// `spill_array` at the crate root carries the rule.
    pub fn spill_array(
        array: &ArrayRef,
        folder: Option<&LocalFolder>,
    ) -> Result<Option<(ArrayRef, Mapping)>> {
        Ok(
            super::spill_array(array, folder)?
                .map(|(rebuilt, mapping)| (rebuilt, Mapping(mapping))),
        )
    }

    /// Resolve the options from what the two variables held.
    ///
    /// # Errors
    ///
    /// `SpillOptions::parse_env` carries the rule.
    pub fn parse_env(
        byte_size: Option<OsString>,
        folder: Option<OsString>,
    ) -> Result<SpillOptions> {
        SpillOptions::parse_env(byte_size, folder)
    }
}
