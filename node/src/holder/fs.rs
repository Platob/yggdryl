//! A synchronous JavaScript [`FileSystem`] handler.
//!
//! The adapter retains the caller's object, binds every Arrow filesystem
//! operation once, and forwards streams without collecting their payload.
//! JavaScript values remain confined to the isolate thread that created them.

use std::any::Any;
use std::collections::HashMap;
use std::io::SeekFrom;
use std::thread::ThreadId;

use napi::bindgen_prelude::{
    BigInt, Env, FnArgs, FromNapiValue, Function, FunctionRef, JsObjectValue as _,
    JsValuesTupleIntoVec, Object, Uint8Array,
};
use napi::{JsError, JsValue as _};
use napi_derive::napi;

use yggdryl::holder::fs::{
    ByteReader, ByteWriter, FileInfo, FileInfos, FileSelector as CoreFileSelector, FileSystem,
    OutputMetadata, RandomAccessReader,
};
use yggdryl::{Error, IOKind, Result};

use crate::napi_error;

/// A caller-supplied Arrow filesystem protocol object.
pub(crate) type FileSystemInput<'a> = Object<'a>;

/// Options for one filesystem listing.
#[napi(object)]
pub struct FileSelector {
    /// Opaque directory path supplied to the filesystem.
    pub base_dir: String,
    /// Whether descendants below direct children are included.
    pub recursive: bool,
    /// Whether a missing base directory produces an empty listing.
    pub allow_not_found: bool,
}

impl From<&CoreFileSelector> for FileSelector {
    fn from(value: &CoreFileSelector) -> Self {
        Self {
            base_dir: value.base_dir.clone(),
            recursive: value.recursive,
            allow_not_found: value.allow_not_found,
        }
    }
}

/// Arrow-compatible information for one exact path.
#[napi(object)]
pub struct ArrowFileInfo {
    /// Exact opaque path reported by the filesystem.
    pub path: String,
    /// Resource kind: `file`, `directory`, or `not-found`.
    pub kind: String,
    /// Exact file size, omitted for directories and missing paths.
    pub size: Option<BigInt>,
    /// Optional UTC modification time in nanoseconds since the Unix epoch.
    pub mtime_ns: Option<BigInt>,
}

impl ArrowFileInfo {
    fn into_core(self) -> Result<FileInfo> {
        let kind = match self.kind.as_str() {
            "file" => IOKind::File,
            "directory" => IOKind::Directory,
            "not-found" => IOKind::Unknown,
            value => {
                return Err(invalid(format!(
                    "expected file info kind to be 'file', 'directory', or 'not-found', got {value:?}"
                )));
            }
        };
        let size = self
            .size
            .map(|value| exact_bigint_u64(&value, "size"))
            .transpose()?;
        let mtime_ns = self
            .mtime_ns
            .map(|value| exact_bigint_i64(&value, "mtimeNs"))
            .transpose()?;
        if kind != IOKind::File && size.is_some() {
            return Err(invalid(format!(
                "expected {:?} file info to omit size",
                self.kind
            )));
        }
        Ok(FileInfo {
            path: self.path,
            kind,
            size,
            mtime_ns,
        })
    }

    pub(crate) fn from_core(value: FileInfo) -> Self {
        Self {
            path: value.path,
            kind: if value.kind == IOKind::Unknown {
                "not-found".to_owned()
            } else {
                value.kind.as_str().to_owned()
            },
            size: value.size.map(BigInt::from),
            mtime_ns: value.mtime_ns.map(BigInt::from),
        }
    }
}

pub(crate) fn exact_bigint_u64(value: &BigInt, name: &str) -> Result<u64> {
    let (negative, value, lossless) = value.get_u64();
    if negative || !lossless {
        return Err(invalid(format!(
            "expected {name} to fit an unsigned 64-bit integer"
        )));
    }
    Ok(value)
}

pub(crate) fn exact_bigint_i64(value: &BigInt, name: &str) -> Result<i64> {
    let (value, lossless) = value.get_i64();
    if !lossless {
        return Err(invalid(format!(
            "expected {name} to fit a signed 64-bit integer"
        )));
    }
    Ok(value)
}

