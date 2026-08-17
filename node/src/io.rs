//! `IOBase`, exposed to JavaScript with the method names `fs` and `path` use.
//!
//! The core trait is positional and fully random-access, so there are no flags
//! to open with and no descriptor to keep: `readBytes`, `writeBytes`, `ls`,
//! `glob`, `mkdir`, and `unlink` all mean here what they mean on a path in
//! `node:fs`, and each one is answered by the core implementation for the
//! backend the location names. Code written against a local directory therefore
//! runs against a bucket when that backend lands, because only the handle
//! changes.

use napi::bindgen_prelude::{
    Buffer, ClassInstance, Either, Either3, Reference, Result, Uint8Array,
};
use napi_derive::napi;

use yggdryl::generic::{Holder, IORecordOptions as _};
use yggdryl::io::IOBase as _;

use crate::arrow::JsBatchReader;
use crate::field::JsField;
use crate::generic::JsRecordOptions;
use crate::uri::{JsUrl, PartitionEntry, partition_entries};
use crate::{exact_u64, napi_error};

/// A native handle, a native `Url`, or anything that names a location.
pub(crate) type LocationInput<'a> =
    Either3<ClassInstance<'a, JsIOBase>, ClassInstance<'a, JsUrl>, String>;

/// A mapping of partition columns to values, or the same pairs as entries.
type PartitionFilters = Either<Vec<PartitionEntry>, std::collections::HashMap<String, String>>;

/// Build a local handle for the location a `Url` names.
fn local_holder(url: &yggdryl::Url) -> Result<Holder> {
    Holder::local(url.to_path().map_err(napi_error)?).map_err(napi_error)
}

/// Build a container handle for the location `value` names.
///
/// A folder is asked for by name rather than discovered, because a location
/// that holds nothing yet reads as a leaf: `Holder::local` cannot tell a
/// directory that does not exist from a file that does not exist, and a table
/// root has to be the former before anything is written into it.
pub(crate) fn folder_from_input(value: LocationInput<'_>) -> Result<Holder> {
    let url = match value {
        Either3::A(handle) => handle
            .inner
            .url()
            .cloned()
            .ok_or_else(|| napi_error("an in-memory resource cannot contain a table"))?,
        Either3::B(url) => url.inner.clone(),
        Either3::C(value) => yggdryl::Url::from_str(&value).map_err(napi_error)?,
    };
    Holder::folder(url.to_path().map_err(napi_error)?).map_err(napi_error)
}

/// A random-access resource: a local file, a directory, or a memory buffer.
#[napi(js_name = "IOBase")]
pub struct JsIOBase {
    inner: Holder,
}

impl JsIOBase {
    fn from_core(inner: Holder) -> Self {
        Self { inner }
    }

    /// Build a second handle on the same location.
    ///
    /// A handle owns backend state - a mapping, an open descriptor - so it is
    /// not copied; the location it describes is what gets rebuilt.
    fn rebuilt(&self) -> Result<Self> {
        let url = self
            .inner
            .url()
            .ok_or_else(|| napi_error("an in-memory resource has no location to rebuild from"))?;
        local_holder(url).map(Self::from_core)
    }

    fn holders(values: Vec<Holder>) -> Vec<Self> {
        values.into_iter().map(Self::from_core).collect()
    }

    /// Build a container handle for one recorded location.
    pub(crate) fn folder_at(location: &str) -> Result<Self> {
        let url = yggdryl::Url::from_str(location).map_err(napi_error)?;
        Holder::folder(url.to_path().map_err(napi_error)?)
            .map(Self::from_core)
            .map_err(napi_error)
    }
}

#[napi]
impl JsIOBase {
    /// Describe a location without touching it.
    ///
    /// Accepts anything that names one: a path or URL string, a native
    /// [`Url`][crate::uri::JsUrl], or another handle. Per the laziness
    /// contract, nothing is opened, created, or read here.
    #[napi(constructor)]
    pub fn new(value: LocationInput<'_>) -> Result<Self> {
        match value {
            Either3::A(handle) => handle.rebuilt(),
            Either3::B(url) => local_holder(&url.inner).map(Self::from_core),
            // The core already reads a Windows drive, a UNC share, and a
            // scheme-less path as a `file:` URL, so there is nothing to sniff
            // here.
            Either3::C(value) => local_holder(&yggdryl::Url::from_str(&value).map_err(napi_error)?)
                .map(Self::from_core),
        }
    }

    /// Infer a handle from a native handle, a `Url`, or a location string.
    #[napi(factory, js_name = "from")]
    pub fn from_js(value: LocationInput<'_>) -> Result<Self> {
        Self::new(value)
    }

