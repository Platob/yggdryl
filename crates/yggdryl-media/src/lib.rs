//! # yggdryl-media
//!
//! Media (MIME) type detection for the **yggdryl** project, built on the
//! [`yggdryl-core`](https://crates.io/crates/yggdryl-core) parsing traits.
//!
//! - [`MimeType`] is an enum of common, individual MIME types (with an
//!   [`Other`](MimeType::Other) escape hatch). Each type's canonical string, file
//!   extensions and magic-byte signatures live in a **global registry** that can
//!   be extended or trimmed at runtime ([`MimeType::register`] /
//!   [`MimeType::unregister`]).
//! - [`MediaType`] is an ordered stack of [`MimeType`]s describing a layered
//!   file, so `data.csv.gz` becomes `MediaType([MimeType::Csv, MimeType::Gzip])`.
//!
//! ```
//! use yggdryl_media::{FromInput, MediaType, MimeType};
//!
//! assert_eq!(MimeType::from_str("application/json", true).unwrap(), MimeType::Json);
//! assert_eq!(MimeType::from_extension("parquet"), Some(MimeType::Parquet));
//! assert_eq!(MimeType::from_magic(b"PK\x03\x04..."), Some(MimeType::Zip));
//!
//! let stack = MediaType::from_path("data.csv.gz");
//! assert_eq!(stack.types(), [MimeType::Csv, MimeType::Gzip]);
//! ```

use std::fmt;
use std::sync::{OnceLock, RwLock};

pub use yggdryl_core::{FromInput, Mapping, ToOutput};

/// Error returned when a media or MIME type cannot be interpreted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaError {
    /// The input was empty.
    Empty,
    /// The input was not a `type/subtype` MIME form.
    Invalid(String),
}

impl fmt::Display for MediaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MediaError::Empty => write!(f, "media type is empty"),
            MediaError::Invalid(value) => write!(f, "media type '{value}' is not 'type/subtype'"),
        }
    }
}

impl std::error::Error for MediaError {}

/// A single common MIME type, or [`Other`](MimeType::Other) for anything not in
/// the built-in registry.
///
/// Each variant maps to a canonical MIME string; the associated file extensions
/// and magic-byte signatures are held in the runtime registry (see
/// [`MimeType::extensions`], [`MimeType::from_magic`], [`MimeType::register`]).
///
/// ```
/// use yggdryl_media::MimeType;
///
/// let png = MimeType::Png;
/// assert_eq!(png.mime(), "image/png");
/// assert_eq!(png.type_(), "image");
/// assert_eq!(png.extension(), Some("png".to_string()));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MimeType {
    // text/*
    /// `text/plain`
    Plain,
    /// `text/html`
    Html,
    /// `text/css`
    Css,
    /// `text/csv`
    Csv,
    /// `text/markdown`
    Markdown,
    /// `text/javascript`
    JavaScript,
    // application/*
    /// `application/json`
    Json,
    /// `application/xml`
    Xml,
    /// `application/pdf`
    Pdf,
    /// `application/zip`
    Zip,
    /// `application/gzip`
    Gzip,
    /// `application/x-tar`
    Tar,
    /// `application/x-bzip2`
    Bzip2,
    /// `application/zstd`
    Zstd,
    /// `application/x-7z-compressed`
    SevenZip,
    /// `application/vnd.apache.parquet`
    Parquet,
    /// `application/vnd.apache.arrow.file`
    Arrow,
    /// `application/vnd.apache.avro`
    Avro,
    /// `application/wasm`
    Wasm,
    /// `application/vnd.sqlite3`
    Sqlite,
    /// `application/octet-stream`
    OctetStream,
    // image/*
    /// `image/png`
    Png,
    /// `image/jpeg`
    Jpeg,
    /// `image/gif`
    Gif,
    /// `image/webp`
    Webp,
    /// `image/bmp`
    Bmp,
    /// `image/svg+xml`
    Svg,
    /// `image/x-icon`
    Icon,
    /// `image/tiff`
    Tiff,
    // audio/*
    /// `audio/mpeg`
    Mp3,
    /// `audio/wav`
    Wav,
    /// `audio/flac`
    Flac,
    /// `audio/ogg`
    Ogg,
    // video/*
    /// `video/mp4`
    Mp4,
    /// `video/webm`
    Webm,
    /// `video/x-msvideo`
    Avi,
    // font/*
    /// `font/woff`
    Woff,
    /// `font/woff2`
    Woff2,
    /// `font/ttf`
    Ttf,
    /// `font/otf`
    Otf,
    /// Any MIME type outside the built-in registry, holding its `type/subtype`.
    Other(String),
}

