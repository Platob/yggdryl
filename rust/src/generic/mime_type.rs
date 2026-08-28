use std::cmp::Ordering;
use std::collections::HashSet;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smol_str::{SmolStr, SmolStrBuilder};

use crate::text::Format;
use crate::{Error, Result, stable_hash_display};

#[derive(Clone, Debug)]
enum MimeTypeValue {
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
    Markdown,
    Html,
    Css,
    JavaScript,
    Xml,
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

/// A validated, canonical MIME media type.
///
/// Common data, document, media, archive, and encoding values use static
/// allocation-free representations. Any valid RFC restricted `type/subtype`
/// name remains supported and is stored once in canonical ASCII lowercase.
#[derive(Clone, Debug)]
pub struct MimeType(MimeTypeValue);

impl MimeType {
    /// Arbitrary binary data and the default MIME type.
    pub const OCTET_STREAM: Self = Self(MimeTypeValue::OctetStream);
    /// JSON structured data.
    pub const JSON: Self = Self(MimeTypeValue::Json);
    /// Newline-delimited JSON data.
    pub const JSON_LINES: Self = Self(MimeTypeValue::JsonLines);
    /// YAML structured data.
    pub const YAML: Self = Self(MimeTypeValue::Yaml);
    /// TOML structured data.
    pub const TOML: Self = Self(MimeTypeValue::Toml);
    /// Comma-separated tabular text.
    pub const CSV: Self = Self(MimeTypeValue::Csv);
    /// Tab-separated tabular text.
    pub const TSV: Self = Self(MimeTypeValue::Tsv);
    /// Apache Parquet tabular data.
    pub const PARQUET: Self = Self(MimeTypeValue::Parquet);
    /// An Apache Arrow IPC file.
    pub const ARROW_FILE: Self = Self(MimeTypeValue::ArrowFile);
    /// An Apache Arrow IPC stream.
    pub const ARROW_STREAM: Self = Self(MimeTypeValue::ArrowStream);
    /// Apache Avro data.
    pub const AVRO: Self = Self(MimeTypeValue::Avro);
    /// Apache ORC tabular data.
    pub const ORC: Self = Self(MimeTypeValue::Orc);
    /// Apache Puffin statistics and index data.
    pub const PUFFIN: Self = Self(MimeTypeValue::Puffin);
    /// Unformatted plain text.
    pub const PLAIN_TEXT: Self = Self(MimeTypeValue::PlainText);
    /// Markdown text.
    pub const MARKDOWN: Self = Self(MimeTypeValue::Markdown);
    /// HTML text.
    pub const HTML: Self = Self(MimeTypeValue::Html);
    /// Cascading Style Sheets text.
    pub const CSS: Self = Self(MimeTypeValue::Css);
    /// JavaScript source text.
    pub const JAVASCRIPT: Self = Self(MimeTypeValue::JavaScript);
    /// XML structured data.
    pub const XML: Self = Self(MimeTypeValue::Xml);
    /// A PDF document.
    pub const PDF: Self = Self(MimeTypeValue::Pdf);
    /// CBOR structured data.
    pub const CBOR: Self = Self(MimeTypeValue::Cbor);
    /// MessagePack structured data.
    pub const MESSAGE_PACK: Self = Self(MimeTypeValue::MessagePack);
    /// Protocol Buffers data.
    pub const PROTOBUF: Self = Self(MimeTypeValue::Protobuf);
    /// A SQLite 3 database.
    pub const SQLITE3: Self = Self(MimeTypeValue::Sqlite3);
    /// A PNG image.
    pub const PNG: Self = Self(MimeTypeValue::Png);
    /// A JPEG image.
    pub const JPEG: Self = Self(MimeTypeValue::Jpeg);
    /// A GIF image.
    pub const GIF: Self = Self(MimeTypeValue::Gif);
    /// A WebP image.
    pub const WEBP: Self = Self(MimeTypeValue::WebP);
    /// An SVG image.
    pub const SVG: Self = Self(MimeTypeValue::Svg);
    /// MPEG audio, conventionally an MP3 file.
    pub const MP3: Self = Self(MimeTypeValue::Mp3);
    /// WAV audio.
    pub const WAV: Self = Self(MimeTypeValue::Wav);
    /// Ogg audio.
    pub const OGG: Self = Self(MimeTypeValue::Ogg);
    /// FLAC audio.
    pub const FLAC: Self = Self(MimeTypeValue::Flac);
    /// MP4 video.
    pub const MP4: Self = Self(MimeTypeValue::Mp4);
    /// WebM video.
    pub const WEBM: Self = Self(MimeTypeValue::WebM);
    /// A Web Open Font Format font.
    pub const WOFF: Self = Self(MimeTypeValue::Woff);
    /// A Web Open Font Format 2 font.
    pub const WOFF2: Self = Self(MimeTypeValue::Woff2);
    /// A TrueType font.
    pub const TTF: Self = Self(MimeTypeValue::Ttf);
    /// An OpenType font.
    pub const OTF: Self = Self(MimeTypeValue::Otf);
    /// A legacy Microsoft Excel workbook.
    pub const XLS: Self = Self(MimeTypeValue::Xls);
    /// An Office Open XML spreadsheet.
    pub const XLSX: Self = Self(MimeTypeValue::Xlsx);
    /// An OpenDocument spreadsheet.
    pub const ODS: Self = Self(MimeTypeValue::Ods);
    /// A legacy Microsoft Word document.
    pub const DOC: Self = Self(MimeTypeValue::Doc);
    /// An Office Open XML word-processing document.
    pub const DOCX: Self = Self(MimeTypeValue::Docx);
    /// Gzip-compressed data.
    pub const GZIP: Self = Self(MimeTypeValue::Gzip);
    /// Zstandard-compressed data.
    pub const ZSTD: Self = Self(MimeTypeValue::Zstd);
    /// Brotli-compressed data.
    pub const BROTLI: Self = Self(MimeTypeValue::Brotli);
    /// Zlib-wrapped DEFLATE data.
    pub const ZLIB: Self = Self(MimeTypeValue::Zlib);
    /// Historic UNIX `compress` data.
    pub const COMPRESS: Self = Self(MimeTypeValue::Compress);
    /// Bzip2-compressed data.
    pub const BZIP2: Self = Self(MimeTypeValue::Bzip2);
    /// XZ-compressed data.
    pub const XZ: Self = Self(MimeTypeValue::Xz);
    /// LZ4-compressed data.
    pub const LZ4: Self = Self(MimeTypeValue::Lz4);
    /// Snappy-framed compressed data.
    pub const SNAPPY: Self = Self(MimeTypeValue::Snappy);
    /// A ZIP archive.
    pub const ZIP: Self = Self(MimeTypeValue::Zip);
    /// A 7-Zip archive.
    pub const SEVEN_ZIP: Self = Self(MimeTypeValue::SevenZip);
    /// A RAR archive.
    pub const RAR: Self = Self(MimeTypeValue::Rar);
    /// A tar archive.
    pub const TAR: Self = Self(MimeTypeValue::Tar);
    /// A file system directory, which holds entries rather than bytes.
    pub const DIRECTORY: Self = Self(MimeTypeValue::Directory);
    /// A regular file whose contents are not identified any further.
    ///
    /// This is the local-leaf counterpart of [`Self::DIRECTORY`]: it says the
    /// resource is a file, not what is in it. A file whose type *is* known
    /// reports that type instead.
    pub const FILE: Self = Self(MimeTypeValue::File);

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
        matches!(self.0, MimeTypeValue::Directory | MimeTypeValue::File)
    }

