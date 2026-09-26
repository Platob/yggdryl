use std::cmp::Ordering;

use std::collections::HashSet;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smol_str::{SmolStr, SmolStrBuilder};

use crate::text::Format;
use crate::{Error, Result, hashing::stable_hash_display};

mod datatype;

pub use datatype::*;

#[derive(Clone, Debug)]
enum MimeTypeWire {
    OctetStream,
    Json,
    JsonLines,
    Yaml,
    Toml,
    Csv,
    Tsv,
    Parquet,
    ArrowFile,
    ArrowStream,
    Avro,
    Orc,
    Puffin,
    PlainText,
    KeyValue,
    Ullink,
    Fix,
    Fixul,
    Fixml,
    Markdown,
    Html,
    Css,
    JavaScript,
    Xml,
    Http,
    Pdf,
    Cbor,
    MessagePack,
    Protobuf,
    Sqlite3,
    Png,
    Jpeg,
    Gif,
    WebP,
    Svg,
    Mp3,
    Wav,
    Ogg,
    Flac,
    Mp4,
    WebM,
    Woff,
    Woff2,
    Ttf,
    Otf,
    Xls,
    Xlsx,
    Ods,
    Doc,
    Docx,
    Gzip,
    Zstd,
    Brotli,
    Zlib,
    Compress,
    Bzip2,
    Xz,
    Lz4,
    Snappy,
    Zip,
    SevenZip,
    Rar,
    Tar,
    Directory,
    File,
    Custom(SmolStr),
}

pub(crate) mod line;
mod registry;

use registry::{known_from_extension, known_from_mime};

/// A validated, canonical MIME media type.
///
/// Common data, document, media, archive, and encoding values use static
/// allocation-free representations. Any valid RFC restricted `type/subtype`
/// name remains supported and is stored once in canonical ASCII lowercase.
#[derive(Clone, Debug)]
pub struct MimeType(MimeTypeWire);

impl MimeType {
    /// Arbitrary binary data and the default MIME type.
    pub const OCTET_STREAM: Self = Self(MimeTypeWire::OctetStream);
    /// JSON structured data.
    pub const JSON: Self = Self(MimeTypeWire::Json);
    /// Newline-delimited JSON data.
    pub const JSON_LINES: Self = Self(MimeTypeWire::JsonLines);
    /// YAML structured data.
    pub const YAML: Self = Self(MimeTypeWire::Yaml);
    /// TOML structured data.
    pub const TOML: Self = Self(MimeTypeWire::Toml);
    /// Comma-separated tabular text.
    pub const CSV: Self = Self(MimeTypeWire::Csv);
    /// Tab-separated tabular text.
    pub const TSV: Self = Self(MimeTypeWire::Tsv);
    /// Apache Parquet tabular data.
    pub const PARQUET: Self = Self(MimeTypeWire::Parquet);
    /// An Apache Arrow IPC file.
    pub const ARROW_FILE: Self = Self(MimeTypeWire::ArrowFile);
    /// An Apache Arrow IPC stream.
    pub const ARROW_STREAM: Self = Self(MimeTypeWire::ArrowStream);
    /// Apache Avro data.
    pub const AVRO: Self = Self(MimeTypeWire::Avro);
    /// Apache ORC tabular data.
    pub const ORC: Self = Self(MimeTypeWire::Orc);
    /// Apache Puffin statistics and index data.
    pub const PUFFIN: Self = Self(MimeTypeWire::Puffin);
    /// Unformatted plain text.
    pub const PLAIN_TEXT: Self = Self(MimeTypeWire::PlainText);
    /// Infers what one captured byte line is, without a dictionary.
    ///
    /// A capture is millions of lines and most are not the protocol a reader
    /// is after. One shallow scan decides: numeric `tag=value` entries prove
    /// [`Self::FIX`], `#`-marked or `MSGTYPE=` keys prove [`Self::ULLINK`],
    /// both prove [`Self::FIXUL`], and an `XmlData(213)` payload opening with
    /// a tag proves [`Self::FIXML`]. A line that is no frame but opens as a
    /// document is [`Self::XML`] or [`Self::JSON`] - a bridge configuration
    /// document is JSON, which is what it is, and what makes one a
    /// configuration is a shape the codec reads rather than a name this scan
    /// gives it; one that is still `key=value` throughout is
    /// [`Self::KEYVALUE`]; anything else is [`Self::OCTET_STREAM`].
    ///
    /// The scan reads no message and allocates nothing.
    ///
    /// ```
    /// use yggdryl::MimeType;
    ///
    /// let framed = b"sending 8=FIX.4.4|35=D|55=AAPL|10=001| queued seq=7";
    /// assert_eq!(MimeType::infer_bytes(framed), MimeType::FIX);
    /// assert_eq!(
    ///     MimeType::infer_bytes(b"#MSGTYPE=D|#SYMBOL=AAPL"),
    ///     MimeType::ULLINK
    /// );
    /// // No frame, but pairs throughout.
    /// assert_eq!(
    ///     MimeType::infer_bytes(b"level=INFO worker=3 took=12ms"),
    ///     MimeType::KEYVALUE
    /// );
    /// // A document wins over the pair rules.
    /// assert_eq!(MimeType::infer_bytes(b"<Order id='1'/>"), MimeType::XML);
    /// assert_eq!(MimeType::infer_bytes(br#"{"a":1}"#), MimeType::JSON);
    /// // A bridge's own object model inside one is JSON like any other.
    /// let jolokia = br#"{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*","type":"read"},"status":200}"#;
    /// assert_eq!(MimeType::infer_bytes(jolokia), MimeType::JSON);
    /// assert_eq!(
    ///     MimeType::infer_bytes(b"no level printed by this plugin"),
    ///     MimeType::OCTET_STREAM
    /// );
    /// ```
    #[must_use]
    pub fn infer_bytes(line: &[u8]) -> Self {
        line::classify(line).0
    }