/// A magic-byte signature: `bytes` expected at a fixed `offset` from the start of
/// a file. Used to build registry entries for [`MimeType::register`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    offset: usize,
    bytes: Vec<u8>,
}

impl Signature {
    /// A signature matched at the very start of the data.
    pub fn prefix(bytes: impl Into<Vec<u8>>) -> Signature {
        Signature {
            offset: 0,
            bytes: bytes.into(),
        }
    }

    /// A signature matched at a fixed `offset` from the start.
    pub fn at(offset: usize, bytes: impl Into<Vec<u8>>) -> Signature {
        Signature {
            offset,
            bytes: bytes.into(),
        }
    }

    /// Whether `data` carries this signature at its offset.
    fn matches(&self, data: &[u8]) -> bool {
        let end = self.offset + self.bytes.len();
        data.len() >= end && data[self.offset..end] == self.bytes[..]
    }
}

/// A static magic-byte signature, used only to seed [`BUILTINS`].
struct Magic {
    offset: usize,
    bytes: &'static [u8],
}

/// One built-in registry row: a constructor for the [`MimeType`], its canonical
/// MIME string, default file extensions (first is canonical) and magic bytes.
struct Builtin {
    new: fn() -> MimeType,
    mime: &'static str,
    extensions: &'static [&'static str],
    magic: &'static [Magic],
}

/// Shorthand for one built-in row.
const fn builtin(
    new: fn() -> MimeType,
    mime: &'static str,
    extensions: &'static [&'static str],
    magic: &'static [Magic],
) -> Builtin {
    Builtin {
        new,
        mime,
        extensions,
        magic,
    }
}

/// Shorthand for a static signature at the start of the data.
const fn mag(bytes: &'static [u8]) -> Magic {
    Magic { offset: 0, bytes }
}

/// Shorthand for a static signature at a fixed offset.
const fn mag_at(offset: usize, bytes: &'static [u8]) -> Magic {
    Magic { offset, bytes }
}

