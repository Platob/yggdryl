//! `IOBase`, exposed to JavaScript with the method names `fs` and `path` use.
//!
//! The core trait is positional and fully random-access, so there are no flags
//! to open with and no descriptor to keep: `readBytes`, `writeBytes`, `ls`,
//! `glob`, `mkdir`, and `unlink` all mean here what they mean on a path in
//! `node:fs`, and each one is answered by the core implementation for the
//! backend the location names. Code written against a local directory therefore
//! runs against a bucket when that backend lands, because only the handle
//! changes.

use std::time::Duration;

use napi::bindgen_prelude::{
    Buffer, ClassInstance, Either, Either3, Either4, Env, Function, Reference, Result, Uint8Array,
};
use napi_derive::napi;

use yggdryl::IOMode;
use yggdryl::buffered::BufferedOptions;
use yggdryl::generic::{Holder, IORecordOptions as _};
use yggdryl::io::{IOBase as _, IOMedia as _};
use yggdryl::text::TextLineOptions;

use crate::arrow::JsBatchReader;
use crate::arrowfs::{ArrowFileSystemInput, JsArrowFileSystem};
use crate::codec::{
    DEFAULT_JS_DEPTH, JsScalar, decoded_value_for_field, value_to_transport_for_field,
};
use crate::field::JsField;
use crate::generic::JsRecordOptions;
use crate::uri::{JsUrl, PartitionEntry, partition_entries};
use crate::{exact_u64, napi_error};

/// Bytes accumulated before a record write is flushed to the resource.
const LINE_WRITE_CHUNK: usize = 64 * 1024;
/// Default byte-stream window, shared with the Rust core.
const BYTE_STREAM_BATCH_SIZE: usize = yggdryl::io::DEFAULT_STREAM_BATCH_SIZE;
/// Largest integer a JavaScript `number` represents exactly.
const JS_MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

/// Saturate a native count at JavaScript's exact-integer boundary.
fn safe_js_count(value: u64) -> i64 {
    i64::try_from(value.min(JS_MAX_SAFE_INTEGER)).unwrap_or(i64::MAX)
}

/// A native handle, a native `Url`, or anything that names a location.
pub(crate) type LocationInput<'a> =
    Either3<ClassInstance<'a, JsIOBase>, ClassInstance<'a, JsUrl>, String>;

/// What the constructor takes first: a location, or the file system one of
/// its locations sits on.
pub(crate) type LocationOrFileSystemInput<'a> = Either4<
    ClassInstance<'a, JsIOBase>,
    ClassInstance<'a, JsUrl>,
    String,
    ArrowFileSystemInput<'a>,
>;

/// A mapping of partition columns to values, or the same pairs as entries.
type PartitionFilters = Either<Vec<PartitionEntry>, std::collections::HashMap<String, String>>;

/// Build a local handle for the location a `Url` names.
fn local_holder(url: &yggdryl::Url) -> Result<Holder> {
    Holder::local(url.clone().into_path().map_err(napi_error)?).map_err(napi_error)
}

/// Rebuild a foreign-file-system handle, keeping the file system it stands on.
///
/// `None` for anything else, so the local rebuild stays the default path.
fn rebuilt_arrow_holder(inner: &Holder) -> Option<Holder> {
    match inner {
        Holder::ArrowFolder(folder) => Some(Holder::ArrowFolder(folder.clone())),
        Holder::ArrowFile(file) => Some(Holder::ArrowFile(yggdryl::arrowfs::File::new(
            file.filesystem().clone(),
            file.url().clone(),
        ))),
        Holder::ArrowPath(path) => Some(Holder::ArrowPath(yggdryl::arrowfs::Path::new(
            path.filesystem().clone(),
            path.url().clone(),
        ))),
        _ => None,
    }
}

/// Address a foreign-file-system handle's location as a container.
pub(crate) fn arrow_folder_holder(inner: &Holder) -> Option<Holder> {
    let folder = match inner {
        Holder::ArrowFolder(folder) => folder.clone(),
        Holder::ArrowFile(file) => {
            yggdryl::arrowfs::Folder::new(file.filesystem().clone(), file.url().clone())
        }
        Holder::ArrowPath(path) => {
            yggdryl::arrowfs::Folder::new(path.filesystem().clone(), path.url().clone())
        }
        _ => return None,
    };
    Some(Holder::ArrowFolder(folder))
}

/// Build a container handle for the location `value` names.
///
/// A folder is asked for by name rather than discovered, because a location
/// that holds nothing yet reads as a leaf: `Holder::local` cannot tell a
/// directory that does not exist from a file that does not exist, and a table
/// root has to be the former before anything is written into it. A handle on
/// a foreign Arrow file system becomes a container on that same file system,
/// so a table reached this way never learns which backend it stands on.
pub(crate) fn folder_from_input(value: LocationInput<'_>) -> Result<Holder> {
    let url = match value {
        Either3::A(handle) => {
            if let Some(holder) = arrow_folder_holder(&handle.inner) {
                return Ok(holder);
            }
            handle
                .inner
                .url()
                .cloned()
                .ok_or_else(|| napi_error("an in-memory resource cannot contain a table"))?
        }
        Either3::B(url) => url.inner.clone(),
        Either3::C(value) => yggdryl::Url::from_str(&value).map_err(napi_error)?,
    };
    Holder::folder(url.into_path().map_err(napi_error)?).map_err(napi_error)
}

/// A random-access resource: a local file, a directory, or a memory buffer.
#[napi(js_name = "IOBase")]
pub struct JsIOBase {
    inner: Holder,
}

