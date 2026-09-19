//! Byte storage handles and the concrete [`Holder`] that unifies them.
//!
//! [`Buffer`] owns in-memory bytes, [`crate::local`], [`crate::fs`], and [`crate::zip`] supply the
//! location/container/leaf backend roles - a local tree, a foreign filesystem,
//! and the members one archive holds inside a single file - and [`buffered`]
//! adds a page cache over any [`IOBase`] implementation.

mod buffer;
pub mod buffered;
pub mod counted;

pub use buffer::Buffer;

use crate::coding::Coded;
use crate::holder::buffered::{Buffered, BufferedOptions};
use crate::local::{File, Folder};
use crate::{MediaType, Result, Url};

use crate::IOBase;

/// One instant in UTC nanoseconds since the Unix epoch, from a system clock
/// reading.
///
/// The one owner of that conversion: a store reports a modification time as a
/// `SystemTime`, every column and accessor that carries one counts nanoseconds
/// from the epoch, and a reading before the epoch counts backwards rather than
/// saturating at it.
pub(crate) fn system_time_ns(value: std::time::SystemTime) -> Option<i64> {
    match value.duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => i64::try_from(duration.as_nanos()).ok(),
        Err(error) => i64::try_from(error.duration().as_nanos())
            .ok()
            .and_then(i64::checked_neg),
    }
}

/// A concrete, sized value holding any core [`IOBase`] implementation.
///
/// Hierarchy accessors such as [`IOBase::parent`], [`IOBase::child_by_path`], and
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
/// use yggdryl::holder::Holder;
/// use yggdryl::IOBase;
/// use yggdryl::local::Folder;
///
/// # fn main() -> yggdryl::Result<()> {
/// let root = Holder::folder(Folder::temporary()?.path()?)?;
/// assert!(root.is_container());
///
/// // A leaf is a mapped file, and it need not exist yet.
/// let leaf = root.child_by_path("yggdryl-holder-doc.bin")?;
/// assert!(!leaf.is_container());
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
#[non_exhaustive]
#[allow(clippy::large_enum_variant)]
pub enum Holder {
    /// An in-memory byte array.
    Buffer(Buffer),
    /// A local directory.
    Folder(Folder),
    /// A local location that resolves to whatever it turns out to be.
    Path(crate::local::Path),
    /// A memory-mapped local file.
    File(File),
    /// A directory on a foreign filesystem.
    FsFolder(crate::fs::Folder),
    /// A foreign-filesystem location that resolves to whatever it turns out
    /// to be.
    FsPath(crate::fs::Path),
    /// A stream-backed file on an Arrow-compatible filesystem.
    FsFile(crate::fs::File),
    /// A key prefix, or a whole container, on an object store.
    #[cfg(feature = "object")]
    ObjectFolder(crate::object::Folder),
    /// An object-store location that resolves to whatever it turns out to be.
    #[cfg(feature = "object")]
    ObjectPath(crate::object::Path),
    /// One object on an object store.
    #[cfg(feature = "object")]
    ObjectFile(crate::object::File),
    /// A prefix of one ZIP archive's members, or the archive root.
    ZipNode(crate::zip::Node),
    /// A location inside a ZIP archive that resolves to whatever it holds.
    ZipPath(crate::zip::Path),
    /// One member of a ZIP archive, addressed positionally.
    ZipLeaf(crate::zip::Leaf),
    /// Any of the others, read through a page cache.
    ///
    /// The box is what keeps the enum a fixed size: this variant holds a
    /// handle of the very type it belongs to.
    Buffered(Box<Buffered<Self>>),
    /// Any of the others, presenting the decoded bytes of its content coding.
    ///
    /// Boxed for the same reason: a [`Coded`] handle owns the `Holder` that
    /// holds the encoded form.
    Coded(Box<Coded>),
    /// Any handle retained as plain-text record media.
    ///
    /// Boxed because the text wrapper owns another `Holder` while keeping its
    /// flat [`TextOptions`](crate::text::TextOptions) as the default record
    /// configuration.
    Text(Box<crate::text::Text<Self>>),
    /// Any of the others, retained behind its inferred record encoding.
    ///
    /// The box breaks the recursive shape: a [`Media`](crate::media::Media)
    /// owns a `Holder` as its byte handle, while this variant lets a binding
    /// keep that media wrapper (and its opened-session metadata cache) without
    /// changing from the one `Holder` surface.
    Media(Box<crate::media::Media>),
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
    /// The returned [`Self::Path`] resolves only when an operation needs to
    /// know what is there. A caller that already knows the role can select
    /// [`Self::folder`] or [`Self::file`] explicitly.
    ///
    /// # Errors
    ///
    /// Returns an error only when the path cannot be expressed as a canonical
    /// `file:` URL.
    pub fn local(path: impl AsRef<std::path::Path>) -> Result<Self> {
        Ok(Self::Path(crate::local::Path::new(path)?))
    }