    /// Describe an in-memory resource holding `data`.
    #[napi(factory)]
    pub fn from_bytes(data: Option<Uint8Array>) -> Self {
        let bytes = data.map(|data| data.to_vec()).unwrap_or_default();
        Self::from_core(Holder::Buffer(yggdryl::io::Buffer::from_bytes(bytes)))
    }

    /// The location this handle addresses.
    #[napi(getter)]
    pub fn url(&self) -> Option<JsUrl> {
        self.inner.url().cloned().map(JsUrl::from_core)
    }

    /// The final path component, as `path.basename`.
    #[napi(getter)]
    pub fn name(&self) -> String {
        self.inner
            .url()
            .and_then(|url| url.file_name())
            .unwrap_or_default()
            .to_owned()
    }

    /// The media type of the bytes here.
    #[napi(getter)]
    pub fn media_type(&self) -> crate::media::JsMediaType {
        crate::media::JsMediaType::from_core(self.inner.media_type().clone())
    }

    /// Declare what the bytes here are.
    ///
    /// A located resource infers this from its name, so this is what an
    /// in-memory one uses to say which record encoding it holds: the record
    /// methods read the encoding off the handle rather than taking a format.
    #[napi(setter)]
    pub fn set_media_type(&mut self, value: crate::media::MediaTypeInput<'_>) -> Result<()> {
        self.inner
            .set_media_type(crate::media::media_type_from_input(value)?);
        Ok(())
    }

    /// The number of bytes here, as `fs.Stats.size`.
    #[napi(getter)]
    pub fn size(&self) -> i64 {
        i64::try_from(self.inner.size()).unwrap_or(i64::MAX)
    }

    /// The containing resource, as `path.dirname`.
    #[napi(getter)]
    pub fn parent(&self) -> Option<JsIOBase> {
        self.inner.parent().map(Self::from_core)
    }

    /// Resolve a child of this resource, as `path.join`.
    #[napi]
    pub fn joinpath(&self, others: Vec<String>) -> Result<Self> {
        let mut resolved: Option<Holder> = None;
        for other in &others {
            let base = resolved.as_ref().unwrap_or(&self.inner);
            resolved = Some(base.child_by(other).map_err(napi_error)?);
        }
        match resolved {
            Some(handle) => Ok(Self::from_core(handle)),
            // `joinpath()` with nothing to join is the same location.
            None => self.rebuilt(),
        }
    }

    /// Return whether anything is here now, as `fs.existsSync`.
    #[napi]
    pub fn exists(&self) -> bool {
        self.inner.kind() != yggdryl::IOKind::Unknown
    }

    /// Return whether this resource contains others, as `Stats.isDirectory`.
    #[napi]
    pub fn is_dir(&self) -> bool {
        self.inner.is_container()
    }

    /// Return whether this resource holds bytes, as `Stats.isFile`.
    #[napi]
    pub fn is_file(&self) -> bool {
        self.inner.kind() == yggdryl::IOKind::File
    }

    /// Iterate the immediate children, as `fs.readdirSync`.
    ///
    /// Private entries - names beginning with a dot - are skipped unless
    /// `includePrivate` asks for them.
    #[napi]
    pub fn iterdir(&self, include_private: Option<bool>) -> Result<Vec<JsIOBase>> {
        self.inner
            .ls(false, include_private.unwrap_or(false))
            .map(Self::holders)
            .map_err(napi_error)
    }

    /// List the children, optionally descending, as the core `ls`.
    #[napi]
    pub fn ls(
        &self,
        recursive: Option<bool>,
        include_private: Option<bool>,
    ) -> Result<Vec<JsIOBase>> {
        self.inner
            .ls(recursive.unwrap_or(false), include_private.unwrap_or(false))
            .map(Self::holders)
            .map_err(napi_error)
    }

    /// Expand a glob against this resource, as `fs.globSync`.
    #[napi]
    pub fn glob(&self, pattern: String, include_private: Option<bool>) -> Result<Vec<JsIOBase>> {
        self.inner
            .glob(&pattern, include_private.unwrap_or(false))
            .map(Self::holders)
            .map_err(napi_error)
    }

    /// Expand a glob at any depth, so a pattern needs no leading `**/`.
    #[napi]
    pub fn rglob(&self, pattern: String, include_private: Option<bool>) -> Result<Vec<JsIOBase>> {
        self.glob(format!("**/{pattern}"), include_private)
    }

    /// The Hive partition pairs this resource's location spells out.
    #[napi(getter)]
    pub fn partitions(&self) -> Vec<PartitionEntry> {
        partition_entries(self.inner.partitions())
    }