/// The built-in defaults that seed the runtime registry — the single source of
/// truth for each known [`MimeType`]'s MIME string, extensions and magic bytes.
static BUILTINS: &[Builtin] = &[
    // text/*
    builtin(
        || MimeType::Plain,
        "text/plain",
        &["txt", "text", "log"],
        &[],
    ),
    builtin(|| MimeType::Html, "text/html", &["html", "htm"], &[]),
    builtin(|| MimeType::Css, "text/css", &["css"], &[]),
    builtin(|| MimeType::Csv, "text/csv", &["csv"], &[]),
    builtin(
        || MimeType::Markdown,
        "text/markdown",
        &["md", "markdown"],
        &[],
    ),
    builtin(
        || MimeType::JavaScript,
        "text/javascript",
        &["js", "mjs"],
        &[],
    ),
    // application/*
    builtin(|| MimeType::Json, "application/json", &["json"], &[]),
    builtin(
        || MimeType::Xml,
        "application/xml",
        &["xml"],
        &[mag(b"<?xml")],
    ),
    builtin(
        || MimeType::Pdf,
        "application/pdf",
        &["pdf"],
        &[mag(b"%PDF-")],
    ),
    builtin(
        || MimeType::Zip,
        "application/zip",
        &["zip"],
        &[mag(b"PK\x03\x04")],
    ),
    builtin(
        || MimeType::Gzip,
        "application/gzip",
        &["gz", "gzip"],
        &[mag(b"\x1f\x8b")],
    ),
    builtin(
        || MimeType::Tar,
        "application/x-tar",
        &["tar"],
        &[mag_at(257, b"ustar")],
    ),
    builtin(
        || MimeType::Bzip2,
        "application/x-bzip2",
        &["bz2"],
        &[mag(b"BZh")],
    ),
    builtin(
        || MimeType::Zstd,
        "application/zstd",
        &["zst"],
        &[mag(b"\x28\xb5\x2f\xfd")],
    ),
    builtin(
        || MimeType::SevenZip,
        "application/x-7z-compressed",
        &["7z"],
        &[mag(b"7z\xbc\xaf\x27\x1c")],
    ),
    builtin(
        || MimeType::Parquet,
        "application/vnd.apache.parquet",
        &["parquet"],
        &[mag(b"PAR1")],
    ),
    builtin(
        || MimeType::Arrow,
        "application/vnd.apache.arrow.file",
        &["arrow", "arrows", "ipc"],
        &[mag(b"ARROW1")],
    ),
    builtin(
        || MimeType::Avro,
        "application/vnd.apache.avro",
        &["avro"],
        &[mag(b"Obj\x01")],
    ),
    builtin(
        || MimeType::Wasm,
        "application/wasm",
        &["wasm"],
        &[mag(b"\x00asm")],
    ),
    builtin(
        || MimeType::Sqlite,
        "application/vnd.sqlite3",
        &["sqlite", "sqlite3", "db"],
        &[mag(b"SQLite format 3\x00")],
    ),
    builtin(
        || MimeType::OctetStream,
        "application/octet-stream",
        &["bin"],
        &[],
    ),
    // image/*
    builtin(
        || MimeType::Png,
        "image/png",
        &["png"],
        &[mag(b"\x89PNG\r\n\x1a\n")],
    ),
    builtin(
        || MimeType::Jpeg,
        "image/jpeg",
        &["jpg", "jpeg"],
        &[mag(b"\xff\xd8\xff")],
    ),
    builtin(
        || MimeType::Gif,
        "image/gif",
        &["gif"],
        &[mag(b"GIF87a"), mag(b"GIF89a")],
    ),
    builtin(
        || MimeType::Webp,
        "image/webp",
        &["webp"],
        &[mag_at(8, b"WEBP")],
    ),
    builtin(|| MimeType::Bmp, "image/bmp", &["bmp"], &[mag(b"BM")]),
    builtin(|| MimeType::Svg, "image/svg+xml", &["svg"], &[]),
    builtin(
        || MimeType::Icon,
        "image/x-icon",
        &["ico"],
        &[mag(b"\x00\x00\x01\x00")],
    ),
    builtin(
        || MimeType::Tiff,
        "image/tiff",
        &["tif", "tiff"],
        &[mag(b"II\x2a\x00"), mag(b"MM\x00\x2a")],
    ),
    // audio/*
    builtin(|| MimeType::Mp3, "audio/mpeg", &["mp3"], &[mag(b"ID3")]),
    builtin(
        || MimeType::Wav,
        "audio/wav",
        &["wav"],
        &[mag_at(8, b"WAVE")],
    ),
    builtin(|| MimeType::Flac, "audio/flac", &["flac"], &[mag(b"fLaC")]),
    builtin(
        || MimeType::Ogg,
        "audio/ogg",
        &["ogg", "oga"],
        &[mag(b"OggS")],
    ),
    // video/*
    builtin(
        || MimeType::Mp4,
        "video/mp4",
        &["mp4", "m4v"],
        &[mag_at(4, b"ftyp")],
    ),
    builtin(
        || MimeType::Webm,
        "video/webm",
        &["webm"],
        &[mag(b"\x1a\x45\xdf\xa3")],
    ),
    builtin(
        || MimeType::Avi,
        "video/x-msvideo",
        &["avi"],
        &[mag_at(8, b"AVI ")],
    ),
    // font/*
    builtin(|| MimeType::Woff, "font/woff", &["woff"], &[mag(b"wOFF")]),
    builtin(
        || MimeType::Woff2,
        "font/woff2",
        &["woff2"],
        &[mag(b"wOF2")],
    ),
    builtin(
        || MimeType::Ttf,
        "font/ttf",
        &["ttf"],
        &[mag(b"\x00\x01\x00\x00")],
    ),
    builtin(|| MimeType::Otf, "font/otf", &["otf"], &[mag(b"OTTO")]),
];

