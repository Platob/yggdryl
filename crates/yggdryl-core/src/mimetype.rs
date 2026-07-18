//! [`MimeType`] — one media type (`type/subtype`) with its known extensions and magic-byte
//! signatures, plus the [`MimeRegistry`] trait and the [`MimeCatalog`] of known types that
//! resolves a `MimeType` from a **mime string**, a **file name**, an **extension**, or the
//! **magic bytes** of a file's head.

use core::fmt;

use crate::io::{IoError, Serializable};

/// One media type: a lowercased `type/subtype` **essence** (the mime string without
/// parameters), the file **extensions** it is known by, and the **magic-byte** signatures a
/// file of this type begins with. A value type — equal, hashable, and byte-serializable.
///
/// ```
/// use yggdryl_core::mimetype::MimeType;
///
/// let json = MimeType::parse_str("application/json; charset=utf-8").unwrap();
/// assert_eq!(json.essence(), "application/json"); // parameters dropped
/// assert_eq!(json.type_(), "application");
/// assert_eq!(json.subtype(), "json");
///
/// // Resolve from a file name / extension / magic bytes via the default catalog.
/// assert_eq!(MimeType::from_name("report.pdf").unwrap().essence(), "application/pdf");
/// assert_eq!(MimeType::from_extension("png").unwrap().essence(), "image/png");
/// assert_eq!(MimeType::from_magic(b"\x89PNG\r\n\x1a\n...").unwrap().essence(), "image/png");
/// ```
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct MimeType {
    /// The lowercased `type/subtype` essence, e.g. `"application/json"`.
    essence: String,
    /// Short **names** / aliases the type is known by, e.g. `["gzip"]` for `application/gzip`.
    names: Vec<String>,
    /// The known file extensions (lowercase, no dot). The first is the **primary**
    /// ([`extension`](MimeType::extension)).
    extensions: Vec<String>,
    /// The magic-byte prefixes a file of this type starts with (any one matches).
    magic: Vec<Vec<u8>>,
}

impl MimeType {
    /// The universal fallback essence — an opaque byte stream of unknown type.
    pub const OCTET_STREAM: &'static str = "application/octet-stream";

    // ---- construction -------------------------------------------------------------------

    /// A media type from its `essence` (`type/subtype`), known `extensions` (no dot), and
    /// `magic` signatures. The essence is lowercased; extensions are lowercased and stripped
    /// of a leading dot.
    pub fn new(
        essence: impl Into<String>,
        extensions: impl IntoIterator<Item = impl Into<String>>,
        magic: impl IntoIterator<Item = Vec<u8>>,
    ) -> MimeType {
        MimeType::named(essence, Vec::<String>::new(), extensions, magic)
    }

    /// A media type with explicit **names** as well as extensions and magic. Essence and names
    /// are lowercased; extensions are lowercased and lose a leading dot.
    pub fn named(
        essence: impl Into<String>,
        names: impl IntoIterator<Item = impl Into<String>>,
        extensions: impl IntoIterator<Item = impl Into<String>>,
        magic: impl IntoIterator<Item = Vec<u8>>,
    ) -> MimeType {
        MimeType {
            essence: essence.into().trim().to_ascii_lowercase(),
            names: names
                .into_iter()
                .map(|n| n.into().trim().to_ascii_lowercase())
                .collect(),
            extensions: extensions
                .into_iter()
                .map(|ext| ext.into().trim_start_matches('.').to_ascii_lowercase())
                .collect(),
            magic: magic.into_iter().collect(),
        }
    }

    /// The `application/octet-stream` fallback — an opaque byte stream.
    pub fn octet_stream() -> MimeType {
        MimeType::new(Self::OCTET_STREAM, Vec::<String>::new(), [])
    }

    /// Parses a mime string (`type/subtype` with optional `;`-separated parameters, which are
    /// dropped), returning its **essence** with no extensions or magic. Case-insensitive.
    ///
    /// # Errors
    /// [`IoError::UnknownName`] when the string is not a `type/subtype` essence (missing or
    /// empty `type` / `subtype`, or more than one `/`).
    ///
    /// ```
    /// use yggdryl_core::mimetype::MimeType;
    ///
    /// assert_eq!(MimeType::parse_str("Text/HTML").unwrap().essence(), "text/html");
    /// assert!(MimeType::parse_str("notamime").is_err());
    /// ```
    pub fn parse_str(s: &str) -> Result<MimeType, IoError> {
        let essence = s.split(';').next().unwrap_or("").trim();
        let (type_, subtype) = essence
            .split_once('/')
            .filter(|(t, s)| !t.is_empty() && !s.is_empty() && !s.contains('/'))
            .ok_or_else(|| IoError::UnknownName {
                kind: "MimeType",
                input: s.to_string(),
                expected: "a type/subtype essence, e.g. text/plain or application/json",
            })?;
        Ok(MimeType::new(
            format!("{type_}/{subtype}"),
            Vec::<String>::new(),
            [],
        ))
    }