    /// Iterate the leaves beneath this one carrying every given partition.
    ///
    /// `filters` is a mapping or a sequence of pairs, so a partitioned write
    /// selects the parts it has to touch and leaves the rest alone.
    #[napi]
    pub fn children_where(
        &self,
        filters: PartitionFilters,
        include_private: Option<bool>,
    ) -> Result<Vec<JsIOBase>> {
        let pairs: Vec<(String, String)> = match filters {
            Either::A(entries) => entries
                .into_iter()
                .map(|entry| (entry.column, entry.value))
                .collect(),
            Either::B(values) => values.into_iter().collect(),
        };
        let borrowed: Vec<(&str, &str)> = pairs
            .iter()
            .map(|(column, value)| (column.as_str(), value.as_str()))
            .collect();
        Ok(self
            .inner
            .children_where(&borrowed, include_private.unwrap_or(false))
            .map_err(napi_error)?
            .map(Self::from_core)
            .collect())
    }

    /// Read every byte here, as `fs.readFileSync`.
    ///
    /// A resource that does not exist reads as empty rather than throwing, per
    /// the laziness contract.
    #[napi]
    pub fn read_bytes(&self) -> Result<Buffer> {
        self.inner.read_all().map(Buffer::from).map_err(napi_error)
    }

    /// Read every byte here as UTF-8 text.
    #[napi]
    pub fn read_text(&self) -> Result<String> {
        let bytes = self.inner.read_all().map_err(napi_error)?;
        String::from_utf8(bytes).map_err(napi_error)
    }

    /// Replace what is here with `data`, as `fs.writeFileSync`.
    #[napi]
    pub fn write_bytes(&mut self, data: Uint8Array) -> Result<u32> {
        self.inner.write_all_bytes(&data).map_err(napi_error)?;
        u32::try_from(data.len()).map_err(napi_error)
    }

    /// Replace what is here with `text`, encoded as UTF-8.
    #[napi]
    pub fn write_text(&mut self, text: String) -> Result<u32> {
        self.inner
            .write_all_bytes(text.as_bytes())
            .map_err(napi_error)?;
        u32::try_from(text.len()).map_err(napi_error)
    }

    /// Read `length` bytes from `offset`, which a path cannot do.
    #[napi]
    pub fn pread(&self, offset: f64, length: u32) -> Result<Buffer> {
        self.inner
            .read_range(exact_u64(offset, "offset")?, length as usize)
            .map(Buffer::from)
            .map_err(napi_error)
    }

    /// Write `data` at `offset`, growing and zero-filling as needed.
    #[napi]
    pub fn pwrite(&mut self, offset: f64, data: Uint8Array) -> Result<u32> {
        let written = self
            .inner
            .pwrite(exact_u64(offset, "offset")?, &data)
            .map_err(napi_error)?;
        u32::try_from(written).map_err(napi_error)
    }

    /// Append `data` after the last byte, returning the offset it landed at.
    #[napi]
    pub fn append(&mut self, data: Uint8Array) -> Result<i64> {
        let offset = self.inner.append(&data).map_err(napi_error)?;
        Ok(i64::try_from(offset).unwrap_or(i64::MAX))
    }

    /// Create this resource as a container, as `fs.mkdirSync`.
    ///
    /// Parents are created too, and an existing container is left alone, which
    /// is `mkdir` with `recursive: true`. An undecided location is what this
    /// decides: it becomes a container, and the handle keeps working as one
    /// afterwards - a plain byte write would have made it a file instead.
    #[napi]
    pub fn mkdir(&mut self) -> Result<()> {
        let url = self
            .inner
            .url()
            .ok_or_else(|| napi_error("an in-memory resource cannot become a directory"))?;
        let mut folder = Holder::folder(url.to_path().map_err(napi_error)?).map_err(napi_error)?;
        folder.truncate(0).map_err(napi_error)?;
        self.inner = folder;
        Ok(())
    }

    /// Create this resource as an empty leaf, as `fs.closeSync(fs.openSync)`.
    ///
    /// An existing leaf keeps its bytes, as `touch` does.
    #[napi]
    pub fn touch(&mut self) -> Result<()> {
        if self.inner.is_container() {
            return Err(napi_error(format!(
                "expected a file to touch, got the directory {}",
                self.name()
            )));
        }
        if self.exists() {
            return Ok(());
        }
        self.inner.write_all_bytes(b"").map_err(napi_error)
    }

    /// Remove the bytes here, as `fs.unlinkSync` on a leaf.
    #[napi]
    pub fn unlink(&mut self) -> Result<()> {
        self.inner.clear().map_err(napi_error)
    }

