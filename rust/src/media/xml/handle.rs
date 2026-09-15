//! Stateful XML record media over one byte handle.

#[cfg(feature = "arrow")]
use smol_str::SmolStr;

#[cfg(feature = "arrow")]
use crate::media::{IORecordOptions as _, RecordOptions};
#[cfg(feature = "arrow")]
use crate::{Field, Result};
use crate::{IOBase, IOMedia};

use super::XmlOptions;

/// A byte handle retained with one XML record configuration.
///
/// `Xml` adds no document-specific read surface: rows flow through the
/// ordinary [`IOMedia`] methods, and the wrapper only retains the
/// [`XmlOptions`] that [`IOMedia::record_options`] answers.
///
/// Nothing is cached between calls. A document states no schema and carries no
/// index, so there is no header to hold onto - the metadata a cache would keep
/// is the read itself, and holding it would be holding the document.
#[derive(Debug)]
pub struct Xml<H: IOBase> {
    handle: H,
    options: XmlOptions,
}

impl<H: IOBase> Xml<H> {
    /// Wrap a handle with default XML record options.
    #[must_use]
    pub fn new(handle: H) -> Self {
        Self {
            handle,
            options: XmlOptions::new(),
        }
    }

    /// Return this media with a complete XML configuration.
    #[must_use]
    pub fn with_options(mut self, options: XmlOptions) -> Self {
        self.options = options;
        self
    }

    /// Return this media with a declared canonical row field.
    #[must_use]
    #[cfg(feature = "arrow")]
    pub fn with_field(mut self, field: Field) -> Self {
        self.options.set_field(field);
        self
    }

    /// Borrow the retained XML options.
    pub const fn options(&self) -> &XmlOptions {
        &self.options
    }

    /// Borrow the retained XML options mutably.
    pub fn options_mut(&mut self) -> &mut XmlOptions {
        &mut self.options
    }

    /// Borrow the underlying byte handle.
    pub const fn handle(&self) -> &H {
        &self.handle
    }

    /// Borrow the underlying byte handle mutably.
    pub fn handle_mut(&mut self) -> &mut H {
        &mut self.handle
    }

    /// Consume this media and return its byte handle.
    pub fn into_handle(self) -> H {
        self.handle
    }

    #[cfg(feature = "arrow")]
    fn require_xml_options<'a>(&self, options: &'a RecordOptions) -> Result<&'a XmlOptions> {
        match options {
            RecordOptions::Xml(options) => Ok(options),
            _ => Err(crate::Error::InvalidRecord {
                path: SmolStr::new_static("$.encoding"),
                reason: crate::text::expected_got("XML record options", options.mime_type()),
            }),
        }
    }
}

impl<H: IOBase> IOMedia for Xml<H> {
    fn as_io_base(&self) -> &dyn IOBase {
        self
    }

    fn as_io_base_mut(&mut self) -> &mut dyn IOBase {
        self
    }

    #[cfg(feature = "arrow")]
    fn row_size(&self) -> Result<u64> {
        super::row_size(&self.handle, &self.options)
    }

    #[cfg(feature = "arrow")]
    fn column_size(&self) -> Result<usize> {
        if let Some(field) = self.options.field() {
            return Ok(field.field_len());
        }
        Ok(super::read_field(&self.handle, &self.options)?.field_len())
    }

    #[cfg(feature = "arrow")]
    fn record_options(&self) -> Result<RecordOptions> {
        Ok(RecordOptions::Xml(Box::new(self.options.clone())))
    }

    #[cfg(feature = "arrow")]
    fn read_arrow_field(&self, options: &RecordOptions) -> Result<Field> {
        let xml = self.require_xml_options(options)?;
        Ok(super::read_field(&self.handle, xml)?)
    }

    #[cfg(feature = "arrow")]
    fn read_arrow_reader(&self, options: &RecordOptions) -> Result<crate::arrow::BatchReader> {
        self.require_xml_options(options)?;
        IOMedia::read_arrow_reader(&self.handle, options)
    }

    #[cfg(feature = "arrow")]
    fn overwrite_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &RecordOptions,
    ) -> Result<()> {
        self.require_xml_options(options)?;
        IOMedia::overwrite_arrow_reader(&mut self.handle, batches, options)
    }

    #[cfg(feature = "arrow")]
    fn overwrite_prepared_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &RecordOptions,
    ) -> Result<()> {
        self.require_xml_options(options)?;
        IOMedia::overwrite_prepared_arrow_reader(&mut self.handle, batches, options)
    }

    #[cfg(feature = "arrow")]
    fn append_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &RecordOptions,
    ) -> Result<()> {
        self.require_xml_options(options)?;
        IOMedia::append_arrow_reader(&mut self.handle, batches, options)
    }

    #[cfg(feature = "arrow")]
    fn merge_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &RecordOptions,
    ) -> Result<()> {
        self.require_xml_options(options)?;
        IOMedia::merge_arrow_reader(&mut self.handle, batches, options)
    }
}

impl<H: IOBase> IOBase for Xml<H> {
    crate::delegate_iobase!(handle: pread, read_all_bytes, read_range_bytes, pstream_bytes,
        pwrite, size, capacity, reserve,
        truncate, url, bound_location, mtime, media_type, set_media_type, flush, open, opened, close, parent,
        child_by_path, ls, kind, clear, remove, is_atomic, is_io);

    fn is_tabular(&self) -> bool {
        true
    }
}
