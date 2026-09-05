//! A caller-supplied JavaScript file system as a core [`FileSystem`].
//!
//! Arrow JS ships no file system, so where the Python binding hands the core
//! a `pyarrow.fs.FileSystem` that already exists, this one takes the same
//! vtable from the caller: a plain object whose methods are Arrow's own
//! `FileSystem` calls spelled in camelCase. Anything a Node program can
//! already reach - a `Map`, `node:fs`, an S3 client, a caching layer over
//! either - becomes a Yggdryl handle by answering those six calls, so no
//! backend needs code of its own here, exactly as none does in Python.
//!
//! # A handler-backed handle belongs to one JavaScript thread
//!
//! [`FileSystem`] is `Send + Sync`, because the core holds one behind an
//! `Arc` and never assumes which thread reads through it. A JavaScript object
//! is the opposite: it lives in one isolate and may only be touched from the
//! thread that owns it. Node-API's one way across that line is a threadsafe
//! function, and it is asynchronous - it queues a call and answers later -
//! while every method of this vtable has to answer now, in the middle of a
//! synchronous core read.
//!
//! So the invariant this module enforces is the honest one: the handler is
//! only ever called on the JavaScript thread that supplied it. Every method
//! compares the running thread against the one recorded at construction and
//! refuses by name rather than touching the reference. That refusal is what
//! makes the type sound to share: nothing here is `unsafe`, because
//! [`FunctionRef`] is `Send + Sync` in Node-API itself - it routes its own
//! release back to the owning thread - and the environment is kept as its
//! exposed address rather than as a pointer, so `Send + Sync` follows from
//! the fields and the thread check is what keeps it true.
//!
//! What that costs a caller: a `Worker` cannot read through a handle whose
//! handler belongs to another thread, and a staged write published by a
//! handle dropped on another thread is discarded rather than written. A
//! worker that wants its own view builds its own handler - the location
//! string is all that has to travel.

use std::thread::ThreadId;

use napi::bindgen_prelude::{
    BigInt, Buffer, Either, Env, FnArgs, FromNapiValue, Function, FunctionRef, JsObjectValue as _,
    JsValuesTupleIntoVec, Object, Uint8Array,
};
use napi_derive::napi;

use yggdryl::holder::fs::{FileInfo, FileInfos, FileSystem};
use yggdryl::{Error, IOKind, Result};

use crate::napi_error;

/// A caller-supplied Arrow file system: the vtable as a plain object.
pub(crate) type FileSystemInput<'a> = Object<'a>;

/// What a file system handler reports about one path.
///
/// The shape `pyarrow.fs.FileInfo` carries, in the spellings the core already
/// publishes: `kind` is an [`IOKind`] name and `size` a 64-bit length, so a
/// value larger than a JavaScript number holds still crosses exactly.
#[napi(object)]
pub struct ArrowFileInfo {
    /// The location, as the file system itself names it. Omitted, it is the
    /// path that was asked about - which is what a handler echoes anyway.
    pub path: Option<String>,
    /// `'file'`, `'directory'`, or `'unknown'` for a path holding nothing
    /// yet. Arrow spells that last one `'not-found'`, and so may a handler.
    pub kind: String,
    /// The byte length, as a `bigint`; a `number` is read when it is exact.
    /// Anything but a file has none.
    pub size: Option<Either<BigInt, f64>>,
}

impl ArrowFileInfo {
    /// Read one handler answer into the core's shape.
    ///
    /// `requested` stands in for an omitted path, so a handler that answers
    /// `{ kind: 'file', size: 12n }` needs no ceremony to repeat what it was
    /// just asked about.
    fn into_core(self, requested: &str) -> Result<FileInfo> {
        // `not-found` is Arrow's own spelling of the kind the core calls
        // unknown, and a handler transcribing `pyarrow.fs.FileType` will
        // reach for it; every other name goes through the core parser rather
        // than a second table of kind names.
        let kind = match self.kind.as_str() {
            "not-found" | "not_found" | "notFound" | "NotFound" => IOKind::Unknown,
            named => IOKind::from_str(named)?,
        };
        let size = match self.size {
            Some(size) if kind == IOKind::File => exact_u64(size)?,
            _ => 0,
        };
        Ok(FileInfo {
            path: self.path.unwrap_or_else(|| requested.to_owned()),
            kind,
            size,
        })
    }
}

/// Read a 64-bit length exactly: a `bigint` as it stands, a number below 2^53.
///
/// A length is what a `bigint` is for at this boundary - an object larger than
/// a JavaScript number holds is a real object - but a handler over `fs.Stats`
/// has a number in hand, and the same exactness check the rest of the package
/// applies to one is what decides whether it may cross.
fn exact_u64(value: Either<BigInt, f64>) -> Result<u64> {
    match value {
        Either::A(value) => {
            let (negative, size, lossless) = value.get_u64();
            if negative || !lossless {
                return Err(invalid(format!(
                    "expected a size that fits in an unsigned 64-bit integer, got {}{size}",
                    if negative { "-" } else { "" },
                )));
            }
            Ok(size)
        }
        Either::B(value) => crate::exact_u64(value, "size").map_err(|error| invalid(error.reason)),
    }
}