    /// Return whether this MIME value names a container of other entries.
    pub const fn is_directory(&self) -> bool {
        matches!(self.0, MimeTypeValue::Directory)
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
        if value.eq_ignore_ascii_case("gzip") || value.eq_ignore_ascii_case("x-gzip") {
            Ok(Self::GZIP)
        } else if value.eq_ignore_ascii_case("zstd") {
            Ok(Self::ZSTD)
        } else if value.eq_ignore_ascii_case("br") {
            Ok(Self::BROTLI)
        } else if value.eq_ignore_ascii_case("deflate") {
            Ok(Self::ZLIB)
        } else if value.eq_ignore_ascii_case("compress") || value.eq_ignore_ascii_case("x-compress")
        {
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
            MimeTypeValue::OctetStream => "application/octet-stream",
            MimeTypeValue::Json => "application/json",
            MimeTypeValue::JsonLines => "application/x-ndjson",
            MimeTypeValue::Yaml => "application/yaml",
            MimeTypeValue::Toml => "application/toml",
            MimeTypeValue::Csv => "text/csv",
            MimeTypeValue::Tsv => "text/tab-separated-values",
            MimeTypeValue::Parquet => "application/vnd.apache.parquet",
            MimeTypeValue::ArrowFile => "application/vnd.apache.arrow.file",
            MimeTypeValue::ArrowStream => "application/vnd.apache.arrow.stream",
            MimeTypeValue::Avro => "application/avro",
            MimeTypeValue::Orc => "application/vnd.apache.orc",
            MimeTypeValue::Puffin => "application/vnd.apache.puffin",
            MimeTypeValue::PlainText => "text/plain",
            MimeTypeValue::Markdown => "text/markdown",
            MimeTypeValue::Html => "text/html",
            MimeTypeValue::Css => "text/css",
            MimeTypeValue::JavaScript => "text/javascript",
            MimeTypeValue::Xml => "application/xml",
            MimeTypeValue::Pdf => "application/pdf",
            MimeTypeValue::Cbor => "application/cbor",
            MimeTypeValue::MessagePack => "application/vnd.msgpack",
            MimeTypeValue::Protobuf => "application/protobuf",
            MimeTypeValue::Sqlite3 => "application/vnd.sqlite3",
            MimeTypeValue::Png => "image/png",
            MimeTypeValue::Jpeg => "image/jpeg",
            MimeTypeValue::Gif => "image/gif",
            MimeTypeValue::WebP => "image/webp",
            MimeTypeValue::Svg => "image/svg+xml",
            MimeTypeValue::Mp3 => "audio/mpeg",
            MimeTypeValue::Wav => "audio/wav",
            MimeTypeValue::Ogg => "audio/ogg",
            MimeTypeValue::Flac => "audio/flac",
            MimeTypeValue::Mp4 => "video/mp4",
            MimeTypeValue::WebM => "video/webm",
            MimeTypeValue::Woff => "font/woff",
            MimeTypeValue::Woff2 => "font/woff2",
            MimeTypeValue::Ttf => "font/ttf",
            MimeTypeValue::Otf => "font/otf",
            MimeTypeValue::Xls => "application/vnd.ms-excel",
            MimeTypeValue::Xlsx => {
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
            }
            MimeTypeValue::Ods => "application/vnd.oasis.opendocument.spreadsheet",
            MimeTypeValue::Doc => "application/msword",
            MimeTypeValue::Docx => {
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
            }
            MimeTypeValue::Gzip => "application/gzip",
            MimeTypeValue::Zstd => "application/zstd",
            MimeTypeValue::Brotli => "application/x-brotli",
            MimeTypeValue::Zlib => "application/zlib",
            MimeTypeValue::Compress => "application/x-compress",
            MimeTypeValue::Bzip2 => "application/x-bzip2",
            MimeTypeValue::Xz => "application/x-xz",
            MimeTypeValue::Lz4 => "application/x-lz4",
            MimeTypeValue::Snappy => "application/x-snappy-framed",
            MimeTypeValue::Zip => "application/zip",
            MimeTypeValue::SevenZip => "application/x-7z-compressed",
            MimeTypeValue::Rar => "application/vnd.rar",
            MimeTypeValue::Tar => "application/x-tar",
            MimeTypeValue::Directory => "inode/directory",
            MimeTypeValue::File => "inode/file",
            MimeTypeValue::Custom(value) => value.as_str(),
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
            MimeTypeValue::OctetStream => Some("bin"),
            MimeTypeValue::Json => Some("json"),
            MimeTypeValue::JsonLines => Some("jsonl"),
            MimeTypeValue::Yaml => Some("yaml"),
            MimeTypeValue::Toml => Some("toml"),
            MimeTypeValue::Csv => Some("csv"),
            MimeTypeValue::Tsv => Some("tsv"),
            MimeTypeValue::Parquet => Some("parquet"),
            MimeTypeValue::ArrowFile => Some("arrow"),
            MimeTypeValue::ArrowStream => Some("arrows"),
            MimeTypeValue::Avro => Some("avro"),
            MimeTypeValue::Orc => Some("orc"),
            MimeTypeValue::Puffin => Some("puffin"),
            MimeTypeValue::PlainText => Some("txt"),
            MimeTypeValue::Markdown => Some("md"),
            MimeTypeValue::Html => Some("html"),
            MimeTypeValue::Css => Some("css"),
            MimeTypeValue::JavaScript => Some("js"),
            MimeTypeValue::Xml => Some("xml"),
            MimeTypeValue::Pdf => Some("pdf"),
            MimeTypeValue::Cbor => Some("cbor"),
            MimeTypeValue::MessagePack => Some("msgpack"),
            MimeTypeValue::Protobuf => Some("pb"),
            MimeTypeValue::Sqlite3 => Some("sqlite3"),
            MimeTypeValue::Png => Some("png"),
            MimeTypeValue::Jpeg => Some("jpg"),
            MimeTypeValue::Gif => Some("gif"),
            MimeTypeValue::WebP => Some("webp"),
            MimeTypeValue::Svg => Some("svg"),
            MimeTypeValue::Mp3 => Some("mp3"),
            MimeTypeValue::Wav => Some("wav"),
            MimeTypeValue::Ogg => Some("ogg"),
            MimeTypeValue::Flac => Some("flac"),
            MimeTypeValue::Mp4 => Some("mp4"),
            MimeTypeValue::WebM => Some("webm"),
            MimeTypeValue::Woff => Some("woff"),
            MimeTypeValue::Woff2 => Some("woff2"),
            MimeTypeValue::Ttf => Some("ttf"),
            MimeTypeValue::Otf => Some("otf"),
            MimeTypeValue::Xls => Some("xls"),
            MimeTypeValue::Xlsx => Some("xlsx"),
            MimeTypeValue::Ods => Some("ods"),
            MimeTypeValue::Doc => Some("doc"),
            MimeTypeValue::Docx => Some("docx"),
            MimeTypeValue::Gzip => Some("gz"),
            MimeTypeValue::Zstd => Some("zst"),
            MimeTypeValue::Brotli => Some("br"),
            MimeTypeValue::Zlib => Some("zz"),
            MimeTypeValue::Compress => Some("Z"),
            MimeTypeValue::Bzip2 => Some("bz2"),
            MimeTypeValue::Xz => Some("xz"),
            MimeTypeValue::Lz4 => Some("lz4"),
            MimeTypeValue::Snappy => Some("snappy"),
            MimeTypeValue::Zip => Some("zip"),
            MimeTypeValue::SevenZip => Some("7z"),
            MimeTypeValue::Rar => Some("rar"),
            MimeTypeValue::Tar => Some("tar"),
            // A file system entry is named by its kind, not by an extension.
            MimeTypeValue::Directory | MimeTypeValue::File => None,
            MimeTypeValue::Custom(_) => match self.structured_suffix() {
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
            MimeTypeValue::Gzip => Some("gzip"),
            MimeTypeValue::Zstd => Some("zstd"),
            MimeTypeValue::Brotli => Some("br"),
            MimeTypeValue::Zlib => Some("deflate"),
            MimeTypeValue::Compress => Some("compress"),
            _ => None,
        }
    }

    /// Return the Yggdryl structured-text format represented by this MIME type.
    pub fn format(&self) -> Option<Format> {
        match self.0 {
            MimeTypeValue::Json => Some(Format::Json),
            MimeTypeValue::JsonLines => Some(Format::JsonLines),
            MimeTypeValue::Yaml => Some(Format::Yaml),
            MimeTypeValue::Toml => Some(Format::Toml),
            _ => match self.structured_suffix() {
                Some("json") => Some(Format::Json),
                Some("yaml") => Some(Format::Yaml),
                _ => None,
            },
        }
    }

    /// Return whether this value uses a static, allocation-free representation.
    pub const fn is_known(&self) -> bool {
        !matches!(self.0, MimeTypeValue::Custom(_))
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
                MimeTypeValue::Json
                    | MimeTypeValue::JsonLines
                    | MimeTypeValue::Yaml
                    | MimeTypeValue::Toml
                    | MimeTypeValue::Xml
                    | MimeTypeValue::Svg
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
                MimeTypeValue::Json
                    | MimeTypeValue::JsonLines
                    | MimeTypeValue::Yaml
                    | MimeTypeValue::Toml
                    | MimeTypeValue::Xml
                    | MimeTypeValue::Cbor
                    | MimeTypeValue::MessagePack
                    | MimeTypeValue::Protobuf
                    | MimeTypeValue::Sqlite3
                    | MimeTypeValue::Puffin
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
            MimeTypeValue::Csv
                | MimeTypeValue::Tsv
                | MimeTypeValue::Parquet
                | MimeTypeValue::ArrowFile
                | MimeTypeValue::ArrowStream
                | MimeTypeValue::Avro
                | MimeTypeValue::Orc
                | MimeTypeValue::Xls
                | MimeTypeValue::Xlsx
                | MimeTypeValue::Ods
        )
    }