/// One mutable registry entry: everything known about one MIME type.
#[derive(Clone)]
struct Entry {
    mime: String,
    extensions: Vec<String>,
    magic: Vec<Signature>,
}

impl Entry {
    /// Materialises a mutable entry from a built-in default.
    fn from_builtin(b: &Builtin) -> Entry {
        Entry {
            mime: b.mime.to_string(),
            extensions: b.extensions.iter().map(|s| s.to_string()).collect(),
            magic: b
                .magic
                .iter()
                .map(|m| Signature::at(m.offset, m.bytes))
                .collect(),
        }
    }
}

/// The process-global registry, seeded from [`BUILTINS`] on first use.
static REGISTRY: OnceLock<RwLock<Vec<Entry>>> = OnceLock::new();

/// Returns the global registry, initialising it with the built-in defaults.
fn registry() -> &'static RwLock<Vec<Entry>> {
    REGISTRY.get_or_init(|| RwLock::new(BUILTINS.iter().map(Entry::from_builtin).collect()))
}

/// Returns `true` for an [RFC 2045](https://www.rfc-editor.org/rfc/rfc2045) token.
fn is_token(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|b| {
            b.is_ascii_alphanumeric()
                || matches!(
                    b,
                    b'!' | b'#' | b'$' | b'&' | b'-' | b'^' | b'_' | b'.' | b'+'
                )
        })
}

/// Returns `true` if `mime` is a `type/subtype` pair of valid tokens.
fn is_valid_mime(mime: &str) -> bool {
    match mime.split_once('/') {
        Some((type_, subtype)) => is_token(type_) && is_token(subtype),
        None => false,
    }
}

