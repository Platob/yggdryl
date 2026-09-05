//! Node.js views of native MIME and compound media types.

mod vocabulary;

use napi::bindgen_prelude::{ClassInstance, Either, Either3, Result};
use napi_derive::napi;
use yggdryl::{MediaType as CoreMediaType, MimeType as CoreMimeType};

use crate::{napi_error, ordering_value};

pub(crate) type MimeTypeInput<'a> = Either<ClassInstance<'a, JsMimeType>, String>;
pub(crate) type MediaTypeInput<'a> =
    Either3<ClassInstance<'a, JsMediaType>, ClassInstance<'a, JsMimeType>, String>;

/// Extract an exact native MIME wrapper or parse a MIME/extension string.
pub(crate) fn mime_type_from_input(value: MimeTypeInput<'_>) -> Result<CoreMimeType> {
    match value {
        Either::A(value) => Ok(value.inner.clone()),
        Either::B(value) => CoreMimeType::from_str(&value).map_err(napi_error),
    }
}

/// Extract an exact media wrapper, promote a MIME wrapper, or parse a string.
pub(crate) fn media_type_from_input(value: MediaTypeInput<'_>) -> Result<CoreMediaType> {
    match value {
        Either3::A(value) => Ok(value.inner.clone()),
        Either3::B(value) => Ok(CoreMediaType::new(value.inner.clone())),
        Either3::C(value) => CoreMediaType::from_str(&value).map_err(napi_error),
    }
}

fn mime_types_from_inputs(values: Vec<MimeTypeInput<'_>>) -> Result<Vec<CoreMimeType>> {
    values.into_iter().map(mime_type_from_input).collect()
}

/// An immutable canonical MIME `type/subtype` value.
#[napi(js_name = "MimeType")]
pub struct JsMimeType {
    pub(crate) inner: CoreMimeType,
}

impl Clone for JsMimeType {
    fn clone(&self) -> Self {
        Self::from_core(self.inner.clone())
    }
}

impl JsMimeType {
    pub(crate) fn from_core(inner: CoreMimeType) -> Self {
        Self { inner }
    }
}

#[napi]
impl JsMimeType {
    /// Parse a MIME/extension string or cheaply clone another native value.
    #[napi(constructor)]
    pub fn new(value: Option<MimeTypeInput<'_>>) -> Result<Self> {
        value.map_or_else(
            || Ok(Self::from_core(CoreMimeType::default())),
            |value| mime_type_from_input(value).map(Self::from_core),
        )
    }

    /// Infer from a native wrapper or MIME/extension string.
    #[napi(factory, js_name = "from")]
    pub fn from_js(value: MimeTypeInput<'_>) -> Result<Self> {
        mime_type_from_input(value).map(Self::from_core)
    }