    // ---- accessors ----------------------------------------------------------------------

    /// The `type/subtype` essence, e.g. `"application/json"`.
    pub fn essence(&self) -> &str {
        &self.essence
    }

    /// The top-level type, e.g. `"application"` of `"application/json"`.
    pub fn type_(&self) -> &str {
        self.essence.split_once('/').map_or("", |(t, _)| t)
    }

    /// The subtype, e.g. `"json"` of `"application/json"`.
    pub fn subtype(&self) -> &str {
        self.essence.split_once('/').map_or("", |(_, s)| s)
    }

    /// The known file extensions (lowercase, no dot).
    pub fn extensions(&self) -> &[String] {
        &self.extensions
    }

    /// The **primary** extension (the first), or `None` when the type has none.
    pub fn extension(&self) -> Option<&str> {
        self.extensions.first().map(String::as_str)
    }

    /// The short names / aliases this type is known by (`["gzip"]` for `application/gzip`).
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// The magic-byte signatures a file of this type starts with.
    pub fn magic(&self) -> &[Vec<u8>] {
        &self.magic
    }

    /// Whether this type is a **compression** format (gzip / zstd / xz-lzma / zlib) — the
    /// accessor that decides whether a source's bytes can be run through a
    /// [`Compression`](crate::compression::Compression) codec.
    ///
    /// ```
    /// use yggdryl_core::mimetype::MimeType;
    ///
    /// assert!(MimeType::from_extension("gz").unwrap().is_compression());
    /// assert!(!MimeType::from_extension("json").unwrap().is_compression());
    /// ```
    pub fn is_compression(&self) -> bool {
        COMPRESSION_ESSENCES.contains(&self.essence.as_str())
    }

    /// Whether this type is registered under `ext` (case-insensitive, leading dot ignored).
    pub fn has_extension(&self, ext: &str) -> bool {
        let ext = ext.trim_start_matches('.');
        self.extensions.iter().any(|e| e.eq_ignore_ascii_case(ext))
    }

    /// Whether `head` (the start of a file) begins with one of this type's magic signatures.
    pub fn matches_magic(&self, head: &[u8]) -> bool {
        self.magic.iter().any(|sig| head.starts_with(sig))
    }

    /// Whether this is the `application/octet-stream` fallback.
    pub fn is_octet_stream(&self) -> bool {
        self.essence == Self::OCTET_STREAM
    }

    // ---- default-catalog resolution (the ergonomic front door) --------------------------

    /// Resolves a media type from a file **extension** (no dot) via the default catalog, or
    /// `None` if unknown. See [`MimeRegistry::from_extension`].
    pub fn from_extension(ext: &str) -> Option<MimeType> {
        default_catalog().from_extension(ext)
    }

    /// Resolves a media type from a **file name** (its last extension) via the default
    /// catalog, or `None`. See [`MimeRegistry::from_name`].
    pub fn from_name(name: &str) -> Option<MimeType> {
        default_catalog().from_name(name)
    }

    /// Resolves a media type from a short **name** / alias (`"gzip"`) via the default catalog,
    /// or `None`. See [`MimeRegistry::from_alias`].
    pub fn from_alias(name: &str) -> Option<MimeType> {
        default_catalog().from_alias(name)
    }

    /// Resolves a media type from the **magic bytes** at the start of a file via the default
    /// catalog, or `None`. See [`MimeRegistry::from_magic`].
    pub fn from_magic(head: &[u8]) -> Option<MimeType> {
        default_catalog().from_magic(head)
    }

    /// The **best guess** for a file `name` (with optional `head` bytes): magic bytes win when
    /// they match, then the name's extension, else [`octet_stream`](MimeType::octet_stream).
    /// The one call the io layer uses to infer a type it always has an answer for.
    pub fn guess(name: &str, head: &[u8]) -> MimeType {
        MimeType::from_magic(head)
            .or_else(|| MimeType::from_name(name))
            .unwrap_or_else(MimeType::octet_stream)
    }
}

impl fmt::Display for MimeType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.essence)
    }
}

impl fmt::Debug for MimeType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MimeType({})", self.essence)
    }
}