fn invalid(message: impl Into<String>) -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        message.into(),
    ))
}

/// Preserve distinctions exposed by ordinary Node filesystem errors.
fn foreign(env: &Env, error: napi::Error) -> Error {
    let mut reason = error.reason.clone();
    let js_env = Env::from_raw(env.raw());
    let value = JsError::from(error).into_unknown(js_env);
    let code = value
        .coerce_to_object()
        .ok()
        .and_then(|object| object.get::<String>("code").ok().flatten());
    if let Some(code) = &code
        && !reason.contains(code)
    {
        reason = format!("{code}: {reason}");
    }
    let kind = match code.as_deref() {
        Some("NotFound" | "ENOENT") => std::io::ErrorKind::NotFound,
        Some("PermissionDenied" | "EACCES" | "EPERM") => std::io::ErrorKind::PermissionDenied,
        Some("AlreadyExists" | "EEXIST") => std::io::ErrorKind::AlreadyExists,
        Some("NotADirectory" | "ENOTDIR") => std::io::ErrorKind::NotADirectory,
        Some("IsADirectory" | "EISDIR") => std::io::ErrorKind::IsADirectory,
        Some("DirectoryNotEmpty" | "ENOTEMPTY") => std::io::ErrorKind::DirectoryNotEmpty,
        Some("Unsupported" | "ENOTSUP" | "EOPNOTSUPP") => std::io::ErrorKind::Unsupported,
        Some("Transport") | None | Some(_) => std::io::ErrorKind::Other,
    };
    Error::Io(std::io::Error::new(kind, reason))
}

fn off_thread(name: &str) -> Error {
    Error::Io(std::io::Error::other(format!(
        "filesystem handler {name:?} belongs to another JavaScript isolate thread"
    )))
}

fn bound_method<Args: JsValuesTupleIntoVec, Return: FromNapiValue>(
    handler: &Object<'_>,
    name: &str,
    signature: &str,
) -> napi::Result<FunctionRef<Args, Return>> {
    let method: Function<'_, Args, Return> = handler.get_named_property(name).map_err(|error| {
        napi_error(format!(
            "expected a filesystem handler defining {signature}: {error}"
        ))
    })?;
    method.bind(*handler)?.create_ref()
}