/// Opaque state for one asynchronous, cadence-bounded record write.
///
/// The JavaScript loader removes this class from the public exports. Only the
/// private `IOBase` bridges below can construct or advance it.
#[napi(js_name = "ArrowWriteSession")]
pub struct JsArrowWriteSession {
    inner: yggdryl::io::ArrowWriteSession,
}

impl JsIOBase {
    pub(crate) fn from_core(inner: Holder) -> Self {
        Self { inner }
    }

    /// Build a second handle on the same location.
    ///
    /// A handle owns backend state - a mapping, an open descriptor, a staged
    /// value - so it is not copied; the location it describes is what gets
    /// rebuilt. A handle on a foreign Arrow file system rebuilds onto that
    /// same file system, because its location alone would not say where it
    /// lives.
    fn rebuilt(&self) -> Result<Self> {
        if let Some(holder) = rebuilt_arrow_holder(&self.inner) {
            return Ok(Self::from_core(holder));
        }
        let url = self
            .inner
            .url()
            .ok_or_else(|| napi_error("an in-memory resource has no location to rebuild from"))?;
        local_holder(url).map(Self::from_core)
    }

    /// Build a handle on `path` over a held JavaScript file system handler.
    fn over_arrow_fs(env: Env, filesystem: &ArrowFileSystemInput<'_>, path: &str) -> Result<Self> {
        let backend: std::sync::Arc<dyn yggdryl::arrowfs::ArrowFileSystem> =
            std::sync::Arc::new(JsArrowFileSystem::new(env, filesystem)?);
        let url = yggdryl::arrowfs::location_url(backend.as_ref(), path).map_err(napi_error)?;
        Ok(Self::from_core(yggdryl::arrowfs::located(backend, url)))
    }

