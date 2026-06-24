//! # yggdryl-media
//!
//! Media (MIME) type detection for the **yggdryl** project, built on the
//! [`yggdryl-core`](https://crates.io/crates/yggdryl-core) parsing traits.
//!
//! [`MediaType`] is an enum of common media types (with an [`Other`](MediaType::Other)
//! escape hatch for anything else). It can be parsed from a MIME string, inferred
//! from a file extension ([`from_extension`](MediaType::from_extension)) or from a
//! file's leading bytes ([`from_magic`](MediaType::from_magic)) — the latter
//! recognises container and columnar formats such as Apache Arrow IPC, Parquet,
//! ZIP and gzip.
//!
//! ```
//! use yggdryl_media::{FromInput, MediaType};
//!
//! assert_eq!(MediaType::from_str("application/json", true).unwrap().subtype(), "json");
//! assert_eq!(MediaType::from_extension("parquet"), Some(MediaType::Parquet));
//! assert_eq!(MediaType::from_magic(b"PK\x03\x04..."), Some(MediaType::Zip));
//! ```

use std::fmt;

pub use yggdryl_core::{FromInput, Mapping, ToOutput};

/// Error returned when [`MediaType::from_`] cannot interpret its input.
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

/// A common media (MIME) type, or [`Other`](MediaType::Other) for anything not in
/// the built-in registry.
///
/// The canonical MIME string, the file extensions and the magic-byte signatures
/// for each known variant live in a single registry (see [`MediaType::mime`],
/// [`MediaType::extensions`], [`MediaType::from_magic`]).
///
/// ```
/// use yggdryl_media::MediaType;
///
/// let png = MediaType::Png;
/// assert_eq!(png.mime(), "image/png");
/// assert_eq!(png.type_(), "image");
/// assert_eq!(png.extension(), Some("png"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MediaType {
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
    /// Any media type outside the built-in registry, holding its `type/subtype`.
    Other(String),
}

/// A magic-byte signature: `bytes` expected at a fixed `offset` from the start.
#[derive(Clone, Copy)]
struct Signature {
    offset: usize,
    bytes: &'static [u8],
}

impl Signature {
    /// Whether `data` carries this signature at its offset.
    fn matches(&self, data: &[u8]) -> bool {
        let end = self.offset + self.bytes.len();
        data.len() >= end && &data[self.offset..end] == self.bytes
    }
}

/// One row of the media-type registry: a constructor for the [`MediaType`], its
/// canonical MIME string, the file extensions that map to it (first is the
/// canonical one) and the magic-byte signatures that identify it by content.
#[derive(Clone, Copy)]
struct Row {
    new: fn() -> MediaType,
    mime: &'static str,
    extensions: &'static [&'static str],
    magic: &'static [Signature],
}

/// Shorthand for one registry row.
const fn row(
    new: fn() -> MediaType,
    mime: &'static str,
    extensions: &'static [&'static str],
    magic: &'static [Signature],
) -> Row {
    Row {
        new,
        mime,
        extensions,
        magic,
    }
}

/// Shorthand for a signature at the start of the data.
const fn sig(bytes: &'static [u8]) -> Signature {
    Signature { offset: 0, bytes }
}

/// Shorthand for a signature at a fixed offset.
const fn sig_at(offset: usize, bytes: &'static [u8]) -> Signature {
    Signature { offset, bytes }
}