/// Bind equality and retain the original handler on the bound function.
fn bound_equality(handler: &Object<'_>) -> napi::Result<FunctionRef<Object<'static>, bool>> {
    let method: Function<'_, Object<'static>, bool> =
        handler.get_named_property("equals").map_err(|error| {
            napi_error(format!(
                "expected a filesystem handler defining equals(other): {error}"
            ))
        })?;
    let mut bound = method.bind(*handler)?;
    bound.set_named_property("__yggdrylFilesystemHandler", *handler)?;
    bound.create_ref()
}

type Metadata = Option<HashMap<String, String>>;

/// One held JavaScript handler presented as the core filesystem vtable.
pub(crate) struct JsFileSystem {
    name: String,
    equals: FunctionRef<Object<'static>, bool>,
    normalize_path: FunctionRef<String, String>,
    file_info: FunctionRef<String, ArrowFileInfo>,
    list: FunctionRef<FileSelector, Object<'static>>,
    create_dir: FunctionRef<FnArgs<(String, bool)>, ()>,
    delete_dir: FunctionRef<String, ()>,
    delete_dir_contents: FunctionRef<FnArgs<(String, bool)>, ()>,
    delete_root_dir_contents: FunctionRef<(), ()>,
    delete_file: FunctionRef<String, ()>,
    copy_file: FunctionRef<FnArgs<(String, String)>, ()>,
    move_file: FunctionRef<FnArgs<(String, String)>, ()>,
    open_input_file: FunctionRef<String, Object<'static>>,
    open_input_stream: FunctionRef<String, Object<'static>>,
    open_output_stream: FunctionRef<FnArgs<(String, Metadata)>, Object<'static>>,
    open_append_stream: FunctionRef<FnArgs<(String, Metadata)>, Object<'static>>,
    environment: usize,
    thread: ThreadId,
}

impl JsFileSystem {
    pub(crate) fn new(env: Env, handler: &Object<'_>) -> napi::Result<Self> {
        let name = handler
            .get::<String>("typeName")?
            .ok_or_else(|| napi_error("expected a filesystem handler with a string typeName"))?;
        Ok(Self {
            name,
            equals: bound_equality(handler)?,
            normalize_path: bound_method(handler, "normalizePath", "normalizePath(path)")?,
            file_info: bound_method(handler, "fileInfo", "fileInfo(path)")?,
            list: bound_method(handler, "list", "list(selector)")?,
            create_dir: bound_method(handler, "createDir", "createDir(path, recursive)")?,
            delete_dir: bound_method(handler, "deleteDir", "deleteDir(path)")?,
            delete_dir_contents: bound_method(
                handler,
                "deleteDirContents",
                "deleteDirContents(path, missingDirOk)",
            )?,
            delete_root_dir_contents: bound_method(
                handler,
                "deleteRootDirContents",
                "deleteRootDirContents()",
            )?,
            delete_file: bound_method(handler, "deleteFile", "deleteFile(path)")?,
            copy_file: bound_method(handler, "copyFile", "copyFile(source, target)")?,
            move_file: bound_method(handler, "move", "move(source, target)")?,
            open_input_file: bound_method(handler, "openInputFile", "openInputFile(path)")?,
            open_input_stream: bound_method(handler, "openInputStream", "openInputStream(path)")?,
            open_output_stream: bound_method(
                handler,
                "openOutputStream",
                "openOutputStream(path, metadata?)",
            )?,
            open_append_stream: bound_method(
                handler,
                "openAppendStream",
                "openAppendStream(path, metadata?)",
            )?,
            environment: env.raw().expose_provenance(),
            thread: std::thread::current().id(),
        })
    }

    fn on_js_thread<T>(&self, call: impl FnOnce(&Env) -> napi::Result<T>) -> Result<T> {
        if std::thread::current().id() != self.thread {
            return Err(off_thread(&self.name));
        }
        let env = Env::from_raw(std::ptr::with_exposed_provenance_mut(self.environment));
        call(&env).map_err(|error| foreign(&env, error))
    }

    pub(crate) fn handler<'env>(&self, env: &'env Env) -> napi::Result<Object<'env>> {
        if env.raw().expose_provenance() != self.environment
            || std::thread::current().id() != self.thread
        {
            return Err(napi_error(
                "filesystem handler belongs to another JavaScript isolate",
            ));
        }
        let handler: Object<'env> = self
            .equals
            .borrow_back(env)?
            .get_named_property("__yggdrylFilesystemHandler")?;
        Ok(handler
            .get::<Object<'env>>("__yggdrylOriginalFilesystemHandler")?
            .unwrap_or(handler))
    }

    fn metadata(metadata: Option<&OutputMetadata>) -> Metadata {
        metadata.map(|metadata| {
            metadata
                .iter()
                .map(|(key, value)| (key.to_owned(), value.to_owned()))
                .collect()
        })
    }

    fn collect_iterable(env: &Env, iterable: Object<'static>) -> napi::Result<Vec<ArrowFileInfo>> {
        let global = env.get_global()?;
        let array: Function<'_, (), Object<'_>> = global.get_named_property("Array")?;
        let from: Function<'_, Object<'static>, Vec<ArrowFileInfo>> =
            array.get_named_property("from")?;
        from.call(iterable)
    }

    pub(crate) fn input_file(&self, path: &str) -> Result<JsRandomAccessReader> {
        let stream =
            self.on_js_thread(|env| self.open_input_file.borrow_back(env)?.call(path.to_owned()))?;
        let env = Env::from_raw(std::ptr::with_exposed_provenance_mut(self.environment));
        JsRandomAccessReader::new(self.environment, self.thread, stream)
            .map_err(|error| foreign(&env, error))
    }

    pub(crate) fn input_stream(&self, path: &str) -> Result<JsByteReader> {
        let stream = self.on_js_thread(|env| {
            self.open_input_stream
                .borrow_back(env)?
                .call(path.to_owned())
        })?;
        let env = Env::from_raw(std::ptr::with_exposed_provenance_mut(self.environment));
        JsByteReader::new(self.environment, self.thread, stream)
            .map_err(|error| foreign(&env, error))
    }

    pub(crate) fn output_stream(
        &self,
        path: &str,
        metadata: Option<&OutputMetadata>,
    ) -> Result<JsByteWriter> {
        let stream = self.on_js_thread(|env| {
            self.open_output_stream
                .borrow_back(env)?
                .call((path.to_owned(), Self::metadata(metadata)).into())
        })?;
        let env = Env::from_raw(std::ptr::with_exposed_provenance_mut(self.environment));
        JsByteWriter::new(self.environment, self.thread, stream)
            .map_err(|error| foreign(&env, error))
    }

    pub(crate) fn append_stream(
        &self,
        path: &str,
        metadata: Option<&OutputMetadata>,
    ) -> Result<JsByteWriter> {
        let stream = self.on_js_thread(|env| {
            self.open_append_stream
                .borrow_back(env)?
                .call((path.to_owned(), Self::metadata(metadata)).into())
        })?;
        let env = Env::from_raw(std::ptr::with_exposed_provenance_mut(self.environment));
        JsByteWriter::new(self.environment, self.thread, stream)
            .map_err(|error| foreign(&env, error))
    }
}