    /// Hold the resource a URL names, opened with properties.
    ///
    /// This is the one door every backend is behind, and what a plan's
    /// target opens: a `file:` URL is a local path, resolved to a folder or a
    /// file when an operation needs to know; a `file:` URL with a fragment is
    /// a member of a ZIP archive; an object-store URL - `s3:`, `gs:`, `az:`
    /// and their aliases - is held through the `object` feature, configured
    /// by the properties the store's own tooling names, read the way the
    /// object store options read them.
    ///
    /// Two properties are read here whatever the scheme: `media_type` (or
    /// `mime_type`, `content_type`) declares what the bytes are, and `codec`
    /// (or `content_encoding`) presents them decoded. Every other property
    /// is left to the backend, which ignores what it does not know.
    ///
    /// ```
    /// use yggdryl::holder::Holder;
    /// use yggdryl::{IOBase, MimeType, Url};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let url = Url::from_str("file:///lake/trades.bin")?;
    /// let held = Holder::from_url(&url, [("media_type", "application/vnd.apache.parquet")])?;
    /// assert_eq!(held.media_type().base(), &MimeType::PARQUET);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when the scheme is one no backend of this build
    /// holds, or a property this method reads does not parse.
    pub fn from_url<K, V>(url: &Url, properties: impl IntoIterator<Item = (K, V)>) -> Result<Self>
    where
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let properties: Vec<(String, String)> = properties
            .into_iter()
            .map(|(name, value)| (name.as_ref().to_owned(), value.as_ref().to_owned()))
            .collect();
        let mut held = if url.is_local() {
            if url
                .fragment(false)?
                .is_some_and(|fragment| !fragment.is_empty())
            {
                crate::zip::from_url(url)?
            } else {
                Self::Path(crate::local::Path::from_url(url.clone())?)
            }
        } else if url.scheme().is_object_store() {
            #[cfg(feature = "object")]
            {
                let options = crate::object::ObjectOptions::from_properties(
                    properties.iter().map(|(name, value)| (name, value)),
                )?;
                crate::object::located_with(&url.to_string(), options)?
            }
            #[cfg(not(feature = "object"))]
            {
                return Err(crate::Error::unsupported(
                    "holding an object store location without the object feature",
                    url.scheme().as_str(),
                ));
            }
        } else {
            return Err(crate::Error::unsupported(
                "holding a location of this scheme",
                url.scheme().as_str(),
            ));
        };
        for (name, value) in &properties {
            match name.to_ascii_lowercase().replace('-', "_").as_str() {
                "media_type" | "mime_type" | "content_type" => {
                    held.set_media_type(value.parse::<MediaType>()?);
                }
                "codec" | "content_encoding" => {
                    let codec = value.parse::<crate::Codec>()?;
                    held = held.into_coded_with(codec, crate::Level::default());
                }
                _ => {}
            }
        }
        Ok(held)
    }

    /// Hold the members of the archive `handle` addresses.
    ///
    /// The answer is the archive root: a container whose children are the
    /// members, resolved and walked exactly as any other container's are.
    /// Nothing is read until an operation needs the archive's index.
    ///
    /// ```
    /// use yggdryl::holder::{Buffer, Holder};
    /// use yggdryl::IOBase;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let root = Holder::zip(Holder::buffer(Buffer::new()));
    /// root.child_by_path("trades/eu.csv")?
    ///     .write_all_bytes(b"symbol,price\nAAPL,187.23\n")?;
    ///
    /// assert_eq!(root.child_by_path("trades/eu.csv")?.size(), 25);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn zip(handle: Self) -> Self {
        crate::zip::mount(handle)
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

    /// Retain the record implementation inferred from this handle's media type.
    ///
    /// The conversion is lazy: it only adds the stateful wrapper and reads no
    /// bytes. IPC, Parquet (when enabled), and Avro are held through
    /// [`Self::Media`]; plain text is retained through [`Self::Text`].
    /// A page cache remains the outermost wrapper, so promotion followed by
    /// repeated buffering cannot stack caches.
    ///
    /// A directory, JSON document, or any other ordinary byte representation
    /// is deliberately returned unchanged. This is a best-fitting view, not a
    /// request that the handle must be tabular, so unsupported media is never
    /// an error.
    #[must_use]
    pub fn into_media(self) -> Self {
        {
            let base = self.media_type().base().clone();
            self.into_media_base(&base)
        }
    }

    /// Retain the content coding *and* record implementation this handle's
    /// *name* declares.
    ///
    /// [`Self::into_coded`] and [`Self::into_media`] each ask the handle what
    /// it holds, and an unresolved location answers that by looking at the
    /// store. This asks the location instead, so composing costs nothing, works
    /// on a resource that is not there yet, and never turns a description into
    /// a round trip - which is what lets a handle arrive composed the moment it
    /// is described. A handle with no location - an in-memory buffer - keeps
    /// answering from what it holds, because that is already free.
    ///
    /// The coding goes underneath, because the record implementation reads the
    /// *decoded* bytes: `trades.txt.gz` becomes text records over a gzip view
    /// over the location. Both halves stay lazy and both stay idempotent, so a
    /// handle that already presents decoded bytes or already retains a record
    /// implementation keeps the one it has.
    ///
    /// A name is a name, not a probe: a *container* whose own name ends in a
    /// record suffix composes as that encoding, exactly as
    /// [`Self::into_media`] would once it had looked.
    ///
    /// ```
    /// use yggdryl::holder::{Buffer, Holder};
    /// use yggdryl::{Codec, IOBase, MimeType, Url};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let named = Url::from_str("file:///trades.txt.gz")?;
    /// let stored = Codec::Gzip.dump(b"AAPL,1\n")?;
    /// let handle = Holder::buffer(Buffer::from_bytes(stored).with_media_type(named.media_type()));
    ///
    /// let composed = handle.into_declared_media();
    ///
    /// // Text records over the decoded bytes, with the coding removed.
    /// assert!(matches!(composed, Holder::Text(_)));
    /// assert_eq!(composed.read_all_bytes()?, b"AAPL,1\n");
    /// assert_eq!(composed.media_type().base(), &MimeType::PLAIN_TEXT);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn into_declared_media(self) -> Self {
        // Only the two unresolved roles would look at the store to answer
        // [`IOBase::media_type`]; their name says the same thing for free.
        // Every other variant already answers from what it is, which is how a
        // directory - whose own name may end in a record suffix - stays a
        // directory here.
        let media_type = match &self {
            Self::Path(path) => path.url().media_type(),
            Self::FsPath(path) => path.url().media_type(),
            other => other.media_type().clone(),
        };
        self.into_media_as(&media_type)
    }

    /// Retain the content coding and record implementation `media_type` names.
    #[must_use]
    fn into_media_as(self, media_type: &MediaType) -> Self {
        // A handle that already retains a record implementation is already
        // composed; re-applying the coding underneath it would stack a second
        // one for the same declaration.
        if self.has_media_surface() {
            return self;
        }

        let codec = crate::Codec::from_media_type(media_type);

        // Parquet compresses internally, so `trades.parquet.gz` names a file no
        // other Parquet reader can open. Composing it would hide that behind a
        // decoded view; leaving the name alone keeps the writer's refusal,
        // which is the answer a caller needs before the file exists.
        #[cfg(feature = "parquet")]
        if !codec.is_identity() && *media_type.base() == crate::MimeType::PARQUET {
            return self;
        }

        let coded = match codec {
            crate::Codec::Identity => self,
            codec => self.into_coded_with(codec, crate::Level::DEFAULT),
        };

        coded.into_media_base(media_type.base())
    }

    /// Retain the record implementation one base representation names.
    #[must_use]
    fn into_media_base(self, base: &crate::MimeType) -> Self {
        if self.has_media_surface() {
            return self;
        }

        let supported = *base == crate::MimeType::ARROW_STREAM
            || *base == crate::MimeType::ARROW_FILE
            || *base == crate::MimeType::AVRO
            || *base == crate::MimeType::PLAIN_TEXT
            || cfg!(feature = "parquet") && *base == crate::MimeType::PARQUET;
        if !supported {
            return self;
        }

        // Keep an existing page cache outside the media wrapper. Besides
        // preserving the cache's one-layer invariant, this lets its
        // IOMedia delegation reach the retained encoding override.
        if let Self::Buffered(buffered) = self {
            let options = *buffered.options();
            let held = buffered.into_handle().into_media_base(base);
            return Self::Buffered(Box::new(Buffered::new(held, options)));
        }

        if *base == crate::MimeType::ARROW_STREAM || *base == crate::MimeType::ARROW_FILE {
            return Self::Media(Box::new(crate::media::Media::ipc(self)));
        }
        #[cfg(feature = "parquet")]
        if *base == crate::MimeType::PARQUET {
            return Self::Media(Box::new(crate::media::Media::parquet(self)));
        }
        if *base == crate::MimeType::PLAIN_TEXT {
            return self.into_text();
        }
        debug_assert_eq!(*base, crate::MimeType::AVRO);
        Self::Media(Box::new(crate::media::Media::avro(self)))
    }

    /// Materialize this holder and retain any record metadata the inferred
    /// media implementation can cache.
    ///
    /// This inherent method intentionally shadows [`IOBase::open`] for a
    /// concrete `Holder`. Stateful media wrappers call `open` through their
    /// generic `H: IOBase`, so their inner holder reaches the trait method and
    /// cannot recursively promote itself into the same encoding.
    ///
    /// # Errors
    ///
    /// Returns the selected handle or media implementation's open failure.
    pub fn open(&mut self) -> Result<()> {
        let held = std::mem::replace(self, Self::Buffer(Buffer::new()));
        *self = held.into_media();
        IOBase::open(self)
    }

    /// Return whether this holder already retains a media implementation.
    fn has_media_surface(&self) -> bool {
        match self {
            Self::Media(_) | Self::Text(_) => true,
            Self::Buffered(buffered) => buffered.handle().has_media_surface(),
            _ => false,
        }
    }

    /// Retain this holder as plain-text record media.
    ///
    /// Repeating the conversion preserves the existing `TextOptions`, and a
    /// page cache stays the outermost wrapper.
    #[must_use]
    pub fn into_text(self) -> Self {
        match self {
            Self::Text(text) => Self::Text(text),
            Self::Buffered(buffered) => {
                let options = *buffered.options();
                let held = buffered.into_handle().into_text();
                Self::Buffered(Box::new(Buffered::new(held, options)))
            }
            other => Self::Text(Box::new(crate::text::Text::new(other))),
        }
    }

    /// Retain this holder as plain-text record media with explicit options.
    ///
    /// Repeating the conversion replaces the retained text configuration
    /// without nesting another media wrapper.
    #[must_use]
    pub fn into_text_with(self, options: crate::text::TextOptions) -> Self {
        match self {
            Self::Text(text) => Self::Text(Box::new(text.with_options(options))),
            Self::Buffered(buffered) => {
                let buffered_options = *buffered.options();
                let held = buffered.into_handle().into_text_with(options);
                Self::Buffered(Box::new(Buffered::new(held, buffered_options)))
            }
            other => Self::Text(Box::new(
                crate::text::Text::new(other).with_options(options),
            )),
        }
    }

    /// Retain this holder behind the content coding its media type names.
    ///
    /// The wrapper presents the *decoded* bytes, so a handle named
    /// `app.log.gz` reads as the log it holds and reports `text/plain`. The
    /// conversion is lazy: it reads nothing and decodes nothing until a read
    /// asks for bytes.
    ///
    /// A handle whose name declares no coding wraps as
    /// [`Codec::Identity`](crate::Codec::Identity), which passes its bytes
    /// through unchanged, so this is safe to call on any leaf. A holder that
    /// already presents decoded bytes is returned unchanged - decoded bytes
    /// have no second coding to remove - and a page cache stays the outermost
    /// wrapper.
    #[must_use]
    pub fn into_coded(self) -> Self {
        let codec = self.codec();
        self.into_coded_with(codec, crate::Level::DEFAULT)
    }

    /// Retain this holder behind an explicit content coding and level.
    ///
    /// `level` is the scale writes encode at; reads ignore it. As with
    /// [`Self::into_coded`], a holder that already presents decoded bytes is
    /// returned unchanged, so neither argument re-codes an existing view.
    ///
    /// A page cache and a retained plain-text configuration both stay outside
    /// the coding: the cache for its one-layer invariant, and the text wrapper
    /// because its options describe the *decoded* rows, which a coding placed
    /// over it would hide.
    #[must_use]
    pub fn into_coded_with(self, codec: crate::Codec, level: crate::Level) -> Self {
        match self {
            Self::Coded(coded) => Self::Coded(coded),
            Self::Buffered(buffered) => {
                let options = *buffered.options();
                let held = buffered.into_handle().into_coded_with(codec, level);
                Self::Buffered(Box::new(Buffered::new(held, options)))
            }
            Self::Text(text) => {
                let options = text.options().clone();
                let held = text.into_handle().into_coded_with(codec, level);
                Self::Text(Box::new(crate::text::Text::new(held).with_options(options)))
            }
            other => Self::Coded(Box::new(Coded::wrap(other, codec).with_level(level))),
        }
    }

    /// Borrow the held implementation as a trait object.
    pub fn as_io(&self) -> &dyn IOBase {
        match self {
            Self::Buffer(inner) => inner,
            Self::Folder(inner) => inner,
            Self::Path(inner) => inner,
            Self::File(inner) => inner,
            Self::FsFolder(inner) => inner,
            Self::FsPath(inner) => inner,
            Self::FsFile(inner) => inner,
            #[cfg(feature = "object")]
            Self::ObjectFolder(inner) => inner,
            #[cfg(feature = "object")]
            Self::ObjectPath(inner) => inner,
            #[cfg(feature = "object")]
            Self::ObjectFile(inner) => inner,
            Self::ZipNode(inner) => inner,
            Self::ZipPath(inner) => inner,
            Self::ZipLeaf(inner) => inner,
            Self::Buffered(inner) => inner.as_ref(),
            Self::Coded(inner) => inner.as_io(),
            Self::Text(inner) => inner.as_ref(),
            Self::Media(inner) => inner.as_ref(),
        }
    }

    /// Borrow the held implementation mutably as a trait object.
    pub fn as_io_mut(&mut self) -> &mut dyn IOBase {
        match self {
            Self::Buffer(inner) => inner,
            Self::Folder(inner) => inner,
            Self::Path(inner) => inner,
            Self::File(inner) => inner,
            Self::FsFolder(inner) => inner,
            Self::FsPath(inner) => inner,
            Self::FsFile(inner) => inner,
            #[cfg(feature = "object")]
            Self::ObjectFolder(inner) => inner,
            #[cfg(feature = "object")]
            Self::ObjectPath(inner) => inner,
            #[cfg(feature = "object")]
            Self::ObjectFile(inner) => inner,
            Self::ZipNode(inner) => inner,
            Self::ZipPath(inner) => inner,
            Self::ZipLeaf(inner) => inner,
            Self::Buffered(inner) => inner.as_mut(),
            Self::Coded(inner) => inner.as_io_mut(),
            Self::Text(inner) => inner.as_mut(),
            Self::Media(inner) => inner.as_mut(),
        }
    }

    /// Borrow the held implementation through its media contract.
    fn as_media(&self) -> &dyn crate::IOMedia {
        match self {
            Self::Buffer(inner) => inner,
            Self::Folder(inner) => inner,
            Self::Path(inner) => inner,
            Self::File(inner) => inner,
            Self::FsFolder(inner) => inner,
            Self::FsPath(inner) => inner,
            Self::FsFile(inner) => inner,
            #[cfg(feature = "object")]
            Self::ObjectFolder(inner) => inner,
            #[cfg(feature = "object")]
            Self::ObjectPath(inner) => inner,
            #[cfg(feature = "object")]
            Self::ObjectFile(inner) => inner,
            Self::ZipNode(inner) => inner,
            Self::ZipPath(inner) => inner,
            Self::ZipLeaf(inner) => inner,
            Self::Buffered(inner) => inner.as_ref(),
            Self::Coded(inner) => inner.as_ref(),
            Self::Text(inner) => inner.as_ref(),
            Self::Media(inner) => inner.as_ref(),
        }
    }

    /// Mutably borrow the held implementation through its media contract.
    fn as_media_mut(&mut self) -> &mut dyn crate::IOMedia {
        match self {
            Self::Buffer(inner) => inner,
            Self::Folder(inner) => inner,
            Self::Path(inner) => inner,
            Self::File(inner) => inner,
            Self::FsFolder(inner) => inner,
            Self::FsPath(inner) => inner,
            Self::FsFile(inner) => inner,
            #[cfg(feature = "object")]
            Self::ObjectFolder(inner) => inner,
            #[cfg(feature = "object")]
            Self::ObjectPath(inner) => inner,
            #[cfg(feature = "object")]
            Self::ObjectFile(inner) => inner,
            Self::ZipNode(inner) => inner,
            Self::ZipPath(inner) => inner,
            Self::ZipLeaf(inner) => inner,
            Self::Buffered(inner) => inner.as_mut(),
            Self::Coded(inner) => inner.as_mut(),
            Self::Text(inner) => inner.as_mut(),
            Self::Media(inner) => inner.as_mut(),
        }
    }
}