/// The built-in media-type registry — the single source of truth mapping each
/// known [`MediaType`] to its MIME string, extensions and magic bytes.
static ROWS: &[Row] = &[
    // text/*
    row(
        || MediaType::Plain,
        "text/plain",
        &["txt", "text", "log"],
        &[],
    ),
    row(|| MediaType::Html, "text/html", &["html", "htm"], &[]),
    row(|| MediaType::Css, "text/css", &["css"], &[]),
    row(|| MediaType::Csv, "text/csv", &["csv"], &[]),
    row(
        || MediaType::Markdown,
        "text/markdown",
        &["md", "markdown"],
        &[],
    ),
    row(
        || MediaType::JavaScript,
        "text/javascript",
        &["js", "mjs"],
        &[],
    ),
    // application/*
    row(|| MediaType::Json, "application/json", &["json"], &[]),
    row(
        || MediaType::Xml,
        "application/xml",
        &["xml"],
        &[sig(b"<?xml")],
    ),
    row(
        || MediaType::Pdf,
        "application/pdf",
        &["pdf"],
        &[sig(b"%PDF-")],
    ),
    row(
        || MediaType::Zip,
        "application/zip",
        &["zip"],
        &[sig(b"PK\x03\x04")],
    ),
    row(
        || MediaType::Gzip,
        "application/gzip",
        &["gz", "gzip"],
        &[sig(b"\x1f\x8b")],
    ),
    row(
        || MediaType::Tar,
        "application/x-tar",
        &["tar"],
        &[sig_at(257, b"ustar")],
    ),
    row(
        || MediaType::Bzip2,
        "application/x-bzip2",
        &["bz2"],
        &[sig(b"BZh")],
    ),
    row(
        || MediaType::Zstd,
        "application/zstd",
        &["zst"],
        &[sig(b"\x28\xb5\x2f\xfd")],
    ),
    row(
        || MediaType::SevenZip,
        "application/x-7z-compressed",
        &["7z"],
        &[sig(b"7z\xbc\xaf\x27\x1c")],
    ),
    row(
        || MediaType::Parquet,
        "application/vnd.apache.parquet",
        &["parquet"],
        &[sig(b"PAR1")],
    ),
    row(
        || MediaType::Arrow,
        "application/vnd.apache.arrow.file",
        &["arrow", "arrows", "ipc"],
        &[sig(b"ARROW1")],
    ),
    row(
        || MediaType::Avro,
        "application/vnd.apache.avro",
        &["avro"],
        &[sig(b"Obj\x01")],
    ),
    row(
        || MediaType::Wasm,
        "application/wasm",
        &["wasm"],
        &[sig(b"\x00asm")],
    ),
    row(
        || MediaType::Sqlite,
        "application/vnd.sqlite3",
        &["sqlite", "sqlite3", "db"],
        &[sig(b"SQLite format 3\x00")],
    ),
    row(
        || MediaType::OctetStream,
        "application/octet-stream",
        &["bin"],
        &[],
    ),
    // image/*
    row(
        || MediaType::Png,
        "image/png",
        &["png"],
        &[sig(b"\x89PNG\r\n\x1a\n")],
    ),
    row(
        || MediaType::Jpeg,
        "image/jpeg",
        &["jpg", "jpeg"],
        &[sig(b"\xff\xd8\xff")],
    ),
    row(
        || MediaType::Gif,
        "image/gif",
        &["gif"],
        &[sig(b"GIF87a"), sig(b"GIF89a")],
    ),
    row(
        || MediaType::Webp,
        "image/webp",
        &["webp"],
        &[sig_at(8, b"WEBP")],
    ),
    row(|| MediaType::Bmp, "image/bmp", &["bmp"], &[sig(b"BM")]),
    row(|| MediaType::Svg, "image/svg+xml", &["svg"], &[]),
    row(
        || MediaType::Icon,
        "image/x-icon",
        &["ico"],
        &[sig(b"\x00\x00\x01\x00")],
    ),
    row(
        || MediaType::Tiff,
        "image/tiff",
        &["tif", "tiff"],
        &[sig(b"II\x2a\x00"), sig(b"MM\x00\x2a")],
    ),
    // audio/*
    row(|| MediaType::Mp3, "audio/mpeg", &["mp3"], &[sig(b"ID3")]),
    row(
        || MediaType::Wav,
        "audio/wav",
        &["wav"],
        &[sig_at(8, b"WAVE")],
    ),
    row(|| MediaType::Flac, "audio/flac", &["flac"], &[sig(b"fLaC")]),
    row(
        || MediaType::Ogg,
        "audio/ogg",
        &["ogg", "oga"],
        &[sig(b"OggS")],
    ),
    // video/*
    row(
        || MediaType::Mp4,
        "video/mp4",
        &["mp4", "m4v"],
        &[sig_at(4, b"ftyp")],
    ),
    row(
        || MediaType::Webm,
        "video/webm",
        &["webm"],
        &[sig(b"\x1a\x45\xdf\xa3")],
    ),
    row(
        || MediaType::Avi,
        "video/x-msvideo",
        &["avi"],
        &[sig_at(8, b"AVI ")],
    ),
    // font/*
    row(|| MediaType::Woff, "font/woff", &["woff"], &[sig(b"wOFF")]),
    row(
        || MediaType::Woff2,
        "font/woff2",
        &["woff2"],
        &[sig(b"wOF2")],
    ),
    row(
        || MediaType::Ttf,
        "font/ttf",
        &["ttf"],
        &[sig(b"\x00\x01\x00\x00")],
    ),
    row(|| MediaType::Otf, "font/otf", &["otf"], &[sig(b"OTTO")]),
];