/// The value form of a media type is its **essence bytes** — the mime string. Extensions and
/// magic are catalog metadata; two `MimeType`s with the same essence are the same type.
impl Serializable for MimeType {
    type Error = IoError;

    fn serialize_bytes(&self) -> Vec<u8> {
        self.essence.clone().into_bytes()
    }

    fn deserialize_bytes(bytes: &[u8]) -> Result<Self, IoError> {
        let s = core::str::from_utf8(bytes).map_err(|e| IoError::InvalidUtf8 {
            position: e.valid_up_to(),
        })?;
        MimeType::parse_str(s)
    }
}

/// The **resolution contract** over a set of known media types: look one up from a mime
/// string, a file name, an extension, or the magic bytes at the head of a file. A family of
/// types plugs in by implementing this (the built-in [`MimeCatalog`] is the default); the io
/// layer and `Uri` resolve through it.
// The `from_*` lookups intentionally take `&self` — they resolve *from* a key *against this
// registry* (a map-style accessor), not a constructor.
#[allow(clippy::wrong_self_convention)]
pub trait MimeRegistry {
    /// The registered type whose essence equals the parsed mime string, or `None`.
    fn from_mime(&self, mime: &str) -> Option<MimeType>;

    /// The registered type known by `ext` (no dot, case-insensitive), or `None`.
    fn from_extension(&self, ext: &str) -> Option<MimeType>;

    /// The registered type known by a short **name** / alias (`"gzip"`), or `None`. The
    /// default has no name index (returns `None`); a real catalog matches the
    /// [`names`](MimeType::names) field.
    fn from_alias(&self, name: &str) -> Option<MimeType> {
        let _ = name;
        None
    }

    /// The registered type for a **file name** — its last extension is looked up. `None` when
    /// the name has no extension or the extension is unknown.
    fn from_name(&self, name: &str) -> Option<MimeType> {
        from_name_extension(name).and_then(|ext| self.from_extension(ext))
    }

    /// The registered type whose magic signature prefixes `head` (longest signature wins), or
    /// `None`.
    fn from_magic(&self, head: &[u8]) -> Option<MimeType>;
}

/// A registry of known [`MimeType`]s — the default [`MimeRegistry`] implementation. Small and
/// linearly scanned (like [`Headers`](crate::headers::Headers)); a real catalog is a few dozen
/// entries, where a linear scan beats hashing and keeps registration order. Extend it with
/// [`register`](MimeCatalog::register).
#[derive(Clone, Debug, Default)]
pub struct MimeCatalog {
    types: Vec<MimeType>,
}

impl MimeCatalog {
    /// An empty catalog.
    pub fn new() -> MimeCatalog {
        MimeCatalog { types: Vec::new() }
    }

    /// Registers `mime`, overriding any earlier entry with the same essence (so a later
    /// registration wins). Chainable via [`with`](MimeCatalog::with).
    pub fn register(&mut self, mime: MimeType) {
        self.types.retain(|m| m.essence != mime.essence);
        self.types.push(mime);
    }

    /// [`register`](MimeCatalog::register) as a chainable builder.
    pub fn with(mut self, mime: MimeType) -> MimeCatalog {
        self.register(mime);
        self
    }

    /// The registered types, in registration order.
    pub fn types(&self) -> &[MimeType] {
        &self.types
    }

    /// The number of registered types.
    pub fn len(&self) -> usize {
        self.types.len()
    }

    /// Whether the catalog is empty.
    pub fn is_empty(&self) -> bool {
        self.types.is_empty()
    }

    /// A catalog seeded with the **built-in known types** — the common web / data / archive /
    /// image formats, with their extensions and (where distinctive) magic signatures.
    pub fn defaults() -> MimeCatalog {
        let mut catalog = MimeCatalog::new();
        for (essence, names, exts, magic) in KNOWN {
            catalog.register(MimeType::named(
                *essence,
                names.iter().copied(),
                exts.iter().copied(),
                magic.iter().map(|m| m.to_vec()),
            ));
        }
        catalog
    }
}

impl MimeRegistry for MimeCatalog {
    fn from_mime(&self, mime: &str) -> Option<MimeType> {
        let parsed = MimeType::parse_str(mime).ok()?;
        self.types
            .iter()
            .find(|m| m.essence == parsed.essence)
            .cloned()
            .or(Some(parsed))
    }

    fn from_extension(&self, ext: &str) -> Option<MimeType> {
        self.types.iter().find(|m| m.has_extension(ext)).cloned()
    }

    fn from_alias(&self, name: &str) -> Option<MimeType> {
        self.types
            .iter()
            .find(|m| m.names.iter().any(|n| n.eq_ignore_ascii_case(name)))
            .cloned()
    }

