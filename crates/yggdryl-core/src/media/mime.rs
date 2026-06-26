//! The [`MimeType`] enum, its [`Signature`] magic-byte type, and the mutable
//! global registry that backs extension/magic lookups.

use std::fmt;
use std::sync::{OnceLock, RwLock};

#[allow(unused_imports)]
use crate::log_event;
use crate::media::MediaType;
use crate::{Mapping, MediaError};

/// The broad role a [`MimeType`] plays, independent of its concrete `type/subtype`.
///
/// Groups types by how their bytes are consumed rather than by format: a Parquet
/// file and a CSV are both [`Tabular`](Category::Tabular), gzip and zstd are both
/// [`Codec`](Category::Codec), a Python and a Rust source file are both
/// [`Code`](Category::Code). The default is [`Blob`](Category::Blob), an opaque,
/// random-access byte holder. Each type's category lives in the same global
/// registry as its extensions/magic, so [`MimeType::register`] can set it.
///
/// ```
/// use yggdryl_core::{Category, MimeType};
///
/// assert_eq!(MimeType::Parquet.category(), Category::Tabular);
/// assert_eq!(MimeType::Gzip.category(), Category::Codec);
/// assert_eq!(MimeType::Python.category(), Category::Code);
/// assert_eq!(MimeType::Png.category(), Category::Blob);
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Category {
    /// An opaque, random-access byte holder (the default): images, audio, video,
    /// PDFs, databases, archives, fonts, …
    #[default]
    Blob,
    /// A hierarchical container of other resources (`inode/directory`).
    Directory,
    /// Row/column record data: CSV, Parquet, Arrow, Avro, NDJSON.
    Tabular,
    /// Source code, markup or configuration: a programming language, HTML/CSS,
    /// JSON/XML/YAML/TOML, Markdown, …
    Code,
    /// A compression codec: gzip, bzip2, zstd, brotli, xz, lz4.
    Codec,
}

impl Category {
    /// Parses a category name (case-insensitive), e.g. `"tabular"` →
    /// [`Tabular`](Category::Tabular). Returns [`MediaError::UnknownCategory`]
    /// otherwise.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Category, MediaError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "blob" => Ok(Category::Blob),
            "directory" | "dir" => Ok(Category::Directory),
            "tabular" | "table" => Ok(Category::Tabular),
            "code" => Ok(Category::Code),
            "codec" | "compression" => Ok(Category::Codec),
            _ => Err(MediaError::UnknownCategory(value.to_string())),
        }
    }

    /// The lowercase name (`"blob"` / `"directory"` / `"tabular"` / `"code"` /
    /// `"codec"`), used by the bindings and [`Display`](fmt::Display).
    pub fn as_str(&self) -> &'static str {
        match self {
            Category::Blob => "blob",
            Category::Directory => "directory",
            Category::Tabular => "tabular",
            Category::Code => "code",
            Category::Codec => "codec",
        }
    }
}