impl crate::IOMedia for Holder {
    fn as_io_base(&self) -> &dyn IOBase {
        self.as_io()
    }

    fn as_io_base_mut(&mut self) -> &mut dyn IOBase {
        self.as_io_mut()
    }

    fn row_size(&self) -> Result<u64> {
        crate::IOMedia::row_size(self.as_media())
    }

    fn column_size(&self) -> Result<usize> {
        crate::IOMedia::column_size(self.as_media())
    }

    fn record_options(&self) -> Result<crate::media::RecordOptions> {
        crate::IOMedia::record_options(self.as_media())
    }

    #[cfg(feature = "parquet")]
    fn read_parquet_statistics(&self) -> Result<crate::parquet::FileStatistics> {
        crate::IOMedia::read_parquet_statistics(self.as_media())
    }

    #[cfg(feature = "parquet")]
    fn read_parquet_geospatial_statistics(
        &self,
        column: &str,
    ) -> Result<crate::parquet::GeospatialStatistics> {
        crate::IOMedia::read_parquet_geospatial_statistics(self.as_media(), column)
    }

    fn read_arrow_field(&self, options: &crate::media::RecordOptions) -> Result<crate::Field> {
        crate::IOMedia::read_arrow_field(self.as_media(), options)
    }

    fn read_arrow_reader(
        &self,
        options: &crate::media::RecordOptions,
    ) -> Result<crate::arrow::BatchReader> {
        crate::IOMedia::read_arrow_reader(self.as_media(), options)
    }

