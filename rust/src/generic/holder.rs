//! One concrete value for any [`IOBase`] implementation.

use crate::buffered::{Buffered, BufferedOptions};
use crate::{MediaType, Result, Url};

use crate::io::{Buffer, IOBase};
use crate::local::Folder;

use crate::local::File;

/// A concrete, sized value holding any core [`IOBase`] implementation.
///
/// Hierarchy accessors such as [`IOBase::parent`], [`IOBase::child_by`], and
/// [`IOBase::ls`] have to return *some* handle without knowing which kind it
/// will be, and `Box<dyn IOBase>` would erase the concrete type a caller needs
/// to match on. `Holder` is that return type: an enum over the implementations
/// the core ships, which itself implements [`IOBase`] by delegation.
///
/// A local directory therefore yields [`Holder::Folder`] for its
/// subdirectories and [`Holder::File`] for its files, and a caller can walk the
/// tree through one type.
///
/// ```
/// use yggdryl::generic::Holder;
/// use yggdryl::io::IOBase;
///
/// # fn main() -> yggdryl::Result<()> {
/// let root = Holder::folder(std::env::temp_dir())?;
/// assert!(root.is_container());
///
/// // A leaf is a mapped file, and it need not exist yet.
/// let leaf = root.child_by("yggdryl-holder-doc.bin")?;
/// assert!(!leaf.is_container());
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
#[non_exhaustive]
pub enum Holder {
    /// An in-memory byte array.
    Buffer(Buffer),
    /// A local directory.
    Folder(Folder),
    /// A local location that resolves to whatever it turns out to be.
    Path(crate::local::Path),
    /// A memory-mapped local file.
    File(File),
    /// A directory on a foreign Arrow filesystem.
    ArrowFolder(crate::arrowfs::Folder),
    /// A foreign-filesystem location that resolves to whatever it turns out
    /// to be.
    ArrowPath(crate::arrowfs::Path),
    /// A staged whole-value file on a foreign Arrow filesystem.
    ArrowFile(crate::arrowfs::File),
    /// Any of the others, read through a page cache.
    ///
    /// The box is what keeps the enum a fixed size: this variant holds a
    /// handle of the very type it belongs to.
    Buffered(Box<Buffered<Self>>),
    /// Any of the others, read and written as text records.
    ///
    /// A [`Text`](crate::text::Text) handler mirrors its handle's bytes and
    /// answers the record surface through the line projection, so a holder
    /// wrapped this way reads and writes Arrow batches as lines by default.
    /// Boxed for the same reason [`Self::Buffered`] is.
    Text(Box<crate::text::Text<Self>>),
}

impl Holder {
    /// Hold an in-memory buffer.
    pub const fn buffer(buffer: Buffer) -> Self {
        Self::Buffer(buffer)
    }

    /// Hold a local directory, without touching it.
    ///
    /// # Errors
    ///
    /// Returns an error only when the path cannot be expressed as a canonical
    /// `file:` URL.
    pub fn folder(path: impl AsRef<std::path::Path>) -> Result<Self> {
        Ok(Self::Folder(Folder::new(path)?))
    }

    /// Hold a memory-mapped local file, without touching it.
    ///
    /// # Errors
    ///
    /// Returns an error only when the path cannot be expressed as a canonical
    /// `file:` URL.
    pub fn file(path: impl AsRef<std::path::Path>) -> Result<Self> {
        Ok(Self::File(File::new(path)?))
    }

    /// Hold the local resource a path names.
    ///
    /// An existing directory becomes [`Self::Folder`]; anything else,
    /// including a path that does not exist yet, becomes a mapped file. A
    /// caller that knows which it wants should say so explicitly.
    ///
    /// # Errors
    ///
    /// Returns an error only when the path cannot be expressed as a canonical
    /// `file:` URL.
    pub fn local(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let path = path.as_ref();
        if path.is_dir() {
            Self::folder(path)
        } else {
            Self::file(path)
        }
    }

    /// Hold this resource behind a page cache.
    ///
    /// A holder that is already buffered is re-wrapped with the new options
    /// rather than nested, so there is never a second cache layer. This is the
    /// inherent spelling of [`IOBase::buffered`], and it wins method
    /// resolution over it.
    #[must_use]
    pub fn buffered(self, options: BufferedOptions) -> Self {
        let held = match self {
            Self::Buffered(buffered) => buffered.into_handle(),
            other => other,
        };
        Self::Buffered(Box::new(Buffered::new(held, options)))
    }

    /// Hold this resource behind the text-line surface.
    ///
    /// Idempotent exactly as [`Text::into_text`](crate::text::Text::into_text)
    /// is: a holder that is already text keeps its handler - and the options
    /// it carries - rather than nesting a second one. This is the inherent
    /// spelling of [`IOBase::into_text`], and it wins method resolution over
    /// it, so a `Holder` stays a `Holder`.
    #[must_use]
    pub fn into_text(self) -> Self {
        match self {
            Self::Text(text) => Self::Text(text),
            other => Self::Text(Box::new(crate::text::Text::new(other))),
        }
    }

    /// Hold this resource behind the text-line surface, under `options`.
    ///
    /// An already-text holder keeps its handle and replaces the extractor.
    #[must_use]
    pub fn into_text_with(self, options: crate::text::TextLineOptions) -> Self {
        match self {
            Self::Text(text) => Self::Text(Box::new(text.with_options_of(options))),
            other => Self::Text(Box::new(crate::text::Text::with_options(other, options))),
        }
    }