    /// Resolve one exact known constant for the public loader facade.
    #[napi(factory, js_name = "_known", skip_typescript)]
    pub fn known(name: String) -> Result<Self> {
        let value = match name.as_str() {
            "OCTET_STREAM" => CoreMimeType::OCTET_STREAM,
            "JSON" => CoreMimeType::JSON,
            "JSON_LINES" => CoreMimeType::JSON_LINES,
            "YAML" => CoreMimeType::YAML,
            "TOML" => CoreMimeType::TOML,
            "CSV" => CoreMimeType::CSV,
            "TSV" => CoreMimeType::TSV,
            "PARQUET" => CoreMimeType::PARQUET,
            "ARROW_FILE" => CoreMimeType::ARROW_FILE,
            "ARROW_STREAM" => CoreMimeType::ARROW_STREAM,
            "AVRO" => CoreMimeType::AVRO,
            "ORC" => CoreMimeType::ORC,
            "PUFFIN" => CoreMimeType::PUFFIN,
            "PLAIN_TEXT" => CoreMimeType::PLAIN_TEXT,
            "ULLINK" => CoreMimeType::ULLINK,
            "FIX" => CoreMimeType::FIX,
            "FIXML" => CoreMimeType::FIXML,
            "MARKDOWN" => CoreMimeType::MARKDOWN,
            "HTML" => CoreMimeType::HTML,
            "CSS" => CoreMimeType::CSS,
            "JAVASCRIPT" => CoreMimeType::JAVASCRIPT,
            "XML" => CoreMimeType::XML,
            "PDF" => CoreMimeType::PDF,
            "CBOR" => CoreMimeType::CBOR,
            "MESSAGE_PACK" => CoreMimeType::MESSAGE_PACK,
            "PROTOBUF" => CoreMimeType::PROTOBUF,
            "SQLITE3" => CoreMimeType::SQLITE3,
            "PNG" => CoreMimeType::PNG,
            "JPEG" => CoreMimeType::JPEG,
            "GIF" => CoreMimeType::GIF,
            "WEBP" => CoreMimeType::WEBP,
            "SVG" => CoreMimeType::SVG,
            "MP3" => CoreMimeType::MP3,
            "WAV" => CoreMimeType::WAV,
            "OGG" => CoreMimeType::OGG,
            "FLAC" => CoreMimeType::FLAC,
            "MP4" => CoreMimeType::MP4,
            "WEBM" => CoreMimeType::WEBM,
            "WOFF" => CoreMimeType::WOFF,
            "WOFF2" => CoreMimeType::WOFF2,
            "TTF" => CoreMimeType::TTF,
            "OTF" => CoreMimeType::OTF,
            "XLS" => CoreMimeType::XLS,
            "XLSX" => CoreMimeType::XLSX,
            "ODS" => CoreMimeType::ODS,
            "DOC" => CoreMimeType::DOC,
            "DOCX" => CoreMimeType::DOCX,
            "GZIP" => CoreMimeType::GZIP,
            "ZSTD" => CoreMimeType::ZSTD,
            "BROTLI" => CoreMimeType::BROTLI,
            "ZLIB" => CoreMimeType::ZLIB,
            "COMPRESS" => CoreMimeType::COMPRESS,
            "BZIP2" => CoreMimeType::BZIP2,
            "XZ" => CoreMimeType::XZ,
            "LZ4" => CoreMimeType::LZ4,
            "SNAPPY" => CoreMimeType::SNAPPY,
            "ZIP" => CoreMimeType::ZIP,
            "SEVEN_ZIP" => CoreMimeType::SEVEN_ZIP,
            "RAR" => CoreMimeType::RAR,
            "TAR" => CoreMimeType::TAR,
            _ => return Err(napi::Error::from_reason("unknown native MIME constant")),
        };
        Ok(Self::from_core(value))
    }