impl FileSystem for JsFileSystem {
    fn type_name(&self) -> &str {
        &self.name
    }

    fn equals(&self, other: &dyn FileSystem) -> bool {
        self.try_equals(other).unwrap_or(false)
    }

    fn try_equals(&self, other: &dyn FileSystem) -> Result<bool> {
        let Some(other) = other.as_any().downcast_ref::<Self>() else {
            return Ok(false);
        };
        if self.environment != other.environment || self.thread != other.thread {
            return Ok(false);
        }
        self.on_js_thread(|env| {
            let other = other.handler(env)?;
            self.equals.borrow_back(env)?.call(other)
        })
    }

    fn normalize_path(&self, path: &str) -> Result<String> {
        self.on_js_thread(|env| self.normalize_path.borrow_back(env)?.call(path.to_owned()))
    }

    fn file_info(&self, path: &str) -> Result<FileInfo> {
        self.on_js_thread(|env| self.file_info.borrow_back(env)?.call(path.to_owned()))?
            .into_core()
    }

    fn list(&self, selector: &CoreFileSelector) -> FileInfos {
        let listed = self.on_js_thread(|env| {
            let iterable = self
                .list
                .borrow_back(env)?
                .call(FileSelector::from(selector))?;
            Self::collect_iterable(env, iterable)
        });
        match listed {
            Ok(entries) => {
                let mut entries = match entries
                    .into_iter()
                    .map(ArrowFileInfo::into_core)
                    .collect::<Result<Vec<_>>>()
                {
                    Ok(entries) => entries,
                    Err(error) => return FileInfos::failing(error),
                };
                entries.sort_by(|left, right| left.path.cmp(&right.path));
                FileInfos::new(entries.into_iter().map(Ok))
            }
            Err(error) => FileInfos::failing(error),
        }
    }

    fn create_dir(&self, path: &str, recursive: bool) -> Result<()> {
        self.on_js_thread(|env| {
            self.create_dir
                .borrow_back(env)?
                .call((path.to_owned(), recursive).into())
        })
    }

    fn delete_dir(&self, path: &str) -> Result<()> {
        self.on_js_thread(|env| self.delete_dir.borrow_back(env)?.call(path.to_owned()))
    }

    fn delete_dir_contents(&self, path: &str, missing_dir_ok: bool) -> Result<()> {
        self.on_js_thread(|env| {
            self.delete_dir_contents
                .borrow_back(env)?
                .call((path.to_owned(), missing_dir_ok).into())
        })
    }

    fn delete_root_dir_contents(&self) -> Result<()> {
        self.on_js_thread(|env| self.delete_root_dir_contents.borrow_back(env)?.call(()))
    }

