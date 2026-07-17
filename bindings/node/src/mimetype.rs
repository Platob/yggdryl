//! The `yggdryl.mimetype` namespace — one media type (`type/subtype`) with its known
//! extensions and magic-byte signatures, plus the [`MimeCatalog`] registry that resolves a
//! [`MimeType`] from a mime string, a file name, an extension, or a file's magic bytes.
//!
//! Mirrors `yggdryl_core::mimetype`'s root-level [`MimeType`] and the concrete [`MimeCatalog`]
//! (the `MimeRegistry` trait itself is not mirrored — the binding exposes the concrete
//! catalog). A [`MimeType`] is a value type with the usual surface — a byte codec
//! (`serializeBytes` / `deserializeBytes` over its **essence bytes**), content equality
//! (`equals`), a Java-style `hashCode`, and `toString` (the essence) — so it round-trips and
//! works as a map key exactly like the core value. A bad mime string handed to `parse` /
//! `deserializeBytes` throws a guided `Error` carrying the core's text unchanged.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use napi::bindgen_prelude::Buffer;
use napi_derive::napi;

use yggdryl_core::io::Serializable;
use yggdryl_core::mimetype::{self as core, MimeRegistry};

/// Maps any core error to a thrown JS `Error` (its guided text).
fn to_error(error: impl std::fmt::Display) -> napi::Error {
    napi::Error::from_reason(error.to_string())
}

/// A Java-style `i32` content hash of a value, folding the 64-bit hash halves.
fn java_hash<T: Hash>(value: &T) -> i32 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    let hash = hasher.finish();
    (hash as u32 ^ (hash >> 32) as u32) as i32
}

/// One media type: a lowercased `type/subtype` **essence** (the mime string without
/// parameters), the file **extensions** it is known by, and the **magic-byte** signatures a
/// file of this type begins with. A value type — equal, hashable, and byte-serializable (its
/// essence bytes; two `MimeType`s with the same essence are the same type).
#[napi(namespace = "mimetype")]
pub struct MimeType {
    pub(crate) inner: core::MimeType,
}

#[napi(namespace = "mimetype")]
impl MimeType {
    /// Builds a media type from its `essence` (`type/subtype`), known `extensions` (no dot),
    /// and `magic` signatures (an array of `Buffer` / `Uint8Array`). The essence is lowercased;
    /// extensions are lowercased and stripped of a leading dot.
    #[napi(constructor)]
    pub fn new(
        essence: String,
        extensions: Option<Vec<String>>,
        magic: Option<Vec<Buffer>>,
    ) -> Self {
        let magic = magic
            .unwrap_or_default()
            .into_iter()
            .map(|sig| sig.to_vec());
        Self {
            inner: core::MimeType::new(essence, extensions.unwrap_or_default(), magic),
        }
    }

    /// Parses a mime string (`type/subtype` with optional `;`-separated parameters, which are
    /// dropped), returning its **essence** with no extensions or magic. Case-insensitive.
    /// Throws a guided `Error` when the string is not a `type/subtype` essence.
    #[napi(factory)]
    pub fn parse(s: String) -> napi::Result<MimeType> {
        core::MimeType::parse_str(&s)
            .map(|inner| Self { inner })
            .map_err(to_error)
    }

    /// The `application/octet-stream` fallback — an opaque byte stream of unknown type.
    #[napi(factory)]
    pub fn octet_stream() -> MimeType {
        Self {
            inner: core::MimeType::octet_stream(),
        }
    }

    /// Resolves a media type from a file **extension** (no dot) via the default catalog, or
    /// `null` if unknown.
    #[napi]
    pub fn from_extension(ext: String) -> Option<MimeType> {
        core::MimeType::from_extension(&ext).map(|inner| Self { inner })
    }

    /// Resolves a media type from a **file name** (its last extension) via the default catalog,
    /// or `null`.
    #[napi]
    pub fn from_name(name: String) -> Option<MimeType> {
        core::MimeType::from_name(&name).map(|inner| Self { inner })
    }

    /// Resolves a media type from the **magic bytes** at the start of a file via the default
    /// catalog, or `null`.
    #[napi]
    pub fn from_magic(head: Buffer) -> Option<MimeType> {
        core::MimeType::from_magic(head.as_ref()).map(|inner| Self { inner })
    }

    /// The **best guess** for a file `name` (with optional `head` bytes): magic bytes win when
    /// they match, then the name's extension, else `octetStream` — always an answer.
    #[napi(factory)]
    pub fn guess(name: String, head: Buffer) -> MimeType {
        Self {
            inner: core::MimeType::guess(&name, head.as_ref()),
        }
    }

    /// The `type/subtype` essence, e.g. `"application/json"`.
    #[napi(getter)]
    pub fn essence(&self) -> String {
        self.inner.essence().to_string()
    }

    /// The top-level type, e.g. `"application"` of `"application/json"`.
    #[napi(getter, js_name = "type")]
    pub fn type_(&self) -> String {
        self.inner.type_().to_string()
    }

    /// The subtype, e.g. `"json"` of `"application/json"`.
    #[napi(getter)]
    pub fn subtype(&self) -> String {
        self.inner.subtype().to_string()
    }

    /// The known file extensions (lowercase, no dot).
    #[napi(getter)]
    pub fn extensions(&self) -> Vec<String> {
        self.inner.extensions().to_vec()
    }

    /// The magic-byte signatures a file of this type starts with, as an array of `Buffer`.
    #[napi(getter)]
    pub fn magic(&self) -> Vec<Buffer> {
        self.inner
            .magic()
            .iter()
            .map(|sig| sig.clone().into())
            .collect()
    }