    /// Parse a canonical MIME name or one unambiguous extension alias.
    #[napi(factory)]
    pub fn from_string(value: String) -> Result<Self> {
        CoreMimeType::from_str(&value)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Infer from one extension, with or without its leading dot.
    #[napi(factory)]
    pub fn from_extension(value: String) -> Result<Self> {
        CoreMimeType::from_extension(&value)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Infer from the final extension of a path.
    #[napi(factory)]
    pub fn from_path(value: String) -> Result<Self> {
        CoreMimeType::from_path(value)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Parse the base MIME value from Content-Type, validating parameters.
    #[napi(factory)]
    pub fn from_content_type(value: String) -> Result<Self> {
        CoreMimeType::from_content_type(&value)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Parse one registered HTTP Content-Encoding token.
    #[napi(factory)]
    pub fn from_content_coding(value: String) -> Result<Self> {
        CoreMimeType::from_content_coding(&value)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Deserialize the structural native JSON representation.
    #[napi(factory, js_name = "fromJSON")]
    pub fn from_json(value: serde_json::Value) -> Result<Self> {
        serde_json::from_value(value)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Top-level media category.
    #[napi(getter)]
    pub fn top_level(&self) -> String {
        self.inner.top_level().to_owned()
    }

    /// Media subtype.
    #[napi(getter)]
    pub fn subtype(&self) -> String {
        self.inner.subtype().to_owned()
    }

    /// Structured syntax suffix, when present.
    #[napi(getter)]
    pub fn structured_suffix(&self) -> Option<String> {
        self.inner.structured_suffix().map(ToOwned::to_owned)
    }

    /// Preferred filename extension.
    #[napi(getter)]
    pub fn extension(&self) -> Option<String> {
        self.inner.extension().map(ToOwned::to_owned)
    }

    /// Registered HTTP Content-Encoding token.
    #[napi(
        getter,
        ts_return_type = "'gzip' | 'compress' | 'deflate' | 'br' | 'zstd' | null"
    )]
    pub fn content_coding(&self) -> Option<String> {
        self.inner.content_coding().map(ToOwned::to_owned)
    }

    /// Yggdryl structured-text format represented by this MIME value.
    #[napi(
        getter,
        ts_return_type = "'json' | 'json_lines' | 'yaml' | 'toml' | null"
    )]
    pub fn format(&self) -> Option<String> {
        self.inner.format().map(|format| format.as_str().to_owned())
    }

    /// Whether this uses a built-in allocation-free value.
    #[napi]
    pub fn is_known(&self) -> bool {
        self.inner.is_known()
    }

    /// Whether the top-level type is application.
    #[napi]
    pub fn is_application(&self) -> bool {
        self.inner.is_application()
    }

    /// Whether the top-level type is audio.
    #[napi]
    pub fn is_audio(&self) -> bool {
        self.inner.is_audio()
    }

    /// Whether the top-level type is font.
    #[napi]
    pub fn is_font(&self) -> bool {
        self.inner.is_font()
    }

    /// Whether the top-level type is haptics.
    #[napi]
    pub fn is_haptics(&self) -> bool {
        self.inner.is_haptics()
    }

    /// Whether the top-level type is image.
    #[napi]
    pub fn is_image(&self) -> bool {
        self.inner.is_image()
    }

    /// Whether the top-level type is message.
    #[napi]
    pub fn is_message(&self) -> bool {
        self.inner.is_message()
    }

    /// Whether the top-level type is model.
    #[napi]
    pub fn is_model(&self) -> bool {
        self.inner.is_model()
    }

    /// Whether the top-level type is multipart.
    #[napi]
    pub fn is_multipart(&self) -> bool {
        self.inner.is_multipart()
    }

    /// Whether the top-level type is text.
    #[napi]
    pub fn is_text(&self) -> bool {
        self.inner.is_text()
    }

    /// Whether the top-level type is video.
    #[napi]
    pub fn is_video(&self) -> bool {
        self.inner.is_video()
    }

    /// Whether the representation is textual.
    #[napi]
    pub fn is_textual(&self) -> bool {
        self.inner.is_textual()
    }

    /// Whether the representation is machine-structured.
    #[napi]
    pub fn is_structured(&self) -> bool {
        self.inner.is_structured()
    }

    /// Whether the representation is tabular.
    #[napi]
    pub fn is_tabular(&self) -> bool {
        self.inner.is_tabular()
    }

    /// Whether this MIME value can describe an I/O value.
    #[napi]
    pub fn is_io(&self) -> bool {
        self.inner.is_io()
    }

    /// Whether this MIME value denotes a transparent encoding.
    #[napi]
    pub fn is_encoding(&self) -> bool {
        self.inner.is_encoding()
    }

    /// Whether this MIME value denotes an archive/container.
    #[napi]
    pub fn is_archive(&self) -> bool {
        self.inner.is_archive()
    }

    /// Whether the representation is conservatively binary.
    #[napi]
    pub fn is_binary(&self) -> bool {
        self.inner.is_binary()
    }

    /// Exact normalized equality.
    #[napi]
    pub fn equals(&self, other: &JsMimeType) -> bool {
        self.inner == other.inner
    }

    /// Total native ordering: -1, 0, or 1.
    #[napi]
    pub fn compare(&self, other: &JsMimeType) -> i32 {
        ordering_value(self.inner.cmp(&other.inner))
    }

    /// Deterministic native hash.
    #[napi]
    pub fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    /// Make a cheap native clone.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        self.clone()
    }

    /// Canonical MIME syntax.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.inner.to_string()
    }

    /// Serialize to version-independent structural JSON.
    #[napi(js_name = "toJSON")]
    pub fn js_json(&self) -> Result<serde_json::Value> {
        serde_json::to_value(&self.inner).map_err(napi_error)
    }
}

/// A base MIME type plus ordered transparent encodings.
#[napi(js_name = "MediaType")]
pub struct JsMediaType {
    pub(crate) inner: CoreMediaType,
}

impl Clone for JsMediaType {
    fn clone(&self) -> Self {
        Self::from_core(self.inner.clone())
    }
}

impl JsMediaType {
    pub(crate) fn from_core(inner: CoreMediaType) -> Self {
        Self { inner }
    }
}

#[napi]
impl JsMediaType {
    /// Parse/clone a media value or promote one MIME value.
    #[napi(constructor)]
    pub fn new(value: Option<MediaTypeInput<'_>>) -> Result<Self> {
        value.map_or_else(
            || Ok(Self::from_core(CoreMediaType::default())),
            |value| media_type_from_input(value).map(Self::from_core),
        )
    }