    /// Return whether this MIME value denotes a transparent byte encoding.
    pub const fn is_encoding(&self) -> bool {
        matches!(
            self.0,
            MimeTypeValue::Gzip
                | MimeTypeValue::Zstd
                | MimeTypeValue::Brotli
                | MimeTypeValue::Zlib
                | MimeTypeValue::Compress
                | MimeTypeValue::Bzip2
                | MimeTypeValue::Xz
                | MimeTypeValue::Lz4
                | MimeTypeValue::Snappy
        )
    }

    /// Return whether this is an archive/container rather than a transparent encoding.
    pub const fn is_archive(&self) -> bool {
        matches!(
            self.0,
            MimeTypeValue::Zip | MimeTypeValue::SevenZip | MimeTypeValue::Rar | MimeTypeValue::Tar
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
                MimeTypeValue::OctetStream
                    | MimeTypeValue::Parquet
                    | MimeTypeValue::ArrowFile
                    | MimeTypeValue::ArrowStream
                    | MimeTypeValue::Avro
                    | MimeTypeValue::Orc
                    | MimeTypeValue::Puffin
                    | MimeTypeValue::Pdf
                    | MimeTypeValue::Cbor
                    | MimeTypeValue::MessagePack
                    | MimeTypeValue::Protobuf
                    | MimeTypeValue::Sqlite3
                    | MimeTypeValue::Xls
                    | MimeTypeValue::Xlsx
                    | MimeTypeValue::Ods
                    | MimeTypeValue::Doc
                    | MimeTypeValue::Docx
                    | MimeTypeValue::File
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
        return Ok(MimeType(MimeTypeValue::Custom(SmolStr::new(value))));
    }
    let mut canonical = SmolStrBuilder::new();
    for byte in bytes {
        canonical.push(char::from(byte.to_ascii_lowercase()));
    }
    Ok(MimeType(MimeTypeValue::Custom(canonical.into())))
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

macro_rules! known_mimes {
    ($value:expr, $(($mime:literal, $constant:ident)),+ $(,)?) => {{
        $(if $value.eq_ignore_ascii_case($mime) {
            return Some(MimeType::$constant);
        })+
        None
    }};
}

fn known_from_mime(value: &str) -> Option<MimeType> {
    known_mimes!(
        value,
        ("application/octet-stream", OCTET_STREAM),
        ("inode/directory", DIRECTORY),
        ("inode/file", FILE),
        ("application/json", JSON),
        ("application/x-ndjson", JSON_LINES),
        ("application/ndjson", JSON_LINES),
        ("application/jsonl", JSON_LINES),
        ("application/yaml", YAML),
        ("application/x-yaml", YAML),
        ("text/yaml", YAML),
        ("application/toml", TOML),
        ("text/csv", CSV),
        ("text/tab-separated-values", TSV),
        ("application/vnd.apache.parquet", PARQUET),
        ("application/vnd.apache.arrow.file", ARROW_FILE),
        ("application/vnd.apache.arrow.stream", ARROW_STREAM),
        ("application/avro", AVRO),
        ("application/vnd.apache.avro", AVRO),
        ("application/vnd.apache.orc", ORC),
        ("application/vnd.apache.puffin", PUFFIN),
        ("text/plain", PLAIN_TEXT),
        ("text/markdown", MARKDOWN),
        ("text/html", HTML),
        ("text/css", CSS),
        ("text/javascript", JAVASCRIPT),
        ("application/javascript", JAVASCRIPT),
        ("application/xml", XML),
        ("text/xml", XML),
        ("application/pdf", PDF),
        ("application/cbor", CBOR),
        ("application/vnd.msgpack", MESSAGE_PACK),
        ("application/msgpack", MESSAGE_PACK),
        ("application/protobuf", PROTOBUF),
        ("application/x-protobuf", PROTOBUF),
        ("application/vnd.sqlite3", SQLITE3),
        ("image/png", PNG),
        ("image/jpeg", JPEG),
        ("image/gif", GIF),
        ("image/webp", WEBP),
        ("image/svg+xml", SVG),
        ("audio/mpeg", MP3),
        ("audio/wav", WAV),
        ("audio/x-wav", WAV),
        ("audio/ogg", OGG),
        ("audio/flac", FLAC),
        ("video/mp4", MP4),
        ("video/webm", WEBM),
        ("font/woff", WOFF),
        ("font/woff2", WOFF2),
        ("font/ttf", TTF),
        ("font/otf", OTF),
        ("application/vnd.ms-excel", XLS),
        (
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            XLSX
        ),
        ("application/vnd.oasis.opendocument.spreadsheet", ODS),
        ("application/msword", DOC),
        (
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            DOCX
        ),
        ("application/gzip", GZIP),
        ("application/x-gzip", GZIP),
        ("application/zstd", ZSTD),
        ("application/x-brotli", BROTLI),
        ("application/zlib", ZLIB),
        ("application/x-compress", COMPRESS),
        ("application/x-bzip2", BZIP2),
        ("application/x-xz", XZ),
        ("application/x-lz4", LZ4),
        ("application/x-snappy-framed", SNAPPY),
        ("application/zip", ZIP),
        ("application/x-7z-compressed", SEVEN_ZIP),
        ("application/vnd.rar", RAR),
        ("application/x-rar-compressed", RAR),
        ("application/x-tar", TAR),
    )
}

macro_rules! known_extensions {
    ($value:expr, $(($extension:literal, $constant:ident)),+ $(,)?) => {{
        $(if $value.eq_ignore_ascii_case($extension) {
            return Some(MimeType::$constant);
        })+
        None
    }};
}

fn known_from_extension(value: &str) -> Option<MimeType> {
    known_extensions!(
        value,
        ("bin", OCTET_STREAM),
        ("json", JSON),
        ("jsonl", JSON_LINES),
        ("ndjson", JSON_LINES),
        ("yaml", YAML),
        ("yml", YAML),
        ("toml", TOML),
        ("csv", CSV),
        ("tsv", TSV),
        ("tab", TSV),
        ("parquet", PARQUET),
        ("pq", PARQUET),
        ("arrow", ARROW_FILE),
        ("feather", ARROW_FILE),
        ("ipc", ARROW_FILE),
        ("arrows", ARROW_STREAM),
        ("avro", AVRO),
        ("orc", ORC),
        ("puffin", PUFFIN),
        ("txt", PLAIN_TEXT),
        ("text", PLAIN_TEXT),
        ("log", PLAIN_TEXT),
        ("md", MARKDOWN),
        ("markdown", MARKDOWN),
        ("html", HTML),
        ("htm", HTML),
        ("css", CSS),
        ("js", JAVASCRIPT),
        ("mjs", JAVASCRIPT),
        ("cjs", JAVASCRIPT),
        ("xml", XML),
        ("pdf", PDF),
        ("cbor", CBOR),
        ("msgpack", MESSAGE_PACK),
        ("mpk", MESSAGE_PACK),
        ("pb", PROTOBUF),
        ("protobuf", PROTOBUF),
        ("sqlite", SQLITE3),
        ("sqlite3", SQLITE3),
        ("db", SQLITE3),
        ("png", PNG),
        ("jpg", JPEG),
        ("jpeg", JPEG),
        ("jpe", JPEG),
        ("gif", GIF),
        ("webp", WEBP),
        ("svg", SVG),
        ("mp3", MP3),
        ("wav", WAV),
        ("wave", WAV),
        ("ogg", OGG),
        ("oga", OGG),
        ("flac", FLAC),
        ("mp4", MP4),
        ("m4v", MP4),
        ("webm", WEBM),
        ("woff", WOFF),
        ("woff2", WOFF2),
        ("ttf", TTF),
        ("otf", OTF),
        ("xls", XLS),
        ("xlsx", XLSX),
        ("ods", ODS),
        ("doc", DOC),
        ("docx", DOCX),
        ("gz", GZIP),
        ("gzip", GZIP),
        ("zst", ZSTD),
        ("zstd", ZSTD),
        ("br", BROTLI),
        ("zz", ZLIB),
        ("zlib", ZLIB),
        ("Z", COMPRESS),
        ("bz2", BZIP2),
        ("bzip2", BZIP2),
        ("xz", XZ),
        ("lz4", LZ4),
        ("snappy", SNAPPY),
        ("sz", SNAPPY),
        ("zip", ZIP),
        ("7z", SEVEN_ZIP),
        ("rar", RAR),
        ("tar", TAR),
    )
}