    fn delete_file(&self, path: &str) -> Result<()> {
        self.on_js_thread(|env| self.delete_file.borrow_back(env)?.call(path.to_owned()))
    }

    fn copy_file(&self, source: &str, target: &str) -> Result<()> {
        self.on_js_thread(|env| {
            self.copy_file
                .borrow_back(env)?
                .call((source.to_owned(), target.to_owned()).into())
        })
    }

    fn move_file(&self, source: &str, target: &str) -> Result<()> {
        self.on_js_thread(|env| {
            self.move_file
                .borrow_back(env)?
                .call((source.to_owned(), target.to_owned()).into())
        })
    }

    fn open_input_file(&self, path: &str) -> Result<Box<dyn RandomAccessReader>> {
        self.input_file(path)
            .map(|stream| Box::new(stream) as Box<dyn RandomAccessReader>)
    }

    fn open_input_stream(&self, path: &str) -> Result<Box<dyn ByteReader>> {
        self.input_stream(path)
            .map(|stream| Box::new(stream) as Box<dyn ByteReader>)
    }

    fn open_output_stream(
        &self,
        path: &str,
        metadata: Option<&OutputMetadata>,
    ) -> Result<Box<dyn ByteWriter>> {
        self.output_stream(path, metadata)
            .map(|stream| Box::new(stream) as Box<dyn ByteWriter>)
    }

    fn open_append_stream(
        &self,
        path: &str,
        metadata: Option<&OutputMetadata>,
    ) -> Result<Box<dyn ByteWriter>> {
        self.append_stream(path, metadata)
            .map(|stream| Box::new(stream) as Box<dyn ByteWriter>)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct StreamState {
    environment: usize,
    thread: ThreadId,
    position: u64,
    closed: bool,
    failure: Option<String>,
}

impl StreamState {
    fn new(environment: usize, thread: ThreadId) -> Self {
        Self {
            environment,
            thread,
            position: 0,
            closed: false,
            failure: None,
        }
    }

    fn env(&self) -> Result<Env> {
        if std::thread::current().id() != self.thread {
            return Err(off_thread("stream"));
        }
        Ok(Env::from_raw(std::ptr::with_exposed_provenance_mut(
            self.environment,
        )))
    }

    fn ensure_open(&self) -> Result<()> {
        if self.closed {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "filesystem stream is closed",
            )));
        }
        if let Some(error) = &self.failure {
            return Err(Error::Io(std::io::Error::other(error.clone())));
        }
        Ok(())
    }

    fn remember<T>(&mut self, env: &Env, result: napi::Result<T>) -> Result<T> {
        match result {
            Ok(value) => Ok(value),
            Err(error) => {
                let error = foreign(env, error);
                self.failure = Some(error.to_string());
                Err(error)
            }
        }
    }
}

pub(crate) struct JsByteReader {
    state: StreamState,
    read: FunctionRef<BigInt, Uint8Array>,
    close: FunctionRef<(), ()>,
}

impl JsByteReader {
    fn new(environment: usize, thread: ThreadId, stream: Object<'static>) -> napi::Result<Self> {
        let tell: FunctionRef<(), BigInt> = bound_method(&stream, "tell", "tell()")?;
        let env = Env::from_raw(std::ptr::with_exposed_provenance_mut(environment));
        let position = tell.borrow_back(&env)?.call(()).and_then(|position| {
            exact_bigint_u64(&position, "stream position").map_err(napi_error)
        })?;
        Ok(Self {
            state: StreamState {
                position,
                ..StreamState::new(environment, thread)
            },
            read: bound_method(&stream, "read", "read(length)")?,
            close: bound_method(&stream, "close", "close()")?,
        })
    }

    pub(crate) fn read_owned(&mut self, length: u64) -> Result<Uint8Array> {
        self.state.ensure_open()?;
        let env = self.state.env()?;
        let result = self
            .read
            .borrow_back(&env)
            .and_then(|read| read.call(BigInt::from(length)));
        let bytes = self.state.remember(&env, result)?;
        if bytes.len() as u64 > length {
            return Err(invalid(format!(
                "filesystem stream read returned {} bytes for a {length} byte request",
                bytes.len()
            )));
        }
        self.state.position = self.state.position.saturating_add(bytes.len() as u64);
        Ok(bytes)
    }