impl fmt::Display for Category {
    /// Renders the lowercase category name.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A single common MIME type, or [`Other`](MimeType::Other) for anything not in
/// the built-in registry.
///
/// Each variant maps to a canonical MIME string; the associated file extensions
/// and magic-byte signatures are held in the runtime registry (see
/// [`MimeType::extensions`], [`MimeType::from_magic`], [`MimeType::register`]).
///
/// ```
/// use yggdryl_core::MimeType;
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
    /// `application/x-brotli` (HTTP `Content-Encoding: br`). A raw Brotli stream
    /// carries no magic bytes, so it is recognised by extension (`.br`) only.
    Brotli,
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
    // text / data
    /// `application/yaml` (`.yaml` / `.yml`)
    Yaml,
    /// `application/toml` (`.toml`)
    Toml,
    /// `application/x-ndjson` — newline-delimited JSON (`.ndjson` / `.jsonl`)
    Ndjson,
    /// `application/rtf` (`.rtf`)
    Rtf,
    // compression / archives
    /// `application/x-xz` (`.xz`)
    Xz,
    /// `application/x-lz4` (`.lz4`)
    Lz4,
    // documents (OOXML / EPUB are ZIP containers, so recognised by extension)
    /// `application/epub+zip` (`.epub`)
    Epub,
    /// `application/vnd.openxmlformats-officedocument.wordprocessingml.document` (`.docx`)
    Docx,
    /// `application/vnd.openxmlformats-officedocument.spreadsheetml.sheet` (`.xlsx`)
    Xlsx,
    /// `application/vnd.openxmlformats-officedocument.presentationml.presentation` (`.pptx`)
    Pptx,
    // image
    /// `image/avif` (`.avif`)
    Avif,
    /// `image/heic` (`.heic` / `.heif`)
    Heic,
    // audio / video
    /// `audio/aac` (`.aac`)
    Aac,
    /// `audio/opus` (`.opus`)
    Opus,
    /// `video/x-matroska` (`.mkv`)
    Matroska,
    // filesystem
    /// `inode/directory` — a directory (the [`Directory`](Category::Directory) role).
    Directory,
    // programming languages / source code (the `Code` category)
    /// `text/x-python` (`.py` / `.pyi`)
    Python,
    /// `text/x-rust` (`.rs`)
    Rust,
    /// `text/x-typescript` (`.ts` / `.tsx` / `.mts` / `.cts`)
    TypeScript,
    /// `text/x-go` (`.go`)
    Go,
    /// `text/x-c` (`.c` / `.h`)
    C,
    /// `text/x-c++` (`.cpp` / `.cc` / `.cxx` / `.hpp` / `.hh` / `.hxx`)
    Cpp,
    /// `text/x-csharp` (`.cs`)
    CSharp,
    /// `text/x-java` (`.java`)
    Java,
    /// `text/x-ruby` (`.rb`)
    Ruby,
    /// `application/x-httpd-php` (`.php`)
    Php,
    /// `application/x-sh` (`.sh` / `.bash` / `.zsh`)
    Shell,
    /// `text/x-swift` (`.swift`)
    Swift,
    /// `text/x-kotlin` (`.kt` / `.kts`)
    Kotlin,
    /// `application/sql` (`.sql`)
    Sql,
    /// `text/x-lua` (`.lua`)
    Lua,
    /// `text/x-perl` (`.pl` / `.pm`)
    Perl,
    /// `text/x-scala` (`.scala` / `.sc`)
    Scala,
    /// `text/x-r` (`.r`)
    RLang,
    /// `application/dart` (`.dart`)
    Dart,
    /// `text/x-haskell` (`.hs`)
    Haskell,
    /// Any MIME type outside the built-in registry, holding its `type/subtype`.
    Other(String),
}

/// A magic-byte signature: `bytes` expected at a fixed `offset` from the start of
/// a file. Used to build registry entries for [`MimeType::register`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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

/// One built-in registry row: the [`Category`], a constructor for the
/// [`MimeType`], its canonical MIME string, default file extensions (first is
/// canonical) and magic bytes.
struct Builtin {
    category: Category,
    new: fn() -> MimeType,
    mime: &'static str,
    extensions: &'static [&'static str],
    magic: &'static [Magic],
}

