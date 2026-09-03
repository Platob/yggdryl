//! Stateful plain-text record media over one byte handle.

#[cfg(feature = "arrow")]
use smol_str::SmolStr;

#[cfg(feature = "arrow")]
use crate::generic::{IORecordOptions as _, RecordOptions};
use crate::io::{IOBase, IOMedia};
#[cfg(feature = "arrow")]
use crate::{Field, Result};

use super::TextOptions;

/// A byte handle retained with one flat plain-text record configuration.
///
/// `Text` adds no line-specific read surface. Rows still flow through the
/// ordinary [`IOMedia`] methods; the wrapper only retains the `TextOptions`
/// returned by [`IOMedia::record_options`].
#[derive(Debug)]
pub struct Text<H: IOBase> {
    handle: H,
    options: TextOptions,
}

impl<H: IOBase> Text<H> {
    /// Wrap a handle with default plain-text record options.
    #[must_use]
    pub fn new(handle: H) -> Self {
        Self {
            handle,
            options: TextOptions::new(),
        }
    }

    /// Return this media with a complete flat text configuration.
    #[must_use]
    pub fn with_options(mut self, options: TextOptions) -> Self {
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

    /// Borrow the retained text options.
    pub const fn options(&self) -> &TextOptions {
        &self.options
    }

    /// Borrow the retained text options mutably.
    pub fn options_mut(&mut self) -> &mut TextOptions {
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

    /// Return this text media unchanged.
    ///
    /// This inherent method makes ordinary `handle.into_text().into_text()`
    /// idempotent because inherent methods win over [`IOBase::into_text`].
    #[must_use]
    pub const fn into_text(self) -> Self {
        self
    }

    /// Replace the retained configuration without nesting another wrapper.
    #[must_use]
    pub fn into_text_with(self, options: TextOptions) -> Self {
        self.with_options(options)
    }

    #[cfg(feature = "arrow")]
    fn require_text_options<'a>(&self, options: &'a RecordOptions) -> Result<&'a TextOptions> {
        match options {
            RecordOptions::Text(options) => Ok(options),
            _ => Err(crate::Error::InvalidRecord {
                path: SmolStr::new_static("$.encoding"),
                reason: crate::text::expected_got("plain-text record options", options.mime_type()),
            }),
        }
    }
}

impl<H: IOBase> IOMedia for Text<H> {
    fn as_io_base(&self) -> &dyn IOBase {
        self
    }

    fn as_io_base_mut(&mut self) -> &mut dyn IOBase {
        self
    }

    #[cfg(feature = "arrow")]
    fn row_size(&self) -> Result<u64> {
        super::arrow::row_size(&self.handle, &self.options)
    }

    #[cfg(feature = "arrow")]
    fn column_size(&self) -> Result<usize> {
        if let Some(field) = self.options.field() {
            return Ok(field.field_len());
        }
        if self.handle.is_empty() {
            return Ok(0);
        }
        let options = RecordOptions::Text(Box::new(self.options.clone()));
        Ok(IOMedia::read_arrow_field(self, &options)?.field_len())
    }

    #[cfg(feature = "arrow")]
    fn record_options(&self) -> Result<RecordOptions> {
        Ok(self.options.clone().into())
    }

    #[cfg(feature = "arrow")]
    fn read_arrow_field(&self, options: &RecordOptions) -> Result<Field> {
        self.require_text_options(options)?;
        IOMedia::read_arrow_field(&self.handle, options)
    }

    #[cfg(feature = "arrow")]
    fn read_arrow_reader(&self, options: &RecordOptions) -> Result<crate::arrow::BatchReader> {
        self.require_text_options(options)?;
        IOMedia::read_arrow_reader(&self.handle, options)
    }

    #[cfg(feature = "arrow")]
    fn overwrite_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &RecordOptions,
    ) -> Result<()> {
        self.require_text_options(options)?;
        IOMedia::overwrite_arrow_reader(&mut self.handle, batches, options)
    }

    #[cfg(feature = "arrow")]
    fn overwrite_prepared_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &RecordOptions,
    ) -> Result<()> {
        self.require_text_options(options)?;
        IOMedia::overwrite_prepared_arrow_reader(&mut self.handle, batches, options)
    }

    #[cfg(feature = "arrow")]
    fn append_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &RecordOptions,
    ) -> Result<()> {
        self.require_text_options(options)?;
        IOMedia::append_arrow_reader(&mut self.handle, batches, options)
    }

    #[cfg(feature = "arrow")]
    fn merge_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &RecordOptions,
    ) -> Result<()> {
        self.require_text_options(options)?;
        IOMedia::merge_arrow_reader(&mut self.handle, batches, options)
    }
}

impl<H: IOBase> IOBase for Text<H> {
    crate::delegate_iobase!(handle: pread, pstream_bytes, pwrite, size, capacity, reserve,
        truncate, url, media_type, set_media_type, flush, open, opened, close, parent,
        child_by_path, ls, kind, clear, remove, is_atomic, is_io);

    fn is_tabular(&self) -> bool {
        true
    }
}
