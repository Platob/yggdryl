//! [`MediaTable`]: a table over any location a record medium reads.

use std::fmt;

use smol_str::SmolStr;

use super::handle::{Handle, Site};
use super::object::path_text;
use super::{FolderLayout, IntoObjectPath, ObjectValue, Properties, TableValue};
use crate::holder::Holder;
use crate::media::{IORecordOptions, RecordOptions};
use crate::{DataType, Error, Field, IOBase, IOKind, IOMedia, MediaType, Result, Uri, Url};

/// A table over any location a record medium reads: a leaf, a folder read
/// as the rows beneath it, or a folder laid out as a table format.
///
/// The table holds a path, the identifier of its storage, what it states, an
/// optional description, how its rows are laid out and at most one declared
/// row schema, whose name is always the table's; its handle is opened on the
/// first verb that needs one, with [`Holder::from_url`] under the table's
/// effective properties and composed as its name declares. Every record
/// verb delegates to that handle, [`record_options`](IOMedia::record_options)
/// applying the declared field and the table name first and
/// [`read_arrow_field`](IOMedia::read_arrow_field) answering the declared
/// field before anything is read. A successful leaf record write closes its
/// located holder's session, releasing mappings and wrapper caches while
/// retaining its media and backend options. A bound handle with no site is
/// retained as the data itself. Folder and format writes and direct byte
/// operations keep their held session.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MediaTable {
    path: Vec<SmolStr>,
    uri: Option<Uri>,
    handle: Handle,
    stated: Properties,
    inherited: Properties,
    description: Option<String>,
    layout: FolderLayout,
    field: Option<Field>,
}

impl MediaTable {
    /// The table at `path` over the storage `uri` identifies, touching
    /// nothing.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] when the path is not one of parts or
    /// is empty, and the identifier's own refusal when it names no location.
    pub fn new(path: impl IntoObjectPath, uri: impl Into<Uri>) -> Result<Self> {
        let path = named_path(path)?;
        let uri = uri.into();
        let url = uri.locator()?;
        Ok(Self {
            handle: Handle::at(Site::Url(url), true, &path, Properties::new()),
            path,
            uri: Some(uri),
            stated: Properties::new(),
            inherited: Properties::new(),
            description: None,
            layout: FolderLayout::Leaf,
            field: None,
        })
    }

    /// The table at `path` over a handle already in hand.
    ///
    /// The handle is composed as its name declares and kept; a clone
    /// rebuilds one from the handle's location, and says so by name when
    /// the handle has none.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] when the path is not one of parts or
    /// is empty.
    pub fn bound(path: impl IntoObjectPath, holder: Holder) -> Result<Self> {
        let path = named_path(path)?;
        let uri = holder.uri().cloned();
        let layout = if holder.is_container() {
            FolderLayout::Folder
        } else {
            FolderLayout::Leaf
        };
        Ok(Self {
            handle: Handle::bound(holder, true, &path, Properties::new()),
            path,
            uri,
            stated: Properties::new(),
            inherited: Properties::new(),
            description: None,
            layout,
            field: None,
        })
    }

    /// The table at `path` a folder listing found, its layout decided by the
    /// listing so the store is asked nothing more.
    pub(crate) fn listed(path: Vec<SmolStr>, holder: Holder, layout: FolderLayout) -> Self {
        let uri = holder.uri().cloned();
        Self {
            handle: Handle::bound(holder, true, &path, Properties::new()),
            path,
            uri,
            stated: Properties::new(),
            inherited: Properties::new(),
            description: None,
            layout,
            field: None,
        }
    }

    /// Return this table with its row schema declared.
    ///
    /// The field is renamed after the table, since a declared field's name
    /// is always the table's; its datatype and metadata are kept as given.
    #[must_use]
    pub fn with_field(mut self, field: Field) -> Self {
        self.field = Some(field.with_name(self.name().to_owned()));
        self
    }