/// Shorthand for one built-in row.
const fn builtin(
    category: Category,
    new: fn() -> MimeType,
    mime: &'static str,
    extensions: &'static [&'static str],
    magic: &'static [Magic],
) -> Builtin {
    Builtin {
        category,
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
        Category::Blob,
        || MimeType::Plain,
        "text/plain",
        &["txt", "text", "log"],
        &[],
    ),
    builtin(
        Category::Code,
        || MimeType::Html,
        "text/html",
        &["html", "htm"],
        &[],
    ),
    builtin(Category::Code, || MimeType::Css, "text/css", &["css"], &[]),
    builtin(
        Category::Tabular,
        || MimeType::Csv,
        "text/csv",
        &["csv"],
        &[],
    ),
    builtin(
        Category::Code,
        || MimeType::Markdown,
        "text/markdown",
        &["md", "markdown"],
        &[],
    ),
    builtin(
        Category::Code,
        || MimeType::JavaScript,
        "text/javascript",
        &["js", "mjs", "cjs"],
        &[],
    ),
    // application/*
    builtin(
        Category::Code,
        || MimeType::Json,
        "application/json",
        &["json"],
        &[],
    ),
    builtin(
        Category::Code,
        || MimeType::Xml,
        "application/xml",
        &["xml"],
        &[mag(b"<?xml")],
    ),
    builtin(
        Category::Blob,
        || MimeType::Pdf,
        "application/pdf",
        &["pdf"],
        &[mag(b"%PDF-")],
    ),
    builtin(
        Category::Blob,
        || MimeType::Zip,
        "application/zip",
        &["zip"],
        &[mag(b"PK\x03\x04")],
    ),
    builtin(
        Category::Codec,
        || MimeType::Gzip,
        "application/gzip",
        &["gz", "gzip"],
        &[mag(b"\x1f\x8b")],
    ),
    builtin(
        Category::Blob,
        || MimeType::Tar,
        "application/x-tar",
        &["tar"],
        &[mag_at(257, b"ustar")],
    ),
    builtin(
        Category::Codec,
        || MimeType::Bzip2,
        "application/x-bzip2",
        &["bz2"],
        &[mag(b"BZh")],
    ),
    builtin(
        Category::Codec,
        || MimeType::Zstd,
        "application/zstd",
        &["zst"],
        &[mag(b"\x28\xb5\x2f\xfd")],
    ),
    builtin(
        // Brotli has no magic bytes — recognised by the `.br` extension alone.
        Category::Codec,
        || MimeType::Brotli,
        "application/x-brotli",
        &["br", "brotli"],
        &[],
    ),
    builtin(
        Category::Blob,
        || MimeType::SevenZip,
        "application/x-7z-compressed",
        &["7z"],
        &[mag(b"7z\xbc\xaf\x27\x1c")],
    ),
    builtin(
        Category::Tabular,
        || MimeType::Parquet,
        "application/vnd.apache.parquet",
        &["parquet"],
        &[mag(b"PAR1")],
    ),
    builtin(
        Category::Tabular,
        || MimeType::Arrow,
        "application/vnd.apache.arrow.file",
        &["arrow", "arrows", "ipc"],
        &[mag(b"ARROW1")],
    ),
    builtin(
        Category::Tabular,
        || MimeType::Avro,
        "application/vnd.apache.avro",
        &["avro"],
        &[mag(b"Obj\x01")],
    ),
    builtin(
        Category::Blob,
        || MimeType::Wasm,
        "application/wasm",
        &["wasm"],
        &[mag(b"\x00asm")],
    ),
    builtin(
        Category::Blob,
        || MimeType::Sqlite,
        "application/vnd.sqlite3",
        &["sqlite", "sqlite3", "db"],
        &[mag(b"SQLite format 3\x00")],
    ),
    builtin(
        Category::Blob,
        || MimeType::OctetStream,
        "application/octet-stream",
        &["bin"],
        &[],
    ),
    // image/*
    builtin(
        Category::Blob,
        || MimeType::Png,
        "image/png",
        &["png"],
        &[mag(b"\x89PNG\r\n\x1a\n")],
    ),
    builtin(
        Category::Blob,
        || MimeType::Jpeg,
        "image/jpeg",
        &["jpg", "jpeg"],
        &[mag(b"\xff\xd8\xff")],
    ),
    builtin(
        Category::Blob,
        || MimeType::Gif,
        "image/gif",
        &["gif"],
        &[mag(b"GIF87a"), mag(b"GIF89a")],
    ),
    builtin(
        Category::Blob,
        || MimeType::Webp,
        "image/webp",
        &["webp"],
        &[mag_at(8, b"WEBP")],
    ),
    builtin(
        Category::Blob,
        || MimeType::Bmp,
        "image/bmp",
        &["bmp"],
        &[mag(b"BM")],
    ),
    builtin(
        Category::Blob,
        || MimeType::Svg,
        "image/svg+xml",
        &["svg"],
        &[],
    ),
    builtin(
        Category::Blob,
        || MimeType::Icon,
        "image/x-icon",
        &["ico"],
        &[mag(b"\x00\x00\x01\x00")],
    ),
    builtin(
        Category::Blob,
        || MimeType::Tiff,
        "image/tiff",
        &["tif", "tiff"],
        &[mag(b"II\x2a\x00"), mag(b"MM\x00\x2a")],
    ),
    // AVIF carries an `ftyp` box with the `avif` brand — matched before MP4's
    // generic `ftyp` magic below. HEIC's brand varies, so it is recognised by
    // extension only (a bare `ftyp` would clash with MP4).
    builtin(
        Category::Blob,
        || MimeType::Avif,
        "image/avif",
        &["avif"],
        &[mag_at(4, b"ftypavif")],
    ),
    builtin(
        Category::Blob,
        || MimeType::Heic,
        "image/heic",
        &["heic", "heif"],
        &[],
    ),
    // audio/*
    builtin(
        Category::Blob,
        || MimeType::Mp3,
        "audio/mpeg",
        &["mp3"],
        &[mag(b"ID3")],
    ),
    builtin(
        Category::Blob,
        || MimeType::Wav,
        "audio/wav",
        &["wav"],
        &[mag_at(8, b"WAVE")],
    ),
    builtin(
        Category::Blob,
        || MimeType::Flac,
        "audio/flac",
        &["flac"],
        &[mag(b"fLaC")],
    ),
    builtin(
        Category::Blob,
        || MimeType::Ogg,
        "audio/ogg",
        &["ogg", "oga"],
        &[mag(b"OggS")],
    ),
    // video/*
    builtin(
        Category::Blob,
        || MimeType::Mp4,
        "video/mp4",
        &["mp4", "m4v"],
        &[mag_at(4, b"ftyp")],
    ),
    builtin(
        Category::Blob,
        || MimeType::Webm,
        "video/webm",
        &["webm"],
        &[mag(b"\x1a\x45\xdf\xa3")],
    ),
    builtin(
        Category::Blob,
        || MimeType::Avi,
        "video/x-msvideo",
        &["avi"],
        &[mag_at(8, b"AVI ")],
    ),
    // font/*
    builtin(
        Category::Blob,
        || MimeType::Woff,
        "font/woff",
        &["woff"],
        &[mag(b"wOFF")],
    ),
    builtin(
        Category::Blob,
        || MimeType::Woff2,
        "font/woff2",
        &["woff2"],
        &[mag(b"wOF2")],
    ),
    builtin(
        Category::Blob,
        || MimeType::Ttf,
        "font/ttf",
        &["ttf"],
        &[mag(b"\x00\x01\x00\x00")],
    ),
    builtin(
        Category::Blob,
        || MimeType::Otf,
        "font/otf",
        &["otf"],
        &[mag(b"OTTO")],
    ),
    // text / data
    builtin(
        Category::Code,
        || MimeType::Yaml,
        "application/yaml",
        &["yaml", "yml"],
        &[],
    ),
    builtin(
        Category::Code,
        || MimeType::Toml,
        "application/toml",
        &["toml"],
        &[],
    ),
    builtin(
        Category::Tabular,
        || MimeType::Ndjson,
        "application/x-ndjson",
        &["ndjson", "jsonl"],
        &[],
    ),
    builtin(
        Category::Blob,
        || MimeType::Rtf,
        "application/rtf",
        &["rtf"],
        &[mag(b"{\\rtf")],
    ),
    // compression / archives
    builtin(
        Category::Codec,
        || MimeType::Xz,
        "application/x-xz",
        &["xz"],
        &[mag(b"\xfd7zXZ\x00")],
    ),
    builtin(
        Category::Codec,
        || MimeType::Lz4,
        "application/x-lz4",
        &["lz4"],
        &[mag(b"\x04\x22\x4d\x18")],
    ),
    // documents — OOXML / EPUB are ZIP containers (PK magic), so the ZIP magic is
    // intentionally left off here and they are recognised by extension only.
    builtin(
        Category::Blob,
        || MimeType::Epub,
        "application/epub+zip",
        &["epub"],
        &[],
    ),
    builtin(
        Category::Blob,
        || MimeType::Docx,
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        &["docx"],
        &[],
    ),
    builtin(
        Category::Blob,
        || MimeType::Xlsx,
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        &["xlsx"],
        &[],
    ),
    builtin(
        Category::Blob,
        || MimeType::Pptx,
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        &["pptx"],
        &[],
    ),
    // audio / video — AAC (ADTS), Opus (Ogg container) and Matroska (EBML, shared
    // with WebM) are recognised by extension to avoid ambiguous magic matches.
    builtin(Category::Blob, || MimeType::Aac, "audio/aac", &["aac"], &[]),
    builtin(
        Category::Blob,
        || MimeType::Opus,
        "audio/opus",
        &["opus"],
        &[],
    ),
    builtin(
        Category::Blob,
        || MimeType::Matroska,
        "video/x-matroska",
        &["mkv"],
        &[],
    ),
    // filesystem — a directory has no extension or magic.
    builtin(
        Category::Directory,
        || MimeType::Directory,
        "inode/directory",
        &[],
        &[],
    ),
    // programming languages / source code — text with no reliable magic bytes, so
    // recognised by file extension only. The first extension is canonical.
    builtin(
        Category::Code,
        || MimeType::Python,
        "text/x-python",
        &["py", "pyi"],
        &[],
    ),
    builtin(
        Category::Code,
        || MimeType::Rust,
        "text/x-rust",
        &["rs"],
        &[],
    ),
    builtin(
        Category::Code,
        || MimeType::TypeScript,
        "text/x-typescript",
        &["ts", "tsx", "mts", "cts"],
        &[],
    ),
    builtin(Category::Code, || MimeType::Go, "text/x-go", &["go"], &[]),
    builtin(Category::Code, || MimeType::C, "text/x-c", &["c", "h"], &[]),
    builtin(
        Category::Code,
        || MimeType::Cpp,
        "text/x-c++",
        &["cpp", "cc", "cxx", "hpp", "hh", "hxx"],
        &[],
    ),
    builtin(
        Category::Code,
        || MimeType::CSharp,
        "text/x-csharp",
        &["cs"],
        &[],
    ),
    builtin(
        Category::Code,
        || MimeType::Java,
        "text/x-java",
        &["java"],
        &[],
    ),
    builtin(
        Category::Code,
        || MimeType::Ruby,
        "text/x-ruby",
        &["rb"],
        &[],
    ),
    builtin(
        Category::Code,
        || MimeType::Php,
        "application/x-httpd-php",
        &["php"],
        &[],
    ),
    builtin(
        Category::Code,
        || MimeType::Shell,
        "application/x-sh",
        &["sh", "bash", "zsh"],
        &[],
    ),
    builtin(
        Category::Code,
        || MimeType::Swift,
        "text/x-swift",
        &["swift"],
        &[],
    ),
    builtin(
        Category::Code,
        || MimeType::Kotlin,
        "text/x-kotlin",
        &["kt", "kts"],
        &[],
    ),
    builtin(
        Category::Code,
        || MimeType::Sql,
        "application/sql",
        &["sql"],
        &[],
    ),
    builtin(
        Category::Code,
        || MimeType::Lua,
        "text/x-lua",
        &["lua"],
        &[],
    ),
    builtin(
        Category::Code,
        || MimeType::Perl,
        "text/x-perl",
        &["pl", "pm"],
        &[],
    ),
    builtin(
        Category::Code,
        || MimeType::Scala,
        "text/x-scala",
        &["scala", "sc"],
        &[],
    ),
    builtin(Category::Code, || MimeType::RLang, "text/x-r", &["r"], &[]),
    builtin(
        Category::Code,
        || MimeType::Dart,
        "application/dart",
        &["dart"],
        &[],
    ),
    builtin(
        Category::Code,
        || MimeType::Haskell,
        "text/x-haskell",
        &["hs"],
        &[],
    ),
];