    /// Infer from a native media/MIME wrapper or media string.
    #[napi(factory, js_name = "from")]
    pub fn from_js(value: MediaTypeInput<'_>) -> Result<Self> {
        media_type_from_input(value).map(Self::from_core)
    }

    /// Parse canonical media syntax, a MIME name, or filename/path syntax.
    #[napi(factory)]
    pub fn from_string(value: String) -> Result<Self> {
        CoreMediaType::from_str(&value)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Internal array boundary adapted to a public iterable by the loader.
    #[napi(factory, js_name = "_fromParts", skip_typescript)]
    pub fn from_parts(base: MimeTypeInput<'_>, encodings: Vec<MimeTypeInput<'_>>) -> Result<Self> {
        CoreMediaType::from_parts(
            mime_type_from_input(base)?,
            mime_types_from_inputs(encodings)?,
        )
        .map(Self::from_core)
        .map_err(napi_error)
    }

    /// Infer from one filename extension.
    #[napi(factory)]
    pub fn from_extension(value: String) -> Self {
        Self::from_core(CoreMediaType::from_extension(&value))
    }

    /// Internal array boundary adapted to a public iterable by the loader.
    #[napi(factory, js_name = "_fromExtensions", skip_typescript)]
    pub fn from_extensions(values: Vec<String>) -> Self {
        Self::from_core(CoreMediaType::from_extensions(values))
    }

    /// Infer from a UTF-8 filename.
    #[napi(factory)]
    pub fn from_file_name(value: String) -> Self {
        Self::from_core(CoreMediaType::from_file_name(&value))
    }

    /// Infer from all compound path suffixes.
    #[napi(factory)]
    pub fn from_path(value: String) -> Result<Self> {
        CoreMediaType::from_path(value)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Parse Content-Type and Content-Encoding header values together.
    #[napi(factory)]
    pub fn from_content_headers(
        content_type: Option<String>,
        content_encoding: Option<String>,
    ) -> Result<Self> {
        CoreMediaType::from_content_headers(content_type.as_deref(), content_encoding.as_deref())
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Deserialize the structural native JSON representation.
    #[napi(factory, js_name = "fromJSON")]
    pub fn from_json(value: serde_json::Value) -> Result<Self> {
        serde_json::from_value(value)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Underlying unencoded MIME type.
    #[napi(getter)]
    pub fn base(&self) -> JsMimeType {
        JsMimeType::from_core(self.inner.base().clone())
    }

    /// Ordered transparent encodings.
    #[napi(getter)]
    pub fn encodings(&self) -> Vec<JsMimeType> {
        self.inner
            .encodings()
            .iter()
            .cloned()
            .map(JsMimeType::from_core)
            .collect()
    }

    /// Final outermost encoding.
    #[napi(getter)]
    pub fn encoding(&self) -> Option<JsMimeType> {
        self.inner.encoding().cloned().map(JsMimeType::from_core)
    }

    /// Number of applied encodings.
    #[napi(getter, js_name = "length")]
    pub fn encoding_len(&self) -> u32 {
        u32::try_from(self.inner.encoding_len()).unwrap_or(u32::MAX)
    }

    /// Final preferred filename extension.
    #[napi(getter)]
    pub fn extension(&self) -> Option<String> {
        self.inner.extension().map(ToOwned::to_owned)
    }

    /// Preferred base and encoding extensions in filename order.
    #[napi(getter)]
    pub fn extensions(&self) -> Vec<String> {
        self.inner.extensions().map(ToOwned::to_owned).collect()
    }

    /// Replace the base MIME type.
    #[napi]
    pub fn set_base(&mut self, value: MimeTypeInput<'_>) -> Result<()> {
        self.inner.set_base(mime_type_from_input(value)?);
        Ok(())
    }

    /// Internal array boundary adapted to a public iterable by the loader.
    #[napi(js_name = "_setEncodings", skip_typescript)]
    pub fn set_encodings(&mut self, values: Vec<MimeTypeInput<'_>>) -> Result<()> {
        self.inner
            .set_encodings(mime_types_from_inputs(values)?)
            .map_err(napi_error)
    }

    /// Append one outer encoding.
    #[napi]
    pub fn push_encoding(&mut self, value: MimeTypeInput<'_>) -> Result<()> {
        self.inner
            .push_encoding(mime_type_from_input(value)?)
            .map_err(napi_error)
    }

    /// Remove all encodings.
    #[napi]
    pub fn clear_encodings(&mut self) -> bool {
        self.inner.clear_encodings()
    }

    /// Get one encoding by positive index.
    #[napi]
    pub fn get_encoding(&self, index: u32) -> Option<JsMimeType> {
        self.inner
            .get_encoding(index as usize)
            .cloned()
            .map(JsMimeType::from_core)
    }

    /// Get one encoding by Array-compatible positive or negative index.
    #[napi]
    pub fn at(&self, index: i32) -> Option<JsMimeType> {
        let len = i64::from(self.encoding_len());
        let index = i64::from(index);
        let resolved = if index < 0 { len + index } else { index };
        usize::try_from(resolved).ok().and_then(|index| {
            self.inner
                .get_encoding(index)
                .cloned()
                .map(JsMimeType::from_core)
        })
    }

    /// Test whether the ordered encoding collection contains a MIME value.
    #[napi]
    pub fn contains(&self, value: MimeTypeInput<'_>) -> Result<bool> {
        Ok(self
            .inner
            .encodings()
            .contains(&mime_type_from_input(value)?))
    }

    /// Whether at least one transparent encoding is present.
    #[napi]
    pub fn is_encoded(&self) -> bool {
        self.inner.is_encoded()
    }

    /// Whether the base top-level type is application.
    #[napi]
    pub fn is_application(&self) -> bool {
        self.inner.is_application()
    }

    /// Whether the base top-level type is audio.
    #[napi]
    pub fn is_audio(&self) -> bool {
        self.inner.is_audio()
    }

    /// Whether the base top-level type is font.
    #[napi]
    pub fn is_font(&self) -> bool {
        self.inner.is_font()
    }

    /// Whether the base top-level type is haptics.
    #[napi]
    pub fn is_haptics(&self) -> bool {
        self.inner.is_haptics()
    }

    /// Whether the base top-level type is image.
    #[napi]
    pub fn is_image(&self) -> bool {
        self.inner.is_image()
    }

    /// Whether the base top-level type is message.
    #[napi]
    pub fn is_message(&self) -> bool {
        self.inner.is_message()
    }

    /// Whether the base top-level type is model.
    #[napi]
    pub fn is_model(&self) -> bool {
        self.inner.is_model()
    }

    /// Whether the base top-level type is multipart.
    #[napi]
    pub fn is_multipart(&self) -> bool {
        self.inner.is_multipart()
    }

    /// Whether the base top-level type is text.
    #[napi]
    pub fn is_text(&self) -> bool {
        self.inner.is_text()
    }

    /// Whether the base top-level type is video.
    #[napi]
    pub fn is_video(&self) -> bool {
        self.inner.is_video()
    }

    /// Whether the underlying representation is textual.
    #[napi]
    pub fn is_textual(&self) -> bool {
        self.inner.is_textual()
    }

    /// Whether the underlying representation is machine-structured.
    #[napi]
    pub fn is_structured(&self) -> bool {
        self.inner.is_structured()
    }

    /// Whether the underlying representation is tabular.
    #[napi]
    pub fn is_tabular(&self) -> bool {
        self.inner.is_tabular()
    }

    /// Whether the base MIME value can describe an I/O value.
    #[napi]
    pub fn is_io(&self) -> bool {
        self.inner.is_io()
    }

    /// Whether the base MIME value itself denotes an encoding.
    #[napi]
    pub fn is_encoding(&self) -> bool {
        self.inner.is_encoding()
    }

    /// Whether the base MIME value denotes an archive/container.
    #[napi]
    pub fn is_archive(&self) -> bool {
        self.inner.is_archive()
    }

    /// Whether the encoded wire representation is certainly binary.
    #[napi]
    pub fn is_binary(&self) -> bool {
        self.inner.is_binary()
    }

    /// Exact normalized equality.
    #[napi]
    pub fn equals(&self, other: &JsMediaType) -> bool {
        self.inner == other.inner
    }

    /// Total native ordering: -1, 0, or 1.
    #[napi]
    pub fn compare(&self, other: &JsMediaType) -> i32 {
        ordering_value(self.inner.cmp(&other.inner))
    }

    /// Deterministic native hash.
    #[napi]
    pub fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    /// Make a cheap native clone.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        self.clone()
    }

    /// Canonical media syntax.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.inner.to_string()
    }

    /// Serialize to version-independent structural JSON.
    #[napi(js_name = "toJSON")]
    pub fn js_json(&self) -> Result<serde_json::Value> {
        serde_json::to_value(&self.inner).map_err(napi_error)
    }
}