    /// Return this table with its row datatype declared.
    ///
    /// An existing declared field keeps its nullability and metadata under
    /// the new datatype; without one, the table declares a required field
    /// of that datatype.
    ///
    /// # Errors
    ///
    /// Returns the field's refusal of the datatype.
    pub fn with_dtype(mut self, dtype: DataType) -> Result<Self> {
        let field = match self.field.take() {
            Some(mut field) => {
                field.set_dtype(dtype)?;
                field
            }
            None => Field::new(self.name().to_owned(), dtype, false),
        };
        self.field = Some(field);
        Ok(self)
    }

    /// Return this table with a description.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Return this table with stated properties, which its handle opens
    /// with.
    #[must_use]
    pub fn with_properties(mut self, properties: Properties) -> Self {
        self.stated = properties;
        self.handle.set_properties(self.effective());
        self
    }

    /// Return this table with its layout stated.
    #[must_use]
    pub const fn with_layout(mut self, layout: FolderLayout) -> Self {
        self.layout = layout;
        self
    }

    /// How the rows are laid out.
    #[must_use]
    pub const fn layout(&self) -> FolderLayout {
        self.layout
    }

    /// The declared row schema, when one was declared.
    #[must_use]
    pub const fn declared_field(&self) -> Option<&Field> {
        self.field.as_ref()
    }

    /// The identifier of the storage, when the table was given one.
    #[must_use]
    pub const fn uri(&self) -> Option<&Uri> {
        self.uri.as_ref()
    }

    /// The effective properties: what is inherited, then what is stated.
    fn effective(&self) -> Properties {
        self.stated.inherit(&self.inherited)
    }

    /// The handle, opened on the first call.
    fn handle(&self) -> Result<&Holder> {
        self.handle.get()
    }

    /// The handle, mutably, opened on the first call.
    fn handle_mut(&mut self) -> Result<&mut Holder> {
        self.handle.get_mut()
    }

    /// A completed leaf write no longer owns a read snapshot or a writer
    /// mapping. Close its located session before returning the descriptor.
    fn finish_write<T>(&mut self, result: Result<T>) -> Result<T> {
        if result.is_ok() && self.layout == FolderLayout::Leaf {
            self.handle.release_after_write()?;
        }
        result
    }

    /// Whether anything is at the table's location now.
    pub(crate) fn exists(&self) -> bool {
        self.handle.exists()
    }

    pub(crate) fn inheriting(mut self, parent: &Properties) -> Self {
        self.inherited = parent.clone();
        self.handle.set_properties(self.effective());
        self
    }

    /// The options every record verb runs with when the caller states none:
    /// the handle's, carrying the declared field and the table's name.
    fn options(&self) -> Result<RecordOptions> {
        if self.layout == FolderLayout::Format
            && crate::media::format::format_named(super::folder::TABLE_LAYOUT_FORMAT).is_none()
        {
            return Err(crate::media::format::unregistered(path_text(&self.path)));
        }
        let handle = self.handle()?;
        // A leaf's encoding is what its name declares, with no question to
        // the store; a folder's is what its leaves hold.
        let mut options = match self.layout {
            FolderLayout::Leaf => RecordOptions::for_media_type(handle.media_type())?,
            FolderLayout::Folder | FolderLayout::Format => handle.record_options()?,
        };
        if let Some(field) = &self.field {
            options.set_field(field.clone());
        }
        options.set_name(self.name().into());
        Ok(options)
    }
}

/// A non-empty path of parts, which a table needs for its name.
fn named_path(path: impl IntoObjectPath) -> Result<Vec<SmolStr>> {
    let path = path.into_object_path()?;
    if path.is_empty() {
        return Err(Error::InvalidRecord {
            path: SmolStr::new_static("$.path"),
            reason: SmolStr::new_static("expected a path naming the table, got the empty path"),
        });
    }
    Ok(path)
}

impl ObjectValue for MediaTable {
    fn name(&self) -> &str {
        self.path.last().map_or("", SmolStr::as_str)
    }

    fn path(&self) -> &[SmolStr] {
        &self.path
    }

    fn kind(&self) -> IOKind {
        IOKind::Table
    }

    fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    fn url(&self) -> Option<&Url> {
        self.handle.url()
    }

    fn modified(&self) -> Option<i64> {
        self.handle().ok()?.mtime()
    }

    fn properties(&self) -> Result<Properties> {
        Ok(self.effective())
    }
}