/// One mutable registry entry: everything known about one MIME type.
#[derive(Clone)]
struct Entry {
    mime: String,
    category: Category,
    extensions: Vec<String>,
    magic: Vec<Signature>,
}

impl Entry {
    /// Materialises a mutable entry from a built-in default.
    fn from_builtin(b: &Builtin) -> Entry {
        Entry {
            mime: b.mime.to_string(),
            category: b.category,
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

/// Resolves a short, slash-less name to a [`MimeType`] by matching a file
/// extension first, then a MIME subtype (both case-insensitive), e.g. `"json"` /
/// `"gz"` / `"gzip"` → the matching type, `"zstd"` → [`Zstd`](MimeType::Zstd).
pub(crate) fn resolve_name(name: &str) -> Option<MimeType> {
    let lower = name.to_ascii_lowercase();
    MimeType::from_extension(&lower).or_else(|| {
        BUILTINS
            .iter()
            .find(|b| b.mime.rsplit('/').next() == Some(lower.as_str()))
            .map(|b| (b.new)())
    })
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
        // Fast path: an already-lowercase string matches a built-in by borrow, so
        // no allocation is needed unless it falls through to `Other`. Registry
        // lookups (`from_extension`/`from_magic`) always hit this path.
        if !mime.bytes().any(|b| b.is_ascii_uppercase()) {
            return BUILTINS
                .iter()
                .find(|b| b.mime == mime)
                .map(|b| (b.new)())
                .unwrap_or_else(|| MimeType::Other(mime.to_string()));
        }
        let lower = mime.to_ascii_lowercase();
        BUILTINS
            .iter()
            .find(|b| b.mime == lower)
            .map(|b| (b.new)())
            .unwrap_or(MimeType::Other(lower))
    }

    /// Builds a [`MimeType`] from its `type` and `subtype` parts directly, without
    /// parsing a combined string — `from_parts("text", "csv")` is
    /// [`Csv`](MimeType::Csv). A well-formed but unknown pair becomes
    /// [`Other`](MimeType::Other).
    pub fn from_parts(type_: &str, subtype: &str) -> MimeType {
        MimeType::from_mime(&format!("{type_}/{subtype}"))
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

    /// The [`Category`] this type belongs to (read from the registry), or
    /// [`Category::Blob`] (the default) for an unregistered
    /// [`Other`](MimeType::Other).
    pub fn category(&self) -> Category {
        let mime = self.mime();
        let registry = registry().read().unwrap();
        registry
            .iter()
            .find(|e| e.mime == mime)
            .map(|e| e.category)
            .unwrap_or_default()
    }

    /// Registers (or replaces) a MIME type in the global registry, so subsequent
    /// [`from_extension`](MimeType::from_extension) /
    /// [`from_magic`](MimeType::from_magic) lookups recognise it and
    /// [`category`](MimeType::category) reports `category`. The change is
    /// process-wide.
    pub fn register(mime: &str, extensions: &[&str], magic: &[Signature], category: Category) {
        let mime = mime.to_ascii_lowercase();
        let entry = Entry {
            extensions: extensions.iter().map(|s| s.to_ascii_lowercase()).collect(),
            magic: magic.to_vec(),
            category,
            mime: mime.clone(),
        };
        let mut registry = registry().write().unwrap();
        log_event!(
            info,
            "MimeType::register {mime:?} ({category}, {} extensions)",
            entry.extensions.len()
        );
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
        let removed = registry.len() != before;
        log_event!(info, "MimeType::unregister {mime:?} (removed: {removed})");
        removed
    }

    /// Restores the global registry to its built-in defaults, discarding every
    /// [`register`](MimeType::register) / [`unregister`](MimeType::unregister).
    pub fn reset_registry() {
        let mut registry = registry().write().unwrap();
        *registry = BUILTINS.iter().map(Entry::from_builtin).collect();
        log_event!(
            info,
            "MimeType::reset_registry ({} built-ins)",
            registry.len()
        );
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

/// String/mapping parsers.
impl MimeType {
    /// Parses a MIME string such as `"text/html"` (any `;parameters` are dropped),
    /// or a short name like `"json"`, `"gzip"` or `"zstd"` (matched as a file
    /// extension or MIME subtype). A full `type/subtype` must be a valid token
    /// pair — unknown but well-formed ones become [`Other`](MimeType::Other);
    /// unknown short names are an error.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(input: &str) -> Result<MimeType, MediaError> {
        if input.is_empty() {
            return Err(MediaError::Empty);
        }
        let essence = input.split(';').next().unwrap_or(input).trim();
        // A slash-less token is a short name (extension or subtype), not a MIME.
        if !essence.contains('/') {
            let resolved = resolve_name(essence);
            log_event!(
                debug,
                "MimeType::from_str: short name {essence:?} -> {resolved:?}"
            );
            return resolved.ok_or_else(|| MediaError::Invalid(input.to_string()));
        }
        let essence = essence.to_ascii_lowercase();
        if !is_valid_mime(&essence) {
            return Err(MediaError::Invalid(input.to_string()));
        }
        Ok(MimeType::from_mime(&essence))
    }

    /// Builds a [`MimeType`] from a [`Mapping`]. Recognised keys: `type` and
    /// `subtype`; both must be present and valid tokens.
    pub fn from_mapping(fields: &Mapping) -> Result<MimeType, MediaError> {
        let type_ = fields.get("type").map_or("", String::as_str);
        let subtype = fields.get("subtype").map_or("", String::as_str);
        if !is_valid_mime(&format!("{type_}/{subtype}")) {
            return Err(MediaError::Invalid(format!("{type_}/{subtype}")));
        }
        Ok(MimeType::from_parts(type_, subtype))
    }
}

impl fmt::Display for MimeType {
    /// Renders the canonical MIME string.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.mime())
    }
}

/// Component rendering: the inverse of the `from_str` / `from_mapping` parsers.
impl MimeType {
    /// Renders to the canonical MIME string (the `encode` flag is irrelevant).
    pub fn to_str(&self, _encode: bool) -> String {
        self.mime().to_string()
    }

    /// The inverse of `from_mapping`: keys `type` and `subtype`.
    pub fn to_mapping(&self) -> Mapping {
        Mapping::from([
            ("type".to_string(), self.type_().to_string()),
            ("subtype".to_string(), self.subtype().to_string()),
        ])
    }
}

/// Serialises as the canonical MIME string, the inverse of
/// [`MimeType::from_mime`] (which round-trips [`Other`](MimeType::Other) too).
#[cfg(feature = "serde")]
impl serde::Serialize for MimeType {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.mime())
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for MimeType {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<MimeType, D::Error> {
        let raw = <String as serde::Deserialize>::deserialize(deserializer)?;
        Ok(MimeType::from_mime(&raw))
    }
}