    fn from_magic(&self, head: &[u8]) -> Option<MimeType> {
        // Longest matching signature wins, so a specific prefix beats a shorter shared one.
        self.types
            .iter()
            .filter_map(|m| {
                m.magic
                    .iter()
                    .filter(|sig| head.starts_with(sig))
                    .map(|sig| sig.len())
                    .max()
                    .map(|len| (len, m))
            })
            .max_by_key(|(len, _)| *len)
            .map(|(_, m)| m.clone())
    }
}

/// The process-wide default catalog of known media types (built once, lazily). The ergonomic
/// [`MimeType::from_extension`] / [`from_name`](MimeType::from_name) /
/// [`from_magic`](MimeType::from_magic) resolve through it.
pub fn default_catalog() -> &'static MimeCatalog {
    static DEFAULT: std::sync::LazyLock<MimeCatalog> =
        std::sync::LazyLock::new(MimeCatalog::defaults);
    &DEFAULT
}

/// The last extension of a file name (lowercase-insensitive slice), ignoring a leading-dot
/// hidden file — the shared "name → extension" step of [`MimeRegistry::from_name`].
fn from_name_extension(name: &str) -> Option<&str> {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    match base.rfind('.') {
        Some(i) if i > 0 && i + 1 < base.len() => Some(&base[i + 1..]),
        _ => None,
    }
}

/// The essences that are **compression** formats — [`MimeType::is_compression`] and the
/// [`Compression`](crate::compression::Compression) codec resolution key off this list.
pub(crate) const COMPRESSION_ESSENCES: &[&str] = &[
    "application/gzip",
    "application/zstd",
    "application/x-xz",
    "application/x-lzma",
    "application/zlib",
];

/// The built-in known types: `(essence, names, extensions, magic signatures)`. Deliberately
/// compact — the common web, data, archive, image, and **compression** formats.
#[allow(clippy::type_complexity)]
const KNOWN: &[(&str, &[&str], &[&str], &[&[u8]])] = &[
    ("text/plain", &["text"], &["txt", "text", "log"], &[]),
    ("text/html", &["html"], &["html", "htm"], &[]),
    ("text/css", &["css"], &["css"], &[]),
    ("text/csv", &["csv"], &["csv"], &[]),
    ("text/markdown", &["markdown"], &["md", "markdown"], &[]),
    ("application/json", &["json"], &["json"], &[]),
    ("application/xml", &["xml"], &["xml"], &[]),
    (
        "application/javascript",
        &["javascript"],
        &["js", "mjs"],
        &[],
    ),
    ("application/pdf", &["pdf"], &["pdf"], &[b"%PDF-"]),
    (
        "application/zip",
        &["zip"],
        &["zip"],
        &[b"PK\x03\x04", b"PK\x05\x06"],
    ),
    ("application/x-tar", &["tar"], &["tar"], &[]),
    ("application/wasm", &["wasm"], &["wasm"], &[b"\x00asm"]),
    (
        "application/vnd.apache.parquet",
        &["parquet"],
        &["parquet"],
        &[b"PAR1"],
    ),
    (
        "application/vnd.apache.arrow.file",
        &["arrow"],
        &["arrow"],
        &[b"ARROW1"],
    ),
    ("image/png", &["png"], &["png"], &[b"\x89PNG\r\n\x1a\n"]),
    (
        "image/jpeg",
        &["jpeg"],
        &["jpg", "jpeg"],
        &[b"\xff\xd8\xff"],
    ),
    ("image/gif", &["gif"], &["gif"], &[b"GIF87a", b"GIF89a"]),
    ("image/webp", &["webp"], &["webp"], &[]),
    // Compression formats (see COMPRESSION_ESSENCES + the compression codecs).
    ("application/gzip", &["gzip", "gz"], &["gz"], &[b"\x1f\x8b"]),
    (
        "application/zstd",
        &["zstd", "zst"],
        &["zst"],
        &[b"\x28\xb5\x2f\xfd"],
    ),
    (
        "application/x-xz",
        &["xz", "lzma"],
        &["xz"],
        &[b"\xfd7zXZ\x00"],
    ),
    (
        "application/x-lzma",
        &["lzma-alone"],
        &["lzma"],
        &[b"\x5d\x00\x00"],
    ),
    (
        "application/zlib",
        &["zlib"],
        &["zz"],
        &[b"\x78\x9c", b"\x78\x01", b"\x78\xda"],
    ),
    (
        "application/octet-stream",
        &["octet-stream", "binary"],
        &["bin"],
        &[],
    ),
];