impl TableValue for MediaTable {
    fn field(&self) -> Result<Field> {
        if let Some(field) = &self.field {
            return Ok(field.clone());
        }
        let options = self.options()?;
        self.handle()?.read_arrow_field(&options)
    }

    fn storage(&self) -> String {
        match self.layout {
            FolderLayout::Format => IOKind::Table.as_str().to_owned(),
            FolderLayout::Folder => IOKind::Directory.as_str().to_owned(),
            FolderLayout::Leaf => match self.handle.url() {
                Some(url) => url.media_type().to_string(),
                None => self.handle().map_or_else(
                    |_| MediaType::default().to_string(),
                    |handle| handle.media_type().to_string(),
                ),
            },
        }
    }
}

impl fmt::Display for MediaTable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        super::object::write_path(formatter, &self.path)
    }
}

impl IOBase for MediaTable {
    fn pread(&self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        self.handle()?.pread(offset, buffer)
    }

    fn pstream_bytes(&self, position: u64, batch_size: usize) -> Result<crate::ByteStream<'_>> {
        self.handle()?.pstream_bytes(position, batch_size)
    }

    fn read_all_bytes(&self) -> Result<Vec<u8>> {
        self.handle()?.read_all_bytes()
    }

    fn read_range_bytes(&self, offset: u64, length: usize) -> Result<Vec<u8>> {
        self.handle()?.read_range_bytes(offset, length)
    }

    fn read_tail_bytes(&self, length: usize) -> Result<(Vec<u8>, u64)> {
        self.handle()?.read_tail_bytes(length)
    }

    fn read_digest(&self, algorithm: crate::DigestAlgorithm) -> Result<crate::Digest> {
        self.handle()?.read_digest(algorithm)
    }

    fn read_range_digest(
        &self,
        offset: u64,
        length: usize,
        algorithm: crate::DigestAlgorithm,
    ) -> Result<crate::Digest> {
        self.handle()?.read_range_digest(offset, length, algorithm)
    }

    fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> Result<usize> {
        self.handle_mut()?.pwrite(offset, bytes)
    }

    fn write_all_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.handle_mut()?.write_all_bytes(bytes)
    }

    fn create_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.handle_mut()?.create_bytes(bytes)
    }

    fn append_bytes(&mut self, bytes: &[u8]) -> Result<u64> {
        self.handle_mut()?.append_bytes(bytes)
    }

    fn size(&self) -> u64 {
        self.handle().map_or(0, IOBase::size)
    }

    fn capacity(&self) -> u64 {
        self.handle().map_or(0, IOBase::capacity)
    }

    fn reserve(&mut self, capacity: u64) -> Result<()> {
        self.handle_mut()?.reserve(capacity)
    }

    fn truncate(&mut self, size: u64) -> Result<()> {
        self.handle_mut()?.truncate(size)
    }

    fn uri(&self) -> Option<&Uri> {
        self.uri.as_ref()
    }

    fn url(&self) -> Option<&Url> {
        self.handle.url()
    }

    fn bound_location(&self) -> Option<&crate::fs::BoundLocation> {
        self.handle().ok()?.bound_location()
    }

    fn mtime(&self) -> Option<i64> {
        self.handle().ok()?.mtime()
    }

    fn media_type(&self) -> &MediaType {
        self.handle()
            .map_or(&crate::iobase::UNRESOLVED_MEDIA_TYPE, IOBase::media_type)
    }

    fn applied_codec(&self) -> crate::Codec {
        self.handle()
            .map_or(crate::Codec::Identity, IOBase::applied_codec)
    }

    fn set_media_type(&mut self, media_type: MediaType) {
        if let Ok(handle) = self.handle_mut() {
            handle.set_media_type(media_type);
        }
    }

    fn flush(&mut self) -> Result<()> {
        self.handle_mut()?.flush()
    }

    fn open(&mut self) -> Result<()> {
        self.handle_mut()?.open()
    }

    fn opened(&self) -> bool {
        self.handle.held().is_some_and(IOBase::opened)
    }

    fn close(&mut self) -> Result<()> {
        if self.handle.held().is_some() {
            // A media wrapper can hold mapped bytes without an explicit open.
            self.handle_mut()?.close()
        } else {
            // Closing an unresolved descriptor never opens its storage.
            Ok(())
        }
    }

    fn clear(&mut self) -> Result<()> {
        self.handle_mut()?.clear()
    }

    fn remove(&mut self, recursive: bool) -> Result<()> {
        self.handle_mut()?.remove(recursive)
    }

    fn parent(&self) -> Option<Holder> {
        self.handle().ok()?.parent()
    }

    fn child_by_path(&self, path: &str) -> Result<Holder> {
        self.handle()?.child_by_path(path)
    }

    fn ls(&self, recursive: bool, include_private: bool) -> crate::Listing {
        match self.handle() {
            Ok(handle) => handle.ls(recursive, include_private),
            Err(error) => crate::Listing::failing(error),
        }
    }

    fn glob(&self, pattern: &str, include_private: bool) -> Result<crate::Listing> {
        self.handle()?.glob(pattern, include_private)
    }

    fn partitions(&self) -> Vec<(String, String)> {
        self.handle().map(IOBase::partitions).unwrap_or_default()
    }

    fn kind(&self) -> IOKind {
        match self.layout {
            FolderLayout::Format => IOKind::Table,
            _ => self.handle().map_or(IOKind::Unknown, IOBase::kind),
        }
    }

    fn is_container(&self) -> bool {
        self.layout != FolderLayout::Leaf
    }

    fn is_atomic(&self) -> bool {
        false
    }

    fn is_tabular(&self) -> bool {
        true
    }
}