    fn overwrite_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &crate::media::RecordOptions,
    ) -> Result<()> {
        crate::IOMedia::overwrite_arrow_reader(self.as_media_mut(), batches, options)
    }

    fn overwrite_prepared_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &crate::media::RecordOptions,
    ) -> Result<()> {
        crate::IOMedia::overwrite_prepared_arrow_reader(self.as_media_mut(), batches, options)
    }

    fn overwrite_arrow_batch(
        &mut self,
        batch: arrow_array::RecordBatch,
        options: &crate::media::RecordOptions,
    ) -> Result<()> {
        crate::IOMedia::overwrite_arrow_batch(self.as_media_mut(), batch, options)
    }

    fn append_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &crate::media::RecordOptions,
    ) -> Result<()> {
        crate::IOMedia::append_arrow_reader(self.as_media_mut(), batches, options)
    }

    fn append_arrow_batch(
        &mut self,
        batch: arrow_array::RecordBatch,
        options: &crate::media::RecordOptions,
    ) -> Result<()> {
        crate::IOMedia::append_arrow_batch(self.as_media_mut(), batch, options)
    }

    fn merge_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &crate::media::RecordOptions,
    ) -> Result<()> {
        crate::IOMedia::merge_arrow_reader(self.as_media_mut(), batches, options)
    }

    fn merge_arrow_batch(
        &mut self,
        batch: arrow_array::RecordBatch,
        options: &crate::media::RecordOptions,
    ) -> Result<()> {
        crate::IOMedia::merge_arrow_batch(self.as_media_mut(), batch, options)
    }
}