impl MimeType {
    /// The built-in default for a known variant, or `None` for [`Other`](MimeType::Other).
    fn builtin(&self) -> Option<&'static Builtin> {
        if matches!(self, MimeType::Other(_)) {
            return None;
        }
        BUILTINS.iter().find(|b| (b.new)() == *self)
    }

    /// Looks up a [`MimeType`] by its (case-insensitive) MIME string, falling back
    /// to [`Other`](MimeType::Other) for anything not built in.
    pub fn from_mime(mime: &str) -> MimeType {
        let mime = mime.to_ascii_lowercase();
        BUILTINS
            .iter()
            .find(|b| b.mime == mime)
            .map(|b| (b.new)())
            .unwrap_or(MimeType::Other(mime))
    }

    /// Infers a [`MimeType`] from a file `extension` (with or without a leading
    /// `.`) via the registry, or `None` if it is not registered.
    pub fn from_extension(extension: &str) -> Option<MimeType> {
        let ext = extension.trim_start_matches('.').to_ascii_lowercase();
        let registry = registry().read().unwrap();
        registry
            .iter()
            .find(|e| e.extensions.contains(&ext))
            .map(|e| MimeType::from_mime(&e.mime))
    }

    /// Infers a [`MimeType`] from a file's leading bytes by matching the registry's
    /// magic-byte signatures, or `None` if none match. Recognises container and
    /// columnar formats such as Arrow IPC, Parquet, ZIP and gzip.
    pub fn from_magic(data: &[u8]) -> Option<MimeType> {
        let registry = registry().read().unwrap();
        registry
            .iter()
            .find(|e| e.magic.iter().any(|s| s.matches(data)))
            .map(|e| MimeType::from_mime(&e.mime))
    }

    /// Infers the outermost [`MimeType`] from a path's last known file extension,
    /// e.g. `Gzip` for `data.csv.gz`, or `None` if none is known. For the full
    /// layered view use [`MediaType::from_path`].
    pub fn from_path(path: &str) -> Option<MimeType> {
        MediaType::from_path(path).last().cloned()
    }

    /// The canonical MIME string, e.g. `"image/png"`. For
    /// [`Other`](MimeType::Other) this is the stored `type/subtype`.
    pub fn mime(&self) -> &str {
        match self {
            MimeType::Other(mime) => mime,
            _ => self.builtin().map(|b| b.mime).unwrap_or(""),
        }
    }

    /// The top-level type, e.g. `"image"` for `image/png`.
    pub fn type_(&self) -> &str {
        self.mime().split('/').next().unwrap_or("")
    }

    /// The subtype, e.g. `"png"` for `image/png` or `"svg+xml"` for `image/svg+xml`.
    pub fn subtype(&self) -> &str {
        self.mime().split_once('/').map_or("", |(_, s)| s)
    }

    /// The file extensions registered for this type (the first is canonical); empty
    /// if the type has been unregistered or is an unknown [`Other`](MimeType::Other).
    pub fn extensions(&self) -> Vec<String> {
        let mime = self.mime();
        let registry = registry().read().unwrap();
        registry
            .iter()
            .find(|e| e.mime == mime)
            .map(|e| e.extensions.clone())
            .unwrap_or_default()
    }

    /// The canonical (first) file extension, if any.
    pub fn extension(&self) -> Option<String> {
        self.extensions().into_iter().next()
    }

    /// Whether this is a built-in variant rather than [`Other`](MimeType::Other).
    pub fn is_known(&self) -> bool {
        !matches!(self, MimeType::Other(_))
    }

    /// Registers (or replaces) a MIME type in the global registry, so subsequent
    /// [`from_extension`](MimeType::from_extension) /
    /// [`from_magic`](MimeType::from_magic) lookups recognise it. The change is
    /// process-wide.
    pub fn register(mime: &str, extensions: &[&str], magic: &[Signature]) {
        let mime = mime.to_ascii_lowercase();
        let entry = Entry {
            extensions: extensions.iter().map(|s| s.to_ascii_lowercase()).collect(),
            magic: magic.to_vec(),
            mime: mime.clone(),
        };
        let mut registry = registry().write().unwrap();
        match registry.iter_mut().find(|e| e.mime == mime) {
            Some(slot) => *slot = entry,
            None => registry.push(entry),
        }
    }

    /// Removes a MIME type from the global registry by its canonical string,
    /// returning whether an entry was present. The change is process-wide.
    pub fn unregister(mime: &str) -> bool {
        let mime = mime.to_ascii_lowercase();
        let mut registry = registry().write().unwrap();
        let before = registry.len();
        registry.retain(|e| e.mime != mime);
        registry.len() != before
    }

    /// Restores the global registry to its built-in defaults, discarding every
    /// [`register`](MimeType::register) / [`unregister`](MimeType::unregister).
    pub fn reset_registry() {
        let mut registry = registry().write().unwrap();
        *registry = BUILTINS.iter().map(Entry::from_builtin).collect();
    }
}

impl Default for MimeType {
    /// The fallback type, [`OctetStream`](MimeType::OctetStream)
    /// (`application/octet-stream`), used when no more specific type is known —
    /// e.g. `MimeType::from_path(p).unwrap_or_default()`.
    fn default() -> MimeType {
        MimeType::OctetStream
    }
}

impl FromInput for MimeType {
    type Err = MediaError;

    /// Parses a MIME string such as `"text/html"` (any `;parameters` are dropped).
    /// When `safe`, the essence must be a `type/subtype` pair of valid tokens;
    /// when not `safe`, the input is taken as-is. Unknown but well-formed types
    /// become [`Other`](MimeType::Other).
    fn from_str(input: &str, safe: bool) -> Result<MimeType, MediaError> {
        if input.is_empty() {
            return Err(MediaError::Empty);
        }
        let essence = input
            .split(';')
            .next()
            .unwrap_or(input)
            .trim()
            .to_ascii_lowercase();
        if safe && !is_valid_mime(&essence) {
            return Err(MediaError::Invalid(input.to_string()));
        }
        Ok(MimeType::from_mime(&essence))
    }

    /// Builds a [`MimeType`] from a [`Mapping`]. Recognised keys: `type` and
    /// `subtype`. When `safe`, both must be present and valid tokens.
    fn from_mapping(fields: &Mapping, safe: bool) -> Result<MimeType, MediaError> {
        let type_ = fields.get("type").map_or("", String::as_str);
        let subtype = fields.get("subtype").map_or("", String::as_str);
        let mime = format!("{type_}/{subtype}");
        if safe && !is_valid_mime(&mime) {
            return Err(MediaError::Invalid(mime));
        }
        Ok(MimeType::from_mime(&mime))
    }
}