/// Returns `true` for an [RFC 2045](https://www.rfc-editor.org/rfc/rfc2045) token:
/// a non-empty run of `ALPHA / DIGIT` and the punctuation MIME types may contain.
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

impl MediaType {
    /// The registry row for a known variant, or `None` for [`Other`](MediaType::Other).
    fn row(&self) -> Option<&'static Row> {
        if matches!(self, MediaType::Other(_)) {
            return None;
        }
        ROWS.iter().find(|r| (r.new)() == *self)
    }

    /// Looks up a [`MediaType`] by its (case-insensitive) MIME string, falling
    /// back to [`Other`](MediaType::Other) for anything unknown.
    pub fn from_mime(mime: &str) -> MediaType {
        let mime = mime.to_ascii_lowercase();
        ROWS.iter()
            .find(|r| r.mime == mime)
            .map(|r| (r.new)())
            .unwrap_or(MediaType::Other(mime))
    }

    /// Infers a [`MediaType`] from a file `extension` (with or without a leading
    /// `.`), or `None` if it is not in the registry.
    pub fn from_extension(extension: &str) -> Option<MediaType> {
        let ext = extension.trim_start_matches('.').to_ascii_lowercase();
        ROWS.iter()
            .find(|r| r.extensions.contains(&ext.as_str()))
            .map(|r| (r.new)())
    }

    /// Infers a [`MediaType`] from a file's leading bytes by matching the
    /// registry's magic-byte signatures, or `None` if none match. Recognises
    /// container and columnar formats such as Arrow IPC, Parquet, ZIP and gzip.
    pub fn from_magic(data: &[u8]) -> Option<MediaType> {
        ROWS.iter()
            .find(|r| r.magic.iter().any(|s| s.matches(data)))
            .map(|r| (r.new)())
    }

    /// Infers a [`MediaType`] from the last `.`-extension of a path's file name,
    /// or `None` if there is no known extension.
    pub fn from_path(path: &str) -> Option<MediaType> {
        let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
        let (_, ext) = name.rsplit_once('.')?;
        MediaType::from_extension(ext)
    }

    /// The canonical MIME string, e.g. `"image/png"`. For
    /// [`Other`](MediaType::Other) this is the stored `type/subtype`.
    pub fn mime(&self) -> &str {
        match self {
            MediaType::Other(mime) => mime,
            _ => self.row().map(|r| r.mime).unwrap_or(""),
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

    /// The file extensions associated with this type (the first is canonical);
    /// empty for [`Other`](MediaType::Other).
    pub fn extensions(&self) -> &'static [&'static str] {
        self.row().map_or(&[], |r| r.extensions)
    }

    /// The canonical (first) file extension, if any.
    pub fn extension(&self) -> Option<&'static str> {
        self.extensions().first().copied()
    }

    /// Whether this is a registry variant rather than [`Other`](MediaType::Other).
    pub fn is_known(&self) -> bool {
        !matches!(self, MediaType::Other(_))
    }
}

impl FromInput for MediaType {
    type Err = MediaError;

    /// Parses a MIME string such as `"text/html"` (any `;parameters` are dropped).
    /// When `safe`, the essence must be a `type/subtype` pair of valid tokens;
    /// when not `safe`, the input is taken as-is. Unknown but well-formed types
    /// become [`Other`](MediaType::Other).
    fn from_str(input: &str, safe: bool) -> Result<MediaType, MediaError> {
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
        Ok(MediaType::from_mime(&essence))
    }

    /// Builds a [`MediaType`] from a [`Mapping`]. Recognised keys: `type` and
    /// `subtype`. When `safe`, both must be present and valid tokens.
    fn from_mapping(fields: &Mapping, safe: bool) -> Result<MediaType, MediaError> {
        let type_ = fields.get("type").map_or("", String::as_str);
        let subtype = fields.get("subtype").map_or("", String::as_str);
        let mime = format!("{type_}/{subtype}");
        if safe && !is_valid_mime(&mime) {
            return Err(MediaError::Invalid(mime));
        }
        Ok(MediaType::from_mime(&mime))
    }
}