    /// Cut this resource to `size` bytes, as `fs.truncateSync`.
    #[napi]
    pub fn truncate(&mut self, size: f64) -> Result<()> {
        self.inner
            .truncate(exact_u64(size, "size")?)
            .map_err(napi_error)
    }

    /// Flush anything buffered.
    #[napi]
    pub fn flush(&mut self) -> Result<()> {
        self.inner.flush().map_err(napi_error)
    }

    /// Copy every byte here into `target`, returning the count.
    #[napi]
    pub fn copy_into(&self, target: &mut JsIOBase) -> Result<i64> {
        let copied = self
            .inner
            .copy_into(&mut target.inner)
            .map_err(napi_error)?;
        Ok(i64::try_from(copied).unwrap_or(i64::MAX))
    }

    /// Iterate the resource's decoded text lines, one line at a time.
    ///
    /// Any content codings the resource's name declares - `trades.jsonl.gz`,
    /// `log.txt.zst` - decode as streams, so a compressed resource is read
    /// without ever holding its decompressed value. A line is what `\n` ends,
    /// A positioned view over this resource.
    ///
    /// The cursor shares this handle - a write through the cursor is a write
    /// here - and owns only its position: `read`/`write` advance it, `seek`
    /// and `tell` move and report it, and two cursors advance independently.
    #[napi]
    // The position crosses as a JavaScript number, exact to 2^53 - the same
    // contract `size` already publishes.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn cursor(&self, reference: Reference<JsIOBase>, position: Option<f64>) -> JsIOCursor {
        JsIOCursor {
            handle: reference,
            position: position.map_or(0, |position| position.max(0.0) as u64),
        }
    }

    /// a trailing `\r` belongs to the terminator, and the last line needs no
    /// terminator. The returned iterator owns a rebuilt handle, so it stays
    /// valid however long the caller keeps it.
    /// With `pattern`, lines group into records: one starts at a matching
    /// line and carries every following line until the next match.
    #[napi]
    pub fn read_lines(&self, pattern: Option<String>) -> Result<JsLineIterator> {
        let handle = self.rebuilt()?;
        let inner: Box<dyn Iterator<Item = yggdryl::Result<String>> + Send> = match pattern {
            Some(pattern) => Box::new(
                handle
                    .inner
                    .into_read_lines_matching(&pattern)
                    .map_err(napi_error)?,
            ),
            None => Box::new(handle.inner.into_read_lines().map_err(napi_error)?),
        };
        Ok(JsLineIterator { inner })
    }

    /// Return the record settings this handle's media type names.
    ///
    /// The encoding is never guessed: it is whatever the handle already says it
    /// holds, which is why no record method below takes a format argument.
    #[napi]
    pub fn record_options(&self) -> Result<JsRecordOptions> {
        self.inner
            .record_options()
            .map(JsRecordOptions::from_core)
            .map_err(napi_error)
    }

    /// Read the canonical non-null struct root `Field` of this resource.
    #[napi]
    pub fn read_arrow_field(&self, options: Option<&JsRecordOptions>) -> Result<JsField> {
        let options = JsRecordOptions::resolved(options, &self.inner)?;
        self.inner
            .read_arrow_field(&options)
            .map(JsField::from_core)
            .map_err(napi_error)
    }

    /// Read this resource's rows as one `BatchReader`.
    ///
    /// A schema on the options selects and casts during the read: the columns it
    /// names that this resource stores become the encoding's own projection, so
    /// the rest are skipped rather than read and discarded, and what comes back
    /// is the shape it declares. A handle addressing a folder reads across the
    /// partitions beneath it, restoring the columns their directory names spell
    /// out.
    #[napi]
    pub fn read_arrow_batch_reader(
        &self,
        options: Option<&JsRecordOptions>,
    ) -> Result<JsBatchReader> {
        let options = JsRecordOptions::resolved(options, &self.inner)?;
        let reader = self
            .inner
            .read_arrow_batch_reader(&options)
            .map_err(napi_error)?;
        Ok(JsBatchReader::from_core(reader, options.root_name()))
    }

    /// Replace or merge this resource's rows with every batch `batches` yields.
    ///
    /// An empty `mergeByNames` overwrites. A non-empty one names the columns a row is
    /// matched on, so a row whose key is already stored updates it and a row
    /// whose key is not appends. Nothing reaches the handle until the last batch
    /// is encoded, so a failure leaves the resource exactly as it was.
    #[napi]
    pub fn write_arrow_batch_reader(
        &mut self,
        batches: &mut JsBatchReader,
        options: Option<&JsRecordOptions>,
    ) -> Result<()> {
        let options = JsRecordOptions::resolved(options, &self.inner)?;
        self.inner
            .write_arrow_batch_reader(batches.take()?, &options)
            .map_err(napi_error)
    }

    /// Add every batch `batches` yields after the rows this resource holds.
    ///
    /// Both sides stream: what is stored is chained ahead of what arrives, and
    /// incoming batches are cast to the target shape as they are pulled.
    #[napi]
    pub fn append_arrow_batch_reader(
        &mut self,
        batches: &mut JsBatchReader,
        options: Option<&JsRecordOptions>,
    ) -> Result<()> {
        let options = JsRecordOptions::resolved(options, &self.inner)?;
        self.inner
            .append_arrow_batch_reader(batches.take()?, &options)
            .map_err(napi_error)
    }

    /// Decode this location as a host-independent forward-slash path.
    #[napi]
    pub fn to_path(&self) -> Result<String> {
        let url = self
            .inner
            .url()
            .ok_or_else(|| napi_error("this resource has no file system path"))?;
        url.to_path()
            .map_err(napi_error)?
            .into_os_string()
            .into_string()
            .map_err(|_| napi_error("file URI path cannot be represented as a JavaScript string"))
    }

    /// Return the location as text, so a handle prints where it points.
    #[napi]
    pub fn to_string(&self) -> String {
        self.inner
            .url()
            .map_or_else(|| "<memory>".to_owned(), ToString::to_string)
    }
}