impl fmt::Display for MimeType {
    /// Renders the canonical MIME string.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.mime())
    }
}

impl ToOutput for MimeType {
    fn to_str(&self, _encode: bool) -> String {
        self.mime().to_string()
    }

    /// The inverse of `from_mapping`: keys `type` and `subtype`.
    fn to_mapping(&self) -> Mapping {
        Mapping::from([
            ("type".to_string(), self.type_().to_string()),
            ("subtype".to_string(), self.subtype().to_string()),
        ])
    }
}

/// An ordered stack of [`MimeType`]s describing a layered file, innermost content
/// first. Parsing `data.csv.gz` yields `MediaType([MimeType::Csv, MimeType::Gzip])`.
///
/// ```
/// use yggdryl_media::{MediaType, MimeType};
///
/// let stack = MediaType::from_path("/tmp/data.csv.gz");
/// assert_eq!(stack.types(), [MimeType::Csv, MimeType::Gzip]);
/// assert_eq!(stack.first(), Some(&MimeType::Csv));
/// assert_eq!(stack.last(), Some(&MimeType::Gzip));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MediaType {
    types: Vec<MimeType>,
}

/// Splits a file name into its extensions, e.g. `"a.csv.gz"` → `["csv", "gz"]` and
/// `".bashrc"` → `[]` (a leading dot starts a dotfile, not an extension).
fn name_extensions(name: &str) -> Vec<&str> {
    let after_first_dot = if name.len() > 1 {
        name[1..].find('.').map(|i| i + 2)
    } else {
        None
    };
    match after_first_dot {
        Some(idx) => name[idx..].split('.').filter(|s| !s.is_empty()).collect(),
        None => Vec::new(),
    }
}

impl MediaType {
    /// Builds a [`MediaType`] from an ordered list of [`MimeType`]s.
    pub fn new(types: Vec<MimeType>) -> MediaType {
        MediaType { types }
    }

    /// Builds the stack from an ordered list of file extensions, keeping those that
    /// resolve in the registry (unknown extensions are skipped). `["csv", "gz"]`
    /// yields `[Csv, Gzip]`.
    pub fn from_extensions(extensions: &[&str]) -> MediaType {
        MediaType {
            types: extensions
                .iter()
                .filter_map(|ext| MimeType::from_extension(ext))
                .collect(),
        }
    }

    /// Builds a single-type stack from one file extension (empty if unknown).
    pub fn from_extension(extension: &str) -> MediaType {
        MediaType::from_extensions(&[extension])
    }

    /// Builds the stack from a path's file name, mapping each `.`-extension that
    /// resolves in the registry (unknown extensions are skipped). `data.csv.gz`
    /// yields `[Csv, Gzip]`.
    pub fn from_path(path: &str) -> MediaType {
        let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
        MediaType::from_extensions(&name_extensions(name))
    }

    /// The ordered [`MimeType`]s, innermost content first.
    pub fn types(&self) -> &[MimeType] {
        &self.types
    }

    /// The innermost (content) type, e.g. `Csv` for `data.csv.gz`.
    pub fn first(&self) -> Option<&MimeType> {
        self.types.first()
    }

    /// The outermost (container) type, e.g. `Gzip` for `data.csv.gz`.
    pub fn last(&self) -> Option<&MimeType> {
        self.types.last()
    }

    /// The number of types in the stack.
    pub fn len(&self) -> usize {
        self.types.len()
    }

    /// Whether the stack is empty (no known extension was found).
    pub fn is_empty(&self) -> bool {
        self.types.is_empty()
    }
}

impl Default for MediaType {
    /// The fallback stack: a single [`MimeType::OctetStream`]
    /// (`application/octet-stream`), used when no type can be inferred.
    fn default() -> MediaType {
        MediaType::new(vec![MimeType::default()])
    }
}

impl FromInput for MediaType {
    type Err = MediaError;