impl fmt::Display for MediaType {
    /// Renders the canonical MIME string.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.mime())
    }
}

impl ToOutput for MediaType {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_splits_mime() {
        let m = MediaType::from_str("application/json", true).unwrap();
        assert_eq!(m, MediaType::Json);
        assert_eq!(m.mime(), "application/json");
        assert_eq!(m.type_(), "application");
        assert_eq!(m.subtype(), "json");
        // Parameters are dropped; case is normalised.
        assert_eq!(
            MediaType::from_str("Text/HTML; charset=utf-8", true).unwrap(),
            MediaType::Html
        );
    }

    #[test]
    fn unknown_becomes_other() {
        let m = MediaType::from_str("application/x-custom", true).unwrap();
        assert_eq!(m, MediaType::Other("application/x-custom".to_string()));
        assert!(!m.is_known());
        assert_eq!(m.subtype(), "x-custom");
        assert_eq!(m.extension(), None);
    }

    #[test]
    fn errors() {
        assert_eq!(MediaType::from_str("", true), Err(MediaError::Empty));
        assert_eq!(
            MediaType::from_str("notamime", true),
            Err(MediaError::Invalid("notamime".to_string()))
        );
        // The fast path keeps whatever it is given.
        assert_eq!(
            MediaType::from_str("notamime", false).unwrap(),
            MediaType::Other("notamime".to_string())
        );
    }

    #[test]
    fn from_extension_maps_known_types() {
        assert_eq!(
            MediaType::from_extension("parquet"),
            Some(MediaType::Parquet)
        );
        assert_eq!(MediaType::from_extension(".GZ"), Some(MediaType::Gzip));
        assert_eq!(MediaType::from_extension("jpeg"), Some(MediaType::Jpeg));
        assert_eq!(MediaType::from_extension("nope"), None);
    }

    #[test]
    fn from_magic_sniffs_content() {
        assert_eq!(
            MediaType::from_magic(b"PAR1\x15\x04"),
            Some(MediaType::Parquet)
        );
        assert_eq!(
            MediaType::from_magic(b"ARROW1\x00\x00"),
            Some(MediaType::Arrow)
        );
        assert_eq!(
            MediaType::from_magic(b"PK\x03\x04\x14"),
            Some(MediaType::Zip)
        );
        assert_eq!(
            MediaType::from_magic(b"\x1f\x8b\x08\x00"),
            Some(MediaType::Gzip)
        );
        assert_eq!(
            MediaType::from_magic(b"\x89PNG\r\n\x1a\n\x00"),
            Some(MediaType::Png)
        );
        // Offset-based signatures: tar's `ustar` lives at byte 257.
        let mut tar = vec![0u8; 270];
        tar[257..262].copy_from_slice(b"ustar");
        assert_eq!(MediaType::from_magic(&tar), Some(MediaType::Tar));
        assert_eq!(MediaType::from_magic(b"not magic"), None);
    }

    #[test]
    fn from_path_uses_last_extension() {
        assert_eq!(
            MediaType::from_path("/data/sales.parquet"),
            Some(MediaType::Parquet)
        );
        // Compound extensions resolve to the outer container.
        assert_eq!(
            MediaType::from_path("archive.tar.gz"),
            Some(MediaType::Gzip)
        );
        // Dotfiles and extension-less names have no media type.
        assert_eq!(MediaType::from_path("/etc/.bashrc"), None);
        assert_eq!(MediaType::from_path("/usr/bin/env"), None);
    }

    #[test]
    fn round_trips_through_mapping() {
        let m = MediaType::Svg;
        assert_eq!(m.mime(), "image/svg+xml");
        let map = m.to_mapping();
        assert_eq!(map.get("type"), Some(&"image".to_string()));
        assert_eq!(map.get("subtype"), Some(&"svg+xml".to_string()));
        assert_eq!(MediaType::from_(&map).unwrap(), m);
    }

    #[test]
    fn extensions_and_to_str() {
        assert_eq!(MediaType::Jpeg.extensions(), &["jpg", "jpeg"]);
        assert_eq!(MediaType::Jpeg.extension(), Some("jpg"));
        assert!(MediaType::Other("x/y".to_string()).extensions().is_empty());
        assert_eq!(MediaType::Gzip.to_str(true), "application/gzip");
    }
}