impl IOMedia for MediaTable {
    fn as_io_base(&self) -> &dyn IOBase {
        self
    }

    fn as_io_base_mut(&mut self) -> &mut dyn IOBase {
        self
    }

    fn row_size(&self) -> Result<u64> {
        IOMedia::row_size(self.handle()?)
    }

    fn column_size(&self) -> Result<usize> {
        if let Some(field) = &self.field {
            return Ok(field.field_len());
        }
        IOMedia::column_size(self.handle()?)
    }

    fn record_options(&self) -> Result<RecordOptions> {
        self.options()
    }

    // A handle that cannot open holds no medium's state; the verb that
    // reads it next reports why.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        self.handle().ok().and_then(IOMedia::as_any)
    }

    fn read_arrow_field(&self, options: &RecordOptions) -> Result<Field> {
        if options.field().is_none()
            && let Some(field) = &self.field
        {
            return Ok(field.clone());
        }
        IOMedia::read_arrow_field(self.handle()?, options)
    }

    // The table's own options carry its declared field and its name, so an
    // absent option set is resolved here rather than by the handle.
    fn read_serie(&self, options: Option<&RecordOptions>) -> Result<crate::Serie> {
        let options = crate::iomedia::own_options(self, options)?;
        IOMedia::read_serie(self.handle()?, Some(&options))
    }

    fn overwrite_serie(
        &mut self,
        value: crate::Serie,
        options: Option<&RecordOptions>,
    ) -> Result<crate::IOResult> {
        let options = crate::iomedia::own_options(&*self, options)?;
        let result = IOMedia::overwrite_serie(self.handle_mut()?, value, Some(&options));
        self.finish_write(result)
    }

    fn overwrite_prepared_serie(
        &mut self,
        value: crate::StreamChunkedSerie,
        options: &RecordOptions,
    ) -> Result<()> {
        let result = IOMedia::overwrite_prepared_serie(self.handle_mut()?, value, options);
        self.finish_write(result)
    }

    fn append_serie(
        &mut self,
        value: crate::Serie,
        options: Option<&RecordOptions>,
    ) -> Result<crate::IOResult> {
        let options = crate::iomedia::own_options(&*self, options)?;
        let result = IOMedia::append_serie(self.handle_mut()?, value, Some(&options));
        self.finish_write(result)
    }

    fn merge_serie(
        &mut self,
        value: crate::Serie,
        options: Option<&RecordOptions>,
    ) -> Result<crate::IOResult> {
        let options = crate::iomedia::own_options(&*self, options)?;
        let result = IOMedia::merge_serie(self.handle_mut()?, value, Some(&options));
        self.finish_write(result)
    }
}