    /// Parses a path or file name into its [`MimeType`] stack (see
    /// [`from_path`](MediaType::from_path)). `safe` is accepted for trait
    /// uniformity; only an empty input is an error.
    fn from_str(input: &str, _safe: bool) -> Result<MediaType, MediaError> {
        if input.is_empty() {
            return Err(MediaError::Empty);
        }
        Ok(MediaType::from_path(input))
    }

    /// Builds the stack from a [`Mapping`]; reads the `path` key (falling back to
    /// `str`) and parses it like [`from_path`](MediaType::from_path).
    fn from_mapping(fields: &Mapping, safe: bool) -> Result<MediaType, MediaError> {
        let path = fields
            .get("path")
            .or_else(|| fields.get("str"))
            .map_or("", String::as_str);
        MediaType::from_str(path, safe)
    }
}

impl fmt::Display for MediaType {
    /// Renders the canonical extension chain, e.g. `"csv.gz"`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_str(true))
    }
}

impl ToOutput for MediaType {
    /// Renders the canonical extension chain, e.g. `"csv.gz"` (the inverse of
    /// [`from_path`](MediaType::from_path) for canonical extensions).
    fn to_str(&self, _encode: bool) -> String {
        self.types
            .iter()
            .filter_map(MimeType::extension)
            .collect::<Vec<_>>()
            .join(".")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_splits_mime() {
        let m = MimeType::from_str("application/json", true).unwrap();
        assert_eq!(m, MimeType::Json);
        assert_eq!(m.mime(), "application/json");
        assert_eq!(m.type_(), "application");
        assert_eq!(m.subtype(), "json");
        // Parameters are dropped; case is normalised.
        assert_eq!(
            MimeType::from_str("Text/HTML; charset=utf-8", true).unwrap(),
            MimeType::Html
        );
    }

    #[test]
    fn unknown_becomes_other() {
        let m = MimeType::from_str("application/x-custom", true).unwrap();
        assert_eq!(m, MimeType::Other("application/x-custom".to_string()));
        assert!(!m.is_known());
        assert_eq!(m.subtype(), "x-custom");
        assert_eq!(m.extension(), None);
    }

    #[test]
    fn errors() {
        assert_eq!(MimeType::from_str("", true), Err(MediaError::Empty));
        assert_eq!(
            MimeType::from_str("notamime", true),
            Err(MediaError::Invalid("notamime".to_string()))
        );
        assert_eq!(
            MimeType::from_str("notamime", false).unwrap(),
            MimeType::Other("notamime".to_string())
        );
    }

    #[test]
    fn from_extension_maps_known_types() {
        assert_eq!(MimeType::from_extension("parquet"), Some(MimeType::Parquet));
        assert_eq!(MimeType::from_extension(".GZ"), Some(MimeType::Gzip));
        assert_eq!(MimeType::from_extension("jpeg"), Some(MimeType::Jpeg));
        assert_eq!(MimeType::from_extension("nope"), None);
    }

    #[test]
    fn from_magic_sniffs_content() {
        assert_eq!(
            MimeType::from_magic(b"PAR1\x15\x04"),
            Some(MimeType::Parquet)
        );
        assert_eq!(
            MimeType::from_magic(b"ARROW1\x00\x00"),
            Some(MimeType::Arrow)
        );
        assert_eq!(MimeType::from_magic(b"PK\x03\x04\x14"), Some(MimeType::Zip));
        assert_eq!(
            MimeType::from_magic(b"\x1f\x8b\x08\x00"),
            Some(MimeType::Gzip)
        );
        assert_eq!(
            MimeType::from_magic(b"\x89PNG\r\n\x1a\n\x00"),
            Some(MimeType::Png)
        );
        // Offset-based signatures: tar's `ustar` lives at byte 257.
        let mut tar = vec![0u8; 270];
        tar[257..262].copy_from_slice(b"ustar");
        assert_eq!(MimeType::from_magic(&tar), Some(MimeType::Tar));
        assert_eq!(MimeType::from_magic(b"not magic"), None);
    }

    #[test]
    fn media_type_is_an_ordered_stack() {
        let stack = MediaType::from_path("data.csv.gz");
        assert_eq!(stack.types(), [MimeType::Csv, MimeType::Gzip]);
        assert_eq!(stack.first(), Some(&MimeType::Csv));
        assert_eq!(stack.last(), Some(&MimeType::Gzip));
        assert_eq!(stack.len(), 2);
        assert_eq!(stack.to_str(true), "csv.gz");
        // Unknown extensions are skipped; nested dirs and dotfiles handled.
        assert_eq!(
            MediaType::from_path("/srv/dump.bak.parquet").types(),
            [MimeType::Parquet]
        );
        assert!(MediaType::from_path("/etc/.bashrc").is_empty());
        assert!(MediaType::from_path("/usr/bin/env").is_empty());
    }

    #[test]
    fn media_type_explicit_and_round_trip() {
        let stack = MediaType::new(vec![MimeType::Csv, MimeType::Gzip]);
        assert_eq!(stack, MediaType::from_path("x.csv.gz"));
        assert_eq!(
            MediaType::from_str("a/b/c.tar.gz", true)
                .unwrap()
                .to_str(true),
            "tar.gz"
        );
    }

    #[test]
    fn defaults_to_octet_stream() {
        assert_eq!(MimeType::default(), MimeType::OctetStream);
        assert_eq!(MimeType::default().mime(), "application/octet-stream");
        assert_eq!(MediaType::default().types(), [MimeType::OctetStream]);
        // The conventional fallback for failed inference.
        assert_eq!(
            MimeType::from_extension("nope").unwrap_or_default(),
            MimeType::OctetStream
        );
        assert_eq!(
            MimeType::from_path("notes").unwrap_or_default().mime(),
            "application/octet-stream"
        );
    }

    #[test]
    fn convenient_from_constructors() {
        // MimeType: a single (outermost) type from a path.
        assert_eq!(MimeType::from_path("data.csv.gz"), Some(MimeType::Gzip));
        assert_eq!(MimeType::from_path("notes"), None);
        // MediaType: from one or many extensions, and from a mapping.
        assert_eq!(MediaType::from_extension("json").types(), [MimeType::Json]);
        assert_eq!(
            MediaType::from_extensions(&["csv", "nope", "gz"]).types(),
            [MimeType::Csv, MimeType::Gzip]
        );
        let map = Mapping::from([("path".to_string(), "report.csv.gz".to_string())]);
        assert_eq!(
            MediaType::from_(&map).unwrap(),
            MediaType::new(vec![MimeType::Csv, MimeType::Gzip])
        );
    }

    #[test]
    fn registry_add_and_remove() {
        // A custom type is unknown until registered.
        assert_eq!(MimeType::from_extension("ygg"), None);
        MimeType::register(
            "application/x-yggdryl",
            &["ygg"],
            &[Signature::prefix(b"YGG1")],
        );
        let m = MimeType::from_extension("ygg").unwrap();
        assert_eq!(m, MimeType::Other("application/x-yggdryl".to_string()));
        assert_eq!(m.extensions(), vec!["ygg".to_string()]);
        assert_eq!(
            MimeType::from_magic(b"YGG1\x00"),
            Some(MimeType::Other("application/x-yggdryl".to_string()))
        );
        // Unregistering removes it again.
        assert!(MimeType::unregister("application/x-yggdryl"));
        assert_eq!(MimeType::from_extension("ygg"), None);
        assert!(!MimeType::unregister("application/x-yggdryl"));
    }

    #[test]
    fn round_trips_through_mapping() {
        let m = MimeType::Svg;
        assert_eq!(m.mime(), "image/svg+xml");
        let map = m.to_mapping();
        assert_eq!(map.get("type"), Some(&"image".to_string()));
        assert_eq!(map.get("subtype"), Some(&"svg+xml".to_string()));
        assert_eq!(MimeType::from_(&map).unwrap(), m);
    }

    #[test]
    fn extensions_and_to_str() {
        assert_eq!(MimeType::Jpeg.extensions(), vec!["jpg", "jpeg"]);
        assert_eq!(MimeType::Jpeg.extension(), Some("jpg".to_string()));
        assert!(MimeType::Other("x/y".to_string()).extensions().is_empty());
        assert_eq!(MimeType::Gzip.to_str(true), "application/gzip");
    }
}