    /// Infers what one captured text line is.
    #[must_use]
    pub fn infer_text(line: &str) -> Self {
        Self::infer_bytes(line.as_bytes())
    }

    /// A line of plain `key=value` pairs, with no frame around them.
    ///
    /// The generic shape a log attribute run has: no numeric tags, no
    /// `#`-marked keys, no envelope. It is what a line still is when every
    /// frame rule declined it and it is nevertheless pairs throughout.
    pub const KEYVALUE: Self = Self(MimeTypeWire::KeyValue);
    /// A symbolic-key Ullink text frame.
    pub const ULLINK: Self = Self(MimeTypeWire::Ullink);
    /// A numeric-tag FIX text frame.
    pub const FIX: Self = Self(MimeTypeWire::Fix);
    /// A FIX text frame containing Ullink symbolic key/value entries.
    pub const FIXUL: Self = Self(MimeTypeWire::Fixul);
    /// A FIXML frame carrying XML.
    pub const FIXML: Self = Self(MimeTypeWire::Fixml);
    /// Markdown text.
    pub const MARKDOWN: Self = Self(MimeTypeWire::Markdown);
    /// HTML text.
    pub const HTML: Self = Self(MimeTypeWire::Html);
    /// Cascading Style Sheets text.
    pub const CSS: Self = Self(MimeTypeWire::Css);
    /// JavaScript source text.
    pub const JAVASCRIPT: Self = Self(MimeTypeWire::JavaScript);
    /// XML structured data.
    pub const XML: Self = Self(MimeTypeWire::Xml);
    /// An HTTP message: a request or a response with its head and its body,
    /// as RFC 9112 frames one on the wire.
    pub const HTTP: Self = Self(MimeTypeWire::Http);
    /// A PDF document.
    pub const PDF: Self = Self(MimeTypeWire::Pdf);
    /// CBOR structured data.
    pub const CBOR: Self = Self(MimeTypeWire::Cbor);
    /// MessagePack structured data.
    pub const MESSAGE_PACK: Self = Self(MimeTypeWire::MessagePack);
    /// Protocol Buffers data.
    pub const PROTOBUF: Self = Self(MimeTypeWire::Protobuf);
    /// A SQLite 3 database.
    pub const SQLITE3: Self = Self(MimeTypeWire::Sqlite3);
    /// A PNG image.
    pub const PNG: Self = Self(MimeTypeWire::Png);
    /// A JPEG image.
    pub const JPEG: Self = Self(MimeTypeWire::Jpeg);
    /// A GIF image.
    pub const GIF: Self = Self(MimeTypeWire::Gif);
    /// A WebP image.
    pub const WEBP: Self = Self(MimeTypeWire::WebP);
    /// An SVG image.
    pub const SVG: Self = Self(MimeTypeWire::Svg);
    /// MPEG audio, conventionally an MP3 file.
    pub const MP3: Self = Self(MimeTypeWire::Mp3);
    /// WAV audio.
    pub const WAV: Self = Self(MimeTypeWire::Wav);
    /// Ogg audio.
    pub const OGG: Self = Self(MimeTypeWire::Ogg);
    /// FLAC audio.
    pub const FLAC: Self = Self(MimeTypeWire::Flac);
    /// MP4 video.
    pub const MP4: Self = Self(MimeTypeWire::Mp4);
    /// WebM video.
    pub const WEBM: Self = Self(MimeTypeWire::WebM);
    /// A Web Open Font Format font.
    pub const WOFF: Self = Self(MimeTypeWire::Woff);
    /// A Web Open Font Format 2 font.
    pub const WOFF2: Self = Self(MimeTypeWire::Woff2);
    /// A TrueType font.
    pub const TTF: Self = Self(MimeTypeWire::Ttf);
    /// An OpenType font.
    pub const OTF: Self = Self(MimeTypeWire::Otf);
    /// A binary Microsoft Excel workbook.
    pub const XLS: Self = Self(MimeTypeWire::Xls);
    /// An Office Open XML spreadsheet.
    pub const XLSX: Self = Self(MimeTypeWire::Xlsx);
    /// An OpenDocument spreadsheet.
    pub const ODS: Self = Self(MimeTypeWire::Ods);
    /// A binary Microsoft Word document.
    pub const DOC: Self = Self(MimeTypeWire::Doc);
    /// An Office Open XML word-processing document.
    pub const DOCX: Self = Self(MimeTypeWire::Docx);
    /// Gzip-compressed data.
    pub const GZIP: Self = Self(MimeTypeWire::Gzip);
    /// Zstandard-compressed data.
    pub const ZSTD: Self = Self(MimeTypeWire::Zstd);
    /// Brotli-compressed data.
    pub const BROTLI: Self = Self(MimeTypeWire::Brotli);
    /// Zlib-wrapped DEFLATE data.
    pub const ZLIB: Self = Self(MimeTypeWire::Zlib);
    /// Historic UNIX `compress` data.
    pub const COMPRESS: Self = Self(MimeTypeWire::Compress);
    /// Bzip2-compressed data.
    pub const BZIP2: Self = Self(MimeTypeWire::Bzip2);
    /// XZ-compressed data.
    pub const XZ: Self = Self(MimeTypeWire::Xz);
    /// LZ4-compressed data.
    pub const LZ4: Self = Self(MimeTypeWire::Lz4);
    /// Snappy-framed compressed data.
    pub const SNAPPY: Self = Self(MimeTypeWire::Snappy);
    /// A ZIP archive.
    pub const ZIP: Self = Self(MimeTypeWire::Zip);
    /// A 7-Zip archive.
    pub const SEVEN_ZIP: Self = Self(MimeTypeWire::SevenZip);
    /// A RAR archive.
    pub const RAR: Self = Self(MimeTypeWire::Rar);
    /// A tar archive.
    pub const TAR: Self = Self(MimeTypeWire::Tar);
    /// A file system directory, which holds entries rather than bytes.
    pub const DIRECTORY: Self = Self(MimeTypeWire::Directory);
    /// A regular file whose contents are not identified any further.
    ///
    /// This is the local-leaf counterpart of [`Self::DIRECTORY`]: it says the
    /// resource is a file, not what is in it. A file whose type *is* known
    /// reports that type instead.
    pub const FILE: Self = Self(MimeTypeWire::File);