    /// Build a container handle for one recorded location.
    pub(crate) fn folder_at(location: &str) -> Result<Self> {
        let url = yggdryl::Url::from_str(location).map_err(napi_error)?;
        Holder::folder(url.into_path().map_err(napi_error)?)
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
    ///
    /// An Arrow file system handler as the first argument names the *backend*
    /// rather than the location, so the second says where on it:
    /// `new IOBase(handler, 'bucket/key.parquet')`. What comes back is this
    /// same class - nothing file-system-specific leaks into the surface.
    #[napi(constructor)]
    pub fn new(
        env: Env,
        value: LocationOrFileSystemInput<'_>,
        path: Option<String>,
    ) -> Result<Self> {
        let value = match value {
            Either4::A(handle) => Either3::A(handle),
            Either4::B(url) => Either3::B(url),
            Either4::C(value) => Either3::C(value),
            Either4::D(filesystem) => {
                let path = path.ok_or_else(|| {
                    napi_error(
                        "expected a path on the file system as the second argument, got none",
                    )
                })?;
                return Self::over_arrow_fs(env, &filesystem, &path);
            }
        };
        if let Some(path) = path {
            return Err(napi_error(format!(
                "expected an Arrow file system handler to resolve {path:?} against, got a location"
            )));
        }
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
        match value {
            Either3::A(handle) => handle.rebuilt(),
            Either3::B(url) => local_holder(&url.inner).map(Self::from_core),
            Either3::C(value) => local_holder(&yggdryl::Url::from_str(&value).map_err(napi_error)?)
                .map(Self::from_core),
        }
    }

    /// Describe a resource on any Arrow file system a caller supplies.
    ///
    /// This is the explicit spelling of what the constructor infers, and it is
    /// the whole surface a foreign file system needs. Arrow JS ships none, so
    /// `filesystem` is the vtable `pyarrow.fs` implements, written as a plain
    /// object in camelCase: `typeName`, `fileInfo`, `list`, `readRange`,
    /// `writeFull`, `createDir`, `deleteFile`. A `Map`, `node:fs`, an S3
    /// client, or a caching layer over one reaches those same six calls, so
    /// none of them needs code of its own here.
    ///
    /// ```js
    /// const handle = IOBase.fromArrowFs(handler, 'bucket/key.parquet')
    /// const rows = handle.readArrowReader().intoTable()
    /// ```
    ///
    /// The result is an ordinary handle: `ls`, `glob`, `joinpath`, and
    /// `parent` return handles that still carry the file system, and the three
    /// record methods work exactly as they do on a local file. Per the
    /// laziness contract nothing is opened, created, or read here.
    ///
    /// A write publishes when the handle is flushed, because an Arrow file
    /// system replaces whole files rather than writing ranges - so a file
    /// another reader will open is flushed before it is handed over.
    ///
    /// The handler is called synchronously, on the JavaScript thread that
    /// supplied it and no other: a handle built here cannot be read from a
    /// `Worker`, because a JavaScript value belongs to one isolate and this
    /// boundary refuses rather than pretending otherwise.
    #[napi(factory)]
    pub fn from_arrow_fs(
        env: Env,
        filesystem: ArrowFileSystemInput<'_>,
        path: String,
    ) -> Result<Self> {
        Self::over_arrow_fs(env, &filesystem, &path)
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
        safe_js_count(self.inner.size())
    }

    /// The exact core storage role: memory, file, directory, table,
    /// namespace, catalog, or unknown.
    #[napi(getter)]
    pub fn kind(&self) -> String {
        self.inner.kind().as_str().to_owned()
    }

    /// The number of logical rows in this media value.
    ///
    /// This is metadata, not a materialized read. The core uses an encoding's
    /// cheap count when it has one and caches successful answers while the
    /// handle is open. Values beyond JavaScript's exact integer range saturate
    /// at `Number.MAX_SAFE_INTEGER`, as [`Self::size`] does.
    #[napi(getter)]
    pub fn row_size(&self) -> Result<i64> {
        self.inner.row_size().map(safe_js_count).map_err(napi_error)
    }

    /// The number of columns in this media value's logical root field.
    ///
    /// The core answers from schema metadata and caches successful answers
    /// while the handle is open; no JavaScript-side schema or count is kept.
    #[napi(getter)]
    pub fn column_size(&self) -> Result<i64> {
        self.inner
            .column_size()
            .map(|columns| safe_js_count(u64::try_from(columns).unwrap_or(u64::MAX)))
            .map_err(napi_error)
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
            resolved = Some(base.child_by_path(other).map_err(napi_error)?);
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

    /// Return whether this handle exposes either the byte or record surface.
    ///
    /// Media type answers first; only an undecided directory may need to ask
    /// its first leaf. Containers holding neither bytes nor rows return false.
    #[napi]
    pub fn is_io(&self) -> bool {
        self.inner.is_io()
    }

    /// Return whether this resource is one whole byte value.
    ///
    /// The byte surface - `readBytes` and `writeBytes` - is for an atomic
    /// resource; `isTabular` names the record surface instead. A container
    /// holding neither answers `false` to both.
    #[napi]
    pub fn is_atomic(&self) -> bool {
        self.inner.is_atomic()
    }

    /// Return whether this resource holds rows and columns.
    ///
    /// The record surface - `readArrowReader` and the three explicit write
    /// intents - is for a tabular resource: a leaf whose media type names a
    /// record encoding, a folder that reads as the table beneath it, or a
    /// table format's own folder.
    #[napi]
    pub fn is_tabular(&self) -> bool {
        self.inner.is_tabular()
    }

    /// Iterate the immediate children, as `fs.readdirSync`.
    ///
    /// The listing is lazy and iterable: `for...of` walks it, and taking three
    /// entries from a folder of a hundred thousand costs three. Private
    /// entries - names beginning with a dot - are skipped unless
    /// `includePrivate` asks for them.
    #[napi]
    pub fn iterdir(&self, include_private: Option<bool>) -> JsListing {
        JsListing {
            inner: self.inner.ls(false, include_private.unwrap_or(false)),
        }
    }

    /// List the children, optionally descending, as the core `ls`.
    ///
    /// Lazy and iterable: see `iterdir`.
    #[napi]
    pub fn ls(&self, recursive: Option<bool>, include_private: Option<bool>) -> JsListing {
        JsListing {
            inner: self
                .inner
                .ls(recursive.unwrap_or(false), include_private.unwrap_or(false)),
        }
    }

    /// Expand a glob against this resource, as `fs.globSync`.
    ///
    /// Lazy and iterable: a pattern whose fixed prefix names nothing touches
    /// nothing beneath it, because the walk only starts on the first `next`.
    #[napi]
    pub fn glob(&self, pattern: String, include_private: Option<bool>) -> Result<JsListing> {
        Ok(JsListing {
            inner: self
                .inner
                .glob(&pattern, include_private.unwrap_or(false))
                .map_err(napi_error)?,
        })
    }

    /// Expand a glob at any depth, so a pattern needs no leading `**/`.
    #[napi]
    pub fn rglob(&self, pattern: String, include_private: Option<bool>) -> Result<JsListing> {
        self.glob(format!("**/{pattern}"), include_private)
    }

    /// The Hive partition pairs this resource's location spells out.
    #[napi(getter)]
    pub fn partitions(&self) -> Vec<PartitionEntry> {
        partition_entries(self.inner.partitions())
    }

    /// Iterate the entries beneath this one a predicate does not rule out.
    ///
    /// The predicate is asked of the holder, not of the rows: `&holder.name`,
    /// `&holder.partition['year']`, `&holder.size`. A conjunct that reads a
    /// row column cannot be answered by a listing, so it is dropped rather
    /// than guessed at - this may keep a file the rows later discard and can
    /// never discard one they would have kept.
    ///
    /// `filter` is an `Expression` or the text of one, which parses.
    #[napi]
    pub fn children_matching(
        &self,
        filter: napi::bindgen_prelude::Either<
            napi::bindgen_prelude::ClassInstance<'_, crate::expression::JsExpression>,
            String,
        >,
        include_private: Option<bool>,
    ) -> Result<JsListing> {
        let filter = crate::expression::expression_from_input(filter)?;
        Ok(JsListing {
            inner: self
                .inner
                .children_matching(&filter, include_private.unwrap_or(false))
                .map_err(napi_error)?,
        })
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
    ) -> Result<JsListing> {
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
        Ok(JsListing {
            inner: self
                .inner
                .children_where(&borrowed, include_private.unwrap_or(false))
                .map_err(napi_error)?,
        })
    }

    /// Read every byte here, as `fs.readFileSync`.
    ///
    /// A resource that does not exist reads as empty rather than throwing, per
    /// the laziness contract.
    #[napi]
    pub fn read_bytes(&self) -> Result<Buffer> {
        self.inner
            .read_all_bytes()
            .map(Buffer::from)
            .map_err(napi_error)
    }

    /// Read every byte here as UTF-8 text.
    #[napi]
    pub fn read_text(&self) -> Result<String> {
        let bytes = self.inner.read_all_bytes().map_err(napi_error)?;
        String::from_utf8(bytes).map_err(napi_error)
    }

    /// Decode one structured value through the native inferred text codec.
    #[napi(js_name = "_readScalarNative", skip_typescript)]
    pub fn read_scalar_native(
        &self,
        field: Option<ClassInstance<'_, JsField>>,
        native_scalar: Option<bool>,
    ) -> Result<Either<JsScalar, serde_json::Value>> {
        let value = self
            .inner
            .read_value(field.as_ref().map(|field| &field.inner))
            .map_err(napi_error)?;
        decoded_value_for_field(
            value,
            field.as_ref().map(|field| &field.inner),
            DEFAULT_JS_DEPTH,
            native_scalar.unwrap_or(false),
        )
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

    /// Encode one native structured value through the inferred text codec.
    #[napi(js_name = "_writeScalarNative", skip_typescript)]
    pub fn write_scalar_native(&mut self, value: &JsScalar) -> Result<()> {
        self.inner.write_value(&value.inner).map_err(napi_error)
    }

    /// Read `length` bytes from `offset`, which a path cannot do.
    #[napi]
    pub fn pread(&self, offset: f64, length: u32) -> Result<Buffer> {
        self.inner
            .read_range(exact_u64(offset, "offset")?, length as usize)
            .map(Buffer::from)
            .map_err(napi_error)
    }

    /// Stream bounded byte arrays from an explicit position.
    ///
    /// Construction performs no read. Each `next()` asks the Rust core for
    /// exactly one bounded chunk, and the iterator keeps this native handle
    /// alive for as long as JavaScript keeps the stream. `position` defaults
    /// to zero and `batchSize` to 64 KiB.
    #[napi]
    pub fn pstream_bytes(
        &self,
        reference: Reference<JsIOBase>,
        position: Option<f64>,
        batch_size: Option<f64>,
    ) -> Result<JsByteIterator> {
        Ok(JsByteIterator {
            source: ByteIteratorSource::Handle {
                handle: reference,
                position: position.map_or(Ok(0), |value| exact_u64(value, "position"))?,
            },
            batch_size: byte_stream_batch_size(batch_size)?,
            done: false,
        })
    }

    /// Replace this handle with one page-cached view of the same resource.
    ///
    /// The JavaScript wrapper returns `this` for chaining. Repeating the call
    /// reconfigures the existing cache around its held resource rather than
    /// stacking another cache layer.
    #[napi(js_name = "_bufferedNative", skip_typescript)]
    pub fn buffered_native(
        &mut self,
        page_size: Option<f64>,
        max_bytes: Option<f64>,
        ttl_ms: Option<f64>,
    ) -> Result<()> {
        let mut options = BufferedOptions::default();
        if let Some(page_size) = page_size {
            let page_size = exact_u64(page_size, "pageSize")?;
            let page_size = usize::try_from(page_size).map_err(|_| {
                napi_error(format!(
                    "pageSize {page_size} exceeds this platform's byte-count range"
                ))
            })?;
            options = options.with_page_size(page_size);
        }
        if let Some(max_bytes) = max_bytes {
            options = options.with_max_bytes(exact_u64(max_bytes, "maxBytes")?);
        }
        if let Some(ttl_ms) = ttl_ms {
            options = options.with_ttl(Duration::from_millis(exact_u64(ttl_ms, "ttlMs")?));
        }

        // Every fallible conversion happened above. The temporary is replaced
        // immediately and can therefore never become observable to JavaScript.
        let held = std::mem::replace(&mut self.inner, Holder::buffer(yggdryl::io::Buffer::new()));
        self.inner = held.buffered(options);
        Ok(())
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
        // A handle on a foreign file system becomes a container on that file
        // system. Rebuilding from the location alone would silently move the
        // handle to the local disk, because a location does not say which
        // backend it belongs to.
        let mut folder = if let Some(holder) = arrow_folder_holder(&self.inner) {
            holder
        } else {
            let url = self
                .inner
                .url()
                .ok_or_else(|| napi_error("an in-memory resource cannot become a directory"))?;
            Holder::folder(url.clone().into_path().map_err(napi_error)?).map_err(napi_error)?
        };
        folder.truncate(0).map_err(napi_error)?;
        self.inner = folder;
        Ok(())
    }

    /// Create this resource as an empty leaf, as `fs.closeSync(fs.openSync)`.
    ///
    /// An existing leaf keeps its bytes, as `touch` does.
    #[napi]
    pub fn touch(&mut self) -> Result<()> {
        // An empty positional write is the non-truncating act: it creates a
        // missing leaf, preserves an existing value, and lets a directory
        // reject the write itself. No existence or kind probe races the act.
        self.inner.pwrite(0, b"").map_err(napi_error)?;
        self.inner.flush().map_err(napi_error)
    }

    /// Delete the resource here, as `fs.unlinkSync` on a leaf.
    ///
    /// A thin spelling of `remove(false)`; unlike `fs.unlinkSync`, a resource
    /// that is not there is not an error, because absence is a no-op success
    /// everywhere on this handle.
    #[napi]
    pub fn unlink(&mut self) -> Result<()> {
        self.inner.remove(false).map_err(napi_error)
    }

    /// Empty the contents, keeping the resource.
    ///
    /// A leaf keeps existing with size 0; a directory keeps existing and is
    /// emptied of every child, recursively; a resource that is not there is
    /// left alone. Nothing is created.
    #[napi]
    pub fn clear(&mut self) -> Result<()> {
        self.inner.clear().map_err(napi_error)
    }

    /// Delete the resource completely.
    ///
    /// After this returns nothing of what the handle addressed remains - the
    /// bytes, the tree below a directory, and any cached schema or footer. A
    /// leaf ignores `recursive`. A directory needs `recursive` to delete
    /// anything below it; without it, one that still has children throws
    /// rather than silently succeeding or silently recursing. A resource that
    /// is not there succeeds, having done nothing.
    #[napi]
    pub fn remove(&mut self, recursive: Option<bool>) -> Result<()> {
        self.inner
            .remove(recursive.unwrap_or(false))
            .map_err(napi_error)
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

    /// Materialize the resource and cache what repeated calls would re-derive.
    ///
    /// A handle works without this - every operation materializes what it
    /// needs - so calling it moves that cost to a known point. Opening a
    /// resource that does not exist yet succeeds without creating it. The
    /// loader binds `using` to this and to [`Self::close`], so a scope is
    /// what publishes a written file.
    #[napi]
    pub fn open(&mut self) -> Result<()> {
        self.inner.open().map_err(napi_error)
    }

    /// Return whether cached state is currently held.
    #[napi]
    pub fn opened(&self) -> bool {
        self.inner.opened()
    }

    /// Return whether no cached state is currently held.
    #[napi]
    pub fn closed(&self) -> bool {
        self.inner.closed()
    }

    /// Publish and release everything [`Self::open`] cached.
    ///
    /// The handle stays usable afterwards; a later operation re-materializes.
    /// This is what publishes a written file at its exact length, and on a
    /// backend that replaces whole files - any Arrow file system - it is what
    /// hands the staged value over, so a file another reader will open is
    /// written inside a scope.
    #[napi]
    pub fn close(&mut self) -> Result<()> {
        self.inner.close().map_err(napi_error)
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

    /// The content coding this resource's name declares, or `null` for none.
    ///
    /// A located resource reads its coding off its own compound name -
    /// `trades.json.gz` is `"gzip"` - and an in-memory one off the media type
    /// it was told to hold. Bytes that carry no coding answer `null` rather
    /// than `"identity"`, because the question a caller asks here is whether
    /// there is anything to undo.
    #[napi(getter)]
    pub fn codec(&self) -> Option<String> {
        let codec = self.inner.codec();
        (!codec.is_identity()).then(|| codec.as_str().to_owned())
    }

    /// Encode every byte here into `target`, returning the bytes written.
    ///
    /// `codec` defaults to the coding `target`'s own name declares, so writing
    /// into `trades.json.gz` gzips without anyone naming gzip twice; passing one
    /// explicitly is how an in-memory target, which has no name to declare
    /// anything, picks a coding. A target that declares none is refused rather
    /// than silently copied, because a coding nobody named is a coding nobody
    /// can decode by name later. `level` is the shared 0-9 scale the
    /// whole-buffer codecs use. The target's media type records the added
    /// coding, so [`decompressInto`](Self::decompress_into) later needs no
    /// argument.
    #[napi]
    pub fn compress_into(
        &self,
        target: &mut JsIOBase,
        codec: Option<String>,
        level: Option<u8>,
    ) -> Result<i64> {
        let codec = match codec {
            Some(name) => yggdryl::Codec::from_str(&name).map_err(napi_error)?,
            None => match target.inner.codec() {
                found if found.is_identity() => {
                    return Err(napi::Error::from_reason(format!(
                        "expected a target declaring a content coding, got {}; pass a codec to \
                         say which coding to write",
                        target.inner.media_type(),
                    )));
                }
                found => found,
            },
        };
        let level = level.map_or(yggdryl::Level::DEFAULT, yggdryl::Level::new);
        let written = self
            .inner
            .compress_into_with_level(&mut target.inner, codec, level)
            .map_err(napi_error)?;
        Ok(i64::try_from(written).unwrap_or(i64::MAX))
    }

    /// Decode every byte here into `target`, returning the bytes written.
    ///
    /// `codec` defaults to the coding this resource's own name declares, which
    /// is what makes `handle.decompressInto(plain)` the whole of reading a
    /// `.gz` back. An explicit one overrides that reading - the escape hatch for
    /// bytes whose name lies, or for raw DEFLATE, which no name can declare.
    /// The target's media type comes back with the coding removed.
    #[napi]
    pub fn decompress_into(&self, target: &mut JsIOBase, codec: Option<String>) -> Result<i64> {
        let codec = match codec {
            Some(name) => yggdryl::Codec::from_str(&name).map_err(napi_error)?,
            None => self.inner.codec(),
        };
        let written = self
            .inner
            .decompress_into_with(&mut target.inner, codec)
            .map_err(napi_error)?;
        Ok(i64::try_from(written).unwrap_or(i64::MAX))
    }

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

    /// Iterate this resource's text records, one at a time.
    ///
    /// The loader's `readLines` wraps this with its option coercion: the whole
    /// extractor crosses as one native `Scalar` - the same shape a YAML or TOML
    /// document parses into - so a reader is specifiable from configuration
    /// alone, in JavaScript or in a file.
    ///
    /// Any content codings the resource's name declares - `trades.jsonl.gz`,
    /// `log.txt.zst` - decode as streams, so a compressed resource is read
    /// without ever holding its decompressed value. The iterator owns a
    /// rebuilt handle, so it stays valid however long the caller keeps it.
    #[napi(js_name = "_readLinesNative", skip_typescript)]
    pub fn read_lines_native(
        &self,
        options: Option<ClassInstance<'_, JsScalar>>,
    ) -> Result<JsLineIterator> {
        let built = text_line_options(options.as_deref())?;
        let handle = self.rebuilt()?;
        let inner = handle
            .inner
            .into_text_with(built)
            .into_read_lines()
            .map_err(napi_error)?;
        Ok(JsLineIterator { inner })
    }

    /// Project this resource's text records into a `BatchReader`.
    ///
    /// A text-line surface beside `readLines`, **never a record method**: each
    /// record becomes one row - `url`, `rownum`, `date`, `time`, `unix`,
    /// `hash`, `header`, `message`, `offset`, `lines`, then in log mode the
    /// fixed `level`, `logger`, and `thread`, then one nullable column per
    /// named capture group, then the constant custom columns.
    ///
    /// The boundary is the standard copied IPC one - each batch crosses as its
    /// own self-contained Arrow IPC stream, never zero-copy.
    #[napi(js_name = "_readArrowLinesNative", skip_typescript)]
    pub fn read_arrow_lines_native(
        &self,
        options: Option<ClassInstance<'_, JsScalar>>,
    ) -> Result<JsBatchReader> {
        let built = text_line_options(options.as_deref())?;
        // The borrowed core projection: it reopens a located leaf itself -
        // keeping a declared media-type override - and snapshots an
        // in-memory handle, so `fromBytes` parses exactly as a file does.
        let reader = self.inner.read_arrow_lines(&built).map_err(napi_error)?;
        Ok(JsBatchReader::from_core(reader, "row"))
    }

    /// Replace this resource's records with what `pull` yields, each terminated.
    ///
    /// The loader turns any iterable into `pull`, a function answering the next
    /// record or `null` at the end, so the records stream: they are never
    /// collected into an array on either side of the boundary, and a
    /// million-record write costs one reused buffer.
    #[napi(js_name = "_writeLinesNative", skip_typescript)]
    pub fn write_lines_native(
        &mut self,
        pull: Function<'_, (), Option<Either<String, Uint8Array>>>,
        options: Option<ClassInstance<'_, JsScalar>>,
    ) -> Result<()> {
        self.inner.truncate(0).map_err(napi_error)?;
        self.append_lines_native(pull, options)
    }

    /// Append what `pull` yields after this resource's current end.
    ///
    /// Streams exactly as `_writeLinesNative` does, and publishes when it
    /// finishes: appending records is a complete operation, so a staging
    /// backend must not be left holding it.
    #[napi(js_name = "_appendLinesNative", skip_typescript)]
    pub fn append_lines_native(
        &mut self,
        pull: Function<'_, (), Option<Either<String, Uint8Array>>>,
        options: Option<ClassInstance<'_, JsScalar>>,
    ) -> Result<()> {
        let built = text_line_options(options.as_deref())?;
        let terminator = built.write_linesep().to_vec();
        // One reused buffer, flushed in chunks, exactly as the core writes.
        let mut pending: Vec<u8> = Vec::with_capacity(LINE_WRITE_CHUNK);
        let mut offset = self.inner.size();
        while let Some(record) = pull.call(())? {
            match &record {
                Either::A(text) => pending.extend_from_slice(text.as_bytes()),
                Either::B(bytes) => pending.extend_from_slice(bytes.as_ref()),
            }
            pending.extend_from_slice(&terminator);
            if pending.len() >= LINE_WRITE_CHUNK {
                self.inner
                    .pwrite_all(offset, &pending)
                    .map_err(napi_error)?;
                offset += pending.len() as u64;
                pending.clear();
            }
        }
        if !pending.is_empty() {
            self.inner
                .pwrite_all(offset, &pending)
                .map_err(napi_error)?;
        }
        self.inner.flush().map_err(napi_error)
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

    /// Read one Parquet leaf's footer statistics without decoding rows.
    #[napi(js_name = "_readParquetStatisticsNative", skip_typescript)]
    pub fn read_parquet_statistics_native(&self) -> Result<serde_json::Value> {
        let statistics = self.inner.read_parquet_statistics().map_err(napi_error)?;
        value_to_transport_for_field(&yggdryl::Scalar::from(statistics), None, DEFAULT_JS_DEPTH)
    }

    /// Recompute one Parquet geospatial column's bounds and geometry types.
    #[napi(js_name = "_readParquetGeospatialStatisticsNative", skip_typescript)]
    pub fn read_parquet_geospatial_statistics_native(
        &self,
        column: String,
    ) -> Result<serde_json::Value> {
        let statistics = self
            .inner
            .read_parquet_geospatial_statistics(&column)
            .map_err(napi_error)?;
        value_to_transport_for_field(&yggdryl::Scalar::from(statistics), None, DEFAULT_JS_DEPTH)
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
    /// A field on the options selects and casts during the read: the columns it
    /// names that this resource stores become the encoding's own projection, so
    /// the rest are skipped rather than read and discarded, and what comes back
    /// is the shape it declares. A handle addressing a folder reads across the
    /// partitions beneath it, restoring the columns their directory names spell
    /// out.
    #[napi]
    pub fn read_arrow_reader(&self, options: Option<&JsRecordOptions>) -> Result<JsBatchReader> {
        let options = JsRecordOptions::resolved(options, &self.inner)?;
        let reader = self.inner.read_arrow_reader(&options).map_err(napi_error)?;
        Ok(JsBatchReader::from_core(reader, options.root_name()))
    }

    /// Replace this resource's rows with every batch `batches` yields.
    ///
    /// This is the native-reader publication hook. The incoming stream is cast
    /// to `options.field` once in the core, and a match key is refused because
    /// overwrite never infers merge intent.
    #[napi]
    pub fn overwrite_arrow_reader(
        &mut self,
        batches: &mut JsBatchReader,
        options: Option<&JsRecordOptions>,
    ) -> Result<()> {
        let options = JsRecordOptions::resolved(options, &self.inner)?;
        options
            .require_write_mode(IOMode::Overwrite)
            .map_err(napi_error)?;
        self.inner
            .overwrite_arrow_reader(batches.take()?, &options)
            .map_err(napi_error)
    }

    /// Add every batch `batches` yields after the rows this resource holds.
    ///
    /// Both sides stream: what is stored is chained ahead of what arrives, and
    /// incoming batches are cast to the target shape as they are pulled.
    #[napi]
    pub fn append_arrow_reader(
        &mut self,
        batches: &mut JsBatchReader,
        options: Option<&JsRecordOptions>,
    ) -> Result<()> {
        let options = JsRecordOptions::resolved(options, &self.inner)?;
        options
            .require_write_mode(IOMode::Append)
            .map_err(napi_error)?;
        self.inner
            .append_arrow_reader(batches.take()?, &options)
            .map_err(napi_error)
    }

    /// Merge every incoming row by `options.mergeByNames`.
    ///
    /// A non-empty match key is required. The core keeps the incoming reader
    /// streaming, applies `options.field` once, and publishes through the
    /// implementor's overwrite hook without casting the shaped rows twice.
    #[napi]
    pub fn merge_arrow_reader(
        &mut self,
        batches: &mut JsBatchReader,
        options: Option<&JsRecordOptions>,
    ) -> Result<()> {
        let options = JsRecordOptions::resolved(options, &self.inner)?;
        options
            .require_write_mode(IOMode::Merge)
            .map_err(napi_error)?;
        self.inner
            .merge_arrow_reader(batches.take()?, &options)
            .map_err(napi_error)
    }

    /// Write a native reader using one explicit mode.
    ///
    /// The JavaScript adapter exposes this method with the closed `IOMode`
    /// union and the wider `RecordOptionsInput`. Keep napi-rs from also
    /// publishing a looser `string` overload in the generated declarations.
    #[napi(skip_typescript)]
    pub fn write_arrow_reader(
        &mut self,
        batches: &mut JsBatchReader,
        mode: String,
        options: Option<&JsRecordOptions>,
    ) -> Result<()> {
        let mode = IOMode::from_str(&mode).map_err(napi_error)?;
        let options = JsRecordOptions::resolved(options, &self.inner)?;
        options.require_write_mode(mode).map_err(napi_error)?;
        self.inner
            .write_arrow_reader(batches.take()?, mode, &options)
            .map_err(napi_error)
    }

    /// Start the private mode-selected session used between async pulls.
    #[napi(js_name = "_beginArrowWriteSessionNative", skip_typescript)]
    pub fn begin_arrow_write_session(
        &self,
        mode: String,
        options: &JsRecordOptions,
    ) -> Result<JsArrowWriteSession> {
        let mode = IOMode::from_str(&mode).map_err(napi_error)?;
        yggdryl::io::ArrowWriteSession::new(mode, &options.inner)
            .map(|inner| JsArrowWriteSession { inner })
            .map_err(napi_error)
    }

    /// Push one decoded IPC chunk and report whether another may be pulled.
    #[napi(js_name = "_pushArrowWriteSessionNative", skip_typescript)]
    pub fn push_arrow_write_session(
        &mut self,
        session: &mut JsArrowWriteSession,
        batches: &mut JsBatchReader,
    ) -> Result<bool> {
        session
            .inner
            .push(&mut self.inner, batches.take()?)
            .map_err(napi_error)
    }

    /// Publish the final partial cadence and close the private session.
    #[napi(js_name = "_finishArrowWriteSessionNative", skip_typescript)]
    pub fn finish_arrow_write_session(&mut self, session: &mut JsArrowWriteSession) -> Result<()> {
        session.inner.finish(&mut self.inner).map_err(napi_error)
    }

    /// Discard the private session's unpublished partial cadence.
    #[napi(js_name = "_abortArrowWriteSessionNative", skip_typescript)]
    pub fn abort_arrow_write_session(&mut self, session: &mut JsArrowWriteSession) {
        session.inner.abort();
    }

    /// Decode this location as a host-independent forward-slash path.
    #[napi]
    pub fn into_path(&self) -> Result<String> {
        let url = self
            .inner
            .url()
            .ok_or_else(|| napi_error("this resource has no file system path"))?;
        url.clone()
            .into_path()
            .map_err(napi_error)?
            .into_os_string()
            .into_string()
            .map_err(|_| napi_error("file URI path cannot be represented as a JavaScript string"))
    }

    /// Return the location as text, so a handle prints where it points.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.inner
            .url()
            .map_or_else(|| "<memory>".to_owned(), ToString::to_string)
    }
}

/// The entries of one listing, one at a time.
///
/// Built by `iterdir`, `ls`, `glob`, `rglob`, `childrenMatching`, and
/// `childrenWhere`. It wraps the core listing directly, so nothing is
/// collected on the way across the boundary; `next()` is the native half of
/// the iteration protocol and the loader wraps it so `for...of` yields
/// handles. A failure throws at the entry it happened on, after which the
/// listing is exhausted.
#[napi(js_name = "Listing")]
pub struct JsListing {
    inner: yggdryl::io::Listing,
}

#[napi]
impl JsListing {
    /// The next entry, or `null` when the listing is exhausted.
    #[napi]
    pub fn next(&mut self) -> Result<Option<JsIOBase>> {
        match self.inner.next() {
            None => Ok(None),
            Some(entry) => entry
                .map(|entry| Some(JsIOBase::from_core(entry)))
                .map_err(napi_error),
        }
    }
}

/// Iterator over a resource's text records, one at a time.
///
/// Built by `readLines`. The handle is rebuilt from its location and owned
/// here, bytes stream through one bounded window, and any content codings the
/// name declares decode as streams, so a compressed resource costs one window
/// rather than its decoded size. `next()` is the native half of the iteration
/// protocol; the loader wraps it so `for...of` yields strings.
///
/// Each record crosses as a JavaScript string. The core hands back a
/// *borrowed* view whose lifetime ends at the next read, and a JavaScript
/// value cannot borrow it - so this is the one place the line surface copies,
/// and it copies because the boundary requires it, not because the reader
/// does.
#[napi(js_name = "LineIterator")]
pub struct JsLineIterator {
    inner: yggdryl::text::TextLines<Box<dyn std::io::Read + Send + 'static>>,
}

#[napi]
impl JsLineIterator {
    /// The next record, or `null` when the resource is exhausted.
    #[napi]
    pub fn next(&mut self) -> Result<Option<String>> {
        match self.inner.next() {
            None => Ok(None),
            Some(record) => record
                .and_then(|record| record.text().map(str::to_owned))
                .map(Some)
                .map_err(napi_error),
        }
    }
}

/// A lazy iterator of bounded byte arrays.
///
/// Built by `IOBase.pstreamBytes` and `IOCursor.streamBytes`. The iterator
/// retains a native reference to its source, so dropping the originating
/// JavaScript variable cannot invalidate an in-flight stream. One `next()`
/// performs one bounded core stream step; EOF and the first failure fuse it.
#[napi(js_name = "ByteIterator")]
pub struct JsByteIterator {
    source: ByteIteratorSource,
    batch_size: usize,
    done: bool,
}

enum ByteIteratorSource {
    Handle {
        handle: Reference<JsIOBase>,
        position: u64,
    },
    Cursor {
        cursor: Reference<JsIOCursor>,
    },
}

#[napi]
impl JsByteIterator {
    /// Return the next byte chunk, or `null` after EOF.
    #[napi]
    pub fn next(&mut self) -> Result<Option<Buffer>> {
        if self.done {
            return Ok(None);
        }

        let item = match &mut self.source {
            ByteIteratorSource::Handle { handle, position } => {
                let next = match handle.inner.pstream_bytes(*position, self.batch_size) {
                    Ok(mut stream) => stream.next(),
                    Err(error) => {
                        self.done = true;
                        return Err(napi_error(error));
                    }
                };
                match &next {
                    Some(Ok(bytes)) => {
                        *position = position
                            .checked_add(bytes.len() as u64)
                            .ok_or_else(|| napi_error("byte stream position exceeds u64::MAX"))?;
                    }
                    Some(Err(_)) | None => self.done = true,
                }
                next
            }
            ByteIteratorSource::Cursor { cursor } => {
                let position = cursor.position;
                let next = match cursor.handle.inner.pstream_bytes(position, self.batch_size) {
                    Ok(mut stream) => stream.next(),
                    Err(error) => {
                        self.done = true;
                        return Err(napi_error(error));
                    }
                };
                match &next {
                    Some(Ok(bytes)) => {
                        cursor.position = position
                            .checked_add(bytes.len() as u64)
                            .ok_or_else(|| napi_error("byte stream position exceeds u64::MAX"))?;
                    }
                    Some(Err(_)) | None => self.done = true,
                }
                next
            }
        };

        match item {
            None => Ok(None),
            Some(Ok(bytes)) => Ok(Some(Buffer::from(bytes))),
            Some(Err(error)) => {
                self.done = true;
                Err(napi_error(error))
            }
        }
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

    /// Stream bounded byte arrays from the current position.
    ///
    /// The iterator keeps this cursor and its backing handle alive. Its
    /// position advances only as chunks are yielded, so dropping a partially
    /// consumed iterator leaves the cursor immediately after the last chunk.
    /// `batchSize` defaults to 64 KiB.
    #[napi]
    // This must remain an object method: NAPI supplies `reference` for this
    // exact cursor instance so the iterator can own it after the call returns.
    #[allow(clippy::unused_self)]
    pub fn stream_bytes(
        &self,
        reference: Reference<JsIOCursor>,
        batch_size: Option<f64>,
    ) -> Result<JsByteIterator> {
        Ok(JsByteIterator {
            source: ByteIteratorSource::Cursor { cursor: reference },
            batch_size: byte_stream_batch_size(batch_size)?,
            done: false,
        })
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

/// Validate and narrow a JavaScript byte-stream batch size.
fn byte_stream_batch_size(value: Option<f64>) -> Result<usize> {
    let value = value.map_or(Ok(BYTE_STREAM_BATCH_SIZE as u64), |value| {
        exact_u64(value, "batchSize")
    })?;
    let value = usize::try_from(value).map_err(|_| napi_error("batchSize is too large"))?;
    if value == 0 {
        return Err(napi_error(
            "byte stream batch_size must be greater than zero",
        ));
    }
    Ok(value)
}

/// Build the line projection's root Struct Field straight from a pattern.
///
/// The loader's `fieldFromPattern` wraps this with the same option coercion
/// `readArrowLines` uses: the schema the reader emits - named captures typed
/// by inference or declaration - without a resource or a reader in sight, so
/// a caller marks its partition columns and creates the Iceberg table before
/// the first log line exists.
#[napi(js_name = "_fieldFromPatternNative", skip_typescript)]
// Discovered through NAPI's generated registration inventory rather than an
// ordinary Rust call site.
#[allow(dead_code)]
pub fn field_from_pattern_native(options: Option<ClassInstance<'_, JsScalar>>) -> Result<JsField> {
    text_line_options(options.as_deref()).map(|options| JsField::from_core(options.into_field()))
}

/// Read the whole extractor out of the one native `Scalar` the loader built.
///
/// Every text-line entry point comes here, so the JavaScript surface and a
/// configuration document are validated by exactly the same core conversion -
/// there is no second option parser to drift.
fn text_line_options(options: Option<&JsScalar>) -> Result<TextLineOptions> {
    match options {
        Some(value) => TextLineOptions::from_value(value.inner.clone()).map_err(napi_error),
        None => Ok(TextLineOptions::new()),
    }
}