impl IOBase for Holder {
    fn pread(&self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        self.as_io().pread(offset, buffer)
    }

    fn pstream_bytes(&self, position: u64, batch_size: usize) -> Result<crate::ByteStream<'_>> {
        self.as_io().pstream_bytes(position, batch_size)
    }

    fn read_all_bytes(&self) -> Result<Vec<u8>> {
        self.as_io().read_all_bytes()
    }

    fn read_range_bytes(&self, offset: u64, length: usize) -> Result<Vec<u8>> {
        self.as_io().read_range_bytes(offset, length)
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

    fn bound_location(&self) -> Option<&crate::fs::BoundLocation> {
        self.as_io().bound_location()
    }

    fn mtime(&self) -> Option<i64> {
        self.as_io().mtime()
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

    fn child_by_path(&self, name: &str) -> Result<Self> {
        self.as_io().child_by_path(name)
    }

    fn ls(&self, recursive: bool, include_private: bool) -> crate::Listing {
        self.as_io().ls(recursive, include_private)
    }

    fn glob(&self, pattern: &str, include_private: bool) -> Result<crate::Listing> {
        self.as_io().glob(pattern, include_private)
    }

    /// Forwarded, because a handle can spell its partitions somewhere other
    /// than in its URL path - an archive member spells them in its own name.
    fn partitions(&self) -> Vec<(String, String)> {
        self.as_io().partitions()
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
}

impl TryFrom<&Url> for Holder {
    type Error = crate::Error;

    fn try_from(url: &Url) -> Result<Self> {
        let none: [(&str, &str); 0] = [];
        Self::from_url(url, none)
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

impl From<crate::fs::Folder> for Holder {
    fn from(value: crate::fs::Folder) -> Self {
        Self::FsFolder(value)
    }
}

impl From<crate::fs::Path> for Holder {
    fn from(value: crate::fs::Path) -> Self {
        Self::FsPath(value)
    }
}

impl From<crate::fs::File> for Holder {
    fn from(value: crate::fs::File) -> Self {
        Self::FsFile(value)
    }
}

impl From<Buffered<Holder>> for Holder {
    fn from(value: Buffered<Self>) -> Self {
        Self::Buffered(Box::new(value))
    }
}

impl From<Coded> for Holder {
    fn from(value: Coded) -> Self {
        Self::Coded(Box::new(value))
    }
}

impl From<crate::text::Text<Holder>> for Holder {
    fn from(value: crate::text::Text<Self>) -> Self {
        Self::Text(Box::new(value))
    }
}

impl From<crate::media::Media> for Holder {
    fn from(value: crate::media::Media) -> Self {
        Self::Media(Box::new(value))
    }
}