    /// Whether this type is registered under `ext` (case-insensitive, leading dot ignored).
    #[napi]
    pub fn has_extension(&self, ext: String) -> bool {
        self.inner.has_extension(&ext)
    }

    /// Whether `head` (the start of a file) begins with one of this type's magic signatures.
    #[napi]
    pub fn matches_magic(&self, head: Buffer) -> bool {
        self.inner.matches_magic(head.as_ref())
    }

    /// Whether this is the `application/octet-stream` fallback.
    #[napi]
    pub fn is_octet_stream(&self) -> bool {
        self.inner.is_octet_stream()
    }

    /// An explicit copy of this media type — the cross-language name for a clone.
    #[napi]
    pub fn copy(&self) -> MimeType {
        Self {
            inner: self.inner.clone(),
        }
    }

    /// The value form — the **essence bytes** (the mime string). Extensions and magic are
    /// catalog metadata, not part of the byte identity.
    #[napi]
    pub fn serialize_bytes(&self) -> Buffer {
        Serializable::serialize_bytes(&self.inner).into()
    }

    /// Reconstructs a media type from the essence bytes produced by `serializeBytes` (an
    /// essence-only value, no extensions or magic), throwing a guided `Error` on non-UTF-8
    /// bytes or a bad essence.
    #[napi(factory)]
    pub fn deserialize_bytes(data: Buffer) -> napi::Result<MimeType> {
        <core::MimeType as Serializable>::deserialize_bytes(data.as_ref())
            .map(|inner| Self { inner })
            .map_err(to_error)
    }

    /// Content equality (equal iff the essences are equal).
    #[napi]
    pub fn equals(&self, other: &MimeType) -> bool {
        self.inner == other.inner
    }

    /// Java-style `i32` content hash.
    #[napi]
    pub fn hash_code(&self) -> i32 {
        java_hash(&self.inner)
    }

    /// The essence string, e.g. `"application/json"`.
    #[napi(js_name = "toString")]
    pub fn text(&self) -> String {
        self.inner.to_string()
    }
}

/// A registry of known [`MimeType`]s — resolves a `MimeType` from a mime string, a file name,
/// an extension, or the magic bytes of a file's head. Small and linearly scanned; seed it with
/// the built-in known types via [`defaults`](MimeCatalog::defaults) or start empty and
/// [`register`](MimeCatalog::register) your own.
#[napi(namespace = "mimetype")]
#[derive(Default)]
pub struct MimeCatalog {
    pub(crate) inner: core::MimeCatalog,
}

#[napi(namespace = "mimetype")]
impl MimeCatalog {
    /// An empty catalog.
    #[napi(constructor)]
    pub fn new() -> Self {
        Self {
            inner: core::MimeCatalog::new(),
        }
    }

    /// A catalog seeded with the **built-in known types** — the common web / data / archive /
    /// image formats, with their extensions and (where distinctive) magic signatures.
    #[napi(factory)]
    pub fn defaults() -> MimeCatalog {
        Self {
            inner: core::MimeCatalog::defaults(),
        }
    }

    /// Registers `mime`, overriding any earlier entry with the same essence (later registration
    /// wins). In-place; `with` is the chainable, non-mutating builder.
    #[napi]
    pub fn register(&mut self, mime: &MimeType) {
        self.inner.register(mime.inner.clone());
    }

    /// Returns a copy of this catalog with `mime` registered — the chainable, non-mutating
    /// builder (`catalog.with(a).with(b)`).
    #[napi]
    pub fn with(&self, mime: &MimeType) -> MimeCatalog {
        Self {
            inner: self.inner.clone().with(mime.inner.clone()),
        }
    }

    /// The registered types, in registration order.
    #[napi]
    pub fn types(&self) -> Vec<MimeType> {
        self.inner
            .types()
            .iter()
            .map(|inner| MimeType {
                inner: inner.clone(),
            })
            .collect()
    }

    /// The number of registered types — an `i64` (a JS number).
    #[napi]
    pub fn len(&self) -> i64 {
        self.inner.len() as i64
    }

    /// Whether the catalog has no registered types.
    #[napi]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// The registered type whose essence equals the parsed mime string, or `null`.
    #[napi]
    pub fn from_mime(&self, mime: String) -> Option<MimeType> {
        self.inner.from_mime(&mime).map(|inner| MimeType { inner })
    }

    /// The registered type known by `ext` (no dot, case-insensitive), or `null`.
    #[napi]
    pub fn from_extension(&self, ext: String) -> Option<MimeType> {
        self.inner
            .from_extension(&ext)
            .map(|inner| MimeType { inner })
    }

    /// The registered type for a **file name** — its last extension is looked up. `null` when
    /// the name has no extension or the extension is unknown.
    #[napi]
    pub fn from_name(&self, name: String) -> Option<MimeType> {
        self.inner.from_name(&name).map(|inner| MimeType { inner })
    }

    /// The registered type whose magic signature prefixes `head` (longest wins), or `null`.
    #[napi]
    pub fn from_magic(&self, head: Buffer) -> Option<MimeType> {
        self.inner
            .from_magic(head.as_ref())
            .map(|inner| MimeType { inner })
    }

    /// An explicit copy of this catalog — the cross-language name for a clone.
    #[napi]
    pub fn copy(&self) -> MimeCatalog {
        Self {
            inner: self.inner.clone(),
        }
    }

    /// A short debug string of the form `MimeCatalog(<N types>)`.
    #[napi(js_name = "toString")]
    pub fn text(&self) -> String {
        format!("MimeCatalog(<{} types>)", self.inner.len())
    }
}