    pub(crate) fn tell(&self) -> u64 {
        self.state.position
    }

    pub(crate) fn close(&mut self) -> Result<()> {
        <Self as ByteReader>::close(self)
    }

    pub(crate) fn closed(&self) -> bool {
        self.state.closed
    }
}

impl ByteReader for JsByteReader {
    fn read(&mut self, buffer: &mut [u8]) -> Result<usize> {
        let bytes = self.read_owned(buffer.len() as u64)?;
        buffer[..bytes.len()].copy_from_slice(&bytes);
        Ok(bytes.len())
    }

    fn tell(&self) -> u64 {
        self.state.position
    }

    fn close(&mut self) -> Result<()> {
        if self.state.closed {
            return self.state.failure.as_ref().map_or(Ok(()), |error| {
                Err(Error::Io(std::io::Error::other(error.clone())))
            });
        }
        self.state.closed = true;
        let env = self.state.env()?;
        let result = self
            .close
            .borrow_back(&env)
            .and_then(|close| close.call(()));
        self.state.remember(&env, result)?;
        self.state.failure.as_ref().map_or(Ok(()), |error| {
            Err(Error::Io(std::io::Error::other(error.clone())))
        })
    }

    fn closed(&self) -> bool {
        self.state.closed
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

pub(crate) struct JsRandomAccessReader {
    reader: JsByteReader,
    read_at: FunctionRef<FnArgs<(BigInt, BigInt)>, Uint8Array>,
    seek: FunctionRef<FnArgs<(BigInt, String)>, BigInt>,
}

impl JsRandomAccessReader {
    fn new(environment: usize, thread: ThreadId, stream: Object<'static>) -> napi::Result<Self> {
        Ok(Self {
            reader: JsByteReader::new(environment, thread, stream)?,
            read_at: bound_method(&stream, "readAt", "readAt(offset, length)")?,
            seek: bound_method(&stream, "seek", "seek(offset, whence)")?,
        })
    }

    pub(crate) fn read_owned(&mut self, length: u64) -> Result<Uint8Array> {
        self.reader.read_owned(length)
    }

    pub(crate) fn read_at_owned(&mut self, offset: u64, length: u64) -> Result<Uint8Array> {
        self.reader.state.ensure_open()?;
        let env = self.reader.state.env()?;
        let result = self
            .read_at
            .borrow_back(&env)
            .and_then(|read| read.call((BigInt::from(offset), BigInt::from(length)).into()));
        let bytes = self.reader.state.remember(&env, result)?;
        if bytes.len() as u64 > length {
            return Err(invalid(format!(
                "filesystem readAt returned {} bytes for a {length} byte request",
                bytes.len()
            )));
        }
        Ok(bytes)
    }

    pub(crate) fn seek_to(&mut self, from: SeekFrom) -> Result<u64> {
        <Self as RandomAccessReader>::seek(self, from)
    }

    pub(crate) fn tell(&self) -> u64 {
        self.reader.tell()
    }

    pub(crate) fn close(&mut self) -> Result<()> {
        self.reader.close()
    }

    pub(crate) fn closed(&self) -> bool {
        self.reader.closed()
    }
}

impl ByteReader for JsRandomAccessReader {
    fn read(&mut self, buffer: &mut [u8]) -> Result<usize> {
        self.reader.read(buffer)
    }

    fn tell(&self) -> u64 {
        self.reader.tell()
    }

    fn close(&mut self) -> Result<()> {
        self.reader.close()
    }

    fn closed(&self) -> bool {
        self.reader.closed()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

impl RandomAccessReader for JsRandomAccessReader {
    fn read_at(&mut self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        let bytes = self.read_at_owned(offset, buffer.len() as u64)?;
        buffer[..bytes.len()].copy_from_slice(&bytes);
        Ok(bytes.len())
    }

    fn seek(&mut self, from: SeekFrom) -> Result<u64> {
        self.reader.state.ensure_open()?;
        let (offset, whence) = match from {
            SeekFrom::Start(offset) => (
                i64::try_from(offset).map_err(|_| invalid("seek offset exceeds signed 64 bits"))?,
                "start",
            ),
            SeekFrom::Current(offset) => (offset, "current"),
            SeekFrom::End(offset) => (offset, "end"),
        };
        let env = self.reader.state.env()?;
        let result = self
            .seek
            .borrow_back(&env)
            .and_then(|seek| seek.call((BigInt::from(offset), whence.to_owned()).into()));
        let position = self.reader.state.remember(&env, result)?;
        let position = exact_bigint_u64(&position, "seek result")?;
        self.reader.state.position = position;
        Ok(position)
    }

    fn into_random_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

pub(crate) struct JsByteWriter {
    state: StreamState,
    write: FunctionRef<Uint8Array, BigInt>,
    flush: FunctionRef<(), ()>,
    close: FunctionRef<(), ()>,
}

impl JsByteWriter {
    fn new(environment: usize, thread: ThreadId, stream: Object<'static>) -> napi::Result<Self> {
        let tell: FunctionRef<(), BigInt> = bound_method(&stream, "tell", "tell()")?;
        let env = Env::from_raw(std::ptr::with_exposed_provenance_mut(environment));
        let position = tell.borrow_back(&env)?.call(()).and_then(|position| {
            exact_bigint_u64(&position, "stream position").map_err(napi_error)
        })?;
        Ok(Self {
            state: StreamState {
                position,
                ..StreamState::new(environment, thread)
            },
            write: bound_method(&stream, "write", "write(bytes)")?,
            flush: bound_method(&stream, "flush", "flush()")?,
            close: bound_method(&stream, "close", "close()")?,
        })
    }

    pub(crate) fn write_owned(&mut self, bytes: Uint8Array) -> Result<u64> {
        self.state.ensure_open()?;
        let length = bytes.len() as u64;
        let env = self.state.env()?;
        let result = self
            .write
            .borrow_back(&env)
            .and_then(|write| write.call(bytes));
        let written = self.state.remember(&env, result)?;
        let written = exact_bigint_u64(&written, "write result")?;
        if written > length {
            return Err(invalid(format!(
                "filesystem stream wrote {written} bytes from a {length} byte input"
            )));
        }
        self.state.position = self.state.position.saturating_add(written);
        Ok(written)
    }

    pub(crate) fn tell(&self) -> u64 {
        self.state.position
    }

    pub(crate) fn flush(&mut self) -> Result<()> {
        <Self as ByteWriter>::flush(self)
    }

    pub(crate) fn close(&mut self) -> Result<()> {
        <Self as ByteWriter>::close(self)
    }

    pub(crate) fn closed(&self) -> bool {
        self.state.closed
    }
}

impl ByteWriter for JsByteWriter {
    fn write(&mut self, bytes: &[u8]) -> Result<usize> {
        let written = self.write_owned(Uint8Array::from(bytes.to_vec()))?;
        usize::try_from(written).map_err(|_| invalid("write result exceeds usize"))
    }

    fn tell(&self) -> u64 {
        self.state.position
    }

    fn flush(&mut self) -> Result<()> {
        self.state.ensure_open()?;
        let env = self.state.env()?;
        let result = self
            .flush
            .borrow_back(&env)
            .and_then(|flush| flush.call(()));
        self.state.remember(&env, result)
    }

    fn close(&mut self) -> Result<()> {
        if self.state.closed {
            return self.state.failure.as_ref().map_or(Ok(()), |error| {
                Err(Error::Io(std::io::Error::other(error.clone())))
            });
        }
        self.state.closed = true;
        let env = self.state.env()?;
        let result = self
            .close
            .borrow_back(&env)
            .and_then(|close| close.call(()));
        self.state.remember(&env, result)?;
        self.state.failure.as_ref().map_or(Ok(()), |error| {
            Err(Error::Io(std::io::Error::other(error.clone())))
        })
    }

    fn closed(&self) -> bool {
        self.state.closed
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}