    /// Report the MIME type of one local path from the file system.
    ///
    /// An existing directory is [`Self::DIRECTORY`]. Anything else is
    /// identified from its extension, falling back to [`Self::FILE`] when the
    /// name says nothing - so the answer always distinguishes a container from
    /// a leaf, which [`Self::from_path`] alone cannot.
    pub fn from_local_path(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref();
        if path.is_dir() {
            return Self::DIRECTORY;
        }
        Self::from_path(path).unwrap_or(Self::FILE)
    }

    /// Return whether this MIME value names a file system entry rather than a
    /// content format.
    pub const fn is_filesystem(&self) -> bool {
        matches!(self.0, MimeTypeWire::Directory | MimeTypeWire::File)
    }

    /// Return whether this MIME value names a container of other entries.
    pub const fn is_directory(&self) -> bool {
        matches!(self.0, MimeTypeWire::Directory)
    }

    /// Return whether this MIME value can describe an I/O value.
    ///
    /// A directory is the one MIME value that names a container rather than a
    /// value: every other known or custom MIME type can be presented as bytes
    /// or records by an I/O handle.
    pub const fn is_io(&self) -> bool {
        !self.is_directory()
    }

    /// Parse a canonical MIME name or one unambiguous extension/name alias.
    ///
    /// MIME parameters are deliberately outside this value. Use
    /// [`Self::from_content_type`] when parsing an HTTP-style `Content-Type`
    /// value that may contain parameters.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
        <Self as FromStr>::from_str(value)
    }

    /// Infer a MIME type from one extension, with or without its leading dot.
    pub fn from_extension(extension: &str) -> Result<Self> {
        let original = extension;
        let (extension, offset) = trim_ows(extension);
        let extension = extension.strip_prefix('.').unwrap_or(extension);
        let offset = offset + usize::from(original[offset..].starts_with('.'));
        if extension.is_empty()
            || extension
                .bytes()
                .any(|byte| matches!(byte, b'.' | b'/' | b'\\'))
        {
            return Err(parse_error(
                "MIME extension",
                offset,
                "expected one non-empty filename extension",
            ));
        }
        known_from_extension(extension)
            .ok_or_else(|| parse_error("MIME extension", offset, "unknown filename extension"))
    }

    /// Infer a MIME type from the final extension of a path.
    ///
    /// This method borrows the path extension directly and does not create an
    /// intermediate path string.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .ok_or_else(|| {
                parse_error(
                    "MIME path",
                    path.as_os_str().len(),
                    "path has no supported UTF-8 extension",
                )
            })?;
        Self::from_extension(extension)
    }

    /// Parse the base MIME name from an HTTP-style `Content-Type` value.
    ///
    /// Parameters are validated, including quoted values and duplicate names,
    /// but are not retained because `MimeType` represents only `type/subtype`.
    pub fn from_content_type(value: &str) -> Result<Self> {
        let (trimmed, offset) = trim_ows(value);
        let parameter_start = trimmed.find(';');
        let base_end = parameter_start.unwrap_or(trimmed.len());
        let base = trimmed[..base_end].trim_end_matches([' ', '\t']);
        if !base.contains('/') {
            return Err(parse_error(
                "content type",
                offset,
                "expected a MIME type/subtype before parameters",
            ));
        }
        let mime = parse_mime(base, "content type", offset)?;
        if let Some(start) = parameter_start {
            validate_content_type_parameters(trimmed, start, offset)?;
        }
        Ok(mime)
    }

    /// Convert a registered HTTP content-coding token to its MIME value.
    ///
    /// `identity` is not valid in `Content-Encoding`; unknown tokens are
    /// rejected instead of being guessed as custom MIME values.
    pub fn from_content_coding(value: &str) -> Result<Self> {
        let (value, offset) = trim_ows(value);
        if value.eq_ignore_ascii_case("gzip") {
            Ok(Self::GZIP)
        } else if value.eq_ignore_ascii_case("zstd") {
            Ok(Self::ZSTD)
        } else if value.eq_ignore_ascii_case("br") {
            Ok(Self::BROTLI)
        } else if value.eq_ignore_ascii_case("deflate") {
            Ok(Self::ZLIB)
        } else if value.eq_ignore_ascii_case("compress") {
            Ok(Self::COMPRESS)
        } else if value.eq_ignore_ascii_case("identity") {
            Err(parse_error(
                "content coding",
                offset,
                "identity must not appear in Content-Encoding",
            ))
        } else {
            Err(parse_error(
                "content coding",
                offset,
                "unknown or unsupported HTTP content coding",
            ))
        }
    }

    /// Return the canonical lowercase MIME name without allocating.
    pub fn as_str(&self) -> &str {
        match &self.0 {
            MimeTypeWire::OctetStream => "application/octet-stream",
            MimeTypeWire::Json => "application/json",
            MimeTypeWire::JsonLines => "application/x-ndjson",
            MimeTypeWire::Yaml => "application/yaml",
            MimeTypeWire::Toml => "application/toml",
            MimeTypeWire::Csv => "text/csv",
            MimeTypeWire::Tsv => "text/tab-separated-values",
            MimeTypeWire::Parquet => "application/vnd.apache.parquet",
            MimeTypeWire::ArrowFile => "application/vnd.apache.arrow.file",
            MimeTypeWire::ArrowStream => "application/vnd.apache.arrow.stream",
            MimeTypeWire::Avro => "application/avro",
            MimeTypeWire::Orc => "application/vnd.apache.orc",
            MimeTypeWire::Puffin => "application/vnd.apache.puffin",
            MimeTypeWire::PlainText => "text/plain",
            MimeTypeWire::KeyValue => "text/key-value",
            MimeTypeWire::Ullink => "text/ullink",
            MimeTypeWire::Fix => "text/fix",
            MimeTypeWire::Fixul => "text/fixul",
            MimeTypeWire::Fixml => "text/fixml",
            MimeTypeWire::Markdown => "text/markdown",
            MimeTypeWire::Html => "text/html",
            MimeTypeWire::Css => "text/css",
            MimeTypeWire::JavaScript => "text/javascript",
            MimeTypeWire::Xml => "application/xml",
            MimeTypeWire::Http => "message/http",
            MimeTypeWire::Pdf => "application/pdf",
            MimeTypeWire::Cbor => "application/cbor",
            MimeTypeWire::MessagePack => "application/vnd.msgpack",
            MimeTypeWire::Protobuf => "application/protobuf",
            MimeTypeWire::Sqlite3 => "application/vnd.sqlite3",
            MimeTypeWire::Png => "image/png",
            MimeTypeWire::Jpeg => "image/jpeg",
            MimeTypeWire::Gif => "image/gif",
            MimeTypeWire::WebP => "image/webp",
            MimeTypeWire::Svg => "image/svg+xml",
            MimeTypeWire::Mp3 => "audio/mpeg",
            MimeTypeWire::Wav => "audio/wav",
            MimeTypeWire::Ogg => "audio/ogg",
            MimeTypeWire::Flac => "audio/flac",
            MimeTypeWire::Mp4 => "video/mp4",
            MimeTypeWire::WebM => "video/webm",
            MimeTypeWire::Woff => "font/woff",
            MimeTypeWire::Woff2 => "font/woff2",
            MimeTypeWire::Ttf => "font/ttf",
            MimeTypeWire::Otf => "font/otf",
            MimeTypeWire::Xls => "application/vnd.ms-excel",
            MimeTypeWire::Xlsx => {
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
            }
            MimeTypeWire::Ods => "application/vnd.oasis.opendocument.spreadsheet",
            MimeTypeWire::Doc => "application/msword",
            MimeTypeWire::Docx => {
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
            }
            MimeTypeWire::Gzip => "application/gzip",
            MimeTypeWire::Zstd => "application/zstd",
            MimeTypeWire::Brotli => "application/x-brotli",
            MimeTypeWire::Zlib => "application/zlib",
            MimeTypeWire::Compress => "application/x-compress",
            MimeTypeWire::Bzip2 => "application/x-bzip2",
            MimeTypeWire::Xz => "application/x-xz",
            MimeTypeWire::Lz4 => "application/x-lz4",
            MimeTypeWire::Snappy => "application/x-snappy-framed",
            MimeTypeWire::Zip => "application/zip",
            MimeTypeWire::SevenZip => "application/x-7z-compressed",
            MimeTypeWire::Rar => "application/vnd.rar",
            MimeTypeWire::Tar => "application/x-tar",
            MimeTypeWire::Directory => "inode/directory",
            MimeTypeWire::File => "inode/file",
            MimeTypeWire::Custom(value) => value.as_str(),
        }
    }

    /// Return the top-level media category without allocating.
    pub fn top_level(&self) -> &str {
        let value = self.as_str();
        let slash = value.find('/').unwrap_or(value.len());
        &value[..slash]
    }

    /// Return the media subtype without allocating.
    pub fn subtype(&self) -> &str {
        let value = self.as_str();
        value
            .find('/')
            .and_then(|slash| value.get(slash + 1..))
            .unwrap_or("")
    }

    /// Return a structured syntax suffix such as `json` or `xml`.
    pub fn structured_suffix(&self) -> Option<&str> {
        self.subtype()
            .rsplit_once('+')
            .map(|(_, suffix)| suffix)
            .filter(|suffix| !suffix.is_empty())
    }

    /// Return the preferred filename extension without a leading dot.
    pub fn extension(&self) -> Option<&'static str> {
        match self.0 {
            MimeTypeWire::OctetStream => Some("bin"),
            MimeTypeWire::Json => Some("json"),
            MimeTypeWire::JsonLines => Some("jsonl"),
            MimeTypeWire::Yaml => Some("yaml"),
            MimeTypeWire::Toml => Some("toml"),
            MimeTypeWire::Csv => Some("csv"),
            MimeTypeWire::Tsv => Some("tsv"),
            MimeTypeWire::Parquet => Some("parquet"),
            MimeTypeWire::ArrowFile => Some("arrow"),
            MimeTypeWire::ArrowStream => Some("arrows"),
            MimeTypeWire::Avro => Some("avro"),
            MimeTypeWire::Orc => Some("orc"),
            MimeTypeWire::Puffin => Some("puffin"),
            MimeTypeWire::PlainText => Some("txt"),
            // These classify embedded frame syntax, not a filename format.
            MimeTypeWire::KeyValue
            | MimeTypeWire::Ullink
            | MimeTypeWire::Fix
            | MimeTypeWire::Fixul
            | MimeTypeWire::Fixml => None,
            MimeTypeWire::Markdown => Some("md"),
            MimeTypeWire::Html => Some("html"),
            MimeTypeWire::Css => Some("css"),
            MimeTypeWire::JavaScript => Some("js"),
            MimeTypeWire::Xml => Some("xml"),
            MimeTypeWire::Http => Some("http"),
            MimeTypeWire::Pdf => Some("pdf"),
            MimeTypeWire::Cbor => Some("cbor"),
            MimeTypeWire::MessagePack => Some("msgpack"),
            MimeTypeWire::Protobuf => Some("pb"),
            MimeTypeWire::Sqlite3 => Some("sqlite3"),
            MimeTypeWire::Png => Some("png"),
            MimeTypeWire::Jpeg => Some("jpg"),
            MimeTypeWire::Gif => Some("gif"),
            MimeTypeWire::WebP => Some("webp"),
            MimeTypeWire::Svg => Some("svg"),
            MimeTypeWire::Mp3 => Some("mp3"),
            MimeTypeWire::Wav => Some("wav"),
            MimeTypeWire::Ogg => Some("ogg"),
            MimeTypeWire::Flac => Some("flac"),
            MimeTypeWire::Mp4 => Some("mp4"),
            MimeTypeWire::WebM => Some("webm"),
            MimeTypeWire::Woff => Some("woff"),
            MimeTypeWire::Woff2 => Some("woff2"),
            MimeTypeWire::Ttf => Some("ttf"),
            MimeTypeWire::Otf => Some("otf"),
            MimeTypeWire::Xls => Some("xls"),
            MimeTypeWire::Xlsx => Some("xlsx"),
            MimeTypeWire::Ods => Some("ods"),
            MimeTypeWire::Doc => Some("doc"),
            MimeTypeWire::Docx => Some("docx"),
            MimeTypeWire::Gzip => Some("gz"),
            MimeTypeWire::Zstd => Some("zst"),
            MimeTypeWire::Brotli => Some("br"),
            MimeTypeWire::Zlib => Some("zz"),
            MimeTypeWire::Compress => Some("Z"),
            MimeTypeWire::Bzip2 => Some("bz2"),
            MimeTypeWire::Xz => Some("xz"),
            MimeTypeWire::Lz4 => Some("lz4"),
            MimeTypeWire::Snappy => Some("snappy"),
            MimeTypeWire::Zip => Some("zip"),
            MimeTypeWire::SevenZip => Some("7z"),
            MimeTypeWire::Rar => Some("rar"),
            MimeTypeWire::Tar => Some("tar"),
            // A file system entry is named by its kind, not by an extension.
            MimeTypeWire::Directory | MimeTypeWire::File => None,
            MimeTypeWire::Custom(_) => match self.structured_suffix() {
                Some("json") => Some("json"),
                Some("xml") => Some("xml"),
                Some("yaml") => Some("yaml"),
                Some("cbor") => Some("cbor"),
                _ => None,
            },
        }
    }

    /// Return the registered HTTP content-coding token, when one exists.
    pub const fn content_coding(&self) -> Option<&'static str> {
        match self.0 {
            MimeTypeWire::Gzip => Some("gzip"),
            MimeTypeWire::Zstd => Some("zstd"),
            MimeTypeWire::Brotli => Some("br"),
            MimeTypeWire::Zlib => Some("deflate"),
            MimeTypeWire::Compress => Some("compress"),
            _ => None,
        }
    }

    /// Return the Yggdryl structured-text format represented by this MIME type.
    pub fn format(&self) -> Option<Format> {
        match self.0 {
            // A FIX frame carrying XML in a tag is not a document and does
            // not read as one; a bridge configuration document is JSON, and
            // is answered as the JSON it is.
            MimeTypeWire::Json => Some(Format::Json),
            MimeTypeWire::JsonLines => Some(Format::JsonLines),
            MimeTypeWire::Yaml => Some(Format::Yaml),
            MimeTypeWire::Toml => Some(Format::Toml),
            MimeTypeWire::Xml => Some(Format::Xml),
            _ => match self.structured_suffix() {
                Some("json") => Some(Format::Json),
                Some("xml") => Some(Format::Xml),
                Some("yaml") => Some(Format::Yaml),
                _ => None,
            },
        }
    }

    /// Return whether this value uses a static, allocation-free representation.
    pub const fn is_known(&self) -> bool {
        !matches!(self.0, MimeTypeWire::Custom(_))
    }

    /// Return whether this has the `application` top-level type.
    pub fn is_application(&self) -> bool {
        self.top_level() == "application"
    }

    /// Return whether this has the `audio` top-level type.
    pub fn is_audio(&self) -> bool {
        self.top_level() == "audio"
    }

    /// Return whether this has the `font` top-level type.
    pub fn is_font(&self) -> bool {
        self.top_level() == "font"
    }

    /// Return whether this has the `haptics` top-level type.
    pub fn is_haptics(&self) -> bool {
        self.top_level() == "haptics"
    }

    /// Return whether this has the `image` top-level type.
    pub fn is_image(&self) -> bool {
        self.top_level() == "image"
    }

    /// Return whether this has the `message` top-level type.
    pub fn is_message(&self) -> bool {
        self.top_level() == "message"
    }

    /// Return whether this has the `model` top-level type.
    pub fn is_model(&self) -> bool {
        self.top_level() == "model"
    }

    /// Return whether this has the `multipart` top-level type.
    pub fn is_multipart(&self) -> bool {
        self.top_level() == "multipart"
    }

    /// Return whether this has the `text` top-level type.
    pub fn is_text(&self) -> bool {
        self.top_level() == "text"
    }

    /// Return whether this has the `video` top-level type.
    pub fn is_video(&self) -> bool {
        self.top_level() == "video"
    }

    /// Return whether the representation is inherently readable text.
    pub fn is_textual(&self) -> bool {
        self.is_text()
            || matches!(
                self.0,
                MimeTypeWire::Json
                    | MimeTypeWire::JsonLines
                    | MimeTypeWire::Yaml
                    | MimeTypeWire::Toml
                    | MimeTypeWire::Xml
                    | MimeTypeWire::Svg
            )
            || matches!(
                self.structured_suffix(),
                Some("json" | "xml" | "yaml" | "toml")
            )
    }

    /// Return whether the representation has a machine-readable structure.
    pub fn is_structured(&self) -> bool {
        self.is_tabular()
            || matches!(
                self.0,
                MimeTypeWire::Json
                    | MimeTypeWire::JsonLines
                    | MimeTypeWire::Yaml
                    | MimeTypeWire::Toml
                    | MimeTypeWire::Xml
                    | MimeTypeWire::Cbor
                    | MimeTypeWire::MessagePack
                    | MimeTypeWire::Protobuf
                    | MimeTypeWire::Sqlite3
                    | MimeTypeWire::Puffin
            )
            || matches!(
                self.structured_suffix(),
                Some("json" | "xml" | "yaml" | "toml" | "cbor")
            )
    }

    /// Return whether the representation stores logical rows and columns.
    pub const fn is_tabular(&self) -> bool {
        matches!(
            self.0,
            MimeTypeWire::Csv
                | MimeTypeWire::Tsv
                | MimeTypeWire::Parquet
                | MimeTypeWire::ArrowFile
                | MimeTypeWire::ArrowStream
                | MimeTypeWire::Avro
                | MimeTypeWire::Orc
                | MimeTypeWire::Xls
                | MimeTypeWire::Xlsx
                | MimeTypeWire::Ods
        )
    }

    /// Return whether this MIME value denotes a transparent byte encoding.
    pub const fn is_encoding(&self) -> bool {
        matches!(
            self.0,
            MimeTypeWire::Gzip
                | MimeTypeWire::Zstd
                | MimeTypeWire::Brotli
                | MimeTypeWire::Zlib
                | MimeTypeWire::Compress
                | MimeTypeWire::Bzip2
                | MimeTypeWire::Xz
                | MimeTypeWire::Lz4
                | MimeTypeWire::Snappy
        )
    }

    /// Return whether this is an archive/container rather than a transparent encoding.
    pub const fn is_archive(&self) -> bool {
        matches!(
            self.0,
            MimeTypeWire::Zip | MimeTypeWire::SevenZip | MimeTypeWire::Rar | MimeTypeWire::Tar
        )
    }

    /// Conservatively identify MIME values whose registered representation is binary.
    pub fn is_binary(&self) -> bool {
        if self.is_textual() {
            return false;
        }
        self.is_encoding()
            || self.is_archive()
            || self.is_image()
            || self.is_audio()
            || self.is_video()
            || self.is_font()
            || matches!(
                self.0,
                MimeTypeWire::OctetStream
                    | MimeTypeWire::Parquet
                    | MimeTypeWire::ArrowFile
                    | MimeTypeWire::ArrowStream
                    | MimeTypeWire::Avro
                    | MimeTypeWire::Orc
                    | MimeTypeWire::Puffin
                    | MimeTypeWire::Pdf
                    | MimeTypeWire::Cbor
                    | MimeTypeWire::MessagePack
                    | MimeTypeWire::Protobuf
                    | MimeTypeWire::Sqlite3
                    | MimeTypeWire::Xls
                    | MimeTypeWire::Xlsx
                    | MimeTypeWire::Ods
                    | MimeTypeWire::Doc
                    | MimeTypeWire::Docx
                    | MimeTypeWire::File
            )
    }

    /// Return a deterministic cross-language hash of the canonical MIME name.
    pub fn stable_hash(&self) -> u64 {
        stable_hash_display(self)
    }
}