    /// Read this resource's records, consuming the holder.
    ///
    /// The inherent spelling of [`IOBase::into_read_lines`], and it wins
    /// method resolution over it - which matters: the trait default wraps a
    /// *fresh* text handler, while a holder that is already text must read
    /// under the extractor it carries rather than silently under defaults.
    ///
    /// # Errors
    ///
    /// Construction itself cannot fail; each yielded item carries the read or
    /// decode failure of its record.
    pub fn into_read_lines(
        self,
    ) -> Result<crate::text::TextLines<Box<dyn std::io::Read + Send + 'static>>> {
        let text = match self {
            Self::Text(text) => *text,
            other => crate::text::Text::new(other),
        };
        text.into_read_lines()
    }

    /// Borrow the held implementation as a trait object.
    pub fn as_io(&self) -> &dyn IOBase {
        match self {
            Self::Buffer(inner) => inner,
            Self::Folder(inner) => inner,
            Self::Path(inner) => inner,
            Self::File(inner) => inner,
            Self::ArrowFolder(inner) => inner,
            Self::ArrowPath(inner) => inner,
            Self::ArrowFile(inner) => inner,
            Self::Buffered(inner) => inner.as_ref(),
            Self::Text(inner) => inner.as_ref(),
        }
    }

    /// Borrow the held implementation mutably as a trait object.
    pub fn as_io_mut(&mut self) -> &mut dyn IOBase {
        match self {
            Self::Buffer(inner) => inner,
            Self::Folder(inner) => inner,
            Self::Path(inner) => inner,
            Self::File(inner) => inner,
            Self::ArrowFolder(inner) => inner,
            Self::ArrowPath(inner) => inner,
            Self::ArrowFile(inner) => inner,
            Self::Buffered(inner) => inner.as_mut(),
            Self::Text(inner) => inner.as_mut(),
        }
    }
}

impl IOBase for Holder {
    fn pread(&self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        self.as_io().pread(offset, buffer)
    }

    fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> Result<usize> {
        self.as_io_mut().pwrite(offset, bytes)
    }

    fn size(&self) -> u64 {
        self.as_io().size()
    }

    fn capacity(&self) -> u64 {
        self.as_io().capacity()
    }

    fn reserve(&mut self, capacity: u64) -> Result<()> {
        self.as_io_mut().reserve(capacity)
    }

    fn truncate(&mut self, size: u64) -> Result<()> {
        self.as_io_mut().truncate(size)
    }

    fn url(&self) -> Option<&Url> {
        self.as_io().url()
    }

    fn media_type(&self) -> &MediaType {
        self.as_io().media_type()
    }

    fn set_media_type(&mut self, media_type: MediaType) {
        self.as_io_mut().set_media_type(media_type);
    }

    fn flush(&mut self) -> Result<()> {
        self.as_io_mut().flush()
    }

    fn open(&mut self) -> Result<()> {
        self.as_io_mut().open()
    }

    fn opened(&self) -> bool {
        self.as_io().opened()
    }

    fn close(&mut self) -> Result<()> {
        self.as_io_mut().close()
    }

    fn clear(&mut self) -> Result<()> {
        self.as_io_mut().clear()
    }

    fn remove(&mut self, recursive: bool) -> Result<()> {
        self.as_io_mut().remove(recursive)
    }

    fn parent(&self) -> Option<Self> {
        self.as_io().parent()
    }

    fn child_by(&self, name: &str) -> Result<Self> {
        self.as_io().child_by(name)
    }

    fn ls(&self, recursive: bool, include_private: bool) -> Result<Vec<Self>> {
        self.as_io().ls(recursive, include_private)
    }

    fn kind(&self) -> crate::IOKind {
        self.as_io().kind()
    }

    fn is_atomic(&self) -> bool {
        self.as_io().is_atomic()
    }

    fn is_tabular(&self) -> bool {
        self.as_io().is_tabular()
    }

    /// Forwarded rather than defaulted, so a [`Self::Text`] holder answers
    /// with its own extractor instead of the media type's default.
    #[cfg(feature = "arrow")]
    fn record_options(&self) -> Result<crate::generic::RecordOptions> {
        self.as_io().record_options()
    }
}

impl From<Buffer> for Holder {
    fn from(value: Buffer) -> Self {
        Self::Buffer(value)
    }
}

impl From<Folder> for Holder {
    fn from(value: Folder) -> Self {
        Self::Folder(value)
    }
}

impl From<File> for Holder {
    fn from(value: File) -> Self {
        Self::File(value)
    }
}

impl From<crate::arrowfs::Folder> for Holder {
    fn from(value: crate::arrowfs::Folder) -> Self {
        Self::ArrowFolder(value)
    }
}

impl From<crate::arrowfs::Path> for Holder {
    fn from(value: crate::arrowfs::Path) -> Self {
        Self::ArrowPath(value)
    }
}

impl From<crate::arrowfs::File> for Holder {
    fn from(value: crate::arrowfs::File) -> Self {
        Self::ArrowFile(value)
    }
}

impl From<Buffered<Holder>> for Holder {
    fn from(value: Buffered<Self>) -> Self {
        Self::Buffered(Box::new(value))
    }
}

impl From<crate::text::Text<Holder>> for Holder {
    fn from(value: crate::text::Text<Self>) -> Self {
        Self::Text(Box::new(value))
    }
}