/// Refuse a handler answer the core cannot use, naming what arrived.
fn invalid(message: String) -> Error {
    Error::Io(std::io::Error::other(message))
}

/// Carry a JavaScript exception across as a core failure, message intact.
///
/// What a bucket, a handler, or a credential chain said is what the caller
/// needs; rewording it would only hide it.
fn foreign(error: napi::Error) -> Error {
    Error::Io(std::io::Error::other(error.reason))
}

/// Refuse a call that reached the handler from the wrong thread.
fn off_thread(name: &str) -> Error {
    Error::Io(std::io::Error::other(format!(
        "expected the {name} file system handler to be used on the JavaScript thread that \
         supplied it, got another thread: a handler-backed handle cannot be read or written \
         from a worker, because a JavaScript value belongs to one isolate"
    )))
}

/// Take one handler method, bound to the handler it came from.
///
/// Binding is what lets a handler be written the way a JavaScript object is -
/// methods reaching their own state through `this` - instead of as a bag of
/// closures. `signature` spells the method the way a caller would write it,
/// so a missing or misspelled one is refused by its own name.
fn bound_method<Args: JsValuesTupleIntoVec, Return: FromNapiValue>(
    handler: &Object<'_>,
    name: &str,
    signature: &str,
) -> napi::Result<FunctionRef<Args, Return>> {
    let method: Function<'_, Args, Return> = handler.get_named_property(name).map_err(|error| {
        napi_error(format!(
            "expected an Arrow file system handler defining {signature}: {error}"
        ))
    })?;
    method.bind(*handler)?.create_ref()
}

/// A held JavaScript file system handler, presented as the core vtable.
pub(crate) struct JsFileSystem {
    /// The handler's `typeName`, read once at construction.
    ///
    /// It is the one thing the core asks for outside a fallible call - the
    /// scheme a handle's location carries - and it never changes for a given
    /// handler, so reading it eagerly keeps [`FileSystem::type_name`]
    /// from needing the JavaScript thread at all.
    name: String,
    file_info: FunctionRef<String, ArrowFileInfo>,
    list: FunctionRef<FnArgs<(String, bool)>, Vec<ArrowFileInfo>>,
    read_range: FunctionRef<FnArgs<(String, BigInt, u32)>, Option<Uint8Array>>,
    write_full: FunctionRef<FnArgs<(String, Buffer)>, ()>,
    create_dir: FunctionRef<String, ()>,
    delete_file: FunctionRef<String, ()>,
    /// The exposed address of the environment the handler arrived on.
    ///
    /// An address rather than a pointer, because a pointer would make this
    /// type neither `Send` nor `Sync` and the core requires both. It is
    /// turned back into an environment only after `thread` has been checked,
    /// which is the whole of the invariant this module documents.
    environment: usize,
    /// The JavaScript thread that supplied the handler.
    thread: ThreadId,
}

impl JsFileSystem {
    /// Hold `handler`, taking the six calls and the name it reports.
    pub(crate) fn new(env: Env, handler: &Object<'_>) -> napi::Result<Self> {
        // A missing or unreadable name is not a failure: it only decides the
        // scheme the handle's location carries, and `fs` is the generic
        // one the core falls back to anyway.
        let name = handler
            .get::<String>("typeName")
            .ok()
            .flatten()
            .unwrap_or_else(|| "fs".to_owned());
        Ok(Self {
            name,
            file_info: bound_method(handler, "fileInfo", "fileInfo(path)")?,
            list: bound_method(handler, "list", "list(path, recursive)")?,
            read_range: bound_method(handler, "readRange", "readRange(path, offset, length)")?,
            write_full: bound_method(handler, "writeFull", "writeFull(path, bytes)")?,
            create_dir: bound_method(handler, "createDir", "createDir(path)")?,
            delete_file: bound_method(handler, "deleteFile", "deleteFile(path)")?,
            environment: env.raw().expose_provenance(),
            thread: std::thread::current().id(),
        })
    }

    /// Run `call` against the handler, on the thread that supplied it.
    ///
    /// The environment address is recovered only here, after the thread it
    /// belongs to has been established, so the reference is never reached
    /// from a thread that may not touch it.
    fn on_js_thread<T>(&self, call: impl FnOnce(&Env) -> napi::Result<T>) -> Result<T> {
        if std::thread::current().id() != self.thread {
            return Err(off_thread(&self.name));
        }
        let env = Env::from_raw(std::ptr::with_exposed_provenance_mut(self.environment));
        call(&env).map_err(foreign)
    }

    /// What the handler says is at `path`, or `None` when it cannot say.
    fn kind_of(&self, path: &str) -> Option<IOKind> {
        self.file_info(path).ok().map(|info| info.kind)
    }