impl Default for MimeType {
    fn default() -> Self {
        Self::OCTET_STREAM
    }
}

impl FromStr for MimeType {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let original = value;
        let (value, offset) = trim_ows(value);
        if value.contains('/') {
            parse_mime(value, "MIME type", offset)
        } else {
            Self::from_extension(original)
        }
    }
}

impl PartialEq for MimeType {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for MimeType {}

impl PartialOrd for MimeType {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for MimeType {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl Hash for MimeType {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

impl AsRef<str> for MimeType {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for MimeType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for MimeType {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for MimeType {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct MimeTypeVisitor;

        impl<'de> serde::de::Visitor<'de> for MimeTypeVisitor {
            type Value = MimeType;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a MIME type/subtype string")
            }

            fn visit_borrowed_str<E>(self, value: &'de str) -> std::result::Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                MimeType::from_str(value).map_err(E::custom)
            }

            fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                MimeType::from_str(value).map_err(E::custom)
            }
        }

        deserializer.deserialize_str(MimeTypeVisitor)
    }
}

fn parse_mime(value: &str, target: &'static str, offset: usize) -> Result<MimeType> {
    if let Some(known) = known_from_mime(value) {
        return Ok(known);
    }

    let bytes = value.as_bytes();
    let Some(slash) = bytes.iter().position(|byte| *byte == b'/') else {
        return Err(parse_error(target, offset, "expected MIME type/subtype"));
    };
    if let Some(second) = bytes[slash + 1..].iter().position(|byte| *byte == b'/') {
        return Err(parse_error(
            target,
            offset + slash + 1 + second,
            "MIME name must contain exactly one slash",
        ));
    }

    validate_restricted_name(&bytes[..slash], target, offset, "type")?;
    validate_restricted_name(&bytes[slash + 1..], target, offset + slash + 1, "subtype")?;

    if bytes.iter().all(|byte| !byte.is_ascii_uppercase()) {
        return Ok(MimeType(MimeTypeWire::Custom(SmolStr::new(value))));
    }
    let mut canonical = SmolStrBuilder::new();
    for byte in bytes {
        canonical.push(char::from(byte.to_ascii_lowercase()));
    }
    Ok(MimeType(MimeTypeWire::Custom(canonical.into())))
}

fn validate_restricted_name(
    bytes: &[u8],
    target: &'static str,
    offset: usize,
    component: &'static str,
) -> Result<()> {
    if bytes.is_empty() {
        return Err(parse_error(
            target,
            offset,
            if component == "type" {
                "MIME type must not be empty"
            } else {
                "MIME subtype must not be empty"
            },
        ));
    }
    if bytes.len() > 127 {
        return Err(parse_error(
            target,
            offset + 127,
            if component == "type" {
                "MIME type exceeds 127 bytes"
            } else {
                "MIME subtype exceeds 127 bytes"
            },
        ));
    }
    if !bytes[0].is_ascii_alphanumeric() {
        return Err(parse_error(
            target,
            offset,
            "MIME type and subtype must start with an ASCII letter or digit",
        ));
    }
    if let Some(position) = bytes.iter().position(|byte| {
        !(byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'!' | b'#' | b'$' | b'&' | b'-' | b'^' | b'_' | b'.' | b'+'
            ))
    }) {
        return Err(parse_error(
            target,
            offset + position,
            "invalid character in MIME restricted name",
        ));
    }
    Ok(())
}

fn validate_content_type_parameters(value: &str, mut cursor: usize, offset: usize) -> Result<()> {
    let bytes = value.as_bytes();
    // Most Content-Type values have zero or one parameter. Keep that hot path
    // allocation-free, then promote wide inputs to a case-insensitive hash set
    // so adversarial parameter lists do not create quadratic work.
    let mut small_names: [Option<&str>; 8] = [None; 8];
    let mut small_len = 0;
    let mut wide_names: Option<HashSet<Caseless<'_>>> = None;
    while cursor < bytes.len() {
        if bytes[cursor] != b';' {
            return Err(parse_error(
                "content type",
                offset + cursor,
                "expected semicolon before Content-Type parameter",
            ));
        }
        cursor += 1;
        skip_ows(bytes, &mut cursor);
        let name_start = cursor;
        while cursor < bytes.len() && is_http_token_byte(bytes[cursor]) {
            cursor += 1;
        }
        if cursor == name_start {
            return Err(parse_error(
                "content type",
                offset + cursor,
                "Content-Type parameter name must not be empty",
            ));
        }
        let name = &value[name_start..cursor];
        let duplicate = if let Some(names) = wide_names.as_mut() {
            !names.insert(Caseless(name))
        } else if small_names[..small_len]
            .iter()
            .flatten()
            .any(|existing| existing.eq_ignore_ascii_case(name))
        {
            true
        } else if small_len < small_names.len() {
            small_names[small_len] = Some(name);
            small_len += 1;
            false
        } else {
            let mut names = HashSet::with_capacity(small_names.len() * 2);
            names.extend(small_names.iter().flatten().copied().map(Caseless));
            let inserted = names.insert(Caseless(name));
            wide_names = Some(names);
            !inserted
        };
        if duplicate {
            return Err(parse_error(
                "content type",
                offset + name_start,
                "duplicate Content-Type parameter",
            ));
        }

        skip_ows(bytes, &mut cursor);
        if bytes.get(cursor) != Some(&b'=') {
            return Err(parse_error(
                "content type",
                offset + cursor,
                "expected equals after Content-Type parameter name",
            ));
        }
        cursor += 1;
        skip_ows(bytes, &mut cursor);
        if bytes.get(cursor) == Some(&b'"') {
            cursor += 1;
            let mut closed = false;
            while cursor < bytes.len() {
                match bytes[cursor] {
                    b'"' => {
                        cursor += 1;
                        closed = true;
                        break;
                    }
                    b'\\' => {
                        cursor += 1;
                        let Some(escaped) = bytes.get(cursor) else {
                            return Err(parse_error(
                                "content type",
                                offset + cursor,
                                "unterminated quoted Content-Type parameter",
                            ));
                        };
                        if !is_quoted_parameter_byte(*escaped) {
                            return Err(parse_error(
                                "content type",
                                offset + cursor,
                                "invalid escaped byte in quoted Content-Type parameter",
                            ));
                        }
                        cursor += 1;
                    }
                    byte if is_quoted_parameter_byte(byte) && byte != b'\\' => cursor += 1,
                    _ => {
                        return Err(parse_error(
                            "content type",
                            offset + cursor,
                            "invalid byte in quoted Content-Type parameter",
                        ));
                    }
                }
            }
            if !closed {
                return Err(parse_error(
                    "content type",
                    offset + cursor,
                    "unterminated quoted Content-Type parameter",
                ));
            }
        } else {
            let value_start = cursor;
            while cursor < bytes.len() && is_http_token_byte(bytes[cursor]) {
                cursor += 1;
            }
            if cursor == value_start {
                return Err(parse_error(
                    "content type",
                    offset + cursor,
                    "Content-Type parameter value must not be empty",
                ));
            }
        }
        skip_ows(bytes, &mut cursor);
        if cursor < bytes.len() && bytes[cursor] != b';' {
            return Err(parse_error(
                "content type",
                offset + cursor,
                "unexpected data after Content-Type parameter",
            ));
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
struct Caseless<'a>(&'a str);

impl PartialEq for Caseless<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.0.eq_ignore_ascii_case(other.0)
    }
}

impl Eq for Caseless<'_> {}

impl Hash for Caseless<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        for byte in self.0.bytes() {
            byte.to_ascii_lowercase().hash(state);
        }
    }
}

fn skip_ows(bytes: &[u8], cursor: &mut usize) {
    while bytes
        .get(*cursor)
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        *cursor += 1;
    }
}

const fn is_http_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'!' | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'-'
                | b'.'
                | b'^'
                | b'_'
                | b'`'
                | b'|'
                | b'~'
        )
}

const fn is_quoted_parameter_byte(byte: u8) -> bool {
    matches!(byte, b'\t' | b' '..=b'~') || byte >= 0x80
}

fn trim_ows(value: &str) -> (&str, usize) {
    let trimmed_start = value.trim_start_matches([' ', '\t']);
    let offset = value.len() - trimmed_start.len();
    (trimmed_start.trim_end_matches([' ', '\t']), offset)
}

fn parse_error(target: &'static str, position: usize, reason: &'static str) -> Error {
    Error::Parse {
        target,
        position,
        reason: reason.into(),
    }
}