/// Iterator over a resource's decoded text lines, one line at a time.
///
/// Built by [`JsIOBase::read_lines`]. The handle is rebuilt from its location
/// and owned here, bytes stream through a fixed buffer, and any content
/// codings the name declares decode as streams, so a compressed resource is
/// read without ever holding its decompressed value. `next()` is the native
/// half of the iteration protocol; the loader wraps it so `for...of` yields
/// strings.
#[napi(js_name = "LineIterator")]
pub struct JsLineIterator {
    inner: Box<dyn Iterator<Item = yggdryl::Result<String>> + Send>,
}

#[napi]
impl JsLineIterator {
    /// The next line, or `null` when the resource is exhausted.
    #[napi]
    pub fn next(&mut self) -> Result<Option<String>> {
        self.inner.next().transpose().map_err(napi_error)
    }
}

/// A positioned view over one handle, sharing the handle's bytes.
///
/// Reads and writes advance the position; `seek`/`tell` move and report it;
/// two cursors over one handle advance independently, exactly as two `pread`
/// callers do.
#[napi(js_name = "IOCursor")]
pub struct JsIOCursor {
    handle: Reference<JsIOBase>,
    position: u64,
}

// Positions cross as JavaScript numbers, exact to 2^53 - the same contract
// `size` already publishes - so the casts here are the boundary, not a loss.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
#[napi]
impl JsIOCursor {
    /// The current position, in bytes from the start.
    #[napi(getter)]
    pub fn position(&self) -> f64 {
        self.position as f64
    }

    /// Set the absolute position; past the end is allowed.
    #[napi(setter)]
    pub fn set_position(&mut self, position: f64) {
        self.position = position.max(0.0) as u64;
    }

    /// The current position, as `tell` spells it everywhere else.
    #[napi]
    pub fn tell(&self) -> f64 {
        self.position as f64
    }

    /// Set the absolute position, returning it.
    #[napi]
    pub fn seek(&mut self, position: f64) -> f64 {
        self.set_position(position);
        self.position as f64
    }

    /// Read from the position, advancing it; omit `length` to read to the end.
    #[napi]
    pub fn read(&mut self, length: Option<f64>) -> Result<Buffer> {
        let size = self.handle.inner.size();
        let remaining = size.saturating_sub(self.position);
        let wanted = length.map_or(remaining, |length| remaining.min(length.max(0.0) as u64));
        let mut buffer = vec![0_u8; wanted as usize];
        let read = self
            .handle
            .inner
            .pread(self.position, &mut buffer)
            .map_err(napi_error)?;
        buffer.truncate(read);
        self.position += read as u64;
        Ok(Buffer::from(buffer))
    }

    /// Write at the position, advancing it, returning the bytes written.
    #[napi]
    pub fn write(&mut self, data: Uint8Array) -> Result<f64> {
        let written = self
            .handle
            .inner
            .pwrite(self.position, data.as_ref())
            .map_err(napi_error)?;
        self.position += written as u64;
        Ok(written as f64)
    }

    /// Flush the handle the cursor writes through.
    #[napi]
    pub fn flush(&mut self) -> Result<()> {
        self.handle.inner.flush().map_err(napi_error)
    }
}