    /// Answer `recovered` when the handler only failed over what is there.
    ///
    /// Absence is a normal answer in this vtable, not a failure - but a
    /// handler is written against the storage it wraps, and `node:fs` throws
    /// `ENOENT` rather than returning nothing. So the state is asked for
    /// *after* a failure rather than before every call: the working path
    /// stays one call, which is what a ranged read over an object store has
    /// to be, and only positive evidence about the path turns a failure into
    /// an answer. A handler that cannot say anything surfaces its own error.
    fn recovered<T>(
        &self,
        path: &str,
        error: Error,
        recovered: T,
        accepts: impl FnOnce(IOKind) -> bool,
    ) -> Result<T> {
        if self.kind_of(path).is_some_and(accepts) {
            return Ok(recovered);
        }
        Err(error)
    }

    /// Hand the whole value to the handler, once.
    fn publish(&self, path: &str, bytes: &[u8]) -> Result<()> {
        self.on_js_thread(|env| {
            self.write_full
                .borrow_back(env)?
                .call((path.to_owned(), Buffer::from(bytes.to_vec())).into())
        })
    }
}

impl FileSystem for JsFileSystem {
    fn type_name(&self) -> &str {
        &self.name
    }

    fn file_info(&self, path: &str) -> Result<FileInfo> {
        self.on_js_thread(|env| self.file_info.borrow_back(env)?.call(path.to_owned()))?
            .into_core(path)
    }

    fn list(&self, path: &str, recursive: bool) -> FileInfos {
        // The JavaScript side answers with an array, so the foreign call is
        // what collects: Node-API has no shape here to pull one entry at a
        // time through. The bound is that call's own answer, and it is stated
        // here because this is where the collection happens.
        match self.list_collected(path, recursive) {
            Ok(entries) => FileInfos::new(entries.into_iter().map(Ok)),
            Err(error) => FileInfos::failing(error),
        }
    }

    fn read_range(&self, path: &str, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        // A read larger than a 32-bit length is asked for in as many calls as
        // it takes: a short read is the contract's own answer, and every
        // caller of this vtable already loops on one.
        let wanted = u32::try_from(buffer.len()).unwrap_or(u32::MAX);
        let read = self.on_js_thread(|env| {
            self.read_range
                .borrow_back(env)?
                .call((path.to_owned(), BigInt::from(offset), wanted).into())
        });
        let bytes = match read {
            Ok(bytes) => bytes,
            // A file that is not there reads nothing rather than failing.
            Err(error) => return self.recovered(path, error, 0, |kind| kind != IOKind::File),
        };
        let Some(bytes) = bytes else {
            return Ok(0);
        };
        let count = bytes.len().min(buffer.len());
        buffer[..count].copy_from_slice(&bytes[..count]);
        Ok(count)
    }

    fn write_full(&self, path: &str, bytes: &[u8]) -> Result<()> {
        let Err(error) = self.publish(path, bytes) else {
            return Ok(());
        };
        // The parents are this vtable's business, not the handler's, and the
        // cheapest way to create only the ones that are missing is to write
        // first and create after a failure - an object store has none to
        // create and would pay for a call that never mattered. Whole-value
        // replacement is idempotent, so the second attempt is safe.
        let Some((parent, _)) = path.rsplit_once('/') else {
            return Err(error);
        };
        if parent.is_empty() || self.create_dir(parent).is_err() {
            return Err(error);
        }
        self.publish(path, bytes)
    }

    fn create_dir(&self, path: &str) -> Result<()> {
        let created =
            self.on_js_thread(|env| self.create_dir.borrow_back(env)?.call(path.to_owned()));
        match created {
            Ok(()) => Ok(()),
            // A directory that is already there is what was asked for.
            Err(error) => self.recovered(path, error, (), IOKind::is_container),
        }
    }

    fn delete_file(&self, path: &str) -> Result<()> {
        let deleted =
            self.on_js_thread(|env| self.delete_file.borrow_back(env)?.call(path.to_owned()));
        match deleted {
            Ok(()) => Ok(()),
            // A file that is not there is already gone.
            Err(error) => self.recovered(path, error, (), |kind| kind != IOKind::File),
        }
    }
}

impl JsFileSystem {
    /// The JavaScript listing, whole, as that side hands it over.
    fn list_collected(&self, path: &str, recursive: bool) -> Result<Vec<FileInfo>> {
        let listed = self.on_js_thread(|env| {
            self.list
                .borrow_back(env)?
                .call((path.to_owned(), recursive).into())
        });
        let entries = match listed {
            Ok(entries) => entries,
            // A directory that is not there lists empty rather than failing.
            Err(error) => return self.recovered(path, error, Vec::new(), |kind| !kind.is_known()),
        };
        entries
            .into_iter()
            .map(|entry| {
                // A listing entry stands for a location of its own, so unlike
                // `fileInfo` there is nothing sensible to assume when one
                // arrives nameless.
                if entry.path.is_none() {
                    return Err(invalid(format!(
                        "expected every entry list({path:?}) returns to name its own path, \
                         got one without"
                    )));
                }
                entry.into_core(path)
            })
            .collect()
    }
}
